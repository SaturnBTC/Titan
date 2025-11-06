
use crate::{alkanes::store::AlkanesBatch, db::{RocksDB, RocksDBError}, index::updater::rollback::Rollback};
use async_trait::async_trait;
use bitcoin::BlockHash;
use metashrew_sync::{StorageAdapter, StorageStats, SyncError, SyncResult, RuntimeAdapter, AtomicBlockResult, ViewCall, PreviewCall};
use rocksdb::{DBWithThreadMode, MultiThreaded};
use std::sync::Arc;
use borsh::BorshDeserialize;

pub struct TitanStorageAdapter {
    db: Arc<RocksDB>,
    settings: Arc<Settings>,
}

impl TitanStorageAdapter {
    pub fn new(db: Arc<RocksDB>, settings: Arc<Settings>) -> Self {
        Self { db, settings }
    }
}

#[async_trait]
impl StorageAdapter for TitanStorageAdapter {
    type Settings = crate::index::Settings;
    async fn get_indexed_height(&self) -> SyncResult<u32> {
        self.db.get_block_count().map(|h| h as u32).map_err(|e| SyncError::Storage(e.to_string()))
    }

    async fn set_indexed_height(&mut self, height: u32) -> SyncResult<()> {
        self.db.set_block_count(height as u64).map_err(|e| SyncError::Storage(e.to_string()))
    }

    async fn store_block_hash(&mut self, height: u32, hash: &[u8]) -> SyncResult<()> {
        let block_hash = BlockHash::from_slice(hash).map_err(|e| SyncError::Storage(e.to_string()))?;
        self.db.store_block_hash(height as u64, &block_hash).map_err(|e| SyncError::Storage(e.to_string()))
    }

    async fn get_block_hash(&self, height: u32) -> SyncResult<Option<Vec<u8>>> {
        match self.db.get_block_hash(height as u64) {
            Ok(hash) => Ok(Some(hash.to_vec())),
            Err(RocksDBError::NotFound(_)) => Ok(None),
            Err(e) => Err(SyncError::Storage(e.to_string())),
        }
    }

    async fn store_state_root(&mut self, height: u32, root: &[u8]) -> SyncResult<()> {
        Ok(())
    }

    async fn get_state_root(&self, height: u32) -> SyncResult<Option<Vec<u8>>> {
        Ok(None)
    }

    async fn rollback_to_height(&mut self, height: u32) -> SyncResult<()> {
        let current_height = self.get_indexed_height().await?;
        let block_hashes = self.db.get_block_hashes_by_height(height as u64, current_height as u64).map_err(|e| SyncError::Storage(e.to_string()))?;
        let blocks = self.db.get_blocks_by_hashes(&block_hashes).map_err(|e| SyncError::Storage(e.to_string()))?;
        let txids: Vec<_> = blocks.values().flat_map(|block| block.txdata.iter().map(|tx| tx.txid())).collect();
        let mut rollback_updater = Rollback::new(&self.db, (*self.settings).clone().into(), false).map_err(|e| SyncError::Storage(e.to_string()))?;
        rollback_updater.revert_transactions(&txids).map_err(|e| SyncError::Storage(e.to_string()))?;
        self.set_indexed_height(height).await
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn get_stats(&self) -> SyncResult<StorageStats> {
        Ok(StorageStats::default())
    }

    async fn get_db_handle(&self) -> SyncResult<Arc<DBWithThreadMode<MultiThreaded>>> {
        Ok(self.db.db_handle())
    }

    fn get_settings(&self) -> &Self::Settings {
        &self.settings
    }
}

pub struct TitanRuntimeAdapter;

#[async_trait]
impl RuntimeAdapter for TitanRuntimeAdapter {
    async fn process_block_atomic(&mut self, height: u32, block_data: &[u8], block_hash: &[u8]) -> SyncResult<AtomicBlockResult> {
        Err(SyncError::Runtime("not implemented".to_string()))
    }

    async fn process_block(&mut self, height: u32, block_data: &[u8]) -> SyncResult<()> {
        Ok(())
    }

    async fn get_state_root(&self, height: u32) -> SyncResult<Vec<u8>> {
        Ok(vec![])
    }

    async fn execute_view(&self, call: ViewCall) -> SyncResult<ViewCall> {
        Err(SyncError::Runtime("not implemented".to_string()))
    }

    async fn execute_preview(&self, call: PreviewCall) -> SyncResult<PreviewCall> {
        Err(SyncError::Runtime("not implemented".to_string()))
    }

    async fn is_ready(&self) -> bool {
        true
    }

    async fn refresh_memory(&mut self) -> SyncResult<()> {
        Ok(())
    }
}
