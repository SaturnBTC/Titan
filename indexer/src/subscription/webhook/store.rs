use {
    crate::db::{RocksDB, RocksDBError},
    thiserror::Error,
    titan_types_api::Subscription,
    uuid::Uuid,
};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("db error {0}")]
    DB(RocksDBError),
    #[error("not found: {0}")]
    NotFound(String),
}

impl From<RocksDBError> for StoreError {
    fn from(error: RocksDBError) -> Self {
        match error {
            RocksDBError::NotFound(msg) => StoreError::NotFound(msg),
            other => StoreError::DB(other),
        }
    }
}

pub trait Store: Send + Sync {
    // subscriptions
    fn set_subscription(&self, sub: &Subscription) -> Result<(), StoreError>;
    fn update_subscription_last_success(
        &self,
        id: &Uuid,
        last_success: u64,
    ) -> Result<(), StoreError>;
    fn get_subscription(&self, id: &Uuid) -> Result<Subscription, StoreError>;
    fn get_subscriptions(&self) -> Result<Vec<Subscription>, StoreError>;
    fn delete_subscription(&self, id: &Uuid) -> Result<(), StoreError>;
}

impl Store for RocksDB {
    fn set_subscription(&self, sub: &Subscription) -> Result<(), StoreError> {
        Ok(self.set_subscription(sub)?)
    }

    fn update_subscription_last_success(
        &self,
        id: &Uuid,
        last_success: u64,
    ) -> Result<(), StoreError> {
        Ok(self.update_subscription_last_success(id, last_success)?)
    }

    fn get_subscription(&self, id: &Uuid) -> Result<Subscription, StoreError> {
        Ok(self.get_subscription(id)?)
    }

    fn get_subscriptions(&self) -> Result<Vec<Subscription>, StoreError> {
        Ok(self.get_subscriptions()?)
    }

    fn delete_subscription(&self, id: &Uuid) -> Result<(), StoreError> {
        Ok(self.delete_subscription(id)?)
    }
}
