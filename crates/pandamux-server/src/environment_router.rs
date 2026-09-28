//! Hub routing and event relaying across local and remote environments.
//!
//! Provides the [`EnvironmentRouter`] and [`EnvironmentTransport`] abstractions
//! that allow the Hub to forward environment-scoped RPC requests to remote node
//! daemons and relay event streams back into the Hub's broadcast bus.

use pandamux_core::ids::EnvironmentId;
use pandamux_protocol::{EventEnvelope, RpcError, RpcRequest, RpcResponse};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

pub type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Transport contract for communicating with a remote environment daemon.
pub trait EnvironmentTransport: Send + Sync {
    /// Forwards an RPC request across the environment transport and awaits the response.
    fn forward_request<'a>(
        &'a self,
        request: &'a RpcRequest,
    ) -> BoxFuture<'a, Result<RpcResponse, RpcError>>;

    /// Subscribes to event envelopes emitted by the remote node.
    fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope>;
}

/// Orchestrates request routing and event relaying between local hub and remote environments.
#[derive(Clone)]
pub struct EnvironmentRouter {
    local_environment_id: EnvironmentId,
    transports: Arc<RwLock<HashMap<EnvironmentId, Arc<dyn EnvironmentTransport>>>>,
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
        let mut map = self.transports.write().await;
        map.insert(env_id.clone(), transport.clone());

        if let Some(broadcaster) = hub_broadcaster {
            let mut rx = transport.subscribe_events();
            let env_id_clone = env_id.clone();
            tokio::spawn(async move {
                while let Ok(mut envelope) = rx.recv().await {
                    envelope.environment_id = env_id_clone.clone();
                    let _ = broadcaster.send(envelope);
                }
            });
        }
    }

    /// Unregisters an environment transport when disconnected.
    pub async fn unregister_transport(&self, env_id: &EnvironmentId) {
        let mut map = self.transports.write().await;
        map.remove(env_id);
    }

    /// Checks if a remote environment has an active connected transport.
    pub async fn is_connected(&self, env_id: &EnvironmentId) -> bool {
        let map = self.transports.read().await;
        map.contains_key(env_id)
    }

    /// Forwards an RPC request to the specified remote environment.
    pub async fn forward_request(
        &self,
        env_id: &EnvironmentId,
        request: &RpcRequest,
    ) -> Result<RpcResponse, RpcError> {
        let transport = {
            let map = self.transports.read().await;
            map.get(env_id).cloned()
        };

        match transport {
            Some(t) => t.forward_request(request).await,
            None => Err(RpcError::invalid_request(format!(
                "Environment '{}' is not connected or unreachable",
                env_id.as_str()
            ))),
        }
    }
}

/// Simulated mock transport for deterministic tests of routing and relaying.
pub struct MockEnvironmentTransport {
    #[allow(clippy::type_complexity)]
    responder: Arc<dyn Fn(&RpcRequest) -> Result<RpcResponse, RpcError> + Send + Sync>,
    broadcaster: broadcast::Sender<EventEnvelope>,
}

impl MockEnvironmentTransport {
    pub fn new(
        responder: impl Fn(&RpcRequest) -> Result<RpcResponse, RpcError> + Send + Sync + 'static,
    ) -> Self {
        let (tx, _) = broadcast::channel(128);
        Self {
            responder: Arc::new(responder),
            broadcaster: tx,
        }
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
    }
}
