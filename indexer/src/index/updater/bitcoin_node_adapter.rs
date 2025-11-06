
use async_trait::async_trait;
use metashrew_sync::{BitcoinNodeAdapter, BlockInfo, ChainTip, SyncError, SyncResult};
use crate::bitcoin_rpc::{RpcClientPool, RpcClientError};

pub struct TitanBitcoinNodeAdapter {
    rpc_client_pool: RpcClientPool,
}

impl TitanBitcoinNodeAdapter {
    pub fn new(rpc_client_pool: RpcClientPool) -> Self {
        Self { rpc_client_pool }
    }
}

#[async_trait]
impl BitcoinNodeAdapter for TitanBitcoinNodeAdapter {
    async fn get_tip_height(&self) -> SyncResult<u32> {
        let client = self.rpc_client_pool.get().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        let info = client.get_blockchain_info().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        Ok(info.blocks)
    }

    async fn get_block_hash(&self, height: u32) -> SyncResult<Vec<u8>> {
        let client = self.rpc_client_pool.get().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        let hash = client.get_block_hash(height as u64).map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        Ok(hash.to_vec())
    }

    async fn get_block_data(&self, height: u32) -> SyncResult<Vec<u8>> {
        let client = self.rpc_client_pool.get().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        let hash = client.get_block_hash(height as u64).map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        let block = client.get_block(&hash).map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        Ok(bitcoin::consensus::encode::serialize(&block))
    }

    async fn get_block_info(&self, height: u32) -> SyncResult<BlockInfo> {
        let client = self.rpc_client_pool.get().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        let hash = client.get_block_hash(height as u64).map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        let block = client.get_block(&hash).map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        Ok(BlockInfo {
            hash: hash.to_vec(),
            height,
            data: bitcoin::consensus::encode::serialize(&block),
        })
    }

    async fn get_chain_tip(&self) -> SyncResult<ChainTip> {
        let client = self.rpc_client_pool.get().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        let info = client.get_blockchain_info().map_err(|e| SyncError::BitcoinNode(e.to_string()))?;
        Ok(ChainTip {
            height: info.blocks,
            hash: info.best_block_hash.to_vec(),
        })
    }

    async fn is_connected(&self) -> bool {
        self.rpc_client_pool.get().is_ok()
    }
}
