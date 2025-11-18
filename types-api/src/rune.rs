use {
    serde::{Deserialize, Serialize},
    titan_types_core::{InscriptionId, RuneId, SerializedTxid, SpacedRune},
};

mod serde_str {
    pub use titan_types_core::serde_str::{deserialize, serialize};
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MintResponse {
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub mintable: bool,
    #[serde(with = "serde_str")]
    pub cap: u128,
    #[serde(with = "serde_str")]
    pub amount: u128,
    #[serde(with = "serde_str")]
    pub mints: u128,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuneResponse {
    pub id: RuneId,
    pub block: u64,
    #[serde(with = "serde_str")]
    pub burned: u128,
    pub divisibility: u8,
    pub etching: SerializedTxid,
    pub number: u64,
    #[serde(with = "serde_str")]
    pub premine: u128,
    #[serde(with = "serde_str")]
    pub supply: u128,
    #[serde(with = "serde_str")]
    pub max_supply: u128,
    pub spaced_rune: SpacedRune,
    pub symbol: Option<char>,
    pub mint: Option<MintResponse>,
    #[serde(with = "serde_str")]
    pub burns: u128,
    #[serde(with = "serde_str")]
    pub pending_burns: u128,
    #[serde(with = "serde_str")]
    pub pending_mints: u128,
    pub inscription_id: Option<InscriptionId>,
    pub timestamp: u64,
    pub turbo: bool,
}
