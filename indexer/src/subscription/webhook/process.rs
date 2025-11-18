use {
    super::store::Store,
    reqwest::Client,
    std::{
        sync::Arc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    },
    thiserror::Error,
    titan_types_core::{Event, EventType},
    tokio::time::sleep,
    tracing::error,
};

#[derive(Debug, Error)]
pub enum SendEventError {
    #[error("reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("timeout after {attempts} retries")]
    Timeout { attempts: usize },
}

// Helper to send event to endpoint with retry logic
async fn send_event_with_retry(
    client: &Client,
    endpoint: &str,
    event: &Event,
    max_retries: usize,
) -> Result<(), SendEventError> {
    let mut attempt = 0;
    loop {
        let res = client.post(endpoint).json(event).send().await;

        match res {
            Ok(response) if response.status().is_success() => {
                // SUCCESS - stop retrying
                return Ok(());
            }
            Ok(response) => {
                error!("Non-success HTTP status: {}", response.status());
            }
            Err(e) => {
                error!("HTTP request error: {}", e);
            }
        }

        attempt += 1;
        if attempt >= max_retries {
            error!("Max retries reached for endpoint: {}", endpoint);
            return Err(SendEventError::Timeout {
                attempts: max_retries,
            });
        }

        // Exponential backoff
        let delay = Duration::from_secs(2u64.pow(attempt as u32));
        sleep(delay).await;
    }
}

/// Process an event and send it to all interested webhook subscriptions
pub async fn process_event(
    store: &Arc<dyn Store>,
    client: &Client,
    event: &Event,
) -> Result<(), Box<dyn std::error::Error>> {
    // Determine event type
    let event_type = EventType::from(event.clone());

    // Get all subscriptions from DB
    if let Ok(subscriptions) = store.get_subscriptions() {
        // Filter subscriptions interested in this event type
        let interested: Vec<_> = subscriptions
            .into_iter()
            .filter(|sub| sub.event_types.contains(&event_type))
            .collect();

        // For each subscription, dispatch the event asynchronously
        for sub in interested {
            let client_clone = client.clone();
            let endpoint = sub.endpoint.clone();
            let event_clone = event.clone();
            let store_clone = store.clone();
            tokio::spawn(async move {
                let result = send_event_with_retry(&client_clone, &endpoint, &event_clone, 5).await;

                if result.is_ok() {
                    // Mark subscription as successful
                    let now_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let _ = store_clone.update_subscription_last_success(&sub.id, now_secs);
                }
            });
        }
    }

    Ok(())
}
