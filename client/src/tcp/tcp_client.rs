use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json;
use thiserror::Error;
use titan_types_api::TcpSubscriptionRequest;
use titan_types_core::Event;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tracing::{debug, error, info, warn};

use crate::tcp::{
    connection_status::ConnectionStatus,
    reconnection::{self, ReconnectionManager},
};

use super::connection_status::ConnectionStatusTracker;

#[derive(Debug, Error)]
pub enum TcpClientError {
    #[error("io error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("serde error: {0}")]
    SerdeError(#[from] serde_json::Error),
    #[error("join error: task panicked")]
    JoinError,
    #[error("lock error: failed to acquire lock")]
    LockError,
}

/// Settings for reconnecting.
#[derive(Debug, Clone)]
pub struct Config {
    /// Maximum number of reconnect attempts. Use `None` for unlimited retries.
    pub max_retries: Option<u32>,
    /// Delay between reconnect attempts.
    pub retry_delay: Duration,
    /// Initial capacity of the read buffer (in bytes)
    pub read_buffer_capacity: usize,
    /// Maximum allowed size for the read buffer (in bytes)
    pub max_buffer_size: usize,
    /// Interval between ping messages
    pub ping_interval: Duration,
    /// Timeout for waiting for pong responses
    pub pong_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_retries: None,
            retry_delay: Duration::from_secs(1), // Match the default used by ReconnectionConfig
            read_buffer_capacity: 4096,          // 4KB initial capacity
            max_buffer_size: 10 * 1024 * 1024,   // 10MB max buffer size (same as sync client)
            ping_interval: Duration::from_secs(30), // Send ping every 30 seconds
            pong_timeout: Duration::from_secs(10), // Wait 10 seconds for pong response
        }
    }
}

/// Shared shutdown channel that can be safely modified
struct ShutdownChannel {
    sender: watch::Sender<()>,
    receiver: watch::Receiver<()>,
}

impl ShutdownChannel {
    fn new() -> Self {
        let (sender, receiver) = watch::channel(());
        Self { sender, receiver }
    }

    fn get_receiver(&self) -> watch::Receiver<()> {
        self.receiver.clone()
    }

    fn send(&self) -> Result<(), watch::error::SendError<()>> {
        self.sender.send(())
    }
}

/// Asynchronous TCP client that encapsulates the shutdown signal and reconnect settings.
pub struct AsyncTcpClient {
    shutdown_channel: Arc<Mutex<ShutdownChannel>>,
    config: Config,
    status_tracker: ConnectionStatusTracker,
    worker_task: Mutex<Option<JoinHandle<()>>>,
}

impl AsyncTcpClient {
    /// Creates a new instance with custom reconnect settings.
    pub fn new_with_config(config: Config) -> Self {
        Self {
            shutdown_channel: Arc::new(Mutex::new(ShutdownChannel::new())),
            config,
            status_tracker: ConnectionStatusTracker::new(),
            worker_task: Mutex::new(None),
        }
    }

    /// Creates a new instance with default reconnect settings:
    /// unlimited retries with a 1-second base delay that increases with exponential backoff.
    pub fn new() -> Self {
        Self::new_with_config(Config::default())
    }

    /// Get the current connection status
    pub fn get_status(&self) -> ConnectionStatus {
        self.status_tracker.get_status()
    }

    /// Get whether the client was disconnected at any point in time
    pub fn create_status_subscriber(&self) -> mpsc::Receiver<ConnectionStatus> {
        let (tx, rx) = mpsc::channel(100);
        self.status_tracker.register_listener(tx);
        rx
    }

    /// Checks if there is an active worker task.
    ///
    /// Returns true if a worker task is currently running.
    pub fn has_active_task(&self) -> Result<bool, TcpClientError> {
        match self.worker_task.lock() {
            Ok(lock) => Ok(lock.is_some()),
            Err(_) => {
                error!("Failed to acquire worker task lock");
                Err(TcpClientError::LockError)
            }
        }
    }

    /// Subscribes to the TCP server at `addr` with the given subscription request.
    /// Returns a channel receiver for incoming events.
    ///
    /// This method includes reconnect logic with exponential backoff. If the connection is lost,
    /// the client will automatically try to reconnect using the provided settings.
    ///
    /// If there's already an active subscription task, it will be shut down and a new one will be created.
    pub async fn subscribe(
        &self,
        addr: &str,
        subscription_request: TcpSubscriptionRequest,
    ) -> Result<mpsc::Receiver<Event>, TcpClientError> {
        info!("Subscribing to {}", addr);

        // Check if we already have a worker task running
        let mut worker_lock = self
            .worker_task
            .lock()
            .map_err(|_| TcpClientError::LockError)?;

        // If there's an existing task, shut it down and join it
        if let Some(handle) = worker_lock.take() {
            info!("Shutting down existing subscription task before starting a new one");
            // Send shutdown signal
            self.send_shutdown_signal()?;

            // Create a new shutdown channel for the new task
            let mut shutdown_guard = self
                .shutdown_channel
                .lock()
                .map_err(|_| TcpClientError::LockError)?;
            *shutdown_guard = ShutdownChannel::new();
            drop(shutdown_guard); // Release the lock

            // Attempt to join the existing task with a timeout
            match tokio::time::timeout(Duration::from_secs(5), handle).await {
                Ok(join_result) => match join_result {
                    Ok(_) => info!("Successfully joined existing task"),
                    Err(_) => error!("Error joining existing task: it panicked"),
                },
                Err(_) => {
                    // Task didn't complete within timeout
                    error!("Timed out waiting for existing task to complete, proceeding with new task anyway");
                }
            }
        }

        // Create a channel to forward events.
        let (tx, rx) = mpsc::channel::<Event>(100);

        // Clone the settings and shutdown receiver for the spawned task.
        let reconnect_settings = self.config.clone();
        let shutdown_receiver = {
            let guard = self
                .shutdown_channel
                .lock()
                .map_err(|_| TcpClientError::LockError)?;
            guard.get_receiver()
        };
        let addr = addr.to_owned();
        let subscription_request = subscription_request;
        let status_tracker = self.status_tracker.clone();

        // Set initial status to Connecting
        status_tracker.update_status(ConnectionStatus::Connecting);

        // Create the reconnection config from settings
        let reconnection_config = reconnection::from_async_reconnect_settings(&reconnect_settings);

        info!("Creating reconnection manager");

        // Spawn a task to manage connection, reading, and reconnection.
        let handle = tokio::spawn(async move {
            // Create a status updater for use in the async task
            let update_status = |new_status| status_tracker.update_status(new_status);

            // Create the reconnection manager
            let mut reconnection_manager = ReconnectionManager::new(reconnection_config);

            // Use the shutdown receiver
            let mut shutdown_rx = shutdown_receiver;

            // Ping-pong monitoring
            let ping_interval = reconnect_settings.ping_interval;
            let pong_timeout = reconnect_settings.pong_timeout;
            let mut last_pong_time = std::time::Instant::now();
            let mut ping_timer = tokio::time::interval(ping_interval);
            ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut awaiting_pong = false;

            loop {
                // Before each connection attempt, check for a shutdown signal.
                if shutdown_rx.has_changed().unwrap_or(false) {
                    info!("Shutdown signal received. Exiting subscription task.");
                    update_status(ConnectionStatus::Disconnected);
                    break;
                }
                info!("Attempting to connect to {}", addr);
                update_status(ConnectionStatus::Connecting);

                match TcpStream::connect(&addr).await {
                    Ok(stream) => {
                        info!("Connected to {}.", addr);
                        update_status(ConnectionStatus::Connected);

                        // Reset reconnection attempts after successful connection
                        reconnection_manager.reset();

                        let (reader, mut writer) = stream.into_split();
                        let mut reader = BufReader::new(reader);

                        // Ping-pong monitoring
                        let ping_interval = reconnect_settings.ping_interval;
                        let pong_timeout = reconnect_settings.pong_timeout;
                        let mut last_pong_time = std::time::Instant::now();
                        let mut ping_timer = tokio::time::interval(ping_interval);
                        ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                        let mut awaiting_pong = false;

                        // Serialize and send the subscription request.
                        match serde_json::to_string(&subscription_request) {
                            Ok(req_json) => {
                                if let Err(e) = writer.write_all(req_json.as_bytes()).await {
                                    error!("Error sending subscription request: {}", e);
                                    // Let the reconnect loop try again.
                                    continue;
                                }
                                if let Err(e) = writer.write_all(b"\n").await {
                                    error!("Error writing newline: {}", e);
                                    continue;
                                }
                                if let Err(e) = writer.flush().await {
                                    error!("Error flushing writer: {}", e);
                                    continue;
                                }
                            }
                            Err(e) => {
                                error!("Error serializing subscription request: {}", e);
                                update_status(ConnectionStatus::Disconnected);
                                break;
                            }
                        }

                        // Initialize the byte buffer with the configured capacity
                        let mut byte_buf =
                            Vec::with_capacity(reconnect_settings.read_buffer_capacity);
                        // Read loop: continuously receive events.
                        loop {
                            // Check if the buffer has grown too large
                            if byte_buf.capacity() > reconnect_settings.max_buffer_size {
                                error!(
                                    "Buffer capacity exceeded maximum allowed size ({}), resetting connection.",
                                    reconnect_settings.max_buffer_size
                                );
                                break; // Break inner loop to trigger reconnect
                            }

                            tokio::select! {
                                result = reader.read_until(b'\n', &mut byte_buf) => {
                                    match result {
                                        Ok(0) => {
                                            // Connection closed by the server.
                                            warn!("TCP connection closed by server.");
                                            update_status(ConnectionStatus::Reconnecting);
                                            break; // Break inner loop to trigger reconnect
                                        }
                                        Ok(n) if n > 0 => {
                                            // We read n bytes, including the newline.
                                            // Process the received bytes.
                                            // Note: read_until includes the delimiter in the buffer.
                                            // Trim whitespace and the trailing newline before processing.
                                            let message_bytes = byte_buf.trim_ascii_end(); // Efficient trim for ASCII/UTF8 whitespace + newline

                                            if !message_bytes.is_empty() {
                                                // Check if this is a pong response
                                                if message_bytes == b"PONG" {
                                                    if awaiting_pong {
                                                        awaiting_pong = false;
                                                        last_pong_time = std::time::Instant::now();
                                                        debug!("Received PONG");
                                                    } else {
                                                        warn!("Received unexpected PONG");
                                                    }
                                                } else {
                                                    // Check if message size exceeds limit *before* parsing JSON
                                                    if message_bytes.len() > reconnect_settings.max_buffer_size {
                                                        error!(
                                                            "Received message exceeds maximum allowed size ({}), skipping. Message starts with: {:?}",
                                                            reconnect_settings.max_buffer_size,
                                                            String::from_utf8_lossy(&message_bytes[..std::cmp::min(message_bytes.len(), 50)]) // Log first 50 bytes
                                                        );
                                                        // Note: We don't break here, just clear the buffer and continue reading the next message.
                                                    } else {
                                                        // Try to parse as an event from the byte slice
                                                        match serde_json::from_slice::<Event>(message_bytes) {
                                                            Ok(event) => {
                                                                // Every successful message resets pong timer
                                                                last_pong_time = std::time::Instant::now();
                                                                awaiting_pong = false; // Also reset awaiting_pong if we received a valid event
                                                                if let Err(e) = tx.send(event).await {
                                                                    error!("Failed to send event to channel: {}", e);
                                                                    update_status(ConnectionStatus::Disconnected); // Channel broken, can't continue
                                                                    return; // Exit the task
                                                                }
                                                            }
                                                            Err(e) => {
                                                                error!(
                                                                    "Failed to parse event: {}. Raw data (first 100 bytes): {:?}",
                                                                    e,
                                                                    String::from_utf8_lossy(&message_bytes[..std::cmp::min(message_bytes.len(), 100)])
                                                                );
                                                                update_status(ConnectionStatus::Reconnecting);
                                                                break;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            // Clear the buffer for the next message AFTER processing the current one
                                            byte_buf.clear();
                                        }
                                        Ok(_) => {
                                            // n == 0, should be handled by Ok(0) case, but safety belt
                                            byte_buf.clear();
                                        }
                                        Err(e) => {
                                            error!("Error reading from TCP socket using read_until: {}", e);
                                            update_status(ConnectionStatus::Reconnecting);
                                            break; // Break inner loop to trigger reconnect
                                        }
                                    }
                                }
                                _ = ping_timer.tick() => {
                                    // Time to send a ping
                                    if awaiting_pong {
                                        // We're still waiting for a pong from the previous ping
                                        let elapsed = last_pong_time.elapsed();
                                        if elapsed > pong_timeout {
                                            warn!("Pong response timed out after {:?}, considering connection dead", elapsed);
                                            update_status(ConnectionStatus::Reconnecting);
                                            break;
                                        }
                                    } else {
                                        // Send a ping
                                        match writer.write_all(b"PING\n").await {
                                            Ok(_) => {
                                                if let Err(e) = writer.flush().await {
                                                    error!("Failed to flush ping: {}", e);
                                                    update_status(ConnectionStatus::Reconnecting);
                                                    break;
                                                }
                                                awaiting_pong = true;
                                            }
                                            Err(e) => {
                                                error!("Failed to send ping: {}", e);
                                                update_status(ConnectionStatus::Reconnecting);
                                                break;
                                            }
                                        }
                                    }
                                }
                                _ = shutdown_rx.changed() => {
                                    info!("Shutdown signal received. Exiting TCP subscription task.");
                                    update_status(ConnectionStatus::Disconnected);
                                    return;
                                }
                            }
                        }
                        // When the inner loop ends (e.g. connection lost), try to reconnect.
                        info!("Lost connection. Preparing to reconnect...");
                        update_status(ConnectionStatus::Reconnecting);
                    }
                    Err(e) => {
                        error!("Failed to connect to {}: {}. Will retry...", addr, e);
                        update_status(ConnectionStatus::Reconnecting);
                    }
                }

                // Get the next delay from the reconnection manager
                match reconnection_manager.next_delay() {
                    Some(wait_time) => {
                        info!(
                            "Reconnecting in {:?}... (attempt {}/{:?})",
                            wait_time,
                            reconnection_manager.current_attempt(),
                            reconnection_manager.config().max_attempts
                        );
                        tokio::time::sleep(wait_time).await;
                    }
                    None => {
                        error!(
                            "Reached maximum reconnection attempts ({}). Exiting subscription task.",
                            reconnection_manager.config().max_attempts.unwrap_or(0)
                        );
                        update_status(ConnectionStatus::Disconnected);
                        break;
                    }
                }
            }

            info!("Exiting TCP subscription task.");
            update_status(ConnectionStatus::Disconnected);
        });

        // Store the task handle
        *worker_lock = Some(handle);

        Ok(rx)
    }

    /// Helper method to send a shutdown signal
    fn send_shutdown_signal(&self) -> Result<(), TcpClientError> {
        let channel = match self.shutdown_channel.lock() {
            Ok(channel) => channel,
            Err(_) => {
                error!("Failed to acquire shutdown channel lock");
                return Err(TcpClientError::LockError);
            }
        };

        if let Err(e) = channel.send() {
            error!("Failed to send shutdown signal: {:?}", e);
            return Err(TcpClientError::IOError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to send shutdown signal",
            )));
        }

        Ok(())
    }

    /// Signals the client to shut down by sending a signal through the watch channel.
    /// Does not wait for the worker task to complete.
    pub fn shutdown(&self) {
        // First directly update the status locally to ensure it's set immediately
        self.status_tracker
            .update_status(ConnectionStatus::Disconnected);

        // Then send the shutdown signal to the worker task
        if let Err(e) = self.send_shutdown_signal() {
            error!("Error in shutdown: {:?}", e);
        }
    }

    /// Signals the client to shut down and waits for the worker task to complete.
    /// Returns true if the task was successfully joined, false otherwise.
    pub async fn shutdown_and_join(&self) -> Result<(), TcpClientError> {
        // Signal shutdown
        self.shutdown();

        // Try to join the task
        self.join().await
    }

    /// Waits for the worker task to complete.
    /// Returns Ok(()) if the task was successfully joined, or an error if joining failed.
    pub async fn join(&self) -> Result<(), TcpClientError> {
        // Acquire the lock on the worker task
        let mut worker_lock = self
            .worker_task
            .lock()
            .map_err(|_| TcpClientError::LockError)?;

        // Take the task handle out (replacing it with None)
        if let Some(handle) = worker_lock.take() {
            match handle.await {
                Ok(_) => {
                    info!("Successfully joined worker task");
                    Ok(())
                }
                Err(_) => {
                    error!("Failed to join worker task due to panic");
                    Err(TcpClientError::JoinError)
                }
            }
        } else {
            // No task to join
            info!("No worker task to join");
            Ok(())
        }
    }
}

impl Drop for AsyncTcpClient {
    fn drop(&mut self) {
        // Signal task to terminate
        self.shutdown();

        // We can't await in drop, so just log that the task will continue to run until it checks the shutdown signal
        info!("AsyncTcpClient dropped, task will continue running until shutdown completes");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, TcpListener};
    use std::sync::Arc;
    use std::sync::Once;
    use titan_types_core::EventType;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener as TokioTcpListener;
    use tokio::select;
    use tokio::signal::unix::{signal, SignalKind};
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;
    use tokio::time::sleep;
    use tracing_subscriber::{self, EnvFilter};

    // Initialize the logger once for all tests
    static INIT: Once = Once::new();

    fn init_test_logger() {
        INIT.call_once(|| {
            // Initialize a subscriber that prints all logs to stderr
            let filter =
                EnvFilter::from_default_env().add_directive("titan_client=trace".parse().unwrap());

            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_test_writer() // This ensures logs go to the test output
                .init();

            println!("Test logger initialized at TRACE level");
        });
    }

    // Start an async test server
    async fn start_async_test_server() -> (JoinHandle<()>, SocketAddr, oneshot::Sender<()>) {
        // Initialize the logger for testing
        init_test_logger();

        // Create a shutdown channel
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

        // Create the server
        let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Log that the server is starting
        info!("Test server starting on {}", addr);

        // Spawn the server task
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok((mut stream, client_addr)) => {
                                info!("Test server accepted connection from {}", client_addr);

                                // Handle the connection in a new task
                                tokio::spawn(async move {
                                    let (reader, mut writer) = stream.split();
                                    let mut reader = BufReader::new(reader);
                                    let mut request_buf = Vec::new();

                                    // Read the request line
                                    match reader.read_until(b'\n', &mut request_buf).await {
                                        Ok(n) if n > 0 => {
                                            let request_bytes = request_buf.trim_ascii_end();
                                            info!("Test server received request: {}", String::from_utf8_lossy(request_bytes));

                                            // Send back a test event with a small delay
                                            sleep(Duration::from_millis(10)).await;

                                            let event_json = r#"{"type":"TransactionsAdded","data": { "txids":["2222222222222222222222222222222222222222222222222222222222222222"]}}"#;
                                            if let Err(e) = writer.write_all(event_json.as_bytes()).await {
                                                 error!("Test server failed to write event: {}", e); return;
                                            }
                                            if let Err(e) = writer.write_all(b"\n").await {
                                                error!("Test server failed to write newline: {}", e); return;
                                            }
                                            if let Err(e) = writer.flush().await {
                                                 error!("Test server failed to flush: {}", e); return;
                                            }
                                            info!("Test server sent event and newline");

                                            // Handle PING/PONG
                                            let mut line_buf = Vec::new();
                                            loop {
                                                 // Clear buffer for next read
                                                line_buf.clear();
                                                tokio::select! {
                                                    read_res = reader.read_until(b'\n', &mut line_buf) => {
                                                        match read_res {
                                                            Ok(0) => {
                                                                info!("Test server: Client disconnected");
                                                                break; // Exit loop on disconnect
                                                            }
                                                            Ok(m) if m > 0 => {
                                                                let trimmed_line = line_buf.trim_ascii_end();
                                                                if trimmed_line == b"PING" {
                                                                    info!("Test server received PING");
                                                                    if let Err(e) = writer.write_all(b"PONG\n").await {
                                                                        error!("Test server failed to send PONG: {}", e);
                                                                        break; // Exit loop on write error
                                                                    }
                                                                    if let Err(e) = writer.flush().await {
                                                                         error!("Test server failed to flush PONG: {}", e);
                                                                         break; // Exit loop on flush error
                                                                    }
                                                                    info!("Test server sent PONG");
                                                                } else if !trimmed_line.is_empty() {
                                                                    // Log unexpected input
                                                                    info!("Test server received unexpected data: {}", String::from_utf8_lossy(trimmed_line));
                                                                }
                                                            }
                                                            Ok(_) => {
                                                                // Should not happen with Ok(0) case above
                                                            }
                                                            Err(e) => {
                                                                error!("Test server read error in PING loop: {}", e);
                                                                break; // Exit loop on read error
                                                            }
                                                        }
                                                    }
                                                    // Add a timeout or shutdown mechanism here if needed for the test server's loop
                                                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                                                        // Periodically check or just idle
                                                    }
                                                }
                                            }
                                        },
                                        Ok(0) => {
                                            info!("Test server received empty read, client closed connection early");
                                        },
                                        Ok(n) => {
                                             info!("Test server received {} bytes, but not processing as initial request", n);
                                        },
                                        Err(e) => error!("Test server initial read error: {}", e),
                                    }
                                });
                            },
                            Err(e) => {
                                error!("Test server accept error: {}", e);
                                // Add a short delay to prevent tight loop on accept errors
                                sleep(Duration::from_millis(10)).await;
                            }
                        }
                    },
                    _ = &mut shutdown_rx => {
                        info!("Test server received shutdown signal");
                        break;
                    }
                }
            }
            info!("Test server shutting down");
        });

        // Small delay to ensure server is ready
        sleep(Duration::from_millis(10)).await;
        info!("Test server setup complete, returning handle");

        (handle, addr, shutdown_tx)
    }

    #[tokio::test]
    async fn test_shutdown_and_join() {
        // Initialize logging for tests
        init_test_logger();
        info!("Starting test_shutdown_and_join");

        // Create a TCP client with a mock server (that doesn't exist)
        let client = AsyncTcpClient::new_with_config(Config {
            max_retries: Some(2),                    // Limit retries for quicker test
            retry_delay: Duration::from_millis(100), // Short delay for quicker test
            read_buffer_capacity: 4096,
            max_buffer_size: 10 * 1024 * 1024,
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
        });

        // Subscribe to a non-existent server - this will keep retrying
        let subscription_request = TcpSubscriptionRequest { subscribe: vec![] };
        info!("Subscribing to non-existent server to test shutdown");

        // We know this will fail to connect, but it starts the background task
        let result = client.subscribe("127.0.0.1:1", subscription_request).await;
        assert!(result.is_ok());
        info!("Subscription started, checking active task");

        // Check that we have an active task
        assert!(client.has_active_task().unwrap());

        // Wait a bit to let the reconnect logic run
        info!("Waiting for reconnection attempts to begin");
        sleep(Duration::from_millis(300)).await;

        // Shutdown and join
        info!("Shutting down client");
        let result = client.shutdown_and_join().await;

        // Should succeed
        assert!(result.is_ok());
        info!("Client shutdown successfully");

        // Check we no longer have an active task
        assert!(!client.has_active_task().unwrap());
        info!("Test completed successfully");
    }

    #[tokio::test]
    async fn test_multiple_subscribes() {
        // Initialize logging for tests
        init_test_logger();
        info!("Starting test_multiple_subscribes");

        // Create a TCP client
        let client = AsyncTcpClient::new_with_config(Config {
            max_retries: Some(1),                    // Limit retries for quicker test
            retry_delay: Duration::from_millis(100), // Short delay for quicker test
            read_buffer_capacity: 4096,
            max_buffer_size: 10 * 1024 * 1024,
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
        });

        // First subscription
        let subscription_request1 = TcpSubscriptionRequest { subscribe: vec![] };
        info!("Creating first subscription");
        let result1 = client.subscribe("127.0.0.1:1", subscription_request1).await;
        assert!(result1.is_ok());

        // We should have an active task
        assert!(client.has_active_task().unwrap());
        info!("First subscription active");

        // Wait a bit
        sleep(Duration::from_millis(200)).await;

        // Second subscription - should replace the first one
        let subscription_request2 = TcpSubscriptionRequest { subscribe: vec![] };
        info!("Creating second subscription (should replace the first)");
        let result2 = client.subscribe("127.0.0.1:2", subscription_request2).await;
        assert!(result2.is_ok());

        // We should still have an active task
        assert!(client.has_active_task().unwrap());
        info!("Second subscription active");

        // Shutdown and join
        info!("Shutting down client");
        let join_result = client.shutdown_and_join().await;
        assert!(join_result.is_ok());

        // We should no longer have an active task
        assert!(!client.has_active_task().unwrap());
        info!("Test completed successfully");
    }

    #[tokio::test]
    async fn test_async_connection_status_transitions() {
        // Initialize logging for tests
        init_test_logger();
        info!("Starting test_async_connection_status_transitions");

        // Start a test server
        info!("Starting test server");
        let (server_handle, server_addr, shutdown_tx) = start_async_test_server().await;

        // Create a client
        let client = AsyncTcpClient::new_with_config(Config {
            max_retries: Some(2),
            retry_delay: Duration::from_millis(100),
            read_buffer_capacity: 4096,
            max_buffer_size: 10 * 1024 * 1024,
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
        });

        // Initially disconnected
        assert_eq!(client.get_status(), ConnectionStatus::Disconnected);
        info!("Initial status: {:?}", client.get_status());

        // Subscribe to the server
        let subscription_request = TcpSubscriptionRequest {
            subscribe: vec![EventType::TransactionsAdded],
        };

        info!("Subscribing to test server at {}", server_addr);
        let rx = client
            .subscribe(
                &format!("127.0.0.1:{}", server_addr.port()),
                subscription_request,
            )
            .await
            .unwrap();

        // Give it time to connect
        info!("Waiting for connection to establish");
        sleep(Duration::from_millis(100)).await;

        // Should be connected now
        let status = client.get_status();
        info!("Status after connection attempt: {:?}", status);
        assert_eq!(status, ConnectionStatus::Connected);

        // Shutdown the client
        info!("Shutting down client");
        client.shutdown_and_join().await.unwrap();

        // Check the client is disconnected
        let final_status = client.get_status();
        info!("Final status: {:?}", final_status);
        assert_eq!(final_status, ConnectionStatus::Disconnected);

        // Shutdown the server
        info!("Shutting down test server");
        let _ = shutdown_tx.send(());
        let _ = server_handle.await;
        info!("Test completed successfully");
    }

    #[tokio::test]
    async fn test_async_receive_events() {
        // Initialize logging for tests
        init_test_logger();
        info!("Starting test_async_receive_events");

        // Start a test server
        info!("Starting test server");
        let (server_handle, server_addr, shutdown_tx) = start_async_test_server().await;
        info!("Test server started at {}", server_addr);

        // Create a client
        let client = AsyncTcpClient::new_with_config(Config {
            max_retries: Some(2),
            retry_delay: Duration::from_millis(100),
            read_buffer_capacity: 4096,
            max_buffer_size: 10 * 1024 * 1024,
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
        });

        // Subscribe to the server
        let subscription_request = TcpSubscriptionRequest {
            subscribe: vec![EventType::TransactionsAdded],
        };

        info!("Subscribing to test server at {}", server_addr);
        let mut rx = client
            .subscribe(
                &format!("127.0.0.1:{}", server_addr.port()),
                subscription_request,
            )
            .await
            .unwrap();

        // Give it enough time to establish the connection
        info!("Waiting for connection to establish");
        sleep(Duration::from_millis(100)).await;

        info!("Checking connection status: {:?}", client.get_status());
        assert_eq!(
            client.get_status(),
            ConnectionStatus::Connected,
            "Expected client to be connected, but status is {:?}",
            client.get_status()
        );

        // Try to receive an event with timeout
        info!("Waiting to receive an event");
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;

        if let Err(e) = &event {
            error!("Timeout waiting for event: {}", e);
            // Try to diagnose what happened
            info!("Current client status: {:?}", client.get_status());
            assert!(false, "Failed to receive event within timeout");
        }

        let event = event.unwrap();

        if let None = &event {
            error!("Received None from channel - sender was likely dropped");
            info!("Current client status: {:?}", client.get_status());
            assert!(false, "Expected Some(event) but got None");
        }

        match event.unwrap() {
            Event::TransactionsAdded { txids } => {
                info!("Received transaction event with {} txids", txids.len());
                assert_eq!(txids.len(), 1, "Expected 1 txid in event");
                assert_eq!(
                    txids[0].to_string(),
                    "2222222222222222222222222222222222222222222222222222222222222222"
                );
            }
            other => {
                error!("Received unexpected event type: {:?}", other);
                panic!("Received unexpected event type: {:?}", other);
            }
        }

        // Shutdown the client
        info!("Shutting down client");
        client.shutdown_and_join().await.unwrap();

        // Shutdown the server
        info!("Shutting down test server");
        let _ = shutdown_tx.send(());
        let _ = server_handle.await;
        info!("Test completed successfully");
    }

    #[tokio::test]
    async fn test_async_connection_error_handling() {
        // Initialize logging for tests
        init_test_logger();
        info!("Starting test_async_connection_error_handling");

        // Create a client with short timeout
        let client = AsyncTcpClient::new_with_config(Config {
            max_retries: Some(2),
            retry_delay: Duration::from_millis(100),
            read_buffer_capacity: 4096,
            max_buffer_size: 10 * 1024 * 1024,
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
        });

        // Initially disconnected
        assert_eq!(client.get_status(), ConnectionStatus::Disconnected);
        info!("Initial status: {:?}", client.get_status());

        // Try to connect to a non-existent server
        let subscription_request = TcpSubscriptionRequest {
            subscribe: vec![EventType::TransactionsAdded],
        };

        info!("Subscribing to non-existent server to test error handling");
        let rx = client
            .subscribe("127.0.0.1:1", subscription_request)
            .await
            .unwrap();

        // Give it time to attempt connection and reconnection
        info!("Waiting for connection attempts");
        sleep(Duration::from_millis(500)).await;

        // Should be in reconnecting state or disconnected if it already gave up
        let status = client.get_status();
        info!("Status after connection attempts: {:?}", status);
        assert!(
            status == ConnectionStatus::Reconnecting || status == ConnectionStatus::Disconnected,
            "Expected Reconnecting or Disconnected state, got {:?}",
            status
        );

        // Shutdown the client
        info!("Shutting down client");
        client.shutdown_and_join().await.unwrap();

        // Check the client is disconnected
        let final_status = client.get_status();
        info!("Final status: {:?}", final_status);
        assert_eq!(final_status, ConnectionStatus::Disconnected);
        info!("Test completed successfully");
    }

    #[tokio::test]
    async fn test_async_shutdown_during_reconnect() {
        // Initialize logging for tests
        init_test_logger();
        info!("Starting test_async_shutdown_during_reconnect");

        // Create a client with many more retries and a longer delay to ensure we
        // have time to catch it in the reconnecting state
        let client = AsyncTcpClient::new_with_config(Config {
            max_retries: Some(100), // Many more retries so it won't finish quickly
            retry_delay: Duration::from_millis(500), // Longer delay between attempts
            read_buffer_capacity: 4096,
            max_buffer_size: 10 * 1024 * 1024,
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
        });

        // Subscribe to a non-existent server to trigger reconnection attempts
        let subscription_request = TcpSubscriptionRequest {
            subscribe: vec![EventType::TransactionsAdded],
        };

        info!("Subscribing to non-existent server to trigger reconnection");
        let rx = client
            .subscribe("127.0.0.1:1", subscription_request)
            .await
            .unwrap();

        // Give it just enough time for the first connection attempt to fail and enter reconnecting
        // This is much shorter than before to ensure we don't go through all retries
        info!("Waiting for client to enter reconnection state");
        sleep(Duration::from_millis(50)).await;

        let status_before_shutdown = client.get_status();
        info!("Status before shutdown: {:?}", status_before_shutdown);

        // If we're not in the reconnecting state yet, wait a little longer
        if status_before_shutdown != ConnectionStatus::Reconnecting {
            info!("Not in reconnecting state yet, waiting longer");
            sleep(Duration::from_millis(50)).await;
            let status_before_shutdown = client.get_status();
            info!("Status after additional wait: {:?}", status_before_shutdown);
        }

        // Now assert - this should work because we're either catching it during the first
        // reconnection attempt or we've waited a bit longer
        assert_eq!(client.get_status(), ConnectionStatus::Reconnecting);

        // Shutdown the client while it's reconnecting
        info!("Shutting down client during reconnection");
        client.shutdown();

        // Status should be immediately set to Disconnected by the shutdown method
        let status_after_shutdown = client.get_status();
        info!(
            "Status immediately after shutdown: {:?}",
            status_after_shutdown
        );
        assert_eq!(status_after_shutdown, ConnectionStatus::Disconnected);

        // Now try joining the task
        info!("Joining worker task");
        sleep(Duration::from_millis(50)).await;
        let result = client.join().await;
        info!("Join result: {:?}", result);
        assert!(
            result.is_ok(),
            "Failed to join task during reconnection: {:?}",
            result
        );

        // Status should definitely be disconnected now
        let final_status = client.get_status();
        info!("Final status: {:?}", final_status);
        assert_eq!(final_status, ConnectionStatus::Disconnected);
        info!("Test completed successfully");
    }

    #[tokio::test]
    async fn test_async_buffer_size_limit() {
        // Initialize logging for tests
        init_test_logger();
        info!("Starting test_async_buffer_size_limit");

        // Create a special test server that sends oversized data
        info!("Starting oversized data test server");
        let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        info!("Test server bound to {}", server_addr);

        // Spawn server
        let server_handle = tokio::spawn(async move {
            info!("Test server waiting for connection");
            if let Ok((mut stream, client_addr)) = listener.accept().await {
                info!("Test server accepted connection from {}", client_addr);
                // Read the request
                let mut buf = [0; 1024];
                match stream.read(&mut buf).await {
                    Ok(n) => {
                        let request = String::from_utf8_lossy(&buf[..n]);
                        info!("Test server received request: {}", request);

                        // Send a large response that exceeds the small buffer size
                        info!("Sending 100KB payload to test buffer limits");

                        // Start with a valid JSON prefix - make sure it's valid Event format
                        let prefix = r#"{"type":"TransactionsAdded", "data": {"txids":["#;
                        stream.write_all(prefix.as_bytes()).await.unwrap();

                        // Add the large payload inside the JSON
                        let large_payload = "x".repeat(100_000); // 100KB of 'x' characters
                        stream.write_all(large_payload.as_bytes()).await.unwrap();

                        // Close the JSON properly
                        let suffix = r#"}]}}"#;
                        stream.write_all(suffix.as_bytes()).await.unwrap();
                        stream.write_all(b"\n").await.unwrap();
                        info!("Large payload sent");

                        // Keep connection open for a bit
                        sleep(Duration::from_millis(100)).await;
                    }
                    Err(e) => {
                        error!("Test server read error: {}", e);
                    }
                }
            }
            info!("Test server shutting down");
        });

        // Create a client with a very small buffer size
        info!("Creating client with small buffer size");
        let client = AsyncTcpClient::new_with_config(Config {
            max_retries: Some(1),
            retry_delay: Duration::from_millis(100),
            read_buffer_capacity: 1024, // 1KB initial capacity
            max_buffer_size: 10 * 1024, // Only 10KB max buffer size
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
        });

        // Subscribe to the server
        let subscription_request = TcpSubscriptionRequest {
            subscribe: vec![EventType::TransactionsAdded],
        };

        info!("Subscribing to server with buffer size limit test");
        let _rx = client
            .subscribe(
                &format!("127.0.0.1:{}", server_addr.port()),
                subscription_request,
            )
            .await
            .unwrap();

        // Give some time for the client to connect and process data
        info!("Waiting for buffer overflow to occur");
        sleep(Duration::from_millis(300)).await;

        // The client should have disconnected due to buffer overflow
        let status = client.get_status();
        info!("Client status after buffer overflow test: {:?}", status);
        assert!(
            status == ConnectionStatus::Reconnecting || status == ConnectionStatus::Disconnected,
            "Expected client to disconnect due to buffer overflow, but status is {:?}",
            status
        );

        // Clean up
        info!("Cleaning up");
        client.shutdown_and_join().await.unwrap();
        let _ = server_handle.await;
        info!("Test completed successfully");
    }
}
