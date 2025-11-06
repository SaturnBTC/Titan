use {
    crate::db::RocksDB,
    std::sync::{Arc, RwLock},
};

pub struct StoreWithLock(RwLock<Arc<RocksDB>>);

impl StoreWithLock {
    pub fn new(db: Arc<RocksDB>) -> Self {
        Self(RwLock::new(db))
    }

    pub fn read(&self) -> Arc<RocksDB> {
        let result = self.0.read();
        match result {
            Ok(db) => db.clone(),
            Err(e) => panic!("failed to read db: {e}"),
        }
    }

    pub fn write(&self) -> Arc<RocksDB> {
        let result = self.0.write();
        match result {
            Ok(db) => db.clone(),
            Err(e) => panic!("failed to write db: {e}"),
        }
    }
}
