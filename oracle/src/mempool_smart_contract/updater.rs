use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{anyhow, Result};
use bitcoin::hashes::Hash;
use bitcoin::key::{Keypair, UntweakedKeypair};
use bitcoin::secp256k1::SecretKey;
use bitcoin::Txid;
use titan_types::MempoolEntry as TitanMempoolEntry;
use tracing::{debug, error, info, warn};

use crate::mempool_smart_contract::state;
use crate::mempool_smart_contract::transaction;
use crate::mempool_smart_contract::types::*;
use crate::mempool_smart_contract::utils::*;

/// Handles updating the smart contract with mempool data
pub struct MempoolSmartContractUpdater {
    config: SmartContractConfig,
    signer: Keypair,
    processed_txids: Arc<RwLock<HashSet<Txid>>>,
    current_oracle_state: Arc<RwLock<Option<MempoolOracleState>>>,
    pending_transactions: Arc<RwLock<VecDeque<PendingTransaction>>>,
    unprocessed_updates: Arc<RwLock<MempoolOracleInstruction>>,
}

impl MempoolSmartContractUpdater {
    /// Create a new smart contract updater
    pub fn new(config: SmartContractConfig) -> Result<Self, Box<dyn std::error::Error>> {
        // Get the private key from the environment variable
        let private_key_hex = env::var("MEMPOOL_UPDATER_PRIVATE_KEY")
            .map_err(|_| "MEMPOOL_UPDATER_PRIVATE_KEY environment variable not set")?;

        let secret_key = SecretKey::from_slice(&hex::decode(&private_key_hex)?)
            .map_err(|e| format!("Invalid private key: {}", e))?;

        let signer = Keypair::from_secret_key(&bitcoin::secp256k1::Secp256k1::new(), &secret_key);

        let updater = Self {
            config,
            signer,
            processed_txids: Arc::new(RwLock::new(HashSet::new())),
            current_oracle_state: Arc::new(RwLock::new(None)),
            pending_transactions: Arc::new(RwLock::new(VecDeque::new())),
            unprocessed_updates: Arc::new(RwLock::new(MempoolOracleInstruction {
                removed: Vec::new(),
                inserted: Vec::new(),
            })),
        };

        // Start transaction status checker in background
        updater.start_transaction_status_checker();

        Ok(updater)
    }

    /// Start a background task to check transaction status
    fn start_transaction_status_checker(&self) {
        let pending_transactions = self.pending_transactions.clone();
        let unprocessed_updates = self.unprocessed_updates.clone();
        let endpoint = self.config.arch_api_endpoint.clone();
        let verbose_logging = self.config.verbose_logging;

        tokio::spawn(async move {
            loop {
                // Sleep first to give time for transactions to be processed
                tokio::time::sleep(Duration::from_secs(TRANSACTION_CHECK_INTERVAL)).await;

                let transactions_to_check = transaction::collect_pending_transactions(
                    &pending_transactions,
                    verbose_logging,
                );

                if transactions_to_check.is_empty() {
                    continue;
                }

                // Check status of each transaction
                transaction::process_pending_transactions(
                    &endpoint,
                    &transactions_to_check,
                    &pending_transactions,
                    &unprocessed_updates,
                    verbose_logging,
                )
                .await;

                // Process expired transactions and cleanup
                transaction::cleanup_transactions(
                    &pending_transactions,
                    &unprocessed_updates,
                    verbose_logging,
                )
                .await;
            }
        });

        info!("Started transaction status checker task");
    }

    /// Initialize the smart contract updater
    pub async fn initialize(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.config.verbose_logging {
            debug!("Initializing mempool smart contract updater");
        }

        // Try to fetch existing state from the blockchain
        match state::fetch_state(
            &self.config.arch_api_endpoint,
            &self.config.mempool_account.to_string(),
            self.config.verbose_logging,
        )
        .await
        {
            Ok(current_state) => {
                // Initialize with existing state
                state::initialize_state_and_processed_txids(
                    &self.current_oracle_state,
                    &self.processed_txids,
                    current_state,
                    self.config.verbose_logging,
                );
                info!("Initialized with existing state from blockchain");
            }
            Err(e) => {
                // No existing state or error reading it
                warn!(
                    "Failed to fetch existing state: {}. Initializing empty state.",
                    e
                );
                state::initialize_empty_state(
                    &self.current_oracle_state,
                    self.config.verbose_logging,
                );
            }
        }

        Ok(())
    }

    /// Calculate differences between current mempool and stored state
    fn calculate_mempool_differences<'a>(
        &self,
        mempool_entries: &'a [TitanMempoolEntry],
    ) -> (Vec<Txid>, Vec<&'a TitanMempoolEntry>) {
        let oracle_state_read = self.current_oracle_state.read().unwrap();
        let processed_txids_read = self.processed_txids.read().unwrap();

        // Create a set of txids in the current mempool
        let current_mempool_txids: HashSet<Txid> =
            mempool_entries.iter().map(|entry| entry.txid).collect();

        // Calculate removed txids (in processed_txids but not in current mempool)
        let removed_txids: Vec<Txid> = processed_txids_read
            .iter()
            .filter(|txid| !current_mempool_txids.contains(txid))
            .cloned()
            .collect();

        // Calculate new entries (in current mempool but not in processed_txids)
        let new_entries: Vec<&TitanMempoolEntry> = mempool_entries
            .iter()
            .filter(|entry| !processed_txids_read.contains(&entry.txid))
            .collect();

        (removed_txids, new_entries)
    }

    /// Create balanced update batches
    fn create_balanced_batches<'a>(
        &self,
        removed: &[Txid],
        inserted: &[&'a TitanMempoolEntry],
    ) -> Vec<MempoolOracleInstruction> {
        let max_entries = self.config.max_batch_size;
        let mut batches = Vec::new();

        // Estimate how many inserted entries can fit with removed entries
        // This is simplified - assuming removed/inserted roughly same size
        let total_entries = removed.len() + inserted.len();
        let batch_count = (total_entries + max_entries - 1) / max_entries; // Ceiling division

        if batch_count == 0 {
            return Vec::new(); // Nothing to update
        }

        // Calculate approximately how many of each type per batch
        let removed_per_batch = (removed.len() + batch_count - 1) / batch_count;
        let inserted_per_batch = (inserted.len() + batch_count - 1) / batch_count;

        // Create iterator chunks
        let removed_chunks: Vec<Vec<Txid>> = removed
            .chunks(removed_per_batch)
            .map(|chunk| chunk.to_vec())
            .collect();

        let inserted_chunks: Vec<Vec<MempoolEntry>> = inserted
            .chunks(inserted_per_batch)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|entry| MempoolEntry {
                        txid: entry.txid.to_byte_array(),
                        fee: entry.fee as u16,
                        vsize: entry.vsize as u16,
                        descendant_count: entry.descendant_count as u8,
                        ancestor_count: entry.ancestor_count as u8,
                    })
                    .collect()
            })
            .collect();

        // Determine how many batches we'll need (use the longer of the two)
        let batch_count = removed_chunks.len().max(inserted_chunks.len());

        // Create batches
        for i in 0..batch_count {
            let removed_batch = if i < removed_chunks.len() {
                removed_chunks[i]
                    .iter()
                    .map(|txid| txid.to_byte_array())
                    .collect()
            } else {
                Vec::new()
            };

            let inserted_batch = if i < inserted_chunks.len() {
                inserted_chunks[i].clone()
            } else {
                Vec::new()
            };

            batches.push(MempoolOracleInstruction {
                removed: removed_batch,
                inserted: inserted_batch,
            });
        }

        batches
    }

    /// Create update batches from differences
    fn create_update_batches<'a>(
        &self,
        removed_txids: &[Txid],
        new_entries: &[&'a TitanMempoolEntry],
    ) -> Vec<MempoolOracleInstruction> {
        // Create balanced batches
        if removed_txids.is_empty() && new_entries.is_empty() {
            return Vec::new(); // No changes needed
        }

        // Use balanced batch creation to distribute evenly
        self.create_balanced_batches(removed_txids, new_entries)
    }

    /// Process a single update batch
    async fn process_update_batch<'a>(
        &self,
        batch: MempoolOracleInstruction,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.config.verbose_logging {
            debug!(
                "Processing update batch: {} removed, {} inserted",
                batch.removed.len(),
                batch.inserted.len()
            );
        }

        // Serialize the instruction
        let instruction_data =
            borsh::to_vec(&batch).map_err(|e| format!("Failed to serialize instruction: {}", e))?;

        // Send transaction
        let txid = transaction::send_transaction(
            &self.config.program_id,
            &self.config.mempool_account,
            &self.signer,
            instruction_data,
            &self.config.arch_api_endpoint,
            self.config.verbose_logging,
        )
        .await?;

        // Create a record of the pending transaction
        let pending_tx = transaction::create_pending_transaction(
            txid,
            batch.clone(),
            self.config.verbose_logging,
        );

        // Add to pending queue
        transaction::add_transaction_to_pending_queue(
            &self.pending_transactions,
            pending_tx,
            self.config.verbose_logging,
        );

        // Update our processed set immediately (optimistically)
        // Note: If the transaction fails, the cleanup process will add these back to unprocessed
        self.update_after_transaction_sent(&batch);

        Ok(())
    }

    /// Update tracked state after transaction is sent
    fn update_after_transaction_sent(&self, batch: &MempoolOracleInstruction) {
        // 1. Update processed txids
        let mut processed_txids_write = self.processed_txids.write().unwrap();

        // Remove txids that are being removed from state
        for removed_bytes in &batch.removed {
            let mut txid_array = [0u8; 32];
            txid_array.copy_from_slice(removed_bytes);
            let txid = Txid::from_byte_array(txid_array);
            processed_txids_write.remove(&txid);
        }

        // Add txids that are being inserted
        for entry in &batch.inserted {
            let mut txid_array = [0u8; 32];
            txid_array.copy_from_slice(&entry.txid);
            let txid = Txid::from_byte_array(txid_array);
            processed_txids_write.insert(txid);
        }

        drop(processed_txids_write);

        // 2. Update current state optimistically
        let mut oracle_state_write = self.current_oracle_state.write().unwrap();

        if let Some(ref mut state) = *oracle_state_write {
            // Remove entries
            for removed_bytes in &batch.removed {
                state.entries.retain(|entry| *removed_bytes != entry.txid);
            }

            // Add new entries
            state.entries.extend_from_slice(&batch.inserted);

            // Update last_updated timestamp
            state.last_updated = get_current_timestamp();
        }
    }

    /// Handle cases where transactions are replaced
    fn handle_replaced_txids(&self, processed_txids: HashSet<Txid>) {
        let mut processed_txids_write = self.processed_txids.write().unwrap();

        // Replace the processed txids with the provided set
        *processed_txids_write = processed_txids;

        if self.config.verbose_logging {
            debug!("Updated processed txids due to transaction replacement");
        }
    }

    /// Main method to update the smart contract with mempool changes
    pub async fn update_smart_contract(
        &self,
        mempool_entries: &[TitanMempoolEntry],
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Make sure we're initialized
        if self.current_oracle_state.read().unwrap().is_none() {
            self.initialize().await?;
        }

        // Calculate differences between current mempool and our stored state
        let (removed_txids, new_entries) = self.calculate_mempool_differences(mempool_entries);

        if removed_txids.is_empty() && new_entries.is_empty() {
            if self.config.verbose_logging {
                debug!("No mempool changes detected, skipping update");
            }
            return Ok(());
        }

        // Log update statistics
        info!(
            "Mempool update: {} removed, {} inserted",
            removed_txids.len(),
            new_entries.len()
        );

        // Create batches of updates
        let update_batches = self.create_update_batches(&removed_txids, &new_entries);

        if update_batches.is_empty() {
            // This is unlikely unless the differences were eliminated during batch creation
            debug!("No update batches created after processing differences");
            return Ok(());
        }

        info!("Created {} update batches", update_batches.len());

        // Process each batch
        for batch in update_batches {
            self.process_update_batch(batch).await?;
        }

        Ok(())
    }
}
