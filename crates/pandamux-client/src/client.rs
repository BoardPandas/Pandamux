use crate::projections::ThreadProjection;
use crate::transport::{MockTransport, TransportError};
use pandamux_core::{Thread, ThreadEvent, ThreadId};
use pandamux_protocol::{
    EventEnvelope, HelloParams, PROTOCOL_VERSION, RpcId, RpcRequest, SubscribeParams,
};
use std::collections::HashMap;

/// Client interface managing transport, request routing, and live projections.
pub struct PandamuxClient {
    transport: MockTransport,
    next_id: i64,
    thread_projections: HashMap<ThreadId, ThreadProjection>,
}

impl PandamuxClient {
    pub fn new(transport: MockTransport) -> Self {
        Self {
            transport,
            next_id: 1,
            thread_projections: HashMap::new(),
        }
    }

    fn next_id(&mut self) -> RpcId {
        let id = self.next_id;
        self.next_id += 1;
        RpcId::Number(id)
    }

    /// Perform initial protocol handshake.
    pub async fn hello(
        &mut self,
        client_kind: &str,
        client_version: &str,
    ) -> Result<HelloParams, TransportError> {
        let params = HelloParams {
            protocol_version: PROTOCOL_VERSION,
            client_kind: client_kind.to_string(),
            client_version: client_version.to_string(),
        };
        let req = RpcRequest::new(
            self.next_id(),
            "system.hello",
            Some(serde_json::to_value(&params)?),
        );
        self.transport.send_request(&req).await?;
        Ok(params)
    }

    /// Register or track a thread projection.
    pub fn track_thread(&mut self, thread: Thread) {
        let id = thread.id.clone();
        self.thread_projections
            .insert(id, ThreadProjection::new(thread));
    }

    pub fn get_thread_projection(&self, id: &ThreadId) -> Option<&ThreadProjection> {
        self.thread_projections.get(id)
    }

    pub fn get_thread_projection_mut(&mut self, id: &ThreadId) -> Option<&mut ThreadProjection> {
        self.thread_projections.get_mut(id)
    }

    /// Ingest an incoming EventEnvelope, routing to relevant projections.
    pub fn ingest_envelope(&mut self, envelope: &EventEnvelope) {
        if let Some(thread_id) = &envelope.thread_id
            && let Some(proj) = self.thread_projections.get_mut(thread_id)
        {
            let event = ThreadEvent {
                thread_id: thread_id.clone(),
                seq: envelope.seq,
                at_ms: envelope.at_ms,
                kind: envelope.kind.clone(),
            };
            proj.apply_event(&event);
        }
    }

    /// Prepare a subscribe request with sinceSeq for seamless reconnect replay.
    pub async fn subscribe_thread(
        &mut self,
        thread_id: &ThreadId,
        since_seq: Option<u64>,
    ) -> Result<RpcRequest, TransportError> {
        let params = SubscribeParams {
            topic: "thread.events".to_string(),
            environment_id: None,
            thread_id: Some(thread_id.clone()),
            run_id: None,
            since_seq,
        };
        let req = RpcRequest::new(
            self.next_id(),
            "system.subscribe",
            Some(serde_json::to_value(&params)?),
        );
        self.transport.send_request(&req).await?;
        Ok(req)
    }
}
