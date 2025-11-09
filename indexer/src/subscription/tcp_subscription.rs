use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::sync::Arc;
use std::time::Duration;
use titan_types::{Event, EventType, TcpSubscriptionRequest};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::error::TrySendError;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch, RwLock},
};
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LinesCodec};
use tracing::{debug, error, info};
use uuid::Uuid;

const MAX_LINE: usize = 8 * 1024;
const HANDSHAKE_TIMEOUT_SECS: u64 = 10;

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
            subscriptions: RwLock::new(HashMap::default()),
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
                // Non-blocking send to avoid stalling the dispatcher on slow clients.
                match sub.sender.try_send(event.clone()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        info!(
                            "Dropping event and evicting slow TCP subscriber {} due to full channel",
                            id
                        );
                        failed_ids.push(*id);
                    }
                    Err(TrySendError::Closed(_)) => {
                        info!(
                            "Evicting TCP subscriber {} because its channel is closed",
                            id
                        );
                        failed_ids.push(*id);
                    }
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
    let mut lines = FramedRead::new(reader, LinesCodec::new_with_max_length(MAX_LINE));

    // Read subscription request with timeout.
    let request: TcpSubscriptionRequest = tokio::time::timeout(
        Duration::from_secs(HANDSHAKE_TIMEOUT_SECS),
        read_handshake_request(&mut lines, &mut writer),
    )
    .await??;

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

    // Loop until the connection is closed.
    loop {
        tokio::select! {
            // Send events received from the channel to the client.
            maybe_event = rx.recv() => {
                match maybe_event {
                    Some(event) => {
                        let json_bytes = serde_json::to_vec(&event)?;
                        writer.write_all(&json_bytes).await?;
                        writer.write_all(b"\n").await?;
                    },
                    None => {
                        info!("Event channel closed for subscription {}", sub_id);
                        break;
                    }
                }
            }
            // Also monitor the connection for any client input (to detect disconnect).
            result = lines.next() => {
                match result {
                    Some(Ok(line)) => {
                        let trimmed = line.trim();
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
                            debug!("Received message from client: {}", trimmed);
                        }
                    }
                    Some(Err(e)) => {
                        error!("Error reading from TCP connection: {:?}", e);
                        break;
                    }
                    None => {
                        info!("TCP client disconnected for subscription {}", sub_id);
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

async fn read_handshake_request(
    lines: &mut FramedRead<OwnedReadHalf, LinesCodec>,
    writer: &mut OwnedWriteHalf,
) -> Result<TcpSubscriptionRequest, Box<dyn std::error::Error>> {
    // Be tolerant to empty lines and early PINGs from clients or probes.
    // Put a small cap on how many preamble lines we accept to avoid abuse.
    const MAX_PREAMBLE_LINES: usize = 10;
    let mut preamble_lines = 0usize;
    loop {
        match lines.next().await {
            Some(Ok(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    preamble_lines += 1;
                    if preamble_lines >= MAX_PREAMBLE_LINES {
                        return Err("Too many empty lines before subscription request".into());
                    }
                    continue;
                }
                if trimmed == "PING" {
                    if let Err(e) = writer.write_all(b"PONG\n").await {
                        error!("Failed to send PONG: {:?}", e);
                        return Err("Failed to send PONG".into());
                    }
                    if let Err(e) = writer.flush().await {
                        error!("Failed to flush after PONG: {:?}", e);
                        return Err("Failed to flush after PONG".into());
                    }
                    preamble_lines += 1;
                    if preamble_lines >= MAX_PREAMBLE_LINES {
                        return Err("Too many preamble lines before subscription request".into());
                    }
                    continue;
                }
                if trimmed.starts_with('{') {
                    match serde_json::from_str::<TcpSubscriptionRequest>(trimmed) {
                        Ok(req) => return Ok(req),
                        Err(e) => {
                            error!(
                                "Failed to parse TCP subscription request JSON: {} (data starts: {:?})",
                                e,
                                &trimmed.chars().take(80).collect::<String>()
                            );
                            return Err("Invalid subscription request JSON".into());
                        }
                    }
                } else {
                    debug!(
                        "Ignoring non-JSON preface before subscription: {:?}",
                        &trimmed.chars().take(80).collect::<String>()
                    );
                    preamble_lines += 1;
                    if preamble_lines >= MAX_PREAMBLE_LINES {
                        return Err(
                            "Too many non-JSON preface lines before subscription request".into(),
                        );
                    }
                    continue;
                }
            }
            Some(Err(e)) => {
                error!("Error reading handshake line: {:?}", e);
                return Err(e.into());
            }
            None => {
                return Err("Connection closed before sending subscription request".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_util::codec::{FramedRead, LinesCodec};

    async fn setup_pair(
    ) -> Result<(TcpStream, OwnedReadHalf, OwnedWriteHalf), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let client = TcpStream::connect(addr).await?;
        let (server, _) = listener.accept().await?;
        let (server_read, server_write) = server.into_split();
        Ok((client, server_read, server_write))
    }

    #[tokio::test]
    async fn handshake_success_with_ping_and_json() -> Result<(), Box<dyn std::error::Error>> {
        let (client, server_read, mut server_write) = setup_pair().await?;
        let mut client_reader = BufReader::new(client);

        let mut lines = FramedRead::new(server_read, LinesCodec::new_with_max_length(MAX_LINE));

        let server_task = tokio::spawn(async move {
            read_handshake_request(&mut lines, &mut server_write)
                .await
                .map_err(|e| e.to_string())
        });

        // Send an empty line, a PING (expect PONG), then a valid JSON subscription
        let client_stream = client_reader.get_mut();
        client_stream.write_all(b"\n").await?;
        client_stream.write_all(b"PING\n").await?;
        client_stream
            .write_all(b"{\"subscribe\":[\"RuneEtched\",\"NewBlock\"]}\n")
            .await?;
        client_stream.flush().await?;

        // Read PONG from server
        let mut pong_line = String::new();
        tokio::time::timeout(
            Duration::from_secs(1),
            client_reader.read_line(&mut pong_line),
        )
        .await??;
        assert_eq!(pong_line, "PONG\n");

        // Wait for server to parse the JSON
        let req = tokio::time::timeout(Duration::from_secs(1), server_task).await??;
        let req = req?;
        assert_eq!(req.subscribe.len(), 2);
        assert_eq!(req.subscribe[0], EventType::RuneEtched);
        assert_eq!(req.subscribe[1], EventType::NewBlock);
        Ok(())
    }

    #[tokio::test]
    async fn handshake_errors_on_too_many_empty_lines() -> Result<(), Box<dyn std::error::Error>> {
        let (mut client, server_read, mut server_write) = setup_pair().await?;
        let mut lines = FramedRead::new(server_read, LinesCodec::new_with_max_length(MAX_LINE));

        let server_task = tokio::spawn(async move {
            read_handshake_request(&mut lines, &mut server_write)
                .await
                .map_err(|e| e.to_string())
        });

        // Send many empty lines to exceed preamble limit inside the function
        for _ in 0..20 {
            client.write_all(b"\n").await?;
        }
        client.flush().await?;

        let res = tokio::time::timeout(Duration::from_secs(1), server_task).await??;
        assert!(res.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn handshake_errors_on_invalid_json() -> Result<(), Box<dyn std::error::Error>> {
        let (mut client, server_read, mut server_write) = setup_pair().await?;
        let mut lines = FramedRead::new(server_read, LinesCodec::new_with_max_length(MAX_LINE));

        let server_task = tokio::spawn(async move {
            read_handshake_request(&mut lines, &mut server_write)
                .await
                .map_err(|e| e.to_string())
        });

        client.write_all(b"{invalid}\n").await?;
        client.flush().await?;

        let res = tokio::time::timeout(Duration::from_secs(1), server_task).await??;
        assert!(res.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn handshake_errors_on_oversized_line() -> Result<(), Box<dyn std::error::Error>> {
        let (mut client, server_read, mut server_write) = setup_pair().await?;
        let mut lines = FramedRead::new(server_read, LinesCodec::new_with_max_length(MAX_LINE));

        let server_task = tokio::spawn(async move {
            read_handshake_request(&mut lines, &mut server_write)
                .await
                .map_err(|e| e.to_string())
        });

        // Create a line longer than MAX_LINE
        let long = "a".repeat(MAX_LINE + 1);
        client.write_all(long.as_bytes()).await?;
        client.write_all(b"\n").await?;
        client.flush().await?;

        let res = tokio::time::timeout(Duration::from_secs(1), server_task).await??;
        assert!(res.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn handshake_ignores_noise_then_accepts_json() -> Result<(), Box<dyn std::error::Error>> {
        let (mut client, server_read, mut server_write) = setup_pair().await?;
        let mut lines = FramedRead::new(server_read, LinesCodec::new_with_max_length(MAX_LINE));
        let server_task = tokio::spawn(async move {
            read_handshake_request(&mut lines, &mut server_write)
                .await
                .map_err(|e| e.to_string())
        });

        client.write_all(b"GET / HTTP/1.1\n").await?;
        client.write_all(b"ping\n").await?; // lowercase, should be ignored (no PONG)
        client.write_all(b"   \n").await?; // whitespace line
        client
            .write_all(b"{\"subscribe\":[\"NewBlock\"]}\n")
            .await?;
        client.flush().await?;

        let req = tokio::time::timeout(Duration::from_secs(1), server_task).await??;
        let req = req?;
        assert_eq!(req.subscribe, vec![EventType::NewBlock]);
        Ok(())
    }

    #[tokio::test]
    async fn handshake_many_pings_then_json_under_cap() -> Result<(), Box<dyn std::error::Error>> {
        let (client, server_read, mut server_write) = setup_pair().await?;
        let mut client_reader = BufReader::new(client);
        let mut lines = FramedRead::new(server_read, LinesCodec::new_with_max_length(MAX_LINE));
        let server_task = tokio::spawn(async move {
            read_handshake_request(&mut lines, &mut server_write)
                .await
                .map_err(|e| e.to_string())
        });

        // 9 PINGs is under the 10-line cap
        for _ in 0..9 {
            client_reader.get_mut().write_all(b"PING\n").await?;
        }
        client_reader
            .get_mut()
            .write_all(b"{\"subscribe\":[\"Reorg\"]}\n")
            .await?;
        client_reader.get_mut().flush().await?;

        for _ in 0..9 {
            let mut pong = String::new();
            tokio::time::timeout(Duration::from_secs(1), client_reader.read_line(&mut pong))
                .await??;
            assert_eq!(pong, "PONG\n");
        }

        let req = tokio::time::timeout(Duration::from_secs(1), server_task).await??;
        let req = req?;
        assert_eq!(req.subscribe, vec![EventType::Reorg]);
        Ok(())
    }

    #[tokio::test]
    async fn handshake_too_many_pings_errors_and_sends_all_pongs(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (client, server_read, mut server_write) = setup_pair().await?;
        let mut client_reader = BufReader::new(client);
        let mut lines = FramedRead::new(server_read, LinesCodec::new_with_max_length(MAX_LINE));
        let server_task = tokio::spawn(async move {
            read_handshake_request(&mut lines, &mut server_write)
                .await
                .map_err(|e| e.to_string())
        });

        // 10 PINGs hits the cap (>= 10) and should error after responding to the 10th
        for _ in 0..10 {
            client_reader.get_mut().write_all(b"PING\n").await?;
        }
        client_reader.get_mut().flush().await?;

        for _ in 0..10 {
            let mut pong = String::new();
            tokio::time::timeout(Duration::from_secs(1), client_reader.read_line(&mut pong))
                .await??;
            assert_eq!(pong, "PONG\n");
        }

        let res = tokio::time::timeout(Duration::from_secs(1), server_task).await??;
        assert!(res.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn handshake_trims_whitespace_and_crlf() -> Result<(), Box<dyn std::error::Error>> {
        let (client, server_read, mut server_write) = setup_pair().await?;
        let mut client_reader = BufReader::new(client);
        let mut lines = FramedRead::new(server_read, LinesCodec::new_with_max_length(MAX_LINE));
        let server_task = tokio::spawn(async move {
            read_handshake_request(&mut lines, &mut server_write)
                .await
                .map_err(|e| e.to_string())
        });

        client_reader.get_mut().write_all(b"  PING  \r\n").await?;
        client_reader
            .get_mut()
            .write_all(b"  {\"subscribe\":[\"RuneMinted\"]}  \r\n")
            .await?;
        client_reader.get_mut().flush().await?;

        let mut pong = String::new();
        tokio::time::timeout(Duration::from_secs(1), client_reader.read_line(&mut pong)).await??;
        assert_eq!(pong, "PONG\n");

        let req = tokio::time::timeout(Duration::from_secs(1), server_task).await??;
        let req = req?;
        assert_eq!(req.subscribe, vec![EventType::RuneMinted]);
        Ok(())
    }

    #[tokio::test]
    async fn handshake_invalid_event_type_errors() -> Result<(), Box<dyn std::error::Error>> {
        let (mut client, server_read, mut server_write) = setup_pair().await?;
        let mut lines = FramedRead::new(server_read, LinesCodec::new_with_max_length(MAX_LINE));
        let server_task = tokio::spawn(async move {
            read_handshake_request(&mut lines, &mut server_write)
                .await
                .map_err(|e| e.to_string())
        });

        client
            .write_all(b"{\"subscribe\":[\"NotAnEvent\"]}\n")
            .await?;
        client.flush().await?;

        let res = tokio::time::timeout(Duration::from_secs(1), server_task).await??;
        assert!(res.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn handshake_connection_closed_before_request() -> Result<(), Box<dyn std::error::Error>>
    {
        let (client, server_read, mut server_write) = setup_pair().await?;
        let mut lines = FramedRead::new(server_read, LinesCodec::new_with_max_length(MAX_LINE));
        let server_task = tokio::spawn(async move {
            read_handshake_request(&mut lines, &mut server_write)
                .await
                .map_err(|e| e.to_string())
        });

        drop(client); // close without sending anything

        let res = tokio::time::timeout(Duration::from_secs(1), server_task).await??;
        assert!(res.is_err());
        Ok(())
    }
}
