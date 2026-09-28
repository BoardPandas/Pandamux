use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use pandamux_protocol::{
    EventEnvelope, HelloParams, PROTOCOL_VERSION, RpcId, RpcRequest, RpcResponse, SubscribeParams,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

/// Represents server connection status observed by the GPUI desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerStatus {
    Connecting,
    Connected {
        pipe_path: String,
        pid: u32,
        server_version: String,
    },
    Disconnected,
    Failed(String),
}

impl std::fmt::Display for ServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connecting => write!(f, "Connecting..."),
            Self::Connected {
                server_version,
                pid,
                ..
            } => {
                write!(f, "Connected (v{}, PID {})", server_version, pid)
            }
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Failed(err) => write!(f, "Failed: {}", err),
        }
    }
}

/// Runtime metadata read from `~/.pandamux/server.json` or `%LOCALAPPDATA%/pandamux/server.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub pid: u32,
    pub role: String,
    pub server_version: String,
    pub protocol_version: u32,
    pub pipe_path: String,
    pub token: String,
    pub started_at_ms: u64,
}

pub type PendingRequests = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<RpcResponse, String>>>>>;

/// Commands sent from the GPUI application to the background Tokio bridge.
pub enum BridgeCommand {
    Request {
        method: &'static str,
        params: Option<serde_json::Value>,
        response_tx: oneshot::Sender<Result<RpcResponse, String>>,
    },
}

/// Handle allowing the GPUI thread to interact with the Tokio server bridge.
///
/// NOTE on Drop ordering: `command_tx` and `shutdown_tx` are declared BEFORE
/// `worker_thread` so that dropping the handle drops the senders first,
/// unblocking any Tokio `recv()` before `worker_thread.join()` runs.
/// This prevents thread join deadlocks per LL-G `join-on-drop-sender-field-order`.
pub struct ServerBridgeHandle {
    command_tx: Option<mpsc::Sender<BridgeCommand>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    last_seen_seq: Arc<AtomicU64>,
    worker_thread: Option<std::thread::JoinHandle<()>>,
}

impl ServerBridgeHandle {
    /// Dispatches an asynchronous RPC request to the connected server.
    pub fn send_request(
        &self,
        method: &'static str,
        params: Option<serde_json::Value>,
    ) -> oneshot::Receiver<Result<RpcResponse, String>> {
        let (tx, rx) = oneshot::channel();
        if let Some(cmd_tx) = &self.command_tx {
            let _ = cmd_tx.try_send(BridgeCommand::Request {
                method,
                params,
                response_tx: tx,
            });
        } else {
            let _ = tx.send(Err("Bridge channel closed".to_string()));
        }
        rx
    }

    /// Returns a cloned command sender allowing background tasks to send requests.
    pub fn request_sender(&self) -> Option<mpsc::Sender<BridgeCommand>> {
        self.command_tx.clone()
    }

    /// Returns the highest sequence number processed by this bridge.
    pub fn last_seen_seq(&self) -> u64 {
        self.last_seen_seq.load(Ordering::SeqCst)
    }
}

/// Dispatches an RPC request over an optional BridgeCommand sender channel.
pub fn send_request_channel(
    cmd_tx: &Option<mpsc::Sender<BridgeCommand>>,
    method: &'static str,
    params: Option<serde_json::Value>,
) -> oneshot::Receiver<Result<RpcResponse, String>> {
    let (tx, rx) = oneshot::channel();
    if let Some(chan) = cmd_tx {
        let _ = chan.try_send(BridgeCommand::Request {
            method,
            params,
            response_tx: tx,
        });
    } else {
        let _ = tx.send(Err("Bridge channel closed".to_string()));
    }
    rx
}

impl Drop for ServerBridgeHandle {
    fn drop(&mut self) {
        // 1. Signal shutdown and drop sender channels explicitly
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.try_send(());
            drop(tx);
        }
        self.command_tx.take();

        // 2. Safely join the bridge worker thread
        if let Some(handle) = self.worker_thread.take() {
            let _ = handle.join();
        }
    }
}

/// Spawns the background Tokio server bridge with `sinceSeq` reconnection support.
pub fn spawn_server_bridge(
    event_tx: smol::channel::Sender<EventEnvelope>,
    status_tx: smol::channel::Sender<ServerStatus>,
) -> ServerBridgeHandle {
    let (command_tx, mut command_rx) = mpsc::channel::<BridgeCommand>(128);
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let last_seen_seq = Arc::new(AtomicU64::new(0));
    let last_seen_seq_worker = Arc::clone(&last_seen_seq);

    let worker_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build();

        let rt = match rt {
            Ok(r) => r,
            Err(e) => {
                let _ = status_tx.try_send(ServerStatus::Failed(format!(
                    "Failed to build tokio runtime: {e}"
                )));
                return;
            }
        };

        rt.block_on(async move {
            let pending_requests: PendingRequests =
                Arc::new(Mutex::new(HashMap::new()));
            let next_request_id = Arc::new(AtomicI64::new(1));
            let mut retry_backoff_ms = 250;

            'reconnect_loop: loop {
                if shutdown_rx.try_recv().is_ok() {
                    break 'reconnect_loop;
                }

                let _ = status_tx.send(ServerStatus::Connecting).await;

                // 1. Discover or auto-spawn server
                let runtime_info = match ensure_server_running().await {
                    Ok(info) => info,
                    Err(err) => {
                        let _ = status_tx.send(ServerStatus::Failed(err)).await;
                        tokio::select! {
                            _ = shutdown_rx.recv() => break 'reconnect_loop,
                            _ = tokio::time::sleep(Duration::from_millis(retry_backoff_ms)) => {
                                retry_backoff_ms = (retry_backoff_ms * 3 / 2).min(3000);
                                continue 'reconnect_loop;
                            }
                        }
                    }
                };

                let pipe_path = runtime_info.pipe_path.clone();

                // 2. Connect over IPC
                let stream_res = connect_ipc(&pipe_path).await;
                let stream = match stream_res {
                    Ok(s) => {
                        retry_backoff_ms = 250;
                        s
                    }
                    Err(e) => {
                        let _ = status_tx
                            .send(ServerStatus::Failed(format!("IPC connection failed: {e}")))
                            .await;
                        tokio::select! {
                            _ = shutdown_rx.recv() => break 'reconnect_loop,
                            _ = tokio::time::sleep(Duration::from_millis(retry_backoff_ms)) => {
                                retry_backoff_ms = (retry_backoff_ms * 3 / 2).min(3000);
                                continue 'reconnect_loop;
                            }
                        }
                    }
                };

                let (reader, mut writer) = tokio::io::split(stream);
                let mut lines = BufReader::new(reader).lines();

                // 3. Handshake system.hello
                let hello_id = next_request_id.fetch_add(1, Ordering::SeqCst);
                let hello_params = HelloParams {
                    protocol_version: PROTOCOL_VERSION,
                    client_kind: "desktop".to_string(),
                    client_version: env!("CARGO_PKG_VERSION").to_string(),
                };
                let hello_req = RpcRequest::new(
                    RpcId::Number(hello_id),
                    "system.hello",
                    Some(serde_json::to_value(&hello_params).unwrap_or_default()),
                );

                let mut hello_line = serde_json::to_string(&hello_req).unwrap_or_default();
                hello_line.push('\n');
                if writer.write_all(hello_line.as_bytes()).await.is_err() {
                    let _ = status_tx.send(ServerStatus::Disconnected).await;
                    tokio::select! {
                        _ = shutdown_rx.recv() => break 'reconnect_loop,
                        _ = tokio::time::sleep(Duration::from_millis(500)) => continue 'reconnect_loop,
                    }
                }

                // 4. system.subscribe with since_seq from tracked sequence!
                let current_seq = last_seen_seq_worker.load(Ordering::SeqCst);
                let since_seq = if current_seq > 0 {
                    Some(current_seq)
                } else {
                    None
                };

                let sub_id = next_request_id.fetch_add(1, Ordering::SeqCst);
                let sub_params = SubscribeParams {
                    topic: "thread.events".to_string(),
                    environment_id: None,
                    thread_id: None,
                    run_id: None,
                    since_seq,
                };
                let sub_req = RpcRequest::new(
                    RpcId::Number(sub_id),
                    "system.subscribe",
                    Some(serde_json::to_value(&sub_params).unwrap_or_default()),
                );

                let mut sub_line = serde_json::to_string(&sub_req).unwrap_or_default();
                sub_line.push('\n');
                if writer.write_all(sub_line.as_bytes()).await.is_err() {
                    let _ = status_tx.send(ServerStatus::Disconnected).await;
                    tokio::select! {
                        _ = shutdown_rx.recv() => break 'reconnect_loop,
                        _ = tokio::time::sleep(Duration::from_millis(500)) => continue 'reconnect_loop,
                    }
                }

                let _ = status_tx
                    .send(ServerStatus::Connected {
                        pipe_path: runtime_info.pipe_path,
                        pid: runtime_info.pid,
                        server_version: runtime_info.server_version,
                    })
                    .await;

                // 5. Main event and request loop
                'event_loop: loop {
                    tokio::select! {
                        _ = shutdown_rx.recv() => {
                            break 'reconnect_loop;
                        }

                        cmd = command_rx.recv() => {
                            match cmd {
                                Some(BridgeCommand::Request { method, params, response_tx }) => {
                                    let id = next_request_id.fetch_add(1, Ordering::SeqCst);
                                    {
                                        let mut pending = pending_requests.lock().await;
                                        pending.insert(id, response_tx);
                                    }
                                    let req = RpcRequest::new(RpcId::Number(id), method, params);
                                    if let Ok(mut json) = serde_json::to_string(&req) {
                                        json.push('\n');
                                        if writer.write_all(json.as_bytes()).await.is_err() {
                                            let _ = status_tx.send(ServerStatus::Disconnected).await;
                                            break 'event_loop;
                                        }
                                    }
                                }
                                None => break 'reconnect_loop,
                            }
                        }

                        line_res = lines.next_line() => {
                            match line_res {
                                Ok(Some(line)) => {
                                    // Parse EventEnvelope and track highest sequence number
                                    if let Ok(envelope) = serde_json::from_str::<EventEnvelope>(&line) {
                                        last_seen_seq_worker.fetch_max(envelope.seq, Ordering::SeqCst);
                                        let _ = event_tx.send(envelope).await;
                                        continue;
                                    }

                                    // Parse RpcResponse
                                    if let Ok(response) = serde_json::from_str::<RpcResponse>(&line)
                                        && let Some(RpcId::Number(id)) = response.id {
                                            let mut pending = pending_requests.lock().await;
                                            if let Some(tx) = pending.remove(&id) {
                                                let _ = tx.send(Ok(response));
                                            }
                                        }
                                }
                                Ok(None) => {
                                    let _ = status_tx.send(ServerStatus::Disconnected).await;
                                    break 'event_loop;
                                }
                                Err(_) => {
                                    let _ = status_tx.send(ServerStatus::Disconnected).await;
                                    break 'event_loop;
                                }
                            }
                        }
                    }
                }

                // Pause briefly before attempting reconnection
                tokio::select! {
                    _ = shutdown_rx.recv() => break 'reconnect_loop,
                    _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                }
            }
        });
    });

    ServerBridgeHandle {
        command_tx: Some(command_tx),
        shutdown_tx: Some(shutdown_tx),
        last_seen_seq,
        worker_thread: Some(worker_thread),
    }
}

/// Ensures `server.json` is available by discovering an active instance or spawning one.
async fn ensure_server_running() -> Result<RuntimeInfo, String> {
    if let Some(info) = discover_server_runtime() {
        return Ok(info);
    }

    // Attempt to locate pandamux-server executable
    let server_bin = locate_server_binary();
    if let Some(bin_path) = server_bin {
        let mut cmd = std::process::Command::new(bin_path);
        cmd.arg("--role").arg("hub");

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let _ = cmd.spawn();
    }

    // Poll for server.json with exponential backoff up to 2.5 seconds
    let mut delay_ms = 50;
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        if let Some(info) = discover_server_runtime() {
            return Ok(info);
        }
        delay_ms = (delay_ms * 3 / 2).min(300);
    }

    Err("PandaMUX server discovery timed out".to_string())
}

/// Locates the `pandamux-server` binary in debug/release target directories or PATH.
fn locate_server_binary() -> Option<PathBuf> {
    let mut candidate = std::env::current_exe().ok()?;
    candidate.pop(); // directory of current executable

    let exe_name = if cfg!(windows) {
        "pandamux-server.exe"
    } else {
        "pandamux-server"
    };

    let local_path = candidate.join(exe_name);
    if local_path.exists() {
        return Some(local_path);
    }

    // Search up to workspace target directory
    candidate.pop(); // target/
    let debug_path = candidate.join("debug").join(exe_name);
    if debug_path.exists() {
        return Some(debug_path);
    }

    let release_path = candidate.join("release").join(exe_name);
    if release_path.exists() {
        return Some(release_path);
    }

    None
}

/// Reads and parses `server.json` if it exists.
pub fn discover_server_runtime() -> Option<RuntimeInfo> {
    let server_json_path = get_server_runtime_path()?;
    if !server_json_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(server_json_path).ok()?;
    serde_json::from_str::<RuntimeInfo>(&content).ok()
}

/// Resolves standard location for `server.json`.
fn get_server_runtime_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
        Some(
            PathBuf::from(local_app_data)
                .join("pandamux")
                .join("server.json"),
        )
    }

    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".pandamux").join("server.json"))
    }
}

/// Connects to server IPC (Windows Named Pipe or Unix Domain Socket).
async fn connect_ipc(pipe_path: &str) -> Result<Box<dyn AsyncReadWrite + Send>, std::io::Error> {
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        let client = ClientOptions::new().open(pipe_path)?;
        Ok(Box::new(client))
    }

    #[cfg(not(windows))]
    {
        use tokio::net::UnixStream;
        let stream = UnixStream::connect(pipe_path).await?;
        Ok(Box::new(stream))
    }
}

/// Helper trait combining Tokio's `AsyncRead`, `AsyncWrite`, and `Unpin`.
pub trait AsyncReadWrite: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> AsyncReadWrite for T {}
