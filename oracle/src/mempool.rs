use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Duration,
};

use bitcoin::{OutPoint, Txid};
use titan_client::{AsyncTcpClient, TitanApi, TitanClient, TitanTcpClientError};
use titan_types::{Event, EventType, MempoolEntry, TcpSubscriptionRequest};
use tokio::{
    sync::{mpsc, watch},
    time::sleep,
};
use tracing::{debug, error, info};

use crate::mempool_smart_contract::{MempoolSmartContractUpdater, SmartContractConfig};

/// Configuration for the mempool service
#[derive(Debug, Clone)]
pub struct MempoolServiceConfig {
    /// Whether to batch fetch mempool entries when receiving many transactions at once
    pub batch_size: usize,
    /// Whether to log detailed information about mempool changes
    pub verbose_logging: bool,
    /// Configuration for the smart contract updater (None if disabled)
    pub smart_contract_config: Option<SmartContractConfig>,
}

impl Default for MempoolServiceConfig {
    fn default() -> Self {
        Self {
            batch_size: 100,
            verbose_logging: false,
            smart_contract_config: None,
        }
    }
}

/// A service that maintains a real-time view of the Bitcoin mempool
pub struct MempoolService {
    /// The mempool data, a mapping of transaction IDs to their mempool entries
    mempool: Arc<RwLock<HashMap<Txid, MempoolEntry>>>,
    /// The HTTP client for initial population and periodic refresh
    titan_client: TitanClient,
    /// The TCP client for receiving real-time updates
    titan_tcp_client: AsyncTcpClient,
    /// Configuration for the mempool service
    config: MempoolServiceConfig,
    /// Channel to signal shutdown to background tasks
    shutdown_tx: watch::Sender<()>,
    /// Smart contract updater (if enabled)
    smart_contract_updater: Option<Arc<MempoolSmartContractUpdater>>,
}

impl MempoolService {
    /// Create a new MempoolService with default configuration
    pub async fn new(
        http_endpoint: &str,
        tcp_endpoint: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::with_config(http_endpoint, tcp_endpoint, MempoolServiceConfig::default()).await
    }

    /// Create a new MempoolService with custom configuration
    pub async fn with_config(
        http_endpoint: &str,
        tcp_endpoint: &str,
        config: MempoolServiceConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let http_client = TitanClient::new(http_endpoint);
        let tcp_client = AsyncTcpClient::new();
        let (shutdown_tx, _) = watch::channel(());

        // Initialize smart contract updater if configured
        let smart_contract_updater = if let Some(sc_config) = &config.smart_contract_config {
            match MempoolSmartContractUpdater::new(sc_config.clone()) {
                Ok(updater) => {
                    info!("Smart contract updater initialized");
                    Some(Arc::new(updater))
                }
                Err(e) => {
                    error!("Failed to initialize smart contract updater: {:?}", e);
                    None
                }
            }
        } else {
            None
        };

        let service = Self {
            mempool: Arc::new(RwLock::new(HashMap::new())),
            titan_client: http_client,
            titan_tcp_client: tcp_client,
            config,
            shutdown_tx,
            smart_contract_updater,
        };

        // Initialize the mempool with current state
        service.populate_mempool().await?;

        // Start the subscription to keep it updated
        service.start_subscription(tcp_endpoint).await?;

        Ok(service)
    }

    /// Get a clone of the current mempool state
    pub fn get_mempool(&self) -> HashMap<Txid, MempoolEntry> {
        match self.mempool.read() {
            Ok(mempool) => mempool.clone(),
            Err(e) => {
                error!("Failed to read mempool: {:?}", e);
                HashMap::new()
            }
        }
    }

    /// Get the number of transactions in the mempool
    pub fn mempool_size(&self) -> usize {
        match self.mempool.read() {
            Ok(mempool) => mempool.len(),
            Err(e) => {
                error!("Failed to read mempool size: {:?}", e);
                0
            }
        }
    }

    /// Get a specific mempool entry by transaction ID
    pub fn get_entry(&self, txid: &Txid) -> Option<MempoolEntry> {
        match self.mempool.read() {
            Ok(mempool) => mempool.get(txid).cloned(),
            Err(e) => {
                error!("Failed to read mempool for txid {}: {:?}", txid, e);
                None
            }
        }
    }

    /// Check if a transaction is in the mempool
    pub fn has_transaction(&self, txid: &Txid) -> bool {
        match self.mempool.read() {
            Ok(mempool) => mempool.contains_key(txid),
            Err(e) => {
                error!("Failed to check mempool for txid {}: {:?}", txid, e);
                false
            }
        }
    }

    /// Find all transactions that spend a particular outpoint
    pub fn find_spending_tx(&self, outpoint: &OutPoint) -> Vec<Txid> {
        match self.mempool.read() {
            Ok(mempool) => {
                mempool
                    .iter()
                    .filter_map(|(txid, entry)| {
                        // Check if this transaction spends the given outpoint
                        if entry.depends.contains(&outpoint.txid) {
                            Some(*txid)
                        } else {
                            None
                        }
                    })
                    .collect()
            }
            Err(e) => {
                error!(
                    "Failed to find spending tx for outpoint {}: {:?}",
                    outpoint, e
                );
                Vec::new()
            }
        }
    }

    /// Populate the mempool with current state from the HTTP API
    async fn populate_mempool(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Populating mempool from HTTP API...");

        // Get all transaction IDs in the mempool
        let txids = self.titan_client.get_mempool_txids().await?;
        info!("Found {} transactions in mempool", txids.len());

        if !txids.is_empty() {
            // Get mempool entries for all transactions
            self.fetch_and_update_entries(&txids).await?;
        }

        Ok(())
    }

    /// Fetch mempool entries for a list of transaction IDs and update the mempool
    async fn fetch_and_update_entries(
        &self,
        txids: &[Txid],
    ) -> Result<(), Box<dyn std::error::Error>> {
        if txids.is_empty() {
            return Ok(());
        }

        if self.config.verbose_logging {
            info!("Fetching details for {} transactions", txids.len());
        } else {
            debug!("Fetching details for {} transactions", txids.len());
        }

        // Process in batches if needed
        let batch_size = self.config.batch_size;
        if txids.len() > batch_size {
            for chunk in txids.chunks(batch_size) {
                let entries = self.titan_client.get_mempool_entries(chunk).await?;
                self.update_mempool_entries(entries);

                // Small delay to avoid overwhelming the API
                sleep(Duration::from_millis(50)).await;
            }
        } else {
            // Get mempool entries for all transactions at once
            let entries = self.titan_client.get_mempool_entries(txids).await?;
            self.update_mempool_entries(entries);
        }

        Ok(())
    }

    /// Update the mempool with entries from the API
    fn update_mempool_entries(&self, entries: HashMap<Txid, Option<MempoolEntry>>) {
        match self.mempool.write() {
            Ok(mut mempool) => {
                let mut added = 0;

                // Add all entries
                for (txid, entry_opt) in entries {
                    if let Some(entry) = entry_opt {
                        mempool.insert(txid, entry);
                        added += 1;
                    }
                }

                if self.config.verbose_logging {
                    info!("Added/updated {} mempool entries", added);
                } else {
                    debug!("Added/updated {} mempool entries", added);
                }

                // Update smart contract if enabled
                if let Some(updater) = &self.smart_contract_updater {
                    let cloned_mempool = mempool.clone();
                    let updater_clone = updater.clone();

                    // Spawn a task to update the smart contract
                    tokio::spawn(async move {
                        if let Err(e) = updater_clone.update_smart_contract(&cloned_mempool).await {
                            error!("Failed to update smart contract: {:?}", e);
                        }
                    });
                }
            }
            Err(e) => {
                error!("Failed to write to mempool: {:?}", e);
            }
        }
    }

    /// Start a background task to periodically refresh the mempool
    pub fn start_periodic_refresh(&self, interval: Duration) {
        let mempool_clone = self.mempool.clone();
        let http_client_clone = self.titan_client.clone();
        let config_clone = self.config.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let smart_contract_updater_clone = self.smart_contract_updater.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = sleep(interval) => {
                        if config_clone.verbose_logging {
                            info!("Performing periodic mempool refresh");
                        } else {
                            debug!("Performing periodic mempool refresh");
                        }

                        if let Err(e) = refresh_mempool(&http_client_clone, &mempool_clone, &config_clone, smart_contract_updater_clone.as_ref()).await {
                            error!("Failed to refresh mempool: {:?}", e);
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        info!("Shutting down periodic refresh task");
                        break;
                    }
                }
            }
        });

        info!("Started periodic mempool refresh task");
    }

    /// Start a TCP subscription to get real-time mempool updates
    async fn start_subscription(&self, tcp_endpoint: &str) -> Result<(), TitanTcpClientError> {
        // Subscribe to mempool events
        let subscription_request = TcpSubscriptionRequest {
            subscribe: vec![
                EventType::MempoolTransactionsAdded,
                EventType::MempoolTransactionsReplaced,
                EventType::MempoolEntriesUpdated,
            ],
        };

        let mut event_rx = self
            .titan_tcp_client
            .subscribe(tcp_endpoint, subscription_request)
            .await?;
        let mempool_clone = self.mempool.clone();
        let http_client_clone = self.titan_client.clone();
        let config_clone = self.config.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let smart_contract_updater_clone = self.smart_contract_updater.clone();

        // Spawn a task to handle the events
        tokio::spawn(async move {
            handle_mempool_events(
                event_rx,
                mempool_clone,
                http_client_clone,
                config_clone,
                shutdown_rx,
                smart_contract_updater_clone,
            )
            .await;
        });

        info!("Started mempool subscription");
        Ok(())
    }

    /// Shutdown the service and all background tasks
    pub fn shutdown(&self) {
        if let Err(e) = self.shutdown_tx.send(()) {
            error!("Failed to send shutdown signal: {:?}", e);
        }

        // Clear processed txids in the smart contract updater
        if let Some(updater) = &self.smart_contract_updater {
            updater.clear_processed_txids();
        }

        info!("Shutdown signal sent to all background tasks");
    }
}

impl Drop for MempoolService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Helper function to refresh the mempool from HTTP API
async fn refresh_mempool(
    http_client: &TitanClient,
    mempool_data: &Arc<RwLock<HashMap<Txid, MempoolEntry>>>,
    config: &MempoolServiceConfig,
    smart_contract_updater: Option<&Arc<MempoolSmartContractUpdater>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get all transaction IDs in the mempool
    let txids = http_client.get_mempool_txids().await?;

    if !txids.is_empty() {
        // Get mempool entries for all transactions
        if txids.len() > config.batch_size {
            let mut updated_entries = HashMap::new();

            for chunk in txids.chunks(config.batch_size) {
                let entries = http_client.get_mempool_entries(chunk).await?;
                updated_entries.extend(entries);

                // Small delay to avoid overwhelming the API
                sleep(Duration::from_millis(50)).await;
            }

            update_mempool_from_refresh(
                mempool_data,
                updated_entries,
                config,
                smart_contract_updater,
            );
        } else {
            let entries = http_client.get_mempool_entries(&txids).await?;
            update_mempool_from_refresh(mempool_data, entries, config, smart_contract_updater);
        }
    }

    Ok(())
}

/// Updates the mempool data from a refresh operation
fn update_mempool_from_refresh(
    mempool_data: &Arc<RwLock<HashMap<Txid, MempoolEntry>>>,
    entries: HashMap<Txid, Option<MempoolEntry>>,
    config: &MempoolServiceConfig,
    smart_contract_updater: Option<&Arc<MempoolSmartContractUpdater>>,
) {
    match mempool_data.write() {
        Ok(mut mempool) => {
            mempool.clear(); // Clear the existing data

            let mut added = 0;
            // Add all entries
            for (txid, entry_opt) in entries {
                if let Some(entry) = entry_opt {
                    mempool.insert(txid, entry);
                    added += 1;
                }
            }

            if config.verbose_logging {
                info!("Mempool refreshed with {} transactions", added);
            } else {
                debug!("Mempool refreshed with {} transactions", added);
            }

            // Update smart contract if enabled
            if let Some(updater) = smart_contract_updater {
                let cloned_mempool = mempool.clone();
                let updater_clone = updater.clone();

                // Spawn a task to update the smart contract
                tokio::spawn(async move {
                    if let Err(e) = updater_clone.update_smart_contract(&cloned_mempool).await {
                        error!("Failed to update smart contract: {:?}", e);
                    }
                });
            }
        }
        Err(e) => {
            error!("Failed to write to mempool: {:?}", e);
        }
    }
}

/// Handle mempool events from the subscription
async fn handle_mempool_events(
    mut event_rx: mpsc::Receiver<Event>,
    mempool_data: Arc<RwLock<HashMap<Txid, MempoolEntry>>>,
    http_client: TitanClient,
    config: MempoolServiceConfig,
    mut shutdown_rx: watch::Receiver<()>,
    smart_contract_updater: Option<Arc<MempoolSmartContractUpdater>>,
) {
    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                match event {
                    Event::MempoolTransactionsAdded { txids } => {
                        if config.verbose_logging {
                            info!("Received {} new mempool transactions", txids.len());
                        } else {
                            debug!("Received {} new mempool transactions", txids.len());
                        }

                        // Fetch the mempool entries for these new transactions
                        if let Err(e) = fetch_and_update_batch(&http_client, &mempool_data, &txids, &config, smart_contract_updater.as_ref()).await {
                            error!("Failed to fetch mempool entries: {:?}", e);
                        }
                    }
                    Event::MempoolTransactionsReplaced { txids } => {
                        if config.verbose_logging {
                            info!("Received {} replaced mempool transactions", txids.len());
                        } else {
                            debug!("Received {} replaced mempool transactions", txids.len());
                        }

                        // Remove replaced transactions from our map
                        if let Ok(mut mempool) = mempool_data.write() {
                            for txid in &txids {
                                mempool.remove(txid);
                            }
                        } else {
                            error!("Failed to acquire write lock for mempool");
                        }

                        // Update the smart contract's processed txids list
                        if let Some(updater) = &smart_contract_updater {
                            updater.handle_replaced_txids(&txids);
                        }
                    }
                    Event::MempoolEntriesUpdated { txids } => {
                        if config.verbose_logging {
                            info!("Received updates for {} mempool entries", txids.len());
                        } else {
                            debug!("Received updates for {} mempool entries", txids.len());
                        }

                        // Update the mempool entries
                        if let Ok(mut mempool) = mempool_data.write() {
                            for (txid, entry) in &txids {
                                mempool.insert(*txid, entry.clone());
                            }

                            // Update smart contract if enabled
                            if let Some(updater) = &smart_contract_updater {
                                let cloned_mempool = mempool.clone();
                                let updater_clone = updater.clone();

                                // Spawn a task to update the smart contract
                                tokio::spawn(async move {
                                    // Create a HashMap with just the updated entries
                                    let updated_entries: HashMap<Txid, MempoolEntry> = txids
                                        .iter()
                                        .map(|(txid, entry)| (*txid, entry.clone()))
                                        .collect();

                                    if let Err(e) = updater_clone.update_smart_contract(&updated_entries).await {
                                        error!("Failed to update smart contract with updated entries: {:?}", e);
                                    }
                                });
                            }
                        } else {
                            error!("Failed to acquire write lock for mempool");
                        }
                    }
                    _ => {
                        // Ignore other events
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                info!("Shutdown signal received. Stopping mempool event handling.");
                break;
            }
            else => {
                error!("Event channel closed. Mempool updates will no longer be received.");
                break;
            }
        }
    }
}

/// Fetch and update mempool entries in batches if necessary
async fn fetch_and_update_batch(
    http_client: &TitanClient,
    mempool_data: &Arc<RwLock<HashMap<Txid, MempoolEntry>>>,
    txids: &[Txid],
    config: &MempoolServiceConfig,
    smart_contract_updater: Option<&Arc<MempoolSmartContractUpdater>>,
) -> Result<(), Box<dyn std::error::Error>> {
    if txids.is_empty() {
        return Ok(());
    }

    // Process in batches if needed
    if txids.len() > config.batch_size {
        for chunk in txids.chunks(config.batch_size) {
            if let Ok(entries) = http_client.get_mempool_entries(chunk).await {
                if let Ok(mut mempool) = mempool_data.write() {
                    let mut updated_entries = HashMap::new();

                    for (txid, entry_opt) in entries {
                        if let Some(entry) = entry_opt {
                            mempool.insert(txid, entry.clone());
                            updated_entries.insert(txid, entry);
                        }
                    }

                    // Update smart contract if enabled
                    if let Some(updater) = smart_contract_updater {
                        let updater_clone = updater.clone();
                        let updated_entries_clone = updated_entries.clone();

                        tokio::spawn(async move {
                            if let Err(e) = updater_clone
                                .update_smart_contract(&updated_entries_clone)
                                .await
                            {
                                error!(
                                    "Failed to update smart contract with batch entries: {:?}",
                                    e
                                );
                            }
                        });
                    }
                }
            }
            // Small delay to avoid overwhelming the API
            sleep(Duration::from_millis(50)).await;
        }
    } else {
        // Get mempool entries for all transactions at once
        if let Ok(entries) = http_client.get_mempool_entries(txids).await {
            if let Ok(mut mempool) = mempool_data.write() {
                let mut updated_entries = HashMap::new();

                for (txid, entry_opt) in entries {
                    if let Some(entry) = entry_opt {
                        mempool.insert(txid, entry.clone());
                        updated_entries.insert(txid, entry);
                    }
                }

                // Update smart contract if enabled
                if let Some(updater) = smart_contract_updater {
                    let updater_clone = updater.clone();
                    let updated_entries_clone = updated_entries.clone();

                    tokio::spawn(async move {
                        if let Err(e) = updater_clone
                            .update_smart_contract(&updated_entries_clone)
                            .await
                        {
                            error!(
                                "Failed to update smart contract with batch entries: {:?}",
                                e
                            );
                        }
                    });
                }
            }
        }
    }

    Ok(())
}
