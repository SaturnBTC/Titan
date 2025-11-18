use {
    super::{tcp_subscription::TcpSubscriptionManager, WebhookSubscriptionManager},
    crate::{
        db::RocksDB,
        subscription::{
            dispatcher::event_dispatcher, tcp_subscription::run_tcp_subscription_server,
            webhook::cleanup_inactive_subscriptions,
        },
    },
    std::{sync::Arc, time::Duration},
    titan_types_core::Event,
    tokio::{
        sync::{mpsc, watch},
        task,
    },
    tracing::{error, info},
};

const DEFAULT_CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);
const DEFAULT_CLEANUP_EXPIRY_SECS: u64 = 86400;

pub struct WebhookSubscriptionSpawnResult {
    pub cleanup_handle: task::JoinHandle<()>,
    pub subscription_manager: Arc<WebhookSubscriptionManager>,
}

pub struct TcpSubscriptionSpawnResult {
    pub tcp_server_handle: task::JoinHandle<()>,
    pub tcp_subscription_manager: Arc<TcpSubscriptionManager>,
}

pub struct SubscriptionSpawnResult {
    pub event_sender: mpsc::Sender<Event>,
    pub dispatcher_handle: task::JoinHandle<()>,
    pub webhook_spawn_result: Option<WebhookSubscriptionSpawnResult>,
    pub tcp_spawn_result: Option<TcpSubscriptionSpawnResult>,
    pub shutdown_tx: watch::Sender<()>,
}

pub struct SubscriptionConfig {
    pub enable_webhook_subscriptions: bool,
    pub enable_tcp_subscriptions: bool,
    pub tcp_address: String,
    pub enable_file_logging: bool,
}

/// Spawns the subscription-related background tasks (dispatcher + cleanup).
pub fn spawn_subscription_tasks(
    db: Arc<RocksDB>,
    config: SubscriptionConfig,
) -> Option<SubscriptionSpawnResult> {
    // If both webhook and TCP subscriptions are disabled, return None
    if !config.enable_webhook_subscriptions && !config.enable_tcp_subscriptions {
        return None;
    }

    // Create a watch channel for shutdown signaling
    let (shutdown_tx, shutdown_rx) = watch::channel(());

    // Create the TCP subscription manager if enabled
    let tcp_spawn_result = if config.enable_tcp_subscriptions {
        let tcp_subscription_manager = Arc::new(TcpSubscriptionManager::new());
        let tcp_subscription_manager_clone = tcp_subscription_manager.clone();
        let shutdown_rx_clone = shutdown_rx.clone();

        let tcp_server_handle = tokio::spawn(async move {
            if let Err(e) = run_tcp_subscription_server(
                &config.tcp_address,
                tcp_subscription_manager_clone,
                shutdown_rx_clone,
            )
            .await
            {
                error!("TCP subscription server error: {:?}", e);
            }
        });

        Some(TcpSubscriptionSpawnResult {
            tcp_server_handle,
            tcp_subscription_manager,
        })
    } else {
        None
    };

    // Create the webhook subscription manager if enabled
    let webhook_spawn_result = if config.enable_webhook_subscriptions {
        let webhook_subscription_manager = Arc::new(WebhookSubscriptionManager::new(db.clone()));

        let cleanup_rx = shutdown_rx.clone();
        let cleanup_db = db.clone();

        let cleanup_handle = tokio::spawn(async move {
            cleanup_inactive_subscriptions(
                cleanup_db,
                DEFAULT_CLEANUP_INTERVAL,
                DEFAULT_CLEANUP_EXPIRY_SECS,
                cleanup_rx,
            )
            .await;
        });

        Some(WebhookSubscriptionSpawnResult {
            cleanup_handle,
            subscription_manager: webhook_subscription_manager,
        })
    } else {
        None
    };

    // Create the event sender
    let (event_sender, event_receiver) = mpsc::channel::<Event>(10000);

    // Clone the receiver for the dispatcher
    let dispatcher_rx = shutdown_rx.clone();

    // Extract clones of the inner Arcs (if present) so we don’t move the original Options.
    let webhook_manager_for_dispatcher = webhook_spawn_result
        .as_ref()
        .map(|r| r.subscription_manager.clone());
    let tcp_manager_for_dispatcher = tcp_spawn_result
        .as_ref()
        .map(|r| r.tcp_subscription_manager.clone());

    let dispatcher_handle = tokio::spawn(async move {
        event_dispatcher(
            event_receiver,
            webhook_manager_for_dispatcher,
            tcp_manager_for_dispatcher,
            dispatcher_rx,
            config.enable_file_logging,
        )
        .await;
    });

    info!("Spawned subscription tasks (dispatcher + cleanup).");

    Some(SubscriptionSpawnResult {
        event_sender,
        dispatcher_handle,
        webhook_spawn_result,
        tcp_spawn_result,
        shutdown_tx,
    })
}

pub async fn shutdown_and_wait_subscription_tasks(spawn_result: SubscriptionSpawnResult) {
    let SubscriptionSpawnResult {
        dispatcher_handle,
        webhook_spawn_result,
        tcp_spawn_result,
        shutdown_tx,
        ..
    } = spawn_result;

    if let Err(e) = shutdown_tx.send(()) {
        error!("Failed to send shutdown signal: {:?}", e);
    }

    if let Err(e) = dispatcher_handle.await {
        error!("Dispatcher task join error: {:?}", e);
    } else {
        info!("Dispatcher task ended cleanly.");
    }

    if let Some(webhook_spawn_result) = webhook_spawn_result {
        if let Err(e) = webhook_spawn_result.cleanup_handle.await {
            error!("Webhook cleanup task join error: {:?}", e);
        } else {
            info!("Webhook cleanup task ended cleanly.");
        }
    }

    if let Some(tcp_spawn_result) = tcp_spawn_result {
        if let Err(e) = tcp_spawn_result.tcp_server_handle.await {
            error!("TCP server task join error: {:?}", e);
        } else {
            info!("TCP server task ended cleanly.");
        }
    }
}
