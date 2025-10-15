use axum_server::Handle;
use bitcoin_rpc::{validate_rpc_connection, RpcClientPool, RpcClientProvider};
use clap::Parser;
use db::RocksDB;
use index::{Index, Settings};
use options::Options;
use server::{Server, ServerConfig};
use std::{io, panic, sync::Arc};
use subscription::{
    shutdown_and_wait_subscription_tasks, spawn_subscription_tasks, SubscriptionSpawnResult,
    WebhookSubscriptionManager,
};
use tokio::{
    signal::unix::{signal, SignalKind},
    task,
};
use tracing::{error, info};

mod api;
mod bitcoin_rpc;
mod db;
mod index;
mod models;
mod options;
mod server;
mod subscription;
mod util;
mod alkanes;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Set up the global tracing subscriber for logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // 2. Parse command-line options
    let options = parse_options()?;

    // 3. Prepare and validate configurations
    let settings = setup_settings(&options)?;
    let server_config = setup_server_config(&options)?;
    validate_rpc(&settings)?;

    // 4. Open RocksDB
    let db_arc = open_rocks_db(&settings)?;
    set_panic_hook(db_arc.clone());

    // 5. If subscriptions are enabled, spawn the dispatcher + cleanup tasks
    let spawn_subscription_result =
        spawn_subscription_tasks(db_arc.clone(), options.clone().into());

    let (webhook_subscription_manager, event_sender) = match spawn_subscription_result.as_ref() {
        Some(sub) => (
            sub.webhook_spawn_result
                .as_ref()
                .map(|r| r.subscription_manager.clone()),
            Some(sub.event_sender.clone()),
        ),
        None => (None, None),
    };

    // 6. Create the index
    let bitcoin_rpc_pool = RpcClientPool::new(
        Arc::new(settings.clone()),
        options.bitcoin_rpc_pool_size as usize,
    );

    let index = Arc::new(Index::new(
        db_arc.clone(),
        bitcoin_rpc_pool.clone(),
        settings.clone(),
        event_sender,
    ));
    index.validate_index()?;

    // 7. Spawn background threads (indexer, ZMQ listener, etc.)
    //    We also receive a signal (`index_shutdown_rx`) that fires when the indexer thread
    //    terminates unexpectedly (e.g., due to `Failed to update to tip`). When that happens
    //    we will initiate a graceful shutdown without relying on external signals.
    let (index_handle, index_shutdown_rx) =
        spawn_background_threads(index.clone(), options.enable_zmq_listener).await;

    // 8. Start the HTTP server
    let handle = Handle::new();
    let server = Server;
    let http_server_jh = server.start(
        index.clone(),
        webhook_subscription_manager
            .unwrap_or(Arc::new(WebhookSubscriptionManager::new(db_arc.clone()))),
        bitcoin_rpc_pool.clone(),
        Arc::new(server_config),
        handle.clone(),
    )?;

    // 9. Wait for SIGINT (Ctrl-C), SIGTERM, or the indexer thread finishing unexpectedly
    wait_for_signals(index_shutdown_rx).await;

    // 10. Graceful shutdown (async)
    graceful_shutdown(
        index,
        spawn_subscription_result,
        db_arc,
        &handle,
        index_handle,
        http_server_jh,
    )
    .await;

    Ok(())
}

/// Parse CLI options
fn parse_options() -> Result<Options, Box<dyn std::error::Error>> {
    let options = Options::parse();
    Ok(options)
}

/// Convert `Options` to your local `Settings` struct, etc.
fn setup_settings(options: &Options) -> Result<Settings, Box<dyn std::error::Error>> {
    let settings = Settings::from(options.clone());
    Ok(settings)
}

fn setup_server_config(options: &Options) -> Result<ServerConfig, Box<dyn std::error::Error>> {
    let config = ServerConfig::from(options.clone());
    Ok(config)
}

/// Validate the RPC connection using your `validate_rpc_connection`
fn validate_rpc(settings: &Settings) -> Result<(), Box<dyn std::error::Error>> {
    validate_rpc_connection(settings.get_new_rpc_client()?, settings.chain)?;
    Ok(())
}

/// Open RocksDB, returning an `Arc<RocksDB>`
fn open_rocks_db(settings: &Settings) -> Result<Arc<RocksDB>, Box<dyn std::error::Error>> {
    let file = settings.chain.to_string();
    let db_path = settings.data_dir.join(file);
    let db_instance = RocksDB::open(db_path.to_str().unwrap())?;
    Ok(Arc::new(db_instance))
}

/// Set a panic hook that closes or flushes the DB on panic
fn set_panic_hook(db_arc: Arc<RocksDB>) {
    let db_for_panic = db_arc.clone();
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        // Print the panic using the original hook
        original_hook(panic_info);

        if let Some(message) = panic_info.payload().downcast_ref::<String>() {
            error!("Panic occurred: {}, shutting down...", message);
        } else {
            error!("Panic occurred: {:?}, shutting down...", panic_info);
        }

        // Attempt to flush DB
        if let Err(e) = db_for_panic.flush() {
            error!("Failed to flush RocksDB in panic hook: {:?}", e);
        }
        std::process::exit(1);
    }));
}

/// Spawn background threads: indexer loop, ZMQ listener, etc. Return their JoinHandle.
async fn spawn_background_threads(
    index: Arc<Index>,
    enable_zmq_listener: bool,
) -> (
    std::thread::JoinHandle<()>,
    tokio::sync::oneshot::Receiver<()>,
) {
    use tokio::sync::oneshot;

    // Channel used to notify the main async runtime that the indexer thread has
    // finished – either normally or because it encountered a fatal error.
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    // 1) Spawn the indexer loop in a blocking thread
    let index_clone = index.clone();
    let index_handle = std::thread::spawn(move || {
        index_clone.index();
        // Ignore send errors – it just means the receiver was dropped.
        let _ = shutdown_tx.send(());
    });

    // 2) Spawn the ZMQ listener (also likely blocking)
    if enable_zmq_listener {
        index.start_zmq_listener().await;
    }

    info!("Spawned background threads");
    (index_handle, shutdown_rx)
}

/// Block until either SIGINT or SIGTERM is received
async fn wait_for_signals(index_shutdown_rx: tokio::sync::oneshot::Receiver<()>) {
    use tokio::select;
    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to open signal stream");

    select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received SIGINT (Ctrl-C), shutting down...");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM, shutting down...");
        }
        _ = index_shutdown_rx => {
            info!("Indexer thread terminated, shutting down...");
        }
    }
}

/// Perform graceful shutdown **asynchronously**, then close RocksDB if possible.
async fn graceful_shutdown(
    index: Arc<Index>,
    spawn_subscription_result: Option<SubscriptionSpawnResult>,
    db_arc: Arc<RocksDB>,
    handle: &Handle,
    index_handle: std::thread::JoinHandle<()>,
    http_server_jh: task::JoinHandle<io::Result<()>>,
) {
    // 1) Tell the Index to shut down
    index.shutdown();

    // 2) Reset the panic hook to drop the RocksDB reference
    panic::set_hook(Box::new(|panic_info| {
        // Restore the default hook
        eprintln!("Panic occurred: {:?}", panic_info);
    }));

    // 3) Graceful HTTP shutdown (axum_server)
    handle.graceful_shutdown(Some(std::time::Duration::from_secs(2)));

    // 4) Join the indexer background thread (blocking)
    if let Err(e) = index_handle.join() {
        error!("Failed to join indexer thread: {:?}", e);
    }

    // 5) Await the Axum server
    match http_server_jh.await {
        Ok(Ok(_)) => info!("Axum server finished cleanly."),
        Ok(Err(e)) => error!("Server error: {:?}", e),
        Err(e) => error!("Failed to join Axum server task: {:?}", e),
    };

    // 6) Drop the index so RocksDB references can possibly be unwrapped
    drop(index);

    // 7) Signal the subscription tasks to stop
    if let Some(result) = spawn_subscription_result {
        shutdown_and_wait_subscription_tasks(result).await;
    }

    // 8) Attempt to close RocksDB
    match Arc::try_unwrap(db_arc) {
        Ok(db) => {
            if let Err(e) = db.close() {
                error!("Error while closing RocksDB: {:?}", e);
            } else {
                info!("RocksDB closed successfully (normal shutdown).");
            }
        }
        Err(db_ref) => {
            // If there are still outstanding references, flush as fallback
            error!(
                "Still {} references to RocksDB exist, flushing before exit...",
                Arc::strong_count(&db_ref)
            );
            let _ = db_ref.flush();
        }
    }

    info!("Graceful shutdown complete. Exiting.");
}
