use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::broadcast;
use pandamux_protocol::{RpcRequest, ServerRole};
use crate::router::Router;
use crate::runtime::RuntimeInfo;
use crate::store::Store;

pub struct ServerConfig {
    pub role: ServerRole,
    pub environment_id: String,
    pub runtime_dir: PathBuf,
    pub db_path: Option<PathBuf>,
}

pub struct Server {
    config: ServerConfig,
    store: Store,
    router: Arc<Router>,
    shutdown_tx: broadcast::Sender<()>,
    is_shutting_down: Arc<AtomicBool>,
}

impl Server {
    pub fn new(config: ServerConfig) -> Result<Self, crate::store::StoreError> {
        let store = match &config.db_path {
            Some(path) => Store::open(path)?,
            None => Store::in_memory()?,
        };

        let router = Arc::new(Router::new(
            store.clone(),
            config.role,
            config.environment_id.clone(),
        ));

        let (shutdown_tx, _) = broadcast::channel(1);

        Ok(Self {
            config,
            store,
            router,
            shutdown_tx,
            is_shutting_down: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn router(&self) -> &Arc<Router> {
        &self.router
    }

    /// Trigger provable graceful shutdown.
    pub fn shutdown(&self) {
        if !self.is_shutting_down.swap(true, Ordering::SeqCst) {
            let _ = self.shutdown_tx.send(());
        }
    }

    /// Write runtime discovery record.
    pub fn register_runtime(&self, pipe_path: &str, token: &str) -> std::io::Result<PathBuf> {
        let info = RuntimeInfo {
            pid: std::process::id(),
            role: match self.config.role {
                ServerRole::Hub => "hub".to_string(),
                ServerRole::Node => "node".to_string(),
            },
            protocol_version: pandamux_protocol::PROTOCOL_VERSION,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            pipe_path: pipe_path.to_string(),
            token: token.to_string(),
            started_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };

        info.write_to_dir(&self.config.runtime_dir)
    }

    /// Clean up runtime file on shutdown.
    pub fn cleanup_runtime(&self) {
        RuntimeInfo::remove_from_dir(&self.config.runtime_dir);
        let _ = self.store.checkpoint_truncate();
    }

    /// Process a single incoming line and return an optional serialized response line.
    pub async fn process_line(&self, line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        let req: RpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let err_res = pandamux_protocol::RpcResponse::error(
                    None,
                    pandamux_protocol::RpcError::parse_error(e.to_string()),
                );
                return Some(serde_json::to_string(&err_res).unwrap() + "\n");
            }
        };

        let res = self.router.handle_request(req).await?;
        Some(serde_json::to_string(&res).unwrap() + "\n")
    }
}

