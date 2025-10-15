use crate::{
    alkanes::store::{AlkanesBatch, AlkanesRocksDBStore},
    db::RocksDB,
};
use bitcoin::{
    consensus::serialize,
    Block as BitcoinBlock,
};
use metashrew_runtime::MetashrewRuntime;
use std::sync::{Arc, Mutex};

const ALKANES_WASM: &[u8] = include_bytes!("../../../vendor/alkanes.wasm");

pub struct AlkanesIndexer {
    runtime: MetashrewRuntime<AlkanesRocksDBStore>,
    batch: Arc<Mutex<AlkanesBatch>>,
}

impl AlkanesIndexer {
    pub fn new(db: Arc<RocksDB>) -> Self {
        let batch = Arc::new(Mutex::new(AlkanesBatch::default()));
        let store = AlkanesRocksDBStore::new(db, batch.clone());
        let runtime = MetashrewRuntime::new(ALKANES_WASM, store).unwrap();
        Self { runtime, batch }
    }

    pub fn index_block(&mut self, block: &BitcoinBlock, height: u64) {
        let mut context = self.runtime.context.lock().unwrap();
        context.block = serialize(block);
        context.height = height as u32;
        drop(context);
        if let Err(e) = self.runtime.run() {
            panic!("AlkanesIndexer: failed to run runtime: {}", e);
        }
    }

    pub fn take_batch(&mut self) -> AlkanesBatch {
        let mut batch = self.batch.lock().unwrap();
        std::mem::take(&mut *batch)
    }
}