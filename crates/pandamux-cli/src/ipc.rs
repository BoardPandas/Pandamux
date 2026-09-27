use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use pandamux_protocol::{RpcId, RpcRequest, RpcResponse};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf};

/// Cross-platform IPC stream combining AsyncRead, AsyncWrite, and Unpin.
pub trait AsyncReadWrite: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

/// Represents an active IPC connection to `pandamux-server`.
pub struct IpcClient {
    lines: Lines<BufReader<ReadHalf<Box<dyn AsyncReadWrite>>>>,
    writer: WriteHalf<Box<dyn AsyncReadWrite>>,
    next_id: Arc<AtomicI64>,
}

impl IpcClient {
    /// Connects to `pandamux-server` using the specified or auto-discovered pipe path.
    pub async fn connect(explicit_pipe: Option<&str>) -> Result<Self, String> {
        let pipe_path = match explicit_pipe {
            Some(p) => p.to_string(),
            None => resolve_pipe_path()?,
        };

        let stream: Box<dyn AsyncReadWrite> = connect_stream(&pipe_path).await?;
        let (reader, writer) = tokio::io::split(stream);
        let lines = BufReader::new(reader).lines();

        Ok(Self {
            lines,
            writer,
            next_id: Arc::new(AtomicI64::new(1)),
        })
    }

    /// Dispatches an RPC method call and waits for the response.
    pub async fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = RpcRequest::new(RpcId::Number(id), method, params);

        let mut line = serde_json::to_string(&request)
            .map_err(|e| format!("Failed to serialize request: {e}"))?;
        line.push('\n');

        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("IPC write error: {e}"))?;
        self.writer
            .flush()
            .await
            .map_err(|e| format!("IPC flush error: {e}"))?;

        while let Some(line) = self
            .lines
            .next_line()
            .await
            .map_err(|e| format!("IPC read error: {e}"))?
        {
            if let Ok(resp) = serde_json::from_str::<RpcResponse>(&line)
                && let Some(resp_id) = resp.id
                && resp_id == RpcId::Number(id)
            {
                if let Some(err) = resp.error {
                    return Err(format!("{}: {}", err.code, err.message));
                }
                return Ok(resp.result.unwrap_or(Value::Null));
            }
        }

        Err("IPC connection closed unexpectedly".to_string())
    }

    /// Dispatches a typed RPC method call.
    pub async fn call_typed<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: &P,
    ) -> Result<R, String> {
        let val =
            serde_json::to_value(params).map_err(|e| format!("Failed to serialize params: {e}"))?;
        let result = self.call(method, Some(val)).await?;
        serde_json::from_value(result).map_err(|e| format!("Failed to parse response: {e}"))
    }

    /// Reads the next incoming line from the server.
    pub async fn read_line(&mut self) -> Result<Option<String>, String> {
        self.lines
            .next_line()
            .await
            .map_err(|e| format!("IPC read error: {e}"))
    }

    /// Sends a raw JSON-RPC string line to the server.
    pub async fn write_line(&mut self, line: &str) -> Result<(), String> {
        let mut msg = line.to_string();
        if !msg.ends_with('\n') {
            msg.push('\n');
        }
        self.writer
            .write_all(msg.as_bytes())
            .await
            .map_err(|e| format!("IPC write error: {e}"))?;
        self.writer
            .flush()
            .await
            .map_err(|e| format!("IPC flush error: {e}"))
    }
}

/// Resolves the server pipe/socket path from env or `server.json`.
pub fn resolve_pipe_path() -> Result<String, String> {
    if let Ok(pipe) = std::env::var("PANDAMUX_PIPE")
        && !pipe.trim().is_empty()
    {
        return Ok(pipe.trim().to_string());
    }

    let server_json = get_server_runtime_path()
        .ok_or_else(|| "Failed to determine PandaMUX runtime path".to_string())?;

    if !server_json.exists() {
        return Err(format!(
            "PandaMUX server is not running (no server.json found at {}). Please start 'pandamux-server' or launch PandaMUX Desktop.",
            server_json.display()
        ));
    }

    let content = std::fs::read_to_string(&server_json)
        .map_err(|e| format!("Failed to read {}: {e}", server_json.display()))?;

    let v: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {e}", server_json.display()))?;

    v.get("pipePath")
        .and_then(|p| p.as_str())
        .map(String::from)
        .ok_or_else(|| "Missing 'pipePath' in server.json".to_string())
}

/// Resolves standard location for `server.json`.
pub fn get_server_runtime_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
        let p1 = PathBuf::from(&local_app_data)
            .join("pandamux")
            .join("server.json");
        if p1.exists() {
            return Some(p1);
        }
        let p2 = PathBuf::from(&local_app_data)
            .join("pandamux")
            .join("run")
            .join("server.json");
        if p2.exists() {
            return Some(p2);
        }
        Some(p1)
    }

    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").ok()?;
        let p1 = PathBuf::from(&home).join(".pandamux").join("server.json");
        if p1.exists() {
            return Some(p1);
        }
        let p2 = PathBuf::from(&home)
            .join(".pandamux")
            .join("run")
            .join("server.json");
        if p2.exists() {
            return Some(p2);
        }
        Some(p1)
    }
}

/// Connects to server IPC (Windows Named Pipe or Unix Domain Socket).
async fn connect_stream(pipe_path: &str) -> Result<Box<dyn AsyncReadWrite>, String> {
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        let client = ClientOptions::new()
            .open(pipe_path)
            .map_err(|e| format!("Failed to open named pipe '{pipe_path}': {e}"))?;
        Ok(Box::new(client))
    }

    #[cfg(not(windows))]
    {
        use tokio::net::UnixStream;
        let stream = UnixStream::connect(pipe_path)
            .await
            .map_err(|e| format!("Failed to connect to Unix socket '{pipe_path}': {e}"))?;
        Ok(Box::new(stream))
    }
}
