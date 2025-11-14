use {
    crate::{
        index::{store::Store, StoreError},
        models::{BatchRollback, RuneEntry},
    },
    bitcoin::ScriptBuf,
    ordinals::Rune,
    rustc_hash::FxHashMap as HashMap,
    std::sync::Arc,
    titan_types::RuneId,
    titan_types::{InscriptionId, SerializedOutPoint, SerializedTxid, TxOut},
    tracing::info,
};

type Result<T> = std::result::Result<T, StoreError>;

#[derive(Default)]
pub struct TempCache {
    txouts: HashMap<SerializedOutPoint, TxOut>,
}

pub struct RollbackCache<'a> {
    db: &'a Arc<dyn Store + Send + Sync>,
    update: BatchRollback,
    temp_cache: TempCache,
    pub mempool: bool,
}

impl<'a> RollbackCache<'a> {
    pub fn new(db: &'a Arc<dyn Store + Send + Sync>, mempool: bool) -> Result<Self> {
        let runes_count = db.get_runes_count()?;

        Ok(Self {
            db,
            update: BatchRollback::new(runes_count),
            temp_cache: TempCache::default(),
            mempool,
        })
    }

    pub fn precache_tx_outs(&mut self, outpoints: &[SerializedOutPoint]) -> Result<()> {
        let tx_outs = self.db.get_tx_outs(outpoints, Some(self.mempool))?;
        self.temp_cache.txouts.extend(tx_outs);
        Ok(())
    }

    pub fn precache_runes(&mut self, rune_ids: &Vec<RuneId>) -> Result<()> {
        let runes = self.db.get_runes_by_ids(rune_ids)?;
        self.update.rune_entry.extend(runes);
        Ok(())
    }

    pub fn get_tx_out(&mut self, outpoint: &SerializedOutPoint) -> Result<TxOut> {
        Ok(self
            .temp_cache
            .txouts
            .get(outpoint)
            .ok_or(StoreError::NotFound(outpoint.to_string()))?
            .clone())
    }

    pub fn set_tx_out(&mut self, outpoint: SerializedOutPoint, tx_out: TxOut) {
        self.update.txouts.insert(outpoint, tx_out);
    }

    pub fn decrement_runes_count(&mut self) {
        self.update.runes_count -= 1;
    }

    pub fn get_rune(&mut self, rune_id: &RuneId) -> Option<RuneEntry> {
        self.update.rune_entry.get(rune_id).cloned()
    }

    pub fn set_rune(&mut self, rune_id: RuneId, rune_entry: RuneEntry) {
        self.update.rune_entry.insert(rune_id, rune_entry);
    }

    pub fn add_tx_to_delete(&mut self, txid: SerializedTxid) {
        self.update.txs_to_delete.push(txid);
    }

    pub fn add_outpoint_to_delete(&mut self, outpoint: SerializedOutPoint) {
        self.update.outpoints_to_delete.push(outpoint);
    }

    pub fn add_rune_to_delete(&mut self, rune_id: RuneId) {
        self.update.runes_to_delete.push(rune_id);
    }

    pub fn add_rune_id_to_delete(&mut self, rune_id: Rune) {
        self.update.runes_ids_to_delete.push(rune_id);
    }

    pub fn add_rune_number_to_delete(&mut self, rune_number: u64) {
        self.update.rune_numbers_to_delete.push(rune_number);
    }

    pub fn add_inscription_to_delete(&mut self, inscription_id: InscriptionId) {
        self.update.inscriptions_to_delete.push(inscription_id);
    }

    pub fn add_delete_all_rune_transactions(&mut self, rune_id: RuneId) {
        self.update.delete_all_rune_transactions.push(rune_id);
    }

    pub fn get_outpoints_to_script_pubkey(
        &self,
        outpoints: &[SerializedOutPoint],
        optimistic: bool,
    ) -> Result<HashMap<SerializedOutPoint, ScriptBuf>> {
        let script_pubkeys =
            self.db
                .get_outpoints_to_script_pubkey(outpoints, Some(self.mempool), optimistic)?;
        Ok(script_pubkeys)
    }

    pub fn set_script_pubkey_entries(
        &mut self,
        script_pubkey_entry: HashMap<ScriptBuf, (Vec<SerializedOutPoint>, Vec<SerializedOutPoint>)>,
    ) {
        self.update.script_pubkey_entry = script_pubkey_entry;
    }

    pub fn add_prev_outpoint_to_delete(&mut self, outpoints: &[SerializedOutPoint]) {
        self.update.prev_outpoints_to_delete.extend(outpoints);
    }

    pub fn flush(&mut self) -> Result<()> {
        info!("Flushing rollback cache: {}", self.update);
        self.db.batch_rollback(&self.update, self.mempool)?;
        Ok(())
    }
}
