use anyhow::{anyhow, Result};
use borsh::BorshSerialize;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tracing::{debug, error, info, warn};

use arch_sdk::arch_program::{account::AccountMeta, instruction::Instruction, pubkey::Pubkey};
use arch_sdk::helper::get_processed_transaction;
use arch_sdk::helper::sign_and_send_instruction;
use arch_sdk::processed_transaction::{ProcessedTransaction, Status};
use bitcoin::key::Keypair;

use crate::mempool_smart_contract::types::*;
use crate::mempool_smart_contract::utils::*;

/// Process a batch of pending transactions
pub async fn process_pending_transactions(
    endpoint: &str,
    transactions_to_check: &[PendingTransaction],
    pending_transactions: &Arc<RwLock<VecDeque<PendingTransaction>>>,
    unprocessed_updates: &Arc<RwLock<MempoolOracleInstruction>>,
    verbose_logging: bool,
) {
    for tx in transactions_to_check {
        if verbose_logging {
            debug!("Checking status for transaction: {}", tx.txid);
        }

        let processed_tx = match get_processed_transaction(endpoint, &tx.txid).await {
            Ok(tx) => tx,
            Err(e) => {
                debug!("Failed to retrieve transaction {}: {}", tx.txid, e);
                continue;
            }
        };

        // Update transaction status
        update_transaction_status(
            pending_transactions,
            &tx.txid,
            processed_tx,
            verbose_logging,
        );
    }
}

/// Update the status of a transaction based on the processed transaction info
pub fn update_transaction_status(
    pending_transactions: &Arc<RwLock<VecDeque<PendingTransaction>>>,
    txid: &str,
    processed_tx: ProcessedTransaction,
    verbose_logging: bool,
) {
    let mut pending = pending_transactions.write().unwrap();

    // Find the transaction in our pending list
    if let Some(tx) = pending.iter_mut().find(|t| t.txid == txid) {
        // Update status based on blockchain status
        match processed_tx.status {
            Status::Success => {
                if verbose_logging {
                    debug!("Transaction {} succeeded", txid);
                }
                tx.status = TransactionStatus::Success;
            }
            Status::Failure(reason) => {
                error!("Transaction {} failed: {}", txid, reason);
                tx.status = TransactionStatus::Failed(reason);
            }
            _ => {
                // Still pending
                if verbose_logging {
                    debug!("Transaction {} is still pending", txid);
                }
            }
        }
    }
}

/// Clean up transactions that need retry or are expired
pub async fn cleanup_transactions(
    pending_transactions: &Arc<RwLock<VecDeque<PendingTransaction>>>,
    unprocessed_updates: &Arc<RwLock<MempoolOracleInstruction>>,
    verbose_logging: bool,
) {
    let now = get_current_timestamp();
    let mut to_remove = Vec::new();
    let mut to_retry = Vec::new();

    // First pass: identify transactions to remove or retry
    {
        let pending = pending_transactions.read().unwrap();
        for (idx, tx) in pending.iter().enumerate() {
            match tx.status {
                TransactionStatus::Success => {
                    // Successfully processed, can be removed
                    to_remove.push(idx);
                }
                TransactionStatus::Failed(ref reason) => {
                    // Failed transaction - determine if it can be retried
                    if tx.retry_count < MAX_RETRY_ATTEMPTS {
                        to_retry.push(idx);
                    } else {
                        error!(
                            "Transaction {} exceeded max retry attempts: {}",
                            tx.txid, reason
                        );
                        to_remove.push(idx);
                    }
                }
                TransactionStatus::Pending => {
                    // Check if transaction has timed out
                    let timeout = DEFAULT_TRANSACTION_TIMEOUT
                        + (tx.retry_count as u64 * RETRY_BACKOFF_MULTIPLIER);

                    if now > tx.timestamp + timeout {
                        error!("Transaction {} timed out after {}s", tx.txid, timeout);
                        if tx.retry_count < MAX_RETRY_ATTEMPTS {
                            to_retry.push(idx);
                        } else {
                            error!("Transaction exceeded max retry attempts");
                            to_remove.push(idx);
                        }
                    }
                }
                TransactionStatus::Expired => {
                    // Expired transactions should be removed
                    to_remove.push(idx);
                }
            }
        }
    }

    // Sort in reverse order to avoid index shifting issues
    to_remove.sort_unstable_by(|a, b| b.cmp(a));
    to_retry.sort_unstable_by(|a, b| b.cmp(a));

    // Second pass: remove transactions
    {
        let mut pending = pending_transactions.write().unwrap();

        // Process retries
        for idx in to_retry {
            if idx < pending.len() {
                let mut tx = pending.remove(idx).unwrap();
                debug!(
                    "Preparing to retry transaction {}, attempt {}",
                    tx.txid,
                    tx.retry_count + 1
                );

                // Add the transaction instructions back to unprocessed updates
                let mut updates = unprocessed_updates.write().unwrap();
                updates.removed.extend_from_slice(&tx.instruction.removed);
                updates.inserted.extend_from_slice(&tx.instruction.inserted);

                if verbose_logging {
                    debug!(
                        "Re-queued {} removed and {} inserted entries for retry",
                        tx.instruction.removed.len(),
                        tx.instruction.inserted.len()
                    );
                }
            }
        }

        // Remove completed or failed transactions
        for idx in to_remove {
            if idx < pending.len() {
                let tx = pending.remove(idx).unwrap();
                if verbose_logging {
                    debug!(
                        "Removed transaction {} from queue, status: {:?}",
                        tx.txid, tx.status
                    );
                }
            }
        }
    }
}

/// Send transaction with mempool updates to the blockchain
pub async fn send_transaction(
    program_id: &Pubkey,
    mempool_account: &Pubkey,
    signer: &Keypair,
    instruction_data: Vec<u8>,
    endpoint: &str,
    verbose_logging: bool,
) -> Result<String> {
    // Create instruction accounts
    let accounts = vec![
        AccountMeta::new(*mempool_account, false), // Mempool oracle account (not signer)
    ];

    // Create the instruction
    let instruction = Instruction {
        program_id: *program_id,
        accounts,
        data: instruction_data,
    };

    // Sign and send the instruction
    let txid = sign_and_send_instruction(endpoint, signer, &instruction).await?;

    if verbose_logging {
        debug!("Sent transaction with txid: {}", txid);
    }

    Ok(txid)
}

/// Create a pending transaction record
pub fn create_pending_transaction(
    txid: String,
    instruction: MempoolOracleInstruction,
    verbose_logging: bool,
) -> PendingTransaction {
    let now = get_current_timestamp();

    if verbose_logging {
        debug!(
            "Created pending transaction record for {}: {} removed, {} inserted",
            txid,
            instruction.removed.len(),
            instruction.inserted.len()
        );
    }

    PendingTransaction {
        txid,
        timestamp: now,
        retry_count: 0,
        instruction,
        status: TransactionStatus::Pending,
    }
}

/// Add transaction to pending queue
pub fn add_transaction_to_pending_queue(
    pending_transactions: &Arc<RwLock<VecDeque<PendingTransaction>>>,
    transaction: PendingTransaction,
    verbose_logging: bool,
) {
    let mut pending = pending_transactions.write().unwrap();

    // Remove oldest transactions if at capacity
    while pending.len() >= MAX_PENDING_TRANSACTIONS {
        if let Some(oldest) = pending.pop_front() {
            warn!(
                "Dropping oldest pending transaction {} due to queue capacity",
                oldest.txid
            );
        }
    }

    // Add the new transaction
    pending.push_back(transaction);

    if verbose_logging {
        debug!(
            "Added transaction to pending queue. Queue size: {}",
            pending.len()
        );
    }
}

/// Collect pending transactions that need status checking
pub fn collect_pending_transactions(
    pending_transactions: &Arc<RwLock<VecDeque<PendingTransaction>>>,
    verbose_logging: bool,
) -> Vec<PendingTransaction> {
    let pending_read = pending_transactions.read().unwrap();

    // Only collect pending transactions (not success/failed/expired)
    let transactions: Vec<PendingTransaction> = pending_read
        .iter()
        .filter(|tx| matches!(tx.status, TransactionStatus::Pending))
        .cloned()
        .collect();

    if verbose_logging && !transactions.is_empty() {
        debug!(
            "Collected {} pending transactions for status check",
            transactions.len()
        );
    }

    transactions
}
