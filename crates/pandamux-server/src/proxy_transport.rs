use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use pandamux_core::EnvironmentId;
use pandamux_protocol::{
    EventEnvelope, PROTOCOL_VERSION, PROXY_PROTOCOL_VERSION, ProxyMessage, RpcError, RpcId,
    RpcRequest, RpcResponse,
};
use pandamux_remote::{SshConfig, SshConnectionPool, upload_file_sftp};
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::environment_router::{BoxFuture, EnvironmentTransport, EnvironmentTransportState};
use crate::proxy_connection::{read_proxy_message, write_proxy_message};

const OUTBOUND_QUEUE_CAPACITY: usize = 128;
const EVENT_BROADCAST_CAPACITY: usize = 512;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const INSTALL_REQUEST_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

type PendingRequests = Arc<Mutex<HashMap<RpcId, oneshot::Sender<Result<RpcResponse, RpcError>>>>>;

struct TransportTasks {
    writer: JoinHandle<()>,
    reader: JoinHandle<()>,
}

pub struct ProxyEnvironmentTransport {
    environment_id: EnvironmentId,
    outgoing: mpsc::Sender<ProxyMessage>,
    pending: PendingRequests,
    events: broadcast::Sender<EventEnvelope>,
    initial_events: Mutex<Option<broadcast::Receiver<EventEnvelope>>>,
    state: Arc<RwLock<EnvironmentTransportState>>,
    last_event_seq: Arc<AtomicU64>,
    next_request_id: AtomicU64,
    shutdown_tx: watch::Sender<bool>,
    alive: Arc<AtomicBool>,
    tasks: tokio::sync::Mutex<Option<TransportTasks>>,
    sftp: Option<(SshConnectionPool, SshConfig)>,
}

impl std::fmt::Debug for ProxyEnvironmentTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProxyEnvironmentTransport")
            .field("environment_id", &self.environment_id)
            .field("state", &self.state())
            .field("last_event_seq", &self.last_event_seq())
            .finish_non_exhaustive()
    }
}

impl ProxyEnvironmentTransport {
    pub async fn connect<S>(
        stream: S,
        environment_id: EnvironmentId,
        last_event_seq: Arc<AtomicU64>,
    ) -> Result<Arc<Self>, RpcError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Self::connect_with_sftp(stream, environment_id, last_event_seq, None).await
    }

    pub async fn connect_with_sftp<S>(
        stream: S,
        environment_id: EnvironmentId,
        last_event_seq: Arc<AtomicU64>,
        sftp: Option<(SshConnectionPool, SshConfig)>,
    ) -> Result<Arc<Self>, RpcError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (read_half, write_half) = tokio::io::split(stream);
        let (outgoing, mut outgoing_rx) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
        let (events, initial_events) = broadcast::channel(EVENT_BROADCAST_CAPACITY);
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(RwLock::new(EnvironmentTransportState::Stale {
            message: "proxy handshake is pending".to_string(),
        }));
        let alive = Arc::new(AtomicBool::new(true));
        let (shutdown_tx, mut writer_shutdown_rx) = watch::channel(false);
        let mut reader_shutdown_rx = shutdown_tx.subscribe();
        let (handshake_tx, handshake_rx) = oneshot::channel();
        let handshake_tx = Arc::new(Mutex::new(Some(handshake_tx)));

        let writer_state = state.clone();
        let writer_pending = pending.clone();
        let writer_alive = alive.clone();
        let writer_shutdown_tx = shutdown_tx.clone();
        let writer = tokio::spawn(async move {
            let mut writer = write_half;
            let mut normal_goodbye = false;
            loop {
                tokio::select! {
                    changed = writer_shutdown_rx.changed() => {
                        if changed.is_err() || *writer_shutdown_rx.borrow() {
                            break;
                        }
                    }
                    message = outgoing_rx.recv() => {
                        let Some(message) = message else {
                            break;
                        };
                        let goodbye = matches!(message, ProxyMessage::Goodbye { .. });
                        if let Err(error) = write_proxy_message(&mut writer, &message).await {
                            set_state(
                                &writer_state,
                                EnvironmentTransportState::Stale {
                                    message: format!("proxy write failed: {error}"),
                                },
                            );
                            fail_pending(
                                &writer_pending,
                                proxy_error("proxy_disconnected", format!("proxy write failed: {error}")),
                            );
                            break;
                        }
                        if goodbye {
                            let _ = tokio::io::AsyncWriteExt::shutdown(&mut writer).await;
                            normal_goodbye = true;
                            break;
                        }
                    }
                }
            }
            if !normal_goodbye {
                writer_alive.store(false, Ordering::SeqCst);
                let _ = writer_shutdown_tx.send(true);
            }
        });

        let reader_state = state.clone();
        let reader_pending = pending.clone();
        let reader_events = events.clone();
        let reader_last_event_seq = last_event_seq.clone();
        let reader_environment_id = environment_id.clone();
        let reader_alive = alive.clone();
        let reader_shutdown_tx = shutdown_tx.clone();
        let reader = tokio::spawn(async move {
            let mut reader = BufReader::new(read_half);
            loop {
                let message = tokio::select! {
                    changed = reader_shutdown_rx.changed() => {
                        if changed.is_err() || *reader_shutdown_rx.borrow() {
                            break;
                        }
                        continue;
                    }
                    message = read_proxy_message(&mut reader) => message,
                };

                match message {
                    Ok(Some(ProxyMessage::HelloAck {
                        proxy_protocol_version,
                        rpc_protocol_version,
                        server_version,
                        environment_id,
                        current_event_seq,
                    })) => {
                        if let Some(sender) = handshake_tx.lock().expect("handshake lock").take() {
                            let _ = sender.send(Ok(HandshakeAck {
                                proxy_protocol_version,
                                rpc_protocol_version,
                                server_version,
                                environment_id,
                                current_event_seq,
                            }));
                        }
                    }
                    Ok(Some(ProxyMessage::Response { response })) => {
                        if let Some(id) = response.id.clone()
                            && let Some(sender) = reader_pending
                                .lock()
                                .expect("pending request lock")
                                .remove(&id)
                        {
                            let _ = sender.send(Ok(response));
                        }
                    }
                    Ok(Some(ProxyMessage::Event {
                        event_seq,
                        mut envelope,
                    })) => {
                        reader_last_event_seq.fetch_max(event_seq, Ordering::SeqCst);
                        envelope.environment_id = reader_environment_id.clone();
                        let _ = reader_events.send(envelope);
                    }
                    Ok(Some(ProxyMessage::Error { code, message })) => {
                        let error = proxy_error(&code, message.clone());
                        if let Some(sender) = handshake_tx.lock().expect("handshake lock").take() {
                            let _ = sender.send(Err(error));
                        } else {
                            set_state(&reader_state, EnvironmentTransportState::Stale { message });
                        }
                    }
                    Ok(Some(ProxyMessage::Goodbye { reason })) => {
                        set_state(
                            &reader_state,
                            EnvironmentTransportState::Stale { message: reason },
                        );
                        break;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        set_state(
                            &reader_state,
                            EnvironmentTransportState::Stale {
                                message: "proxy reached EOF".to_string(),
                            },
                        );
                        break;
                    }
                    Err(error) => {
                        set_state(
                            &reader_state,
                            EnvironmentTransportState::Stale {
                                message: format!("proxy read failed: {error}"),
                            },
                        );
                        break;
                    }
                }
            }

            reader_alive.store(false, Ordering::SeqCst);
            fail_pending(
                &reader_pending,
                proxy_error("proxy_disconnected", "proxy connection is stale"),
            );
            if let Some(sender) = handshake_tx.lock().expect("handshake lock").take() {
                let _ = sender.send(Err(proxy_error(
                    "proxy_handshake_eof",
                    "proxy closed before hello acknowledgement",
                )));
            }
            let _ = reader_shutdown_tx.send(true);
        });

        let transport = Arc::new(Self {
            environment_id: environment_id.clone(),
            outgoing,
            pending,
            events,
            initial_events: Mutex::new(Some(initial_events)),
            state,
            last_event_seq,
            next_request_id: AtomicU64::new(1),
            shutdown_tx,
            alive,
            tasks: tokio::sync::Mutex::new(Some(TransportTasks { writer, reader })),
            sftp,
        });

        transport
            .outgoing
            .send(ProxyMessage::Hello {
                proxy_protocol_version: PROXY_PROTOCOL_VERSION,
                rpc_protocol_version: PROTOCOL_VERSION,
                client_version: env!("CARGO_PKG_VERSION").to_string(),
                environment_id: environment_id.clone(),
                resume_after_event_seq: transport.last_event_seq.load(Ordering::SeqCst),
            })
            .await
            .map_err(|_| proxy_error("proxy_handshake_send_failed", "proxy writer stopped"))?;

        let ack = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake_rx)
            .await
            .map_err(|_| proxy_error("proxy_handshake_timeout", "proxy hello timed out"))?
            .map_err(|_| proxy_error("proxy_handshake_closed", "proxy hello channel closed"))??;

        if ack.proxy_protocol_version != PROXY_PROTOCOL_VERSION {
            let error = proxy_error(
                "proxy_protocol_mismatch",
                format!(
                    "proxy protocol mismatch: Hub supports {}, node reported {}",
                    PROXY_PROTOCOL_VERSION, ack.proxy_protocol_version
                ),
            );
            transport.shutdown().await;
            return Err(error);
        }
        if ack.rpc_protocol_version != PROTOCOL_VERSION {
            let error = proxy_error(
                "rpc_protocol_mismatch",
                format!(
                    "RPC protocol mismatch: Hub supports {}, node reported {}",
                    PROTOCOL_VERSION, ack.rpc_protocol_version
                ),
            );
            transport.shutdown().await;
            return Err(error);
        }
        if ack.environment_id != environment_id {
            let error = proxy_error(
                "proxy_environment_mismatch",
                format!(
                    "proxy expected environment '{}', node identified as '{}'",
                    environment_id.as_str(),
                    ack.environment_id.as_str()
                ),
            );
            transport.shutdown().await;
            return Err(error);
        }
        let _ = (&ack.server_version, ack.current_event_seq);
        if !transport.alive.load(Ordering::SeqCst) {
            transport.shutdown().await;
            return Err(proxy_error(
                "proxy_handshake_eof",
                "proxy closed immediately after hello acknowledgement",
            ));
        }
        set_state(&transport.state, EnvironmentTransportState::Connected);
        Ok(transport)
    }

    pub async fn shutdown(&self) {
        if self.state() == EnvironmentTransportState::Closed {
            return;
        }
        let _ = self
            .outgoing
            .send(ProxyMessage::Goodbye {
                reason: "hub transport shutdown".to_string(),
            })
            .await;

        let tasks = self.tasks.lock().await.take();
        if let Some(mut tasks) = tasks {
            if tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut tasks.writer)
                .await
                .is_err()
            {
                tasks.writer.abort();
                let _ = tasks.writer.await;
            }
            let _ = self.shutdown_tx.send(true);
            if tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut tasks.reader)
                .await
                .is_err()
            {
                tasks.reader.abort();
                let _ = tasks.reader.await;
            }
        } else {
            let _ = self.shutdown_tx.send(true);
        }
        self.alive.store(false, Ordering::SeqCst);
        fail_pending(
            &self.pending,
            proxy_error("proxy_closed", "proxy transport was closed"),
        );
        set_state(&self.state, EnvironmentTransportState::Closed);
    }
}

impl EnvironmentTransport for ProxyEnvironmentTransport {
    fn forward_request<'a>(
        &'a self,
        request: &'a RpcRequest,
    ) -> BoxFuture<'a, Result<RpcResponse, RpcError>> {
        Box::pin(async move {
            if self.state() != EnvironmentTransportState::Connected {
                return Err(proxy_error(
                    "proxy_stale",
                    format!(
                        "environment '{}' proxy is not connected",
                        self.environment_id.as_str()
                    ),
                ));
            }

            if request.is_notification() {
                self.outgoing
                    .send(ProxyMessage::Request {
                        request: request.clone(),
                    })
                    .await
                    .map_err(|_| proxy_error("proxy_queue_closed", "proxy queue is closed"))?;
                return Ok(RpcResponse {
                    jsonrpc: pandamux_protocol::JSONRPC_VERSION.to_string(),
                    id: None,
                    result: Some(serde_json::Value::Null),
                    error: None,
                });
            }

            let original_id = request.id.clone();
            let proxy_id = RpcId::String(format!(
                "proxy-{}",
                self.next_request_id.fetch_add(1, Ordering::SeqCst)
            ));
            let mut forwarded = request.clone();
            forwarded.id = Some(proxy_id.clone());
            let (response_tx, response_rx) = oneshot::channel();
            self.pending
                .lock()
                .expect("pending request lock")
                .insert(proxy_id.clone(), response_tx);
            let mut guard =
                PendingRequestGuard::new(proxy_id, self.pending.clone(), self.outgoing.clone());
            self.outgoing
                .send(ProxyMessage::Request { request: forwarded })
                .await
                .map_err(|_| proxy_error("proxy_queue_closed", "proxy queue is closed"))?;

            let request_timeout = if request.method == "antigravity.install" {
                INSTALL_REQUEST_TIMEOUT
            } else {
                REQUEST_TIMEOUT
            };
            let mut response = tokio::time::timeout(request_timeout, response_rx)
                .await
                .map_err(|_| proxy_error("proxy_request_timeout", "remote request timed out"))?
                .map_err(|_| proxy_error("proxy_response_closed", "response channel closed"))??;
            guard.complete();
            response.id = original_id;
            Ok(response)
        })
    }

    fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.initial_events
            .lock()
            .expect("initial event receiver lock")
            .take()
            .unwrap_or_else(|| self.events.subscribe())
    }

    fn state(&self) -> EnvironmentTransportState {
        self.state.read().expect("transport state lock").clone()
    }

    fn last_event_seq(&self) -> u64 {
        self.last_event_seq.load(Ordering::SeqCst)
    }

    fn upload_file<'a>(
        &'a self,
        local_path: &'a Path,
        remote_path: &'a str,
    ) -> BoxFuture<'a, Result<(), RpcError>> {
        Box::pin(async move {
            if self.state() != EnvironmentTransportState::Connected {
                return Err(proxy_error(
                    "proxy_stale",
                    format!(
                        "environment '{}' proxy is not connected",
                        self.environment_id.as_str()
                    ),
                ));
            }
            let (pool, config) = self.sftp.as_ref().ok_or_else(|| {
                proxy_error(
                    "proxy_upload_unsupported",
                    "proxy transport has no authenticated SFTP side channel",
                )
            })?;
            upload_file_sftp(pool, config.clone(), local_path, remote_path)
                .await
                .map_err(|error| {
                    proxy_error(
                        "proxy_upload_failed",
                        format!("SFTP upload to {remote_path} failed: {error}"),
                    )
                })
        })
    }

    fn shutdown<'a>(&'a self) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            ProxyEnvironmentTransport::shutdown(self).await;
        })
    }
}

impl Drop for ProxyEnvironmentTransport {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Ok(mut tasks) = self.tasks.try_lock()
            && let Some(tasks) = tasks.take()
        {
            tasks.writer.abort();
            tasks.reader.abort();
        }
    }
}

struct PendingRequestGuard {
    id: Option<RpcId>,
    pending: PendingRequests,
    outgoing: mpsc::Sender<ProxyMessage>,
}

impl PendingRequestGuard {
    fn new(id: RpcId, pending: PendingRequests, outgoing: mpsc::Sender<ProxyMessage>) -> Self {
        Self {
            id: Some(id),
            pending,
            outgoing,
        }
    }

    fn complete(&mut self) {
        self.id = None;
    }
}

impl Drop for PendingRequestGuard {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.pending
                .lock()
                .expect("pending request lock")
                .remove(&id);
            let _ = self.outgoing.try_send(ProxyMessage::Cancel { id });
        }
    }
}

struct HandshakeAck {
    proxy_protocol_version: u32,
    rpc_protocol_version: u32,
    server_version: String,
    environment_id: EnvironmentId,
    current_event_seq: u64,
}

fn set_state(state: &Arc<RwLock<EnvironmentTransportState>>, value: EnvironmentTransportState) {
    *state.write().expect("transport state lock") = value;
}

fn fail_pending(pending: &PendingRequests, error: RpcError) {
    let senders = pending
        .lock()
        .expect("pending request lock")
        .drain()
        .map(|(_, sender)| sender)
        .collect::<Vec<_>>();
    for sender in senders {
        let _ = sender.send(Err(error.clone()));
    }
}

fn proxy_error(code: &str, message: impl Into<String>) -> RpcError {
    RpcError::with_data(-32010, message, serde_json::json!({ "proxyCode": code }))
}
