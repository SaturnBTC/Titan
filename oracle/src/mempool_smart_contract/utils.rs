use std::time::{SystemTime, UNIX_EPOCH};
use arch_sdk::arch_program::pubkey::Pubkey;
use anyhow::Result;
use tracing::{info, error, debug, warn};

use crate::mempool_smart_contract::types::*;

/// Configuration for the smart contract updater
#[derive(Debug, Clone)]
pub struct SmartContractConfig {
    /// Program ID of the mempool oracle contract
    pub program_id: Pubkey,
    /// Account that stores the mempool data
    pub mempool_account: Pubkey,
    /// Maximum number of entries to send in a single update
    pub max_batch_size: usize,
    /// Whether to log detailed information
    pub verbose_logging: bool,
    /// Arch API endpoint for account queries
    pub arch_api_endpoint: String,
}

impl Default for SmartContractConfig {
    fn default() -> Self {
        Self {
            program_id: Pubkey::from([0; 32]),  // Must be set by user
            mempool_account: Pubkey::from([0; 32]),  // Must be set by user
            max_batch_size: 50,
            verbose_logging: false,
            arch_api_endpoint: "http://localhost:8899".to_string(), // Default Arch API endpoint
        }
    }
}

/// Get the current unix timestamp
pub fn get_current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| {
            error!("SystemTime before UNIX EPOCH!");
            std::time::Duration::from_secs(0)
        })
        .as_secs()
}

/// Formats a byte array as a hex string
pub fn bytes_to_hex_string(bytes: &[u8]) -> String {
    bytes.iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

/// Converts mempool entries to a format suitable for the smart contract
pub fn convert_mempool_entries(entries: &[MempoolEntry]) -> Result<Vec<u8>> {
    let instruction = MempoolOracleInstruction {
        removed: Vec::new(),
        inserted: entries.to_vec(),
    };
    
    // Serialize the instruction
    let data = borsh::to_vec(&instruction)?;
    Ok(data)
} 