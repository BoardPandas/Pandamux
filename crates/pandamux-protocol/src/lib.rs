pub mod antigravity_rpc;
pub mod attachment_rpc;
pub mod bundle_rpc;
pub mod checkpoint_rpc;
pub mod environment_rpc;
pub mod fs_rpc;
pub mod git_rpc;
pub mod hark_rpc;
pub mod instructions_rpc;
pub mod jsonrpc;
pub mod mcp;
pub mod mutation_rpc;
pub mod personal_config_rpc;
pub mod project_layer_rpc;
pub mod provider_rpc;
pub mod proxy;
pub mod schedule_rpc;
pub mod settings_rpc;
pub mod subagent_rpc;
pub mod subscription;
pub mod system;
pub mod terminal_rpc;
pub mod thread_rpc;
pub mod usage_rpc;
#[cfg(windows)]
pub mod windows_security;
pub mod worktree_rpc;

pub use antigravity_rpc::{
    AntigravityDiscardUploadParams, AntigravityInstallParams, AntigravityInstallResult,
    AntigravityOAuthRelayParams, AntigravityOAuthRelayResult, AntigravityOAuthStartParams,
    AntigravityOAuthStartResult, AntigravityPrepareUploadParams, AntigravityPrepareUploadResult,
    AntigravityStatusParams, AntigravityStatusResult, ManagedBundleFile, ManagedBundleManifest,
};
pub use attachment_rpc::{
    AttachmentGetParams, AttachmentGetResult, AttachmentImportPathParams, AttachmentImportResult,
    AttachmentListParams, AttachmentListResult, AttachmentPutChunkParams, AttachmentPutResult,
};
pub use bundle_rpc::{BUNDLE_PUSH_SCHEMA_VERSION, BundlePushParams, BundlePushResult};
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
pub use hark_rpc::{
    HarkDictatePromptParams, HarkDictatePromptResult, HarkSpellbookEntry, HarkSpellbookSyncParams,
    HarkSpellbookSyncResult, HarkStatusResult,
};
pub use instructions_rpc::{
    InstructionsApproveParams, InstructionsApproveResult, InstructionsCheckParams,
    InstructionsCheckResult, InstructionsDiffParams, InstructionsDiffResult,
    InstructionsHistoryParams, InstructionsHistoryResult, InstructionsProposalResult,
    InstructionsRevertParams, InstructionsSetPolicyParams, InstructionsSetTrackingParams,
    InstructionsStatusParams, InstructionsStatusResult,
};
pub use jsonrpc::{JSONRPC_VERSION, RpcError, RpcId, RpcRequest, RpcResponse};
pub use mcp::{McpCallToolParams, McpCallToolResult, McpContentItem, McpToolDefinition};
pub use mutation_rpc::{
    MutationGetParams, MutationGetResult, MutationListParams, MutationListResult,
    MutationResolveParams, MutationResolveResult,
};
pub use personal_config_rpc::{
    PersonalConfigDiffParams, PersonalConfigDiffResult, PersonalConfigHistoryEntry,
    PersonalConfigHistoryParams, PersonalConfigHistoryResult, PersonalConfigRevertParams,
    PersonalConfigRevertResult,
};
pub use project_layer_rpc::{
    ProjectLayerDiffParams, ProjectLayerDiffResult, ProjectLayerProposalResult,
    ProjectLayerRevokeParams, ProjectLayerStatusParams, ProjectLayerStatusResult,
    ProjectLayerTrustParams,
};
pub use provider_rpc::{
    ProviderLoginFlow, ProviderLoginGuidance, ProviderLoginLaunchParams, ProviderLoginLaunchResult,
    ProviderLoginPreview, ProviderLoginPreviewParams,
};
pub use proxy::{PROXY_PROTOCOL_VERSION, ProxyMessage};
pub use schedule_rpc::{
    ScheduleRunParams, ScheduleRunResult, ScheduleSaveParams, ScheduleValidateParams,
    ScheduleValidationResult,
};
pub use settings_rpc::{
    DEFAULT_PROVIDER_HEALTH_PAGE_SIZE, DEFAULT_PROVIDER_PROBE_CONCURRENCY,
    DEFAULT_PROVIDER_PROBE_TIMEOUT_MS, MAX_PROVIDER_HEALTH_PAGE_SIZE,
    MAX_PROVIDER_PROBE_CONCURRENCY, MAX_PROVIDER_PROBE_TIMEOUT_MS, MIN_PROVIDER_PROBE_TIMEOUT_MS,
    ProviderHealthParams, ProviderHealthReport, ProviderHealthResult, ProviderModelInfo,
    ProviderModelsParams, ProviderModelsReport, ProviderModelsResult, SettingsGetParams,
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
    HubRoutingContext, ThreadCancelTurnParams, ThreadContinueParams, ThreadContinueResult,
    ThreadCreateParams, ThreadGetParams, ThreadGetResult, ThreadListParams,
    ThreadRespondApprovalParams, ThreadResumeParams, ThreadResumeResult, ThreadSendTurnParams,
    ThreadSendTurnResult,
};
pub use usage_rpc::{
    CapacityPushParams, CapacityPushResult, CapacityQueryCursor, CapacityQueryParams,
    CapacityQueryResult, CapacityRefreshParams, CapacityRefreshResult,
    DEFAULT_CAPACITY_REFRESH_TIMEOUT_MS, DEFAULT_USAGE_PAGE_SIZE, MAX_CAPACITY_REFRESH_TIMEOUT_MS,
    MAX_CAPACITY_SNAPSHOTS_PER_PUSH, MAX_USAGE_PAGE_SIZE, RoutingProviderCatalogEntry,
    UsageAckParams, UsageAckResult, UsageAggregate, UsageBackfillCursor, UsageBackfillItem,
    UsageBackfillParams, UsageBackfillResult, UsageCatalogResult, UsageCoverage, UsageCoverageGap,
    UsageEnvironmentSyncCoverage, UsageEnvironmentSyncState, UsageExportItem, UsageExportParams,
    UsageExportResult, UsageNodeCoverageSnapshot, UsageProviderSubscriptionMapping,
    UsageQueryCursor, UsageQueryItem, UsageQueryParams, UsageQueryResult, UsageScannerDiagnostic,
    UsageSourceCoverageState, UsageSubscriptionSetParams,
};
pub use worktree_rpc::{WorktreeSyncDeltaParams, WorktreeSyncDeltaResult};

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
    fn trusted_configuration_contracts_round_trip_without_untyped_hashes() {
        use pandamux_core::{
            ArchiveSha256, BundleSha, BundleSourceManifest, BundleTransferId,
            ConfigurationRevision, InstructionSourceId, SourceCommitSha, TrackingRef,
        };

        let tracking = InstructionsSetTrackingParams {
            request_id: "request-1".to_string(),
            source_id: InstructionSourceId::new("instruction-source-test"),
            tracking: TrackingRef::Commit(
                SourceCommitSha::parse("a".repeat(40)).expect("commit identity"),
            ),
            expected_revision: ConfigurationRevision::new(3).expect("revision"),
        };
        let tracking_json = serde_json::to_value(&tracking).expect("serialize tracking");
        assert_eq!(
            serde_json::from_value::<InstructionsSetTrackingParams>(tracking_json)
                .expect("deserialize tracking"),
            tracking
        );

        let push = BundlePushParams {
            schema_version: BUNDLE_PUSH_SCHEMA_VERSION,
            transfer_id: BundleTransferId::new("bundle-transfer-test"),
            bundle_sha: BundleSha::parse("b".repeat(64)).expect("bundle identity"),
            archive_sha256: ArchiveSha256::parse("c".repeat(64)).expect("archive identity"),
            archive_bytes: 42,
            source_manifest: BundleSourceManifest {
                instructions_commit_sha: SourceCommitSha::parse("d".repeat(40))
                    .expect("source identity"),
                project_layer_content_sha256: None,
                personal_commit_sha: None,
                settings_revision: ConfigurationRevision::new(5).expect("settings revision"),
            },
            compiler_version: "test".to_string(),
            private_artifact_ref: "transfer/test.bundle".to_string(),
            expected_prior_bundle_sha: None,
        };
        let push_json = serde_json::to_value(&push).expect("serialize push");
        assert!(push_json.get("environmentId").is_none());
        assert_eq!(
            serde_json::from_value::<BundlePushParams>(push_json).expect("deserialize push"),
            push
        );
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
