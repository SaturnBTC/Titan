use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct BlockTip {
    pub height: u64,
    pub hash: String,
    pub is_at_tip: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Status {
    pub block_tip: BlockTip,
    pub runes_count: u64,
    pub mempool_tx_count: u64,
}
