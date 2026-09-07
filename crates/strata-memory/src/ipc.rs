use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{debug, error, info, warn};

use strata_core::a2a::{IpcEvent, IpcMessage};
use strata_core::errors::StrataError;

/// Maximum allowed payload size for a single framed IPC line (64 KB).
pub const MAX_FRAME_LENGTH: usize = 65536;

/// Default capacity for the broadcast channel ring buffer.
pub const BROADCAST_CAPACITY: usize = 256;

/// Resolves a deterministic, local-first endpoint identifier for a given workspace.
///
/// On Windows: Returns a Named Pipe path `\\.\pipe\strata-a2a-<hash16>`.
/// On Unix: Returns a Unix Domain Socket path `/tmp/strata-a2a-<hash16>.sock` (strictly bounded < 40 chars).
pub fn endpoint_for_workspace(workspace_root: &Path) -> (String, PathBuf) {
    let canonical = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let path_bytes = canonical.to_string_lossy();
    let hash = blake3::hash(path_bytes.as_bytes());
    let hash16 = &hash.to_hex()[..16];

    #[cfg(windows)]
    {
        let pipe_name = format!(r"\\.\pipe\strata-a2a-{}", hash16);
        (pipe_name.clone(), PathBuf::from(pipe_name))
    }

    #[cfg(unix)]
    {
        let sock_path = PathBuf::from(format!("/tmp/strata-a2a-{}.sock", hash16));
        (sock_path.to_string_lossy().to_string(), sock_path)
    }

    #[cfg(not(any(windows, unix)))]
    {
        let fallback = format!("strata-a2a-{}", hash16);
        (fallback.clone(), PathBuf::from(fallback))
    }
}

/// Reads a single line from an async reader with a strict maximum byte bound.
///
/// Protects against unbounded memory allocation when malformed or hostile streams are encountered.
pub async fn read_bounded_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    max_bytes: usize,
) -> Result<Option<String>, StrataError> {
    let mut line = String::new();
    let mut take = reader.take((max_bytes + 1) as u64);
    let bytes_read = take
        .read_line(&mut line)
        .await
        .map_err(|e| StrataError::Internal(format!("IPC read error: {e}")))?;

    if bytes_read == 0 {
        return Ok(None);
    }

    if line.len() > max_bytes && !line.ends_with('\n') {
        return Err(StrataError::Internal(format!(
            "IPC frame exceeded maximum size of {} bytes",
            max_bytes
        )));
    }

    Ok(Some(line))
}

/// The local IPC server hosting the stigmergic event bus.
pub struct IpcServer {
    endpoint: String,
    tx: broadcast::Sender<IpcMessage>,
    shutdown_tx: watch::Sender<bool>,
    is_running: Arc<AtomicBool>,
    _task: tokio::task::JoinHandle<()>,
}

impl IpcServer {
    /// Starts the IPC server on the specified endpoint.
    pub async fn bind(endpoint: &str) -> Result<Self, StrataError> {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let is_running = Arc::new(AtomicBool::new(true));

        let endpoint_str = endpoint.to_string();
        let server_tx = tx.clone();
        let running_flag = Arc::clone(&is_running);

        #[cfg(windows)]
        let task = {
            let pipe_name = endpoint_str.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    run_windows_listener(&pipe_name, server_tx, shutdown_rx, running_flag).await
                {
                    error!("Windows Named Pipe listener exited with error: {e}");
                }
            })
        };

        #[cfg(unix)]
        let task = {
            let sock_path = PathBuf::from(&endpoint_str);
            tokio::spawn(async move {
                if let Err(e) =
                    run_unix_listener(&sock_path, server_tx, shutdown_rx, running_flag).await
                {
                    error!("Unix Domain Socket listener exited with error: {e}");
                }
            })
        };

        #[cfg(not(any(windows, unix)))]
        let task = tokio::spawn(async move {
            warn!("A2A IPC not supported on current OS platform");
        });

        Ok(Self {
            endpoint: endpoint_str,
            tx,
            shutdown_tx,
            is_running,
            _task: task,
        })
    }

    /// Broadcasts an event to all connected agents.
    pub fn broadcast(&self, event: IpcEvent) -> Result<usize, StrataError> {
        let msg = IpcMessage::Event { event };
        self.tx
            .send(msg)
            .map_err(|_| StrataError::Internal("No active subscribers on IPC bus".to_string()))
    }

    /// Subscribes to events published through this server.
    pub fn subscribe(&self) -> broadcast::Receiver<IpcMessage> {
        self.tx.subscribe()
    }

    /// Endpoint string this server is listening on.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Checks if the server is currently running.
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Gracefully stops the IPC server.
    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
        self.is_running.store(false, Ordering::SeqCst);
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Generic bidirectional stream handler for a single connected client.
async fn handle_connection<R, W>(
    reader: R,
    mut writer: W,
    broadcast_tx: broadcast::Sender<IpcMessage>,
    mut subscriber: broadcast::Receiver<IpcMessage>,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut buf_reader = BufReader::new(reader);

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            // 1. Incoming line from this client
            read_res = read_bounded_line(&mut buf_reader, MAX_FRAME_LENGTH) => {
                match read_res {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<IpcMessage>(trimmed) {
                            Ok(IpcMessage::Event { event }) => {
                                let _ = broadcast_tx.send(IpcMessage::Event { event });
                            }
                            Ok(IpcMessage::Ping { timestamp }) => {
                                let pong = IpcMessage::Pong { timestamp };
                                if let Ok(mut json) = serde_json::to_string(&pong) {
                                    json.push('\n');
                                    let _ = tokio::time::timeout(
                                        Duration::from_millis(500),
                                        writer.write_all(json.as_bytes()),
                                    ).await;
                                }
                            }
                            Ok(IpcMessage::Pong { .. }) => {}
                            Err(e) => {
                                warn!("Failed to parse incoming IPC message: {e} | line: {trimmed}");
                            }
                        }
                    }
                    Ok(None) => {
                        // Client closed connection
                        debug!("Client disconnected from IPC bus");
                        break;
                    }
                    Err(e) => {
                        warn!("Error reading from IPC client: {e}");
                        break;
                    }
                }
            }
            // 2. Outgoing broadcast message to this client
            broadcast_msg = subscriber.recv() => {
                match broadcast_msg {
                    Ok(msg) => {
                        if let Ok(mut json) = serde_json::to_string(&msg) {
                            json.push('\n');
                            let write_res = tokio::time::timeout(
                                Duration::from_millis(500),
                                writer.write_all(json.as_bytes()),
                            ).await;
                            if write_res.is_err() || write_res.unwrap().is_err() {
                                debug!("Client write timed out or disconnected, closing client task");
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        warn!("Client lagged behind by {count} IPC messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
async fn run_windows_listener(
    pipe_name: &str,
    tx: broadcast::Sender<IpcMessage>,
    mut shutdown_rx: watch::Receiver<bool>,
    running_flag: Arc<AtomicBool>,
) -> Result<(), StrataError> {
    use tokio::net::windows::named_pipe::ServerOptions;

    // Allocate the very first pipe instance
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(pipe_name)
        .map_err(|e| StrataError::Internal(format!("Failed to create initial Named Pipe: {e}")))?;

    info!("Windows A2A Named Pipe server listening on {pipe_name}");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            connect_res = server.connect() => {
                if let Err(e) = connect_res {
                    warn!("Named pipe connect error: {e}");
                    continue;
                }

                let connected_pipe = server;

                // CRITICAL (Red Team Mitigation): Allocate next pipe instance immediately
                // before spawning worker to eliminate the race window for subsequent clients.
                match ServerOptions::new().first_pipe_instance(false).create(pipe_name) {
                    Ok(next_instance) => {
                        server = next_instance;
                    }
                    Err(e) => {
                        error!("Failed to allocate next Named Pipe instance: {e}");
                        break;
                    }
                }

                let subscriber = tx.subscribe();
                let client_tx = tx.clone();
                let client_shutdown = shutdown_rx.clone();
                tokio::spawn(async move {
                    let (reader, writer) = tokio::io::split(connected_pipe);
                    handle_connection(reader, writer, client_tx, subscriber, client_shutdown).await;
                });
            }
        }
    }

    running_flag.store(false, Ordering::SeqCst);
    Ok(())
}

#[cfg(unix)]
async fn run_unix_listener(
    sock_path: &Path,
    tx: broadcast::Sender<IpcMessage>,
    mut shutdown_rx: watch::Receiver<bool>,
    running_flag: Arc<AtomicBool>,
) -> Result<(), StrataError> {
    use tokio::net::{UnixListener, UnixStream};

    // Stale socket probe (Red Team Mitigation)
    if sock_path.exists() {
        match UnixStream::connect(sock_path).await {
            Ok(_) => {
                return Err(StrataError::Internal(format!(
                    "Active Strata IPC server already listening at {:?}",
                    sock_path
                )));
            }
            Err(_) => {
                // Stale socket from previous crash; remove safely
                let _ = std::fs::remove_file(sock_path);
            }
        }
    }

    if let Some(parent) = sock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let listener = UnixListener::bind(sock_path)
        .map_err(|e| StrataError::Internal(format!("Failed to bind Unix socket: {e}")))?;

    // Restrict permissions to owner only (0600)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(sock_path, std::fs::Permissions::from_mode(0o600));
    }

    info!("Unix A2A Domain Socket server listening on {:?}", sock_path);

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((stream, _)) => {
                        let subscriber = tx.subscribe();
                        let client_tx = tx.clone();
                        let client_shutdown = shutdown_rx.clone();
                        tokio::spawn(async move {
                            let (reader, writer) = stream.into_split();
                            handle_connection(reader, writer, client_tx, subscriber, client_shutdown).await;
                        });
                    }
                    Err(e) => {
                        warn!("Unix socket accept error: {e}");
                    }
                }
            }
        }
    }

    // Clean up socket file on graceful exit
    let _ = std::fs::remove_file(sock_path);
    running_flag.store(false, Ordering::SeqCst);
    Ok(())
}

/// A connected client to the local A2A IPC bus.
pub struct IpcClient {
    endpoint: String,
    outbound_tx: mpsc::Sender<IpcMessage>,
    event_tx: broadcast::Sender<IpcEvent>,
    is_connected: Arc<AtomicBool>,
}

impl IpcClient {
    /// Connects to the IPC endpoint with retry and backoff.
    pub async fn connect(endpoint: &str) -> Result<Self, StrataError> {
        let (outbound_tx, outbound_rx) = mpsc::channel(64);
        let (event_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let is_connected = Arc::new(AtomicBool::new(true));

        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ClientOptions;

            let mut connected_pipe = None;
            for attempt in 0..6 {
                match ClientOptions::new().open(endpoint) {
                    Ok(client) => {
                        connected_pipe = Some(client);
                        break;
                    }
                    Err(e) if e.raw_os_error() == Some(231) => {
                        // ERROR_PIPE_BUSY: retry with exponential backoff
                        tokio::time::sleep(Duration::from_millis(10 * (1 << attempt).min(100)))
                            .await;
                    }
                    Err(e) => {
                        if attempt == 5 {
                            return Err(StrataError::Internal(format!(
                                "Failed to connect to Named Pipe '{}': {e}",
                                endpoint
                            )));
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                }
            }

            let pipe = connected_pipe.ok_or_else(|| {
                StrataError::Internal(format!("Timed out connecting to Named Pipe '{}'", endpoint))
            })?;

            let (reader, writer) = tokio::io::split(pipe);
            spawn_client_worker(
                reader,
                writer,
                outbound_rx,
                event_tx.clone(),
                Arc::clone(&is_connected),
            );
        }

        #[cfg(unix)]
        {
            use tokio::net::UnixStream;
            let stream = UnixStream::connect(endpoint).await.map_err(|e| {
                StrataError::Internal(format!(
                    "Failed to connect to Unix socket '{}': {e}",
                    endpoint
                ))
            })?;

            let (reader, writer) = stream.into_split();
            spawn_client_worker(
                reader,
                writer,
                outbound_rx,
                event_tx.clone(),
                Arc::clone(&is_connected),
            );
        }

        #[cfg(not(any(windows, unix)))]
        {
            return Err(StrataError::Internal(
                "A2A IPC not supported on this platform".to_string(),
            ));
        }

        Ok(Self {
            endpoint: endpoint.to_string(),
            outbound_tx,
            event_tx,
            is_connected,
        })
    }

    /// Publishes an event over the IPC connection.
    pub async fn publish(&self, event: IpcEvent) -> Result<(), StrataError> {
        self.outbound_tx
            .send(IpcMessage::Event { event })
            .await
            .map_err(|_| StrataError::Internal("IPC client worker channel closed".to_string()))
    }

    /// Subscribes to real-time events received from the IPC bus.
    pub fn subscribe(&self) -> broadcast::Receiver<IpcEvent> {
        self.event_tx.subscribe()
    }

    /// Returns whether the client is currently connected.
    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::SeqCst)
    }

    /// Endpoint string of this client.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

fn spawn_client_worker<R, W>(
    reader: R,
    mut writer: W,
    mut outbound_rx: mpsc::Receiver<IpcMessage>,
    event_tx: broadcast::Sender<IpcEvent>,
    is_connected: Arc<AtomicBool>,
) where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buf_reader = BufReader::new(reader);

        loop {
            tokio::select! {
                // 1. Outgoing messages from client methods
                Some(msg) = outbound_rx.recv() => {
                    if let Ok(mut json) = serde_json::to_string(&msg) {
                        json.push('\n');
                        if writer.write_all(json.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                }
                // 2. Incoming messages from server
                read_res = read_bounded_line(&mut buf_reader, MAX_FRAME_LENGTH) => {
                    match read_res {
                        Ok(Some(line)) => {
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            match serde_json::from_str::<IpcMessage>(trimmed) {
                                Ok(IpcMessage::Event { event }) => {
                                    let _ = event_tx.send(event);
                                }
                                Ok(IpcMessage::Ping { timestamp }) => {
                                    let pong = IpcMessage::Pong { timestamp };
                                    if let Ok(mut json) = serde_json::to_string(&pong) {
                                        json.push('\n');
                                        let _ = writer.write_all(json.as_bytes()).await;
                                    }
                                }
                                Ok(IpcMessage::Pong { .. }) => {}
                                Err(e) => {
                                    debug!("Client ignored unparseable frame: {e}");
                                }
                            }
                        }
                        Ok(None) | Err(_) => {
                            break;
                        }
                    }
                }
            }
        }

        is_connected.store(false, Ordering::SeqCst);
    });
}

/// High-level manager that coordinates local leader election for the workspace IPC bus.
///
/// If an active broker is running, it connects as a client.
/// If no broker is detected, it opportunistically spawns the embedded broker.
pub struct IpcBrokerManager;

impl IpcBrokerManager {
    /// Connects to an existing broker or starts an embedded broker if none is active.
    pub async fn start_or_connect(
        workspace_root: &Path,
    ) -> (Option<Arc<IpcClient>>, Option<Arc<IpcServer>>) {
        let (endpoint, _) = endpoint_for_workspace(workspace_root);

        // 1. Try connecting as a client first
        if let Ok(client) = IpcClient::connect(&endpoint).await {
            info!("Connected to existing workspace A2A IPC broker at {endpoint}");
            return (Some(Arc::new(client)), None);
        }

        // 2. No active broker found: attempt to elect self as the broker
        match IpcServer::bind(&endpoint).await {
            Ok(server) => {
                info!("Elected as workspace A2A IPC broker at {endpoint}");
                let server_arc = Arc::new(server);
                // Connect local client to self
                tokio::time::sleep(Duration::from_millis(50)).await;
                let client = IpcClient::connect(&endpoint).await.ok().map(Arc::new);
                (client, Some(server_arc))
            }
            Err(e) => {
                // If bind failed (e.g. race condition where another process bound just before us)
                warn!("Could not bind A2A IPC server ({e}), attempting secondary client connect");
                tokio::time::sleep(Duration::from_millis(50)).await;
                let client = IpcClient::connect(&endpoint).await.ok().map(Arc::new);
                (client, None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_bounded_frame_reader() {
        let raw = "hello world\n";
        let mut reader = BufReader::new(raw.as_bytes());
        let res = read_bounded_line(&mut reader, 64).await.unwrap();
        assert_eq!(res.as_deref(), Some("hello world\n"));

        // Oversized line without newline
        let oversized = "a".repeat(100);
        let mut reader_over = BufReader::new(oversized.as_bytes());
        let err = read_bounded_line(&mut reader_over, 50).await;
        assert!(err.is_err(), "Must error when frame exceeds max_bytes");
    }

    #[tokio::test]
    async fn test_ipc_server_client_broadcast() {
        let test_id = uuid::Uuid::new_v4();
        #[cfg(windows)]
        let endpoint = format!(r"\\.\pipe\strata-test-{}", test_id);
        #[cfg(unix)]
        let endpoint = format!("/tmp/strata-test-{}.sock", &test_id.to_string()[..8]);

        // Start server
        let server = IpcServer::bind(&endpoint).await.expect("bind server");
        assert!(server.is_running());

        // Connect client 1
        let client1 = IpcClient::connect(&endpoint)
            .await
            .expect("client 1 connect");
        assert!(client1.is_connected());
        let mut sub1 = client1.subscribe();

        // Connect client 2
        let client2 = IpcClient::connect(&endpoint)
            .await
            .expect("client 2 connect");
        assert!(client2.is_connected());
        let mut sub2 = client2.subscribe();

        // Allow async connection tasks to settle
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Server broadcasts event
        let event = IpcEvent::LeaseAcquired {
            resource_id: "crate:strata-memory".to_string(),
            agent_id: "agent-alpha".to_string(),
            expires_at: Utc::now().timestamp() + 60,
            metadata: Some("running tests".to_string()),
            timestamp_us: Utc::now().timestamp_micros(),
        };

        server.broadcast(event.clone()).expect("server broadcast");

        // Client 1 receives event
        let recv1 = tokio::time::timeout(Duration::from_secs(3), sub1.recv())
            .await
            .expect("timeout sub1")
            .expect("recv sub1");
        assert_eq!(recv1, event);

        // Client 2 receives event
        let recv2 = tokio::time::timeout(Duration::from_secs(3), sub2.recv())
            .await
            .expect("timeout sub2")
            .expect("recv sub2");
        assert_eq!(recv2, event);

        // Client 1 publishes event over IPC
        let event2 = IpcEvent::LeaseReleased {
            resource_id: "crate:strata-memory".to_string(),
            agent_id: "agent-alpha".to_string(),
            timestamp_us: Utc::now().timestamp_micros(),
        };

        client1
            .publish(event2.clone())
            .await
            .expect("client publish");

        // Client 2 receives event published by Client 1
        let recv_pub2 = tokio::time::timeout(Duration::from_secs(3), sub2.recv())
            .await
            .expect("timeout sub2 for client pub")
            .expect("recv sub2 for client pub");
        assert_eq!(recv_pub2, event2);

        server.stop();
    }

    #[tokio::test]
    async fn test_broker_leader_election() {
        let temp_dir = std::env::temp_dir().join(format!("strata_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        // 1. First process auto-elects as broker
        let (client1, server1) = IpcBrokerManager::start_or_connect(&temp_dir).await;
        assert!(server1.is_some(), "First process must become server/broker");
        assert!(client1.is_some(), "First process must have client");

        // 2. Second process connects as client to existing broker
        let (client2, server2) = IpcBrokerManager::start_or_connect(&temp_dir).await;
        assert!(
            server2.is_none(),
            "Second process must NOT start another server"
        );
        assert!(client2.is_some(), "Second process must connect as client");

        // Clean shutdown
        if let Some(srv) = server1 {
            srv.stop();
        }
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
