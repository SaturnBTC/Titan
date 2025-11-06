use async_trait::async_trait;
use metashrew_sync::{RuntimeAdapter, AtomicBlockResult, ViewCall, ViewResult, PreviewCall, SyncResult, RuntimeStats};


pub struct TitanRuntimeAdapter;

#[async_trait]
impl RuntimeAdapter for TitanRuntimeAdapter {
    async fn process_block(&mut self, height: u32, block_data: &[u8]) -> SyncResult<()> {
        Ok(())
    }

    async fn process_block_atomic(
        &mut self,
        height: u32,
        block_data: &[u8],
        block_hash: &[u8],
    ) -> SyncResult<AtomicBlockResult> {
        Err(metashrew_sync::SyncError::Runtime("not implemented".to_string()))
    }

    async fn execute_view(&self, call: ViewCall) -> SyncResult<ViewResult> {
        Err(metashrew_sync::SyncError::Runtime("not implemented".to_string()))
    }

    async fn execute_preview(&self, call: PreviewCall) -> SyncResult<ViewResult> {
        Err(metashrew_sync::SyncError::Runtime("not implemented".to_string()))
    }

    async fn get_state_root(&self, height: u32) -> SyncResult<Vec<u8>> {
        Err(metashrew_sync::SyncError::Runtime("not implemented".to_string()))
    }

    async fn refresh_memory(&mut self) -> SyncResult<()> {
        Ok(())
    }

    async fn is_ready(&self) -> bool {
        true
    }

    async fn get_stats(&self) -> SyncResult<RuntimeStats> {
        Ok(RuntimeStats {
            memory_usage_bytes: 0,
            blocks_processed: 0,
            last_refresh_height: None,
        })
    }
}
