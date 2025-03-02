use anyhow::{anyhow, Result};
use arch_sdk::arch_program::pubkey::Pubkey;
use bitcoin::Txid;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use tracing::{debug, error, info, warn};

use arch_sdk::helper::account::read_account_info;
use arch_sdk::helper::account::AccountInfoResult;

use crate::mempool_smart_contract::types::*;
use crate::mempool_smart_contract::utils::*;

/// Fetch the current state from the blockchain
pub async fn fetch_state(
    arch_api_endpoint: &str,
    mempool_account: Pubkey,
    verbose_logging: bool,
) -> Result<MempoolOracleState> {
    // Fetch account data from the blockchain
    let account_info: AccountInfoResult =
        read_account_info(arch_api_endpoint, mempool_account)?;

    if verbose_logging {
        debug!("Account info retrieved: {} bytes", account_info.data.len());
    }

    // Deserialize account data into mempool state
    let state: MempoolOracleState = borsh::from_slice(&account_info.data)
        .map_err(|e| anyhow!("Failed to deserialize mempool state: {}", e))?;

    if verbose_logging {
        debug!(
            "Current mempool state: {} entries, last updated: {}",
            state.entries.len(),
            state.last_updated
        );
    }

    Ok(state)
}

/// Initialize state and processed txids from blockchain data
pub fn initialize_state_and_processed_txids(
    current_oracle_state: &Arc<RwLock<Option<MempoolOracleState>>>,
    processed_txids: &Arc<RwLock<HashSet<Txid>>>,
    state: MempoolOracleState,
    verbose_logging: bool,
) {
    // Update the current state
    let mut oracle_state_write = current_oracle_state.write().unwrap();
    *oracle_state_write = Some(state.clone());

    // Initialize the set of processed txids from the state
    if verbose_logging {
        debug!(
            "Initializing processed txids set with {} entries",
            state.entries.len()
        );
    }

    let mut processed_txids_write = processed_txids.write().unwrap();
    for entry in &state.entries {
        let mut txid_bytes = [0u8; 32];
        txid_bytes.copy_from_slice(&entry.txid);
        let txid = Txid::from_byte_array(txid_bytes);
        processed_txids_write.insert(txid);
    }

    drop(processed_txids_write);
    drop(oracle_state_write);

    if verbose_logging {
        debug!("State initialized successfully");
    }
}

/// Initialize empty state when no data is available
pub fn initialize_empty_state(
    current_oracle_state: &Arc<RwLock<Option<MempoolOracleState>>>,
    verbose_logging: bool,
) {
    if verbose_logging {
        debug!("Initializing empty state");
    }

    let empty_state = MempoolOracleState {
        entries: Vec::new(),
        last_updated: get_current_timestamp(),
    };

    let mut oracle_state_write = current_oracle_state.write().unwrap();
    *oracle_state_write = Some(empty_state);

    drop(oracle_state_write);

    if verbose_logging {
        debug!("Empty state initialized");
    }
}
