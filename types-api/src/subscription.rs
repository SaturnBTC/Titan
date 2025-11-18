use {
    borsh::{BorshDeserialize, BorshSerialize},
    serde::{Deserialize, Serialize},
    titan_types_core::EventType,
    uuid::Uuid,
};

#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub endpoint: String,
    pub event_types: Vec<EventType>,
    pub last_success_epoch_secs: u64,
}

/// The expected subscription request from the TCP client.
/// For example, the client should send:
///   {"subscribe": ["RuneEtched", "RuneMinted"]}
#[derive(Debug, Serialize, Deserialize)]
pub struct TcpSubscriptionRequest {
    pub subscribe: Vec<EventType>,
}
