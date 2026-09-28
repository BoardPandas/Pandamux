pub mod antigravity_rpc;
pub mod attachment_rpc;
pub mod checkpoint_rpc;
pub mod environment_rpc;
pub mod fs_rpc;
pub mod git_rpc;
pub mod jsonrpc;
pub mod mcp;
pub mod settings_rpc;
pub mod subagent_rpc;
pub mod subscription;
pub mod system;
pub mod terminal_rpc;
pub mod thread_rpc;

pub use antigravity_rpc::{
    AntigravityInstallParams, AntigravityInstallResult, AntigravityOAuthRelayParams,
    AntigravityOAuthRelayResult, AntigravityOAuthStartParams, AntigravityOAuthStartResult,
    AntigravityStatusParams, AntigravityStatusResult, ManagedBundleManifest,
};
pub use attachment_rpc::{
    AttachmentGetParams, AttachmentGetResult, AttachmentImportPathParams, AttachmentImportResult,
    AttachmentListParams, AttachmentListResult, AttachmentPutChunkParams, AttachmentPutResult,
};
pub use checkpoint_rpc::{CheckpointDiffParams, CheckpointListParams, CheckpointRollbackParams};
pub use environment_rpc::{
    EnvironmentImportSshConfigParams, EnvironmentImportSshConfigResult, EnvironmentListParams,
    EnvironmentListResult,
};
pub use fs_rpc::{FsEntry, FsListParams, FsListResult, FsReadParams, FsReadResult};
pub use git_rpc::{
    GitCommitParams, GitCommitResult, GitCreatePrParams, GitCreatePrResult, GitFileStatus,
    GitPushParams, GitPushResult, GitStatusParams, GitStatusResult,
};
pub use jsonrpc::{JSONRPC_VERSION, RpcError, RpcId, RpcRequest, RpcResponse};
pub use mcp::{McpCallToolParams, McpCallToolResult, McpContentItem, McpToolDefinition};
pub use settings_rpc::{
    ProviderHealthParams, ProviderHealthReport, ProviderHealthResult, SettingsGetParams,
    SettingsGetResult, SettingsSetParams, SettingsSetResult,
};
pub use subagent_rpc::{SubAgentTreeItem, SubAgentTreeParams, SubAgentTreeResult};
pub use subscription::{EventEnvelope, SubscribeParams, SubscribeResult, UnsubscribeParams};
pub use system::{
    HelloParams, HelloResult, IdentifyResult, PROTOCOL_VERSION, PingResult, ServerCapabilities,
    ServerRole,
};
pub use terminal_rpc::{
    TerminalAttachParams, TerminalAttachResult, TerminalCloseParams, TerminalExitedEvent,
    TerminalInfo, TerminalInputParams, TerminalListResult, TerminalOpenParams, TerminalOpenResult,
    TerminalOutputEvent, TerminalResizeParams,
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
                    attachments: vec![],
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

    #[test]
    fn subagent_tree_round_trip() {
        use pandamux_core::ThreadId;

        let child = SubAgentTreeItem {
            id: "sub-2".to_string(),
            parent_sub_agent_id: Some("sub-1".to_string()),
            parent_item_id: None,
            title: "Nested exploration".to_string(),
            agent_type: "explore".to_string(),
            model: "claude-3-5-haiku".to_string(),
            effort: Some("low".to_string()),
            latest_activity: Some("Inspecting target/".to_string()),
            tokens: 500,
            tool_calls: 2,
            outcome: Some("completed".to_string()),
            elapsed_ms: Some(1200),
            summary: Some("Found 3 build artifacts".to_string()),
            finished: true,
            children: vec![],
        };

        let root = SubAgentTreeItem {
            id: "sub-1".to_string(),
            parent_sub_agent_id: None,
            parent_item_id: None,
            title: "Task runner".to_string(),
            agent_type: "task".to_string(),
            model: "claude-3-7-sonnet".to_string(),
            effort: Some("high".to_string()),
            latest_activity: Some("Waiting on sub-agents".to_string()),
            tokens: 2500,
            tool_calls: 5,
            outcome: None,
            elapsed_ms: Some(4500),
            summary: None,
            finished: false,
            children: vec![child],
        };

        let result = SubAgentTreeResult {
            thread_id: ThreadId::from("thread-100"),
            root_agents: vec![root],
            total_count: 2,
        };

        let json = serde_json::to_string(&result).expect("serialize SubAgentTreeResult");
        let parsed: SubAgentTreeResult =
            serde_json::from_str(&json).expect("deserialize SubAgentTreeResult");
        assert_eq!(parsed.thread_id.as_str(), "thread-100");
        assert_eq!(parsed.total_count, 2);
        assert_eq!(parsed.root_agents.len(), 1);
        assert_eq!(parsed.root_agents[0].children.len(), 1);
        assert_eq!(parsed.root_agents[0].children[0].id, "sub-2");
    }

    #[test]
    fn test_terminal_rpc_round_trip() {
        let open_params = TerminalOpenParams {
            terminal_id: Some("term-1".to_string()),
            environment_id: None,
            cwd: Some("/workspace".to_string()),
            shell: Some("bash".to_string()),
            rows: Some(30),
            cols: Some(120),
            env: None,
        };
        let open_json = serde_json::to_string(&open_params).expect("serialize open");
        let parsed_open: TerminalOpenParams =
            serde_json::from_str(&open_json).expect("deserialize open");
        assert_eq!(parsed_open.terminal_id.as_deref(), Some("term-1"));
        assert_eq!(parsed_open.rows, Some(30));

        let attach_res = TerminalAttachResult {
            terminal_id: "term-1".to_string(),
            rows: 30,
            cols: 120,
            offset: 4096,
            data: "prompt$ ".to_string(),
            truncated: false,
        };
        let attach_json = serde_json::to_string(&attach_res).expect("serialize attach");
        let parsed_attach: TerminalAttachResult =
            serde_json::from_str(&attach_json).expect("deserialize attach");
        assert_eq!(parsed_attach.offset, 4096);
        assert_eq!(parsed_attach.data, "prompt$ ");
        assert!(!parsed_attach.truncated);
    }
}
