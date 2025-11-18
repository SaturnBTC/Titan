use {
    crate::{
        index::{
            store::StoreError,
            updater::{events::Events, store_lock::StoreWithLock, transaction::TransactionStore},
            Chain, Settings,
        },
        models::{
            BatchDelete, BatchUpdate, BlockId, Inscription, RuneEntry, TransactionStateChange,
            TransactionStateChangeInput,
        },
    },
    bitcoin::{consensus, ScriptBuf, Transaction},
    rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet},
    std::{sync::Arc, time::Instant},
    titan_types_core::Rune,
    titan_types_core::RuneId,
    titan_types_core::{
        Event, InscriptionId, Location, MempoolEntry, SerializedOutPoint, SerializedTxid,
        SpenderReference, SpentStatus, TxOut,
    },
    tracing::{debug, info},
};

type Result<T> = std::result::Result<T, StoreError>;

pub struct MempoolCacheSettings {
    pub chain: Chain,
    pub index_addresses: bool,
}

impl MempoolCacheSettings {
    pub fn new(settings: &Settings) -> Self {
        Self {
            chain: settings.chain,
            index_addresses: settings.index_addresses,
        }
    }
}

pub struct MempoolCache {
    db: Arc<StoreWithLock>,
    update: BatchUpdate,
    pub settings: MempoolCacheSettings,
}

impl MempoolCache {
    pub fn new(db: Arc<StoreWithLock>, settings: MempoolCacheSettings) -> Result<Self> {
        let (rune_count, mut block_count, purged_blocks_count) = {
            let db = db.read();
            (
                db.get_runes_count()?,
                db.get_block_count()?,
                db.get_purged_blocks_count()?,
            )
        };

        // In regtest, the first block is not accessible via RPC and for testing purposes we
        // don't need to start at block 0.
        if block_count == 0 && settings.chain == Chain::Regtest {
            block_count = 1;
        }

        Ok(Self {
            db,
            update: BatchUpdate::new(rune_count, block_count, purged_blocks_count),
            settings,
        })
    }

    pub fn get_block_count(&self) -> u64 {
        self.update.block_count
    }

    pub fn does_tx_exist(&self, txid: SerializedTxid) -> Result<bool> {
        if self.update.tx_state_changes.contains_key(&txid) {
            return Ok(true);
        }

        return self.db.read().is_tx_in_mempool(&txid);
    }

    pub fn set_mempool_tx(&mut self, txid: SerializedTxid, mempool_entry: MempoolEntry) -> () {
        self.update.mempool_txs.insert(txid, mempool_entry);
    }

    fn get_tx_out(&self, outpoint: &SerializedOutPoint) -> Result<TxOut> {
        if let Some(tx_out) = self.update.txouts.get(outpoint) {
            return Ok(tx_out.clone());
        }

        let tx_out = self.db.read().get_tx_out(outpoint, None)?;
        Ok(tx_out)
    }

    fn update_script_pubkeys(&mut self) -> Result<()> {
        let mut spk_map: HashMap<ScriptBuf, (Vec<SerializedOutPoint>, Vec<SerializedOutPoint>)> =
            HashMap::default();

        for (outpoint, script_pubkey) in &self.update.script_pubkeys_outpoints {
            let entry: &mut (Vec<SerializedOutPoint>, Vec<SerializedOutPoint>) =
                spk_map.entry(script_pubkey.clone()).or_default();

            entry.0.push(*outpoint); // new
        }

        self.update.script_pubkeys = spk_map;

        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        let db = self.db.write();

        if self.settings.index_addresses {
            self.update_script_pubkeys()?;
        }

        if !self.update.is_empty() {
            let start = Instant::now();
            db.batch_update(&self.update, true)?;
            debug!("Flushed update: {} in {:?}", self.update, start.elapsed());
        }

        Ok(())
    }

    pub fn add_address_events(&mut self, events: &mut Events, chain: Chain) {
        for script_pubkey in self.update.script_pubkeys.keys() {
            let address = chain.address_from_script(script_pubkey);
            if let Ok(address) = address {
                events.add_event(Event::AddressModified {
                    address: address.to_string(),
                    location: Location::mempool(),
                });
            }
        }
    }
}

impl TransactionStore for MempoolCache {
    fn get_tx_outs(
        &mut self,
        outpoints: &[SerializedOutPoint],
    ) -> Result<HashMap<SerializedOutPoint, TxOut>> {
        let mut results = HashMap::default();
        let mut to_fetch = HashSet::default();

        for outpoint in outpoints {
            if let Some(tx_out) = self.update.txouts.get(outpoint) {
                results.insert(*outpoint, tx_out.clone());
                continue;
            }

            to_fetch.insert(*outpoint);
        }

        if !to_fetch.is_empty() {
            let tx_outs = self
                .db
                .read()
                .get_tx_outs(&to_fetch.iter().cloned().collect::<Vec<_>>(), None)?;

            results.extend(tx_outs);
        }

        Ok(results)
    }

    fn set_tx_out(
        &mut self,
        outpoint: SerializedOutPoint,
        tx_out: TxOut,
        script_pubkey: ScriptBuf,
    ) {
        self.update.txouts.insert(outpoint, tx_out);

        if self.settings.index_addresses {
            self.update
                .script_pubkeys_outpoints
                .insert(outpoint, script_pubkey);
        }
    }

    fn set_spent_tx_out(
        &mut self,
        outpoint: &TransactionStateChangeInput,
        spent: SpenderReference,
    ) -> Result<()> {
        match self.get_tx_out(&outpoint.previous_outpoint) {
            Ok(tx_out) => {
                let mut tx_out = tx_out;
                tx_out.spent = SpentStatus::Spent(spent.clone());
                self.update
                    .txouts
                    .insert(outpoint.previous_outpoint, tx_out);
            }
            Err(e) => return Err(e),
        }

        if self.settings.index_addresses {
            self.update
                .spent_outpoints_in_mempool
                .insert(outpoint.previous_outpoint, spent);
        }

        Ok(())
    }

    fn get_transaction(
        &self,
        txid: &SerializedTxid,
    ) -> std::result::Result<Transaction, StoreError> {
        // 1. Check current in-memory update.
        if let Some(transaction) = self.update.transactions.get(txid) {
            return Ok(transaction.clone());
        }

        // 4. Fallback to DB.
        let transaction_raw = self.db.read().get_transaction_raw(txid, None)?;
        let tx: Transaction = consensus::deserialize(&transaction_raw)?;
        Ok(tx)
    }

    fn set_transaction(&mut self, txid: SerializedTxid, transaction: Transaction) {
        self.update.transactions.insert(txid, transaction);
    }

    fn get_transaction_confirming_block(
        &self,
        txid: &SerializedTxid,
    ) -> std::result::Result<BlockId, StoreError> {
        // 1. Check current in-memory update.
        if let Some(block_id) = self.update.transaction_confirming_block.get(txid) {
            return Ok(block_id.clone());
        }

        // 2. Fallback to DB.
        let block_id = self.db.read().get_transaction_confirming_block(txid)?;
        Ok(block_id)
    }

    fn set_transaction_confirming_block(&mut self, _txid: SerializedTxid, _block_id: BlockId) {
        panic!("set_transaction_confirming_block shouldn't be called in mempool cache");
    }

    fn get_rune(&mut self, rune_id: &RuneId) -> std::result::Result<RuneEntry, StoreError> {
        // 1. Current update.
        if let Some(rune) = self.update.runes.get(rune_id) {
            return Ok(rune.clone());
        }

        // 2. DB.
        let rune = self.db.read().get_rune(rune_id)?;
        Ok(rune)
    }

    fn does_rune_exist(&mut self, rune: &Rune) -> std::result::Result<(), StoreError> {
        // 1. Current update.
        if let Some(_) = self.update.rune_ids.get(&rune.0) {
            return Ok(());
        }

        // 2. DB.
        let _ = self.db.read().get_rune_id(rune)?;
        Ok(())
    }

    fn get_runes_count(&self) -> u64 {
        self.update.rune_count
    }

    fn set_rune_id(&mut self, rune: Rune, id: RuneId) {
        self.update.rune_ids.insert(rune.0, id);
    }

    fn set_rune(&mut self, id: RuneId, entry: RuneEntry) {
        self.update.runes.insert(id, entry);
    }

    fn set_rune_id_number(&mut self, number: u64, id: RuneId) {
        self.update.rune_numbers.insert(number, id);
    }

    fn increment_runes_count(&mut self) {
        self.update.rune_count += 1;
    }

    fn set_inscription(&mut self, id: InscriptionId, inscription: Inscription) {
        self.update.inscriptions.insert(id, inscription);
    }

    fn set_tx_state_changes(
        &mut self,
        txid: SerializedTxid,
        tx_state_changes: TransactionStateChange,
    ) {
        self.update.tx_state_changes.insert(txid, tx_state_changes);
    }

    fn add_rune_transaction(&mut self, rune_id: RuneId, txid: SerializedTxid) {
        self.update
            .rune_transactions
            .entry(rune_id)
            .or_insert(vec![])
            .push(txid);
    }
}
