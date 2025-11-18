use {
    super::{
        process_event,
        store::{Store, StoreError},
    },
    reqwest::Client,
    std::sync::Arc,
    titan_types_api::Subscription,
    titan_types_core::Event,
    uuid::Uuid,
};

pub struct SubscriptionManager {
    store: Arc<dyn Store>,
    client: Client,
}

impl SubscriptionManager {
    pub fn new(store: Arc<dyn Store>) -> Self {
        let client = Client::new();
        Self { store, client }
    }

    pub fn add_subscription(&self, subscription: &Subscription) -> Result<(), StoreError> {
        self.store.set_subscription(subscription)
    }

    pub fn delete_subscription(&self, id: &Uuid) -> Result<(), StoreError> {
        self.store.delete_subscription(id)
    }

    pub fn get_subscriptions(&self) -> Result<Vec<Subscription>, StoreError> {
        self.store.get_subscriptions()
    }

    pub fn get_subscription(&self, id: &Uuid) -> Result<Subscription, StoreError> {
        self.store.get_subscription(id)
    }

    pub async fn broadcast(&self, event: &Event) -> Result<(), Box<dyn std::error::Error>> {
        process_event(&self.store, &self.client, event).await
    }
}
