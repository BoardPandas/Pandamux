use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pandamux_protocol::{
    EventEnvelope, HelloParams, PROTOCOL_VERSION, RpcId, RpcRequest, RpcResponse,
    SubscribeParams,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex};

pub const RUNTIME_FILENAME: &str = "server.json";

/// Metadata written into `server.json` for client discovery and authentication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub pid: u32,
    pub role: String,
    pub protocol_version: u32,
    pub server_version: String,
    pub pipe_path: String,
    pub token: String,
    pub started_at_ms: u64,
}

impl RuntimeInfo {
    /// Attempts to read the discovery record from a directory.
    pub fn read_from_dir(dir: &Path) -> std::io::Result<Option<Self>> {
        let path = dir.join(RUNTIME_FILENAME);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)?;
        let info = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Some(info))
    }
}

/// Discovers the standard runtime directory used by PandaMUX servers.
pub fn default_runtime_dir() -> PathBuf {
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_app_data).join("pandamux").join("run")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".pandamux").join("run")
    } else {
        std::env::temp_dir().join("pandamux").join("run")
    }
}

/// Locates `server.json` across standard runtime directories.
pub fn discover_server_runtime() -> Option<RuntimeInfo> {
    let runtime_dir = default_runtime_dir();
    if let Ok(Some(info)) = RuntimeInfo::read_from_dir(&runtime_dir) {
        return Some(info);
    }

    // Fallback to temp dir
    let temp_dir = std::env::temp_dir().join("pandamux").join("run");
    if temp_dir != runtime_dir {
        if let Ok(Some(info)) = RuntimeInfo::read_from_dir(&temp_dir) {
            return Some(info);
        }
    }

    None
}

/// Current connection status to the backend PandaMUX server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerStatus {
    Disconnected,
    Connecting,
    Connected {
        pipe_path: String,
        pid: u32,
        server_version: String,
    },
    Failed(String),
}

/// Commands dispatched from GPUI views to the background Tokio bridge.
pub enum BridgeCommand {
    Request {
        method: String,
        params: Option<serde_json::Value>,
        response_tx: oneshot::Sender<Result<RpcResponse, String>>,
    },
}

/// Handle owning the background Tokio bridge thread.
///
/// Follows LL-G `join-on-drop-sender-field-order`: channel senders are declared
/// before join handles so channels close before the worker thread is joined.
pub struct ServerBridgeHandle {
    // Senders declared FIRST
    command_tx: Option<mpsc::UnboundedSender<BridgeCommand>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    // Joined thread handle declared LAST
    worker_thread: Option<std::thread::JoinHandle<()>>,
}

impl ServerBridgeHandle {
    /// Dispatches an RPC request to the server and returns a oneshot receiver for the response.
    pub fn send_request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> oneshot::Receiver<Result<RpcResponse, String>> {
        let (tx, rx) = oneshot::channel();
        if let Some(cmd_tx) = &self.command_tx {
            let _ = cmd_tx.send(BridgeCommand::Request {
                method: method.to_string(),
                params,
                response_tx: tx,
            });
        } else {
            let _ = tx.send(Err("Bridge is not running".to_string()));
        }
        rx
    }
}

impl Drop for ServerBridgeHandle {
    fn drop(&mut self) {
        // Take and drop channel senders first to unblock worker loops
        self.shutdown_tx.take();
        self.command_tx.take();

        if let Some(handle) = self.worker_thread.take() {
            let _ = handle.join();
        }
    }
}

/// Spawns the dedicated Tokio runtime thread for server discovery and IPC streaming.
pub fn spawn_server_bridge(
    event_tx: smol::channel::Sender<EventEnvelope>,
    status_tx: smol::channel::Sender<ServerStatus>,
) -> ServerBridgeHandle {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<BridgeCommand>();
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

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
            let _ = status_tx.send(ServerStatus::Connecting).await;

            // 1. Discover or auto-spawn server
            let runtime_info = match ensure_server_running().await {
                Ok(info) => info,
                Err(err) => {
                    let _ = status_tx.send(ServerStatus::Failed(err)).await;
                    return;
                }
            };

            let pipe_path = runtime_info.pipe_path.clone();

            // 2. Connect over IPC
            let stream_res = connect_ipc(&pipe_path).await;
            let stream = match stream_res {
                Ok(s) => s,
                Err(e) => {
                    let _ = status_tx
                        .send(ServerStatus::Failed(format!("IPC connection failed: {e}")))
                        .await;
                    return;
                }
            };

            let (reader, mut writer) = tokio::io::split(stream);
            let mut lines = BufReader::new(reader).lines();

            let pending_requests: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<RpcResponse, String>>>>> =
                Arc::new(Mutex::new(HashMap::new()));
            let next_request_id = Arc::new(AtomicI64::new(1));

            // 3. Initial system.hello handshake
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

            let (hello_resp_tx, _hello_resp_rx) = oneshot::channel();
            {
                let mut pending = pending_requests.lock().await;
                pending.insert(hello_id, hello_resp_tx);
            }

            let mut hello_line = serde_json::to_string(&hello_req).unwrap_or_default();
            hello_line.push('\n');
            if writer.write_all(hello_line.as_bytes()).await.is_err() {
                let _ = status_tx
                    .send(ServerStatus::Failed("Failed to send handshake".to_string()))
                    .await;
                return;
            }

            // 4. Initial system.subscribe for thread events
            let sub_id = next_request_id.fetch_add(1, Ordering::SeqCst);
            let sub_params = SubscribeParams {
                topic: "thread.events".to_string(),
                environment_id: None,
                thread_id: None,
                run_id: None,
                since_seq: None,
            };
            let sub_req = RpcRequest::new(
                RpcId::Number(sub_id),
                "system.subscribe",
                Some(serde_json::to_value(&sub_params).unwrap_or_default()),
            );

            let mut sub_line = serde_json::to_string(&sub_req).unwrap_or_default();
            sub_line.push('\n');
            let _ = writer.write_all(sub_line.as_bytes()).await;

            let _ = status_tx
                .send(ServerStatus::Connected {
                    pipe_path: runtime_info.pipe_path,
                    pid: runtime_info.pid,
                    server_version: runtime_info.server_version,
                })
                .await;

            // 5. Main event and request loop
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        break;
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
                                    let _ = writer.write_all(json.as_bytes()).await;
                                }
                            }
                            None => break,
                        }
                    }

                    line_res = lines.next_line() => {
                        match line_res {
                            Ok(Some(line)) => {
                                // Try parsing as EventEnvelope first
                                if let Ok(envelope) = serde_json::from_str::<EventEnvelope>(&line) {
                                    let _ = event_tx.send(envelope).await;
                                    continue;
                                }

                                // Try parsing as RpcResponse
                                if let Ok(response) = serde_json::from_str::<RpcResponse>(&line) {
                                    if let Some(RpcId::Number(id)) = response.id {
                                        let mut pending = pending_requests.lock().await;
                                        if let Some(tx) = pending.remove(&id) {
                                            let _ = tx.send(Ok(response));
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                let _ = status_tx.send(ServerStatus::Disconnected).await;
                                break;
                            }
                            Err(_) => {
                                let _ = status_tx.send(ServerStatus::Disconnected).await;
                                break;
                            }
                        }
                    }
                }
            }
        });
    });

    ServerBridgeHandle {
        command_tx: Some(command_tx),
        shutdown_tx: Some(shutdown_tx),
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

/// Locates the `pandamux-server` binary in the current path, next to the executable, or in target directory.
fn locate_server_binary() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(if cfg!(windows) {
                "pandamux-server.exe"
            } else {
                "pandamux-server"
            });
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // Check cargo build target dirs
    let target_debug = PathBuf::from("target/debug").join(if cfg!(windows) {
        "pandamux-server.exe"
    } else {
        "pandamux-server"
    });
    if target_debug.exists() {
        return Some(target_debug);
    }

    None
}

/// Connects to the local IPC pipe or domain socket.
#[cfg(windows)]
async fn connect_ipc(
    pipe_path: &str,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(pipe_path)
}

/// Connects to the local Unix domain socket.
#[cfg(not(windows))]
async fn connect_ipc(socket_path: &str) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(socket_path).await
}
