use pandamux_core::EnvironmentId;
use serde::{Deserialize, Serialize};

use crate::{EventEnvelope, RpcId, RpcRequest, RpcResponse};

/// Version of the NDJSON envelope spoken over an SSH proxy channel.
pub const PROXY_PROTOCOL_VERSION: u32 = 2;

/// A single bounded NDJSON frame exchanged by the Hub and a node proxy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProxyMessage {
    Hello {
        proxy_protocol_version: u32,
        rpc_protocol_version: u32,
        client_version: String,
        environment_id: EnvironmentId,
        #[serde(default)]
        resume_after_event_seq: u64,
    },
    HelloAck {
        proxy_protocol_version: u32,
        rpc_protocol_version: u32,
        server_version: String,
        environment_id: EnvironmentId,
        current_event_seq: u64,
    },
    Request {
        request: RpcRequest,
    },
    Response {
        response: RpcResponse,
    },
    Cancel {
        id: RpcId,
    },
    Keepalive {
        sequence: u64,
    },
    KeepaliveAck {
        sequence: u64,
    },
    Event {
        event_seq: u64,
        envelope: EventEnvelope,
    },
    Goodbye {
        reason: String,
    },
    Error {
        code: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{ThreadEventKind, ThreadId, event::TurnOutcome};
    use serde_json::json;

    #[test]
    fn proxy_messages_are_stable_tagged_ndjson_values() {
        let hello = ProxyMessage::Hello {
            proxy_protocol_version: PROXY_PROTOCOL_VERSION,
            rpc_protocol_version: crate::PROTOCOL_VERSION,
            client_version: "0.53.44".to_string(),
            environment_id: EnvironmentId::from("env-remote"),
            resume_after_event_seq: 41,
        };
        let json = serde_json::to_string(&hello).expect("serialize proxy hello");
        assert!(json.contains("\"type\":\"hello\""));
        assert!(json.contains("\"resume_after_event_seq\":41"));
        assert_eq!(
            serde_json::from_str::<ProxyMessage>(&json).expect("parse proxy hello"),
            hello
        );

        let response = ProxyMessage::Response {
            response: RpcResponse::success("proxy-7", json!({ "pong": true })),
        };
        assert_eq!(
            serde_json::from_str::<ProxyMessage>(
                &serde_json::to_string(&response).expect("serialize response")
            )
            .expect("parse response"),
            response
        );

        let keepalive = ProxyMessage::Keepalive { sequence: 7 };
        assert_eq!(
            serde_json::from_str::<ProxyMessage>(
                &serde_json::to_string(&keepalive).expect("serialize keepalive")
            )
            .expect("parse keepalive"),
            keepalive
        );

        let event = ProxyMessage::Event {
            event_seq: 42,
            envelope: EventEnvelope {
                subscription_id: "node-proxy".to_string(),
                environment_id: EnvironmentId::from("env-remote"),
                thread_id: Some(ThreadId::from("thread-1")),
                run_id: None,
                seq: 8,
                at_ms: 100,
                kind: ThreadEventKind::TurnCompleted {
                    outcome: TurnOutcome::Success,
                },
            },
        };
        assert_eq!(
            serde_json::from_str::<ProxyMessage>(
                &serde_json::to_string(&event).expect("serialize event")
            )
            .expect("parse event"),
            event
        );
    }
}
