use std::{num::NonZeroUsize, sync::Arc};

use bitcoin::{consensus, BlockHash, ScriptBuf, Transaction};
use clru::CLruCache;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use titan_types_core::{
    Block, Event, Location, SerializedOutPoint, SerializedTxid, SpenderReference, SpentStatus,
    TxOut,
};
use titan_types_core::{Rune, RuneId};
use tracing::info;

use crate::{
    index::{
        updater::{
            cache::bg_writer::{BatchDB, BgWriter, BgWriterSettings},
            events::Events,
            store_lock::StoreWithLock,
            transaction::TransactionStore,
        },
        Chain, Settings, StoreError,
    },
    models::{
        BatchDelete, BatchUpdate, BlockId, RuneEntry, TransactionStateChange,
        TransactionStateChangeInput,
    },
};

type Result<T> = std::result::Result<T, StoreError>;

pub struct BlockCacheSettings {
    pub max_recoverable_reorg_depth: u64,
    pub chain: Chain,
    pub max_async_batches: usize,
    pub index_addresses: bool,
    pub index_spent_outputs: bool,
    pub rune_cache_size: usize,
    pub outpoint_cache_size: usize,
}

impl BlockCacheSettings {
    pub fn new(settings: &Settings) -> Self {
        Self {
            max_recoverable_reorg_depth: settings.max_recoverable_reorg_depth(),
            chain: settings.chain,
            index_addresses: settings.index_addresses,
            index_spent_outputs: settings.index_spent_outputs,
            max_async_batches: 8,
            rune_cache_size: 1000,
            outpoint_cache_size: 10_000_000,
        }
    }
}

// Information needed to perform purging in the background
pub struct PurgeInfo {
    pub from_block_height_to_purge: u64,
    pub to_block_height_to_purge: u64,
}

pub struct BlockCache {
    db: Arc<StoreWithLock>,

    update: BatchUpdate,
    delete: BatchDelete,

    first_block_height: u64,
    last_block_height: Option<u64>,
    settings: BlockCacheSettings,

    outpoints: CLruCache<SerializedOutPoint, TxOut>,

    blocks: HashMap<u64, Block>,
    spent_outpoints: HashMap<SerializedTxid, Vec<TransactionStateChangeInput>>,

    runes: CLruCache<RuneId, RuneEntry>,
    rune_ids: HashSet<u128>,

    bg_writer: BgWriter,
}

impl BlockCache {
    pub fn new(db: Arc<StoreWithLock>, settings: BlockCacheSettings) -> Result<Self> {
        let (rune_count, mut block_count, purged_blocks_count) = {
            let db = db.read();
            (
                db.get_runes_count()?,
                db.get_block_count()?,
                db.get_purged_blocks_count()?,
            )
        };

        let (blocks, spent_outpoints) =
            Self::precache_block(&db, purged_blocks_count, block_count)?;

        // In regtest, the first block is not accessible via RPC and for testing purposes we
        // don't need to start at block 0.
        if block_count == 0 && settings.chain == Chain::Regtest {
            block_count = 1;
        }

        let rune_cache_size = NonZeroUsize::new(settings.rune_cache_size).unwrap();
        let max_async_batches = settings.max_async_batches;
        let outpoint_cache_size = NonZeroUsize::new(settings.outpoint_cache_size).unwrap();

        Ok(Self {
            db: db.clone(),
            update: BatchUpdate::new(rune_count, block_count, purged_blocks_count),
            delete: BatchDelete::new(),

            first_block_height: block_count,
            last_block_height: None,
            settings,

            outpoints: CLruCache::new(outpoint_cache_size),
            blocks,
            spent_outpoints,

            runes: CLruCache::new(rune_cache_size),
            rune_ids: HashSet::default(),

            bg_writer: BgWriter::start(db, BgWriterSettings { max_async_batches }),
        })
    }

    fn precache_block(
        db: &Arc<StoreWithLock>,
        purge_count: u64,
        block_count: u64,
    ) -> Result<(
        HashMap<u64, Block>,
        HashMap<SerializedTxid, Vec<TransactionStateChangeInput>>,
    )> {
        let blocks = {
            let blocks = db.read().get_blocks_by_heights(purge_count, block_count)?;
            let mut result = HashMap::default();
            for (_, block) in blocks {
                result.insert(block.height, block);
            }

            result
        };

        let all_txids = blocks
            .values()
            .flat_map(|b| b.tx_ids.iter().cloned())
            .collect::<Vec<_>>();

        let tx_state_changes = db.read().get_txs_state_changes(&all_txids, false)?;

        let spent_outpoints = tx_state_changes
            .into_iter()
            .map(|(txid, tx_state_change)| {
                (
                    txid,
                    tx_state_change
                        .inputs
                        .into_iter()
                        .map(|input| input)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<HashMap<SerializedTxid, Vec<TransactionStateChangeInput>>>();

        Ok((blocks, spent_outpoints))
    }

    pub fn get_block_height_tip(&self) -> u64 {
        self.update.block_count.saturating_sub(1)
    }

    pub fn get_block_count(&self) -> u64 {
        self.update.block_count
    }

    fn increment_block_count(&mut self) -> () {
        self.last_block_height = Some(self.update.block_count);
        self.update.block_count += 1;
    }

    pub fn get_block_hash(&mut self, height: u64) -> Result<BlockHash> {
        // 1. Check current in-memory update.
        if let Some(hash) = self.update.block_hashes.get(&height) {
            return Ok(hash.clone());
        }

        // 2. Check any pending batches that have been flushed but not yet persisted.
        if let Some(hash) = self
            .bg_writer
            .find_in_pending(|b| b.block_hashes.get(&height).cloned())
        {
            return Ok(hash);
        }

        // 3. Fall back to the persistent store.
        let hash = self.db.read().get_block_hash(height)?;

        Ok(hash)
    }

    pub fn set_new_block(&mut self, block: Block) -> () {
        assert_eq!(
            self.get_block_count(),
            block.height,
            "Block height mismatch"
        );

        let hash = block.header.block_hash();
        self.blocks.insert(block.height, block.clone());
        self.update.blocks.insert(hash, block);
        self.update
            .block_hashes
            .insert(self.get_block_count(), hash);

        self.increment_block_count();
    }

    fn get_tx_out(&mut self, outpoint: &SerializedOutPoint) -> Result<TxOut> {
        // 1. Check current in-memory update.
        if let Some(tx_out) = self.outpoints.get(outpoint) {
            return Ok(tx_out.clone());
        }

        // 2. Check any pending batches that have been flushed but not yet persisted.
        if let Some(tx_out) = self
            .bg_writer
            .find_in_pending(|b| b.txouts.get(outpoint).cloned())
        {
            return Ok(tx_out);
        }

        // 3. Fall back to the persistent store.
        let tx_out = self.db.read().get_tx_out(outpoint, Some(false))?;
        self.outpoints.put(*outpoint, tx_out.clone());
        Ok(tx_out)
    }

    fn prepare_to_delete(&mut self, max_recoverable_reorg_depth: u64) -> Result<()> {
        let purge_info = self.get_purge_info(max_recoverable_reorg_depth);

        if let Some(purge_info) = purge_info {
            // Use the new batched purge implementation to minimize DB round-trips
            self.purge_block_range(
                purge_info.from_block_height_to_purge,
                purge_info.to_block_height_to_purge,
            )?;
        }

        Ok(())
    }

    fn get_purge_info(&self, max_recoverable_reorg_depth: u64) -> Option<PurgeInfo> {
        if let Some(last_block_height) = self.last_block_height {
            let from_block_height_to_purge = self.update.purged_blocks_count;

            let to_block_height_to_purge =
                last_block_height.checked_sub(max_recoverable_reorg_depth);

            if let Some(to_block_height_to_purge) = to_block_height_to_purge {
                return Some(PurgeInfo {
                    from_block_height_to_purge,
                    to_block_height_to_purge,
                });
            }
        }

        None
    }

    /// Purge a contiguous range of block heights `[from, to)` in batch to reduce
    /// RocksDB look-ups. Requires `from < to`.
    fn purge_block_range(&mut self, from: u64, to: u64) -> Result<()> {
        info!("Purging blocks from {} to {}", from, to,);

        if from >= to {
            return Ok(());
        }

        for height in from..to {
            // In regtest, the first block is not accessible via RPC and for testing purposes we
            // don't need to start at block 0.
            if self.settings.chain == Chain::Regtest && height == 0 {
                continue;
            }

            let block = self
                .blocks
                .remove(&height)
                .ok_or(StoreError::NotFound(format!("block not found: {}", height)))?;

            for txid in &block.tx_ids {
                let inputs = self.spent_outpoints.remove(&txid);

                if let Some(inputs) = inputs {
                    for txin in inputs {
                        self.delete.script_pubkeys_outpoints.insert(txin.clone());

                        if !self.settings.index_spent_outputs {
                            self.delete.tx_outs.insert(txin.previous_outpoint.clone());
                        }
                    }

                    self.delete.tx_state_changes.insert(*txid);
                }
            }
        }

        // Update purged count
        self.update.purged_blocks_count = to;

        Ok(())
    }

    fn update_script_pubkeys(&mut self) -> Result<()> {
        for (outpoint, script_pubkey) in self.update.script_pubkeys_outpoints.iter() {
            let entry: &mut (Vec<SerializedOutPoint>, Vec<SerializedOutPoint>) = self
                .update
                .script_pubkeys
                .entry(script_pubkey.clone())
                .or_default();

            let tx_out = self.update.txouts.get(outpoint);

            if let Some(tx_out) = tx_out {
                if matches!(tx_out.spent, SpentStatus::Unspent) {
                    entry.0.push(outpoint.clone());
                } else {
                    entry.1.push(outpoint.clone());
                }
            } else {
                panic!("tx_out not found for outpoint: {}", outpoint);
            }
        }

        Ok(())
    }

    pub fn add_address_events(&mut self, events: &mut Events, chain: Chain) {
        for script_pubkey in self.update.script_pubkeys.keys() {
            let address = chain.address_from_script(script_pubkey);
            if let Ok(address) = address {
                events.add_event(Event::AddressModified {
                    address: address.to_string(),
                    location: Location::block(self.get_block_height_tip()),
                });
            }
        }
    }

    pub fn should_flush(&self, max_size: usize) -> bool {
        self.update.blocks.len() >= max_size
    }

    pub fn flush(&mut self) -> Result<()> {
        info!(
            "Block cache: outpoints: {:?}, spent_outpoints: {:?}, blocks: {:?}",
            self.outpoints.len(),
            self.spent_outpoints.len(),
            self.blocks.len()
        );

        self.prepare_to_delete(self.settings.max_recoverable_reorg_depth)?;

        if self.settings.index_addresses {
            self.update_script_pubkeys()?;
        }

        let new_update = BatchUpdate::new(
            self.update.rune_count,
            self.update.block_count,
            self.update.purged_blocks_count,
        );

        let update = std::mem::replace(&mut self.update, new_update);
        let delete = std::mem::replace(&mut self.delete, BatchDelete::new());

        let batch = Arc::new(BatchDB { update, delete });

        self.bg_writer.save(batch);

        self.last_block_height = None;
        self.first_block_height = self.update.block_count;

        Ok(())
    }

    /// Flush all pending updates and block until they are fully persisted.
    pub fn flush_sync(&mut self) -> Result<()> {
        // Perform the regular flush, which hands the batch off to the background writer.
        self.flush()?;

        // Wait for the background writer to drain the queue so that every update is
        // guaranteed to be visible via the persistent store.
        self.bg_writer.wait_until_empty();

        Ok(())
    }

    /// Flush the current batch and return a receiver that signals when persistence completes.
    pub fn flush_with_notify(&mut self) -> Result<crossbeam_channel::Receiver<()>> {
        self.prepare_to_delete(self.settings.max_recoverable_reorg_depth)?;

        if self.settings.index_addresses {
            self.update_script_pubkeys()?;
        }

        let new_update = BatchUpdate::new(
            self.update.rune_count,
            self.update.block_count,
            self.update.purged_blocks_count,
        );

        let update = std::mem::replace(&mut self.update, new_update);
        let delete = std::mem::replace(&mut self.delete, BatchDelete::new());

        let batch = Arc::new(BatchDB { update, delete });

        let rx = self.bg_writer.save_with_notify(batch);

        self.last_block_height = None;
        self.first_block_height = self.update.block_count;

        Ok(rx)
    }
}

impl TransactionStore for BlockCache {
    fn get_tx_outs(
        &mut self,
        outpoints: &[SerializedOutPoint],
    ) -> Result<HashMap<SerializedOutPoint, TxOut>> {
        let mut tx_outs = HashMap::default();
        let mut to_fetch = vec![];
        for outpoint in outpoints {
            if let Some(tx_out) = self.outpoints.get(outpoint) {
                tx_outs.insert(*outpoint, tx_out.clone());
            } else {
                if let Some(tx_out) = self
                    .bg_writer
                    .find_in_pending(|b| b.txouts.get(outpoint).cloned())
                {
                    tx_outs.insert(*outpoint, tx_out);
                } else {
                    to_fetch.push(*outpoint);
                }
            }
        }

        if !to_fetch.is_empty() {
            let fetched_tx_outs = self.db.read().get_tx_outs(&to_fetch, None)?;
            for (outpoint, tx_out) in fetched_tx_outs {
                tx_outs.insert(outpoint, tx_out);
            }
        }

        Ok(tx_outs)
    }

    fn set_tx_out(
        &mut self,
        outpoint: SerializedOutPoint,
        tx_out: TxOut,
        script_pubkey: ScriptBuf,
    ) {
        if self.settings.index_addresses {
            self.update
                .script_pubkeys_outpoints
                .insert(outpoint.clone(), script_pubkey);
        }

        self.outpoints.put(outpoint, tx_out.clone());
        self.update.txouts.insert(outpoint, tx_out);
    }

    fn set_spent_tx_out(
        &mut self,
        outpoint: &TransactionStateChangeInput,
        spent: SpenderReference,
    ) -> Result<()> {
        let mut tx_out = self.get_tx_out(&outpoint.previous_outpoint)?;

        self.spent_outpoints
            .entry(spent.txid)
            .or_insert(vec![])
            .push(outpoint.clone());

        tx_out.spent = SpentStatus::Spent(spent);

        // Ensure the address index mapping for this outpoint is updated on flush
        // so that a delete is emitted for confirmed spends.
        if self.settings.index_addresses {
            let script_pubkey = tx_out.script_pubkey.clone();
            self.update
                .script_pubkeys_outpoints
                .insert(outpoint.previous_outpoint, script_pubkey);
        }

        self.update
            .txouts
            .insert(outpoint.previous_outpoint, tx_out);
        self.outpoints.pop(&outpoint.previous_outpoint);
        Ok(())
    }

    fn get_transaction(&self, txid: &SerializedTxid) -> Result<Transaction> {
        // 1. Check current in-memory update.
        if let Some(transaction) = self.update.transactions.get(txid) {
            return Ok(transaction.clone());
        }

        // 2. Check pending batches.
        if let Some(tx) = self
            .bg_writer
            .find_in_pending(|b| b.transactions.get(txid).cloned())
        {
            return Ok(tx);
        }

        // 3. Fallback to DB.
        let transaction_raw = self.db.read().get_transaction_raw(txid, None)?;
        let tx: Transaction = consensus::deserialize(&transaction_raw)?;
        Ok(tx)
    }

    fn set_transaction(&mut self, txid: SerializedTxid, transaction: Transaction) {
        self.update.transactions.insert(txid, transaction);
    }

    fn get_transaction_confirming_block(&self, txid: &SerializedTxid) -> Result<BlockId> {
        // 1. Check current in-memory update.
        if let Some(block_id) = self.update.transaction_confirming_block.get(txid) {
            return Ok(block_id.clone());
        }

        // 2. Check pending batches.
        if let Some(block_id) = self
            .bg_writer
            .find_in_pending(|b| b.transaction_confirming_block.get(txid).cloned())
        {
            return Ok(block_id);
        }

        // 3. Fallback to DB.
        let block_id = self.db.read().get_transaction_confirming_block(&txid)?;
        Ok(block_id)
    }

    fn set_transaction_confirming_block(
        &mut self,
        txid: SerializedTxid,
        block_id: crate::models::BlockId,
    ) {
        self.update
            .transaction_confirming_block
            .insert(txid, block_id);
    }

    fn get_rune(
        &mut self,
        rune_id: &RuneId,
    ) -> std::result::Result<crate::models::RuneEntry, StoreError> {
        // 1. Current update.
        if let Some(rune) = self.update.runes.get(rune_id) {
            return Ok(rune.clone());
        }

        // 2. Pending batches.
        if let Some(rune) = self
            .bg_writer
            .find_in_pending(|b| b.runes.get(rune_id).cloned())
        {
            return Ok(rune);
        }

        // 3. Read cache.
        if let Some(rune) = self.runes.get(rune_id) {
            return Ok(rune.clone());
        }

        // 4. DB.
        let rune = self.db.read().get_rune(rune_id)?;
        self.runes.put(*rune_id, rune.clone());
        Ok(rune)
    }

    fn does_rune_exist(&mut self, rune: &Rune) -> Result<()> {
        // 1. Current update.
        if self.update.rune_ids.contains_key(&rune.0) {
            return Ok(());
        }

        // 2. Read cache.
        if self.rune_ids.contains(&rune.0) {
            return Ok(());
        }

        // 2. Pending batches.
        if let Some(_) = self
            .bg_writer
            .find_in_pending(|b| b.rune_ids.get(&rune.0).cloned())
        {
            return Ok(());
        }

        // 4. DB.
        self.db.read().get_rune_id(rune)?;
        self.rune_ids.insert(rune.0);
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

    fn set_inscription(
        &mut self,
        id: titan_types_core::InscriptionId,
        inscription: crate::models::Inscription,
    ) {
        self.update.inscriptions.insert(id, inscription);
    }

    fn set_tx_state_changes(
        &mut self,
        txid: SerializedTxid,
        tx_state_changes: TransactionStateChange,
    ) {
        self.update
            .tx_state_changes
            .insert(txid, tx_state_changes.clone());
    }

    fn add_rune_transaction(&mut self, rune_id: RuneId, txid: SerializedTxid) {
        self.update
            .rune_transactions
            .entry(rune_id)
            .or_insert(vec![])
            .push(txid);
    }
}
