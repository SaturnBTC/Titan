use anyhow::Result;
use metashrew_runtime::{BatchLike, KeyValueStoreLike};
use rocksdb::{Direction, IteratorMode};
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
        self.puts
            .insert(key.as_ref().to_vec(), value.as_ref().to_vec());
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
    db: Option<Arc<RocksDB>>,
    batch: Arc<Mutex<AlkanesBatch>>,
}

impl AlkanesRocksDBStore {
    pub fn new(db: Arc<RocksDB>, batch: Arc<Mutex<AlkanesBatch>>) -> Self {
        Self {
            db: Some(db),
            batch,
        }
    }
    pub fn new_dummy() -> Self {
        Self {
            db: None,
            batch: Arc::new(Mutex::new(<AlkanesBatch as Default>::default())),
        }
    }
}

impl KeyValueStoreLike for AlkanesRocksDBStore {
    type Error = RocksDBError;
    type Batch = AlkanesBatch;

    fn write(&mut self, batch: Self::Batch) -> Result<(), Self::Error> {
        let mut shared_batch = self.batch.lock().map_err(|_| RocksDBError::LockPoisoned)?;
        shared_batch.puts.extend(batch.puts);
        shared_batch.deletes.extend(batch.deletes);
        Ok(())
    }

    fn get<K: AsRef<[u8]>>(&mut self, key: K) -> Result<Option<Vec<u8>>, Self::Error> {
        let batch = self.batch.lock().map_err(|_| RocksDBError::LockPoisoned)?;
        if let Some(value) = batch.puts.get(key.as_ref()) {
            return Ok(Some(value.clone()));
        }
        if batch.deletes.contains(key.as_ref()) {
            return Ok(None);
        }
        if let Some(db) = &self.db {
            let cf = db.cf_handle(ALKANES_CF)?;
            Ok(db.get_cf(&cf, key)?)
        } else {
            Ok(None)
        }
    }

    fn get_immutable<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>, Self::Error> {
        let batch = self.batch.lock().map_err(|_| RocksDBError::LockPoisoned)?;
        if let Some(value) = batch.puts.get(key.as_ref()) {
            return Ok(Some(value.clone()));
        }
        if batch.deletes.contains(key.as_ref()) {
            return Ok(None);
        }
        if let Some(db) = &self.db {
            let cf = db.cf_handle(ALKANES_CF)?;
            Ok(db.get_cf(&cf, key)?)
        } else {
            Ok(None)
        }
    }

    fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, key: K, value: V) -> Result<(), Self::Error> {
        let mut batch = self.batch.lock().map_err(|_| RocksDBError::LockPoisoned)?;
        batch
            .puts
            .insert(key.as_ref().to_vec(), value.as_ref().to_vec());
        Ok(())
    }

    fn delete<K: AsRef<[u8]>>(&mut self, key: K) -> Result<(), Self::Error> {
        let mut batch = self.batch.lock().map_err(|_| RocksDBError::LockPoisoned)?;
        batch.deletes.insert(key.as_ref().to_vec());
        Ok(())
    }

    fn scan_prefix<K: AsRef<[u8]>>(
        &self,
        prefix: K,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Self::Error> {
        if let Some(db) = &self.db {
            let cf = db.cf_handle(ALKANES_CF)?;
            let mut results = vec![];
            let iter = db.iterator_cf(&cf, IteratorMode::From(prefix.as_ref(), Direction::Forward));
            for item in iter {
                let (key, value) = item?;
                if !key.starts_with(prefix.as_ref()) {
                    break;
                }
                results.push((key.to_vec(), value.to_vec()));
            }
            Ok(results)
        } else {
            Ok(vec![])
        }
    }

    fn create_batch(&self) -> Self::Batch {
        <Self::Batch as Default>::default()
    }

    fn keys<'a>(&'a self) -> Result<Box<dyn Iterator<Item = Vec<u8>> + 'a>, Self::Error> {
        if let Some(db) = &self.db {
            let cf = db.cf_handle(ALKANES_CF)?;
            let iter = db.iterator_cf(&cf, IteratorMode::Start);
            let keys = iter.map(|item| item.unwrap().0.to_vec());
            Ok(Box::new(keys))
        } else {
            Ok(Box::new(std::iter::empty()))
        }
    }
}
