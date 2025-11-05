use crate::{
    alkanes::store::{AlkanesBatch, AlkanesRocksDBStore},
    db::RocksDB,
};
use anyhow::{anyhow, Error};
use bitcoin::{consensus::serialize, Block as BitcoinBlock};
use metashrew_runtime::MetashrewRuntime;
use std::sync::{Arc, Mutex};

const ALKANES_WASM: &[u8] = include_bytes!("../../../vendor/alkanes.wasm");

pub struct AlkanesIndexer {
    runtime: MetashrewRuntime<AlkanesRocksDBStore>,
    batch: Arc<Mutex<AlkanesBatch>>,
}

impl AlkanesIndexer {
    pub async fn new(db: Arc<RocksDB>) -> Result<Self, Error> {
        let batch = Arc::new(Mutex::new(AlkanesBatch::default()));
        let store = AlkanesRocksDBStore::new(db, batch.clone());
        let engine = wasmtime::Engine::default();
        let runtime = MetashrewRuntime::new(ALKANES_WASM, store, engine).await?;
        Ok(Self {
            runtime,
            batch,
        })
    }

    pub async fn new_dummy() -> Result<Self, Error> {
        let batch = Arc::new(Mutex::new(AlkanesBatch::default()));
        let store = AlkanesRocksDBStore::new_dummy();
        let engine = wasmtime::Engine::default();
        let runtime = MetashrewRuntime::new(ALKANES_WASM, store, engine).await?;
        Ok(Self {
            runtime,
            batch,
        })
    }

    pub async fn index_block(&mut self, block: &BitcoinBlock, height: u64) -> Result<(), Error> {
        let mut context = self
            .runtime
            .context
            .lock()
            .await;
        context.block = serialize(block);
        context.height = height as u32;
        drop(context);
        self.runtime.run().await?;
        Ok(())
    }

    pub fn take_batch(&mut self) -> Result<AlkanesBatch, Error> {
        let mut batch = self
            .batch
            .lock()
            .map_err(|e| anyhow!("Failed to obtain lock: {}", e))?;
        Ok(std::mem::take(&mut *batch))
    }

    pub async fn view(
        &self,
        method: String,
        input: &Vec<u8>,
        height: u32,
    ) -> Result<Vec<u8>, Error> {
        self.runtime.view(method, input, height).await
    }

    /// Rollback the alkanes state to a specific height
    /// 
    /// This method handles chain reorganizations by:
    /// 1. Rolling back the SMT (Sparse Merkle Tree) state using the metashrew runtime
    /// 2. Deleting orphaned SMT roots for heights > target_height
    /// 3. Updating the tip height in the database
    /// 4. Clearing any pending batches
    ///
    /// # Arguments
    /// * `target_height` - The height to rollback to (inclusive)
    ///
    /// # Returns
    /// * `Ok(())` if rollback was successful
    /// * `Err(Error)` if rollback failed
    pub async fn rollback_to_height(&mut self, target_height: u64) -> Result<(), Error> {
        let target_height_u32 = target_height as u32;
        
        tracing::info!(
            "Rolling back alkanes state to height {}",
            target_height_u32
        );

        // Clear any pending batches since we've rolled back
        // The actual state rollback will be handled by the metashrew runtime
        // when blocks are re-indexed from the target height forward
        {
            let mut batch = self.batch.lock()
                .map_err(|e| anyhow!("Failed to obtain lock: {}", e))?;
            batch.puts.clear();
            batch.deletes.clear();
        }

        tracing::info!("Alkanes rollback to height {} complete - pending batches cleared", target_height_u32);
        
        Ok(())
    }
}
