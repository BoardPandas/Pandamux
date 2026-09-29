use crate::router::Router;
use crate::runtime::RuntimeInfo;
use crate::store::Store;
use pandamux_core::EnvironmentId;
use pandamux_protocol::{RpcError, RpcRequest, ServerRole};
use pandamux_remote::{ProxyTunnel, SshConfig, SshConnectionPool, open_exec_proxy_tunnel};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

use crate::ProxyEnvironmentTransport;
use crate::request_context::{CapabilityAuthority, ClientIdentity};
use crate::usage_scanner::{UsageScannerRuntimeConfig, UsageScannerService};

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
    remote_event_cursors: Mutex<HashMap<EnvironmentId, Arc<AtomicU64>>>,
    usage_scanner: Mutex<Option<UsageScannerService>>,
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
            remote_event_cursors: Mutex::new(HashMap::new()),
            usage_scanner: Mutex::new(None),
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn router(&self) -> &Arc<Router> {
        &self.router
    }

    pub fn environment_id(&self) -> &str {
        &self.config.environment_id
    }

    pub fn runtime_dir(&self) -> &std::path::Path {
        &self.config.runtime_dir
    }

    pub fn capability_authority(&self) -> &Arc<CapabilityAuthority> {
        self.router.capability_authority()
    }

    pub async fn connect_remote_environment(
        &self,
        environment_id: EnvironmentId,
        pool: &SshConnectionPool,
        config: &SshConfig,
        proxy_command: &str,
    ) -> Result<Arc<ProxyEnvironmentTransport>, RpcError> {
        let tunnel = open_exec_proxy_tunnel(pool, config, proxy_command)
            .await
            .map_err(|error| {
                RpcError::with_data(
                    -32010,
                    format!(
                        "Environment '{}' is unreachable: {error}",
                        environment_id.as_str()
                    ),
                    serde_json::json!({
                        "environmentId": environment_id,
                        "state": "unreachable",
                        "remoteCode": error.code,
                    }),
                )
            })?;
        self.register_proxy_stream_with_sftp(
            environment_id,
            tunnel,
            Some((pool.clone(), config.clone())),
        )
        .await
    }

    pub async fn register_proxy_tunnel(
        &self,
        environment_id: EnvironmentId,
        tunnel: ProxyTunnel,
    ) -> Result<Arc<ProxyEnvironmentTransport>, RpcError> {
        self.register_proxy_stream(environment_id, tunnel).await
    }

    pub async fn register_proxy_stream<S>(
        &self,
        environment_id: EnvironmentId,
        stream: S,
    ) -> Result<Arc<ProxyEnvironmentTransport>, RpcError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.register_proxy_stream_with_sftp(environment_id, stream, None)
            .await
    }

    async fn register_proxy_stream_with_sftp<S>(
        &self,
        environment_id: EnvironmentId,
        stream: S,
        sftp: Option<(SshConnectionPool, SshConfig)>,
    ) -> Result<Arc<ProxyEnvironmentTransport>, RpcError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let cursor = {
            let mut cursors = self
                .remote_event_cursors
                .lock()
                .expect("remote event cursor lock");
            cursors
                .entry(environment_id.clone())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };
        let transport = ProxyEnvironmentTransport::connect_with_sftp(
            stream,
            environment_id.clone(),
            cursor,
            sftp,
        )
        .await?;
        self.router
            .environment_router()
            .register_transport(
                environment_id,
                transport.clone(),
                Some(self.router.thread_manager().broadcaster().clone()),
            )
            .await;
        Ok(transport)
    }

    pub async fn disconnect_remote_environment(&self, environment_id: &EnvironmentId) {
        self.router
            .environment_router()
            .unregister_transport(environment_id)
            .await;
    }

    pub async fn shutdown_remote_environments(&self) {
        self.router.environment_router().shutdown_all().await;
    }

    pub async fn shutdown_provider_sessions(&self) {
        self.router.thread_manager().shutdown_all().await;
        self.router.shutdown_antigravity_auth().await;
    }

    pub fn start_usage_scanner(&self) -> std::io::Result<()> {
        let mut scanner = self
            .usage_scanner
            .lock()
            .expect("usage scanner service lock");
        if scanner.is_some() {
            return Ok(());
        }
        let service = UsageScannerService::start(
            self.store.clone(),
            EnvironmentId::new(self.config.environment_id.clone()),
            self.router.data_dir().to_path_buf(),
            UsageScannerRuntimeConfig::default(),
        )?;
        self.router
            .thread_manager()
            .set_usage_scanner(Some(service.handle()));
        *scanner = Some(service);
        Ok(())
    }

    pub async fn shutdown_usage_scanner(&self) -> std::io::Result<()> {
        self.router.thread_manager().set_usage_scanner(None);
        let scanner = self
            .usage_scanner
            .lock()
            .expect("usage scanner service lock")
            .take();
        if let Some(scanner) = scanner {
            scanner.shutdown().await?;
        }
        Ok(())
    }

    /// Trigger provable graceful shutdown.
    pub fn shutdown(&self) {
        if !self.is_shutting_down.swap(true, Ordering::SeqCst) {
            let _ = self.shutdown_tx.send(());
        }
    }

    /// Write runtime discovery record.
    pub fn register_runtime(&self, pipe_path: &str, token: &str) -> std::io::Result<PathBuf> {
        self.register_runtime_with_proxy(pipe_path, None, token)
    }

    pub fn register_runtime_with_proxy(
        &self,
        pipe_path: &str,
        proxy_path: Option<&str>,
        token: &str,
    ) -> std::io::Result<PathBuf> {
        self.register_runtime_with_proxy_and_ownership(pipe_path, proxy_path, token, false)
    }

    pub fn register_runtime_with_proxy_and_ownership(
        &self,
        pipe_path: &str,
        proxy_path: Option<&str>,
        token: &str,
        launcher_owned: bool,
    ) -> std::io::Result<PathBuf> {
        let mcp_token = uuid::Uuid::new_v4().to_string();
        self.capability_authority().configure_local_capabilities(
            pipe_path.to_string(),
            token.to_string(),
            mcp_token.clone(),
        );
        let info = RuntimeInfo {
            pid: std::process::id(),
            role: match self.config.role {
                ServerRole::Hub => "hub".to_string(),
                ServerRole::Node => "node".to_string(),
            },
            protocol_version: pandamux_protocol::PROTOCOL_VERSION,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            environment_id: self.config.environment_id.clone(),
            pipe_path: pipe_path.to_string(),
            proxy_path: proxy_path.map(str::to_string),
            token: token.to_string(),
            mcp_token,
            launcher_owned,
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

    pub async fn serve_local_connection<S>(
        &self,
        stream: S,
        client_identity: ClientIdentity,
    ) -> std::io::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let (reader, mut writer) = tokio::io::split(stream);
        let mut lines = BufReader::new(reader).lines();
        let mut context = None;
        while let Some(line) = lines.next_line().await? {
            let mut request: RpcRequest = match serde_json::from_str(line.trim()) {
                Ok(request) => request,
                Err(error) => {
                    let response = pandamux_protocol::RpcResponse::error(
                        None,
                        RpcError::parse_error(error.to_string()),
                    );
                    write_response(&mut writer, &response).await?;
                    continue;
                }
            };

            if context.is_none() {
                if request.method != "system.hello" {
                    let response = pandamux_protocol::RpcResponse::error(
                        request.id,
                        RpcError::unauthorized(
                            "An authenticated system.hello request is required first",
                        ),
                    );
                    write_response(&mut writer, &response).await?;
                    return Ok(());
                }
                let hello = request
                    .params
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "system.hello omitted its capability",
                        )
                    })
                    .and_then(|params| {
                        serde_json::from_value::<pandamux_protocol::HelloParams>(params).map_err(
                            |error| std::io::Error::new(std::io::ErrorKind::InvalidData, error),
                        )
                    });
                let authenticated = hello
                    .ok()
                    .and_then(|hello| hello.capability)
                    .ok_or_else(|| RpcError::unauthorized("Invalid or expired capability"))
                    .and_then(|token| {
                        self.capability_authority()
                            .authenticate(&token, client_identity.clone())
                    });
                match authenticated {
                    Ok(authenticated) => {
                        context = Some(authenticated);
                        if let Some(params) = request.params.as_mut().and_then(Value::as_object_mut)
                        {
                            params.remove("capability");
                        }
                    }
                    Err(error) => {
                        let response = pandamux_protocol::RpcResponse::error(request.id, error);
                        write_response(&mut writer, &response).await?;
                        return Ok(());
                    }
                }
            }

            if let Some(response) = self
                .router
                .handle_request_with_context(
                    context.as_ref().expect("authenticated context"),
                    request,
                )
                .await
            {
                write_response(&mut writer, &response).await?;
            }
        }
        Ok(())
    }
}

async fn write_response<W>(
    writer: &mut W,
    response: &pandamux_protocol::RpcResponse,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut line = serde_json::to_vec(response)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use std::io;

    use pandamux_core::ThreadId;
    use pandamux_protocol::{PROTOCOL_VERSION, RpcRequest, RpcResponse, ServerRole};
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
    use tokio::task::JoinHandle;

    use super::*;

    fn test_server() -> (TempDir, Arc<Server>, RuntimeInfo) {
        let runtime_dir = tempfile::tempdir().expect("runtime temp dir");
        let server = Arc::new(
            Server::new(ServerConfig {
                role: ServerRole::Hub,
                environment_id: "env-local".to_string(),
                runtime_dir: runtime_dir.path().to_path_buf(),
                db_path: None,
            })
            .expect("server"),
        );
        server
            .register_runtime("test-endpoint", "control-capability")
            .expect("runtime record");
        let runtime = RuntimeInfo::read_from_dir(runtime_dir.path())
            .expect("read runtime")
            .expect("runtime info");
        (runtime_dir, server, runtime)
    }

    fn connection(server: Arc<Server>) -> (BufReader<DuplexStream>, JoinHandle<io::Result<()>>) {
        let (client, accepted) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            server
                .serve_local_connection(
                    accepted,
                    ClientIdentity {
                        description: "test-client".to_string(),
                        process_id: Some(42),
                        user_id: Some(7),
                        executable: Some("test-client".to_string()),
                    },
                )
                .await
        });
        (BufReader::new(client), task)
    }

    async fn exchange(
        connection: &mut BufReader<DuplexStream>,
        request: RpcRequest,
    ) -> RpcResponse {
        let mut request_line = serde_json::to_vec(&request).expect("serialize request");
        request_line.push(b'\n');
        connection
            .get_mut()
            .write_all(&request_line)
            .await
            .expect("write request");
        connection.get_mut().flush().await.expect("flush request");
        let mut response_line = String::new();
        connection
            .read_line(&mut response_line)
            .await
            .expect("read response");
        serde_json::from_str(&response_line).expect("parse response")
    }

    async fn authenticate(
        connection: &mut BufReader<DuplexStream>,
        capability: &str,
        client_kind: &str,
    ) {
        let response = exchange(
            connection,
            RpcRequest::new(
                1,
                "system.hello",
                Some(json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "clientKind": client_kind,
                    "clientVersion": "test",
                    "capability": capability,
                    "peerRole": "internal",
                    "capabilityScope": "internal",
                })),
            ),
        )
        .await;
        assert!(response.is_success(), "authentication failed: {response:?}");
    }

    #[tokio::test]
    async fn local_connection_rejects_requests_before_authenticated_hello() {
        let (_runtime_dir, server, _runtime) = test_server();
        let (mut connection, task) = connection(server);
        let response = exchange(
            &mut connection,
            RpcRequest::new(1, "system.ping", Some(json!({}))),
        )
        .await;
        assert_eq!(response.error.expect("unauthorized error").code, -32001);
        drop(connection);
        task.await
            .expect("connection task")
            .expect("connection result");
    }

    #[tokio::test]
    async fn endpoint_binds_distinct_control_mcp_and_provider_scopes() {
        let (_runtime_dir, server, runtime) = test_server();

        let (mut control, control_task) = connection(server.clone());
        authenticate(&mut control, &runtime.token, "desktop").await;
        assert!(
            exchange(
                &mut control,
                RpcRequest::new(2, "settings.get", Some(json!({}))),
            )
            .await
            .is_success()
        );
        assert_eq!(
            exchange(
                &mut control,
                RpcRequest::new(3, "mcp.list_tools", Some(json!({}))),
            )
            .await
            .error
            .expect("control cannot impersonate MCP")
            .code,
            -32001
        );
        drop(control);
        control_task
            .await
            .expect("control task")
            .expect("control connection");

        let (mut mcp, mcp_task) = connection(server.clone());
        authenticate(&mut mcp, &runtime.mcp_token, "mcp").await;
        assert!(
            exchange(
                &mut mcp,
                RpcRequest::new(2, "mcp.list_tools", Some(json!({}))),
            )
            .await
            .is_success()
        );
        assert_eq!(
            exchange(
                &mut mcp,
                RpcRequest::new(3, "settings.get", Some(json!({}))),
            )
            .await
            .error
            .expect("MCP cannot use control methods")
            .code,
            -32001
        );
        drop(mcp);
        mcp_task.await.expect("MCP task").expect("MCP connection");

        let provider_token = server.capability_authority().issue_provider_child(
            ThreadId::new("thread-allowed"),
            ["notification.post".to_string()],
            60_000,
        );
        let (mut provider, provider_task) = connection(server);
        authenticate(&mut provider, &provider_token, "provider-child").await;
        assert!(
            exchange(
                &mut provider,
                RpcRequest::new(
                    2,
                    "notification.post",
                    Some(json!({
                        "threadId": "thread-allowed",
                        "title": "Allowed",
                        "peerRole": "internal",
                        "capabilityScope": "control",
                    })),
                ),
            )
            .await
            .is_success()
        );
        for method in [
            "settings.get",
            "organization.set",
            "schedule.create",
            "environment.list",
            "bundle.push",
        ] {
            let response = exchange(
                &mut provider,
                RpcRequest::new(
                    3,
                    method,
                    Some(json!({
                        "threadId": "thread-allowed",
                        "peerRole": "internal",
                        "capabilityScope": "control",
                    })),
                ),
            )
            .await;
            assert_eq!(
                response.error.expect("provider method denied").code,
                -32001,
                "unexpected result for {method}"
            );
        }
        drop(provider);
        provider_task
            .await
            .expect("provider task")
            .expect("provider connection");
    }
}
