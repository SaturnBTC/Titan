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
        Ok(Self { runtime, batch })
    }

    pub async fn new_dummy() -> Result<Self, Error> {
        let batch = Arc::new(Mutex::new(AlkanesBatch::default()));
        let store = AlkanesRocksDBStore::new_dummy();
        let engine = wasmtime::Engine::default();
        let runtime = MetashrewRuntime::new(ALKANES_WASM, store, engine).await?;
        Ok(Self { runtime, batch })
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
}
