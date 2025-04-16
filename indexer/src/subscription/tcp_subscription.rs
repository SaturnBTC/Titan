use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use titan_types::{Event, EventType, TcpSubscriptionRequest};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch, RwLock},
};
use tracing::{debug, error, info};
use uuid::Uuid;

/// A subscription coming from a TCP client.
#[derive(Debug)]
pub struct TcpSubscription {
    pub id: Uuid,
    /// The set of event types (as strings) the client wants.
    pub event_types: HashSet<EventType>,
    /// Channel sender to deliver events to this client.
    pub sender: mpsc::Sender<Event>,
}

/// Manages all active TCP subscriptions.
#[derive(Default, Debug)]
pub struct TcpSubscriptionManager {
    subscriptions: RwLock<HashMap<Uuid, TcpSubscription>>,
}

impl TcpSubscriptionManager {
    pub fn new() -> Self {
        Self {
            subscriptions: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new TCP subscription.
    pub async fn register(&self, sub: TcpSubscription) {
        self.subscriptions.write().await.insert(sub.id, sub);
    }

    /// Unregister a subscription by its id.
    pub async fn unregister(&self, id: Uuid) {
        self.subscriptions.write().await.remove(&id);
    }

    /// Broadcast an event to all subscriptions that have registered interest.
    pub async fn broadcast(&self, event: &Event) {
        // Assume you can derive a string event type from your event.
        // For example, if you have a function or trait implementation:
        let event_type: EventType = EventType::from(event.clone()); // adjust as needed

        let subs = self.subscriptions.read().await;
        let mut failed_ids = Vec::new();

        for (id, sub) in subs.iter() {
            if sub.event_types.contains(&event_type) {
                // Try sending the event; if it fails (e.g. channel closed) log the error.
                if let Err(e) = sub.sender.send(event.clone()).await {
                    error!("Failed to send event to subscription {}: {:?}", id, e);
                    failed_ids.push(*id);
                }
            }
        }

        // Drop the read lock before removing subscriptions
        drop(subs);

        // Remove any subscriptions that failed to receive events
        for id in failed_ids {
            self.unregister(id).await;
            info!("Unregistered failed subscription with id {}", id);
        }
    }
}

/// Run the TCP subscription server on the given address.
/// This server listens for incoming TCP connections and spawns a task
/// to handle each connection.
pub async fn run_tcp_subscription_server(
    addr: &str,
    manager: Arc<TcpSubscriptionManager>,
    mut shutdown_rx: watch::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(addr).await?;
    info!("TCP Subscription Server listening on {}", addr);

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                let (socket, remote_addr) = accept_result?;
                info!("New TCP connection from {}", remote_addr);
                let manager_clone = manager.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_tcp_connection(socket, manager_clone).await {
                        error!("Error handling TCP connection from {}: {:?}", remote_addr, e);
                    }
                });
            }
            _ = shutdown_rx.changed() => {
                info!("TCP Subscription Server shutting down");
                break;
            }
        }
    }

    Ok(())
}

/// Handle a single TCP connection:
/// 1. Read a line (JSON) from the client specifying the event types to subscribe to.
/// 2. Create an mpsc channel and register a subscription.
/// 3. Spawn a task to forward events from the channel to the client.
/// 4. Also monitor the connection (for further commands or disconnection) so that when the client disconnects, the subscription is removed.
async fn handle_tcp_connection(
    socket: TcpStream,
    manager: Arc<TcpSubscriptionManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Split the socket into reader and writer.
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);
    let mut buf = String::new();

    // Read the first line containing the subscription request.
    let n = reader.read_line(&mut buf).await?;
    if n == 0 {
        return Err("Connection closed before sending subscription request".into());
    }
    let request: TcpSubscriptionRequest = serde_json::from_str(buf.trim())?;
    info!("Received TCP subscription request: {:?}", request);

    let event_types: HashSet<EventType> = request.subscribe.into_iter().collect();

    // Create an mpsc channel for delivering events to this connection.
    let (tx, mut rx) = mpsc::channel::<Event>(100);
    let sub = TcpSubscription {
        id: Uuid::new_v4(),
        event_types,
        sender: tx,
    };
    let sub_id = sub.id;
    manager.register(sub).await;
    info!("Registered TCP subscription with id {}", sub_id);

    // Clear the buffer for subsequent reads
    buf.clear();

    // Loop until the connection is closed.
    loop {
        tokio::select! {
            // Send events received from the channel to the client.
            maybe_event = rx.recv() => {
                match maybe_event {
                    Some(event) => {
                        let json = serde_json::to_string(&event)?;
                        writer.write_all(json.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                    },
                    None => {
                        info!("Event channel closed for subscription {}", sub_id);
                        break;
                    }
                }
            }
            // Also monitor the connection for any client input (to detect disconnect).
            result = reader.read_line(&mut buf) => {
                match result {
                    Ok(0) => {
                        info!("TCP client disconnected for subscription {}", sub_id);
                        break;
                    },
                    Ok(_) => {
                        // Check if this is a PING message
                        let trimmed = buf.trim();
                        if trimmed == "PING" {
                            if let Err(e) = writer.write_all(b"PONG\n").await {
                                error!("Failed to send PONG: {:?}", e);
                                break;
                            }
                            if let Err(e) = writer.flush().await {
                                error!("Failed to flush after PONG: {:?}", e);
                                break;
                            }
                        } else {
                            // Log any other messages for debugging
                            debug!("Received message from client: {}", trimmed);
                        }
                        // Clear the buffer for the next read
                        buf.clear();
                    },
                    Err(e) => {
                        error!("Error reading from TCP connection: {:?}", e);
                        break;
                    }
                }
            }
        }
    }

    manager.unregister(sub_id).await;
    info!("Unregistered TCP subscription with id {}", sub_id);
    Ok(())
}
