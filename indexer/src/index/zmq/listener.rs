use {
    crate::index::updater::Updater,
    async_zmq::{
        subscribe, Error as AsyncZmqError, Multipart, RecvError, SocketError, StreamExt,
        SubscribeError,
    },
    bitcoin::{consensus::encode, Transaction},
    std::sync::Arc,
    tokio::sync::watch,
    tracing::{debug, error, info},
};

#[derive(Debug, thiserror::Error)]
pub enum ZmqError {
    #[error("ZMQ read error: {0}")]
    ReadError(#[from] RecvError),

    #[error("ZMQ socket error: {0}")]
    SocketError(#[from] SocketError),

    #[error("ZMQ error: {0}")]
    AsyncZmqError(#[from] AsyncZmqError),

    #[error("ZMQ subscribe error: {0}")]
    SubscribeError(#[from] SubscribeError),
}

/// Asynchronous ZMQ listener
pub async fn zmq_listener(
    updater: Arc<Updater>,
    endpoint: String,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<(), ZmqError> {
    // 1. Create and connect an async subscriber socket
    info!("Connecting to ZMQ at {endpoint}");
    let mut sub = subscribe(&endpoint)?.connect()?;

    // 2. Subscribe to "rawtx"
    sub.set_subscribe("rawtx")?;
    debug!("Subscribed to ZMQ topic: rawtx");

    // 3. Main loop
    loop {
        tokio::select! {
            // Check for shutdown signal changes
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("Shutdown signal received, stopping ZMQ listener...");
                    break;
                }
            }

            // Receive next ZMQ message from the stream using .next()
            msg = sub.next() => {
                match msg {
                    Some(msg_result) => {
                        let frames: Multipart = match msg_result {
                            Ok(frames) => frames,
                            Err(e) => {
                                error!("ZMQ read error: {:?}", e);
                                return Err(ZmqError::ReadError(e));
                            }
                        };

                        // Handle the received frames
                        if let Err(e) = process_zmq_message(&updater, frames).await {
                            error!("Failed to process message: {:?}", e);
                        }
                    },
                    // If the stream is exhausted, break out of the loop.
                    None => {
                        info!("ZMQ stream ended");
                        break;
                    }
                }
            }
        }
    }

    info!("ZMQ listener stopped.");
    Ok(())
}

/// Process the frames from a rawtx message
async fn process_zmq_message(
    updater: &Arc<Updater>,
    frames: Multipart,
) -> Result<(), Box<dyn std::error::Error>> {
    if frames.len() < 2 {
        // Typically [topic_frame, data_frame] or more
        error!("Received unexpected frame count: {}", frames.len());
        return Ok(());
    }

    let topic = std::str::from_utf8(&frames[0])?;
    let payload = &frames[1];

    match topic {
        "rawtx" => {
            debug!("Received rawtx, size={} bytes", payload.len());
            handle_raw_tx(updater, payload)?;
        }
        other => {
            error!("Unknown ZMQ topic: {}", other);
        }
    }

    // If there are extra frames (like sequence numbers), you can handle them here or ignore.
    Ok(())
}

/// Decode and store a raw transaction in RocksDB (example)
fn handle_raw_tx(
    updater: &Arc<Updater>,
    tx_bytes: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let tx = encode::deserialize::<Transaction>(tx_bytes)?;
    let txid = tx.compute_txid().into();

    // If the updater is at tip, spawn a new thread to index the transaction.
    if updater.is_at_tip() {
        // Clone the Arc so the thread can use it.
        let updater_clone = Arc::clone(updater);

        match updater_clone.index_zmq_tx(txid, tx) {
            Ok(()) => debug!("Indexed tx {}", txid),
            Err(e) => error!("Failed to index tx {}: {:?}", txid, e),
        }
    }

    Ok(())
}
