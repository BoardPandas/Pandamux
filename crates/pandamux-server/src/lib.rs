pub mod attachment;
pub mod attempt_worktree;
pub mod bundle_store;
pub mod bundle_sync;
pub mod checkpoint;
pub mod configuration_service;
pub mod data_paths;
pub mod driver_registry;
pub mod environment_router;
pub mod execution_admission;
pub mod execution_budget;
pub mod fs;
pub mod git;
pub mod hark;
pub mod instructions_service;
mod managed_git;
pub mod mcp_server;
pub mod mutation_service;
pub mod personal_repo;
pub mod provider_resolver;
pub mod proxy_connection;
pub mod proxy_transport;
pub mod remote_execution;
pub mod request_context;
pub mod router;
pub mod run_execution_service;
pub mod runtime;
pub mod server;
pub mod store;
pub mod terminal;
pub mod thread_manager;
pub mod usage_scanner;
pub mod usage_sync;
#[cfg(windows)]
pub mod windows_security;
pub mod worktree;

pub use attachment::AttachmentManager;
pub use attempt_worktree::{
    AttemptWorktreeCleanupOutcome, AttemptWorktreeError, AttemptWorktreeFailpoint,
    AttemptWorktreeOutcome, AttemptWorktreeService, CleanupAttemptWorktreeRequest,
    CreateAttemptCheckpointRequest, CreateAttemptWorktreeRequest,
};
pub use bundle_store::{
    BUNDLE_ARCHIVE_SCHEMA_VERSION, BundleMaterializationInput, BundleStore, BundleStoreError,
    MaterializedBundle,
};
pub use bundle_sync::{BundleSyncError, BundleSyncReport, BundleSyncService};
pub use checkpoint::{
    ChangedFileStat, CheckpointPhase, compute_changed_files, create_checkpoint, is_git_repository,
    prune_old_checkpoints, prune_thread_checkpoints, rollback_to_checkpoint,
};
pub use configuration_service::{
    ConfigurationCompilation, ConfigurationService, ConfigurationServiceError, ProjectLayerSnapshot,
};
pub use data_paths::DataPaths;
pub use driver_registry::DriverRegistry;
pub use environment_router::{
    EnvironmentRouter, EnvironmentTransport, EnvironmentTransportState, MockEnvironmentTransport,
};
pub use execution_admission::{
    BASE_EXECUTION_RETRY_DELAY_MS, DEFAULT_EXECUTION_ADMISSION_TTL_MS,
    EnqueueManualAdmissionRequest, EnqueueOrchestratedAdmissionRequest, ExecutionAdmissionClaim,
    ExecutionAdmissionClaimRequest, ExecutionAdmissionError, ExecutionAdmissionService,
    MAX_EXECUTION_ADMISSION_TTL_MS, MAX_EXECUTION_RETRY_DELAY_MS, MIN_EXECUTION_ADMISSION_TTL_MS,
    bounded_execution_retry_delay_ms,
};
pub use execution_budget::{
    ExecutionBudgetError, ExecutionBudgetService, GrantExecutionBudgetRequest,
    PlanExecutionBudgetRequest, bind_supplemental_budget_grant_digest,
};
pub use git::{get_git_status, git_commit, git_create_pr, git_push};
pub use hark::HarkBridge;
pub use instructions_service::{
    CandidateAcquisitionIssue, InstructionsCheckOutcome, InstructionsPollingService,
    InstructionsService, InstructionsServiceError, StoredCandidateInspection,
};
pub use mcp_server::McpServer;
pub use mutation_service::{
    DEFAULT_MUTATION_TTL_MS, MUTATION_SWEEP_INTERVAL, MutationProposalOutcome,
    MutationProposalRequest, MutationService, MutationServiceError, MutationSweepService,
    PersonalMutationState,
};
pub use personal_repo::{
    PersonalConfigDiff, PersonalEditRequest, PersonalEditResult, PersonalRepoError,
    PersonalRepoFailpoint, PersonalRepoService,
};
pub use provider_resolver::{
    ProviderResolver, ResolvedProviderExecution, ResolvedProviderInstance,
};
pub use proxy_connection::serve_proxy_connection;
pub use proxy_transport::ProxyEnvironmentTransport;
pub use remote_execution::{
    NodeExecutionFailpoint, NodeExecutionService, remote_dispatch_identity, remote_execution_ids,
    remote_identity_sha256, remote_preparation_hashes, remote_request_sha256,
};
pub use request_context::{
    CapabilityAuthority, CapabilityScope, ClientIdentity, PeerRole, RequestContext,
};
pub use router::Router;
pub use run_execution_service::{
    LocalProviderDispatch, LocalProviderDispatchStatus, LocalProviderExecutionError,
    LocalProviderTerminalOutcome, PrepareLocalProviderAttemptRequest, PreparedLocalProviderAttempt,
    RunExecutionService, RunTransitionNotice,
};
pub use runtime::{RUNTIME_FILENAME, RuntimeInfo};
pub use server::{Server, ServerConfig};
pub use store::{
    AdmissionCapacityOverage, AdmissionCapacityReconciliation, AdmissionClaimStoreOutcome,
    AdmissionClaimStoreRequest, AdmissionOwnership, AdmissionPolicySnapshot,
    AdmissionRenewStoreRequest, AdmissionRequeueStoreRequest, AssignedRunEvent,
    AttemptDispatchRecord, AttemptDispatchState, BudgetConservationMismatch,
    BudgetConservationReport, BudgetConservationVerdict, BudgetCostSource,
    BudgetExecutionFailpoint, BudgetGrantOutcome, BudgetReconciliationEvidence,
    BudgetReconciliationOutcome, BudgetScopeOverage, BundleActivationFailpoint,
    BundleDistributionRecord, BundleObjectRecord, DailyBudgetRecord, EffectClaimOutcome,
    EffectClaimRequest, EffectDispatchOutcome, EffectDispatchRequest, EffectOutcomeRecord,
    EffectOutcomeRequest, EffectOwnership, EffectPreparationOutcome, LocalAttemptAcceptanceOutcome,
    LocalAttemptOutcome, LocalAttemptOutcomeRequest, LocalAttemptPreparationOutcome,
    NodeReceiptKind, NodeReceiptRecord, OrchestratedAdmissionTransition,
    OrchestrationExecutionFailpoint, OrchestrationQueueRecord, OrchestrationReplayAudit,
    OrchestrationRunRecord, OrchestrationRuntimeRecords, OwnedAttemptWorktreeRecord,
    PersonalRepoStateRecord, ProviderExecutionAudit, QueueState, RevisionedSettings, RunTransition,
    RunTransitionRequest, RunTransitionRows, Store, StoreError, StoredBudgetReservation,
    StoredInstructionCandidate, StoredOrchestrationEffect, execution_ledger_utc_date,
    orchestration_snapshot_sha256, supplemental_budget_action_digest,
};
pub use terminal::TerminalServerManager;
pub use thread_manager::{
    PreparedOrchestratedTurn, PreparedTurnDispatch, PreparedTurnDispatchDisposition,
    PreparedTurnDispatchReceipt, PreparedTurnEvent, ThreadManager,
};
pub use usage_scanner::{
    PlatformFileIdentity, UsageScanRunReport, UsageScanSource, UsageScanTrigger,
    UsageScannerHandle, UsageScannerLimits, UsageScannerRuntimeConfig, UsageScannerService,
};
pub use usage_sync::{
    UsageHubSyncService, UsageHubSynchronizer, UsageSyncHandle, UsageSyncReport,
    UsageSyncRuntimeConfig,
};

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_protocol::ServerRole;

    #[tokio::test]
    async fn server_lifecycle_and_graceful_drain() {
        let temp_dir = std::env::temp_dir().join(format!("pandamux_test_{}", uuid::Uuid::new_v4()));
        let config = ServerConfig {
            role: ServerRole::Hub,
            environment_id: "env-local".to_string(),
            runtime_dir: temp_dir.clone(),
            db_path: None,
        };

        let server = Server::new(config).expect("create server");

        // Test request processing
        let ping_line = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"system.ping\"}\n";
        let res_line = server.process_line(ping_line).await.expect("process ping");
        assert!(res_line.contains("\"result\":{\"pong\":true"));

        // Register runtime file
        let runtime_path = server
            .register_runtime("\\\\.\\pipe\\pandamux-test", "secret-token")
            .expect("register runtime");
        assert!(runtime_path.exists());

        let read_info = RuntimeInfo::read_from_dir(&temp_dir)
            .expect("read runtime")
            .expect("info present");
        assert_eq!(read_info.token, "secret-token");
        assert_eq!(read_info.role, "hub");

        // Provable graceful shutdown and cleanup
        server.shutdown();
        server.cleanup_runtime();

        assert!(!runtime_path.exists());
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
