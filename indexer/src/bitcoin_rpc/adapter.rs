use crate::bitcoin_rpc::RpcClientPool;
use async_trait::async_trait;
use metashrew_sync::{BitcoinNodeAdapter, BlockInfo, SyncResult, SyncError};

pub struct TitanBitcoinNodeAdapter {
    pool: RpcClientPool,
}

impl TitanBitcoinNodeAdapter {
    pub fn new(pool: RpcClientPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BitcoinNodeAdapter for TitanBitcoinNodeAdapter {
    async fn get_tip_height(&self) -> SyncResult<u32> {
        let client = self.pool.get().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        client.get_blockchain_info().map(|info| info.blocks as u32).map_err(|e| SyncError::BitcoinNode(e.to_string()))
    }

    async fn get_block_hash(&self, height: u32) -> SyncResult<Vec<u8>> {
        let client = self.pool.get().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        client.get_block_hash(height as u64).map(|hash| hash.to_vec()).map_err(|e| SyncError::BitcoinNode(e.to_string()))
    }

    async fn get_block_data(&self, height: u32) -> SyncResult<Vec<u8>> {
        let client = self.pool.get().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        let hash = self.get_block_hash(height).await?;
        let block = client.get_block(&bitcoin::BlockHash::from_slice(&hash).unwrap()).map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        Ok(bitcoin::consensus::serialize(&block))
    }
    async fn get_block_info(&self, height: u32) -> SyncResult<BlockInfo> {
        let client = self.pool.get().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        let hash = self.get_block_hash(height).await?;
        let data = self.get_block_data(height).await?;
        Ok(BlockInfo {
            hash,
            data,
        })
    }

    async fn is_connected(&self) -> bool {
        self.pool.get().is_ok()
    }
}
