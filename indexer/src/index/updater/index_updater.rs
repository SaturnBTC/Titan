use {
    super::{
        transaction_update::{TransactionChangeSet, TransactionUpdate},
        *,
    },
    crate::{
        bitcoin_rpc::{RpcClientError, RpcClientPool, RpcClientPoolError},
        index::{
            metrics::Metrics,
            store::Store,
            updater::{
                cache::{BlockCache, BlockCacheSettings, MempoolCache, MempoolCacheSettings},
                events::Events,
                transaction::{TransactionParser, TransactionUpdater},
            },
            Chain, Settings, StoreError,
        },
        models::{BlockId, RuneEntry},
    },
    bitcoin::{
        constants::SUBSIDY_HALVING_INTERVAL, hex::HexToArrayError, Block as BitcoinBlock,
        Transaction,
    },
    bitcoincore_rpc::{
        json::{GetBlockchainInfoResult, GetMempoolEntryResult},
        Client, RpcApi,
    },
    fetcher::{block_fetcher::fetch_blocks_from, mempool_fetcher::MempoolError},
    indicatif::{ProgressBar, ProgressStyle},
    ordinals::Terms,
    prometheus::HistogramVec,
    rollback::{Rollback, RollbackError},
    rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet},
    std::{
        fmt::{self, Display, Formatter},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex, RwLock,
        },
        time::{SystemTime, UNIX_EPOCH},
    },
    store_lock::StoreWithLock,
    thiserror::Error,
    titan_types_core::{Block, Event, MempoolEntry, RuneId, SerializedTxid},
    titan_types_core::{Rune, SpacedRune},
    tokio::sync::mpsc::{error::SendError, Sender},
    tracing::{debug, error, info, warn},
};

#[derive(Debug, Error)]
pub enum ReorgError {
    Recoverable { height: u64, depth: u64 },
    Unrecoverable,
    StoreError(#[from] StoreError),
    RPCError(#[from] bitcoincore_rpc::Error),
}

impl Display for ReorgError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Recoverable { height, depth } => {
                write!(f, "{depth} block deep reorg detected at height {height}")
            }
            Self::Unrecoverable => write!(f, "unrecoverable reorg detected"),
            Self::StoreError(e) => write!(f, "store error: {e}"),
            Self::RPCError(e) => write!(f, "RPC error: {e}"),
        }
    }
}

#[derive(Error, Debug)]
pub enum UpdaterError {
    #[error("db error {0}")]
    DB(#[from] StoreError),
    #[error("bitcoin rpc error {0}")]
    BitcoinRpc(#[from] bitcoincore_rpc::Error),
    #[error("bitcoin reorg error {0}")]
    BitcoinReorg(#[from] ReorgError),
    #[error("bitcoin rpc client error {0}")]
    BitcoinRpcClient(#[from] RpcClientError),
    #[error("transaction updater error {0}")]
    TransactionUpdater(#[from] TransactionUpdaterError),
    #[error("rollback error {0}")]
    Rollback(#[from] RollbackError),
    #[error("rune parser error {0}")]
    RuneParser(#[from] TransactionParserError),
    #[error("txid error {0}")]
    Txid(#[from] HexToArrayError),
    #[error("mempool error {0}")]
    Mempool(#[from] MempoolError),
    #[error("mutex error")]
    Mutex,
    #[error("event sender error {0}")]
    EventSender(#[from] SendError<Event>),
    #[error("invalid main chain tip")]
    InvalidMainChainTip,
    #[error("bitcoin rpc pool error {0}")]
    BitcoinRpcPool(#[from] RpcClientPoolError),
}

type Result<T> = std::result::Result<T, UpdaterError>;

pub struct Updater {
    db: Arc<StoreWithLock>,
    settings: Settings,
    is_at_tip: AtomicBool,

    bitcoin_rpc_pool: RpcClientPool,

    shutdown_flag: Arc<AtomicBool>,

    broadcast_lock: Mutex<()>,
    pre_index_submitted_txs: RwLock<HashSet<SerializedTxid>>,

    zmq_received_txs: RwLock<HashMap<SerializedTxid, Transaction>>,

    transaction_update: RwLock<TransactionUpdate>,

    sender: Option<Sender<Event>>,

    // monitoring
    latency: HistogramVec,
}

impl Updater {
    pub fn new(
        db: Arc<dyn Store + Send + Sync>,
        bitcoin_rpc_pool: RpcClientPool,
        settings: Settings,
        metrics: &Metrics,
        shutdown_flag: Arc<AtomicBool>,
        sender: Option<Sender<Event>>,
    ) -> Self {
        Self {
            db: Arc::new(StoreWithLock::new(db)),
            settings,
            bitcoin_rpc_pool,
            is_at_tip: AtomicBool::new(false),
            broadcast_lock: Mutex::new(()),
            pre_index_submitted_txs: RwLock::new(HashSet::default()),
            zmq_received_txs: RwLock::new(HashMap::default()),
            shutdown_flag,
            transaction_update: RwLock::new(TransactionUpdate::default()),
            sender,
            latency: metrics.histogram_vec(
                prometheus::HistogramOpts::new("indexer_latency", "Indexer latency"),
                &["method"],
            ),
        }
    }

    pub fn is_at_tip(&self) -> bool {
        self.is_at_tip.load(Ordering::Relaxed)
    }

    fn is_chain_synced(
        &self,
        cache: &mut BlockCache,
        chain_info: &GetBlockchainInfoResult,
    ) -> Result<bool> {
        if cache.get_block_height_tip() != chain_info.blocks {
            return Ok(false);
        }

        // Add exception when indexing genesis block
        if cache.get_block_height_tip() == 0 && chain_info.blocks == 0 {
            return Ok(true);
        }

        match cache.get_block_hash(chain_info.blocks) {
            Ok(block_hash) => {
                // Compare hashes to detect reorgs
                Ok(block_hash == chain_info.best_block_hash)
            }
            Err(_) => {
                // If we can't get the hash, we're not synced
                Ok(false)
            }
        }
    }

    fn is_already_at_tip(
        &self,
        chain_info: &bitcoincore_rpc::json::GetBlockchainInfoResult,
    ) -> Result<bool> {
        let db_read = self.db.read();
        let db_block_count = db_read.get_block_count()?;
        let db_tip_height = db_block_count.saturating_sub(1);

        let heights_match =
            db_tip_height == chain_info.blocks || (db_tip_height == 0 && chain_info.blocks == 0);

        if !heights_match {
            return Ok(false);
        }

        if db_block_count == 0 {
            return Ok(true);
        }

        Ok(db_read.get_block_hash(db_tip_height)? == chain_info.best_block_hash)
    }

    fn mark_as_at_tip(&self) {
        if !self.shutdown_flag.load(Ordering::SeqCst) {
            let was_at_tip = self.is_at_tip.swap(true, Ordering::AcqRel);
            let db = self.db.write();
            if !was_at_tip {
                if let Err(e) = db.finish_bulk_load() {
                    warn!("Failed to switch database to online mode: {:?}", e);
                }
            }

            if let Err(e) = db.set_is_at_tip(true) {
                warn!("Failed to persist is_at_tip=true: {:?}", e);
            }
        }
    }

    pub fn mark_as_not_at_tip(&self) {
        use std::sync::atomic::Ordering;
        self.is_at_tip.store(false, Ordering::Release);
        let db = self.db.write();
        if let Err(e) = db.set_is_at_tip(false) {
            warn!("Failed to persist is_at_tip=false: {:?}", e);
        }
    }

    /// Lightweight entry-point that performs a quick tip check and, if needed, runs a full sync.
    pub fn update_to_tip(&self) -> Result<()> {
        debug!("Updating to tip");

        let rpc_client = self.bitcoin_rpc_pool.get()?;
        let chain_info = rpc_client.get_blockchain_info()?;

        if self.is_already_at_tip(&chain_info)? {
            self.mark_as_at_tip();
            return Ok(());
        }

        self.perform_full_sync()?;
        Ok(())
    }

    fn perform_full_sync(&self) -> Result<()> {
        debug!("Updating to tip");

        // Every 5000 blocks, commit the changes to the database
        let commit_interval = self.settings.commit_interval as usize;

        // Get RPC client and current blockchain info before doing any heavy work
        let bitcoin_block_client = self.bitcoin_rpc_pool.get()?;
        let mut chain_info = bitcoin_block_client.get_blockchain_info()?;

        // Not at tip – proceed with full indexing workflow.
        let mut cache = BlockCache::new(self.db.clone(), BlockCacheSettings::new(&self.settings))?;
        let mut events = Events::new();

        let mut indexing_first_block = true;

        // Fetch new blocks if needed.
        while !self.is_chain_synced(&mut cache, &chain_info)? {
            self.mark_as_not_at_tip();

            let progress_bar =
                self.open_progress_bar(cache.get_block_height_tip(), chain_info.blocks);

            let current_block_count = cache.get_block_count();

            let (rx, block_fetch_stats) = fetch_blocks_from(
                self.bitcoin_rpc_pool.clone(),
                current_block_count,
                chain_info.blocks + 1,
                self.shutdown_flag.clone(),
            )?;

            let rpc_client = self.bitcoin_rpc_pool.get()?;

            while let Ok(block) = rx.recv() {
                // We have consumed a block from the final channel, reflect that in the stats
                block_fetch_stats.decrement_final();

                let _timer = self
                    .latency
                    .with_label_values(&["full_block_indexing"])
                    .start_timer();

                if self.shutdown_flag.load(Ordering::SeqCst) {
                    info!("Updater received shutdown signal, stopping...");
                    break;
                }

                // Always check for potential reorgs before indexing the next block.
                match self.detect_reorg(
                    &block,
                    cache.get_block_count(),
                    &mut cache,
                    &rpc_client,
                    self.settings.max_recoverable_reorg_depth(),
                ) {
                    Ok(()) => (),
                    Err(ReorgError::Recoverable { height, depth }) => {
                        // Persist any in-memory updates so `handle_reorg` can cleanly roll
                        // back blocks that might still be sitting in the cache and have not
                        // been flushed to RocksDB yet.
                        cache.flush_sync()?;

                        self.handle_reorg(height, depth)?;
                        if let Some(sender) = &self.sender {
                            if let Err(e) = sender.blocking_send(Event::Reorg { height, depth }) {
                                error!("Failed to send reorg event: {:?}", e);
                            }
                        }
                        return Err(ReorgError::Recoverable { height, depth }.into());
                    }
                    Err(e) => {
                        return Err(e.into());
                    }
                }

                let block = self.index_block(
                    block,
                    cache.get_block_count() as u64,
                    &rpc_client,
                    &mut cache,
                    &mut events,
                )?;

                events.add_event(Event::NewBlock {
                    block_height: cache.get_block_count(),
                    block_hash: block.header.block_hash(),
                });

                cache.set_new_block(block);

                if cache.should_flush(commit_interval) {
                    info!("Flushing cache");

                    let _timer = self
                        .latency
                        .with_label_values(&["flush_cache"])
                        .start_timer();

                    cache.add_address_events(&mut events, self.settings.chain);

                    // Flush asynchronously, and emit events only after persistence completes
                    self.emit_events_after_persist(&mut cache, &mut events)?;
                }

                indexing_first_block = false;
                progress_bar.inc(1);

                if chain_info.blocks - cache.get_block_height_tip()
                    > self.settings.max_recoverable_reorg_depth()
                {
                    // clear notifications.
                    self.notify_tx_updates(true)?;
                }
            }

            if self.shutdown_flag.load(Ordering::SeqCst) {
                info!("Updater received shutdown signal, stopping...");
                break;
            }

            info!("Synced to tip {}", chain_info.blocks);
            chain_info = bitcoin_block_client.get_blockchain_info()?;
            progress_bar.finish_and_clear();
        }

        if !indexing_first_block {
            info!("Flushing cache");

            if self.settings.index_addresses {
                cache.add_address_events(&mut events, self.settings.chain);
            }

            self.emit_events_after_persist(&mut cache, &mut events)?;
        }

        self.mark_as_at_tip();

        Ok(())
    }

    fn emit_events_after_persist(&self, cache: &mut BlockCache, events: &mut Events) -> Result<()> {
        let rx = cache.flush_with_notify()?;
        let mut pending_events = events.take_all();
        let sender_clone = self.sender.clone();
        std::thread::spawn(move || {
            let _ = rx.recv();
            info!("emit_events_after_persist: events emitted");
            if let Some(sender) = sender_clone {
                for event in pending_events.drain(..) {
                    let result = sender.blocking_send(event);
                    if let Err(e) = result {
                        error!("Failed to send event: {:?} in emit_events_after_persist", e);
                    }
                }
            }
        });
        Ok(())
    }

    pub fn index_mempool(&self) -> Result<()> {
        let _timer = self
            .latency
            .with_label_values(&["index_mempool"])
            .start_timer();

        let client = self.bitcoin_rpc_pool.get()?;

        // Get current mempool transactions
        let lock = self.broadcast_lock.lock().unwrap();
        let current_mempool = {
            let current_mempool = client.get_raw_mempool_verbose()?;
            current_mempool
                .into_iter()
                .map(|(txid, mempool_entry)| (txid.into(), mempool_entry))
                .collect::<HashMap<SerializedTxid, GetMempoolEntryResult>>()
        };

        // Get our previously indexed mempool transactions
        let stored_mempool = {
            let db = self.db.read();
            db.get_mempool_txids()?
        };

        drop(lock);

        // Find new transactions to index
        let (new_txs, new_txs_with_mempool_entry): (
            Vec<SerializedTxid>,
            Vec<(SerializedTxid, MempoolEntry)>,
        ) = current_mempool
            .iter()
            .filter(|(txid, _)| !stored_mempool.contains_key(&txid))
            .map(|(txid, mempool_entry)| (*txid, (*txid, MempoolEntry::from(mempool_entry))))
            .unzip();

        // Find transactions to remove (they're no longer in mempool)
        let removed_txs: Vec<SerializedTxid> = stored_mempool
            .keys()
            .filter(|txid| !current_mempool.contains_key(txid))
            .cloned()
            .collect();

        // Index new transactions
        let new_txs_len = new_txs.len();

        let mut cache =
            MempoolCache::new(self.db.clone(), MempoolCacheSettings::new(&self.settings))?;

        let mut events = Events::new();

        let updated_txids =
            self.update_mempool_entries(&mut cache, &stored_mempool, &current_mempool);

        self.send_mempool_events(
            &mut events,
            new_txs_with_mempool_entry,
            removed_txs.clone(),
            updated_txids,
        )?;

        if new_txs_len > 0 {
            let tx_map = self.choose_mempool_transactions_to_index(&new_txs)?;

            let tx_order =
                fetcher::mempool_fetcher::sort_transaction_order(&current_mempool, &tx_map)?;

            // Store mempool entries for new transactions
            for txid in &tx_order {
                let tx = tx_map.get(txid).unwrap();
                let mempool_entry = current_mempool.get(txid).unwrap();

                self.index_tx(
                    txid,
                    tx,
                    MempoolEntry::from(mempool_entry),
                    &mut cache,
                    &mut events,
                )?;
            }

            if self.settings.index_addresses {
                cache.add_address_events(&mut events, self.settings.chain);
            }
        }

        cache.flush()?;

        if let Err(e) = events.send_events(&self.sender) {
            if !self.shutdown_flag.load(Ordering::SeqCst) {
                error!("Failed to send events: {:?}", e);
            }
        }

        let removed_len = removed_txs.len();
        if removed_txs.len() > 0 {
            self.remove_txs(&removed_txs, true)?;
        }

        if new_txs_len > 0 || removed_len > 0 {
            debug!(
                "Mempool: New txs: {}. Removed txs: {}",
                new_txs_len, removed_len
            );
        }

        self.transaction_update
            .write()
            .map_err(|_| UpdaterError::Mutex)?
            .update_mempool(TransactionChangeSet {
                added: new_txs.into_iter().collect(),
                removed: removed_txs.into_iter().collect(),
            });

        self.zmq_received_txs
            .write()
            .map_err(|_| UpdaterError::Mutex)?
            .clear();

        Ok(())
    }

    fn update_mempool_entries(
        &self,
        cache: &mut MempoolCache,
        stored_mempool: &HashMap<SerializedTxid, MempoolEntry>,
        current_mempool: &HashMap<SerializedTxid, GetMempoolEntryResult>,
    ) -> Vec<(SerializedTxid, MempoolEntry)> {
        let mut updated_txids = Vec::new();
        for (stored_txid, stored_mempool_entry) in stored_mempool {
            if let Some(mempool_entry_result) = current_mempool.get(stored_txid) {
                let mempool_entry = MempoolEntry::from(mempool_entry_result);

                if *stored_mempool_entry != mempool_entry {
                    cache.set_mempool_tx(*stored_txid, mempool_entry.clone());
                    updated_txids.push((*stored_txid, mempool_entry.clone()));
                }
            }
        }

        updated_txids
    }

    fn send_mempool_events(
        &self,
        events: &mut Events,
        new_txids: Vec<(SerializedTxid, MempoolEntry)>,
        removed_txids: Vec<SerializedTxid>,
        updated_txids: Vec<(SerializedTxid, MempoolEntry)>,
    ) -> Result<()> {
        if !new_txids.is_empty() {
            events.add_event(Event::MempoolTransactionsAdded { txids: new_txids });
        }

        if !removed_txids.is_empty() {
            events.add_event(Event::MempoolTransactionsReplaced {
                txids: removed_txids,
            });
        }

        if !updated_txids.is_empty() {
            events.add_event(Event::MempoolEntriesUpdated {
                txids: updated_txids,
            });
        }

        Ok(())
    }

    fn choose_mempool_transactions_to_index(
        &self,
        new_txs: &Vec<SerializedTxid>,
    ) -> Result<HashMap<SerializedTxid, Transaction>> {
        // Filter out pre-indexed transactions
        let new_txs = {
            let pre_indexed_txs = self
                .pre_index_submitted_txs
                .read()
                .map_err(|_| UpdaterError::Mutex)?
                .clone();

            new_txs
                .iter()
                .filter(|txid| !pre_indexed_txs.contains(*txid))
                .cloned()
                .collect::<Vec<_>>()
        };

        // Get all zmq transactions that we have received.
        let mut zmq_txns = {
            let zmq_txns = self
                .zmq_received_txs
                .read()
                .map_err(|_| UpdaterError::Mutex)?
                .clone();

            zmq_txns
        };

        // Partition the new transactions into those that we already have and those that we need to fetch.
        let (to_index, to_fetch): (Vec<SerializedTxid>, Vec<SerializedTxid>) = new_txs
            .iter()
            .partition(|txid| zmq_txns.contains_key(*txid));

        // If we don't have all the transactions that we received, remove the ones that we don't have.
        if to_index.len() != zmq_txns.len() {
            zmq_txns.retain(|txid, _| to_index.contains(txid));
        }

        // Fetch the transactions that we need to index.
        if !to_fetch.is_empty() {
            let fetched = fetcher::mempool_fetcher::fetch_transactions(
                self.bitcoin_rpc_pool.clone(),
                &to_fetch,
                self.shutdown_flag.clone(),
            );

            zmq_txns.extend(fetched);
        }

        // Return the transactions that we need to index.
        Ok(zmq_txns)
    }

    fn index_block(
        &self,
        bitcoin_block: BitcoinBlock,
        height: u64,
        rpc_client: &Client,
        cache: &mut BlockCache,
        events: &mut Events,
    ) -> Result<Block> {
        let _timer = self
            .latency
            .with_label_values(&["index_block"])
            .start_timer();

        let mut transaction_parser =
            TransactionParser::new(&rpc_client, self.settings.chain, height, false)?;

        let block_header: bitcoin::block::Header = bitcoin_block.header.clone();
        let block_height = height;

        let _block_tx_timer = self
            .latency
            .with_label_values(&["parse_block&index_block_txs"])
            .start_timer();

        let mut transaction_updater = TransactionUpdater::new(self.settings.clone().into(), false)?;

        let mut block = Block::empty_block(height, bitcoin_block.header);

        let mut transaction_update = self
            .transaction_update
            .write()
            .map_err(|_| UpdaterError::Mutex)?;

        for (i, tx) in bitcoin_block.txdata.iter().enumerate() {
            let txid = tx.compute_txid().into();
            match transaction_parser.parse(cache, u32::try_from(i).unwrap(), tx) {
                Ok(result) => {
                    debug!("Indexing tx {} in block {}", txid, block_height);
                    transaction_updater.save(
                        cache,
                        events,
                        block_header.time,
                        Some(BlockId {
                            hash: bitcoin_block.header.block_hash(),
                            height: block_height,
                        }),
                        txid,
                        &bitcoin_block.txdata[i],
                        &result,
                    )?;
                    block.tx_ids.push(txid);
                    transaction_update.add_block_tx(txid);
                    if let Some((id, ..)) = result.etched {
                        block.etched_runes.push(id);
                    }
                }
                Err(e) => {
                    panic!("Failed to index transaction {}: {}", txid, e);
                }
            }
        }

        Ok(block)
    }

    pub fn pre_index_new_submitted_transaction(&self, txid: &SerializedTxid) -> Result<()> {
        let mut pre_index_submitted_txs = self
            .pre_index_submitted_txs
            .write()
            .map_err(|_| UpdaterError::Mutex)?;
        pre_index_submitted_txs.insert(*txid);
        Ok(())
    }

    pub fn remove_pre_index_new_submitted_transaction(&self, txid: &SerializedTxid) -> Result<()> {
        let mut pre_index_submitted_txs = self
            .pre_index_submitted_txs
            .write()
            .map_err(|_| UpdaterError::Mutex)?;
        pre_index_submitted_txs.remove(txid);
        Ok(())
    }

    pub fn index_new_submitted_tx(
        &self,
        txid: &SerializedTxid,
        tx: &Transaction,
        mempool_entry: MempoolEntry,
    ) -> Result<()> {
        let mut cache =
            MempoolCache::new(self.db.clone(), MempoolCacheSettings::new(&self.settings))?;
        let mut events = Events::new();

        let _lock = self.broadcast_lock.lock().unwrap();
        if self.index_tx(txid, tx, mempool_entry.clone(), &mut cache, &mut events)? {
            events.add_event(Event::TransactionSubmitted {
                txid: *txid,
                entry: mempool_entry,
            });

            if self.settings.index_addresses {
                cache.add_address_events(&mut events, self.settings.chain);
            }

            cache.flush()?;
            if let Err(e) = events.send_events(&self.sender) {
                if !self.shutdown_flag.load(Ordering::SeqCst) {
                    error!("Failed to send events: {:?}", e);
                }
            }
        }

        self.remove_pre_index_new_submitted_transaction(&txid)?;
        Ok(())
    }

    pub fn index_zmq_tx(&self, txid: SerializedTxid, tx: Transaction) -> Result<()> {
        self.zmq_received_txs
            .write()
            .map_err(|_| UpdaterError::Mutex)?
            .insert(txid, tx);
        Ok(())
    }

    fn index_tx(
        &self,
        txid: &SerializedTxid,
        tx: &Transaction,
        mempool_entry: MempoolEntry,
        cache: &mut MempoolCache,
        events: &mut Events,
    ) -> Result<bool> {
        if cache.does_tx_exist(*txid)? {
            debug!("Skipping tx {} in mempool because it already exists", txid);
            return Ok(false);
        }

        let _timer = self.latency.with_label_values(&["index_tx"]).start_timer();

        let height = cache.get_block_count();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let rpc_client = self.bitcoin_rpc_pool.get()?;
        let mut transaction_parser =
            TransactionParser::new(&rpc_client, self.settings.chain, height, true)?;

        let result = transaction_parser.parse(cache, 0, tx)?;
        debug!("Indexing tx {}", txid);

        let mut transaction_updater = TransactionUpdater::new(self.settings.clone().into(), true)?;
        transaction_updater.save(cache, events, now as u32, None, *txid, tx, &result)?;

        cache.set_mempool_tx(*txid, mempool_entry);

        Ok(true)
    }

    fn remove_txs(&self, txids: &Vec<SerializedTxid>, mempool: bool) -> Result<()> {
        let db = self.db.write();
        let mut rollback_updater = Rollback::new(&db, self.settings.clone().into(), mempool)?;
        rollback_updater.revert_transactions(txids)?;
        Ok(())
    }

    fn open_progress_bar(&self, current_height: u64, total_height: u64) -> ProgressBar {
        let progress_bar: ProgressBar = ProgressBar::new(total_height.into());
        progress_bar.set_position(current_height.into());
        progress_bar.set_style(
            ProgressStyle::with_template("[indexing blocks] {wide_bar} {pos}/{len}").unwrap(),
        );

        progress_bar
    }

    fn detect_reorg(
        &self,
        block: &BitcoinBlock,
        height: u64,
        cache: &mut BlockCache,
        client: &Client,
        max_recoverable_reorg_depth: u64,
    ) -> std::result::Result<(), ReorgError> {
        if height == 0 {
            return Ok(());
        }

        let bitcoind_prev_blockhash = block.header.prev_blockhash;

        let prev_height = height.checked_sub(1);

        let Some(prev_height) = prev_height else {
            return Ok(());
        };

        // Regtest has no genesis block, so we don't need to check for reorgs.
        if prev_height == 0 && self.settings.chain == Chain::Regtest {
            return Ok(());
        }

        let index_prev_blockhash = cache.get_block_hash(prev_height)?;

        if index_prev_blockhash == bitcoind_prev_blockhash {
            return Ok(());
        }

        // Walk back up to max_recoverable_reorg_depth looking for a common ancestor.
        for depth in 1..max_recoverable_reorg_depth {
            let height_to_check = height.saturating_sub(depth);

            let index_block_hash = cache.get_block_hash(height_to_check)?;

            let bitcoind_block_hash = client.get_block_hash(height_to_check)?;

            if index_block_hash == bitcoind_block_hash {
                info!("Reorg until height {}. Depth: {}", height_to_check, depth);
                return Err(ReorgError::Recoverable { height, depth });
            }
        }

        Err(ReorgError::Unrecoverable)
    }

    fn handle_reorg(&self, height: u64, depth: u64) -> Result<()> {
        // we're not at tip anymore.
        self.mark_as_not_at_tip();

        info!(
            "Reorg detected at height {}, rolling back {} blocks",
            height, depth
        );

        {
            let db = self.db.write();

            // Roll back the persisted block count.
            // `height` refers to the height of the *next* block that was about to be indexed (i.e. equal to
            // the current block count). Since that block has not been written to the database yet, we only
            // want to keep blocks strictly below `height`. Therefore, after reverting `depth` blocks, the
            // new block count must be `height - depth` (the last retained height + 1).
            db.set_block_count(height - depth)?;
        }

        // Find rolled back blocks and revert those txs.
        // The block at `height` has not yet been stored, so begin reverting from `height - 1` down to
        // `height - depth`, inclusive. This rolls back exactly `depth` blocks.
        for i in 0..depth {
            let block_height_rolled_back = height - 1 - i;
            let block = self.get_block_by_height(block_height_rolled_back)?;
            self.revert_block(block_height_rolled_back as u32, &block)?;
        }

        Ok(())
    }

    fn get_block_by_height(&self, height: u64) -> Result<Block> {
        let db = self.db.read();
        let block_hash = db.get_block_hash(height)?;
        let block = db.get_block_by_hash(&block_hash)?;
        Ok(block)
    }

    fn revert_block(&self, height: u32, block: &Block) -> Result<()> {
        let db = self.db.write();

        let mut rollback_updater = Rollback::new(&db, self.settings.clone().into(), false)?;

        rollback_updater.revert_transactions(&block.tx_ids)?;

        {
            let mut transaction_update = self
                .transaction_update
                .write()
                .map_err(|_| UpdaterError::Mutex)?;

            for txid in block.tx_ids.iter().rev() {
                transaction_update.remove_block_tx(*txid);
            }
        }

        info!(
            "Reverted block {}:{} with {} txs",
            height,
            block.header.block_hash(),
            block.tx_ids.len()
        );

        // Delete block
        db.delete_block(&block.header.block_hash())?;
        db.delete_block_hash(height as u64)?;

        Ok(())
    }

    pub fn notify_tx_updates(&self, flush: bool) -> Result<()> {
        let _timer = self
            .latency
            .with_label_values(&["notify_tx_updates"])
            .start_timer();

        let (categorized, block_added_after_mined_and_reorg) = {
            let transaction_update = self
                .transaction_update
                .read()
                .map_err(|_| UpdaterError::Mutex)?;

            let change_set = transaction_update.categorize_to_change_set();
            let block_added_after_mined_and_reorg =
                transaction_update.block_added_after_mined_and_reorg();
            (change_set, block_added_after_mined_and_reorg)
        };

        if let Some(sender) = &self.sender {
            if !flush && (!categorized.removed.is_empty() || !categorized.added.is_empty()) {
                let client = self.bitcoin_rpc_pool.get()?;

                let chain_info = client.get_blockchain_info()?;

                let block = {
                    let db = self.db.read();
                    db.get_block_by_hash(&chain_info.best_block_hash)
                };

                let Ok(block) = block else {
                    return Err(UpdaterError::InvalidMainChainTip);
                };

                if block.height != chain_info.blocks {
                    return Err(UpdaterError::InvalidMainChainTip);
                }
            }

            if !categorized.removed.is_empty() {
                let (_, not_exists) = {
                    let db = self.db.read();
                    db.partition_transactions_by_existence(
                        &categorized.removed.into_iter().collect(),
                    )?
                };

                sender.blocking_send(Event::TransactionsReplaced { txids: not_exists })?;
            }

            // Final added set: normal newly-added UNION
            // the txs that were mined, reorged out, and re-mined (appeared in block_added more than in block_removed),
            let mut added = categorized.added;
            if !block_added_after_mined_and_reorg.is_empty() {
                for txid in block_added_after_mined_and_reorg {
                    added.insert(txid);
                }
            }

            if !added.is_empty() {
                sender.blocking_send(Event::TransactionsAdded {
                    txids: added.into_iter().collect(),
                })?;
            }
        }

        self.transaction_update
            .write()
            .map_err(|_| UpdaterError::Mutex)?
            .reset();

        Ok(())
    }

    fn insert_genesis_rune(&self) -> Result<()> {
        let rune = Rune(2055900680524219742);

        let id = RuneId::new(1, 0);
        let etching = SerializedTxid::all_zeros();

        let rune = RuneEntry {
            block: id.block,
            burned: 0,
            divisibility: 0,
            etching,
            terms: Some(Terms {
                amount: Some(1),
                cap: Some(u128::MAX),
                height: (
                    Some((SUBSIDY_HALVING_INTERVAL * 4).into()),
                    Some((SUBSIDY_HALVING_INTERVAL * 5).into()),
                ),
                offset: (None, None),
            }),
            mints: 0,
            number: 0,
            premine: 0,
            spaced_rune: SpacedRune { rune, spacers: 128 },
            symbol: Some('\u{29C9}'),
            timestamp: 0,
            turbo: true,
            pending_burns: 0,
            pending_mints: 0,
            inscription_id: None,
        };

        Ok(())
    }
}
