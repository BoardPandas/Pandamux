//! Hub routing and event relaying across local and remote environments.
//!
//! Provides the [`EnvironmentRouter`] and [`EnvironmentTransport`] abstractions
//! that allow the Hub to forward environment-scoped RPC requests to remote node
//! daemons and relay event streams back into the Hub's broadcast bus.

use pandamux_core::ids::EnvironmentId;
use pandamux_protocol::{EventEnvelope, RpcError, RpcRequest, RpcResponse};
use std::collections::HashMap;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{RwLock, broadcast, watch};
use tokio::task::JoinHandle;

pub type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvironmentTransportState {
    Connected,
    Stale { message: String },
    Unreachable { message: String },
    Closed,
}

/// Transport contract for communicating with a remote environment daemon.
pub trait EnvironmentTransport: Send + Sync {
    /// Forwards an RPC request across the environment transport and awaits the response.
    fn forward_request<'a>(
        &'a self,
        request: &'a RpcRequest,
    ) -> BoxFuture<'a, Result<RpcResponse, RpcError>>;

    /// Subscribes to event envelopes emitted by the remote node.
    fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope>;

    /// Current transport state. A stale or unreachable transport never falls back local.
    fn state(&self) -> EnvironmentTransportState;

    /// Last durable node event sequence observed by this transport.
    fn last_event_seq(&self) -> u64 {
        0
    }

    /// Streams one private file over the transport's authenticated side channel.
    fn upload_file<'a>(
        &'a self,
        _local_path: &'a Path,
        _remote_path: &'a str,
    ) -> BoxFuture<'a, Result<(), RpcError>> {
        Box::pin(async {
            Err(RpcError::with_data(
                -32010,
                "Environment transport does not support file upload",
                serde_json::json!({ "proxyCode": "proxy_upload_unsupported" }),
            ))
        })
    }

    /// Closes owned IO tasks and waits for their cleanup.
    fn shutdown<'a>(&'a self) -> BoxFuture<'a, ()>;
}

struct TransportRegistration {
    transport: Arc<dyn EnvironmentTransport>,
    relay_shutdown: watch::Sender<bool>,
    relay_task: JoinHandle<()>,
}

/// Orchestrates request routing and event relaying between local hub and remote environments.
#[derive(Clone)]
pub struct EnvironmentRouter {
    local_environment_id: EnvironmentId,
    transports: Arc<RwLock<HashMap<EnvironmentId, TransportRegistration>>>,
}

impl EnvironmentRouter {
    /// Creates a new router anchored at the local environment.
    pub fn new(local_environment_id: EnvironmentId) -> Self {
        Self {
            local_environment_id,
            transports: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// The local environment identifier.
    pub fn local_environment_id(&self) -> &EnvironmentId {
        &self.local_environment_id
    }

    /// Checks if a target environment represents the local host.
    pub fn is_local(&self, env_id: &Option<EnvironmentId>) -> bool {
        match env_id {
            None => true,
            Some(id) => id == &self.local_environment_id,
        }
    }

    /// Registers an active transport for a remote environment and optional event relay.
    pub async fn register_transport(
        &self,
        env_id: EnvironmentId,
        transport: Arc<dyn EnvironmentTransport>,
        hub_broadcaster: Option<broadcast::Sender<EventEnvelope>>,
    ) {
        let previous = {
            let mut transports = self.transports.write().await;
            transports.remove(&env_id)
        };
        if let Some(previous) = previous {
            stop_registration(previous).await;
        }

        let (relay_shutdown, mut relay_shutdown_rx) = watch::channel(false);
        let mut events = transport.subscribe_events();
        let relay_environment_id = env_id.clone();
        let relay_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = relay_shutdown_rx.changed() => {
                        if changed.is_err() || *relay_shutdown_rx.borrow() {
                            break;
                        }
                    }
                    event = events.recv() => {
                        match event {
                            Ok(mut envelope) => {
                                envelope.environment_id = relay_environment_id.clone();
                                if let Some(broadcaster) = &hub_broadcaster {
                                    let _ = broadcaster.send(envelope);
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        });

        self.transports.write().await.insert(
            env_id,
            TransportRegistration {
                transport,
                relay_shutdown,
                relay_task,
            },
        );
    }

    /// Unregisters an environment transport when disconnected.
    pub async fn unregister_transport(&self, env_id: &EnvironmentId) {
        let registration = {
            let mut transports = self.transports.write().await;
            transports.remove(env_id)
        };
        if let Some(registration) = registration {
            stop_registration(registration).await;
        }
    }

    /// Checks if a remote environment has an active connected transport.
    pub async fn is_connected(&self, env_id: &EnvironmentId) -> bool {
        let map = self.transports.read().await;
        map.get(env_id)
            .is_some_and(|entry| entry.transport.state() == EnvironmentTransportState::Connected)
    }

    /// Returns a stable snapshot of currently connected remote environment ids.
    pub async fn connected_environment_ids(&self) -> Vec<EnvironmentId> {
        let map = self.transports.read().await;
        let mut ids = map
            .iter()
            .filter(|(_, entry)| entry.transport.state() == EnvironmentTransportState::Connected)
            .map(|(environment_id, _)| environment_id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    /// Returns a stable snapshot of every registered remote environment.
    pub async fn registered_environment_ids(&self) -> Vec<EnvironmentId> {
        let map = self.transports.read().await;
        let mut ids = map.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub async fn transport_state(&self, env_id: &EnvironmentId) -> EnvironmentTransportState {
        let map = self.transports.read().await;
        map.get(env_id).map_or_else(
            || EnvironmentTransportState::Unreachable {
                message: format!(
                    "Environment '{}' has no registered transport",
                    env_id.as_str()
                ),
            },
            |entry| entry.transport.state(),
        )
    }

    pub async fn last_event_seq(&self, env_id: &EnvironmentId) -> Option<u64> {
        let map = self.transports.read().await;
        map.get(env_id)
            .map(|entry| entry.transport.last_event_seq())
    }

    pub async fn shutdown_all(&self) {
        let registrations = {
            let mut map = self.transports.write().await;
            map.drain().map(|(_, entry)| entry).collect::<Vec<_>>()
        };
        for registration in registrations {
            stop_registration(registration).await;
        }
    }

    /// Forwards an RPC request to the specified remote environment.
    pub async fn forward_request(
        &self,
        env_id: &EnvironmentId,
        request: &RpcRequest,
    ) -> Result<RpcResponse, RpcError> {
        let transport = {
            let map = self.transports.read().await;
            map.get(env_id).map(|entry| entry.transport.clone())
        };

        match transport {
            Some(transport) if transport.state() == EnvironmentTransportState::Connected => {
                transport.forward_request(request).await
            }
            Some(transport) => Err(transport_error(env_id, transport.state())),
            None => Err(transport_error(
                env_id,
                EnvironmentTransportState::Unreachable {
                    message: "no registered transport".to_string(),
                },
            )),
        }
    }

    pub async fn upload_file(
        &self,
        env_id: &EnvironmentId,
        local_path: &Path,
        remote_path: &str,
    ) -> Result<(), RpcError> {
        let transport = {
            let map = self.transports.read().await;
            map.get(env_id).map(|entry| entry.transport.clone())
        };
        match transport {
            Some(transport) if transport.state() == EnvironmentTransportState::Connected => {
                transport.upload_file(local_path, remote_path).await
            }
            Some(transport) => Err(transport_error(env_id, transport.state())),
            None => Err(transport_error(
                env_id,
                EnvironmentTransportState::Unreachable {
                    message: "no registered transport".to_string(),
                },
            )),
        }
    }
}

async fn stop_registration(registration: TransportRegistration) {
    let _ = registration.relay_shutdown.send(true);
    let _ = registration.relay_task.await;
    registration.transport.shutdown().await;
}

fn transport_error(env_id: &EnvironmentId, state: EnvironmentTransportState) -> RpcError {
    let (state_name, detail) = match state {
        EnvironmentTransportState::Connected => ("connected", "transport unavailable".to_string()),
        EnvironmentTransportState::Stale { message } => ("stale", message),
        EnvironmentTransportState::Unreachable { message } => ("unreachable", message),
        EnvironmentTransportState::Closed => ("closed", "transport is closed".to_string()),
    };
    RpcError::with_data(
        -32010,
        format!(
            "Environment '{}' is {state_name}: {detail}",
            env_id.as_str()
        ),
        serde_json::json!({
            "environmentId": env_id,
            "state": state_name,
        }),
    )
}

/// Simulated mock transport for deterministic tests of routing and relaying.
type MockResponder = Arc<dyn Fn(&RpcRequest) -> Result<RpcResponse, RpcError> + Send + Sync>;
type MockUploader = Arc<dyn Fn(&Path, &str) -> Result<(), RpcError> + Send + Sync>;

pub struct MockEnvironmentTransport {
    responder: MockResponder,
    broadcaster: broadcast::Sender<EventEnvelope>,
    connected: Arc<AtomicBool>,
    uploader: Option<MockUploader>,
}

impl MockEnvironmentTransport {
    pub fn new(
        responder: impl Fn(&RpcRequest) -> Result<RpcResponse, RpcError> + Send + Sync + 'static,
    ) -> Self {
        let (tx, _) = broadcast::channel(128);
        Self {
            responder: Arc::new(responder),
            broadcaster: tx,
            connected: Arc::new(AtomicBool::new(true)),
            uploader: None,
        }
    }

    pub fn with_uploader(
        mut self,
        uploader: impl Fn(&Path, &str) -> Result<(), RpcError> + Send + Sync + 'static,
    ) -> Self {
        self.uploader = Some(Arc::new(uploader));
        self
    }

    /// Emits a mock event on the transport.
    pub fn emit_event(&self, envelope: EventEnvelope) {
        let _ = self.broadcaster.send(envelope);
    }
}

impl EnvironmentTransport for MockEnvironmentTransport {
    fn forward_request<'a>(
        &'a self,
        request: &'a RpcRequest,
    ) -> BoxFuture<'a, Result<RpcResponse, RpcError>> {
        let res = (self.responder)(request);
        Box::pin(async move { res })
    }

    fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.broadcaster.subscribe()
    }

    fn state(&self) -> EnvironmentTransportState {
        if self.connected.load(Ordering::SeqCst) {
            EnvironmentTransportState::Connected
        } else {
            EnvironmentTransportState::Closed
        }
    }

    fn upload_file<'a>(
        &'a self,
        local_path: &'a Path,
        remote_path: &'a str,
    ) -> BoxFuture<'a, Result<(), RpcError>> {
        let result = self.uploader.as_ref().map_or_else(
            || {
                Err(RpcError::with_data(
                    -32010,
                    "Mock transport has no uploader",
                    serde_json::json!({ "proxyCode": "proxy_upload_unsupported" }),
                ))
            },
            |uploader| uploader(local_path, remote_path),
        );
        Box::pin(async move { result })
    }

    fn shutdown<'a>(&'a self) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.connected.store(false, Ordering::SeqCst);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::ThreadEventKind;
    use pandamux_core::ids::ThreadId;
    use serde_json::json;

    #[tokio::test]
    async fn test_environment_router_local_and_remote() {
        let local_id = EnvironmentId::new("env_local");
        let remote_id = EnvironmentId::new("env_remote");
        let router = EnvironmentRouter::new(local_id.clone());

        assert!(router.is_local(&None));
        assert!(router.is_local(&Some(local_id.clone())));
        assert!(!router.is_local(&Some(remote_id.clone())));

        // Remote request fails when not connected
        let req = RpcRequest::new(1, "system.ping", None);
        let err = router.forward_request(&remote_id, &req).await.unwrap_err();
        assert!(err.message.contains("unreachable"));

        // Register mock remote transport
        let mock_transport = Arc::new(MockEnvironmentTransport::new(|req| {
            let id = req.id.clone().unwrap_or(1.into());
            Ok(RpcResponse::success(id, json!({ "remotePong": true })))
        }));

        let (hub_tx, mut hub_rx) = broadcast::channel(16);
        router
            .register_transport(remote_id.clone(), mock_transport.clone(), Some(hub_tx))
            .await;

        assert!(router.is_connected(&remote_id).await);

        // Remote request succeeds through forward
        let resp = router.forward_request(&remote_id, &req).await.unwrap();
        assert_eq!(resp.result.unwrap(), json!({ "remotePong": true }));

        // Event relay works
        let envelope = EventEnvelope {
            subscription_id: "sub-1".to_string(),
            environment_id: EnvironmentId::new("original-node"),
            thread_id: Some(ThreadId::new("th-1")),
            run_id: None,
            seq: 42,
            at_ms: 1000,
            kind: ThreadEventKind::TurnCompleted {
                outcome: pandamux_core::event::TurnOutcome::Success,
            },
        };

        mock_transport.emit_event(envelope);
        let relayed = hub_rx.recv().await.unwrap();
        assert_eq!(relayed.environment_id, remote_id);
        assert_eq!(relayed.seq, 42);

        // Unregister
        router.unregister_transport(&remote_id).await;
        assert!(!router.is_connected(&remote_id).await);
        assert_eq!(mock_transport.state(), EnvironmentTransportState::Closed);
    }
}
