use anyhow::Result;
use metashrew_runtime::{BatchLike, KeyValueStoreLike};
use rocksdb::{WriteBatch, IteratorMode, Direction};
use std::sync::{Arc, Mutex};

use crate::db::{RocksDB, RocksDBError};

const ALKANES_CF: &str = "alkanes";

use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Default)]
pub struct AlkanesBatch {
    pub puts: FxHashMap<Vec<u8>, Vec<u8>>,
    pub deletes: FxHashSet<Vec<u8>>,
}

impl BatchLike for AlkanesBatch {
    fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, key: K, value: V) {
        self.puts.insert(key.as_ref().to_vec(), value.as_ref().to_vec());
    }

    fn delete<K: AsRef<[u8]>>(&mut self, key: K) {
        self.deletes.insert(key.as_ref().to_vec());
    }
    fn default() -> Self {
        AlkanesBatch {
            puts: FxHashMap::default(),
            deletes: FxHashSet::default(),
        }
    }
}

#[derive(Clone)]
pub struct AlkanesRocksDBStore {
    db: Arc<RocksDB>,
    batch: Arc<Mutex<AlkanesBatch>>,
}

impl AlkanesRocksDBStore {
    pub fn new(db: Arc<RocksDB>, batch: Arc<Mutex<AlkanesBatch>>) -> Self {
        Self { db, batch }
    }
}

impl KeyValueStoreLike for AlkanesRocksDBStore {
    type Error = RocksDBError;
    type Batch = AlkanesBatch;

    fn write(&mut self, batch: Self::Batch) -> Result<(), Self::Error> {
        let mut shared_batch = self.batch.lock().unwrap();
        shared_batch.puts.extend(batch.puts);
        shared_batch.deletes.extend(batch.deletes);
        Ok(())
    }

    fn get<K: AsRef<[u8]>>(&mut self, key: K) -> Result<Option<Vec<u8>>, Self::Error> {
        self.db.get_cf(ALKANES_CF, key)
    }

    fn get_immutable<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>, Self::Error> {
        self.db.get_cf(ALKANES_CF, key)
    }

    fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, key: K, value: V) -> Result<(), Self::Error> {
        self.db.put_cf(ALKANES_CF, key, value)
    }

    fn delete<K: AsRef<[u8]>>(&mut self, key: K) -> Result<(), Self::Error> {
        self.db.delete_cf(ALKANES_CF, key)
    }

    fn scan_prefix<K: AsRef<[u8]>>(
        &self,
        prefix: K,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Self::Error> {
        self.db.scan_prefix(ALKANES_CF, prefix)
    }

    fn create_batch(&self) -> Self::Batch {
        <Self::Batch as Default>::default()
    }

    fn keys<'a>(&'a self) -> Result<Box<dyn Iterator<Item = Vec<u8>> + 'a>, Self::Error> {
        self.db.keys(ALKANES_CF)
    }
}
