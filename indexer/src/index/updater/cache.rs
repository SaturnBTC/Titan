use {
    super::{index_updater::RuneMintable, store_lock::StoreWithLock},
    crate::{
        index::{store::StoreError, Chain, Settings},
        models::{
            BatchDelete, BatchUpdate, BlockId, Inscription, RuneEntry, TransactionStateChange,
        },
    },
    bitcoin::{consensus, BlockHash, OutPoint, ScriptBuf, Transaction, Txid},
    ordinals::{Rune, RuneId},
    std::{
        cmp,
        collections::{HashMap, HashSet},
        str::FromStr,
        sync::Arc,
        time::Instant,
    },
    titan_types::{Block, Event, InscriptionId, Location, SpenderReference, TxOutEntry},
    tokio::sync::mpsc,
    tracing::{info, trace},
};

type Result<T> = std::result::Result<T, StoreError>;

pub(super) struct UpdaterCacheSettings {
    pub max_recoverable_reorg_depth: u64,
    pub index_spent_outputs: bool,
    pub mempool: bool,
}

impl UpdaterCacheSettings {
    pub fn new(settings: &Settings, mempool: bool) -> Self {
        Self {
            max_recoverable_reorg_depth: settings.max_recoverable_reorg_depth(),
            index_spent_outputs: settings.index_spent_outputs,
            mempool,
        }
    }
}

pub(super) struct UpdaterCache {
    db: Arc<StoreWithLock>,
    update: BatchUpdate,
    delete: BatchDelete,
    events: Vec<Event>,
    first_block_height: u64,
    last_block_height: Option<u64>,
    pub settings: UpdaterCacheSettings,
}

impl UpdaterCache {
    pub fn new(db: Arc<StoreWithLock>, settings: UpdaterCacheSettings) -> Result<Self> {
        let (rune_count, block_count, purged_blocks_count) = {
            let db = db.read();
            (
                db.get_runes_count()?,
                db.get_block_count()?,
                db.get_purged_blocks_count()?,
            )
        };

        Ok(Self {
            db,
            update: BatchUpdate::new(rune_count, block_count, purged_blocks_count),
            delete: BatchDelete::new(),
            events: vec![],
            first_block_height: block_count,
            last_block_height: None,
            settings,
        })
    }

    pub fn get_runes_count(&self) -> u64 {
        self.update.rune_count
    }

    pub fn get_block_height_tip(&self) -> u64 {
        self.update.block_count.saturating_sub(1)
    }

    pub fn get_block_count(&self) -> u64 {
        self.update.block_count
    }

    pub fn get_purged_blocks_count(&self) -> u64 {
        self.update.purged_blocks_count
    }

    fn increment_block_count(&mut self) -> () {
        self.last_block_height = Some(self.update.block_count);
        self.update.block_count += 1;
    }

    pub fn get_block_by_height(&self, height: u64) -> Result<Block> {
        let hash = self.update.block_hashes.get(&height);

        if let Some(hash) = hash {
            return self.get_block(hash);
        } else {
            let hash = self.db.read().get_block_hash(height)?;
            return self.get_block(&hash);
        }
    }

    pub fn get_block(&self, hash: &BlockHash) -> Result<Block> {
        if let Some(block) = self.update.blocks.get(hash) {
            return Ok(block.clone());
        } else {
            let block = self.db.read().get_block_by_hash(hash)?;
            return Ok(block);
        }
    }

    pub fn set_new_block(&mut self, block: Block) -> () {
        assert_eq!(
            self.get_block_count(),
            block.height,
            "Block height mismatch"
        );

        let hash: BlockHash = block.header.block_hash();
        self.update.blocks.insert(hash, block);
        self.update
            .block_hashes
            .insert(self.get_block_count(), hash);
        self.increment_block_count();
    }

    pub fn increment_runes_count(&mut self) -> () {
        self.update.rune_count += 1;
    }

    pub fn decrement_runes_count(&mut self) -> () {
        self.update.rune_count -= 1;
    }

    pub fn get_transaction(&self, txid: Txid) -> Result<Transaction> {
        if let Some(transaction) = self.update.transactions.get(&txid) {
            return Ok(transaction.clone());
        } else {
            let transaction = self.db.read().get_transaction_raw(&txid, None)?;
            return Ok(consensus::deserialize(&transaction)?);
        }
    }

    pub fn get_transaction_confirming_block(&self, txid: Txid) -> Result<BlockId> {
        if let Some(block_id) = self.update.transaction_confirming_block.get(&txid) {
            return Ok(block_id.clone());
        } else {
            let block_id = self.db.read().get_transaction_confirming_block(&txid)?;
            return Ok(block_id);
        }
    }

    pub fn precache_tx_outs(&mut self, txs: &Vec<Transaction>) -> Result<()> {
        let outpoints: Vec<_> = txs
            .iter()
            .flat_map(|tx| tx.input.iter().map(|input| input.previous_output.into()))
            .collect();

        let mut to_fetch = HashSet::new();
        for outpoint in outpoints.iter() {
            if !self.update.txouts.contains_key(outpoint) {
                to_fetch.insert(outpoint.clone());
            }
        }

        if !to_fetch.is_empty() {
            let tx_outs = self
                .db
                .read()
                .get_tx_outs(&to_fetch.iter().cloned().collect(), None)?;

            self.update.txouts.extend(tx_outs);
        }

        Ok(())
    }

    pub fn get_tx_out(&self, outpoint: &OutPoint) -> Result<TxOutEntry> {
        if let Some(tx_out) = self.update.txouts.get(outpoint) {
            return Ok(tx_out.clone());
        } else {
            let tx_out = self.db.read().get_tx_out(outpoint, None)?;
            return Ok(tx_out);
        }
    }

    pub fn get_tx_outs(&self, outpoints: &Vec<OutPoint>) -> Result<HashMap<OutPoint, TxOutEntry>> {
        let mut results = HashMap::new();
        let mut to_fetch = HashSet::new();
        for outpoint in outpoints.iter() {
            if let Some(tx_out) = self.update.txouts.get(outpoint) {
                results.insert(outpoint.clone(), tx_out.clone());
            } else {
                to_fetch.insert(outpoint.clone());
            }
        }

        if !to_fetch.is_empty() {
            let tx_outs = self
                .db
                .read()
                .get_tx_outs(&to_fetch.iter().cloned().collect(), None)?;

            results.extend(tx_outs);
        }

        Ok(results)
    }

    pub fn set_tx_out(&mut self, outpoint: OutPoint, tx_out: TxOutEntry) -> () {
        self.update.txouts.insert(outpoint, tx_out);
    }

    pub fn does_tx_exist(&self, txid: Txid) -> Result<bool> {
        if self.update.tx_state_changes.contains_key(&txid) {
            return Ok(true);
        }

        if self.settings.mempool {
            return self.db.read().is_tx_in_mempool(&txid);
        }

        let tx_state_changes = self
            .db
            .read()
            .get_tx_state_changes(&txid, Some(self.settings.mempool));

        Ok(tx_state_changes.is_ok())
    }

    pub fn set_tx_state_changes(
        &mut self,
        txid: Txid,
        tx_state_changes: TransactionStateChange,
    ) -> () {
        self.update.tx_state_changes.insert(txid, tx_state_changes);
    }

    pub fn set_transaction(&mut self, txid: Txid, transaction: Transaction) -> () {
        self.update.transactions.insert(txid, transaction);
    }

    pub fn set_transaction_confirming_block(&mut self, txid: Txid, block_id: BlockId) -> () {
        self.update
            .transaction_confirming_block
            .insert(txid, block_id);
    }

    pub fn add_rune_transaction(&mut self, rune_id: RuneId, txid: Txid) -> () {
        self.update
            .rune_transactions
            .entry(rune_id)
            .or_insert(vec![])
            .push(txid);
    }

    pub fn get_rune(&self, rune_id: &RuneId) -> Result<RuneEntry> {
        if let Some(rune) = self.update.runes.get(rune_id) {
            return Ok(rune.clone());
        } else {
            let rune = self.db.read().get_rune(rune_id)?;
            return Ok(rune);
        }
    }

    pub fn set_rune(&mut self, rune_id: RuneId, rune: RuneEntry) -> () {
        self.update.runes.insert(rune_id, rune);
    }

    pub fn get_rune_id(&self, rune: &Rune) -> Result<RuneId> {
        if let Some(rune_id) = self.update.rune_ids.get(&rune.0) {
            return Ok(rune_id.clone());
        } else {
            let rune_id = self.db.read().get_rune_id(rune)?;
            return Ok(rune_id);
        }
    }

    pub fn set_rune_id(&mut self, rune: Rune, rune_id: RuneId) -> () {
        self.update.rune_ids.insert(rune.0, rune_id);
    }

    pub fn set_rune_name(&mut self, rune_name: String, rune_id: RuneId) -> () {
        self.update.rune_names.insert(rune_name, rune_id);
    }

    pub fn set_rune_id_number(&mut self, number: u64, rune_id: RuneId) -> () {
        self.update.rune_numbers.insert(number, rune_id);
    }

    pub fn set_rune_mintable_at_height(
        &mut self,
        rune_id: RuneId,
        rune_name: String,
        height: u64,
    ) -> () {
        self.update
            .rune_mintable_at_height
            .insert(rune_id, (rune_name, height));
    }

    pub fn set_rune_unmintable_at_height(
        &mut self,
        rune_id: RuneId,
        rune_name: String,
        height: u64,
    ) -> () {
        self.update
            .rune_unmintable_at_height
            .insert(rune_id, (rune_name, height));
    }

    pub fn set_inscription(
        &mut self,
        inscription_id: InscriptionId,
        inscription: Inscription,
    ) -> () {
        self.update.inscriptions.insert(inscription_id, inscription);
    }

    pub fn set_mempool_tx(&mut self, txid: Txid) -> () {
        self.update.mempool_txs.insert(txid);
    }

    pub fn set_script_pubkey_entries(
        &mut self,
        script_pubkey_entry: HashMap<ScriptBuf, (Vec<OutPoint>, Vec<OutPoint>)>,
    ) -> () {
        self.update.script_pubkeys = script_pubkey_entry;
    }

    pub fn get_outpoints_to_script_pubkey(
        &self,
        outpoints: &Vec<OutPoint>,
        optimistic: bool,
    ) -> Result<HashMap<OutPoint, ScriptBuf>> {
        return Ok(self.db.read().get_outpoints_to_script_pubkey(
            &outpoints,
            Some(self.settings.mempool),
            optimistic,
        )?);
    }

    pub fn batch_set_outpoints_to_script_pubkey(&mut self, items: HashMap<OutPoint, ScriptBuf>) {
        self.update.script_pubkeys_outpoints = items;
    }

    pub fn batch_set_spent_outpoints_in_mempool(
        &mut self,
        outpoints: HashMap<OutPoint, SpenderReference>,
    ) {
        self.update.spent_outpoints_in_mempool.extend(outpoints);
    }

    pub fn add_event(&mut self, event: Event) {
        self.events.push(event);
    }

    pub fn should_flush(&self, max_size: usize) -> bool {
        self.update.blocks.len() >= max_size
    }

    pub fn flush(&mut self) -> Result<()> {
        let db = self.db.write();

        if !self.settings.mempool {
            self.prepare_to_delete(
                self.settings.max_recoverable_reorg_depth,
                self.settings.index_spent_outputs,
            )?;
        }

        if !self.update.is_empty() {
            let start = Instant::now();
            db.batch_update(&self.update, self.settings.mempool)?;
            trace!("Flushed update: {} in {:?}", self.update, start.elapsed());
        }

        if !self.delete.is_empty() {
            let start = Instant::now();
            db.batch_delete(&self.delete)?;
            trace!("Flushed delete: {} in {:?}", self.delete, start.elapsed());
        }

        // Clear the cache
        self.update = BatchUpdate::new(
            self.update.rune_count,
            self.update.block_count,
            self.update.purged_blocks_count,
        );
        self.delete = BatchDelete::new();
        self.first_block_height = self.update.block_count;
        self.last_block_height = None;

        Ok(())
    }

    pub fn add_address_events(&mut self, chain: Chain) {
        for script_pubkey in self.update.script_pubkeys.keys() {
            let address = chain.address_from_script(script_pubkey);
            if let Ok(address) = address {
                self.events.push(Event::AddressModified {
                    address: address.to_string(),
                    location: if self.settings.mempool {
                        Location::mempool()
                    } else {
                        Location::block(self.get_block_height_tip())
                    },
                });
            }
        }
    }

    pub fn send_events(
        &mut self,
        event_sender: &Option<mpsc::Sender<Event>>,
    ) -> std::result::Result<(), mpsc::error::SendError<Event>> {
        if let Some(sender) = event_sender {
            for event in self.events.iter() {
                sender.blocking_send(event.clone())?;
            }
        }

        self.events = vec![];

        Ok(())
    }

    fn prepare_to_delete(
        &mut self,
        max_recoverable_reorg_depth: u64,
        index_spent_outputs: bool,
    ) -> Result<()> {
        if let Some(last_block_height) = self.last_block_height {
            let mut from_block_height_to_purge = self
                .first_block_height
                .checked_sub(max_recoverable_reorg_depth + 1)
                .unwrap_or(0);

            let to_block_height_to_purge =
                last_block_height.checked_sub(max_recoverable_reorg_depth);

            if let Some(to_block_height_to_purge) = to_block_height_to_purge {
                from_block_height_to_purge = cmp::max(
                    from_block_height_to_purge,
                    self.update.purged_blocks_count + 1,
                );

                info!(
                    "Purging blocks from {} to {}",
                    from_block_height_to_purge, to_block_height_to_purge,
                );

                for i in from_block_height_to_purge..to_block_height_to_purge {
                    self.purge_block(i, index_spent_outputs)?;
                }
            }
        }

        Ok(())
    }

    fn get_txs_state_changes(
        &self,
        txids: &Vec<Txid>,
    ) -> Result<HashMap<Txid, TransactionStateChange>> {
        let mut total_tx_state_changes = HashMap::new();
        let mut to_fetch = HashSet::new();
        for txid in txids {
            if let Some(tx_state_changes) = self.update.tx_state_changes.get(txid) {
                total_tx_state_changes.insert(txid.clone(), tx_state_changes.clone());
            } else {
                to_fetch.insert(txid.clone());
            }
        }

        if !to_fetch.is_empty() {
            let tx_state_changes = self.db.read().get_txs_state_changes(
                &to_fetch.iter().cloned().collect(),
                self.settings.mempool,
            )?;

            total_tx_state_changes.extend(tx_state_changes);
        }

        Ok(total_tx_state_changes)
    }

    fn purge_block(&mut self, height: u64, index_spent_outputs: bool) -> Result<()> {
        let block = self.get_block_by_height(height)?;

        let txids = block
            .tx_ids
            .iter()
            .map(|txid| Txid::from_str(txid).unwrap())
            .collect();

        let tx_state_changes = self.get_txs_state_changes(&txids)?;

        for txid in txids {
            let tx_state_changes = tx_state_changes.get(&txid).unwrap();

            for txin in tx_state_changes.inputs.iter() {
                if !index_spent_outputs {
                    self.delete.tx_outs.insert(txin.clone());
                }

                self.delete.script_pubkeys_outpoints.insert(txin.clone());
            }

            self.delete.tx_state_changes.insert(txid);
        }

        self.update.purged_blocks_count = height;

        Ok(())
    }
}

impl RuneMintable for UpdaterCache {
    fn set_mintable_rune(&mut self, rune_id: RuneId, rune_name: String, mintable: bool) {
        if mintable {
            self.update.rune_mintable.insert(rune_id, rune_name);
        } else {
            self.update.rune_unmintable.insert(rune_id, rune_name);
        }
    }
}
