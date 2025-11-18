use bitcoin::Transaction;
use bitcoincore_rpc::json::GetMempoolEntryResult;
use bitcoincore_rpc::RpcApi;
use rustc_hash::FxHashMap as HashMap;
use std::cmp::min;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use threadpool::ThreadPool;
use titan_types_core::SerializedTxid;
use tracing::error;

use crate::bitcoin_rpc::{RpcClientError, RpcClientPool};

#[derive(thiserror::Error, Debug)]
pub enum MempoolError {
    #[error("rpc error: {0}")]
    RpcError(#[from] bitcoincore_rpc::Error),
    #[error("rpc client error: {0}")]
    RpcClientError(#[from] RpcClientError),
    #[error("cycle detected in transaction dependencies")]
    CycleDetected,
}

/// Fetches transactions concurrently using a dedicated thread pool.
/// Each task gets a new RPC client from the provider and fetches the raw transaction.
/// The results are sent over a channel and then collected into a HashMap.
pub fn fetch_transactions(
    bitcoin_rpc_pool: RpcClientPool,
    txids: &Vec<SerializedTxid>,
    interrupt: Arc<AtomicBool>,
) -> HashMap<SerializedTxid, Transaction> {
    // Create an MPSC channel to collect fetched transactions.
    let (sender, receiver) = mpsc::channel();
    // Create a thread pool sized for I/O-bound tasks.
    // Adjust `num_threads` based on your environment and expected concurrency.
    let num_threads = min(50, txids.len());
    let pool = ThreadPool::new(num_threads);

    // For each txid, submit a task to the pool.
    for &txid in txids.iter() {
        let bitcoin_rpc_pool = bitcoin_rpc_pool.clone();
        let sender = sender.clone();
        let interrupt = interrupt.clone();
        pool.execute(move || {
            // If an interrupt has been signaled, skip fetching.
            if interrupt.load(Ordering::SeqCst) {
                return;
            }
            // Get a new RPC client.
            let client = match bitcoin_rpc_pool.get() {
                Ok(c) => c,
                Err(e) => {
                    error!("Failed to get new RPC client for txid {}: {}", txid, e);
                    return;
                }
            };
            // Fetch the transaction.
            match client.get_raw_transaction(&txid.into(), None) {
                Ok(tx) => {
                    if let Err(e) = sender.send((txid, tx)) {
                        error!("Failed to send transaction {} over channel: {}", txid, e);
                    }
                }
                Err(e) => {
                    error!("Failed to fetch transaction {}: {}", txid, e);
                }
            }
        });
    }
    // Drop the original sender so that the channel closes when all tasks are done.
    drop(sender);
    // Wait for all tasks in the pool to complete.
    pool.join();

    // Collect all received transactions into a HashMap.
    receiver.into_iter().collect()
}

/// Sort transactions in dependency order using Kahn's algorithm.
pub fn sort_transaction_order(
    mempool_entries: &HashMap<SerializedTxid, GetMempoolEntryResult>,
    tx_map: &HashMap<SerializedTxid, Transaction>,
) -> Result<Vec<SerializedTxid>, MempoolError> {
    // Step 2: Build dependency graph and indegree count
    let mut graph: HashMap<SerializedTxid, Vec<SerializedTxid>> = HashMap::default();
    let mut indegree: HashMap<SerializedTxid, usize> = HashMap::default();

    // Initialize graph nodes
    for &txid in tx_map.keys() {
        graph.insert(txid, Vec::new());
        indegree.insert(txid, 0);
    }

    // Populate graph with edges and compute indegrees
    for (&txid, tx) in tx_map.iter() {
        for input in &tx.input {
            let dep_txid = input.previous_output.txid.into();
            // Only consider dependencies that are part of our tx_map and in mempool_entries
            if tx_map.contains_key(&dep_txid) && mempool_entries.contains_key(&dep_txid) {
                graph.entry(dep_txid).or_default().push(txid);
                *indegree.entry(txid).or_insert(0) += 1;
            }
        }
    }

    // Step 3: Topological sort using Kahn's algorithm
    let mut queue: VecDeque<SerializedTxid> = indegree
        .iter()
        .filter_map(|(&txid, &deg)| if deg == 0 { Some(txid) } else { None })
        .collect();

    let mut sorted = Vec::with_capacity(tx_map.len());

    while let Some(node) = queue.pop_front() {
        sorted.push(node);

        if let Some(neighbors) = graph.get(&node) {
            for &neighbor in neighbors {
                if let Some(count) = indegree.get_mut(&neighbor) {
                    *count -= 1;
                    if *count == 0 {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
    }

    // If sorted transactions count doesn't match input transactions count, a cycle exists.
    if sorted.len() != tx_map.len() {
        return Err(MempoolError::CycleDetected);
    }

    Ok(sorted)
}
