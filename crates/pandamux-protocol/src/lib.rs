pub mod jsonrpc;
pub mod mcp;
pub mod subscription;
pub mod system;
pub mod thread_rpc;

pub use jsonrpc::{JSONRPC_VERSION, RpcError, RpcId, RpcRequest, RpcResponse};
pub use mcp::{McpCallToolParams, McpCallToolResult, McpContentItem, McpToolDefinition};
pub use subscription::{EventEnvelope, SubscribeParams, SubscribeResult, UnsubscribeParams};
pub use system::{
    HelloParams, HelloResult, IdentifyResult, PROTOCOL_VERSION, PingResult, ServerCapabilities,
    ServerRole,
};
pub use thread_rpc::{
    ThreadCancelTurnParams, ThreadCreateParams, ThreadGetParams, ThreadGetResult, ThreadListParams,
    ThreadRespondApprovalParams, ThreadResumeParams, ThreadResumeResult, ThreadSendTurnParams,
    ThreadSendTurnResult,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rpc_request_and_response_round_trip() {
        let req = RpcRequest::new(1, "system.ping", Some(json!({})));
        let json_str = serde_json::to_string(&req).expect("serialize req");
        let parsed_req: RpcRequest = serde_json::from_str(&json_str).expect("deserialize req");
        assert_eq!(parsed_req.method, "system.ping");
        assert_eq!(parsed_req.id, Some(RpcId::Number(1)));

        let res = RpcResponse::success(1, json!({ "pong": true }));
        let res_str = serde_json::to_string(&res).expect("serialize res");
        let parsed_res: RpcResponse = serde_json::from_str(&res_str).expect("deserialize res");
        assert!(parsed_res.is_success());
        assert_eq!(parsed_res.result.unwrap()["pong"], true);
    }

    #[test]
    fn rpc_error_round_trip() {
        let err = RpcError::method_not_found("unknown.method");
        let res = RpcResponse::error(Some(RpcId::String("req-2".to_string())), err);
        let res_str = serde_json::to_string(&res).expect("serialize res");
        let parsed: RpcResponse = serde_json::from_str(&res_str).expect("deserialize res");
        assert!(!parsed.is_success());
        assert_eq!(parsed.error.unwrap().code, -32601);
    }

    #[test]
    fn event_envelope_serialization() {
        use pandamux_core::{EnvironmentId, ThreadEventKind, ThreadId, TurnId, TurnInput};

        let env = EventEnvelope {
            subscription_id: "sub-1".to_string(),
            environment_id: EnvironmentId::from("env-local"),
            thread_id: Some(ThreadId::from("thread-1")),
            run_id: None,
            seq: 42,
            at_ms: 1700000000000,
            kind: ThreadEventKind::TurnRequested {
                turn_id: TurnId::from("turn-1"),
                input: TurnInput {
                    text: "Hello".to_string(),
                    attachment_ids: vec![],
                    model: None,
                    effort: None,
                },
            },
        };

        let json_str = serde_json::to_string(&env).expect("serialize env");
        let parsed: EventEnvelope = serde_json::from_str(&json_str).expect("deserialize env");
        assert_eq!(parsed.seq, 42);
        assert_eq!(parsed.subscription_id, "sub-1");
    }
}
