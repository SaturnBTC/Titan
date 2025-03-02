use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// Maximum number of pending transactions to track
pub const MAX_PENDING_TRANSACTIONS: usize = 50;

/// How long to wait before checking transaction status (in seconds)
pub const TRANSACTION_CHECK_INTERVAL: u64 = 5;

/// Maximum retry attempts for a failed transaction
pub const MAX_RETRY_ATTEMPTS: u8 = 3;

/// Default timeout for transaction verification (in seconds)
pub const DEFAULT_TRANSACTION_TIMEOUT: u64 = 60;

/// Backoff multiplier for retries (seconds)
pub const RETRY_BACKOFF_MULTIPLIER: u64 = 2;

/// Instruction for updating the mempool oracle
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct MempoolOracleInstruction {
    pub removed: Vec<[u8; 32]>, // Store Txid as bytes
    pub inserted: Vec<MempoolEntry>,
}

/// MempoolEntry structure for the smart contract
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct MempoolEntry {
    pub txid: [u8; 32],
    pub fee: u16,
    pub vsize: u16,
    pub descendant_count: u8,
    pub ancestor_count: u8,
}

/// Current state of the mempool oracle account
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct MempoolOracleState {
    pub entries: Vec<MempoolEntry>,
    pub last_updated: u64, // Unix timestamp
}

/// Status of a transaction sent to the smart contract
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionStatus {
    Pending,
    Success,
    Failed(String),
    Expired,
}

/// Information about a pending transaction
#[derive(Debug, Clone)]
pub struct PendingTransaction {
    /// Unique ID of the transaction
    pub txid: String,
    /// When the transaction was sent
    pub timestamp: u64,
    /// Number of retry attempts
    pub retry_count: u8,
    /// The instruction data contained in the transaction
    pub instruction: MempoolOracleInstruction,
    /// Current status
    pub status: TransactionStatus,
}
