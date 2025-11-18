use {
    super::tcp_subscription::TcpSubscriptionManager,
    crate::subscription::WebhookSubscriptionManager,
    chrono::{DateTime, Utc},
    std::{fs::OpenOptions, io::Write, sync::Arc, time::SystemTime},
    titan_types_core::Event,
    tokio::{
        select,
        sync::{mpsc, watch},
    },
    tracing::{error, info},
};

/// Asynchronously receive events from `receiver` and process them.
/// If a shutdown signal arrives, exit gracefully.
pub async fn event_dispatcher(
    mut receiver: mpsc::Receiver<Event>,
    subscription_manager: Option<Arc<WebhookSubscriptionManager>>,
    tcp_subscription_manager: Option<Arc<TcpSubscriptionManager>>,
    mut shutdown_rx: watch::Receiver<()>,
    enable_file_logging: bool,
) {
    info!("event_dispatcher started");
    loop {
        select! {
            maybe_event = receiver.recv() => {
                match maybe_event {
                    Some(event) => {
                        // Process the event, e.g. dispatch to subscribed endpoints
                        if let Some(manager) = &subscription_manager {
                            if let Err(e) = manager.broadcast(&event).await {
                                error!("Error processing event: {:?}", e);
                            }
                        }

                        if enable_file_logging {
                            if matches!(
                                event,
                                Event::TransactionsAdded { .. }
                                | Event::TransactionsReplaced { .. }
                                | Event::NewBlock { .. }
                                | Event::Reorg { .. }
                            ) {
                                append_to_file("events.log", &format!("{:?}", event)).unwrap();
                            }
                        }

                        // Broadcast to TCP subscribers
                        if let Some(manager) = &tcp_subscription_manager {
                            manager.broadcast(&event).await;
                        }
                    },
                    None => {
                        // The sender side was dropped, so no more events
                        info!("event_dispatcher: channel closed, exiting");
                        break;
                    }
                }
            }

            // If we ever get a shutdown signal, exit
            _ = shutdown_rx.changed() => {
                info!("event_dispatcher received shutdown signal, exiting.");
                break;
            }
        }
    }

    info!("event_dispatcher ended");
}

fn append_to_file(path: &str, content: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true) // Create the file if it doesn't exist
        .append(true) // Open in append mode
        .open(path)?;

    let unix_timestamp = DateTime::<Utc>::from(SystemTime::now())
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    writeln!(file, "[{:?}] {:?}", unix_timestamp, content)?;
    Ok(())
}
