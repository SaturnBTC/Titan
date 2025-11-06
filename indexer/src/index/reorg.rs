use crate::{db::RocksDB, index::{updater::Updater, Settings, store::StoreError}};
use std::sync::Arc;

pub struct ReorgManager {
    db: Arc<RocksDB>,
    updater: Arc<Updater>,
    settings: Arc<Settings>,
}

impl ReorgManager {
    pub fn new(db: Arc<RocksDB>, updater: Arc<Updater>, settings: Arc<Settings>) -> Self {
        Self {
            db,
            updater,
            settings,
        }
    }

    pub fn handle_reorg(&self, height: u32) -> Result<(), StoreError> {
        let current_height = self.db.get_block_count()? as u32;
        let block_hashes = self.db.get_block_hashes_by_height(height as u64, current_height as u64)?;
        let blocks = self.db.get_blocks_by_hashes(&block_hashes)?;
        let txids: Vec<_> = blocks.values().flat_map(|block| block.txdata.iter().map(|tx| tx.txid())).collect();
        self.updater.revert_transactions(&txids, false)?;
        self.db.set_block_count(height as u64)?;
        Ok(())
    }
}
