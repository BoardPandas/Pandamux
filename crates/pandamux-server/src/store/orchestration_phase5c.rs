use super::{Store, StoreError, checked_i64, checked_u32, checked_u64, decode_usage_observation};
use pandamux_core::{
    AgentId, ArtifactKey, Attempt, AttemptBoundary, AttemptProgressId, AttemptProgressKind,
    AttemptProgressRecord, AttemptStatus, AttemptWorktreeId, BlackboardEntry, BlackboardEntryId,
    BudgetAmount, BudgetDimension, BudgetGrantId, BudgetIncomplete, BudgetReservation,
    BudgetReservationId, BudgetReservationState, BudgetUsage, CheckpointId, CheckpointKind,
    EffectDeliveryRecord, EffectDeliveryState, EnvironmentId, ExecutionLease, ExecutionLeaseId,
    LeaseResourceKind, MAX_ARTIFACT_BYTES, MAX_ARTIFACT_CONTAINER_ITEMS, MAX_ARTIFACT_JSON_DEPTH,
    MAX_ARTIFACT_STRING_BYTES, MAX_ARTIFACTS_PER_RUN, MAX_ARTIFACTS_PER_TASK,
    MAX_RUN_ARTIFACT_BYTES, NodeStoreId, OrchestrationEffect, OrchestrationEffectId,
    OrganizationId, OwnedAttemptWorktree, PricingVersionId, ProjectId, ProjectRepositoryId,
    ProviderInstanceId, RecoveryReview, RecoveryReviewStatus, Run, RunAttemptId, RunCommand,
    RunEvent, RunEventId, RunId, RunStatus, RunTaskId, ServerIncarnationId,
    SupplementalBudgetGrant, TaskRouteDecision, ThreadId, TokenCounts, TurnAttemptId,
    TurnAttemptStatus, TurnId, UsageFactKind, UsageObservation, UsageObservationId,
    UsagePricingRates, UsageSourceKind, UsageUnknownField, WorktreeCleanupEligibility,
    WorktreeDirtyState, estimate_usage_micro_usd,
};
use pandamux_protocol::{OrchestrationAttemptPrepareResult, OrchestrationNodeAttemptState};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrchestrationRunRecord {
    pub run: Run,
    pub project_id: ProjectId,
    pub repository_id: ProjectRepositoryId,
    pub settings_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredOrchestrationEffect {
    pub run_id: RunId,
    pub effect: OrchestrationEffect,
    pub delivery: EffectDeliveryRecord,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptDispatchState {
    Prepared,
    DispatchStarted,
    Accepted,
    NotStarted,
    Ambiguous,
    Terminal,
    CancelRequested,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptDispatchRecord {
    pub attempt_id: RunAttemptId,
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub effect_id: OrchestrationEffectId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub turn_attempt_id: TurnAttemptId,
    pub provider_instance_id: pandamux_core::ProviderInstanceId,
    pub route: TaskRouteDecision,
    pub bundle_sha: String,
    pub state: AttemptDispatchState,
    pub last_boundary: AttemptBoundary,
    pub last_progress_at_ms: u64,
    pub cancel_requested_at_ms: Option<u64>,
    pub ambiguity_reason: Option<String>,
    pub outcome_digest: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState {
    Queued,
    Claimed,
    Cancelled,
    Dispatched,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrchestrationQueueRecord {
    pub queue_id: String,
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub attempt_id: RunAttemptId,
    pub agent_id: AgentId,
    pub organization_id: Option<OrganizationId>,
    pub agent_concurrency_limit: Option<u32>,
    pub provider_instance_id: pandamux_core::ProviderInstanceId,
    pub environment_id: pandamux_core::EnvironmentId,
    pub candidate_sha256: String,
    pub priority: i64,
    pub task_enqueued_at_ms: u64,
    pub enqueued_at_ms: u64,
    pub not_before_ms: u64,
    pub state: QueueState,
    pub owner_incarnation_id: Option<ServerIncarnationId>,
    pub lease_id: Option<ExecutionLeaseId>,
    pub claim_revision: u64,
    pub interactive: bool,
    pub claimed_at_ms: Option<u64>,
    pub claim_order: Option<u64>,
    pub interactive_streak: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionPolicySnapshot {
    pub settings_revision: Option<u64>,
    pub global_limit: u32,
    pub environment_limits: BTreeMap<EnvironmentId, u32>,
    pub provider_limits: BTreeMap<(EnvironmentId, ProviderInstanceId), u32>,
    pub organization_limits: BTreeMap<OrganizationId, u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrchestratedAdmissionTransition {
    pub reservation: StoredBudgetReservation,
    pub attempt: Attempt,
    pub daily_budget_limit_micro_usd: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionClaimStoreRequest {
    pub queue_id: String,
    pub expected_candidate_sha256: String,
    pub expected_claim_revision: u64,
    pub owner_incarnation_id: ServerIncarnationId,
    pub lease_id: ExecutionLeaseId,
    pub claimed_at_ms: u64,
    pub lease_expires_at_ms: u64,
    pub policy: AdmissionPolicySnapshot,
    pub orchestration: Option<OrchestratedAdmissionTransition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionClaimStoreOutcome {
    pub record: OrchestrationQueueRecord,
    pub lease: ExecutionLease,
    pub acquired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionOwnership {
    pub queue_id: String,
    pub expected_candidate_sha256: String,
    pub owner_incarnation_id: ServerIncarnationId,
    pub lease_id: ExecutionLeaseId,
    pub claim_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionRenewStoreRequest {
    pub ownership: AdmissionOwnership,
    pub renewed_at_ms: u64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionRequeueStoreRequest {
    pub ownership: AdmissionOwnership,
    pub reclaimed_at_ms: u64,
    pub not_before_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionCapacityOverage {
    pub dimension: String,
    pub resource_id: String,
    pub active: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionCapacityReconciliation {
    pub active_claims: u32,
    pub overages: Vec<AdmissionCapacityOverage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedAttemptWorktreeRecord {
    pub worktree: OwnedAttemptWorktree,
    /// Digest of the immutable checkpoint operation payload keyed by the
    /// checkpoint identity. This is stored in the existing checkpoint JSON
    /// column so a crash before Git still leaves a changed-byte replay guard.
    pub checkpoint_intent_sha256: BTreeMap<CheckpointId, String>,
    pub verified_process_death_sha256: Option<String>,
    pub retained_evidence_sha256: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredWorktreeCheckpoints {
    #[serde(default)]
    ids: BTreeSet<CheckpointId>,
    #[serde(default)]
    intent_sha256: BTreeMap<CheckpointId, String>,
    #[serde(default)]
    verified_process_death_sha256: Option<String>,
    #[serde(default)]
    retained_evidence_sha256: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredWorktreeCheckpointsCompat {
    Current(StoredWorktreeCheckpoints),
    Legacy(BTreeSet<CheckpointId>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredBudgetReservation {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub organization_id: OrganizationId,
    pub agent_budget_limit: BudgetAmount,
    pub reservation: BudgetReservation,
    pub grants: BTreeMap<BudgetGrantId, SupplementalBudgetGrant>,
    pub reconciliation: Option<BudgetReconciliationEvidence>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetCostSource {
    ProviderReported,
    EffectiveDatedEstimate,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetReconciliationEvidence {
    pub turn_id: TurnId,
    pub turn_attempt_id: TurnAttemptId,
    pub usage_observation_ids: BTreeSet<UsageObservationId>,
    pub source_sha256: String,
    pub cost_source: BudgetCostSource,
    pub pricing_version_ids: BTreeSet<PricingVersionId>,
    pub actual: BudgetUsage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetExecutionFailpoint {
    BeforeGrantCommit,
    AfterGrantCommit,
    BeforeReconciliationCommit,
    AfterReconciliationCommit,
    BeforeReleaseCommit,
    AfterReleaseCommit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetGrantOutcome {
    pub run: Run,
    pub snapshot_sha256: String,
    pub grant: SupplementalBudgetGrant,
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetReconciliationOutcome {
    pub run: Run,
    pub snapshot_sha256: String,
    pub reservation: StoredBudgetReservation,
    pub daily_budget: DailyBudgetRecord,
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetConservationMismatch {
    pub scope: String,
    pub dimension: BudgetDimension,
    pub expected: u64,
    pub actual: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetScopeOverage {
    pub scope: String,
    pub resource_id: String,
    pub amount: BudgetAmount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetConservationVerdict {
    Mismatch,
    BalancedSourceUnverified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetConservationReport {
    pub run_id: RunId,
    pub verdict: BudgetConservationVerdict,
    pub mismatches: Vec<BudgetConservationMismatch>,
    pub incomplete: BudgetIncomplete,
    pub run_overage: BudgetAmount,
    pub scope_overages: Vec<BudgetScopeOverage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailyBudgetRecord {
    pub organization_id: OrganizationId,
    pub ledger_utc_date: String,
    pub limit_micro_usd: u64,
    pub reserved_micro_usd: u64,
    pub reconciled_micro_usd: u64,
    pub granted_micro_usd: u64,
    pub revision: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeReceiptKind {
    RepositoryPrepare,
    AttemptPrepare,
    AttemptDispatch,
    AttemptStatus,
    AttemptCancel,
    AttemptEvents,
    AttemptAck,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeReceiptRecord {
    pub receipt_id: String,
    pub node_store_id: NodeStoreId,
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub attempt_id: RunAttemptId,
    pub effect_id: OrchestrationEffectId,
    pub request_kind: NodeReceiptKind,
    pub idempotency_key: String,
    pub request_sha256: String,
    pub result_sha256: String,
    pub result_bytes: Vec<u8>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrchestrationRuntimeRecords {
    pub attempt_dispatches: Vec<AttemptDispatchRecord>,
    pub queue_records: Vec<OrchestrationQueueRecord>,
    pub leases: Vec<ExecutionLease>,
    pub worktrees: Vec<OwnedAttemptWorktreeRecord>,
    pub budget_reservations: Vec<StoredBudgetReservation>,
    pub daily_budgets: Vec<DailyBudgetRecord>,
    pub artifacts: Vec<BlackboardEntry>,
    pub progress: Vec<AttemptProgressRecord>,
    pub recovery_reviews: Vec<RecoveryReview>,
    pub node_receipts: Vec<NodeReceiptRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptProgressDisposition {
    Persisted,
    Duplicate,
    Coalesced,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptProgressOutcome {
    pub disposition: AttemptProgressDisposition,
    pub record: Option<AttemptProgressRecord>,
    pub last_progress_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryLeaseDisposition {
    Reclaimed,
    Unexpired,
    ForeignOwner,
    LiveOwner,
    ClockAnomaly,
    Ineligible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrchestrationExecutionFailpoint {
    AfterEventCommit,
    BeforeEffectClaim,
    AfterEffectClaim,
    BeforeDispatchMarker,
    AfterDispatchMarker,
    BeforeOutcomeCommit,
    AfterOutcomeCommit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignedRunEvent {
    pub event_id: RunEventId,
    pub idempotency_key: String,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunTransitionRows {
    pub leases: Vec<ExecutionLease>,
    pub worktrees: Vec<OwnedAttemptWorktreeRecord>,
    pub queue_records: Vec<OrchestrationQueueRecord>,
    pub budget_reservations: Vec<StoredBudgetReservation>,
    pub daily_budgets: Vec<DailyBudgetRecord>,
    pub artifacts: Vec<BlackboardEntry>,
    pub recovery_reviews: Vec<RecoveryReview>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunTransitionRequest {
    pub run_id: RunId,
    pub expected_last_seq: u64,
    pub expected_snapshot_sha256: String,
    pub command: RunCommand,
    pub assigned_events: Vec<AssignedRunEvent>,
    pub rows: RunTransitionRows,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunTransition {
    pub run: Run,
    pub snapshot_sha256: String,
    pub events: Vec<RunEvent>,
    pub effect_ids: Vec<OrchestrationEffectId>,
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectClaimRequest {
    pub effect_id: OrchestrationEffectId,
    pub expected_payload_sha256: String,
    pub owner_incarnation_id: ServerIncarnationId,
    pub lease: ExecutionLease,
    pub claimed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectClaimOutcome {
    pub effect: StoredOrchestrationEffect,
    pub acquired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectOwnership {
    pub effect_id: OrchestrationEffectId,
    pub expected_payload_sha256: String,
    pub owner_incarnation_id: ServerIncarnationId,
    pub lease_id: ExecutionLeaseId,
    pub claim_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectRenewStoreRequest {
    pub ownership: EffectOwnership,
    pub renewed_at_ms: u64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectRenewStoreOutcome {
    pub effect: StoredOrchestrationEffect,
    pub lease: ExecutionLease,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionLeaseRenewStoreRequest {
    pub lease: ExecutionLease,
    pub renewed_at_ms: u64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectDispatchRequest {
    pub ownership: EffectOwnership,
    pub dispatch_started_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectDispatchOutcome {
    pub effect: StoredOrchestrationEffect,
    pub dispatch_allowed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectPreparationOutcome {
    pub effect: StoredOrchestrationEffect,
    pub prepared: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectOutcomeRequest {
    pub ownership: EffectOwnership,
    pub result_digest: String,
    pub safe_error_code: Option<String>,
    pub recorded_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectOutcomeRecord {
    pub effect: StoredOrchestrationEffect,
    pub recorded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAttemptPreparationOutcome {
    pub effect: StoredOrchestrationEffect,
    pub attempt: AttemptDispatchRecord,
    pub prepared: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAttemptAcceptanceOutcome {
    pub effect: StoredOrchestrationEffect,
    pub attempt: AttemptDispatchRecord,
    pub accepted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAttemptOutcomeRequest {
    pub ownership: EffectOwnership,
    pub attempt_id: RunAttemptId,
    pub state: AttemptDispatchState,
    pub boundary: AttemptBoundary,
    pub result_digest: String,
    pub safe_error_code: Option<String>,
    pub ambiguity_reason: Option<String>,
    pub recorded_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAttemptOutcome {
    pub effect: StoredOrchestrationEffect,
    pub attempt: AttemptDispatchRecord,
    pub recorded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrchestrationReplayAudit {
    pub run_id: RunId,
    pub last_seq: u64,
    pub snapshot_sha256: String,
    pub event_count: usize,
    pub effect_count: usize,
}

pub fn orchestration_snapshot_sha256(run: &Run) -> String {
    sha256_hex(&pandamux_orchestrator::canonical_run_bytes(run))
}

pub fn supplemental_budget_action_digest(grant: &SupplementalBudgetGrant) -> String {
    let dimension = match grant.dimension {
        BudgetDimension::Tokens => "tokens",
        BudgetDimension::CostMicroUsd => "cost_micro_usd",
        BudgetDimension::RuntimeMs => "runtime_ms",
    };
    let amount = grant.amount.to_string();
    let expires_at_ms = grant.expires_at_ms.to_string();
    let mut digest = Sha256::new();
    for field in [
        grant.id.as_str(),
        grant.run_id.as_str(),
        grant.task_id.as_str(),
        grant.attempt_id.as_str(),
        grant.approval_id.as_str(),
        dimension,
        amount.as_str(),
        grant.policy_revision.as_str(),
        expires_at_ms.as_str(),
    ] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

impl Store {
    /// Inserts one canonical run, its immutable event log, and every derived
    /// effect in one transaction. The snapshot is accepted only when replay
    /// produces byte-identical canonical state.
    pub fn insert_orchestration_run(
        &self,
        record: &OrchestrationRunRecord,
        events: &[RunEvent],
    ) -> Result<Vec<StoredOrchestrationEffect>, StoreError> {
        validate_run_metadata(record)?;
        let (replayed, effects) = replay_with_effects(events)?;
        let snapshot_bytes = pandamux_orchestrator::canonical_run_bytes(&record.run);
        let replay_bytes = pandamux_orchestrator::canonical_run_bytes(&replayed);
        if snapshot_bytes != replay_bytes {
            return Err(data_error(
                "orchestration snapshot does not match event replay",
            ));
        }
        let snapshot_sha256 = sha256_hex(&snapshot_bytes);
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO orchestration_runs (
                run_id, organization_id, project_id, repository_id,
                snapshot_bytes, snapshot_sha256, last_seq, status, bundle_sha,
                policy_revision, settings_revision, created_at_ms, updated_at_ms, ended_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                record.run.id.as_str(),
                record.run.organization_id.as_str(),
                record.project_id.as_str(),
                record.repository_id.as_str(),
                snapshot_bytes,
                snapshot_sha256,
                checked_i64(record.run.last_seq, "orchestration run last sequence")?,
                enum_text(record.run.status)?,
                record.run.bundle_sha,
                record.run.policy_revision,
                checked_i64(record.settings_revision, "orchestration settings revision")?,
                checked_i64(record.run.created_at_ms, "orchestration run created at")?,
                checked_i64(record.run.updated_at_ms, "orchestration run updated at")?,
                record
                    .run
                    .ended_at_ms
                    .map(|value| checked_i64(value, "orchestration run ended at"))
                    .transpose()?,
            ],
        )?;
        for event in events {
            insert_event(&tx, event)?;
        }
        let stored_effects = effects
            .into_iter()
            .map(|effect| stored_effect_from_run(&record.run, events, effect))
            .collect::<Result<Vec<_>, _>>()?;
        for effect in &stored_effects {
            insert_effect(&tx, effect)?;
        }
        tx.commit()?;
        Ok(stored_effects)
    }

    /// Loads a canonical run only after verifying row digests, immutable event
    /// bytes, event replay, snapshot metadata, and the derived effect payloads.
    pub fn load_verified_orchestration_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<OrchestrationRunRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT organization_id, project_id, repository_id, snapshot_bytes,
                        snapshot_sha256, last_seq, status, bundle_sha, policy_revision,
                        settings_revision, created_at_ms, updated_at_ms, ended_at_ms
                   FROM orchestration_runs WHERE run_id = ?1",
                [run_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                        row.get::<_, i64>(11)?,
                        row.get::<_, Option<i64>>(12)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            organization_id,
            project_id,
            repository_id,
            snapshot_bytes,
            snapshot_sha256,
            last_seq,
            status,
            bundle_sha,
            policy_revision,
            settings_revision,
            created_at_ms,
            updated_at_ms,
            ended_at_ms,
        )) = row
        else {
            return Ok(None);
        };
        if snapshot_sha256 != sha256_hex(&snapshot_bytes) {
            return Err(data_error("orchestration snapshot digest mismatch"));
        }
        let snapshot: Run = serde_json::from_slice(&snapshot_bytes)?;
        let events = load_events(&conn, run_id)?;
        let (replayed, derived_effects) = replay_with_effects(&events)?;
        let replay_bytes = pandamux_orchestrator::canonical_run_bytes(&replayed);
        if replay_bytes != snapshot_bytes {
            return Err(data_error(
                "orchestration replay differs from stored snapshot",
            ));
        }
        let stored_effects = load_effects(&conn, run_id)?;
        verify_effect_rows(&derived_effects, &stored_effects)?;
        let stored_status: RunStatus = parse_enum(&status, "orchestration run status")?;
        if snapshot.id != *run_id
            || snapshot.organization_id.as_str() != organization_id
            || snapshot.last_seq != checked_u64(last_seq, "orchestration run last sequence")?
            || snapshot.status != stored_status
            || snapshot.bundle_sha != bundle_sha
            || snapshot.policy_revision != policy_revision
            || snapshot.created_at_ms != checked_u64(created_at_ms, "orchestration run created at")?
            || snapshot.updated_at_ms != checked_u64(updated_at_ms, "orchestration run updated at")?
            || snapshot.ended_at_ms
                != ended_at_ms
                    .map(|value| checked_u64(value, "orchestration run ended at"))
                    .transpose()?
        {
            return Err(data_error("orchestration snapshot metadata mismatch"));
        }
        let record = OrchestrationRunRecord {
            run: snapshot,
            project_id: ProjectId::from(project_id),
            repository_id: ProjectRepositoryId::from(repository_id),
            settings_revision: checked_u64(settings_revision, "orchestration settings revision")?,
        };
        validate_run_metadata(&record)?;
        Ok(Some(record))
    }

    pub fn load_orchestration_events(&self, run_id: &RunId) -> Result<Vec<RunEvent>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_events(&conn, run_id)
    }

    pub fn load_orchestration_effects(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<StoredOrchestrationEffect>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_effects(&conn, run_id)
    }

    /// Persists operational records as one all-or-nothing unit. This is the
    /// storage boundary used by later execution checkpoints; filesystem and
    /// provider effects remain outside it.
    pub fn insert_orchestration_runtime_records(
        &self,
        records: &OrchestrationRuntimeRecords,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for lease in &records.leases {
            insert_lease(&tx, lease)?;
        }
        for record in &records.attempt_dispatches {
            insert_attempt_dispatch(&tx, record)?;
        }
        for record in &records.queue_records {
            insert_queue_record(&tx, record)?;
        }
        for record in &records.worktrees {
            insert_worktree(&tx, record)?;
        }
        for record in &records.budget_reservations {
            insert_budget_reservation(&tx, record)?;
        }
        for record in &records.daily_budgets {
            insert_daily_budget(&tx, record)?;
        }
        let mut artifact_snapshots = BTreeMap::new();
        for artifact in &records.artifacts {
            if !artifact_snapshots.contains_key(&artifact.run_id) {
                artifact_snapshots.insert(
                    artifact.run_id.clone(),
                    load_snapshot(&tx, &artifact.run_id)?,
                );
            }
            insert_artifact(
                &tx,
                artifact_snapshots
                    .get(&artifact.run_id)
                    .expect("artifact snapshot was inserted"),
                artifact,
            )?;
        }
        for progress in &records.progress {
            insert_progress(&tx, progress)?;
        }
        for review in &records.recovery_reviews {
            insert_recovery_review(&tx, review)?;
        }
        for receipt in &records.node_receipts {
            insert_node_receipt(&tx, receipt, true)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn commit_run_transition(
        &self,
        request: &RunTransitionRequest,
        failpoint: Option<OrchestrationExecutionFailpoint>,
    ) -> Result<RunTransition, StoreError> {
        validate_failpoint(
            failpoint,
            &[OrchestrationExecutionFailpoint::AfterEventCommit],
        )?;
        if !is_sha256(&request.expected_snapshot_sha256) {
            return Err(data_error(
                "expected orchestration snapshot digest is malformed",
            ));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_snapshot(&tx, &request.run_id)?;
        let stored_events = load_events(&tx, &request.run_id)?;
        let (replayed, derived_effects) = replay_with_effects(&stored_events)?;
        if pandamux_orchestrator::canonical_run_bytes(&current)
            != pandamux_orchestrator::canonical_run_bytes(&replayed)
        {
            return Err(data_error(
                "orchestration replay differs from stored snapshot",
            ));
        }
        let stored_effects = load_effects(&tx, &request.run_id)?;
        verify_effect_rows(&derived_effects, &stored_effects)?;
        if request.expected_last_seq > current.last_seq {
            return Err(data_error(
                "expected orchestration sequence is ahead of durable state",
            ));
        }
        let base_events = stored_events
            .iter()
            .take_while(|event| event.seq <= request.expected_last_seq)
            .cloned()
            .collect::<Vec<_>>();
        let (base, _) = replay_with_effects(&base_events)?;
        if base.last_seq != request.expected_last_seq
            || orchestration_snapshot_sha256(&base) != request.expected_snapshot_sha256
        {
            return Err(data_error(
                "expected orchestration snapshot identity does not match replay",
            ));
        }
        let kinds = pandamux_orchestrator::decide(&base, request.command.clone())
            .map_err(orchestration_error)?;
        if kinds.len() != request.assigned_events.len() {
            return Err(data_error(
                "server-assigned event identities do not match the decided event count",
            ));
        }
        let mut next = base.clone();
        let mut events = Vec::with_capacity(kinds.len());
        let mut new_effects = Vec::new();
        for (kind, assigned) in kinds.into_iter().zip(&request.assigned_events) {
            if assigned.event_id.as_str().trim().is_empty()
                || assigned.idempotency_key.trim().is_empty()
            {
                return Err(data_error(
                    "assigned orchestration event identity is malformed",
                ));
            }
            let event = RunEvent {
                event_id: assigned.event_id.clone(),
                run_id: request.run_id.clone(),
                seq: next
                    .last_seq
                    .checked_add(1)
                    .ok_or_else(|| data_error("orchestration event sequence overflow"))?,
                at_ms: assigned.at_ms,
                idempotency_key: assigned.idempotency_key.clone(),
                kind,
            };
            let outcome =
                pandamux_orchestrator::apply(&next, &event).map_err(orchestration_error)?;
            next = outcome.run;
            new_effects.extend(outcome.new_effects);
            events.push(event);
        }
        let exact_duplicate = transition_events_match(&tx, &events)?;
        if exact_duplicate {
            verify_transition_rows(&tx, &request.run_id, &request.rows)?;
            return Ok(RunTransition {
                snapshot_sha256: orchestration_snapshot_sha256(&next),
                effect_ids: new_effects.iter().map(|effect| effect.id.clone()).collect(),
                run: next,
                events,
                duplicate: true,
            });
        }
        if transition_events_collide(&tx, &events)? {
            return Err(data_error(
                "orchestration event or idempotency identity was reused with different bytes",
            ));
        }
        if current.last_seq != request.expected_last_seq
            || orchestration_snapshot_sha256(&current) != request.expected_snapshot_sha256
        {
            return Err(data_error(
                "orchestration run changed before the transition could commit",
            ));
        }
        for event in &events {
            insert_event(&tx, event)?;
        }
        let stored_new_effects = new_effects
            .iter()
            .cloned()
            .map(|effect| stored_effect_from_run(&next, &events, effect))
            .collect::<Result<Vec<_>, _>>()?;
        for effect in &stored_new_effects {
            insert_effect(&tx, effect)?;
        }
        update_run_snapshot(
            &tx,
            &request.run_id,
            &next,
            request.expected_last_seq,
            &request.expected_snapshot_sha256,
        )?;
        insert_transition_rows(&tx, &request.run_id, &request.rows)?;
        settle_acknowledged_effect(&tx, &events)?;
        tx.commit()?;
        if failpoint == Some(OrchestrationExecutionFailpoint::AfterEventCommit) {
            return Err(injected_failure("after_event_commit"));
        }
        Ok(RunTransition {
            snapshot_sha256: orchestration_snapshot_sha256(&next),
            effect_ids: stored_new_effects
                .iter()
                .map(|effect| effect.effect.id.clone())
                .collect(),
            run: next,
            events,
            duplicate: false,
        })
    }

    pub fn audit_orchestration_replay(
        &self,
        run_id: &RunId,
    ) -> Result<OrchestrationReplayAudit, StoreError> {
        let record = self
            .load_verified_orchestration_run(run_id)?
            .ok_or_else(|| data_error("orchestration run was not found"))?;
        let events = self.load_orchestration_events(run_id)?;
        let effects = self.load_orchestration_effects(run_id)?;
        Ok(OrchestrationReplayAudit {
            run_id: run_id.clone(),
            last_seq: record.run.last_seq,
            snapshot_sha256: orchestration_snapshot_sha256(&record.run),
            event_count: events.len(),
            effect_count: effects.len(),
        })
    }
}

impl Store {
    pub fn get_orchestration_effect(
        &self,
        effect_id: &OrchestrationEffectId,
    ) -> Result<Option<StoredOrchestrationEffect>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_stored_effect_by_id(&conn, effect_id)
    }

    pub fn list_pending_orchestration_effects(
        &self,
        limit: u32,
    ) -> Result<Vec<StoredOrchestrationEffect>, StoreError> {
        let limit = limit.clamp(1, 256);
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT effect_id FROM orchestration_effects
              WHERE delivery_state = 'pending'
              ORDER BY created_at_ms, effect_id LIMIT ?1",
        )?;
        let effect_ids = statement
            .query_map([i64::from(limit)], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        effect_ids
            .into_iter()
            .map(|effect_id| {
                load_stored_effect_by_id(&conn, &OrchestrationEffectId::from(effect_id))?
                    .ok_or_else(|| data_error("pending orchestration effect disappeared"))
            })
            .collect()
    }

    pub fn claim_orchestration_effect(
        &self,
        request: &EffectClaimRequest,
        failpoint: Option<OrchestrationExecutionFailpoint>,
    ) -> Result<EffectClaimOutcome, StoreError> {
        validate_effect_claim_request(request)?;
        validate_failpoint(
            failpoint,
            &[
                OrchestrationExecutionFailpoint::BeforeEffectClaim,
                OrchestrationExecutionFailpoint::AfterEffectClaim,
            ],
        )?;
        if failpoint == Some(OrchestrationExecutionFailpoint::BeforeEffectClaim) {
            return Err(injected_failure("before_effect_claim"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_stored_effect_by_id(&tx, &request.effect_id)?
            .ok_or_else(|| data_error("orchestration effect was not found"))?;
        if current.effect.payload_sha256 != request.expected_payload_sha256 {
            return Err(data_error("orchestration effect payload identity changed"));
        }
        let next_revision = current
            .delivery
            .claim_revision
            .checked_add(1)
            .ok_or_else(|| data_error("effect claim revision overflow"))?;
        let exact_owner =
            effect_has_owner(&current, &request.owner_incarnation_id, &request.lease.id);
        let acquired = match current.delivery.state {
            EffectDeliveryState::Pending => {
                if request.lease.revision != next_revision {
                    return Err(data_error(
                        "effect lease revision does not match the next claim revision",
                    ));
                }
                ensure_execution_lease(&tx, &request.lease)?;
                let changed = tx.execute(
                    "UPDATE orchestration_effects
                        SET delivery_state = 'claimed', owner_incarnation_id = ?1,
                            lease_id = ?2, claim_revision = ?3, updated_at_ms = ?4
                      WHERE effect_id = ?5 AND delivery_state = 'pending'
                        AND claim_revision = ?6 AND payload_sha256 = ?7",
                    params![
                        request.owner_incarnation_id.as_str(),
                        request.lease.id.as_str(),
                        checked_i64(next_revision, "effect claim revision")?,
                        checked_i64(request.claimed_at_ms, "effect claim timestamp")?,
                        request.effect_id.as_str(),
                        checked_i64(current.delivery.claim_revision, "prior claim revision")?,
                        request.expected_payload_sha256,
                    ],
                )?;
                if changed != 1 {
                    return Err(data_error(
                        "orchestration effect claim lost its compare-and-swap",
                    ));
                }
                true
            }
            _ if exact_owner
                && current.delivery.claim_revision == request.lease.revision
                && load_lease(&tx, &request.lease.id)?.as_ref() == Some(&request.lease) =>
            {
                false
            }
            _ => {
                return Err(data_error(
                    "orchestration effect is already owned or no longer claimable",
                ));
            }
        };
        let effect = load_stored_effect_by_id(&tx, &request.effect_id)?
            .ok_or_else(|| data_error("claimed orchestration effect disappeared"))?;
        tx.commit()?;
        if failpoint == Some(OrchestrationExecutionFailpoint::AfterEffectClaim) {
            return Err(injected_failure("after_effect_claim"));
        }
        Ok(EffectClaimOutcome { effect, acquired })
    }

    /// Renews an in-flight effect owner and its generic lease in one CAS. The
    /// returned effect revision is the only ownership accepted by later
    /// dispatch/outcome operations.
    pub fn renew_orchestration_effect(
        &self,
        request: &EffectRenewStoreRequest,
    ) -> Result<EffectRenewStoreOutcome, StoreError> {
        validate_effect_ownership(&request.ownership)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (effect, lease) = load_owned_effect(&tx, &request.ownership)?;
        if !matches!(
            effect.delivery.state,
            EffectDeliveryState::Claimed
                | EffectDeliveryState::Prepared
                | EffectDeliveryState::DispatchStarted
                | EffectDeliveryState::InFlight
        ) || request.renewed_at_ms < lease.last_heartbeat_ms
            || request.renewed_at_ms >= lease.expires_at_ms
            || request.lease_expires_at_ms <= request.renewed_at_ms
        {
            return Err(data_error("orchestration effect lease cannot be renewed"));
        }
        let next_revision = request
            .ownership
            .claim_revision
            .checked_add(1)
            .ok_or_else(|| data_error("orchestration effect revision overflow"))?;
        let effect_changed = tx.execute(
            "UPDATE orchestration_effects SET claim_revision = ?1, updated_at_ms = ?2
              WHERE effect_id = ?3 AND payload_sha256 = ?4
                AND owner_incarnation_id = ?5 AND lease_id = ?6
                AND claim_revision = ?7
                AND delivery_state IN ('claimed', 'prepared', 'dispatch_started', 'in_flight')",
            params![
                checked_i64(next_revision, "renewed effect claim revision")?,
                checked_i64(request.renewed_at_ms, "renewed effect timestamp")?,
                request.ownership.effect_id.as_str(),
                request.ownership.expected_payload_sha256,
                request.ownership.owner_incarnation_id.as_str(),
                request.ownership.lease_id.as_str(),
                checked_i64(request.ownership.claim_revision, "effect claim revision")?,
            ],
        )?;
        let lease_changed = tx.execute(
            "UPDATE orchestration_leases
                SET expires_at_ms = ?1, last_heartbeat_ms = ?2, revision = ?3
              WHERE lease_id = ?4 AND resource_kind = 'effect' AND resource_id = ?5
                AND owner_incarnation_id = ?6 AND revision = ?7",
            params![
                checked_i64(request.lease_expires_at_ms, "renewed effect lease expiry")?,
                checked_i64(request.renewed_at_ms, "renewed effect lease heartbeat")?,
                checked_i64(next_revision, "renewed effect lease revision")?,
                request.ownership.lease_id.as_str(),
                request.ownership.effect_id.as_str(),
                request.ownership.owner_incarnation_id.as_str(),
                checked_i64(request.ownership.claim_revision, "effect lease revision")?,
            ],
        )?;
        if effect_changed != 1 || lease_changed != 1 {
            return Err(data_error(
                "orchestration effect renewal lost its compare-and-swap",
            ));
        }
        let effect = load_stored_effect_by_id(&tx, &request.ownership.effect_id)?
            .ok_or_else(|| data_error("renewed orchestration effect disappeared"))?;
        let lease = load_lease(&tx, &request.ownership.lease_id)?
            .ok_or_else(|| data_error("renewed orchestration effect lease disappeared"))?;
        tx.commit()?;
        Ok(EffectRenewStoreOutcome { effect, lease })
    }

    /// Renews a worktree or remote-attempt lease whose owner record does not
    /// carry a parallel claim revision. Capacity and effect leases must use
    /// their atomic owner-specific renewal transactions instead.
    pub fn renew_owned_execution_lease(
        &self,
        request: &ExecutionLeaseRenewStoreRequest,
    ) -> Result<ExecutionLease, StoreError> {
        if request.lease.id.as_str().trim().is_empty()
            || request.lease.resource_id.trim().is_empty()
            || request
                .lease
                .owner_incarnation_id
                .as_str()
                .trim()
                .is_empty()
            || request.lease.revision == 0
            || !matches!(
                request.lease.resource_kind,
                LeaseResourceKind::Worktree | LeaseResourceKind::RemoteAttempt
            )
            || request.renewed_at_ms < request.lease.last_heartbeat_ms
            || request.renewed_at_ms >= request.lease.expires_at_ms
            || request.lease_expires_at_ms <= request.renewed_at_ms
        {
            return Err(data_error("owned execution lease cannot be renewed"));
        }
        let next_revision = request
            .lease
            .revision
            .checked_add(1)
            .ok_or_else(|| data_error("owned execution lease revision overflow"))?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_lease(&tx, &request.lease.id)?
            .ok_or_else(|| data_error("owned execution lease was not found"))?;
        if current != request.lease {
            return Err(data_error(
                "owned execution lease no longer matches durable state",
            ));
        }
        let changed = tx.execute(
            "UPDATE orchestration_leases
                SET expires_at_ms = ?1, last_heartbeat_ms = ?2, revision = ?3
              WHERE lease_id = ?4 AND resource_kind = ?5 AND resource_id = ?6
                AND owner_incarnation_id = ?7 AND expires_at_ms = ?8
                AND last_heartbeat_ms = ?9 AND revision = ?10",
            params![
                checked_i64(request.lease_expires_at_ms, "renewed owned lease expiry")?,
                checked_i64(request.renewed_at_ms, "renewed owned lease heartbeat")?,
                checked_i64(next_revision, "renewed owned lease revision")?,
                request.lease.id.as_str(),
                enum_text(request.lease.resource_kind)?,
                request.lease.resource_id,
                request.lease.owner_incarnation_id.as_str(),
                checked_i64(request.lease.expires_at_ms, "prior owned lease expiry")?,
                checked_i64(
                    request.lease.last_heartbeat_ms,
                    "prior owned lease heartbeat"
                )?,
                checked_i64(request.lease.revision, "prior owned lease revision")?,
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "owned execution lease renewal lost its compare-and-swap",
            ));
        }
        let renewed = load_lease(&tx, &request.lease.id)?
            .ok_or_else(|| data_error("renewed owned execution lease disappeared"))?;
        tx.commit()?;
        Ok(renewed)
    }

    pub fn mark_orchestration_effect_prepared(
        &self,
        ownership: &EffectOwnership,
        prepared_receipt_digest: &str,
        prepared_at_ms: u64,
    ) -> Result<EffectPreparationOutcome, StoreError> {
        validate_effect_ownership(ownership)?;
        if !is_sha256(prepared_receipt_digest) {
            return Err(data_error("prepared effect receipt digest is malformed"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, lease) = load_owned_effect(&tx, ownership)?;
        validate_effect_action_time(&current, &lease, prepared_at_ms)?;
        let prepared = match current.delivery.state {
            EffectDeliveryState::Claimed => {
                update_effect_state(
                    &tx,
                    ownership,
                    EffectDeliveryState::Claimed,
                    EffectDeliveryState::Prepared,
                    Some(prepared_receipt_digest),
                    None,
                    None,
                    None,
                    prepared_at_ms,
                )?;
                true
            }
            EffectDeliveryState::Prepared
            | EffectDeliveryState::DispatchStarted
            | EffectDeliveryState::InFlight
            | EffectDeliveryState::OutcomeRecorded
                if current.delivery.prepared_receipt_digest.as_deref()
                    == Some(prepared_receipt_digest) =>
            {
                false
            }
            _ => {
                return Err(data_error(
                    "orchestration effect cannot enter the prepared state",
                ));
            }
        };
        let effect = load_stored_effect_by_id(&tx, &ownership.effect_id)?
            .ok_or_else(|| data_error("prepared orchestration effect disappeared"))?;
        tx.commit()?;
        Ok(EffectPreparationOutcome { effect, prepared })
    }

    /// Atomically binds the prepared provider identities to the claimed effect.
    /// Thread, turn, turn-attempt, worktree, admission, and budget rows must
    /// already exist; this transaction is the final durable gate before a
    /// dispatch marker can be written.
    pub fn prepare_local_attempt_dispatch(
        &self,
        record: &AttemptDispatchRecord,
        ownership: &EffectOwnership,
        prepared_receipt_digest: &str,
        prepared_at_ms: u64,
    ) -> Result<LocalAttemptPreparationOutcome, StoreError> {
        validate_effect_ownership(ownership)?;
        validate_attempt_dispatch_record(record)?;
        if record.effect_id != ownership.effect_id
            || record.state != AttemptDispatchState::Prepared
            || record.created_at_ms != prepared_at_ms
            || record.updated_at_ms != prepared_at_ms
            || !is_sha256(prepared_receipt_digest)
        {
            return Err(data_error(
                "local attempt preparation identity is malformed",
            ));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, lease) = load_owned_effect(&tx, ownership)?;
        validate_effect_action_time(&current, &lease, prepared_at_ms)?;
        if current.run_id != record.run_id
            || !effect_matches_attempt(
                &current.effect,
                &record.run_id,
                &record.task_id,
                &record.attempt_id,
                Some(&record.route),
            )
        {
            return Err(data_error(
                "local attempt preparation differs from the claimed effect",
            ));
        }
        validate_local_attempt_preparation_rows(&tx, record)?;
        let existing = load_attempt_dispatches(&tx, &record.run_id)?
            .into_iter()
            .find(|candidate| candidate.attempt_id == record.attempt_id);
        let prepared = match (current.delivery.state, existing) {
            (EffectDeliveryState::Claimed, None) => {
                insert_attempt_dispatch(&tx, record)?;
                update_effect_state(
                    &tx,
                    ownership,
                    EffectDeliveryState::Claimed,
                    EffectDeliveryState::Prepared,
                    Some(prepared_receipt_digest),
                    None,
                    None,
                    None,
                    prepared_at_ms,
                )?;
                true
            }
            (EffectDeliveryState::Prepared, Some(existing))
                if existing == *record
                    && current.delivery.prepared_receipt_digest.as_deref()
                        == Some(prepared_receipt_digest) =>
            {
                false
            }
            _ => {
                return Err(data_error(
                    "local attempt preparation conflicts with durable state",
                ));
            }
        };
        let effect = load_stored_effect_by_id(&tx, &ownership.effect_id)?
            .ok_or_else(|| data_error("prepared local effect disappeared"))?;
        let attempt = load_attempt_dispatches(&tx, &record.run_id)?
            .into_iter()
            .find(|candidate| candidate.attempt_id == record.attempt_id)
            .ok_or_else(|| data_error("prepared local attempt disappeared"))?;
        tx.commit()?;
        Ok(LocalAttemptPreparationOutcome {
            effect,
            attempt,
            prepared,
        })
    }

    /// Atomically binds a Hub-owned dispatch record to the exact durable node
    /// prepare receipt. The node owns its filesystem and provider preparation;
    /// the Hub still owns effect, admission, budget, and recovery ordering.
    pub fn prepare_remote_attempt_dispatch(
        &self,
        record: &AttemptDispatchRecord,
        ownership: &EffectOwnership,
        prepared_result: &OrchestrationAttemptPrepareResult,
        prepared_at_ms: u64,
    ) -> Result<LocalAttemptPreparationOutcome, StoreError> {
        validate_effect_ownership(ownership)?;
        validate_attempt_dispatch_record(record)?;
        let prepared_receipt_digest = prepared_result.receipt.receipt_sha256.as_str();
        if record.effect_id != ownership.effect_id
            || record.state != AttemptDispatchState::Prepared
            || record.created_at_ms != prepared_at_ms
            || record.updated_at_ms != prepared_at_ms
            || !is_sha256(prepared_receipt_digest)
        {
            return Err(data_error(
                "remote attempt preparation identity is malformed",
            ));
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, lease) = load_owned_effect(&tx, ownership)?;
        validate_effect_action_time(&current, &lease, prepared_at_ms)?;
        if current.run_id != record.run_id
            || !effect_matches_attempt(
                &current.effect,
                &record.run_id,
                &record.task_id,
                &record.attempt_id,
                Some(&record.route),
            )
        {
            return Err(data_error(
                "remote attempt preparation differs from the claimed effect",
            ));
        }
        validate_remote_attempt_preparation_rows(&tx, record, prepared_result)?;

        let existing = load_attempt_dispatches(&tx, &record.run_id)?
            .into_iter()
            .find(|candidate| candidate.attempt_id == record.attempt_id);
        let prepared = match (current.delivery.state, existing) {
            (EffectDeliveryState::Claimed, None) => {
                insert_attempt_dispatch(&tx, record)?;
                update_effect_state(
                    &tx,
                    ownership,
                    EffectDeliveryState::Claimed,
                    EffectDeliveryState::Prepared,
                    Some(prepared_receipt_digest),
                    None,
                    None,
                    None,
                    prepared_at_ms,
                )?;
                true
            }
            (EffectDeliveryState::Prepared, Some(existing))
                if existing == *record
                    && current.delivery.prepared_receipt_digest.as_deref()
                        == Some(prepared_receipt_digest) =>
            {
                false
            }
            _ => {
                return Err(data_error(
                    "remote attempt preparation conflicts with durable state",
                ));
            }
        };
        let effect = load_stored_effect_by_id(&tx, &ownership.effect_id)?
            .ok_or_else(|| data_error("prepared remote effect disappeared"))?;
        let attempt = load_attempt_dispatches(&tx, &record.run_id)?
            .into_iter()
            .find(|candidate| candidate.attempt_id == record.attempt_id)
            .ok_or_else(|| data_error("prepared remote attempt disappeared"))?;
        tx.commit()?;
        Ok(LocalAttemptPreparationOutcome {
            effect,
            attempt,
            prepared,
        })
    }

    pub fn mark_orchestration_effect_dispatch_started(
        &self,
        request: &EffectDispatchRequest,
        failpoint: Option<OrchestrationExecutionFailpoint>,
    ) -> Result<EffectDispatchOutcome, StoreError> {
        validate_effect_ownership(&request.ownership)?;
        validate_failpoint(
            failpoint,
            &[
                OrchestrationExecutionFailpoint::BeforeDispatchMarker,
                OrchestrationExecutionFailpoint::AfterDispatchMarker,
            ],
        )?;
        if failpoint == Some(OrchestrationExecutionFailpoint::BeforeDispatchMarker) {
            return Err(injected_failure("before_dispatch_marker"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, lease) = load_owned_effect(&tx, &request.ownership)?;
        validate_effect_action_time(&current, &lease, request.dispatch_started_at_ms)?;
        let dispatch_allowed = match current.delivery.state {
            EffectDeliveryState::Prepared => {
                update_effect_state(
                    &tx,
                    &request.ownership,
                    EffectDeliveryState::Prepared,
                    EffectDeliveryState::DispatchStarted,
                    current.delivery.prepared_receipt_digest.as_deref(),
                    Some(request.dispatch_started_at_ms),
                    None,
                    None,
                    request.dispatch_started_at_ms,
                )?;
                let has_attempt: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM orchestration_attempt_dispatch WHERE effect_id = ?1",
                    [request.ownership.effect_id.as_str()],
                    |row| row.get(0),
                )?;
                if has_attempt == 1 {
                    update_attempt_state_for_effect(
                        &tx,
                        &request.ownership.effect_id,
                        AttemptDispatchState::Prepared,
                        AttemptDispatchState::DispatchStarted,
                        AttemptBoundary::SafeToRetry,
                        None,
                        None,
                        request.dispatch_started_at_ms,
                    )?;
                }
                true
            }
            EffectDeliveryState::DispatchStarted
            | EffectDeliveryState::InFlight
            | EffectDeliveryState::OutcomeRecorded
                if current.delivery.dispatch_started_at_ms
                    == Some(request.dispatch_started_at_ms) =>
            {
                false
            }
            _ => {
                return Err(data_error(
                    "orchestration effect cannot write a dispatch marker",
                ));
            }
        };
        let effect = load_stored_effect_by_id(&tx, &request.ownership.effect_id)?
            .ok_or_else(|| data_error("dispatch-marked orchestration effect disappeared"))?;
        tx.commit()?;
        if failpoint == Some(OrchestrationExecutionFailpoint::AfterDispatchMarker) {
            return Err(injected_failure("after_dispatch_marker"));
        }
        Ok(EffectDispatchOutcome {
            effect,
            dispatch_allowed,
        })
    }

    pub fn mark_local_attempt_accepted(
        &self,
        ownership: &EffectOwnership,
        attempt_id: &RunAttemptId,
        accepted_at_ms: u64,
    ) -> Result<LocalAttemptAcceptanceOutcome, StoreError> {
        validate_effect_ownership(ownership)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, lease) = load_owned_effect(&tx, ownership)?;
        validate_effect_action_time(&current, &lease, accepted_at_ms)?;
        let attempts = load_attempt_dispatches(&tx, &current.run_id)?;
        let existing = attempts
            .into_iter()
            .find(|record| record.attempt_id == *attempt_id)
            .ok_or_else(|| data_error("local attempt dispatch was not found"))?;
        if existing.effect_id != ownership.effect_id {
            return Err(data_error(
                "local attempt acceptance differs from its effect",
            ));
        }
        let accepted = match (current.delivery.state, existing.state) {
            (EffectDeliveryState::DispatchStarted, AttemptDispatchState::DispatchStarted) => {
                update_effect_state(
                    &tx,
                    ownership,
                    EffectDeliveryState::DispatchStarted,
                    EffectDeliveryState::InFlight,
                    current.delivery.prepared_receipt_digest.as_deref(),
                    current.delivery.dispatch_started_at_ms,
                    None,
                    None,
                    accepted_at_ms,
                )?;
                update_attempt_state_for_effect(
                    &tx,
                    &ownership.effect_id,
                    AttemptDispatchState::DispatchStarted,
                    AttemptDispatchState::Accepted,
                    existing.last_boundary,
                    None,
                    None,
                    accepted_at_ms,
                )?;
                true
            }
            (EffectDeliveryState::InFlight, AttemptDispatchState::Accepted) => false,
            _ => {
                return Err(data_error(
                    "local attempt cannot record provider acceptance",
                ));
            }
        };
        let effect = load_stored_effect_by_id(&tx, &ownership.effect_id)?
            .ok_or_else(|| data_error("accepted local effect disappeared"))?;
        let attempt = load_attempt_dispatches(&tx, &current.run_id)?
            .into_iter()
            .find(|record| record.attempt_id == *attempt_id)
            .ok_or_else(|| data_error("accepted local attempt disappeared"))?;
        tx.commit()?;
        Ok(LocalAttemptAcceptanceOutcome {
            effect,
            attempt,
            accepted,
        })
    }

    pub fn record_local_attempt_boundary(
        &self,
        ownership: &EffectOwnership,
        attempt_id: &RunAttemptId,
        boundary: AttemptBoundary,
        recorded_at_ms: u64,
    ) -> Result<AttemptDispatchRecord, StoreError> {
        validate_effect_ownership(ownership)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, lease) = load_owned_effect(&tx, ownership)?;
        validate_effect_action_time(&current, &lease, recorded_at_ms)?;
        if !matches!(current.delivery.state, EffectDeliveryState::InFlight) {
            return Err(data_error(
                "local attempt boundary requires an in-flight effect",
            ));
        }
        let existing = load_attempt_dispatches(&tx, &current.run_id)?
            .into_iter()
            .find(|record| record.attempt_id == *attempt_id)
            .ok_or_else(|| data_error("local attempt dispatch was not found"))?;
        if existing.effect_id != ownership.effect_id
            || !matches!(
                existing.state,
                AttemptDispatchState::Accepted | AttemptDispatchState::CancelRequested
            )
            || boundary < existing.last_boundary
            || recorded_at_ms < existing.updated_at_ms
        {
            return Err(data_error("local attempt boundary is stale or malformed"));
        }
        if boundary > existing.last_boundary {
            update_attempt_state_for_effect(
                &tx,
                &ownership.effect_id,
                existing.state,
                existing.state,
                boundary,
                None,
                None,
                recorded_at_ms,
            )?;
        }
        let attempt = load_attempt_dispatches(&tx, &current.run_id)?
            .into_iter()
            .find(|record| record.attempt_id == *attempt_id)
            .ok_or_else(|| data_error("updated local attempt disappeared"))?;
        tx.commit()?;
        Ok(attempt)
    }

    pub fn record_local_attempt_outcome(
        &self,
        request: &LocalAttemptOutcomeRequest,
        failpoint: Option<OrchestrationExecutionFailpoint>,
    ) -> Result<LocalAttemptOutcome, StoreError> {
        validate_effect_ownership(&request.ownership)?;
        validate_failpoint(
            failpoint,
            &[
                OrchestrationExecutionFailpoint::BeforeOutcomeCommit,
                OrchestrationExecutionFailpoint::AfterOutcomeCommit,
            ],
        )?;
        if !matches!(
            request.state,
            AttemptDispatchState::NotStarted
                | AttemptDispatchState::Ambiguous
                | AttemptDispatchState::Terminal
                | AttemptDispatchState::Cancelled
        ) || !is_sha256(&request.result_digest)
            || request
                .safe_error_code
                .as_deref()
                .is_some_and(|code| !valid_reason_code(code))
            || (request.state == AttemptDispatchState::NotStarted
                && (request.boundary != AttemptBoundary::SafeToRetry
                    || request.safe_error_code.is_none()))
            || (request.state != AttemptDispatchState::NotStarted
                && request.safe_error_code.is_some())
            || (request.state == AttemptDispatchState::Ambiguous
                && request
                    .ambiguity_reason
                    .as_deref()
                    .is_none_or(str::is_empty))
            || (request.state != AttemptDispatchState::Ambiguous
                && request.ambiguity_reason.is_some())
        {
            return Err(data_error("local attempt outcome is malformed"));
        }
        if failpoint == Some(OrchestrationExecutionFailpoint::BeforeOutcomeCommit) {
            return Err(injected_failure("before_outcome_commit"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, _) = load_owned_effect(&tx, &request.ownership)?;
        if request.recorded_at_ms < current.updated_at_ms {
            return Err(data_error(
                "local attempt outcome timestamp moved backwards",
            ));
        }
        let existing = load_attempt_dispatches(&tx, &current.run_id)?
            .into_iter()
            .find(|record| record.attempt_id == request.attempt_id)
            .ok_or_else(|| data_error("local attempt dispatch was not found"))?;
        if existing.effect_id != request.ownership.effect_id
            || request.boundary < existing.last_boundary
        {
            return Err(data_error(
                "local attempt outcome differs from its durable dispatch",
            ));
        }
        let (expected_attempt_state, expected_effect_state) = match request.state {
            AttemptDispatchState::NotStarted => (
                AttemptDispatchState::DispatchStarted,
                EffectDeliveryState::DispatchStarted,
            ),
            AttemptDispatchState::Ambiguous
                if matches!(
                    existing.state,
                    AttemptDispatchState::Accepted | AttemptDispatchState::CancelRequested
                ) && current.delivery.state == EffectDeliveryState::InFlight =>
            {
                (
                    AttemptDispatchState::Accepted,
                    EffectDeliveryState::InFlight,
                )
            }
            AttemptDispatchState::Ambiguous => (
                AttemptDispatchState::DispatchStarted,
                EffectDeliveryState::DispatchStarted,
            ),
            AttemptDispatchState::Terminal
                if matches!(
                    existing.state,
                    AttemptDispatchState::Accepted | AttemptDispatchState::CancelRequested
                ) && current.delivery.state == EffectDeliveryState::InFlight =>
            {
                (existing.state, EffectDeliveryState::InFlight)
            }
            AttemptDispatchState::Cancelled
                if existing.state == AttemptDispatchState::CancelRequested
                    && current.delivery.state == EffectDeliveryState::InFlight =>
            {
                (
                    AttemptDispatchState::CancelRequested,
                    EffectDeliveryState::InFlight,
                )
            }
            _ => {
                return Err(data_error(
                    "local attempt outcome conflicts with its current lifecycle state",
                ));
            }
        };
        let next_effect_state = if request.state == AttemptDispatchState::Ambiguous {
            EffectDeliveryState::ReviewRequired
        } else {
            EffectDeliveryState::OutcomeRecorded
        };
        let recorded = if current.delivery.state == expected_effect_state
            && existing.state == expected_attempt_state
        {
            update_effect_state(
                &tx,
                &request.ownership,
                expected_effect_state,
                next_effect_state,
                current.delivery.prepared_receipt_digest.as_deref(),
                current.delivery.dispatch_started_at_ms,
                Some(&request.result_digest),
                request.safe_error_code.as_deref(),
                request.recorded_at_ms,
            )?;
            update_attempt_state_for_effect(
                &tx,
                &request.ownership.effect_id,
                expected_attempt_state,
                request.state,
                request.boundary,
                request.ambiguity_reason.as_deref(),
                Some(&request.result_digest),
                request.recorded_at_ms,
            )?;
            true
        } else if current.delivery.state == next_effect_state
            && existing.state == request.state
            && current.delivery.result_digest.as_deref() == Some(&request.result_digest)
            && current.delivery.safe_error_code == request.safe_error_code
            && existing.last_boundary == request.boundary
            && existing.ambiguity_reason == request.ambiguity_reason
            && existing.outcome_digest.as_deref() == Some(&request.result_digest)
        {
            false
        } else {
            return Err(data_error(
                "local attempt outcome conflicts with durable state",
            ));
        };
        let effect = load_stored_effect_by_id(&tx, &request.ownership.effect_id)?
            .ok_or_else(|| data_error("outcome local effect disappeared"))?;
        let attempt = load_attempt_dispatches(&tx, &current.run_id)?
            .into_iter()
            .find(|record| record.attempt_id == request.attempt_id)
            .ok_or_else(|| data_error("outcome local attempt disappeared"))?;
        tx.commit()?;
        if failpoint == Some(OrchestrationExecutionFailpoint::AfterOutcomeCommit) {
            return Err(injected_failure("after_outcome_commit"));
        }
        Ok(LocalAttemptOutcome {
            effect,
            attempt,
            recorded,
        })
    }

    pub fn get_attempt_dispatch(
        &self,
        attempt_id: &RunAttemptId,
    ) -> Result<Option<AttemptDispatchRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let run_id = conn
            .query_row(
                "SELECT run_id FROM orchestration_attempt_dispatch WHERE attempt_id = ?1",
                [attempt_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        Ok(load_attempt_dispatches(&conn, &RunId::from(run_id))?
            .into_iter()
            .find(|record| record.attempt_id == *attempt_id))
    }

    pub fn get_attempt_dispatch_for_turn_attempt(
        &self,
        turn_attempt_id: &TurnAttemptId,
    ) -> Result<Option<AttemptDispatchRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let run_id = conn
            .query_row(
                "SELECT run_id FROM orchestration_attempt_dispatch
                  WHERE turn_attempt_id = ?1",
                [turn_attempt_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        Ok(load_attempt_dispatches(&conn, &RunId::from(run_id))?
            .into_iter()
            .find(|record| record.turn_attempt_id == *turn_attempt_id))
    }

    pub fn record_orchestration_effect_outcome(
        &self,
        request: &EffectOutcomeRequest,
        failpoint: Option<OrchestrationExecutionFailpoint>,
    ) -> Result<EffectOutcomeRecord, StoreError> {
        validate_effect_ownership(&request.ownership)?;
        validate_failpoint(
            failpoint,
            &[
                OrchestrationExecutionFailpoint::BeforeOutcomeCommit,
                OrchestrationExecutionFailpoint::AfterOutcomeCommit,
            ],
        )?;
        if !is_sha256(&request.result_digest)
            || request
                .safe_error_code
                .as_deref()
                .is_some_and(|code| !valid_reason_code(code))
        {
            return Err(data_error("orchestration effect outcome is malformed"));
        }
        if failpoint == Some(OrchestrationExecutionFailpoint::BeforeOutcomeCommit) {
            return Err(injected_failure("before_outcome_commit"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, _) = load_owned_effect(&tx, &request.ownership)?;
        if request.recorded_at_ms < current.updated_at_ms {
            return Err(data_error(
                "orchestration effect outcome timestamp moved backwards",
            ));
        }
        let recorded = match current.delivery.state {
            EffectDeliveryState::DispatchStarted | EffectDeliveryState::InFlight => {
                update_effect_state(
                    &tx,
                    &request.ownership,
                    current.delivery.state,
                    EffectDeliveryState::OutcomeRecorded,
                    current.delivery.prepared_receipt_digest.as_deref(),
                    current.delivery.dispatch_started_at_ms,
                    Some(&request.result_digest),
                    request.safe_error_code.as_deref(),
                    request.recorded_at_ms,
                )?;
                true
            }
            EffectDeliveryState::OutcomeRecorded
                if current.delivery.result_digest.as_deref()
                    == Some(request.result_digest.as_str())
                    && current.delivery.safe_error_code == request.safe_error_code =>
            {
                false
            }
            _ => {
                return Err(data_error(
                    "orchestration effect outcome conflicts with durable delivery state",
                ));
            }
        };
        let effect = load_stored_effect_by_id(&tx, &request.ownership.effect_id)?
            .ok_or_else(|| data_error("outcome orchestration effect disappeared"))?;
        tx.commit()?;
        if failpoint == Some(OrchestrationExecutionFailpoint::AfterOutcomeCommit) {
            return Err(injected_failure("after_outcome_commit"));
        }
        Ok(EffectOutcomeRecord { effect, recorded })
    }
}

fn orchestration_error(error: pandamux_orchestrator::OrchestrationError) -> StoreError {
    data_error(format!(
        "orchestration transition rejected with {:?}",
        error.code
    ))
}

fn transition_events_match(
    conn: &rusqlite::Connection,
    events: &[RunEvent],
) -> Result<bool, StoreError> {
    for event in events {
        let expected = serde_json::to_vec(event)?;
        let stored = conn
            .query_row(
                "SELECT event_bytes FROM orchestration_run_events
                  WHERE event_id = ?1 AND run_id = ?2 AND seq = ?3
                    AND idempotency_key = ?4",
                params![
                    event.event_id.as_str(),
                    event.run_id.as_str(),
                    checked_i64(event.seq, "orchestration event sequence")?,
                    event.idempotency_key,
                ],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        if stored.as_deref() != Some(expected.as_slice()) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn transition_events_collide(
    conn: &rusqlite::Connection,
    events: &[RunEvent],
) -> Result<bool, StoreError> {
    for event in events {
        let collision = conn
            .query_row(
                "SELECT 1 FROM orchestration_run_events
                  WHERE event_id = ?1 OR (
                    run_id = ?2 AND (seq = ?3 OR idempotency_key = ?4)
                  ) LIMIT 1",
                params![
                    event.event_id.as_str(),
                    event.run_id.as_str(),
                    checked_i64(event.seq, "orchestration event sequence")?,
                    event.idempotency_key,
                ],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if collision {
            return Ok(true);
        }
    }
    Ok(false)
}

fn update_run_snapshot(
    tx: &Transaction<'_>,
    run_id: &RunId,
    run: &Run,
    expected_last_seq: u64,
    expected_snapshot_sha256: &str,
) -> Result<(), StoreError> {
    if run.id != *run_id || run.last_seq <= expected_last_seq {
        return Err(data_error(
            "orchestration transition did not advance the canonical run",
        ));
    }
    let bytes = pandamux_orchestrator::canonical_run_bytes(run);
    let digest = sha256_hex(&bytes);
    let changed = tx.execute(
        "UPDATE orchestration_runs
            SET snapshot_bytes = ?1, snapshot_sha256 = ?2, last_seq = ?3,
                status = ?4, bundle_sha = ?5, policy_revision = ?6,
                updated_at_ms = ?7, ended_at_ms = ?8
          WHERE run_id = ?9 AND last_seq = ?10 AND snapshot_sha256 = ?11",
        params![
            bytes,
            digest,
            checked_i64(run.last_seq, "orchestration run last sequence")?,
            enum_text(run.status)?,
            run.bundle_sha,
            run.policy_revision,
            checked_i64(run.updated_at_ms, "orchestration run updated at")?,
            run.ended_at_ms
                .map(|value| checked_i64(value, "orchestration run ended at"))
                .transpose()?,
            run_id.as_str(),
            checked_i64(expected_last_seq, "expected orchestration sequence")?,
            expected_snapshot_sha256,
        ],
    )?;
    if changed != 1 {
        return Err(data_error(
            "orchestration snapshot update lost its compare-and-swap",
        ));
    }
    Ok(())
}

fn validate_transition_row_scope(
    conn: &rusqlite::Connection,
    run_id: &RunId,
    rows: &RunTransitionRows,
) -> Result<(), StoreError> {
    if rows
        .worktrees
        .iter()
        .any(|record| record.worktree.run_id != *run_id)
        || rows
            .queue_records
            .iter()
            .any(|record| record.run_id != *run_id)
        || rows
            .budget_reservations
            .iter()
            .any(|record| record.run_id != *run_id)
        || rows
            .artifacts
            .iter()
            .any(|artifact| artifact.run_id != *run_id)
        || rows
            .recovery_reviews
            .iter()
            .any(|review| review.run_id != *run_id)
    {
        return Err(data_error(
            "orchestration transition rows belong to a different run",
        ));
    }
    if rows.leases.len() != rows.worktrees.len()
        || rows.worktrees.iter().any(|record| {
            rows.leases.iter().all(|lease| {
                lease.id != record.worktree.lease_id
                    || lease.resource_kind != LeaseResourceKind::Worktree
                    || lease.resource_id != record.worktree.id.as_str()
            })
        })
        || rows.leases.iter().any(|lease| {
            rows.worktrees.iter().all(|record| {
                record.worktree.lease_id != lease.id
                    || lease.resource_kind != LeaseResourceKind::Worktree
                    || lease.resource_id != record.worktree.id.as_str()
            })
        })
    {
        return Err(data_error(
            "orchestration transition worktree leases do not match their resources",
        ));
    }
    let organization_id = conn.query_row(
        "SELECT organization_id FROM orchestration_runs WHERE run_id = ?1",
        [run_id.as_str()],
        |row| row.get::<_, String>(0),
    )?;
    if rows
        .daily_budgets
        .iter()
        .any(|record| record.organization_id.as_str() != organization_id)
    {
        return Err(data_error(
            "orchestration transition daily budget belongs to another organization",
        ));
    }
    Ok(())
}

fn verify_transition_rows(
    conn: &rusqlite::Connection,
    run_id: &RunId,
    rows: &RunTransitionRows,
) -> Result<(), StoreError> {
    validate_transition_row_scope(conn, run_id, rows)?;
    let stored_queue = load_queue_records(conn, run_id)?;
    let stored_worktrees = load_worktrees(conn, run_id)?;
    let stored_reservations = load_budget_reservations(conn, run_id)?;
    let stored_artifacts = load_artifacts(conn, run_id)?;
    let stored_reviews = load_recovery_reviews(conn, run_id)?;
    for lease in &rows.leases {
        if load_lease(conn, &lease.id)?.as_ref() != Some(lease) {
            return Err(data_error(
                "duplicate orchestration worktree lease changed durable bytes",
            ));
        }
    }
    if rows
        .worktrees
        .iter()
        .any(|record| !stored_worktrees.contains(record))
        || rows
            .queue_records
            .iter()
            .any(|record| !stored_queue.contains(record))
        || rows
            .budget_reservations
            .iter()
            .any(|record| !stored_reservations.contains(record))
        || rows
            .artifacts
            .iter()
            .any(|artifact| !stored_artifacts.contains(artifact))
        || rows
            .recovery_reviews
            .iter()
            .any(|review| !stored_reviews.contains(review))
    {
        return Err(data_error(
            "duplicate orchestration transition rows changed durable bytes",
        ));
    }
    for expected in &rows.daily_budgets {
        let stored = load_daily_budget(conn, &expected.organization_id, &expected.ledger_utc_date)?;
        if stored.as_ref() != Some(expected) {
            return Err(data_error(
                "duplicate orchestration daily budget changed durable bytes",
            ));
        }
    }
    Ok(())
}

fn insert_transition_rows(
    tx: &Transaction<'_>,
    run_id: &RunId,
    rows: &RunTransitionRows,
) -> Result<(), StoreError> {
    validate_transition_row_scope(tx, run_id, rows)?;
    for lease in &rows.leases {
        ensure_execution_lease(tx, lease)?;
    }
    for record in &rows.worktrees {
        insert_worktree(tx, record)?;
    }
    for record in &rows.queue_records {
        insert_queue_record(tx, record)?;
    }
    for record in &rows.daily_budgets {
        insert_daily_budget(tx, record)?;
    }
    for record in &rows.budget_reservations {
        insert_budget_reservation(tx, record)?;
    }
    let artifact_snapshot = (!rows.artifacts.is_empty())
        .then(|| load_snapshot(tx, run_id))
        .transpose()?;
    for artifact in &rows.artifacts {
        insert_artifact(
            tx,
            artifact_snapshot
                .as_ref()
                .expect("nonempty artifacts have a snapshot"),
            artifact,
        )?;
    }
    for review in &rows.recovery_reviews {
        insert_recovery_review(tx, review)?;
    }
    Ok(())
}

fn load_daily_budget(
    conn: &rusqlite::Connection,
    organization_id: &OrganizationId,
    ledger_utc_date: &str,
) -> Result<Option<DailyBudgetRecord>, StoreError> {
    conn.query_row(
        "SELECT limit_micro_usd, reserved_micro_usd, reconciled_micro_usd,
                granted_micro_usd, revision, updated_at_ms
           FROM orchestration_daily_budgets
          WHERE organization_id = ?1 AND ledger_utc_date = ?2",
        params![organization_id.as_str(), ledger_utc_date],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )
    .optional()?
    .map(
        |(limit, reserved, reconciled, granted, revision, updated)| {
            let record = DailyBudgetRecord {
                organization_id: organization_id.clone(),
                ledger_utc_date: ledger_utc_date.to_string(),
                limit_micro_usd: checked_u64(limit, "daily budget limit")?,
                reserved_micro_usd: checked_u64(reserved, "daily budget reserved")?,
                reconciled_micro_usd: checked_u64(reconciled, "daily budget reconciled")?,
                granted_micro_usd: checked_u64(granted, "daily budget granted")?,
                revision: checked_u64(revision, "daily budget revision")?,
                updated_at_ms: checked_u64(updated, "daily budget updated at")?,
            };
            if !valid_utc_date(&record.ledger_utc_date) || record.revision == 0 {
                return Err(data_error("daily budget row is malformed"));
            }
            Ok(record)
        },
    )
    .transpose()
}

fn settle_acknowledged_effect(tx: &Transaction<'_>, events: &[RunEvent]) -> Result<(), StoreError> {
    for event in events {
        let pandamux_core::RunEventKind::EffectAcknowledged { effect_id, receipt } = &event.kind
        else {
            continue;
        };
        let current = load_stored_effect_by_id(tx, effect_id)?
            .ok_or_else(|| data_error("acknowledged orchestration effect was not found"))?;
        if current.run_id != event.run_id
            || current.delivery.state != EffectDeliveryState::OutcomeRecorded
            || current.delivery.result_digest.as_deref() != Some(receipt.result_digest.as_str())
            || receipt.effect_id != *effect_id
            || receipt.acknowledged_at_ms != event.at_ms
            || event.at_ms < current.updated_at_ms
        {
            return Err(data_error(
                "orchestration effect acknowledgement conflicts with durable outcome",
            ));
        }
        let owner = current
            .delivery
            .owner_incarnation_id
            .as_ref()
            .ok_or_else(|| data_error("outcome-recorded effect has no durable owner"))?;
        let lease_id = current
            .delivery
            .lease_id
            .as_ref()
            .ok_or_else(|| data_error("outcome-recorded effect has no durable lease"))?;
        let lease = load_lease(tx, lease_id)?
            .ok_or_else(|| data_error("outcome-recorded effect lease was not found"))?;
        if lease.resource_kind != LeaseResourceKind::Effect
            || lease.resource_id != effect_id.as_str()
            || lease.owner_incarnation_id != *owner
            || lease.revision != current.delivery.claim_revision
        {
            return Err(data_error(
                "outcome-recorded effect lease does not match its owner",
            ));
        }
        let changed = tx.execute(
            "UPDATE orchestration_effects
                SET delivery_state = 'settled', owner_incarnation_id = NULL,
                    lease_id = NULL, updated_at_ms = ?1
              WHERE effect_id = ?2 AND delivery_state = 'outcome_recorded'
                AND owner_incarnation_id = ?3 AND lease_id = ?4
                AND claim_revision = ?5 AND result_digest = ?6",
            params![
                checked_i64(event.at_ms, "effect acknowledgement timestamp")?,
                effect_id.as_str(),
                owner.as_str(),
                lease_id.as_str(),
                checked_i64(current.delivery.claim_revision, "effect claim revision")?,
                receipt.result_digest,
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "orchestration effect acknowledgement lost its compare-and-swap",
            ));
        }
        let deleted = tx.execute(
            "DELETE FROM orchestration_leases
              WHERE lease_id = ?1 AND resource_kind = 'effect' AND resource_id = ?2
                AND owner_incarnation_id = ?3 AND revision = ?4",
            params![
                lease_id.as_str(),
                effect_id.as_str(),
                owner.as_str(),
                checked_i64(current.delivery.claim_revision, "effect claim revision")?,
            ],
        )?;
        if deleted != 1 {
            return Err(data_error(
                "orchestration effect acknowledgement could not release its lease",
            ));
        }
    }
    Ok(())
}

fn injected_failure(name: &str) -> StoreError {
    data_error(format!("injected orchestration failure: {name}"))
}

fn validate_failpoint(
    failpoint: Option<OrchestrationExecutionFailpoint>,
    allowed: &[OrchestrationExecutionFailpoint],
) -> Result<(), StoreError> {
    if failpoint.is_some_and(|candidate| !allowed.contains(&candidate)) {
        return Err(data_error(
            "orchestration failpoint is invalid for this operation",
        ));
    }
    Ok(())
}

fn validate_budget_failpoint(
    failpoint: Option<BudgetExecutionFailpoint>,
    allowed: &[BudgetExecutionFailpoint],
) -> Result<(), StoreError> {
    if failpoint.is_some_and(|candidate| !allowed.contains(&candidate)) {
        return Err(data_error(
            "budget execution failpoint is invalid for this operation",
        ));
    }
    Ok(())
}

fn validate_effect_claim_request(request: &EffectClaimRequest) -> Result<(), StoreError> {
    let lease = &request.lease;
    if request.effect_id.as_str().trim().is_empty()
        || !is_sha256(&request.expected_payload_sha256)
        || request.owner_incarnation_id.as_str().trim().is_empty()
        || lease.id.as_str().trim().is_empty()
        || lease.resource_kind != LeaseResourceKind::Effect
        || lease.resource_id != request.effect_id.as_str()
        || lease.owner_incarnation_id != request.owner_incarnation_id
        || lease.revision == 0
        || lease.last_heartbeat_ms > request.claimed_at_ms
        || request.claimed_at_ms >= lease.expires_at_ms
    {
        return Err(data_error("orchestration effect claim is malformed"));
    }
    Ok(())
}

fn validate_effect_ownership(ownership: &EffectOwnership) -> Result<(), StoreError> {
    if ownership.effect_id.as_str().trim().is_empty()
        || !is_sha256(&ownership.expected_payload_sha256)
        || ownership.owner_incarnation_id.as_str().trim().is_empty()
        || ownership.lease_id.as_str().trim().is_empty()
        || ownership.claim_revision == 0
    {
        return Err(data_error("orchestration effect ownership is malformed"));
    }
    Ok(())
}

fn effect_has_owner(
    effect: &StoredOrchestrationEffect,
    owner: &ServerIncarnationId,
    lease_id: &ExecutionLeaseId,
) -> bool {
    effect.delivery.owner_incarnation_id.as_ref() == Some(owner)
        && effect.delivery.lease_id.as_ref() == Some(lease_id)
}

fn ensure_execution_lease(tx: &Transaction<'_>, lease: &ExecutionLease) -> Result<(), StoreError> {
    if let Some(existing) = load_lease(tx, &lease.id)? {
        if existing == *lease {
            return Ok(());
        }
        return Err(data_error(
            "execution lease identity was reused with different bytes",
        ));
    }
    let resource_kind = enum_text(lease.resource_kind)?;
    let existing_resource = tx
        .query_row(
            "SELECT lease_id FROM orchestration_leases
              WHERE resource_kind = ?1 AND resource_id = ?2",
            params![resource_kind, lease.resource_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if existing_resource.is_some() {
        return Err(data_error(
            "execution lease resource is already owned by another identity",
        ));
    }
    insert_lease(tx, lease)
}

fn load_stored_effect_by_id(
    conn: &rusqlite::Connection,
    effect_id: &OrchestrationEffectId,
) -> Result<Option<StoredOrchestrationEffect>, StoreError> {
    let run_id = conn
        .query_row(
            "SELECT run_id FROM orchestration_effects WHERE effect_id = ?1",
            [effect_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(run_id) = run_id else {
        return Ok(None);
    };
    Ok(load_effects(conn, &RunId::from(run_id))?
        .into_iter()
        .find(|record| record.effect.id == *effect_id))
}

fn load_owned_effect(
    tx: &Transaction<'_>,
    ownership: &EffectOwnership,
) -> Result<(StoredOrchestrationEffect, ExecutionLease), StoreError> {
    let effect = load_stored_effect_by_id(tx, &ownership.effect_id)?
        .ok_or_else(|| data_error("orchestration effect was not found"))?;
    if effect.effect.payload_sha256 != ownership.expected_payload_sha256
        || !effect_has_owner(
            &effect,
            &ownership.owner_incarnation_id,
            &ownership.lease_id,
        )
        || effect.delivery.claim_revision != ownership.claim_revision
    {
        return Err(data_error(
            "orchestration effect ownership no longer matches durable state",
        ));
    }
    let lease = load_lease(tx, &ownership.lease_id)?
        .ok_or_else(|| data_error("orchestration effect lease was not found"))?;
    if lease.resource_kind != LeaseResourceKind::Effect
        || lease.resource_id != ownership.effect_id.as_str()
        || lease.owner_incarnation_id != ownership.owner_incarnation_id
        || lease.revision != ownership.claim_revision
    {
        return Err(data_error(
            "orchestration effect lease no longer matches durable ownership",
        ));
    }
    Ok((effect, lease))
}

fn validate_effect_action_time(
    effect: &StoredOrchestrationEffect,
    lease: &ExecutionLease,
    at_ms: u64,
) -> Result<(), StoreError> {
    if at_ms < effect.updated_at_ms
        || at_ms < lease.last_heartbeat_ms
        || at_ms >= lease.expires_at_ms
    {
        return Err(data_error(
            "orchestration effect action is outside its active lease",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn update_effect_state(
    tx: &Transaction<'_>,
    ownership: &EffectOwnership,
    expected_state: EffectDeliveryState,
    next_state: EffectDeliveryState,
    prepared_receipt_digest: Option<&str>,
    dispatch_started_at_ms: Option<u64>,
    result_digest: Option<&str>,
    safe_error_code: Option<&str>,
    updated_at_ms: u64,
) -> Result<(), StoreError> {
    let changed = tx.execute(
        "UPDATE orchestration_effects
            SET delivery_state = ?1, prepared_receipt_digest = ?2,
                dispatch_started_at_ms = ?3, result_digest = ?4,
                safe_error_code = ?5, updated_at_ms = ?6
          WHERE effect_id = ?7 AND payload_sha256 = ?8
            AND delivery_state = ?9 AND owner_incarnation_id = ?10
            AND lease_id = ?11 AND claim_revision = ?12",
        params![
            enum_text(next_state)?,
            prepared_receipt_digest,
            dispatch_started_at_ms
                .map(|value| checked_i64(value, "effect dispatch timestamp"))
                .transpose()?,
            result_digest,
            safe_error_code,
            checked_i64(updated_at_ms, "effect update timestamp")?,
            ownership.effect_id.as_str(),
            ownership.expected_payload_sha256,
            enum_text(expected_state)?,
            ownership.owner_incarnation_id.as_str(),
            ownership.lease_id.as_str(),
            checked_i64(ownership.claim_revision, "effect claim revision")?,
        ],
    )?;
    if changed != 1 {
        return Err(data_error(
            "orchestration effect update lost its compare-and-swap",
        ));
    }
    Ok(())
}

fn validate_run_metadata(record: &OrchestrationRunRecord) -> Result<(), StoreError> {
    if record.run.id.as_str().is_empty()
        || record.run.organization_id.as_str().is_empty()
        || record.project_id.as_str().is_empty()
        || record.repository_id.as_str().is_empty()
        || record.settings_revision == 0
        || record.run.bundle_sha.len() != 64
        || !is_sha256(&record.run.bundle_sha)
        || record.run.policy_revision.trim().is_empty()
    {
        return Err(data_error("orchestration run metadata is malformed"));
    }
    Ok(())
}

fn replay_with_effects(events: &[RunEvent]) -> Result<(Run, Vec<OrchestrationEffect>), StoreError> {
    let first = events
        .first()
        .ok_or_else(|| data_error("orchestration replay requires an event"))?;
    let pandamux_core::RunEventKind::PlanAccepted { seed, .. } = &first.kind else {
        return Err(data_error(
            "orchestration replay must begin with plan acceptance",
        ));
    };
    let mut run = Run::draft(first.run_id.clone(), seed.clone(), first.at_ms);
    let mut effects = Vec::new();
    for event in events {
        let outcome = pandamux_orchestrator::apply(&run, event)
            .map_err(|error| data_error(format!("orchestration replay failed: {error}")))?;
        effects.extend(outcome.new_effects);
        run = outcome.run;
    }
    Ok((run, effects))
}

fn insert_event(tx: &Transaction<'_>, event: &RunEvent) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec(event)?;
    tx.execute(
        "INSERT INTO orchestration_run_events (
            event_id, run_id, seq, idempotency_key, event_bytes, payload_sha256, at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event.event_id.as_str(),
            event.run_id.as_str(),
            checked_i64(event.seq, "orchestration event sequence")?,
            event.idempotency_key,
            bytes,
            sha256_hex(&bytes),
            checked_i64(event.at_ms, "orchestration event timestamp")?,
        ],
    )?;
    Ok(())
}

fn load_events(conn: &rusqlite::Connection, run_id: &RunId) -> Result<Vec<RunEvent>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT event_id, seq, idempotency_key, event_bytes, payload_sha256, at_ms
           FROM orchestration_run_events WHERE run_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    let mut events = Vec::new();
    for row in rows {
        let (event_id, seq, idempotency_key, bytes, digest, at_ms) = row?;
        if digest != sha256_hex(&bytes) {
            return Err(data_error("orchestration event digest mismatch"));
        }
        let event: RunEvent = serde_json::from_slice(&bytes)?;
        if event.event_id.as_str() != event_id
            || event.run_id != *run_id
            || event.seq != checked_u64(seq, "orchestration event sequence")?
            || event.idempotency_key != idempotency_key
            || event.at_ms != checked_u64(at_ms, "orchestration event timestamp")?
        {
            return Err(data_error("orchestration event row metadata mismatch"));
        }
        events.push(event);
    }
    Ok(events)
}

fn stored_effect_from_run(
    run: &Run,
    events: &[RunEvent],
    effect: OrchestrationEffect,
) -> Result<StoredOrchestrationEffect, StoreError> {
    let source = events
        .iter()
        .find(|event| event.event_id == effect.source_event_id)
        .ok_or_else(|| data_error("derived effect has no source event"))?;
    let settled = run.settled_effects.get(&effect.id);
    let updated_at_ms = settled
        .map(|receipt| receipt.acknowledged_at_ms)
        .unwrap_or(source.at_ms);
    if !is_sha256(&effect.payload_sha256) {
        return Err(data_error(
            "orchestration effect payload digest is malformed",
        ));
    }
    if settled.is_some_and(|receipt| !is_sha256(&receipt.result_digest)) {
        return Err(data_error(
            "orchestration effect result digest is malformed",
        ));
    }
    Ok(StoredOrchestrationEffect {
        run_id: run.id.clone(),
        delivery: EffectDeliveryRecord {
            effect_id: effect.id.clone(),
            payload_sha256: effect.payload_sha256.clone(),
            state: if settled.is_some() {
                EffectDeliveryState::Settled
            } else {
                EffectDeliveryState::Pending
            },
            owner_incarnation_id: None,
            lease_id: None,
            claim_revision: 0,
            prepared_receipt_digest: None,
            dispatch_started_at_ms: None,
            result_digest: settled.map(|receipt| receipt.result_digest.clone()),
            safe_error_code: None,
        },
        effect,
        created_at_ms: source.at_ms,
        updated_at_ms,
    })
}

fn insert_effect(
    tx: &Transaction<'_>,
    record: &StoredOrchestrationEffect,
) -> Result<(), StoreError> {
    validate_effect_record(record)?;
    let bytes = serde_json::to_vec(&record.effect)?;
    tx.execute(
        "INSERT INTO orchestration_effects (
            effect_id, run_id, source_event_id, effect_bytes, payload_sha256,
            delivery_state, owner_incarnation_id, lease_id, claim_revision,
            prepared_receipt_digest, dispatch_started_at_ms, result_digest,
            safe_error_code, created_at_ms, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            record.effect.id.as_str(),
            record.run_id.as_str(),
            record.effect.source_event_id.as_str(),
            bytes,
            record.effect.payload_sha256,
            enum_text(record.delivery.state)?,
            record
                .delivery
                .owner_incarnation_id
                .as_ref()
                .map(ServerIncarnationId::as_str),
            record
                .delivery
                .lease_id
                .as_ref()
                .map(ExecutionLeaseId::as_str),
            checked_i64(record.delivery.claim_revision, "effect claim revision")?,
            record.delivery.prepared_receipt_digest,
            record
                .delivery
                .dispatch_started_at_ms
                .map(|value| checked_i64(value, "effect dispatch timestamp"))
                .transpose()?,
            record.delivery.result_digest,
            record.delivery.safe_error_code,
            checked_i64(record.created_at_ms, "effect created at")?,
            checked_i64(record.updated_at_ms, "effect updated at")?,
        ],
    )?;
    Ok(())
}

fn validate_effect_record(record: &StoredOrchestrationEffect) -> Result<(), StoreError> {
    let owned = record.delivery.owner_incarnation_id.is_some()
        && record.delivery.lease_id.is_some()
        && record.delivery.claim_revision > 0;
    let state_valid = match record.delivery.state {
        EffectDeliveryState::Pending => {
            !owned
                && record.delivery.owner_incarnation_id.is_none()
                && record.delivery.lease_id.is_none()
                && record.delivery.prepared_receipt_digest.is_none()
                && record.delivery.dispatch_started_at_ms.is_none()
                && record.delivery.result_digest.is_none()
                && record.delivery.safe_error_code.is_none()
        }
        EffectDeliveryState::Claimed => {
            owned
                && record.delivery.prepared_receipt_digest.is_none()
                && record.delivery.dispatch_started_at_ms.is_none()
                && record.delivery.result_digest.is_none()
                && record.delivery.safe_error_code.is_none()
        }
        EffectDeliveryState::Prepared => {
            owned
                && record.delivery.prepared_receipt_digest.is_some()
                && record.delivery.dispatch_started_at_ms.is_none()
                && record.delivery.result_digest.is_none()
                && record.delivery.safe_error_code.is_none()
        }
        EffectDeliveryState::DispatchStarted | EffectDeliveryState::InFlight => {
            owned
                && record.delivery.prepared_receipt_digest.is_some()
                && record.delivery.dispatch_started_at_ms.is_some()
                && record.delivery.result_digest.is_none()
                && record.delivery.safe_error_code.is_none()
        }
        EffectDeliveryState::OutcomeRecorded => {
            owned
                && record.delivery.prepared_receipt_digest.is_some()
                && record.delivery.dispatch_started_at_ms.is_some()
                && record.delivery.result_digest.is_some()
        }
        EffectDeliveryState::Settled => {
            record.delivery.owner_incarnation_id.is_none()
                && record.delivery.lease_id.is_none()
                && record.delivery.result_digest.is_some()
        }
        EffectDeliveryState::Cancelled | EffectDeliveryState::ReviewRequired => {
            record.delivery.owner_incarnation_id.is_some() == record.delivery.lease_id.is_some()
        }
    };
    if record.effect.id != record.delivery.effect_id
        || record.effect.payload_sha256 != record.delivery.payload_sha256
        || !is_sha256(&record.effect.payload_sha256)
        || record.updated_at_ms < record.created_at_ms
        || !state_valid
        || record
            .delivery
            .prepared_receipt_digest
            .as_deref()
            .is_some_and(|value| !is_sha256(value))
        || record
            .delivery
            .result_digest
            .as_deref()
            .is_some_and(|value| !is_sha256(value))
        || record
            .delivery
            .safe_error_code
            .as_deref()
            .is_some_and(|value| !valid_reason_code(value))
        || record
            .delivery
            .dispatch_started_at_ms
            .is_some_and(|value| value < record.created_at_ms || value > record.updated_at_ms)
    {
        return Err(data_error("orchestration effect record is malformed"));
    }
    Ok(())
}

fn load_effects(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Vec<StoredOrchestrationEffect>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT effect_id, source_event_id, effect_bytes, payload_sha256,
                delivery_state, owner_incarnation_id, lease_id, claim_revision,
                prepared_receipt_digest, dispatch_started_at_ms, result_digest,
                safe_error_code, created_at_ms, updated_at_ms
           FROM orchestration_effects WHERE run_id = ?1 ORDER BY created_at_ms, effect_id",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<i64>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?,
            row.get::<_, i64>(12)?,
            row.get::<_, i64>(13)?,
        ))
    })?;
    let mut effects = Vec::new();
    for row in rows {
        let (
            effect_id,
            source_event_id,
            effect_bytes,
            payload_sha256,
            delivery_state,
            owner_incarnation_id,
            lease_id,
            claim_revision,
            prepared_receipt_digest,
            dispatch_started_at_ms,
            result_digest,
            safe_error_code,
            created_at_ms,
            updated_at_ms,
        ) = row?;
        let effect: OrchestrationEffect = serde_json::from_slice(&effect_bytes)?;
        if effect.id.as_str() != effect_id
            || effect.source_event_id.as_str() != source_event_id
            || effect.payload_sha256 != payload_sha256
        {
            return Err(data_error("orchestration effect row metadata mismatch"));
        }
        let record = StoredOrchestrationEffect {
            run_id: run_id.clone(),
            delivery: EffectDeliveryRecord {
                effect_id: effect.id.clone(),
                payload_sha256,
                state: parse_enum(&delivery_state, "effect delivery state")?,
                owner_incarnation_id: owner_incarnation_id.map(ServerIncarnationId::from),
                lease_id: lease_id.map(ExecutionLeaseId::from),
                claim_revision: checked_u64(claim_revision, "effect claim revision")?,
                prepared_receipt_digest,
                dispatch_started_at_ms: dispatch_started_at_ms
                    .map(|value| checked_u64(value, "effect dispatch timestamp"))
                    .transpose()?,
                result_digest,
                safe_error_code,
            },
            effect,
            created_at_ms: checked_u64(created_at_ms, "effect created at")?,
            updated_at_ms: checked_u64(updated_at_ms, "effect updated at")?,
        };
        validate_effect_record(&record)?;
        effects.push(record);
    }
    Ok(effects)
}

fn load_effect_by_id(
    conn: &rusqlite::Connection,
    effect_id: &OrchestrationEffectId,
) -> Result<Option<OrchestrationEffect>, StoreError> {
    conn.query_row(
        "SELECT effect_bytes FROM orchestration_effects WHERE effect_id = ?1",
        [effect_id.as_str()],
        |row| row.get::<_, Vec<u8>>(0),
    )
    .optional()?
    .map(|bytes| serde_json::from_slice(&bytes).map_err(StoreError::from))
    .transpose()
}

fn verify_effect_rows(
    derived: &[OrchestrationEffect],
    stored: &[StoredOrchestrationEffect],
) -> Result<(), StoreError> {
    if derived.len() != stored.len() {
        return Err(data_error(
            "derived effect count differs from durable effect rows",
        ));
    }
    for effect in derived {
        let Some(record) = stored.iter().find(|record| record.effect.id == effect.id) else {
            return Err(data_error("derived effect is missing from durable rows"));
        };
        if &record.effect != effect {
            return Err(data_error(
                "derived effect differs from durable effect payload",
            ));
        }
    }
    Ok(())
}

fn insert_lease(tx: &Transaction<'_>, lease: &ExecutionLease) -> Result<(), StoreError> {
    if lease.id.as_str().is_empty()
        || lease.resource_id.trim().is_empty()
        || lease.owner_incarnation_id.as_str().is_empty()
        || lease.revision == 0
        || lease.expires_at_ms < lease.last_heartbeat_ms
    {
        return Err(data_error("execution lease is malformed"));
    }
    tx.execute(
        "INSERT INTO orchestration_leases (
            lease_id, resource_kind, resource_id, owner_incarnation_id,
            expires_at_ms, last_heartbeat_ms, revision
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            lease.id.as_str(),
            enum_text(lease.resource_kind)?,
            lease.resource_id,
            lease.owner_incarnation_id.as_str(),
            checked_i64(lease.expires_at_ms, "lease expiry")?,
            checked_i64(lease.last_heartbeat_ms, "lease heartbeat")?,
            checked_i64(lease.revision, "lease revision")?,
        ],
    )?;
    Ok(())
}

fn insert_attempt_dispatch(
    tx: &Transaction<'_>,
    record: &AttemptDispatchRecord,
) -> Result<(), StoreError> {
    let selected_provider = record.route.provider.selected_provider_instance_id.as_ref();
    if record.attempt_id.as_str().is_empty()
        || record.run_id.as_str().is_empty()
        || record.task_id.as_str().is_empty()
        || record.route.task_id != record.task_id
        || selected_provider != Some(&record.provider_instance_id)
        || !is_sha256(&record.bundle_sha)
        || record.updated_at_ms < record.created_at_ms
        || record.last_progress_at_ms < record.created_at_ms
        || record
            .cancel_requested_at_ms
            .is_some_and(|value| value < record.created_at_ms)
        || (record.state == AttemptDispatchState::Ambiguous
            && record.ambiguity_reason.as_deref().is_none_or(str::is_empty))
        || (record.state == AttemptDispatchState::Terminal
            && record
                .outcome_digest
                .as_deref()
                .is_none_or(|value| !is_sha256(value)))
        || record
            .outcome_digest
            .as_deref()
            .is_some_and(|value| !is_sha256(value))
    {
        return Err(data_error("attempt dispatch record is malformed"));
    }
    let effect_bytes = tx.query_row(
        "SELECT effect_bytes FROM orchestration_effects
          WHERE effect_id = ?1 AND run_id = ?2",
        params![record.effect_id.as_str(), record.run_id.as_str()],
        |row| row.get::<_, Vec<u8>>(0),
    )?;
    let effect: OrchestrationEffect = serde_json::from_slice(&effect_bytes)?;
    if !matches!(
        effect.kind,
        pandamux_core::OrchestrationEffectKind::DispatchTask {
            ref run_id,
            ref task_id,
            ref attempt_id,
            ref route,
        } if run_id == &record.run_id
            && task_id == &record.task_id
            && attempt_id == &record.attempt_id
            && route == &record.route
    ) {
        return Err(data_error(
            "attempt dispatch does not reference its exact provider effect",
        ));
    }
    let snapshot = load_snapshot(tx, &record.run_id)?;
    if snapshot.bundle_sha != record.bundle_sha
        || snapshot
            .attempts
            .get(&record.attempt_id)
            .is_none_or(|canonical| canonical.route != record.route)
    {
        return Err(data_error(
            "attempt dispatch differs from the canonical run snapshot",
        ));
    }
    let route_bytes = serde_json::to_vec(&record.route)?;
    tx.execute(
        "INSERT INTO orchestration_attempt_dispatch (
            attempt_id, run_id, task_id, effect_id, thread_id, turn_id,
            turn_attempt_id, provider_instance_id, route_bytes, route_sha256,
            bundle_sha, state, last_boundary, last_progress_at_ms,
            cancel_requested_at_ms, ambiguity_reason, outcome_digest,
            created_at_ms, updated_at_ms
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
         )",
        params![
            record.attempt_id.as_str(),
            record.run_id.as_str(),
            record.task_id.as_str(),
            record.effect_id.as_str(),
            record.thread_id.as_str(),
            record.turn_id.as_str(),
            record.turn_attempt_id.as_str(),
            record.provider_instance_id.as_str(),
            route_bytes,
            sha256_hex(&route_bytes),
            record.bundle_sha,
            enum_text(record.state)?,
            attempt_boundary_text(record.last_boundary),
            checked_i64(record.last_progress_at_ms, "attempt progress timestamp")?,
            record
                .cancel_requested_at_ms
                .map(|value| checked_i64(value, "attempt cancellation timestamp"))
                .transpose()?,
            record.ambiguity_reason,
            record.outcome_digest,
            checked_i64(record.created_at_ms, "attempt dispatch created at")?,
            checked_i64(record.updated_at_ms, "attempt dispatch updated at")?,
        ],
    )?;
    Ok(())
}

fn validate_local_attempt_preparation_rows(
    tx: &Transaction<'_>,
    record: &AttemptDispatchRecord,
) -> Result<(), StoreError> {
    let snapshot = load_snapshot(tx, &record.run_id)?;
    let attempt = snapshot
        .attempts
        .get(&record.attempt_id)
        .ok_or_else(|| data_error("prepared local attempt is absent from the run"))?;
    if attempt.task_id != record.task_id
        || attempt.route != record.route
        || attempt.status != AttemptStatus::DispatchRequested
        || attempt.worktree_id.is_none()
        || attempt.thread_id.is_some()
        || attempt.turn_attempt_id.is_some()
    {
        return Err(data_error(
            "prepared local attempt differs from the canonical run",
        ));
    }

    let (worktree_id, worktree_path, worktree_environment): (String, String, String) = tx
        .query_row(
            "SELECT worktree_id, path, environment_id
               FROM orchestration_worktrees
              WHERE run_id = ?1 AND task_id = ?2 AND attempt_id = ?3",
            params![
                record.run_id.as_str(),
                record.task_id.as_str(),
                record.attempt_id.as_str(),
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| data_error("prepared local attempt has no owned worktree"))?;
    if attempt.worktree_id.as_ref().map(AttemptWorktreeId::as_str) != Some(worktree_id.as_str()) {
        return Err(data_error(
            "prepared local attempt worktree identity changed",
        ));
    }

    let queue_matches: i64 = tx.query_row(
        "SELECT COUNT(*) FROM orchestration_queue
          WHERE run_id = ?1 AND task_id = ?2 AND attempt_id = ?3
            AND provider_instance_id = ?4 AND environment_id = ?5
            AND state = 'claimed' AND owner_incarnation_id IS NOT NULL
            AND lease_id IS NOT NULL",
        params![
            record.run_id.as_str(),
            record.task_id.as_str(),
            record.attempt_id.as_str(),
            record.provider_instance_id.as_str(),
            worktree_environment,
        ],
        |row| row.get(0),
    )?;
    let budget_matches: i64 = tx.query_row(
        "SELECT COUNT(*) FROM orchestration_budget_reservations
          WHERE run_id = ?1 AND task_id = ?2 AND attempt_id = ?3
            AND state = 'reserved'",
        params![
            record.run_id.as_str(),
            record.task_id.as_str(),
            record.attempt_id.as_str(),
        ],
        |row| row.get(0),
    )?;
    if queue_matches != 1 || budget_matches != 1 {
        return Err(data_error(
            "prepared local attempt lacks its active admission or budget",
        ));
    }

    let (workspace_json, origin_json, thread_environment, thread_provider): (
        String,
        String,
        String,
        String,
    ) = tx
        .query_row(
            "SELECT workspace_json, origin_json, environment_id, provider_instance_id
               FROM threads WHERE id = ?1",
            [record.thread_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| data_error("prepared local thread was not persisted"))?;
    let workspace: pandamux_core::ThreadWorkspace = serde_json::from_str(&workspace_json)?;
    let origin: pandamux_core::ThreadOrigin = serde_json::from_str(&origin_json)?;
    if workspace.effective_path() != worktree_path
        || thread_environment != worktree_environment
        || thread_provider != record.provider_instance_id.as_str()
        || !matches!(
            origin,
            pandamux_core::ThreadOrigin::Orchestrator {
                ref run_id,
                ref task_id,
            } if run_id == &record.run_id && task_id == record.task_id.as_str()
        )
    {
        return Err(data_error(
            "prepared local thread differs from its owned run worktree",
        ));
    }

    let turn_matches: i64 = tx.query_row(
        "SELECT COUNT(*) FROM turns
          WHERE id = ?1 AND thread_id = ?2 AND environment_id = ?3
            AND provider_instance_id = ?4",
        params![
            record.turn_id.as_str(),
            record.thread_id.as_str(),
            worktree_environment,
            record.provider_instance_id.as_str(),
        ],
        |row| row.get(0),
    )?;
    let turn_attempt_matches: i64 = tx.query_row(
        "SELECT COUNT(*) FROM turn_attempts
          WHERE id = ?1 AND turn_id = ?2 AND thread_id = ?3
            AND environment_id = ?4 AND provider_instance_id = ?5
            AND boundary = ?6 AND status = ?7",
        params![
            record.turn_attempt_id.as_str(),
            record.turn_id.as_str(),
            record.thread_id.as_str(),
            worktree_environment,
            record.provider_instance_id.as_str(),
            serde_json::to_string(&AttemptBoundary::SafeToRetry)?,
            serde_json::to_string(&TurnAttemptStatus::Running)?,
        ],
        |row| row.get(0),
    )?;
    if turn_matches != 1 || turn_attempt_matches != 1 {
        return Err(data_error(
            "prepared local turn identities were not persisted exactly",
        ));
    }
    Ok(())
}

fn validate_remote_attempt_preparation_rows(
    tx: &Transaction<'_>,
    record: &AttemptDispatchRecord,
    prepared: &OrchestrationAttemptPrepareResult,
) -> Result<(), StoreError> {
    let snapshot = load_snapshot(tx, &record.run_id)?;
    let attempt = snapshot
        .attempts
        .get(&record.attempt_id)
        .ok_or_else(|| data_error("prepared remote attempt is absent from the run"))?;
    let receipt = &prepared.receipt;
    if attempt.task_id != record.task_id
        || attempt.route != record.route
        || attempt.status != AttemptStatus::DispatchRequested
        || attempt.thread_id.is_some()
        || attempt.turn_attempt_id.is_some()
        || receipt.state != OrchestrationNodeAttemptState::Prepared
        || receipt.run_id != record.run_id
        || receipt.task_id != record.task_id
        || receipt.attempt_id != record.attempt_id
        || receipt.effect_id != record.effect_id
        || receipt.thread_id != record.thread_id
        || receipt.turn_id != record.turn_id
        || receipt.turn_attempt_id != record.turn_attempt_id
        || receipt.boundary != AttemptBoundary::SafeToRetry
        || receipt.terminal_status.is_some()
        || receipt.result_sha256.is_some()
        || receipt.safe_error_code.is_some()
        || prepared.prepared_execution.provider_instance_id != record.provider_instance_id
        || prepared.prepared_execution.prepared_at_ms != record.created_at_ms
    {
        return Err(data_error(
            "prepared remote attempt differs from the canonical run or node receipt",
        ));
    }

    let queue_matches = load_queue_records(tx, &record.run_id)?
        .into_iter()
        .filter(|queue| {
            queue.task_id == record.task_id
                && queue.attempt_id == record.attempt_id
                && queue.provider_instance_id == record.provider_instance_id
                && queue.environment_id == receipt.environment_id
                && queue.state == QueueState::Claimed
                && queue.owner_incarnation_id.is_some()
                && queue.lease_id.is_some()
        })
        .count();
    let budget_matches = load_budget_reservations(tx, &record.run_id)?
        .into_iter()
        .filter(|reservation| {
            reservation.reservation.task_id == record.task_id
                && reservation.reservation.attempt_id == record.attempt_id
                && reservation.reservation.state == BudgetReservationState::Reserved
        })
        .count();
    if queue_matches != 1 || budget_matches != 1 {
        return Err(data_error(
            "prepared remote attempt lacks its active admission or budget",
        ));
    }

    let mut matching_receipts = 0usize;
    for stored in load_node_receipts(tx, &record.run_id)? {
        if stored.request_kind != NodeReceiptKind::AttemptPrepare
            || stored.node_store_id != receipt.node_store_id
            || stored.task_id != record.task_id
            || stored.attempt_id != record.attempt_id
            || stored.effect_id != record.effect_id
        {
            continue;
        }
        let decoded: OrchestrationAttemptPrepareResult =
            serde_json::from_slice(&stored.result_bytes)?;
        if decoded == *prepared {
            matching_receipts += 1;
        }
    }
    if matching_receipts != 1 {
        return Err(data_error(
            "prepared remote attempt lacks one exact durable node receipt",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn update_attempt_state_for_effect(
    tx: &Transaction<'_>,
    effect_id: &OrchestrationEffectId,
    expected_state: AttemptDispatchState,
    next_state: AttemptDispatchState,
    boundary: AttemptBoundary,
    ambiguity_reason: Option<&str>,
    outcome_digest: Option<&str>,
    updated_at_ms: u64,
) -> Result<(), StoreError> {
    let changed = tx.execute(
        "UPDATE orchestration_attempt_dispatch
            SET state = ?1, last_boundary = ?2, last_progress_at_ms = ?3,
                ambiguity_reason = ?4, outcome_digest = ?5, updated_at_ms = ?3
          WHERE effect_id = ?6 AND state = ?7 AND updated_at_ms <= ?3",
        params![
            enum_text(next_state)?,
            attempt_boundary_text(boundary),
            checked_i64(updated_at_ms, "attempt dispatch update timestamp")?,
            ambiguity_reason,
            outcome_digest,
            effect_id.as_str(),
            enum_text(expected_state)?,
        ],
    )?;
    if changed != 1 {
        return Err(data_error(
            "attempt dispatch update lost its compare-and-swap",
        ));
    }
    Ok(())
}

fn insert_queue_record(
    tx: &Transaction<'_>,
    record: &OrchestrationQueueRecord,
) -> Result<(), StoreError> {
    if record.queue_id.trim().is_empty()
        || !is_sha256(&record.candidate_sha256)
        || record.enqueued_at_ms < record.task_enqueued_at_ms
        || record.not_before_ms < record.enqueued_at_ms
        || (record.owner_incarnation_id.is_none() != record.lease_id.is_none())
        || record.interactive == record.agent_concurrency_limit.is_some()
        || (!record.interactive && record.organization_id.is_none())
        || record
            .agent_concurrency_limit
            .is_some_and(|limit| limit == 0)
        || (record.claimed_at_ms.is_none() != record.claim_order.is_none())
        || (record.claimed_at_ms.is_none() != record.interactive_streak.is_none())
        || record
            .claimed_at_ms
            .is_some_and(|claimed| claimed < record.enqueued_at_ms)
        || record.claim_order == Some(0)
        || record.interactive_streak.is_some_and(|streak| streak > 4)
        || match record.state {
            QueueState::Queued | QueueState::Cancelled => {
                record.owner_incarnation_id.is_some() || record.lease_id.is_some()
            }
            QueueState::Claimed => {
                record.owner_incarnation_id.is_none()
                    || record.lease_id.is_none()
                    || record.claimed_at_ms.is_none()
            }
            QueueState::Dispatched => {
                record.owner_incarnation_id.is_some() && record.claimed_at_ms.is_none()
            }
        }
    {
        return Err(data_error("orchestration queue record is malformed"));
    }
    if !record.interactive {
        if let Some(snapshot) = load_snapshot_optional(tx, &record.run_id)? {
            let task = snapshot
                .tasks
                .get(&record.task_id)
                .ok_or_else(|| data_error("queue record has no canonical task"))?;
            let route = snapshot
                .routes
                .get(&record.task_id)
                .ok_or_else(|| data_error("queue record has no canonical task route"))?;
            let task_state_matches = task.status == pandamux_core::RunTaskStatus::Ready
                || (task.status == pandamux_core::RunTaskStatus::DispatchRequested
                    && snapshot
                        .attempts
                        .get(&record.attempt_id)
                        .is_some_and(|attempt| attempt.task_id == record.task_id));
            let existing_attempt = snapshot.attempts.get(&record.attempt_id);
            let prior_attempt_count = snapshot
                .attempts
                .values()
                .filter(|attempt| attempt.task_id == record.task_id)
                .count();
            let route_identity_matches = existing_attempt.map_or_else(
                || {
                    route.selected_agent_id.as_ref() == Some(&record.agent_id)
                        && (prior_attempt_count > 0
                            || route.provider.selected_provider_instance_id.as_ref()
                                == Some(&record.provider_instance_id))
                },
                |attempt| {
                    attempt.route.selected_agent_id.as_ref() == Some(&record.agent_id)
                        && attempt
                            .route
                            .provider
                            .selected_provider_instance_id
                            .as_ref()
                            == Some(&record.provider_instance_id)
                },
            );
            if !task_state_matches
                || Some(&snapshot.organization_id) != record.organization_id.as_ref()
                || !route_identity_matches
            {
                return Err(data_error(
                    "orchestration queue record differs from the canonical run",
                ));
            }
        } else {
            validate_remote_queue_binding(tx, record)?;
        }
    }
    tx.execute(
        "INSERT INTO orchestration_queue (
            queue_id, run_id, task_id, attempt_id, agent_id, organization_id,
            agent_concurrency_limit, provider_instance_id, environment_id,
            candidate_sha256, priority, task_enqueued_at_ms, enqueued_at_ms,
            not_before_ms, state, owner_incarnation_id, lease_id, claim_revision,
            interactive, claimed_at_ms, claim_order, interactive_streak
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22
         )",
        params![
            record.queue_id,
            record.run_id.as_str(),
            record.task_id.as_str(),
            record.attempt_id.as_str(),
            record.agent_id.as_str(),
            record.organization_id.as_ref().map(OrganizationId::as_str),
            record.agent_concurrency_limit.map(i64::from),
            record.provider_instance_id.as_str(),
            record.environment_id.as_str(),
            record.candidate_sha256,
            record.priority,
            checked_i64(record.task_enqueued_at_ms, "queue task age")?,
            checked_i64(record.enqueued_at_ms, "queue timestamp")?,
            checked_i64(record.not_before_ms, "queue not-before")?,
            enum_text(record.state)?,
            record
                .owner_incarnation_id
                .as_ref()
                .map(ServerIncarnationId::as_str),
            record.lease_id.as_ref().map(ExecutionLeaseId::as_str),
            checked_i64(record.claim_revision, "queue claim revision")?,
            record.interactive,
            record
                .claimed_at_ms
                .map(|value| checked_i64(value, "queue claimed timestamp"))
                .transpose()?,
            record
                .claim_order
                .map(|value| checked_i64(value, "queue claim order"))
                .transpose()?,
            record.interactive_streak.map(i64::from),
        ],
    )?;
    Ok(())
}

fn validate_remote_queue_binding(
    conn: &rusqlite::Connection,
    record: &OrchestrationQueueRecord,
) -> Result<(), StoreError> {
    let prepared = remote_prepare_result_for_queue(conn, record)?
        .ok_or_else(|| data_error("remote queue record has no durable node prepare receipt"))?;
    if prepared.prepared_execution.agent_id != record.agent_id
        || Some(&prepared.prepared_execution.organization_id) != record.organization_id.as_ref()
        || prepared.prepared_execution.provider_instance_id != record.provider_instance_id
        || prepared.receipt.environment_id != record.environment_id
        || prepared.prepared_execution.agent_concurrency_limit
            != record.agent_concurrency_limit.unwrap_or(0)
    {
        return Err(data_error(
            "remote queue record differs from its durable node preparation",
        ));
    }
    Ok(())
}

fn remote_prepare_result_for_queue(
    conn: &rusqlite::Connection,
    record: &OrchestrationQueueRecord,
) -> Result<Option<OrchestrationAttemptPrepareResult>, StoreError> {
    let mut matching: Option<OrchestrationAttemptPrepareResult> = None;
    for receipt in load_node_receipts_for_attempt(conn, &record.attempt_id)? {
        if receipt.request_kind != NodeReceiptKind::AttemptPrepare {
            continue;
        }
        let result: OrchestrationAttemptPrepareResult =
            serde_json::from_slice(&receipt.result_bytes)?;
        let prepared_sha = sha256_hex(&serde_json::to_vec(&result.prepared_execution)?);
        if result.receipt.schema_version
            != pandamux_protocol::ORCHESTRATION_EXECUTION_SCHEMA_VERSION
            || result.receipt.state != OrchestrationNodeAttemptState::Prepared
            || result.receipt.node_store_id != receipt.node_store_id
            || result.receipt.request_sha256.as_str() != receipt.request_sha256
            || result.receipt.run_id != record.run_id
            || result.receipt.task_id != record.task_id
            || result.receipt.attempt_id != record.attempt_id
            || result.receipt.effect_id != receipt.effect_id
            || result.receipt.prepared_execution_sha256.as_str() != prepared_sha
            || result.prepared_execution.run_concurrency_limit == 0
            || result.prepared_execution.agent_concurrency_limit == 0
        {
            return Err(data_error("remote node prepare receipt is malformed"));
        }
        if let Some(existing) = matching.as_ref()
            && existing != &result
        {
            return Err(data_error(
                "remote attempt has conflicting durable prepare receipts",
            ));
        }
        matching = Some(result);
    }
    Ok(matching)
}

fn insert_worktree(
    tx: &Transaction<'_>,
    record: &OwnedAttemptWorktreeRecord,
) -> Result<(), StoreError> {
    let worktree = &record.worktree;
    if worktree.id.as_str().is_empty()
        || worktree.git_common_dir_identity.trim().is_empty()
        || worktree.baseline_commit.trim().is_empty()
        || worktree.branch_ref.trim().is_empty()
        || worktree.path.trim().is_empty()
        || record.updated_at_ms < record.created_at_ms
        || record
            .checkpoint_intent_sha256
            .iter()
            .any(|(checkpoint_id, digest)| {
                !worktree.checkpoint_ids.contains(checkpoint_id) || !is_sha256(digest)
            })
        || record
            .verified_process_death_sha256
            .as_deref()
            .is_some_and(|digest| !is_sha256(digest))
        || record
            .retained_evidence_sha256
            .as_deref()
            .is_some_and(|digest| !is_sha256(digest))
    {
        return Err(data_error("owned worktree record is malformed"));
    }
    let snapshot = load_snapshot(tx, &worktree.run_id)?;
    if snapshot
        .attempts
        .get(&worktree.attempt_id)
        .is_none_or(|attempt| {
            attempt.task_id != worktree.task_id
                || attempt.worktree_id.as_ref() != Some(&worktree.id)
        })
    {
        return Err(data_error(
            "owned worktree differs from the canonical attempt owner",
        ));
    }
    let repository_id = tx.query_row(
        "SELECT repository_id FROM orchestration_runs WHERE run_id = ?1",
        [worktree.run_id.as_str()],
        |row| row.get::<_, String>(0),
    )?;
    if repository_id != worktree.repository_id.as_str() {
        return Err(data_error(
            "owned worktree differs from the canonical run repository",
        ));
    }
    let lease = load_lease(tx, &worktree.lease_id)?
        .ok_or_else(|| data_error("owned worktree lease was not found"))?;
    if lease.resource_kind != LeaseResourceKind::Worktree
        || lease.resource_id != worktree.id.as_str()
    {
        return Err(data_error(
            "owned worktree lease differs from its canonical resource",
        ));
    }
    let checkpoints = StoredWorktreeCheckpoints {
        ids: worktree.checkpoint_ids.clone(),
        intent_sha256: record.checkpoint_intent_sha256.clone(),
        verified_process_death_sha256: record.verified_process_death_sha256.clone(),
        retained_evidence_sha256: record.retained_evidence_sha256.clone(),
    };
    tx.execute(
        "INSERT INTO orchestration_worktrees (
            worktree_id, repository_id, run_id, task_id, attempt_id, environment_id,
            git_common_dir_identity, baseline_commit, branch_ref, path, lease_id,
            checkpoint_ids_json, dirty_state, cleanup_eligibility,
            created_at_ms, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            worktree.id.as_str(),
            worktree.repository_id.as_str(),
            worktree.run_id.as_str(),
            worktree.task_id.as_str(),
            worktree.attempt_id.as_str(),
            worktree.environment_id.as_str(),
            worktree.git_common_dir_identity,
            worktree.baseline_commit,
            worktree.branch_ref,
            worktree.path,
            worktree.lease_id.as_str(),
            serde_json::to_string(&checkpoints)?,
            enum_text(worktree.dirty_state)?,
            enum_text(worktree.cleanup_eligibility)?,
            checked_i64(record.created_at_ms, "worktree created at")?,
            checked_i64(record.updated_at_ms, "worktree updated at")?,
        ],
    )?;
    Ok(())
}

fn insert_budget_reservation(
    tx: &Transaction<'_>,
    record: &StoredBudgetReservation,
) -> Result<(), StoreError> {
    let reservation = &record.reservation;
    if reservation.id.as_str().is_empty()
        || reservation.task_id.as_str().is_empty()
        || reservation.attempt_id.as_str().is_empty()
        || !valid_utc_date(&reservation.ledger_utc_date)
        || amount_exceeds_sqlite(&record.agent_budget_limit)
        || record.updated_at_ms < record.created_at_ms
        || !valid_budget_reconciliation(record)
        || record.grants.values().any(|grant| {
            grant.run_id != record.run_id
                || grant.task_id != reservation.task_id
                || grant.attempt_id != reservation.attempt_id
                || grant.amount == 0
                || !is_sha256(&grant.action_digest)
        })
    {
        return Err(data_error("budget reservation record is malformed"));
    }
    let snapshot = load_snapshot(tx, &record.run_id)?;
    if snapshot.reservations.get(&reservation.id) != Some(reservation)
        || snapshot.organization_id != record.organization_id
        || grants_for_attempt_at(
            &snapshot,
            &reservation.attempt_id,
            Some(record.created_at_ms),
        ) != record.grants
    {
        return Err(data_error(
            "budget reservation differs from the canonical run snapshot",
        ));
    }
    tx.execute(
        "INSERT INTO orchestration_budget_reservations (
            reservation_id, run_id, task_id, attempt_id, agent_id, organization_id,
            agent_limit_tokens, agent_limit_micro_usd, agent_limit_runtime_ms,
            ledger_utc_date, reserved_tokens, reserved_micro_usd, reserved_runtime_ms,
            actual_tokens, actual_micro_usd, actual_runtime_ms, state, grants_json,
            reconciliation_json, created_at_ms, updated_at_ms
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
         )",
        params![
            reservation.id.as_str(),
            record.run_id.as_str(),
            reservation.task_id.as_str(),
            reservation.attempt_id.as_str(),
            record.agent_id.as_str(),
            record.organization_id.as_str(),
            checked_i64(record.agent_budget_limit.tokens, "agent token limit")?,
            checked_i64(record.agent_budget_limit.cost_micro_usd, "agent cost limit")?,
            checked_i64(record.agent_budget_limit.runtime_ms, "agent runtime limit")?,
            reservation.ledger_utc_date,
            checked_i64(reservation.amount.tokens, "reserved tokens")?,
            checked_i64(reservation.amount.cost_micro_usd, "reserved cost")?,
            checked_i64(reservation.amount.runtime_ms, "reserved runtime")?,
            reservation
                .actual
                .tokens
                .map(|value| checked_i64(value, "actual tokens"))
                .transpose()?,
            reservation
                .actual
                .cost_micro_usd
                .map(|value| checked_i64(value, "actual cost"))
                .transpose()?,
            reservation
                .actual
                .runtime_ms
                .map(|value| checked_i64(value, "actual runtime"))
                .transpose()?,
            enum_text(reservation.state)?,
            serde_json::to_string(&record.grants)?,
            record
                .reconciliation
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            checked_i64(record.created_at_ms, "budget reservation created at")?,
            checked_i64(record.updated_at_ms, "budget reservation updated at")?,
        ],
    )?;
    Ok(())
}

fn insert_daily_budget(tx: &Transaction<'_>, record: &DailyBudgetRecord) -> Result<(), StoreError> {
    if !valid_utc_date(&record.ledger_utc_date) || record.revision == 0 {
        return Err(data_error("daily budget record is malformed"));
    }
    tx.execute(
        "INSERT INTO orchestration_daily_budgets (
            organization_id, ledger_utc_date, limit_micro_usd, reserved_micro_usd,
            reconciled_micro_usd, granted_micro_usd, revision, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            record.organization_id.as_str(),
            record.ledger_utc_date,
            checked_i64(record.limit_micro_usd, "daily budget limit")?,
            checked_i64(record.reserved_micro_usd, "daily budget reserved")?,
            checked_i64(record.reconciled_micro_usd, "daily budget reconciled")?,
            checked_i64(record.granted_micro_usd, "daily budget granted")?,
            checked_i64(record.revision, "daily budget revision")?,
            checked_i64(record.updated_at_ms, "daily budget updated at")?,
        ],
    )?;
    Ok(())
}

fn insert_artifact(
    tx: &Transaction<'_>,
    snapshot: &Run,
    artifact: &BlackboardEntry,
) -> Result<(), StoreError> {
    if artifact.id.as_str().is_empty()
        || artifact.run_id.as_str().is_empty()
        || artifact.artifact_key
            != pandamux_orchestrator::artifact_key(&artifact.producer, &artifact.output_contract)
        || artifact.schema_sha256
            != pandamux_orchestrator::output_contract_schema_sha256(&artifact.output_contract)
        || artifact.byte_len != u64::try_from(artifact.canonical_json.len()).unwrap_or(u64::MAX)
        || artifact.content_sha256 != sha256_hex(&artifact.canonical_json)
        || artifact.canonical_json.len() > MAX_ARTIFACT_BYTES
    {
        return Err(data_error(
            "blackboard artifact identity or digest is malformed",
        ));
    }
    let content: serde_json::Value = serde_json::from_slice(&artifact.canonical_json)?;
    if pandamux_orchestrator::canonical_artifact_json(&content) != artifact.canonical_json {
        return Err(data_error(
            "blackboard artifact bytes are not canonical JSON",
        ));
    }
    validate_json_bounds(&content, 0)?;
    pandamux_orchestrator::validate_output_value(&artifact.output_contract, &content)
        .map_err(|reason| data_error(format!("blackboard artifact contract mismatch: {reason}")))?;
    if snapshot.blackboard.get(&artifact.id) != Some(artifact) {
        return Err(data_error(
            "blackboard artifact differs from the canonical run snapshot",
        ));
    }
    let producer = snapshot
        .tasks
        .get(&artifact.producer)
        .ok_or_else(|| data_error("blackboard producer does not exist in the run"))?;
    let expected_consumers = snapshot
        .tasks
        .values()
        .filter(|consumer| {
            consumer.inputs.iter().any(|input| {
                input.producer_task_id.as_ref() == Some(&artifact.producer)
                    && input.contract == artifact.output_contract
            })
        })
        .map(|consumer| consumer.id.clone())
        .collect::<BTreeSet<_>>();
    if !producer.outputs.contains(&artifact.output_contract)
        || artifact.consumers != expected_consumers
    {
        return Err(data_error(
            "blackboard artifact scope differs from the run plan",
        ));
    }
    let task_count = tx.query_row(
        "SELECT COUNT(*) FROM orchestration_blackboard_artifacts
          WHERE run_id = ?1 AND producer_task_id = ?2",
        params![artifact.run_id.as_str(), artifact.producer.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    let run_count = tx.query_row(
        "SELECT COUNT(*) FROM orchestration_blackboard_artifacts WHERE run_id = ?1",
        [artifact.run_id.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    let run_bytes = tx.query_row(
        "SELECT COALESCE(SUM(byte_len), 0)
           FROM orchestration_blackboard_artifacts WHERE run_id = ?1",
        [artifact.run_id.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    if checked_u64(task_count, "task artifact count")? >= MAX_ARTIFACTS_PER_TASK as u64
        || checked_u64(run_count, "run artifact count")? >= MAX_ARTIFACTS_PER_RUN as u64
        || checked_u64(run_bytes, "run artifact bytes")?
            .checked_add(artifact.byte_len)
            .is_none_or(|value| value > MAX_RUN_ARTIFACT_BYTES as u64)
    {
        return Err(data_error("blackboard artifact limits are exceeded"));
    }
    let prior = tx
        .query_row(
            "SELECT entry_id, version
               FROM orchestration_blackboard_artifacts
              WHERE run_id = ?1 AND artifact_key = ?2
              ORDER BY version DESC LIMIT 1",
            params![artifact.run_id.as_str(), artifact.artifact_key.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    match prior {
        None if artifact.version == 1 && artifact.previous_entry_id.is_none() => {}
        Some((entry_id, version))
            if checked_u64(version, "artifact version")?.checked_add(1)
                == Some(artifact.version)
                && artifact
                    .previous_entry_id
                    .as_ref()
                    .map(BlackboardEntryId::as_str)
                    == Some(entry_id.as_str()) => {}
        _ => return Err(data_error("blackboard artifact version chain is invalid")),
    }
    tx.execute(
        "INSERT INTO orchestration_blackboard_artifacts (
            entry_id, run_id, artifact_key, version, previous_entry_id,
            producer_task_id, consumers_json, output_contract_json, schema_sha256,
            canonical_json, byte_len, content_sha256, created_event_id, provenance_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            artifact.id.as_str(),
            artifact.run_id.as_str(),
            artifact.artifact_key.as_str(),
            checked_i64(artifact.version, "artifact version")?,
            artifact
                .previous_entry_id
                .as_ref()
                .map(BlackboardEntryId::as_str),
            artifact.producer.as_str(),
            serde_json::to_string(&artifact.consumers)?,
            serde_json::to_string(&artifact.output_contract)?,
            artifact.schema_sha256,
            artifact.canonical_json,
            checked_i64(artifact.byte_len, "artifact byte length")?,
            artifact.content_sha256,
            artifact.created_event_id.as_str(),
            artifact
                .provenance
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        ],
    )?;
    Ok(())
}

fn insert_progress(
    tx: &Transaction<'_>,
    progress: &AttemptProgressRecord,
) -> Result<(), StoreError> {
    if !is_sha256(&progress.payload_digest)
        || (progress.kind == AttemptProgressKind::StreamBytes)
            != progress.cumulative_stream_bytes.is_some()
        || (progress.kind == AttemptProgressKind::TerminalOutcome) != progress.terminal
    {
        return Err(data_error("attempt progress record is malformed"));
    }
    let owner = tx
        .query_row(
            "SELECT run_id, task_id FROM orchestration_attempt_dispatch WHERE attempt_id = ?1",
            [progress.attempt_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    if owner.as_ref().is_none_or(|(run_id, task_id)| {
        run_id != progress.run_id.as_str() || task_id != progress.task_id.as_str()
    }) {
        return Err(data_error(
            "attempt progress ownership does not match dispatch",
        ));
    }
    tx.execute(
        "INSERT INTO orchestration_attempt_progress (
            progress_id, run_id, task_id, attempt_id, kind, payload_digest,
            at_ms, cumulative_stream_bytes, terminal
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            progress.id.as_str(),
            progress.run_id.as_str(),
            progress.task_id.as_str(),
            progress.attempt_id.as_str(),
            enum_text(progress.kind)?,
            progress.payload_digest,
            checked_i64(progress.at_ms, "attempt progress timestamp")?,
            progress
                .cumulative_stream_bytes
                .map(|value| checked_i64(value, "attempt progress bytes"))
                .transpose()?,
            progress.terminal,
        ],
    )?;
    Ok(())
}

fn insert_recovery_review(tx: &Transaction<'_>, review: &RecoveryReview) -> Result<(), StoreError> {
    if !valid_reason_code(&review.reason_code)
        || !is_sha256(&review.evidence_digest)
        || match review.status {
            RecoveryReviewStatus::Pending => {
                review.resolved_at_ms.is_some() || review.resolution.is_some()
            }
            RecoveryReviewStatus::Resolved => {
                review.resolved_at_ms.is_none()
                    || review.resolution.as_deref().is_none_or(str::is_empty)
            }
        }
    {
        return Err(data_error("recovery review record is malformed"));
    }
    let snapshot_owner_matches = load_snapshot(tx, &review.run_id)
        .ok()
        .and_then(|snapshot| snapshot.attempts.get(&review.attempt_id).cloned())
        .is_some_and(|attempt| attempt.task_id == review.task_id);
    let dispatch_owner_matches = tx
        .query_row(
            "SELECT run_id, task_id FROM orchestration_attempt_dispatch
              WHERE attempt_id = ?1",
            [review.attempt_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .is_some_and(|(run_id, task_id)| {
            run_id == review.run_id.as_str() && task_id == review.task_id.as_str()
        });
    if !snapshot_owner_matches && !dispatch_owner_matches {
        return Err(data_error(
            "recovery review differs from the canonical attempt owner",
        ));
    }
    if let Some(effect_id) = &review.effect_id {
        let effect = load_effect_by_id(tx, effect_id)?
            .ok_or_else(|| data_error("recovery review references a missing effect"))?;
        if !effect_matches_attempt(
            &effect,
            &review.run_id,
            &review.task_id,
            &review.attempt_id,
            None,
        ) {
            return Err(data_error(
                "recovery review differs from its canonical effect",
            ));
        }
    }
    tx.execute(
        "INSERT INTO orchestration_recovery_reviews (
            review_id, run_id, task_id, attempt_id, effect_id, reason_code,
            evidence_digest, status, created_at_ms, resolved_at_ms, resolution
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            review.id.as_str(),
            review.run_id.as_str(),
            review.task_id.as_str(),
            review.attempt_id.as_str(),
            review.effect_id.as_ref().map(OrchestrationEffectId::as_str),
            review.reason_code,
            review.evidence_digest,
            enum_text(review.status)?,
            checked_i64(review.created_at_ms, "recovery review created at")?,
            review
                .resolved_at_ms
                .map(|value| checked_i64(value, "recovery review resolved at"))
                .transpose()?,
            review.resolution,
        ],
    )?;
    Ok(())
}

fn insert_node_receipt(
    tx: &Transaction<'_>,
    receipt: &NodeReceiptRecord,
    require_local_effect: bool,
) -> Result<(), StoreError> {
    if receipt.receipt_id.trim().is_empty()
        || receipt.idempotency_key.trim().is_empty()
        || !is_sha256(&receipt.request_sha256)
        || !is_sha256(&receipt.result_sha256)
        || receipt.result_sha256 != sha256_hex(&receipt.result_bytes)
        || receipt.result_bytes.is_empty()
        || receipt.updated_at_ms < receipt.created_at_ms
    {
        return Err(data_error("node receipt is malformed"));
    }
    if require_local_effect {
        let effect = load_effect_by_id(tx, &receipt.effect_id)?
            .ok_or_else(|| data_error("node receipt references a missing effect"))?;
        if !effect_matches_attempt(
            &effect,
            &receipt.run_id,
            &receipt.task_id,
            &receipt.attempt_id,
            None,
        ) {
            return Err(data_error(
                "node receipt differs from its canonical provider effect",
            ));
        }
    }
    tx.execute(
        "INSERT INTO orchestration_node_receipts (
            receipt_id, node_store_id, run_id, task_id, attempt_id, effect_id,
            request_kind, idempotency_key, request_sha256, result_sha256,
            result_bytes, created_at_ms, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            receipt.receipt_id,
            receipt.node_store_id.as_str(),
            receipt.run_id.as_str(),
            receipt.task_id.as_str(),
            receipt.attempt_id.as_str(),
            receipt.effect_id.as_str(),
            enum_text(receipt.request_kind)?,
            receipt.idempotency_key,
            receipt.request_sha256,
            receipt.result_sha256,
            receipt.result_bytes,
            checked_i64(receipt.created_at_ms, "node receipt created at")?,
            checked_i64(receipt.updated_at_ms, "node receipt updated at")?,
        ],
    )?;
    Ok(())
}

impl Store {
    /// Persists a receipt returned by a remote node into the Hub store. The
    /// receipt must remain bound to the exact local provider effect.
    pub fn record_hub_node_receipt(
        &self,
        receipt: &NodeReceiptRecord,
    ) -> Result<NodeReceiptRecord, StoreError> {
        self.record_node_receipt(receipt, true)
    }

    /// Persists a receipt owned by this node. A node intentionally does not
    /// mirror the Hub's complete run/effect store, so the authenticated request
    /// digest is the local authority boundary.
    pub fn record_local_node_receipt(
        &self,
        receipt: &NodeReceiptRecord,
    ) -> Result<NodeReceiptRecord, StoreError> {
        self.record_node_receipt(receipt, false)
    }

    fn record_node_receipt(
        &self,
        receipt: &NodeReceiptRecord,
        require_local_effect: bool,
    ) -> Result<NodeReceiptRecord, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = load_node_receipt_by_key(
            &tx,
            &receipt.node_store_id,
            receipt.request_kind,
            &receipt.idempotency_key,
        )? {
            if existing == *receipt {
                return Ok(existing);
            }
            return Err(data_error(
                "node receipt idempotency identity was reused with different bytes",
            ));
        }
        insert_node_receipt(&tx, receipt, require_local_effect)?;
        let stored = load_node_receipt_by_key(
            &tx,
            &receipt.node_store_id,
            receipt.request_kind,
            &receipt.idempotency_key,
        )?
        .ok_or_else(|| data_error("node receipt disappeared after insertion"))?;
        tx.commit()?;
        Ok(stored)
    }

    pub fn get_node_receipt(
        &self,
        node_store_id: &NodeStoreId,
        request_kind: NodeReceiptKind,
        idempotency_key: &str,
    ) -> Result<Option<NodeReceiptRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_node_receipt_by_key(&conn, node_store_id, request_kind, idempotency_key)
    }

    pub fn list_node_receipts_for_attempt(
        &self,
        attempt_id: &RunAttemptId,
    ) -> Result<Vec<NodeReceiptRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_node_receipts_for_attempt(&conn, attempt_id)
    }
}

impl Store {
    pub fn load_orchestration_runtime_records(
        &self,
        run_id: &RunId,
    ) -> Result<OrchestrationRuntimeRecords, StoreError> {
        let conn = self.conn.lock().unwrap();
        let snapshot = load_snapshot(&conn, run_id)?;
        let events = load_events(&conn, run_id)?;
        let (replayed, derived_effects) = replay_with_effects(&events)?;
        if pandamux_orchestrator::canonical_run_bytes(&snapshot)
            != pandamux_orchestrator::canonical_run_bytes(&replayed)
        {
            return Err(data_error(
                "orchestration replay differs from stored snapshot",
            ));
        }
        let effects = load_effects(&conn, run_id)?;
        verify_effect_rows(&derived_effects, &effects)?;
        let attempt_dispatches = load_attempt_dispatches(&conn, run_id)?;
        let queue_records = load_queue_records(&conn, run_id)?;
        let worktrees = load_worktrees(&conn, run_id)?;
        let budget_reservations = load_budget_reservations(&conn, run_id)?;
        let daily_budgets = load_daily_budgets(&conn, &budget_reservations)?;
        let artifacts = load_artifacts(&conn, run_id)?;
        let progress = load_progress(&conn, run_id)?;
        let recovery_reviews = load_recovery_reviews(&conn, run_id)?;
        let node_receipts = load_node_receipts(&conn, run_id)?;
        let mut lease_ids = queue_records
            .iter()
            .filter_map(|record| record.lease_id.clone())
            .chain(
                worktrees
                    .iter()
                    .map(|record| record.worktree.lease_id.clone()),
            )
            .collect::<BTreeSet<_>>();
        validate_runtime_bindings(
            &snapshot,
            &effects,
            &attempt_dispatches,
            &queue_records,
            &worktrees,
            &budget_reservations,
            &artifacts,
            &progress,
            &recovery_reviews,
            &node_receipts,
        )?;
        for effect in effects {
            if let Some(lease_id) = effect.delivery.lease_id {
                lease_ids.insert(lease_id);
            }
        }
        let leases = lease_ids
            .iter()
            .map(|lease_id| {
                load_lease(&conn, lease_id)?.ok_or_else(|| {
                    data_error("an orchestration record references a missing execution lease")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(OrchestrationRuntimeRecords {
            attempt_dispatches,
            queue_records,
            leases,
            worktrees,
            budget_reservations,
            daily_budgets,
            artifacts,
            progress,
            recovery_reviews,
            node_receipts,
        })
    }

    /// Records only durable, meaningful attempt progress. Streaming bytes are
    /// coalesced until either one second or another 4 KiB has elapsed, and
    /// duplicate digests never advance the watchdog clock.
    pub fn record_attempt_progress(
        &self,
        progress: &AttemptProgressRecord,
    ) -> Result<AttemptProgressOutcome, StoreError> {
        const STREAM_INTERVAL_MS: u64 = 1_000;
        const STREAM_BYTES: u64 = 4 * 1024;
        const RETAINED_NONTERMINAL: i64 = 256;

        if !is_sha256(&progress.payload_digest)
            || (progress.kind == AttemptProgressKind::StreamBytes)
                != progress.cumulative_stream_bytes.is_some()
            || (progress.kind == AttemptProgressKind::TerminalOutcome) != progress.terminal
        {
            return Err(data_error("attempt progress record is malformed"));
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run_id = tx
            .query_row(
                "SELECT run_id FROM orchestration_attempt_dispatch WHERE attempt_id = ?1",
                [progress.attempt_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| data_error("attempt progress owner was not found"))?;
        let run_id = RunId::from(run_id);
        let dispatch = load_attempt_dispatches(&tx, &run_id)?
            .into_iter()
            .find(|record| record.attempt_id == progress.attempt_id)
            .ok_or_else(|| data_error("attempt progress owner disappeared"))?;
        if progress.run_id != dispatch.run_id || progress.task_id != dispatch.task_id {
            return Err(data_error("attempt progress ownership changed"));
        }

        if let Some(existing_run_id) = tx
            .query_row(
                "SELECT run_id FROM orchestration_attempt_progress WHERE progress_id = ?1",
                [progress.id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            let existing = load_progress(&tx, &RunId::from(existing_run_id))?
                .into_iter()
                .find(|record| record.id == progress.id)
                .ok_or_else(|| data_error("attempt progress identity disappeared"))?;
            let same_meaning = existing.run_id == progress.run_id
                && existing.task_id == progress.task_id
                && existing.attempt_id == progress.attempt_id
                && existing.kind == progress.kind
                && existing.payload_digest == progress.payload_digest
                && existing.cumulative_stream_bytes == progress.cumulative_stream_bytes
                && existing.terminal == progress.terminal;
            if !same_meaning {
                return Err(data_error(
                    "attempt progress identity was reused with different bytes",
                ));
            }
            return Ok(AttemptProgressOutcome {
                disposition: AttemptProgressDisposition::Duplicate,
                record: Some(existing),
                last_progress_at_ms: dispatch.last_progress_at_ms,
            });
        }

        if progress.at_ms < dispatch.last_progress_at_ms {
            return Err(data_error("attempt progress timestamp moved backwards"));
        }
        if progress.terminal
            && !matches!(
                dispatch.state,
                AttemptDispatchState::Terminal
                    | AttemptDispatchState::Cancelled
                    | AttemptDispatchState::Ambiguous
                    | AttemptDispatchState::NotStarted
            )
        {
            return Err(data_error(
                "terminal progress requires a terminal dispatch outcome",
            ));
        }
        let records = load_progress(&tx, &run_id)?
            .into_iter()
            .filter(|record| record.attempt_id == progress.attempt_id)
            .collect::<Vec<_>>();
        if records.iter().any(|record| {
            record.kind == progress.kind
                && record.payload_digest == progress.payload_digest
                && record.cumulative_stream_bytes == progress.cumulative_stream_bytes
                && record.terminal == progress.terminal
        }) {
            return Ok(AttemptProgressOutcome {
                disposition: AttemptProgressDisposition::Duplicate,
                record: None,
                last_progress_at_ms: dispatch.last_progress_at_ms,
            });
        }
        if records.iter().any(|record| record.terminal)
            && !matches!(
                progress.kind,
                AttemptProgressKind::CheckpointCreated | AttemptProgressKind::ArtifactPublished
            )
        {
            return Err(data_error(
                "terminal attempt progress accepts only post-terminal evidence",
            ));
        }
        if progress.kind == AttemptProgressKind::StreamBytes {
            let bytes = progress
                .cumulative_stream_bytes
                .expect("validated stream progress has bytes");
            if let Some(previous) = records
                .iter()
                .filter(|record| record.kind == AttemptProgressKind::StreamBytes)
                .max_by_key(|record| (record.at_ms, record.id.as_str()))
            {
                let previous_bytes = previous
                    .cumulative_stream_bytes
                    .expect("stored stream progress has bytes");
                if bytes <= previous_bytes {
                    return Err(data_error(
                        "stream progress did not advance persisted bytes",
                    ));
                }
                if progress.at_ms.saturating_sub(previous.at_ms) < STREAM_INTERVAL_MS
                    && bytes.saturating_sub(previous_bytes) < STREAM_BYTES
                {
                    return Ok(AttemptProgressOutcome {
                        disposition: AttemptProgressDisposition::Coalesced,
                        record: None,
                        last_progress_at_ms: dispatch.last_progress_at_ms,
                    });
                }
            }
        }

        insert_progress(&tx, progress)?;
        let changed = tx.execute(
            "UPDATE orchestration_attempt_dispatch
                SET last_progress_at_ms = ?1, updated_at_ms = max(updated_at_ms, ?1)
              WHERE attempt_id = ?2 AND run_id = ?3 AND task_id = ?4
                AND last_progress_at_ms <= ?1",
            params![
                checked_i64(progress.at_ms, "attempt progress timestamp")?,
                progress.attempt_id.as_str(),
                progress.run_id.as_str(),
                progress.task_id.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "attempt progress lost its durable owner compare-and-swap",
            ));
        }
        tx.execute(
            "DELETE FROM orchestration_attempt_progress
              WHERE progress_id IN (
                    SELECT progress_id FROM orchestration_attempt_progress
                     WHERE attempt_id = ?1 AND terminal = 0
                     ORDER BY at_ms DESC, progress_id DESC
                     LIMIT -1 OFFSET ?2
              )",
            params![progress.attempt_id.as_str(), RETAINED_NONTERMINAL],
        )?;
        tx.commit()?;
        Ok(AttemptProgressOutcome {
            disposition: AttemptProgressDisposition::Persisted,
            record: Some(progress.clone()),
            last_progress_at_ms: progress.at_ms,
        })
    }

    pub fn load_attempt_progress(
        &self,
        attempt_id: &RunAttemptId,
    ) -> Result<Vec<AttemptProgressRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let run_id = conn
            .query_row(
                "SELECT run_id FROM orchestration_attempt_dispatch WHERE attempt_id = ?1",
                [attempt_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(run_id) = run_id else {
            return Ok(Vec::new());
        };
        Ok(load_progress(&conn, &RunId::from(run_id))?
            .into_iter()
            .filter(|record| record.attempt_id == *attempt_id)
            .collect())
    }

    /// Persists cancellation intent before any provider interrupt is allowed.
    pub fn record_attempt_cancel_intent(
        &self,
        attempt_id: &RunAttemptId,
        requested_at_ms: u64,
    ) -> Result<AttemptDispatchRecord, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run_id = tx
            .query_row(
                "SELECT run_id FROM orchestration_attempt_dispatch WHERE attempt_id = ?1",
                [attempt_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| data_error("cancelled attempt dispatch was not found"))?;
        let run_id = RunId::from(run_id);
        let current = load_attempt_dispatches(&tx, &run_id)?
            .into_iter()
            .find(|record| record.attempt_id == *attempt_id)
            .ok_or_else(|| data_error("cancelled attempt dispatch disappeared"))?;
        if current.state == AttemptDispatchState::CancelRequested {
            return Ok(current);
        }
        if !matches!(
            current.state,
            AttemptDispatchState::DispatchStarted | AttemptDispatchState::Accepted
        ) || requested_at_ms < current.updated_at_ms
        {
            return Err(data_error("attempt cannot record cancellation intent"));
        }
        let changed = tx.execute(
            "UPDATE orchestration_attempt_dispatch
                SET state = 'cancel_requested', cancel_requested_at_ms = ?1,
                    last_progress_at_ms = max(last_progress_at_ms, ?1), updated_at_ms = ?1
              WHERE attempt_id = ?2 AND state = ?3 AND updated_at_ms = ?4",
            params![
                checked_i64(requested_at_ms, "attempt cancellation timestamp")?,
                attempt_id.as_str(),
                enum_text(current.state)?,
                checked_i64(current.updated_at_ms, "prior attempt timestamp")?,
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "attempt cancellation intent lost its compare-and-swap",
            ));
        }
        let updated = load_attempt_dispatches(&tx, &run_id)?
            .into_iter()
            .find(|record| record.attempt_id == *attempt_id)
            .ok_or_else(|| data_error("cancelled attempt dispatch disappeared"))?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn list_nonterminal_orchestration_run_ids(&self) -> Result<Vec<RunId>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT run_id FROM orchestration_runs
              WHERE status NOT IN ('succeeded', 'failed', 'cancelled')
              ORDER BY created_at_ms, run_id",
        )?;
        Ok(statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(RunId::from)
            .collect())
    }

    pub(crate) fn load_attempt_dispatches_for_recovery(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<AttemptDispatchRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_attempt_dispatches(&conn, run_id)
    }

    pub fn load_owned_attempt_worktree(
        &self,
        worktree_id: &AttemptWorktreeId,
    ) -> Result<Option<OwnedAttemptWorktreeRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_worktree_by_id(&conn, worktree_id)
    }

    pub fn load_attempt_worktree_for_attempt(
        &self,
        run_id: &RunId,
        attempt_id: &RunAttemptId,
    ) -> Result<Option<OwnedAttemptWorktreeRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(load_worktrees(&conn, run_id)?
            .into_iter()
            .find(|record| record.worktree.attempt_id == *attempt_id))
    }

    pub fn load_active_attempt_worktree(
        &self,
        worktree_id: &AttemptWorktreeId,
        at_ms: u64,
    ) -> Result<(OwnedAttemptWorktreeRecord, ExecutionLease), StoreError> {
        let conn = self.conn.lock().unwrap();
        let record = load_worktree_by_id(&conn, worktree_id)?
            .ok_or_else(|| data_error("owned attempt worktree was not found"))?;
        let lease = load_lease(&conn, &record.worktree.lease_id)?
            .ok_or_else(|| data_error("owned attempt worktree lease was not found"))?;
        validate_active_worktree_lease(&record, &lease, at_ms)?;
        Ok((record, lease))
    }

    pub fn mark_attempt_worktree_materialized(
        &self,
        worktree_id: &AttemptWorktreeId,
        lease_id: &ExecutionLeaseId,
        materialized_at_ms: u64,
    ) -> Result<OwnedAttemptWorktreeRecord, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_worktree_by_id(&tx, worktree_id)?
            .ok_or_else(|| data_error("owned attempt worktree was not found"))?;
        let lease = load_lease(&tx, lease_id)?
            .ok_or_else(|| data_error("owned attempt worktree lease was not found"))?;
        validate_active_worktree_lease(&current, &lease, materialized_at_ms)?;
        if current.worktree.lease_id != *lease_id {
            return Err(data_error("owned attempt worktree lease identity changed"));
        }
        if current.worktree.dirty_state == WorktreeDirtyState::Clean
            && current.worktree.cleanup_eligibility == WorktreeCleanupEligibility::Ineligible
        {
            return Ok(current);
        }
        if current.worktree.dirty_state != WorktreeDirtyState::Ambiguous
            || current.worktree.cleanup_eligibility != WorktreeCleanupEligibility::Ineligible
            || materialized_at_ms < current.updated_at_ms
        {
            return Err(data_error(
                "owned attempt worktree cannot record materialization",
            ));
        }
        update_worktree_state(
            &tx,
            &current,
            WorktreeDirtyState::Clean,
            WorktreeCleanupEligibility::Ineligible,
            materialized_at_ms,
        )?;
        let updated = load_worktree_by_id(&tx, worktree_id)?
            .ok_or_else(|| data_error("materialized attempt worktree disappeared"))?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn record_attempt_worktree_checkpoint_intent(
        &self,
        worktree_id: &AttemptWorktreeId,
        lease_id: &ExecutionLeaseId,
        checkpoint_id: &CheckpointId,
        payload_sha256: &str,
        recorded_at_ms: u64,
    ) -> Result<OwnedAttemptWorktreeRecord, StoreError> {
        if checkpoint_id.as_str().trim().is_empty() || !is_sha256(payload_sha256) {
            return Err(data_error("attempt checkpoint intent is malformed"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut current = load_worktree_by_id(&tx, worktree_id)?
            .ok_or_else(|| data_error("owned attempt worktree was not found"))?;
        let lease = load_lease(&tx, lease_id)?
            .ok_or_else(|| data_error("owned attempt worktree lease was not found"))?;
        validate_active_worktree_lease(&current, &lease, recorded_at_ms)?;
        if current.worktree.lease_id != *lease_id {
            return Err(data_error("owned attempt worktree lease identity changed"));
        }
        if let Some(existing) = current.checkpoint_intent_sha256.get(checkpoint_id) {
            if existing == payload_sha256 {
                return Ok(current);
            }
            return Err(data_error(
                "attempt checkpoint identity was reused with different bytes",
            ));
        }
        if current.worktree.cleanup_eligibility != WorktreeCleanupEligibility::Ineligible
            || recorded_at_ms < current.updated_at_ms
        {
            return Err(data_error(
                "owned attempt worktree cannot accept a checkpoint intent",
            ));
        }
        current
            .worktree
            .checkpoint_ids
            .insert(checkpoint_id.clone());
        current
            .checkpoint_intent_sha256
            .insert(checkpoint_id.clone(), payload_sha256.to_string());
        let checkpoints = StoredWorktreeCheckpoints {
            ids: current.worktree.checkpoint_ids.clone(),
            intent_sha256: current.checkpoint_intent_sha256.clone(),
            verified_process_death_sha256: current.verified_process_death_sha256.clone(),
            retained_evidence_sha256: current.retained_evidence_sha256.clone(),
        };
        let changed = tx.execute(
            "UPDATE orchestration_worktrees
                SET checkpoint_ids_json = ?1, updated_at_ms = ?2
              WHERE worktree_id = ?3 AND lease_id = ?4 AND updated_at_ms = ?5",
            params![
                serde_json::to_string(&checkpoints)?,
                checked_i64(recorded_at_ms, "checkpoint intent timestamp")?,
                worktree_id.as_str(),
                lease_id.as_str(),
                checked_i64(current.updated_at_ms, "prior worktree timestamp")?,
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "attempt checkpoint intent lost its compare-and-swap",
            ));
        }
        let updated = load_worktree_by_id(&tx, worktree_id)?
            .ok_or_else(|| data_error("checkpoint attempt worktree disappeared"))?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn retain_attempt_worktree(
        &self,
        worktree_id: &AttemptWorktreeId,
        lease_id: &ExecutionLeaseId,
        dirty_state: WorktreeDirtyState,
        retained_at_ms: u64,
    ) -> Result<OwnedAttemptWorktreeRecord, StoreError> {
        if dirty_state == WorktreeDirtyState::Clean {
            return Err(data_error("a clean worktree cannot be retained as dirty"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_worktree_by_id(&tx, worktree_id)?
            .ok_or_else(|| data_error("owned attempt worktree was not found"))?;
        let lease = load_lease(&tx, lease_id)?
            .ok_or_else(|| data_error("owned attempt worktree lease was not found"))?;
        validate_worktree_lease_identity(&current, &lease)?;
        if retained_at_ms < lease.last_heartbeat_ms {
            return Err(data_error(
                "owned attempt worktree retention timestamp moved backwards",
            ));
        }
        if current.worktree.lease_id != *lease_id {
            return Err(data_error("owned attempt worktree lease identity changed"));
        }
        if current.worktree.dirty_state == dirty_state
            && current.worktree.cleanup_eligibility == WorktreeCleanupEligibility::Retained
        {
            return Ok(current);
        }
        if current.worktree.cleanup_eligibility == WorktreeCleanupEligibility::Eligible
            || retained_at_ms < current.updated_at_ms
        {
            return Err(data_error("owned attempt worktree cannot be retained"));
        }
        update_worktree_state(
            &tx,
            &current,
            dirty_state,
            WorktreeCleanupEligibility::Retained,
            retained_at_ms,
        )?;
        let updated = load_worktree_by_id(&tx, worktree_id)?
            .ok_or_else(|| data_error("retained attempt worktree disappeared"))?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn mark_attempt_worktree_cleaned(
        &self,
        worktree_id: &AttemptWorktreeId,
        lease_id: &ExecutionLeaseId,
        verified_process_death_sha256: &str,
        retained_evidence_sha256: &str,
        cleaned_at_ms: u64,
    ) -> Result<OwnedAttemptWorktreeRecord, StoreError> {
        if !is_sha256(verified_process_death_sha256) || !is_sha256(retained_evidence_sha256) {
            return Err(data_error("attempt worktree cleanup evidence is malformed"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_worktree_by_id(&tx, worktree_id)?
            .ok_or_else(|| data_error("owned attempt worktree was not found"))?;
        if current.worktree.lease_id != *lease_id {
            return Err(data_error("owned attempt worktree lease identity changed"));
        }
        if current.worktree.dirty_state == WorktreeDirtyState::Clean
            && current.worktree.cleanup_eligibility == WorktreeCleanupEligibility::Eligible
        {
            if current.verified_process_death_sha256.as_deref()
                == Some(verified_process_death_sha256)
                && current.retained_evidence_sha256.as_deref() == Some(retained_evidence_sha256)
            {
                return Ok(current);
            }
            return Err(data_error(
                "attempt worktree cleanup identity was reused with different evidence",
            ));
        }
        let lease = load_lease(&tx, lease_id)?
            .ok_or_else(|| data_error("owned attempt worktree lease was not found"))?;
        validate_active_worktree_lease(&current, &lease, cleaned_at_ms)?;
        if current.worktree.dirty_state != WorktreeDirtyState::Clean
            || current.worktree.cleanup_eligibility != WorktreeCleanupEligibility::Ineligible
            || cleaned_at_ms < current.updated_at_ms
        {
            return Err(data_error(
                "owned attempt worktree cannot record clean removal",
            ));
        }
        let checkpoints = StoredWorktreeCheckpoints {
            ids: current.worktree.checkpoint_ids.clone(),
            intent_sha256: current.checkpoint_intent_sha256.clone(),
            verified_process_death_sha256: Some(verified_process_death_sha256.to_string()),
            retained_evidence_sha256: Some(retained_evidence_sha256.to_string()),
        };
        let changed = tx.execute(
            "UPDATE orchestration_worktrees
                SET checkpoint_ids_json = ?1, dirty_state = 'clean',
                    cleanup_eligibility = 'eligible', updated_at_ms = ?2
              WHERE worktree_id = ?3 AND lease_id = ?4 AND dirty_state = 'clean'
                AND cleanup_eligibility = 'ineligible' AND updated_at_ms = ?5",
            params![
                serde_json::to_string(&checkpoints)?,
                checked_i64(cleaned_at_ms, "worktree cleanup timestamp")?,
                worktree_id.as_str(),
                lease_id.as_str(),
                checked_i64(current.updated_at_ms, "prior worktree timestamp")?,
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "attempt worktree cleanup receipt lost its compare-and-swap",
            ));
        }
        let updated = load_worktree_by_id(&tx, worktree_id)?
            .ok_or_else(|| data_error("cleaned attempt worktree disappeared"))?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn attempt_has_pending_recovery_review(
        &self,
        run_id: &RunId,
        task_id: &RunTaskId,
        attempt_id: &RunAttemptId,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count = conn.query_row(
            "SELECT COUNT(*) FROM orchestration_recovery_reviews
              WHERE run_id = ?1 AND task_id = ?2 AND attempt_id = ?3
                AND status = 'pending'",
            params![run_id.as_str(), task_id.as_str(), attempt_id.as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count != 0)
    }

    /// Inserts a canonical recovery review or returns the existing unresolved
    /// identity. The unique identity deliberately excludes the review id so
    /// repeated startup passes cannot create duplicate human work.
    pub fn upsert_recovery_review(
        &self,
        review: &RecoveryReview,
    ) -> Result<RecoveryReview, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = load_recovery_review_by_identity(&tx, review)? {
            if existing.status == RecoveryReviewStatus::Pending
                || existing == *review
                || (existing.status == RecoveryReviewStatus::Resolved
                    && review.status == RecoveryReviewStatus::Pending)
            {
                return Ok(existing);
            }
            return Err(data_error(
                "recovery review identity conflicts with its durable resolution",
            ));
        }
        if let Some(existing) = load_recovery_review_by_id(&tx, &review.id)? {
            if existing == *review {
                return Ok(existing);
            }
            return Err(data_error(
                "recovery review id was reused with different bytes",
            ));
        }
        insert_recovery_review(&tx, review)?;
        let stored = load_recovery_review_by_id(&tx, &review.id)?
            .ok_or_else(|| data_error("recovery review disappeared after insertion"))?;
        tx.commit()?;
        Ok(stored)
    }

    /// Resolves one review through an audited compare-and-swap. A replay of
    /// the exact local-control decision is idempotent; changed decisions fail.
    pub fn resolve_recovery_review(
        &self,
        review_id: &pandamux_core::RecoveryReviewId,
        resolution: &str,
        resolved_at_ms: u64,
    ) -> Result<RecoveryReview, StoreError> {
        if !valid_reason_code(resolution) {
            return Err(data_error("recovery review resolution is malformed"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_recovery_review_by_id(&tx, review_id)?
            .ok_or_else(|| data_error("recovery review was not found"))?;
        if current.status == RecoveryReviewStatus::Resolved {
            if current.resolution.as_deref() == Some(resolution)
                && current.resolved_at_ms == Some(resolved_at_ms)
            {
                return Ok(current);
            }
            return Err(data_error("recovery review resolution cannot be changed"));
        }
        if resolved_at_ms < current.created_at_ms {
            return Err(data_error(
                "recovery review resolution timestamp moved backwards",
            ));
        }
        let changed = tx.execute(
            "UPDATE orchestration_recovery_reviews
                SET status = 'resolved', resolved_at_ms = ?1, resolution = ?2
              WHERE review_id = ?3 AND status = 'pending'
                AND resolved_at_ms IS NULL AND resolution IS NULL",
            params![
                checked_i64(resolved_at_ms, "recovery review resolution timestamp")?,
                resolution,
                review_id.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "recovery review resolution lost its compare-and-swap",
            ));
        }
        let resolved = load_recovery_review_by_id(&tx, review_id)?
            .ok_or_else(|| data_error("resolved recovery review disappeared"))?;
        tx.commit()?;
        Ok(resolved)
    }

    pub fn get_recovery_review(
        &self,
        review_id: &pandamux_core::RecoveryReviewId,
    ) -> Result<Option<RecoveryReview>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_recovery_review_by_id(&conn, review_id)
    }

    pub fn list_pending_recovery_reviews(&self) -> Result<Vec<RecoveryReview>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT DISTINCT run_id FROM orchestration_recovery_reviews
              WHERE status = 'pending' ORDER BY run_id ASC",
        )?;
        let run_ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut reviews = Vec::new();
        for run_id in run_ids {
            reviews.extend(
                load_recovery_reviews(&conn, &RunId::from(run_id))?
                    .into_iter()
                    .filter(|review| review.status == RecoveryReviewStatus::Pending),
            );
        }
        reviews.sort_by_key(|review| (review.created_at_ms, review.id.clone()));
        Ok(reviews)
    }

    /// Reclaims only an expired, exact, provably undispatched effect claim.
    /// Prepared or post-marker effects are intentionally ineligible.
    pub fn reclaim_expired_effect_claim(
        &self,
        ownership: &EffectOwnership,
        now_ms: u64,
        live_owners: &BTreeSet<ServerIncarnationId>,
    ) -> Result<RecoveryLeaseDisposition, StoreError> {
        validate_effect_ownership(ownership)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(effect) = load_stored_effect_by_id(&tx, &ownership.effect_id)? else {
            return Ok(RecoveryLeaseDisposition::Ineligible);
        };
        if effect.effect.payload_sha256 != ownership.expected_payload_sha256
            || effect.delivery.owner_incarnation_id.as_ref()
                != Some(&ownership.owner_incarnation_id)
            || effect.delivery.lease_id.as_ref() != Some(&ownership.lease_id)
            || effect.delivery.claim_revision != ownership.claim_revision
        {
            return Ok(RecoveryLeaseDisposition::ForeignOwner);
        }
        let Some(lease) = load_lease(&tx, &ownership.lease_id)? else {
            return Ok(RecoveryLeaseDisposition::ForeignOwner);
        };
        if lease.resource_kind != LeaseResourceKind::Effect
            || lease.resource_id != ownership.effect_id.as_str()
            || lease.owner_incarnation_id != ownership.owner_incarnation_id
            || lease.revision != ownership.claim_revision
        {
            return Ok(RecoveryLeaseDisposition::ForeignOwner);
        }
        if now_ms < lease.last_heartbeat_ms {
            return Ok(RecoveryLeaseDisposition::ClockAnomaly);
        }
        if live_owners.contains(&lease.owner_incarnation_id) {
            return Ok(RecoveryLeaseDisposition::LiveOwner);
        }
        if now_ms < lease.expires_at_ms {
            return Ok(RecoveryLeaseDisposition::Unexpired);
        }
        if effect.delivery.state != EffectDeliveryState::Claimed {
            return Ok(RecoveryLeaseDisposition::Ineligible);
        }
        let next_revision = ownership
            .claim_revision
            .checked_add(1)
            .ok_or_else(|| data_error("effect reclaim revision overflow"))?;
        let changed = tx.execute(
            "UPDATE orchestration_effects
                SET delivery_state = 'pending', owner_incarnation_id = NULL,
                    lease_id = NULL, claim_revision = ?1, updated_at_ms = ?2
              WHERE effect_id = ?3 AND payload_sha256 = ?4
                AND delivery_state = 'claimed' AND owner_incarnation_id = ?5
                AND lease_id = ?6 AND claim_revision = ?7",
            params![
                checked_i64(next_revision, "effect reclaim revision")?,
                checked_i64(now_ms, "effect reclaim timestamp")?,
                ownership.effect_id.as_str(),
                ownership.expected_payload_sha256,
                ownership.owner_incarnation_id.as_str(),
                ownership.lease_id.as_str(),
                checked_i64(ownership.claim_revision, "effect claim revision")?,
            ],
        )?;
        let deleted = tx.execute(
            "DELETE FROM orchestration_leases
              WHERE lease_id = ?1 AND resource_kind = 'effect' AND resource_id = ?2
                AND owner_incarnation_id = ?3 AND revision = ?4
                AND expires_at_ms <= ?5",
            params![
                ownership.lease_id.as_str(),
                ownership.effect_id.as_str(),
                ownership.owner_incarnation_id.as_str(),
                checked_i64(ownership.claim_revision, "effect lease revision")?,
                checked_i64(now_ms, "effect reclaim timestamp")?,
            ],
        )?;
        if changed != 1 || deleted != 1 {
            return Err(data_error("effect reclaim lost its compare-and-swap"));
        }
        tx.commit()?;
        Ok(RecoveryLeaseDisposition::Reclaimed)
    }

    /// Requeues only an expired exact capacity claim that never crossed its
    /// dispatch marker. A live owner or anomalous clock always wins over TTL.
    pub fn reclaim_expired_admission(
        &self,
        ownership: &AdmissionOwnership,
        now_ms: u64,
        live_owners: &BTreeSet<ServerIncarnationId>,
    ) -> Result<RecoveryLeaseDisposition, StoreError> {
        validate_admission_ownership(ownership)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(current) = load_queue_record_by_id(&tx, &ownership.queue_id)? else {
            return Ok(RecoveryLeaseDisposition::Ineligible);
        };
        if current.candidate_sha256 != ownership.expected_candidate_sha256
            || current.owner_incarnation_id.as_ref() != Some(&ownership.owner_incarnation_id)
            || current.lease_id.as_ref() != Some(&ownership.lease_id)
            || current.claim_revision != ownership.claim_revision
        {
            return Ok(RecoveryLeaseDisposition::ForeignOwner);
        }
        let Some(lease) = load_lease(&tx, &ownership.lease_id)? else {
            return Ok(RecoveryLeaseDisposition::ForeignOwner);
        };
        if lease.resource_kind != LeaseResourceKind::Capacity
            || lease.resource_id != ownership.queue_id
            || lease.owner_incarnation_id != ownership.owner_incarnation_id
            || lease.revision != ownership.claim_revision
        {
            return Ok(RecoveryLeaseDisposition::ForeignOwner);
        }
        if now_ms < lease.last_heartbeat_ms {
            return Ok(RecoveryLeaseDisposition::ClockAnomaly);
        }
        if live_owners.contains(&lease.owner_incarnation_id) {
            return Ok(RecoveryLeaseDisposition::LiveOwner);
        }
        if now_ms < lease.expires_at_ms {
            return Ok(RecoveryLeaseDisposition::Unexpired);
        }
        if current.state != QueueState::Claimed {
            return Ok(RecoveryLeaseDisposition::Ineligible);
        }
        let dispatch_crossed_marker = load_attempt_dispatches(&tx, &current.run_id)?
            .into_iter()
            .find(|attempt| attempt.attempt_id == current.attempt_id)
            .is_some_and(|attempt| attempt.state != AttemptDispatchState::Prepared)
            || load_effects(&tx, &current.run_id)?
                .into_iter()
                .any(|effect| {
                    matches!(
                        &effect.effect.kind,
                        pandamux_core::OrchestrationEffectKind::DispatchTask { attempt_id, .. }
                            if attempt_id == &current.attempt_id
                    ) && matches!(
                        effect.delivery.state,
                        EffectDeliveryState::DispatchStarted
                            | EffectDeliveryState::InFlight
                            | EffectDeliveryState::OutcomeRecorded
                            | EffectDeliveryState::Settled
                            | EffectDeliveryState::Cancelled
                            | EffectDeliveryState::ReviewRequired
                    )
                });
        if dispatch_crossed_marker {
            return Ok(RecoveryLeaseDisposition::Ineligible);
        }
        let next_revision = ownership
            .claim_revision
            .checked_add(1)
            .ok_or_else(|| data_error("admission reclaim revision overflow"))?;
        let changed = tx.execute(
            "UPDATE orchestration_queue
                SET state = 'queued', owner_incarnation_id = NULL, lease_id = NULL,
                    claim_revision = ?1, not_before_ms = max(not_before_ms, ?2)
              WHERE queue_id = ?3 AND candidate_sha256 = ?4 AND state = 'claimed'
                AND owner_incarnation_id = ?5 AND lease_id = ?6
                AND claim_revision = ?7",
            params![
                checked_i64(next_revision, "admission reclaim revision")?,
                checked_i64(now_ms, "admission reclaim timestamp")?,
                ownership.queue_id,
                ownership.expected_candidate_sha256,
                ownership.owner_incarnation_id.as_str(),
                ownership.lease_id.as_str(),
                checked_i64(ownership.claim_revision, "admission claim revision")?,
            ],
        )?;
        let deleted = tx.execute(
            "DELETE FROM orchestration_leases
              WHERE lease_id = ?1 AND resource_kind = 'capacity' AND resource_id = ?2
                AND owner_incarnation_id = ?3 AND revision = ?4
                AND expires_at_ms <= ?5",
            params![
                ownership.lease_id.as_str(),
                ownership.queue_id,
                ownership.owner_incarnation_id.as_str(),
                checked_i64(ownership.claim_revision, "admission lease revision")?,
                checked_i64(now_ms, "admission reclaim timestamp")?,
            ],
        )?;
        if changed != 1 || deleted != 1 {
            return Err(data_error("admission reclaim lost its compare-and-swap"));
        }
        tx.commit()?;
        Ok(RecoveryLeaseDisposition::Reclaimed)
    }

    /// Transfers a post-marker remote attempt to the current Hub incarnation
    /// only after the caller has persisted and cryptographically verified the
    /// node's exact status receipt. Effect and capacity fencing revisions move
    /// together so an old Hub process cannot resume either owner.
    pub fn reattach_remote_attempt_ownership(
        &self,
        attempt_id: &RunAttemptId,
        effect_id: &OrchestrationEffectId,
        expected_payload_sha256: &str,
        new_owner: &ServerIncarnationId,
        reattached_at_ms: u64,
        lease_ttl_ms: u64,
    ) -> Result<EffectOwnership, StoreError> {
        if attempt_id.as_str().trim().is_empty()
            || effect_id.as_str().trim().is_empty()
            || !is_sha256(expected_payload_sha256)
            || new_owner.as_str().trim().is_empty()
            || lease_ttl_ms == 0
        {
            return Err(data_error("remote reattachment identity is malformed"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_stored_effect_by_id(&tx, effect_id)?
            .ok_or_else(|| data_error("remote reattachment effect was not found"))?;
        let dispatch = load_attempt_dispatches(&tx, &current.run_id)?
            .into_iter()
            .find(|record| record.attempt_id == *attempt_id)
            .ok_or_else(|| data_error("remote reattachment attempt was not found"))?;
        if dispatch.effect_id != *effect_id
            || current.effect.payload_sha256 != expected_payload_sha256
            || !matches!(
                current.delivery.state,
                EffectDeliveryState::DispatchStarted | EffectDeliveryState::InFlight
            )
            || !matches!(
                dispatch.state,
                AttemptDispatchState::DispatchStarted
                    | AttemptDispatchState::Accepted
                    | AttemptDispatchState::CancelRequested
            )
        {
            return Err(data_error(
                "remote reattachment no longer owns the exact post-marker attempt",
            ));
        }
        let old_owner = current
            .delivery
            .owner_incarnation_id
            .as_ref()
            .ok_or_else(|| data_error("remote reattachment effect owner is missing"))?;
        let effect_lease_id = current
            .delivery
            .lease_id
            .as_ref()
            .ok_or_else(|| data_error("remote reattachment effect lease is missing"))?;
        let effect_lease = load_lease(&tx, effect_lease_id)?
            .ok_or_else(|| data_error("remote reattachment effect lease was not found"))?;
        if effect_lease.resource_kind != LeaseResourceKind::Effect
            || effect_lease.resource_id != effect_id.as_str()
            || effect_lease.owner_incarnation_id != *old_owner
            || effect_lease.revision != current.delivery.claim_revision
        {
            return Err(data_error(
                "remote reattachment effect lease identity changed",
            ));
        }

        let queue = load_queue_records(&tx, &current.run_id)?
            .into_iter()
            .find(|record| record.attempt_id == *attempt_id)
            .ok_or_else(|| data_error("remote reattachment capacity owner was not found"))?;
        if !matches!(queue.state, QueueState::Claimed | QueueState::Dispatched) {
            return Err(data_error(
                "remote reattachment capacity is no longer owned",
            ));
        }
        let queue_owner = queue
            .owner_incarnation_id
            .as_ref()
            .ok_or_else(|| data_error("remote reattachment capacity owner is missing"))?;
        let queue_lease_id = queue
            .lease_id
            .as_ref()
            .ok_or_else(|| data_error("remote reattachment capacity lease is missing"))?;
        let queue_lease = load_lease(&tx, queue_lease_id)?
            .ok_or_else(|| data_error("remote reattachment capacity lease was not found"))?;
        if queue_lease.resource_kind != LeaseResourceKind::Capacity
            || queue_lease.resource_id != queue.queue_id
            || queue_lease.owner_incarnation_id != *queue_owner
            || queue_lease.revision != queue.claim_revision
        {
            return Err(data_error(
                "remote reattachment capacity lease identity changed",
            ));
        }

        let effective_at_ms = reattached_at_ms
            .max(current.updated_at_ms)
            .max(effect_lease.last_heartbeat_ms)
            .max(queue_lease.last_heartbeat_ms);
        let expires_at_ms = effective_at_ms
            .checked_add(lease_ttl_ms)
            .ok_or_else(|| data_error("remote reattachment lease expiry overflow"))?;
        let effect_revision = current
            .delivery
            .claim_revision
            .checked_add(1)
            .ok_or_else(|| data_error("remote effect reattachment revision overflow"))?;
        let queue_revision = queue
            .claim_revision
            .checked_add(1)
            .ok_or_else(|| data_error("remote capacity reattachment revision overflow"))?;

        let effect_lease_changed = tx.execute(
            "UPDATE orchestration_leases
                SET owner_incarnation_id = ?1, expires_at_ms = ?2,
                    last_heartbeat_ms = ?3, revision = ?4
              WHERE lease_id = ?5 AND resource_kind = 'effect' AND resource_id = ?6
                AND owner_incarnation_id = ?7 AND revision = ?8",
            params![
                new_owner.as_str(),
                checked_i64(expires_at_ms, "remote effect lease expiry")?,
                checked_i64(effective_at_ms, "remote effect lease heartbeat")?,
                checked_i64(effect_revision, "remote effect lease revision")?,
                effect_lease_id.as_str(),
                effect_id.as_str(),
                old_owner.as_str(),
                checked_i64(
                    current.delivery.claim_revision,
                    "prior effect claim revision"
                )?,
            ],
        )?;
        let effect_changed = tx.execute(
            "UPDATE orchestration_effects
                SET owner_incarnation_id = ?1, claim_revision = ?2, updated_at_ms = ?3
              WHERE effect_id = ?4 AND payload_sha256 = ?5
                AND owner_incarnation_id = ?6 AND lease_id = ?7
                AND claim_revision = ?8 AND delivery_state IN ('dispatch_started', 'in_flight')",
            params![
                new_owner.as_str(),
                checked_i64(effect_revision, "remote effect claim revision")?,
                checked_i64(effective_at_ms, "remote effect reattachment timestamp")?,
                effect_id.as_str(),
                expected_payload_sha256,
                old_owner.as_str(),
                effect_lease_id.as_str(),
                checked_i64(
                    current.delivery.claim_revision,
                    "prior effect claim revision"
                )?,
            ],
        )?;
        let queue_lease_changed = tx.execute(
            "UPDATE orchestration_leases
                SET owner_incarnation_id = ?1, expires_at_ms = ?2,
                    last_heartbeat_ms = ?3, revision = ?4
              WHERE lease_id = ?5 AND resource_kind = 'capacity' AND resource_id = ?6
                AND owner_incarnation_id = ?7 AND revision = ?8",
            params![
                new_owner.as_str(),
                checked_i64(expires_at_ms, "remote capacity lease expiry")?,
                checked_i64(effective_at_ms, "remote capacity lease heartbeat")?,
                checked_i64(queue_revision, "remote capacity lease revision")?,
                queue_lease_id.as_str(),
                queue.queue_id,
                queue_owner.as_str(),
                checked_i64(queue.claim_revision, "prior capacity claim revision")?,
            ],
        )?;
        let queue_changed = tx.execute(
            "UPDATE orchestration_queue
                SET owner_incarnation_id = ?1, claim_revision = ?2
              WHERE queue_id = ?3 AND candidate_sha256 = ?4
                AND owner_incarnation_id = ?5 AND lease_id = ?6
                AND claim_revision = ?7 AND state IN ('claimed', 'dispatched')",
            params![
                new_owner.as_str(),
                checked_i64(queue_revision, "remote capacity claim revision")?,
                queue.queue_id,
                queue.candidate_sha256,
                queue_owner.as_str(),
                queue_lease_id.as_str(),
                checked_i64(queue.claim_revision, "prior capacity claim revision")?,
            ],
        )?;
        if effect_lease_changed != 1
            || effect_changed != 1
            || queue_lease_changed != 1
            || queue_changed != 1
        {
            return Err(data_error(
                "remote attempt reattachment lost its compare-and-swap",
            ));
        }
        tx.commit()?;
        Ok(EffectOwnership {
            effect_id: effect_id.clone(),
            expected_payload_sha256: expected_payload_sha256.to_string(),
            owner_incarnation_id: new_owner.clone(),
            lease_id: effect_lease_id.clone(),
            claim_revision: effect_revision,
        })
    }

    pub fn insert_execution_queue_record(
        &self,
        record: &OrchestrationQueueRecord,
    ) -> Result<OrchestrationQueueRecord, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = load_queue_record_by_id(&tx, &record.queue_id)? {
            if &existing == record {
                return Ok(existing);
            }
            return Err(data_error(
                "execution queue identity was reused with different bytes",
            ));
        }
        insert_queue_record(&tx, record)?;
        let stored = load_queue_record_by_id(&tx, &record.queue_id)?
            .ok_or_else(|| data_error("execution queue record disappeared"))?;
        tx.commit()?;
        Ok(stored)
    }

    pub fn list_execution_queue_records(
        &self,
    ) -> Result<Vec<OrchestrationQueueRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        load_all_queue_records(&conn)
    }

    pub fn try_claim_execution_admission(
        &self,
        request: &AdmissionClaimStoreRequest,
    ) -> Result<Option<AdmissionClaimStoreOutcome>, StoreError> {
        validate_admission_claim_request(request)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_admission_policy(&tx, &request.policy)?;
        let current = load_queue_record_by_id(&tx, &request.queue_id)?
            .ok_or_else(|| data_error("execution queue record was not found"))?;
        if current.candidate_sha256 != request.expected_candidate_sha256 {
            return Err(data_error("execution queue candidate identity changed"));
        }

        if current.state == QueueState::Claimed
            && current.owner_incarnation_id.as_ref() == Some(&request.owner_incarnation_id)
            && current.lease_id.as_ref() == Some(&request.lease_id)
            && current.claim_revision == request.expected_claim_revision.saturating_add(1)
        {
            let lease = load_lease(&tx, &request.lease_id)?
                .ok_or_else(|| data_error("claimed execution lease disappeared"))?;
            if lease.resource_kind != LeaseResourceKind::Capacity
                || lease.resource_id != request.queue_id
                || lease.owner_incarnation_id != request.owner_incarnation_id
                || lease.last_heartbeat_ms != request.claimed_at_ms
                || lease.expires_at_ms != request.lease_expires_at_ms
                || lease.revision != current.claim_revision
            {
                return Err(data_error(
                    "execution admission replay differs from the durable claim",
                ));
            }
            return Ok(Some(AdmissionClaimStoreOutcome {
                record: current,
                lease,
                acquired: false,
            }));
        }

        if current.state != QueueState::Queued
            || current.owner_incarnation_id.is_some()
            || current.lease_id.is_some()
            || current.claim_revision != request.expected_claim_revision
        {
            return Err(data_error(
                "execution admission claim lost its compare-and-swap",
            ));
        }
        let remote_orchestration = !current.interactive
            && request.orchestration.is_none()
            && remote_prepare_result_for_queue(&tx, &current)?.is_some();
        if (current.interactive && request.orchestration.is_some())
            || (!current.interactive && request.orchestration.is_none() && !remote_orchestration)
        {
            return Err(data_error(
                "execution admission class does not match its transition",
            ));
        }

        let records = load_all_queue_records(&tx)?;
        let counts = active_admission_counts(&tx, &records)?;
        let mut interactive = records
            .iter()
            .filter(|record| {
                record.state == QueueState::Queued
                    && record.interactive
                    && record.not_before_ms <= request.claimed_at_ms
            })
            .map(|record| {
                admission_has_capacity(&tx, record, &counts, &request.policy)
                    .map(|eligible| eligible.then_some(record))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut orchestrated = records
            .iter()
            .filter(|record| {
                record.state == QueueState::Queued
                    && !record.interactive
                    && record.not_before_ms <= request.claimed_at_ms
            })
            .map(|record| {
                admission_has_capacity(&tx, record, &counts, &request.policy)
                    .map(|eligible| eligible.then_some(record))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        interactive.sort_by_key(admission_order_key);
        orchestrated.sort_by_key(admission_order_key);

        let previous_streak = records
            .iter()
            .filter_map(|record| record.claim_order.zip(record.interactive_streak))
            .max_by_key(|(order, _)| *order)
            .map(|(_, streak)| streak)
            .unwrap_or(0);
        let selected = match (interactive.first(), orchestrated.first()) {
            (Some(interactive), Some(orchestrated)) => {
                if previous_streak >= 4 {
                    *orchestrated
                } else {
                    *interactive
                }
            }
            (Some(interactive), None) => *interactive,
            (None, Some(orchestrated)) => *orchestrated,
            (None, None) => return Ok(None),
        };
        if selected.queue_id != request.queue_id {
            return Ok(None);
        }

        let claim_revision = request
            .expected_claim_revision
            .checked_add(1)
            .ok_or_else(|| data_error("execution admission revision overflow"))?;
        let claim_order = records
            .iter()
            .filter_map(|record| record.claim_order)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| data_error("execution admission order overflow"))?;
        let interactive_streak = if current.interactive {
            previous_streak.saturating_add(1).min(4)
        } else {
            0
        };
        let lease = ExecutionLease {
            id: request.lease_id.clone(),
            resource_kind: LeaseResourceKind::Capacity,
            resource_id: request.queue_id.clone(),
            owner_incarnation_id: request.owner_incarnation_id.clone(),
            expires_at_ms: request.lease_expires_at_ms,
            last_heartbeat_ms: request.claimed_at_ms,
            revision: claim_revision,
        };
        ensure_execution_lease(&tx, &lease)?;
        if let Some(transition) = request.orchestration.as_ref() {
            apply_admission_orchestration_transition(
                &tx,
                &current,
                transition,
                request.claimed_at_ms,
            )?;
        }
        let changed = tx.execute(
            "UPDATE orchestration_queue
                SET state = 'claimed', owner_incarnation_id = ?1, lease_id = ?2,
                    claim_revision = ?3, claimed_at_ms = ?4, claim_order = ?5,
                    interactive_streak = ?6
              WHERE queue_id = ?7 AND candidate_sha256 = ?8 AND state = 'queued'
                AND owner_incarnation_id IS NULL AND lease_id IS NULL
                AND claim_revision = ?9",
            params![
                request.owner_incarnation_id.as_str(),
                request.lease_id.as_str(),
                checked_i64(claim_revision, "claimed queue revision")?,
                checked_i64(request.claimed_at_ms, "queue claim timestamp")?,
                checked_i64(claim_order, "queue claim order")?,
                i64::from(interactive_streak),
                request.queue_id,
                request.expected_candidate_sha256,
                checked_i64(request.expected_claim_revision, "expected queue revision")?,
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "execution admission claim lost its compare-and-swap",
            ));
        }
        let record = load_queue_record_by_id(&tx, &request.queue_id)?
            .ok_or_else(|| data_error("claimed execution queue record disappeared"))?;
        tx.commit()?;
        Ok(Some(AdmissionClaimStoreOutcome {
            record,
            lease,
            acquired: true,
        }))
    }

    pub fn cancel_queued_execution_admission(
        &self,
        queue_id: &str,
        expected_candidate_sha256: &str,
    ) -> Result<OrchestrationQueueRecord, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_queue_record_by_id(&tx, queue_id)?
            .ok_or_else(|| data_error("execution queue record was not found"))?;
        if current.candidate_sha256 != expected_candidate_sha256 {
            return Err(data_error("execution queue candidate identity changed"));
        }
        match current.state {
            QueueState::Cancelled => return Ok(current),
            QueueState::Queued
                if current.owner_incarnation_id.is_none() && current.lease_id.is_none() => {}
            _ => return Err(data_error("execution queue record cannot be cancelled")),
        }
        let changed = tx.execute(
            "UPDATE orchestration_queue SET state = 'cancelled'
              WHERE queue_id = ?1 AND candidate_sha256 = ?2 AND state = 'queued'
                AND owner_incarnation_id IS NULL AND lease_id IS NULL",
            params![queue_id, expected_candidate_sha256],
        )?;
        if changed != 1 {
            return Err(data_error(
                "execution queue cancellation lost its compare-and-swap",
            ));
        }
        let updated = load_queue_record_by_id(&tx, queue_id)?
            .ok_or_else(|| data_error("cancelled execution queue record disappeared"))?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn mark_execution_admission_dispatched(
        &self,
        ownership: &AdmissionOwnership,
    ) -> Result<OrchestrationQueueRecord, StoreError> {
        validate_admission_ownership(ownership)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, _) = load_owned_admission(&tx, ownership)?;
        match current.state {
            QueueState::Dispatched => return Ok(current),
            QueueState::Claimed => {}
            _ => return Err(data_error("execution admission cannot be dispatched")),
        }
        let changed = tx.execute(
            "UPDATE orchestration_queue SET state = 'dispatched'
              WHERE queue_id = ?1 AND candidate_sha256 = ?2 AND state = 'claimed'
                AND owner_incarnation_id = ?3 AND lease_id = ?4
                AND claim_revision = ?5",
            params![
                ownership.queue_id,
                ownership.expected_candidate_sha256,
                ownership.owner_incarnation_id.as_str(),
                ownership.lease_id.as_str(),
                checked_i64(ownership.claim_revision, "queue claim revision")?,
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "execution dispatch marker lost its compare-and-swap",
            ));
        }
        let updated = load_queue_record_by_id(&tx, &ownership.queue_id)?
            .ok_or_else(|| data_error("dispatched execution queue record disappeared"))?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn renew_execution_admission(
        &self,
        request: &AdmissionRenewStoreRequest,
    ) -> Result<AdmissionClaimStoreOutcome, StoreError> {
        validate_admission_ownership(&request.ownership)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, lease) = load_owned_admission(&tx, &request.ownership)?;
        if !matches!(current.state, QueueState::Claimed | QueueState::Dispatched)
            || request.renewed_at_ms < lease.last_heartbeat_ms
            || request.renewed_at_ms >= lease.expires_at_ms
            || request.lease_expires_at_ms <= request.renewed_at_ms
        {
            return Err(data_error("execution admission lease cannot be renewed"));
        }
        let next_revision = request
            .ownership
            .claim_revision
            .checked_add(1)
            .ok_or_else(|| data_error("execution admission revision overflow"))?;
        let queue_changed = tx.execute(
            "UPDATE orchestration_queue SET claim_revision = ?1
              WHERE queue_id = ?2 AND candidate_sha256 = ?3
                AND owner_incarnation_id = ?4 AND lease_id = ?5
                AND claim_revision = ?6 AND state IN ('claimed', 'dispatched')",
            params![
                checked_i64(next_revision, "renewed queue claim revision")?,
                request.ownership.queue_id,
                request.ownership.expected_candidate_sha256,
                request.ownership.owner_incarnation_id.as_str(),
                request.ownership.lease_id.as_str(),
                checked_i64(request.ownership.claim_revision, "queue claim revision")?,
            ],
        )?;
        let lease_changed = tx.execute(
            "UPDATE orchestration_leases
                SET expires_at_ms = ?1, last_heartbeat_ms = ?2, revision = ?3
              WHERE lease_id = ?4 AND resource_kind = 'capacity' AND resource_id = ?5
                AND owner_incarnation_id = ?6 AND revision = ?7",
            params![
                checked_i64(request.lease_expires_at_ms, "renewed lease expiry")?,
                checked_i64(request.renewed_at_ms, "renewed lease heartbeat")?,
                checked_i64(next_revision, "renewed lease revision")?,
                request.ownership.lease_id.as_str(),
                request.ownership.queue_id,
                request.ownership.owner_incarnation_id.as_str(),
                checked_i64(request.ownership.claim_revision, "lease revision")?,
            ],
        )?;
        if queue_changed != 1 || lease_changed != 1 {
            return Err(data_error(
                "execution admission renewal lost its compare-and-swap",
            ));
        }
        let record = load_queue_record_by_id(&tx, &request.ownership.queue_id)?
            .ok_or_else(|| data_error("renewed execution queue record disappeared"))?;
        let lease = load_lease(&tx, &request.ownership.lease_id)?
            .ok_or_else(|| data_error("renewed execution lease disappeared"))?;
        tx.commit()?;
        Ok(AdmissionClaimStoreOutcome {
            record,
            lease,
            acquired: false,
        })
    }

    pub fn requeue_expired_execution_admission(
        &self,
        request: &AdmissionRequeueStoreRequest,
    ) -> Result<OrchestrationQueueRecord, StoreError> {
        validate_admission_ownership(&request.ownership)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, lease) = load_owned_admission(&tx, &request.ownership)?;
        if current.state != QueueState::Claimed
            || lease.expires_at_ms > request.reclaimed_at_ms
            || request.not_before_ms < request.reclaimed_at_ms
            || request.not_before_ms < current.enqueued_at_ms
        {
            return Err(data_error("execution admission lease cannot be reclaimed"));
        }
        let next_revision = request
            .ownership
            .claim_revision
            .checked_add(1)
            .ok_or_else(|| data_error("execution admission revision overflow"))?;
        let changed = tx.execute(
            "UPDATE orchestration_queue
                SET state = 'queued', owner_incarnation_id = NULL, lease_id = NULL,
                    claim_revision = ?1, not_before_ms = ?2
              WHERE queue_id = ?3 AND candidate_sha256 = ?4 AND state = 'claimed'
                AND owner_incarnation_id = ?5 AND lease_id = ?6
                AND claim_revision = ?7",
            params![
                checked_i64(next_revision, "requeued claim revision")?,
                checked_i64(request.not_before_ms, "requeued not-before")?,
                request.ownership.queue_id,
                request.ownership.expected_candidate_sha256,
                request.ownership.owner_incarnation_id.as_str(),
                request.ownership.lease_id.as_str(),
                checked_i64(request.ownership.claim_revision, "queue claim revision")?,
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "execution admission reclaim lost its compare-and-swap",
            ));
        }
        let deleted = tx.execute(
            "DELETE FROM orchestration_leases
              WHERE lease_id = ?1 AND resource_kind = 'capacity' AND resource_id = ?2
                AND owner_incarnation_id = ?3 AND revision = ?4
                AND expires_at_ms <= ?5",
            params![
                request.ownership.lease_id.as_str(),
                request.ownership.queue_id,
                request.ownership.owner_incarnation_id.as_str(),
                checked_i64(request.ownership.claim_revision, "lease revision")?,
                checked_i64(request.reclaimed_at_ms, "lease reclaim timestamp")?,
            ],
        )?;
        if deleted != 1 {
            return Err(data_error(
                "execution admission lease reclaim was not exact",
            ));
        }
        let updated = load_queue_record_by_id(&tx, &request.ownership.queue_id)?
            .ok_or_else(|| data_error("requeued execution record disappeared"))?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn release_execution_admission(
        &self,
        ownership: &AdmissionOwnership,
    ) -> Result<OrchestrationQueueRecord, StoreError> {
        self.release_execution_admission_with_budget(ownership, super::store_now_ms(), None)
    }

    pub fn release_execution_admission_with_budget(
        &self,
        ownership: &AdmissionOwnership,
        released_at_ms: u64,
        failpoint: Option<BudgetExecutionFailpoint>,
    ) -> Result<OrchestrationQueueRecord, StoreError> {
        validate_admission_ownership(ownership)?;
        validate_budget_failpoint(
            failpoint,
            &[
                BudgetExecutionFailpoint::BeforeReleaseCommit,
                BudgetExecutionFailpoint::AfterReleaseCommit,
            ],
        )?;
        if failpoint == Some(BudgetExecutionFailpoint::BeforeReleaseCommit) {
            return Err(injected_failure("before_budget_release_commit"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_queue_record_by_id(&tx, &ownership.queue_id)?
            .ok_or_else(|| data_error("execution queue record was not found"))?;
        if current.candidate_sha256 != ownership.expected_candidate_sha256 {
            return Err(data_error("execution queue candidate identity changed"));
        }
        if current.lease_id.is_none() && current.owner_incarnation_id.is_none() {
            let released_revision = ownership
                .claim_revision
                .checked_add(1)
                .ok_or_else(|| data_error("execution admission revision overflow"))?;
            if matches!(
                current.state,
                QueueState::Dispatched | QueueState::Cancelled
            ) && current.claim_revision == released_revision
            {
                return Ok(current);
            }
            return Err(data_error("execution admission is not owned"));
        }
        let (_, _) = load_owned_admission(&tx, ownership)?;
        if !matches!(current.state, QueueState::Claimed | QueueState::Dispatched) {
            return Err(data_error("execution admission cannot be released"));
        }
        if current.state == QueueState::Claimed
            && !current.interactive
            && load_snapshot_optional(&tx, &current.run_id)?.is_some()
        {
            release_orchestration_budget_before_dispatch(&tx, &current, released_at_ms)?;
        }
        let next_state = if current.state == QueueState::Dispatched {
            "dispatched"
        } else {
            "cancelled"
        };
        let next_revision = ownership
            .claim_revision
            .checked_add(1)
            .ok_or_else(|| data_error("execution admission revision overflow"))?;
        let changed = tx.execute(
            "UPDATE orchestration_queue
                SET state = ?1, owner_incarnation_id = NULL, lease_id = NULL,
                    claim_revision = ?2
              WHERE queue_id = ?3 AND candidate_sha256 = ?4
                AND owner_incarnation_id = ?5 AND lease_id = ?6
                AND claim_revision = ?7 AND state IN ('claimed', 'dispatched')",
            params![
                next_state,
                checked_i64(next_revision, "released claim revision")?,
                ownership.queue_id,
                ownership.expected_candidate_sha256,
                ownership.owner_incarnation_id.as_str(),
                ownership.lease_id.as_str(),
                checked_i64(ownership.claim_revision, "queue claim revision")?,
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "execution admission release lost its compare-and-swap",
            ));
        }
        let deleted = tx.execute(
            "DELETE FROM orchestration_leases
              WHERE lease_id = ?1 AND resource_kind = 'capacity' AND resource_id = ?2
                AND owner_incarnation_id = ?3 AND revision = ?4",
            params![
                ownership.lease_id.as_str(),
                ownership.queue_id,
                ownership.owner_incarnation_id.as_str(),
                checked_i64(ownership.claim_revision, "lease revision")?,
            ],
        )?;
        if deleted != 1 {
            return Err(data_error(
                "execution admission lease release was not exact",
            ));
        }
        let updated = load_queue_record_by_id(&tx, &ownership.queue_id)?
            .ok_or_else(|| data_error("released execution queue record disappeared"))?;
        tx.commit()?;
        if failpoint == Some(BudgetExecutionFailpoint::AfterReleaseCommit) {
            return Err(injected_failure("after_budget_release_commit"));
        }
        Ok(updated)
    }

    /// Releases an attempt reservation after a post-marker adapter receipt has
    /// positively proved that no provider process or prompt was accepted.
    /// Capacity remains owned until the caller separately verifies worktree
    /// cleanup through `ExecutionAdmissionService`. No other post-marker state
    /// is eligible.
    pub fn release_verified_not_started_budget(
        &self,
        run_id: &RunId,
        attempt_id: &RunAttemptId,
        released_at_ms: u64,
    ) -> Result<StoredBudgetReservation, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_verified_transaction_snapshot(&tx, run_id)?;
        let dispatch = load_attempt_dispatches(&tx, run_id)?
            .into_iter()
            .find(|record| record.attempt_id == *attempt_id)
            .ok_or_else(|| data_error("verified not-started attempt was not found"))?;
        let effect = load_stored_effect_by_id(&tx, &dispatch.effect_id)?
            .ok_or_else(|| data_error("verified not-started effect was not found"))?;
        if dispatch.state != AttemptDispatchState::NotStarted
            || dispatch.last_boundary != AttemptBoundary::SafeToRetry
            || dispatch.outcome_digest.is_none()
            || effect.delivery.state != EffectDeliveryState::OutcomeRecorded
            || effect
                .delivery
                .safe_error_code
                .as_deref()
                .is_none_or(str::is_empty)
            || effect.delivery.result_digest != dispatch.outcome_digest
            || released_at_ms < dispatch.updated_at_ms
        {
            return Err(data_error(
                "budget release lacks an exact cleanup-verified not-started receipt",
            ));
        }
        let mut stored = load_budget_reservations(&tx, run_id)?
            .into_iter()
            .find(|record| record.reservation.attempt_id == *attempt_id)
            .ok_or_else(|| data_error("verified not-started budget reservation was not found"))?;
        if stored.reservation.state == BudgetReservationState::Released {
            tx.commit()?;
            return Ok(stored);
        }
        if stored.reservation.state != BudgetReservationState::Reserved
            || released_at_ms < stored.updated_at_ms
            || current
                .reservations
                .get(&stored.reservation.id)
                .is_none_or(|reservation| reservation != &stored.reservation)
        {
            return Err(data_error(
                "verified not-started budget reservation is not releasable",
            ));
        }
        let turn_attempt = tx
            .query_row(
                "SELECT boundary, partial_output FROM turn_attempts WHERE id = ?1",
                [dispatch.turn_attempt_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or_else(|| data_error("verified not-started turn attempt was not found"))?;
        let boundary: AttemptBoundary = serde_json::from_str(&turn_attempt.0)?;
        let tool_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM turn_tool_ledger WHERE turn_id = ?1",
            [dispatch.turn_id.as_str()],
            |row| row.get(0),
        )?;
        if boundary != AttemptBoundary::SafeToRetry || !turn_attempt.1.is_empty() || tool_count != 0
        {
            return Err(data_error(
                "verified not-started attempt contains provider-visible evidence",
            ));
        }

        let kinds = pandamux_orchestrator::decide(
            &current,
            RunCommand::ReleaseBudget {
                reservation_id: stored.reservation.id.clone(),
            },
        )
        .map_err(orchestration_error)?;
        if kinds.len() != 1 {
            return Err(data_error(
                "verified not-started budget release did not produce one event",
            ));
        }
        let event = RunEvent {
            event_id: RunEventId::new(format!(
                "run-event-budget-not-started-release-{}",
                stored.reservation.id.as_str()
            )),
            run_id: run_id.clone(),
            seq: current
                .last_seq
                .checked_add(1)
                .ok_or_else(|| data_error("orchestration event sequence overflow"))?,
            at_ms: released_at_ms,
            idempotency_key: format!(
                "budget-not-started-release:{}",
                stored.reservation.id.as_str()
            ),
            kind: kinds.into_iter().next().expect("one budget release event"),
        };
        if transition_events_collide(&tx, std::slice::from_ref(&event))? {
            return Err(data_error(
                "verified not-started budget release identity collided",
            ));
        }
        let applied =
            pandamux_orchestrator::apply(&current, &event).map_err(orchestration_error)?;
        insert_event(&tx, &event)?;
        for derived in applied.new_effects {
            insert_effect(
                &tx,
                &stored_effect_from_run(&applied.run, std::slice::from_ref(&event), derived)?,
            )?;
        }
        update_run_snapshot(
            &tx,
            run_id,
            &applied.run,
            current.last_seq,
            &orchestration_snapshot_sha256(&current),
        )?;
        release_daily_budget(&tx, &stored, released_at_ms)?;
        let previous = stored.clone();
        stored.reservation = applied
            .run
            .reservations
            .get(&stored.reservation.id)
            .cloned()
            .ok_or_else(|| data_error("released budget reservation disappeared"))?;
        stored.updated_at_ms = released_at_ms;
        update_budget_reservation_row(&tx, &previous, &stored)?;

        // Capacity remains owned until the run lifecycle has checkpointed and
        // removed (or deliberately retained) the exact attempt worktree. The
        // shared admission service releases it only after that cleanup proof;
        // this transaction is authoritative solely for the no-usage budget.
        tx.commit()?;
        Ok(stored)
    }

    pub fn reconcile_execution_capacity(
        &self,
        policy: &AdmissionPolicySnapshot,
    ) -> Result<AdmissionCapacityReconciliation, StoreError> {
        let conn = self.conn.lock().unwrap();
        validate_admission_policy(&conn, policy)?;
        let records = load_all_queue_records(&conn)?;
        let counts = active_admission_counts(&conn, &records)?;
        Ok(capacity_reconciliation(policy, &counts))
    }
}

impl Store {
    pub fn grant_orchestration_budget(
        &self,
        grant: &SupplementalBudgetGrant,
        granted_at_ms: u64,
        local_control_authenticated: bool,
        failpoint: Option<BudgetExecutionFailpoint>,
    ) -> Result<BudgetGrantOutcome, StoreError> {
        validate_budget_failpoint(
            failpoint,
            &[
                BudgetExecutionFailpoint::BeforeGrantCommit,
                BudgetExecutionFailpoint::AfterGrantCommit,
            ],
        )?;
        if !local_control_authenticated {
            return Err(data_error(
                "supplemental budget grants require connected local control",
            ));
        }
        if grant.action_digest != supplemental_budget_action_digest(grant) {
            return Err(data_error(
                "supplemental budget grant action digest does not bind its exact request",
            ));
        }
        if failpoint == Some(BudgetExecutionFailpoint::BeforeGrantCommit) {
            return Err(injected_failure("before_budget_grant_commit"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_verified_transaction_snapshot(&tx, &grant.run_id)?;
        if current
            .reservations
            .values()
            .find(|reservation| reservation.attempt_id == grant.attempt_id)
            .is_some_and(|reservation| reservation.state == BudgetReservationState::Released)
        {
            return Err(data_error(
                "supplemental budget cannot target a released attempt",
            ));
        }
        if let Some(existing) = current.budget_grants.get(&grant.id) {
            if existing != grant {
                return Err(data_error(
                    "supplemental budget grant identity changed on replay",
                ));
            }
            let outcome = BudgetGrantOutcome {
                run: current.clone(),
                snapshot_sha256: orchestration_snapshot_sha256(&current),
                grant: existing.clone(),
                duplicate: true,
            };
            tx.commit()?;
            return Ok(outcome);
        }

        let kinds = pandamux_orchestrator::decide(
            &current,
            RunCommand::GrantSupplementalBudget {
                grant: grant.clone(),
            },
        )
        .map_err(orchestration_error)?;
        if kinds.len() != 1 {
            return Err(data_error(
                "supplemental budget grant did not produce one event",
            ));
        }
        let event = RunEvent {
            event_id: RunEventId::new(format!("run-event-budget-grant-{}", grant.id.as_str())),
            run_id: grant.run_id.clone(),
            seq: current
                .last_seq
                .checked_add(1)
                .ok_or_else(|| data_error("orchestration event sequence overflow"))?,
            at_ms: granted_at_ms,
            idempotency_key: format!("budget-grant:{}", grant.id.as_str()),
            kind: kinds.into_iter().next().expect("one grant event"),
        };
        if transition_events_collide(&tx, std::slice::from_ref(&event))? {
            return Err(data_error(
                "supplemental budget event identity already exists",
            ));
        }
        let outcome =
            pandamux_orchestrator::apply(&current, &event).map_err(orchestration_error)?;
        insert_event(&tx, &event)?;
        for effect in outcome.new_effects {
            insert_effect(
                &tx,
                &stored_effect_from_run(&outcome.run, std::slice::from_ref(&event), effect)?,
            )?;
        }
        update_run_snapshot(
            &tx,
            &grant.run_id,
            &outcome.run,
            current.last_seq,
            &orchestration_snapshot_sha256(&current),
        )?;
        if let Some(mut reservation) = load_budget_reservations(&tx, &grant.run_id)?
            .into_iter()
            .find(|record| record.reservation.attempt_id == grant.attempt_id)
        {
            if reservation
                .grants
                .insert(grant.id.clone(), grant.clone())
                .is_some()
            {
                return Err(data_error("supplemental budget grant was already attached"));
            }
            update_budget_grants(&tx, &reservation, granted_at_ms)?;
            if grant.dimension == BudgetDimension::CostMicroUsd {
                add_daily_budget_grant(&tx, &reservation, grant.amount, granted_at_ms)?;
            }
        }
        tx.commit()?;
        if failpoint == Some(BudgetExecutionFailpoint::AfterGrantCommit) {
            return Err(injected_failure("after_budget_grant_commit"));
        }
        Ok(BudgetGrantOutcome {
            snapshot_sha256: orchestration_snapshot_sha256(&outcome.run),
            run: outcome.run,
            grant: grant.clone(),
            duplicate: false,
        })
    }

    pub fn reconcile_orchestration_budget(
        &self,
        run_id: &RunId,
        reservation_id: &BudgetReservationId,
        reconciled_at_ms: u64,
        failpoint: Option<BudgetExecutionFailpoint>,
    ) -> Result<BudgetReconciliationOutcome, StoreError> {
        validate_budget_failpoint(
            failpoint,
            &[
                BudgetExecutionFailpoint::BeforeReconciliationCommit,
                BudgetExecutionFailpoint::AfterReconciliationCommit,
            ],
        )?;
        if failpoint == Some(BudgetExecutionFailpoint::BeforeReconciliationCommit) {
            return Err(injected_failure("before_budget_reconciliation_commit"));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_verified_transaction_snapshot(&tx, run_id)?;
        let mut stored = load_budget_reservations(&tx, run_id)?
            .into_iter()
            .find(|record| record.reservation.id == *reservation_id)
            .ok_or_else(|| data_error("budget reservation was not found"))?;
        if reconciled_at_ms < stored.updated_at_ms {
            return Err(data_error(
                "budget reconciliation timestamp moved backwards",
            ));
        }
        let evidence = normalize_managed_attempt_usage(&tx, &current, &stored)?;
        if stored.reconciliation.as_ref() == Some(&evidence)
            && stored.reservation.actual == evidence.actual
            && stored.reservation.state != BudgetReservationState::Reserved
        {
            let daily = load_daily_budget(
                &tx,
                &stored.organization_id,
                &stored.reservation.ledger_utc_date,
            )?
            .ok_or_else(|| data_error("budget reservation UTC ledger was not found"))?;
            let outcome = BudgetReconciliationOutcome {
                run: current.clone(),
                snapshot_sha256: orchestration_snapshot_sha256(&current),
                reservation: stored,
                daily_budget: daily,
                duplicate: true,
            };
            tx.commit()?;
            return Ok(outcome);
        }
        if stored
            .reconciliation
            .as_ref()
            .is_some_and(|existing| existing.source_sha256 == evidence.source_sha256)
        {
            return Err(data_error(
                "budget reconciliation source identity changed durable bytes",
            ));
        }

        let previous_stored = stored.clone();
        let reconcile_kind = pandamux_orchestrator::decide(
            &current,
            RunCommand::ReconcileBudget {
                reservation_id: reservation_id.clone(),
                actual: evidence.actual,
            },
        )
        .map_err(orchestration_error)?;
        if reconcile_kind.len() != 1 {
            return Err(data_error(
                "budget reconciliation did not produce one event",
            ));
        }
        let digest_prefix = evidence.source_sha256[..16].to_string();
        let reconcile_event = RunEvent {
            event_id: RunEventId::new(format!(
                "run-event-budget-reconcile-{}-{digest_prefix}",
                reservation_id.as_str()
            )),
            run_id: run_id.clone(),
            seq: current
                .last_seq
                .checked_add(1)
                .ok_or_else(|| data_error("orchestration event sequence overflow"))?,
            at_ms: reconciled_at_ms,
            idempotency_key: format!(
                "budget-reconcile:{}:{digest_prefix}",
                reservation_id.as_str()
            ),
            kind: reconcile_kind
                .into_iter()
                .next()
                .expect("one reconciliation event"),
        };
        let applied = pandamux_orchestrator::apply(&current, &reconcile_event)
            .map_err(orchestration_error)?;
        let mut next = applied.run;
        let mut events = vec![reconcile_event];
        let mut effects = applied.new_effects;
        let next_reservation = next
            .reservations
            .get(reservation_id)
            .cloned()
            .ok_or_else(|| data_error("reconciled reservation disappeared from run"))?;
        let daily = reconcile_daily_budget(&tx, &stored, &next_reservation, reconciled_at_ms)?;
        stored.reservation = next_reservation;
        stored.reconciliation = Some(evidence);
        stored.updated_at_ms = reconciled_at_ms;
        let agent_overage = agent_budget_overage(&tx, &stored)?;
        let daily_overage = daily_budget_overage(&daily)?;
        if (amount_nonzero_server(agent_overage) || daily_overage != 0)
            && matches!(next.status, RunStatus::Ready | RunStatus::Running)
        {
            let kinds = pandamux_orchestrator::decide(
                &next,
                RunCommand::Pause {
                    reason: "budget_scope_overage".to_string(),
                },
            )
            .map_err(orchestration_error)?;
            if kinds.len() != 1 {
                return Err(data_error("budget overage pause did not produce one event"));
            }
            let event = RunEvent {
                event_id: RunEventId::new(format!(
                    "run-event-budget-pause-{}-{digest_prefix}",
                    reservation_id.as_str()
                )),
                run_id: run_id.clone(),
                seq: next
                    .last_seq
                    .checked_add(1)
                    .ok_or_else(|| data_error("orchestration event sequence overflow"))?,
                at_ms: reconciled_at_ms,
                idempotency_key: format!(
                    "budget-pause:{}:{digest_prefix}",
                    reservation_id.as_str()
                ),
                kind: kinds.into_iter().next().expect("one pause event"),
            };
            let applied =
                pandamux_orchestrator::apply(&next, &event).map_err(orchestration_error)?;
            next = applied.run;
            effects.extend(applied.new_effects);
            events.push(event);
        }
        if transition_events_collide(&tx, &events)? {
            return Err(data_error(
                "budget reconciliation event identity already exists",
            ));
        }
        for event in &events {
            insert_event(&tx, event)?;
        }
        for effect in effects {
            insert_effect(&tx, &stored_effect_from_run(&next, &events, effect)?)?;
        }
        update_run_snapshot(
            &tx,
            run_id,
            &next,
            current.last_seq,
            &orchestration_snapshot_sha256(&current),
        )?;
        update_reconciled_budget_reservation(&tx, &previous_stored, &stored)?;
        tx.commit()?;
        if failpoint == Some(BudgetExecutionFailpoint::AfterReconciliationCommit) {
            return Err(injected_failure("after_budget_reconciliation_commit"));
        }
        Ok(BudgetReconciliationOutcome {
            snapshot_sha256: orchestration_snapshot_sha256(&next),
            run: next,
            reservation: stored,
            daily_budget: daily,
            duplicate: false,
        })
    }

    pub fn audit_budget_conservation(
        &self,
        run_id: &RunId,
    ) -> Result<BudgetConservationReport, StoreError> {
        let conn = self.conn.lock().unwrap();
        let run = load_verified_transaction_snapshot(&conn, run_id)?;
        let reservations = load_budget_reservations(&conn, run_id)?;
        budget_conservation_report(&conn, &run, &reservations)
    }
}

#[derive(Default)]
struct ActiveAdmissionCounts {
    global: u32,
    orchestrated_global: u32,
    environments: BTreeMap<EnvironmentId, u32>,
    orchestrated_environments: BTreeMap<EnvironmentId, u32>,
    providers: BTreeMap<(EnvironmentId, ProviderInstanceId), u32>,
    organizations: BTreeMap<OrganizationId, u32>,
    runs: BTreeMap<RunId, u32>,
    run_limits: BTreeMap<RunId, u32>,
    agents: BTreeMap<AgentId, u32>,
    agent_limits: BTreeMap<AgentId, u32>,
}

fn validate_admission_claim_request(
    request: &AdmissionClaimStoreRequest,
) -> Result<(), StoreError> {
    if request.queue_id.trim().is_empty()
        || !is_sha256(&request.expected_candidate_sha256)
        || request.owner_incarnation_id.as_str().trim().is_empty()
        || request.lease_id.as_str().trim().is_empty()
        || request.lease_expires_at_ms <= request.claimed_at_ms
    {
        return Err(data_error("execution admission claim is malformed"));
    }
    Ok(())
}

fn validate_admission_ownership(ownership: &AdmissionOwnership) -> Result<(), StoreError> {
    if ownership.queue_id.trim().is_empty()
        || !is_sha256(&ownership.expected_candidate_sha256)
        || ownership.owner_incarnation_id.as_str().trim().is_empty()
        || ownership.lease_id.as_str().trim().is_empty()
        || ownership.claim_revision == 0
    {
        return Err(data_error("execution admission ownership is malformed"));
    }
    Ok(())
}

fn validate_admission_policy(
    conn: &rusqlite::Connection,
    policy: &AdmissionPolicySnapshot,
) -> Result<(), StoreError> {
    if policy.global_limit == 0
        || policy
            .environment_limits
            .values()
            .any(|limit| *limit == 0 || *limit > policy.global_limit)
        || policy.provider_limits.values().any(|limit| *limit == 0)
        || policy.organization_limits.values().any(|limit| *limit == 0)
    {
        return Err(data_error("execution admission policy is malformed"));
    }
    if let Some(expected) = policy.settings_revision {
        let actual = conn
            .query_row(
                "SELECT revision FROM settings WHERE key = 'user_settings'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map(|value| checked_u64(value, "settings revision"))
            .transpose()?;
        if actual != Some(expected) {
            return Err(data_error(
                "execution admission policy settings revision changed",
            ));
        }
    }
    Ok(())
}

fn load_queue_record_by_id(
    conn: &rusqlite::Connection,
    queue_id: &str,
) -> Result<Option<OrchestrationQueueRecord>, StoreError> {
    let run_id = conn
        .query_row(
            "SELECT run_id FROM orchestration_queue WHERE queue_id = ?1",
            [queue_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(run_id) = run_id else {
        return Ok(None);
    };
    Ok(load_queue_records(conn, &RunId::from(run_id))?
        .into_iter()
        .find(|record| record.queue_id == queue_id))
}

fn load_all_queue_records(
    conn: &rusqlite::Connection,
) -> Result<Vec<OrchestrationQueueRecord>, StoreError> {
    let mut statement = conn.prepare("SELECT DISTINCT run_id FROM orchestration_queue")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut run_ids = Vec::new();
    for row in rows {
        run_ids.push(RunId::from(row?));
    }
    drop(statement);
    let mut records = Vec::new();
    for run_id in run_ids {
        records.extend(load_queue_records(conn, &run_id)?);
    }
    Ok(records)
}

fn load_owned_admission(
    conn: &rusqlite::Connection,
    ownership: &AdmissionOwnership,
) -> Result<(OrchestrationQueueRecord, ExecutionLease), StoreError> {
    let record = load_queue_record_by_id(conn, &ownership.queue_id)?
        .ok_or_else(|| data_error("execution queue record was not found"))?;
    if record.candidate_sha256 != ownership.expected_candidate_sha256
        || record.owner_incarnation_id.as_ref() != Some(&ownership.owner_incarnation_id)
        || record.lease_id.as_ref() != Some(&ownership.lease_id)
        || record.claim_revision != ownership.claim_revision
    {
        return Err(data_error("execution admission ownership changed"));
    }
    let lease = load_lease(conn, &ownership.lease_id)?
        .ok_or_else(|| data_error("execution admission lease was not found"))?;
    if lease.resource_kind != LeaseResourceKind::Capacity
        || lease.resource_id != ownership.queue_id
        || lease.owner_incarnation_id != ownership.owner_incarnation_id
        || lease.revision != ownership.claim_revision
    {
        return Err(data_error("execution admission lease ownership changed"));
    }
    Ok((record, lease))
}

fn active_admission_counts(
    conn: &rusqlite::Connection,
    records: &[OrchestrationQueueRecord],
) -> Result<ActiveAdmissionCounts, StoreError> {
    let mut counts = ActiveAdmissionCounts::default();
    for record in records.iter().filter(|record| {
        matches!(record.state, QueueState::Claimed | QueueState::Dispatched)
            && record.owner_incarnation_id.is_some()
    }) {
        let lease_id = record
            .lease_id
            .as_ref()
            .ok_or_else(|| data_error("active execution admission has no lease"))?;
        let lease = load_lease(conn, lease_id)?
            .ok_or_else(|| data_error("active execution admission lease disappeared"))?;
        if lease.resource_kind != LeaseResourceKind::Capacity
            || lease.resource_id != record.queue_id
            || record.owner_incarnation_id.as_ref() != Some(&lease.owner_incarnation_id)
            || lease.revision != record.claim_revision
        {
            return Err(data_error(
                "active execution admission lease differs from its queue row",
            ));
        }
        increment_count(&mut counts.global)?;
        increment_map(&mut counts.environments, record.environment_id.clone())?;
        increment_map(
            &mut counts.providers,
            (
                record.environment_id.clone(),
                record.provider_instance_id.clone(),
            ),
        )?;
        if let Some(organization_id) = record.organization_id.clone() {
            increment_map(&mut counts.organizations, organization_id)?;
        }
        if !record.interactive {
            increment_count(&mut counts.orchestrated_global)?;
            increment_map(
                &mut counts.orchestrated_environments,
                record.environment_id.clone(),
            )?;
            if record.organization_id.is_none() {
                return Err(data_error("orchestrated admission has no organization"));
            }
            let agent_limit = record
                .agent_concurrency_limit
                .ok_or_else(|| data_error("orchestrated admission has no agent limit"))?;
            increment_map(&mut counts.runs, record.run_id.clone())?;
            increment_map(&mut counts.agents, record.agent_id.clone())?;
            counts
                .agent_limits
                .entry(record.agent_id.clone())
                .and_modify(|limit| *limit = (*limit).min(agent_limit))
                .or_insert(agent_limit);
            let run_limit = match load_snapshot_optional(conn, &record.run_id)? {
                Some(run) => run.max_parallel_tasks,
                None => {
                    remote_prepare_result_for_queue(conn, record)?
                        .ok_or_else(|| {
                            data_error("remote orchestration admission lost its prepare receipt")
                        })?
                        .prepared_execution
                        .run_concurrency_limit
                }
            };
            counts
                .run_limits
                .entry(record.run_id.clone())
                .and_modify(|limit| *limit = (*limit).min(run_limit))
                .or_insert(run_limit);
        }
    }
    Ok(counts)
}

fn increment_count(value: &mut u32) -> Result<(), StoreError> {
    *value = value
        .checked_add(1)
        .ok_or_else(|| data_error("execution admission count overflow"))?;
    Ok(())
}

fn increment_map<K: Ord>(map: &mut BTreeMap<K, u32>, key: K) -> Result<(), StoreError> {
    increment_count(map.entry(key).or_default())
}

fn admission_has_capacity(
    conn: &rusqlite::Connection,
    record: &OrchestrationQueueRecord,
    counts: &ActiveAdmissionCounts,
    policy: &AdmissionPolicySnapshot,
) -> Result<bool, StoreError> {
    let environment_limit = policy
        .environment_limits
        .get(&record.environment_id)
        .copied()
        .unwrap_or_else(|| {
            pandamux_core::DEFAULT_ENVIRONMENT_EXECUTION_LIMIT.min(policy.global_limit)
        });
    let provider_limit = policy
        .provider_limits
        .get(&(
            record.environment_id.clone(),
            record.provider_instance_id.clone(),
        ))
        .copied();
    let provider_count = counts
        .providers
        .get(&(
            record.environment_id.clone(),
            record.provider_instance_id.clone(),
        ))
        .copied()
        .unwrap_or(0);
    if counts.global >= policy.global_limit
        || counts
            .environments
            .get(&record.environment_id)
            .copied()
            .unwrap_or(0)
            >= environment_limit
        || provider_limit.is_none_or(|limit| provider_count >= limit)
    {
        return Ok(false);
    }
    if let Some(organization_id) = record.organization_id.as_ref()
        && policy
            .organization_limits
            .get(organization_id)
            .is_some_and(|limit| {
                counts
                    .organizations
                    .get(organization_id)
                    .copied()
                    .unwrap_or(0)
                    >= *limit
            })
    {
        return Ok(false);
    }
    if record.interactive {
        return Ok(true);
    }
    if (policy.global_limit >= 2
        && counts.orchestrated_global >= policy.global_limit.saturating_sub(1))
        || (environment_limit >= 2
            && counts
                .orchestrated_environments
                .get(&record.environment_id)
                .copied()
                .unwrap_or(0)
                >= environment_limit.saturating_sub(1))
    {
        return Ok(false);
    }
    if record.organization_id.is_none() {
        return Err(data_error("orchestrated queue record has no organization"));
    }
    let agent_limit = record
        .agent_concurrency_limit
        .ok_or_else(|| data_error("orchestrated queue record has no agent limit"))?;
    if counts.agents.get(&record.agent_id).copied().unwrap_or(0) >= agent_limit {
        return Ok(false);
    }
    let run_limit = match load_snapshot_optional(conn, &record.run_id)? {
        Some(run) => run.max_parallel_tasks,
        None => {
            remote_prepare_result_for_queue(conn, record)?
                .ok_or_else(|| {
                    data_error("remote orchestration admission lost its prepare receipt")
                })?
                .prepared_execution
                .run_concurrency_limit
        }
    };
    Ok(counts.runs.get(&record.run_id).copied().unwrap_or(0) < run_limit)
}

fn admission_order_key(
    record: &&OrchestrationQueueRecord,
) -> (u64, RunId, RunTaskId, RunAttemptId) {
    (
        record.enqueued_at_ms,
        record.run_id.clone(),
        record.task_id.clone(),
        record.attempt_id.clone(),
    )
}

fn capacity_reconciliation(
    policy: &AdmissionPolicySnapshot,
    counts: &ActiveAdmissionCounts,
) -> AdmissionCapacityReconciliation {
    let mut overages = Vec::new();
    push_capacity_overage(
        &mut overages,
        "global",
        "global",
        counts.global,
        policy.global_limit,
    );
    for (environment_id, active) in &counts.environments {
        let limit = policy
            .environment_limits
            .get(environment_id)
            .copied()
            .unwrap_or_else(|| {
                pandamux_core::DEFAULT_ENVIRONMENT_EXECUTION_LIMIT.min(policy.global_limit)
            });
        push_capacity_overage(
            &mut overages,
            "environment",
            environment_id.as_str(),
            *active,
            limit,
        );
    }
    for ((environment_id, provider_id), active) in &counts.providers {
        if let Some(limit) = policy
            .provider_limits
            .get(&(environment_id.clone(), provider_id.clone()))
        {
            push_capacity_overage(
                &mut overages,
                "provider",
                &format!("{}:{}", environment_id.as_str(), provider_id.as_str()),
                *active,
                *limit,
            );
        }
    }
    for (organization_id, active) in &counts.organizations {
        if let Some(limit) = policy.organization_limits.get(organization_id) {
            push_capacity_overage(
                &mut overages,
                "organization",
                organization_id.as_str(),
                *active,
                *limit,
            );
        }
    }
    for (run_id, active) in &counts.runs {
        if let Some(limit) = counts.run_limits.get(run_id) {
            push_capacity_overage(&mut overages, "run", run_id.as_str(), *active, *limit);
        }
    }
    for (agent_id, active) in &counts.agents {
        if let Some(limit) = counts.agent_limits.get(agent_id) {
            push_capacity_overage(&mut overages, "agent", agent_id.as_str(), *active, *limit);
        }
    }
    overages.sort_by(|left, right| {
        (&left.dimension, &left.resource_id).cmp(&(&right.dimension, &right.resource_id))
    });
    AdmissionCapacityReconciliation {
        active_claims: counts.global,
        overages,
    }
}

fn push_capacity_overage(
    overages: &mut Vec<AdmissionCapacityOverage>,
    dimension: &str,
    resource_id: &str,
    active: u32,
    limit: u32,
) {
    if active > limit {
        overages.push(AdmissionCapacityOverage {
            dimension: dimension.to_string(),
            resource_id: resource_id.to_string(),
            active,
            limit,
        });
    }
}

fn apply_admission_orchestration_transition(
    tx: &Transaction<'_>,
    queue: &OrchestrationQueueRecord,
    transition: &OrchestratedAdmissionTransition,
    at_ms: u64,
) -> Result<(), StoreError> {
    let reservation = &transition.reservation;
    let attempt = &transition.attempt;
    if queue.interactive
        || reservation.run_id != queue.run_id
        || reservation.agent_id != queue.agent_id
        || Some(&reservation.organization_id) != queue.organization_id.as_ref()
        || reservation.reservation.task_id != queue.task_id
        || reservation.reservation.attempt_id != queue.attempt_id
        || attempt.id != queue.attempt_id
        || attempt.task_id != queue.task_id
        || attempt.route.selected_agent_id.as_ref() != Some(&queue.agent_id)
        || attempt
            .route
            .provider
            .selected_provider_instance_id
            .as_ref()
            != Some(&queue.provider_instance_id)
        || reservation.reconciliation.is_some()
        || amount_exceeds_sqlite(&reservation.agent_budget_limit)
        || reservation.reservation.ledger_utc_date != execution_ledger_utc_date(at_ms)?
    {
        return Err(data_error(
            "orchestrated admission transition differs from its queue identity",
        ));
    }
    let current = load_snapshot(tx, &queue.run_id)?;
    let events = load_events(tx, &queue.run_id)?;
    let (replayed, effects) = replay_with_effects(&events)?;
    if pandamux_orchestrator::canonical_run_bytes(&current)
        != pandamux_orchestrator::canonical_run_bytes(&replayed)
    {
        return Err(data_error(
            "orchestration replay differs from stored snapshot during admission",
        ));
    }
    verify_effect_rows(&effects, &load_effects(tx, &queue.run_id)?)?;
    if grants_for_attempt_at(&current, &reservation.reservation.attempt_id, Some(at_ms))
        != reservation.grants
    {
        return Err(data_error(
            "execution admission grants differ from the canonical run",
        ));
    }
    validate_agent_budget_for_admission(tx, reservation)?;
    let expected_last_seq = current.last_seq;
    let expected_snapshot_sha256 = orchestration_snapshot_sha256(&current);
    let commands = [
        (
            RunCommand::ReserveBudget {
                reservation: reservation.reservation.clone(),
            },
            RunEventId::new(format!(
                "run-event-admission-budget-{}",
                reservation.reservation.id.as_str()
            )),
            format!(
                "execution-admission:budget:{}",
                reservation.reservation.id.as_str()
            ),
        ),
        (
            RunCommand::RequestDispatch {
                attempt: attempt.clone(),
            },
            RunEventId::new(format!(
                "run-event-admission-dispatch-{}",
                attempt.id.as_str()
            )),
            format!("execution-admission:dispatch:{}", attempt.id.as_str()),
        ),
    ];
    let mut next = current.clone();
    let mut new_events = Vec::new();
    let mut new_effects = Vec::new();
    for (command, event_id, idempotency_key) in commands {
        let kinds = pandamux_orchestrator::decide(&next, command).map_err(orchestration_error)?;
        if kinds.len() != 1 {
            return Err(data_error(
                "execution admission transition did not produce one event",
            ));
        }
        let event = RunEvent {
            event_id,
            run_id: queue.run_id.clone(),
            seq: next
                .last_seq
                .checked_add(1)
                .ok_or_else(|| data_error("orchestration event sequence overflow"))?,
            at_ms,
            idempotency_key,
            kind: kinds.into_iter().next().expect("one decided event"),
        };
        let outcome = pandamux_orchestrator::apply(&next, &event).map_err(orchestration_error)?;
        next = outcome.run;
        new_effects.extend(outcome.new_effects);
        new_events.push(event);
    }
    if transition_events_collide(tx, &new_events)? {
        return Err(data_error(
            "execution admission event identity already exists",
        ));
    }
    for event in &new_events {
        insert_event(tx, event)?;
    }
    for effect in new_effects {
        insert_effect(tx, &stored_effect_from_run(&next, &new_events, effect)?)?;
    }
    update_run_snapshot(
        tx,
        &queue.run_id,
        &next,
        expected_last_seq,
        &expected_snapshot_sha256,
    )?;
    reserve_daily_budget_for_admission(
        tx,
        reservation,
        transition.daily_budget_limit_micro_usd,
        at_ms,
    )?;
    insert_budget_reservation(tx, reservation)?;
    Ok(())
}

fn reserve_daily_budget_for_admission(
    tx: &Transaction<'_>,
    reservation: &StoredBudgetReservation,
    limit_micro_usd: u64,
    updated_at_ms: u64,
) -> Result<(), StoreError> {
    if limit_micro_usd > i64::MAX as u64 {
        return Err(data_error(
            "execution admission daily budget limit is malformed",
        ));
    }
    let amount = reservation.reservation.amount.cost_micro_usd;
    let granted = reservation
        .grants
        .values()
        .try_fold(0_u64, |total, grant| {
            if grant.dimension == BudgetDimension::CostMicroUsd {
                total
                    .checked_add(grant.amount)
                    .ok_or_else(|| data_error("daily budget grant overflow"))
            } else {
                Ok(total)
            }
        })?;
    let current = load_daily_budget(
        tx,
        &reservation.organization_id,
        &reservation.reservation.ledger_utc_date,
    )?;
    let record = if let Some(current) = current {
        if current.limit_micro_usd != limit_micro_usd || updated_at_ms < current.updated_at_ms {
            return Err(data_error(
                "execution admission daily budget policy changed",
            ));
        }
        let reserved_micro_usd = current
            .reserved_micro_usd
            .checked_add(amount)
            .ok_or_else(|| data_error("daily budget reservation overflow"))?;
        let granted_micro_usd = current
            .granted_micro_usd
            .checked_add(granted)
            .ok_or_else(|| data_error("daily budget grant overflow"))?;
        let revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| data_error("daily budget revision overflow"))?;
        let available = current
            .limit_micro_usd
            .checked_add(granted_micro_usd)
            .ok_or_else(|| data_error("daily budget available amount overflow"))?;
        let committed = reserved_micro_usd
            .checked_add(current.reconciled_micro_usd)
            .ok_or_else(|| data_error("daily budget committed amount overflow"))?;
        if committed > available {
            return Err(data_error("daily budget limit would be exceeded"));
        }
        let changed = tx.execute(
            "UPDATE orchestration_daily_budgets
                SET reserved_micro_usd = ?1, granted_micro_usd = ?2,
                    revision = ?3, updated_at_ms = ?4
              WHERE organization_id = ?5 AND ledger_utc_date = ?6
                AND limit_micro_usd = ?7 AND reserved_micro_usd = ?8
                AND reconciled_micro_usd = ?9 AND granted_micro_usd = ?10
                AND revision = ?11 AND updated_at_ms = ?12",
            params![
                checked_i64(reserved_micro_usd, "daily budget reserved")?,
                checked_i64(granted_micro_usd, "daily budget granted")?,
                checked_i64(revision, "daily budget revision")?,
                checked_i64(updated_at_ms, "daily budget updated at")?,
                current.organization_id.as_str(),
                current.ledger_utc_date,
                checked_i64(current.limit_micro_usd, "daily budget limit")?,
                checked_i64(current.reserved_micro_usd, "prior daily budget reserved")?,
                checked_i64(
                    current.reconciled_micro_usd,
                    "prior daily budget reconciled"
                )?,
                checked_i64(current.granted_micro_usd, "prior daily budget granted")?,
                checked_i64(current.revision, "prior daily budget revision")?,
                checked_i64(current.updated_at_ms, "prior daily budget updated at")?,
            ],
        )?;
        if changed != 1 {
            return Err(data_error(
                "daily budget reservation lost its compare-and-swap",
            ));
        }
        DailyBudgetRecord {
            reserved_micro_usd,
            granted_micro_usd,
            revision,
            updated_at_ms,
            ..current
        }
    } else {
        let available = limit_micro_usd
            .checked_add(granted)
            .ok_or_else(|| data_error("daily budget available amount overflow"))?;
        if amount > available {
            return Err(data_error("daily budget limit would be exceeded"));
        }
        let record = DailyBudgetRecord {
            organization_id: reservation.organization_id.clone(),
            ledger_utc_date: reservation.reservation.ledger_utc_date.clone(),
            limit_micro_usd,
            reserved_micro_usd: amount,
            reconciled_micro_usd: 0,
            granted_micro_usd: granted,
            revision: 1,
            updated_at_ms,
        };
        insert_daily_budget(tx, &record)?;
        record
    };
    if record.reserved_micro_usd < amount {
        return Err(data_error("daily budget reservation was not recorded"));
    }
    Ok(())
}

fn validate_agent_budget_for_admission(
    conn: &rusqlite::Connection,
    incoming: &StoredBudgetReservation,
) -> Result<(), StoreError> {
    let existing = load_budget_reservations(conn, &incoming.run_id)?;
    let mut required = BudgetAmount::default();
    for record in existing
        .iter()
        .filter(|record| record.agent_id == incoming.agent_id)
    {
        if record.agent_budget_limit != incoming.agent_budget_limit {
            return Err(data_error(
                "execution admission compiled agent budget changed",
            ));
        }
        required = checked_add_budget(required, committed_budget_after_grants(record)?)?;
    }
    required = checked_add_budget(required, committed_budget_after_grants(incoming)?)?;
    if !budget_within(required, incoming.agent_budget_limit) {
        return Err(data_error("compiled agent budget limit would be exceeded"));
    }
    Ok(())
}

fn committed_budget_after_grants(
    record: &StoredBudgetReservation,
) -> Result<BudgetAmount, StoreError> {
    let reservation = &record.reservation;
    let retains_unknown = matches!(
        reservation.state,
        BudgetReservationState::Reserved | BudgetReservationState::PartiallyReconciled
    );
    let committed = BudgetAmount {
        tokens: reservation.actual.tokens.unwrap_or(if retains_unknown {
            reservation.amount.tokens
        } else {
            0
        }),
        cost_micro_usd: reservation
            .actual
            .cost_micro_usd
            .unwrap_or(if retains_unknown {
                reservation.amount.cost_micro_usd
            } else {
                0
            }),
        runtime_ms: reservation.actual.runtime_ms.unwrap_or(if retains_unknown {
            reservation.amount.runtime_ms
        } else {
            0
        }),
    };
    let grants = grant_amount(&record.grants)?;
    Ok(BudgetAmount {
        tokens: committed.tokens.saturating_sub(grants.tokens),
        cost_micro_usd: committed
            .cost_micro_usd
            .saturating_sub(grants.cost_micro_usd),
        runtime_ms: committed.runtime_ms.saturating_sub(grants.runtime_ms),
    })
}

fn grant_amount(
    grants: &BTreeMap<BudgetGrantId, SupplementalBudgetGrant>,
) -> Result<BudgetAmount, StoreError> {
    grants
        .values()
        .try_fold(BudgetAmount::default(), |mut total, grant| {
            let target = match grant.dimension {
                BudgetDimension::Tokens => &mut total.tokens,
                BudgetDimension::CostMicroUsd => &mut total.cost_micro_usd,
                BudgetDimension::RuntimeMs => &mut total.runtime_ms,
            };
            *target = target
                .checked_add(grant.amount)
                .ok_or_else(|| data_error("supplemental budget grant overflow"))?;
            Ok(total)
        })
}

fn checked_add_budget(left: BudgetAmount, right: BudgetAmount) -> Result<BudgetAmount, StoreError> {
    Ok(BudgetAmount {
        tokens: left
            .tokens
            .checked_add(right.tokens)
            .ok_or_else(|| data_error("budget token total overflow"))?,
        cost_micro_usd: left
            .cost_micro_usd
            .checked_add(right.cost_micro_usd)
            .ok_or_else(|| data_error("budget cost total overflow"))?,
        runtime_ms: left
            .runtime_ms
            .checked_add(right.runtime_ms)
            .ok_or_else(|| data_error("budget runtime total overflow"))?,
    })
}

fn budget_within(value: BudgetAmount, limit: BudgetAmount) -> bool {
    value.tokens <= limit.tokens
        && value.cost_micro_usd <= limit.cost_micro_usd
        && value.runtime_ms <= limit.runtime_ms
}

/// Converts a Unix millisecond timestamp to the exact UTC ledger date without
/// depending on a host locale or wall-clock timezone.
pub fn execution_ledger_utc_date(at_ms: u64) -> Result<String, StoreError> {
    let days = at_ms / 86_400_000;
    let z = i128::from(days) + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    if !(1970..=9999).contains(&year) {
        return Err(data_error(
            "execution admission UTC ledger date is out of range",
        ));
    }
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

fn load_verified_transaction_snapshot(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Run, StoreError> {
    let snapshot = load_snapshot(conn, run_id)?;
    let events = load_events(conn, run_id)?;
    let (replayed, effects) = replay_with_effects(&events)?;
    if pandamux_orchestrator::canonical_run_bytes(&snapshot)
        != pandamux_orchestrator::canonical_run_bytes(&replayed)
    {
        return Err(data_error(
            "orchestration replay differs from stored snapshot during budget operation",
        ));
    }
    verify_effect_rows(&effects, &load_effects(conn, run_id)?)?;
    Ok(snapshot)
}

fn release_orchestration_budget_before_dispatch(
    tx: &Transaction<'_>,
    queue: &OrchestrationQueueRecord,
    released_at_ms: u64,
) -> Result<(), StoreError> {
    let current = load_verified_transaction_snapshot(tx, &queue.run_id)?;
    if load_effects(tx, &queue.run_id)?.iter().any(|effect| {
        effect.delivery.dispatch_started_at_ms.is_some()
            && effect_matches_attempt(
                &effect.effect,
                &queue.run_id,
                &queue.task_id,
                &queue.attempt_id,
                None,
            )
    }) {
        return Err(data_error(
            "budget cannot be released after the durable dispatch marker",
        ));
    }
    let previous = load_budget_reservations(tx, &queue.run_id)?
        .into_iter()
        .find(|record| record.reservation.attempt_id == queue.attempt_id)
        .ok_or_else(|| data_error("pre-dispatch admission has no budget reservation"))?;
    if previous.reservation.state != BudgetReservationState::Reserved
        || previous.reservation.task_id != queue.task_id
        || previous.agent_id != queue.agent_id
        || released_at_ms < previous.updated_at_ms
        || current
            .reservations
            .get(&previous.reservation.id)
            .is_none_or(|reservation| reservation != &previous.reservation)
    {
        return Err(data_error(
            "pre-dispatch budget release differs from its admission ownership",
        ));
    }
    let kinds = pandamux_orchestrator::decide(
        &current,
        RunCommand::ReleaseBudget {
            reservation_id: previous.reservation.id.clone(),
        },
    )
    .map_err(orchestration_error)?;
    if kinds.len() != 1 {
        return Err(data_error(
            "pre-dispatch budget release did not produce one event",
        ));
    }
    let event = RunEvent {
        event_id: RunEventId::new(format!(
            "run-event-budget-release-{}",
            previous.reservation.id.as_str()
        )),
        run_id: queue.run_id.clone(),
        seq: current
            .last_seq
            .checked_add(1)
            .ok_or_else(|| data_error("orchestration event sequence overflow"))?,
        at_ms: released_at_ms,
        idempotency_key: format!("budget-release:{}", previous.reservation.id.as_str()),
        kind: kinds.into_iter().next().expect("one budget release event"),
    };
    if transition_events_collide(tx, std::slice::from_ref(&event))? {
        return Err(data_error("pre-dispatch budget release identity collided"));
    }
    let applied = pandamux_orchestrator::apply(&current, &event).map_err(orchestration_error)?;
    insert_event(tx, &event)?;
    for effect in applied.new_effects {
        insert_effect(
            tx,
            &stored_effect_from_run(&applied.run, std::slice::from_ref(&event), effect)?,
        )?;
    }
    update_run_snapshot(
        tx,
        &queue.run_id,
        &applied.run,
        current.last_seq,
        &orchestration_snapshot_sha256(&current),
    )?;
    release_daily_budget(tx, &previous, released_at_ms)?;
    let mut released = previous.clone();
    released.reservation = applied
        .run
        .reservations
        .get(&previous.reservation.id)
        .cloned()
        .ok_or_else(|| data_error("released budget reservation disappeared"))?;
    released.updated_at_ms = released_at_ms;
    update_budget_reservation_row(tx, &previous, &released)?;
    Ok(())
}

fn update_budget_grants(
    tx: &Transaction<'_>,
    record: &StoredBudgetReservation,
    updated_at_ms: u64,
) -> Result<(), StoreError> {
    if updated_at_ms < record.updated_at_ms {
        return Err(data_error(
            "supplemental budget grant timestamp moved backwards",
        ));
    }
    let changed = tx.execute(
        "UPDATE orchestration_budget_reservations
            SET grants_json = ?1, updated_at_ms = ?2
          WHERE reservation_id = ?3 AND run_id = ?4 AND updated_at_ms = ?5",
        params![
            serde_json::to_string(&record.grants)?,
            checked_i64(updated_at_ms, "budget grant updated at")?,
            record.reservation.id.as_str(),
            record.run_id.as_str(),
            checked_i64(record.updated_at_ms, "prior budget grant updated at")?,
        ],
    )?;
    if changed != 1 {
        return Err(data_error(
            "supplemental budget grant lost its reservation compare-and-swap",
        ));
    }
    Ok(())
}

fn add_daily_budget_grant(
    tx: &Transaction<'_>,
    reservation: &StoredBudgetReservation,
    amount: u64,
    updated_at_ms: u64,
) -> Result<(), StoreError> {
    let current = load_daily_budget(
        tx,
        &reservation.organization_id,
        &reservation.reservation.ledger_utc_date,
    )?
    .ok_or_else(|| data_error("supplemental budget grant has no UTC daily ledger"))?;
    if updated_at_ms < current.updated_at_ms {
        return Err(data_error(
            "supplemental daily grant timestamp moved backwards",
        ));
    }
    let granted = current
        .granted_micro_usd
        .checked_add(amount)
        .ok_or_else(|| data_error("supplemental daily budget grant overflow"))?;
    let revision = current
        .revision
        .checked_add(1)
        .ok_or_else(|| data_error("daily budget revision overflow"))?;
    let changed = tx.execute(
        "UPDATE orchestration_daily_budgets
            SET granted_micro_usd = ?1, revision = ?2, updated_at_ms = ?3
          WHERE organization_id = ?4 AND ledger_utc_date = ?5
            AND granted_micro_usd = ?6 AND revision = ?7 AND updated_at_ms = ?8",
        params![
            checked_i64(granted, "daily budget granted")?,
            checked_i64(revision, "daily budget revision")?,
            checked_i64(updated_at_ms, "daily budget updated at")?,
            current.organization_id.as_str(),
            current.ledger_utc_date,
            checked_i64(current.granted_micro_usd, "prior daily budget granted")?,
            checked_i64(current.revision, "prior daily budget revision")?,
            checked_i64(current.updated_at_ms, "prior daily budget updated at")?,
        ],
    )?;
    if changed != 1 {
        return Err(data_error(
            "supplemental daily budget grant lost its compare-and-swap",
        ));
    }
    Ok(())
}

fn release_daily_budget(
    tx: &Transaction<'_>,
    reservation: &StoredBudgetReservation,
    updated_at_ms: u64,
) -> Result<DailyBudgetRecord, StoreError> {
    let current = load_daily_budget(
        tx,
        &reservation.organization_id,
        &reservation.reservation.ledger_utc_date,
    )?
    .ok_or_else(|| data_error("budget release has no UTC daily ledger"))?;
    if updated_at_ms < current.updated_at_ms {
        return Err(data_error("daily budget release timestamp moved backwards"));
    }
    let cost_grants = grant_amount(&reservation.grants)?.cost_micro_usd;
    let reserved = current
        .reserved_micro_usd
        .checked_sub(reservation.reservation.amount.cost_micro_usd)
        .ok_or_else(|| data_error("daily budget release underflow"))?;
    let granted = current
        .granted_micro_usd
        .checked_sub(cost_grants)
        .ok_or_else(|| data_error("daily budget grant release underflow"))?;
    let revision = current
        .revision
        .checked_add(1)
        .ok_or_else(|| data_error("daily budget revision overflow"))?;
    let changed = tx.execute(
        "UPDATE orchestration_daily_budgets
            SET reserved_micro_usd = ?1, granted_micro_usd = ?2,
                revision = ?3, updated_at_ms = ?4
          WHERE organization_id = ?5 AND ledger_utc_date = ?6
            AND reserved_micro_usd = ?7 AND granted_micro_usd = ?8
            AND revision = ?9 AND updated_at_ms = ?10",
        params![
            checked_i64(reserved, "released daily budget reserved")?,
            checked_i64(granted, "released daily budget granted")?,
            checked_i64(revision, "released daily budget revision")?,
            checked_i64(updated_at_ms, "released daily budget updated at")?,
            current.organization_id.as_str(),
            current.ledger_utc_date,
            checked_i64(current.reserved_micro_usd, "prior daily budget reserved")?,
            checked_i64(current.granted_micro_usd, "prior daily budget granted")?,
            checked_i64(current.revision, "prior daily budget revision")?,
            checked_i64(current.updated_at_ms, "prior daily budget updated at")?,
        ],
    )?;
    if changed != 1 {
        return Err(data_error("daily budget release lost its compare-and-swap"));
    }
    Ok(DailyBudgetRecord {
        reserved_micro_usd: reserved,
        granted_micro_usd: granted,
        revision,
        updated_at_ms,
        ..current
    })
}

fn reconcile_daily_budget(
    tx: &Transaction<'_>,
    previous: &StoredBudgetReservation,
    next: &BudgetReservation,
    updated_at_ms: u64,
) -> Result<DailyBudgetRecord, StoreError> {
    let current = load_daily_budget(
        tx,
        &previous.organization_id,
        &previous.reservation.ledger_utc_date,
    )?
    .ok_or_else(|| data_error("budget reconciliation has no UTC daily ledger"))?;
    if updated_at_ms < current.updated_at_ms {
        return Err(data_error(
            "daily budget reconciliation timestamp moved backwards",
        ));
    }
    if previous.reservation.actual.cost_micro_usd.is_some() || next.actual.cost_micro_usd.is_none()
    {
        return Ok(current);
    }
    let actual = next
        .actual
        .cost_micro_usd
        .expect("known reconciled cost was checked");
    let reserved = current
        .reserved_micro_usd
        .checked_sub(previous.reservation.amount.cost_micro_usd)
        .ok_or_else(|| data_error("daily budget reconciliation underflow"))?;
    let reconciled = current
        .reconciled_micro_usd
        .checked_add(actual)
        .ok_or_else(|| data_error("daily budget reconciliation overflow"))?;
    let revision = current
        .revision
        .checked_add(1)
        .ok_or_else(|| data_error("daily budget revision overflow"))?;
    let changed = tx.execute(
        "UPDATE orchestration_daily_budgets
            SET reserved_micro_usd = ?1, reconciled_micro_usd = ?2,
                revision = ?3, updated_at_ms = ?4
          WHERE organization_id = ?5 AND ledger_utc_date = ?6
            AND reserved_micro_usd = ?7 AND reconciled_micro_usd = ?8
            AND revision = ?9 AND updated_at_ms = ?10",
        params![
            checked_i64(reserved, "reconciled daily budget reserved")?,
            checked_i64(reconciled, "reconciled daily budget actual")?,
            checked_i64(revision, "reconciled daily budget revision")?,
            checked_i64(updated_at_ms, "reconciled daily budget updated at")?,
            current.organization_id.as_str(),
            current.ledger_utc_date,
            checked_i64(current.reserved_micro_usd, "prior daily budget reserved")?,
            checked_i64(
                current.reconciled_micro_usd,
                "prior daily budget reconciled"
            )?,
            checked_i64(current.revision, "prior daily budget revision")?,
            checked_i64(current.updated_at_ms, "prior daily budget updated at")?,
        ],
    )?;
    if changed != 1 {
        return Err(data_error(
            "daily budget reconciliation lost its compare-and-swap",
        ));
    }
    Ok(DailyBudgetRecord {
        reserved_micro_usd: reserved,
        reconciled_micro_usd: reconciled,
        revision,
        updated_at_ms,
        ..current
    })
}

fn update_reconciled_budget_reservation(
    tx: &Transaction<'_>,
    previous: &StoredBudgetReservation,
    next: &StoredBudgetReservation,
) -> Result<(), StoreError> {
    update_budget_reservation_row(tx, previous, next)
}

fn update_budget_reservation_row(
    tx: &Transaction<'_>,
    previous: &StoredBudgetReservation,
    next: &StoredBudgetReservation,
) -> Result<(), StoreError> {
    if previous.run_id != next.run_id
        || previous.reservation.id != next.reservation.id
        || previous.reservation.task_id != next.reservation.task_id
        || previous.reservation.attempt_id != next.reservation.attempt_id
        || previous.agent_id != next.agent_id
        || previous.organization_id != next.organization_id
        || previous.agent_budget_limit != next.agent_budget_limit
        || previous.reservation.amount != next.reservation.amount
        || previous.reservation.ledger_utc_date != next.reservation.ledger_utc_date
        || previous.created_at_ms != next.created_at_ms
        || next.updated_at_ms < previous.updated_at_ms
    {
        return Err(data_error("budget reservation immutable identity changed"));
    }
    let changed = tx.execute(
        "UPDATE orchestration_budget_reservations
            SET actual_tokens = ?1, actual_micro_usd = ?2, actual_runtime_ms = ?3,
                state = ?4, grants_json = ?5, reconciliation_json = ?6,
                updated_at_ms = ?7
          WHERE reservation_id = ?8 AND run_id = ?9 AND state = ?10
            AND updated_at_ms = ?11",
        params![
            next.reservation
                .actual
                .tokens
                .map(|value| checked_i64(value, "actual tokens"))
                .transpose()?,
            next.reservation
                .actual
                .cost_micro_usd
                .map(|value| checked_i64(value, "actual cost"))
                .transpose()?,
            next.reservation
                .actual
                .runtime_ms
                .map(|value| checked_i64(value, "actual runtime"))
                .transpose()?,
            enum_text(next.reservation.state)?,
            serde_json::to_string(&next.grants)?,
            next.reconciliation
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            checked_i64(next.updated_at_ms, "budget reservation updated at")?,
            previous.reservation.id.as_str(),
            previous.run_id.as_str(),
            enum_text(previous.reservation.state)?,
            checked_i64(
                previous.updated_at_ms,
                "prior budget reservation updated at"
            )?,
        ],
    )?;
    if changed != 1 {
        return Err(data_error(
            "budget reservation update lost its compare-and-swap",
        ));
    }
    Ok(())
}

fn agent_budget_overage(
    conn: &rusqlite::Connection,
    updated: &StoredBudgetReservation,
) -> Result<BudgetAmount, StoreError> {
    let mut reservations = load_budget_reservations(conn, &updated.run_id)?;
    let stored = reservations
        .iter_mut()
        .find(|record| record.reservation.id == updated.reservation.id)
        .ok_or_else(|| data_error("reconciled agent reservation disappeared"))?;
    *stored = updated.clone();
    let committed = reservations
        .iter()
        .filter(|record| record.agent_id == updated.agent_id)
        .try_fold(BudgetAmount::default(), |total, record| {
            if record.agent_budget_limit != updated.agent_budget_limit {
                return Err(data_error(
                    "compiled agent budget changed during reconciliation",
                ));
            }
            checked_add_budget(total, committed_budget_after_grants(record)?)
        })?;
    Ok(BudgetAmount {
        tokens: committed
            .tokens
            .saturating_sub(updated.agent_budget_limit.tokens),
        cost_micro_usd: committed
            .cost_micro_usd
            .saturating_sub(updated.agent_budget_limit.cost_micro_usd),
        runtime_ms: committed
            .runtime_ms
            .saturating_sub(updated.agent_budget_limit.runtime_ms),
    })
}

fn daily_budget_overage(daily: &DailyBudgetRecord) -> Result<u64, StoreError> {
    let committed = daily
        .reserved_micro_usd
        .checked_add(daily.reconciled_micro_usd)
        .ok_or_else(|| data_error("daily budget committed total overflow"))?;
    let available = daily
        .limit_micro_usd
        .checked_add(daily.granted_micro_usd)
        .ok_or_else(|| data_error("daily budget available total overflow"))?;
    Ok(committed.saturating_sub(available))
}

fn amount_nonzero_server(amount: BudgetAmount) -> bool {
    amount.tokens != 0 || amount.cost_micro_usd != 0 || amount.runtime_ms != 0
}

fn normalize_managed_attempt_usage(
    conn: &rusqlite::Connection,
    run: &Run,
    stored: &StoredBudgetReservation,
) -> Result<BudgetReconciliationEvidence, StoreError> {
    let attempt = run
        .attempts
        .get(&stored.reservation.attempt_id)
        .ok_or_else(|| data_error("budget reservation attempt was not found"))?;
    let settled = matches!(
        attempt.status,
        AttemptStatus::Succeeded
            | AttemptStatus::Failed
            | AttemptStatus::Cancelled
            | AttemptStatus::Interrupted
    ) && attempt.ended_at_ms.is_some();
    let cancellation_checkpointed = run.status == RunStatus::Cancelling
        && attempt.status == AttemptStatus::Running
        && attempt.checkpoint_ids.iter().any(|checkpoint_id| {
            run.checkpoints
                .get(checkpoint_id)
                .is_some_and(|checkpoint| checkpoint.kind == CheckpointKind::AfterAttempt)
        });
    if !settled && !cancellation_checkpointed {
        return Err(data_error(
            "budget usage can reconcile only after exact settlement or cancellation checkpoint",
        ));
    }
    let thread_id = attempt
        .thread_id
        .as_ref()
        .ok_or_else(|| data_error("settled budget attempt has no thread identity"))?;
    let turn_attempt_id = attempt
        .turn_attempt_id
        .as_ref()
        .ok_or_else(|| data_error("settled budget attempt has no turn-attempt identity"))?;
    let selected_provider = attempt
        .route
        .provider
        .selected_provider_instance_id
        .as_ref()
        .ok_or_else(|| data_error("settled budget attempt has no provider identity"))?;

    let turn_attempt = conn
        .query_row(
            "SELECT turn_id, thread_id, environment_id, provider_instance_id,
                    model, status, started_at_ms, ended_at_ms
               FROM turn_attempts WHERE id = ?1",
            [turn_attempt_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| data_error("settled budget turn attempt was not found"))?;
    let turn_id = TurnId::from(turn_attempt.0);
    let stored_thread_id = ThreadId::from(turn_attempt.1);
    let environment_id = EnvironmentId::from(turn_attempt.2);
    let provider_instance_id = ProviderInstanceId::from(turn_attempt.3);
    let turn_model = turn_attempt.4;
    let turn_status: TurnAttemptStatus = serde_json::from_str(&turn_attempt.5)
        .map_err(|_| data_error("turn attempt status is invalid"))?;
    let turn_started_at_ms = checked_u64(turn_attempt.6, "turn attempt started at")?;
    let turn_ended_at_ms = turn_attempt
        .7
        .map(|value| checked_u64(value, "turn attempt ended at"))
        .transpose()?;
    if stored_thread_id != *thread_id
        || provider_instance_id != *selected_provider
        || !matches!(
            turn_status,
            TurnAttemptStatus::Completed
                | TurnAttemptStatus::Failed
                | TurnAttemptStatus::InterruptedForCapacity
        )
    {
        return Err(data_error(
            "settled budget turn attempt differs from the canonical attempt",
        ));
    }
    let runtime_ms = turn_ended_at_ms
        .map(|ended| {
            ended
                .checked_sub(turn_started_at_ms)
                .ok_or_else(|| data_error("turn attempt runtime underflow"))
        })
        .transpose()?;

    let mut statement = conn.prepare(
        "SELECT observation_id, source_kind, parser_version,
                provider_native_session_id, provider_native_turn_id,
                occurred_at_ms, provider, provider_instance_id, subscription_key,
                model, input_tokens, cached_input_tokens, cache_write_tokens,
                output_tokens, reasoning_tokens, other_tokens_json,
                unknown_fields_json, provider_reported_micro_usd, source_record_count,
                thread_id, turn_id, agent_id, schedule_id, fact_kind
           FROM usage_observations
          WHERE environment_id = ?1 AND source_kind = ?2 AND turn_id = ?3
          ORDER BY observation_id",
    )?;
    let mut rows = statement.query(params![
        environment_id.as_str(),
        UsageSourceKind::ManagedTurn.as_str(),
        turn_id.as_str(),
    ])?;
    let mut observations = Vec::new();
    while let Some(row) = rows.next()? {
        observations.push(decode_usage_observation(row, 0)?);
    }
    drop(rows);
    drop(statement);
    for observation in &observations {
        if observation.provider_instance_id != provider_instance_id
            || observation.thread_id.as_ref() != Some(thread_id)
            || observation.turn_id.as_ref() != Some(&turn_id)
            || observation.agent_id.as_ref() != Some(&stored.agent_id)
        {
            return Err(data_error(
                "managed usage fact differs from the exact attempt identity",
            ));
        }
    }

    let components = observations
        .iter()
        .filter(|observation| observation.fact_kind == UsageFactKind::TokenComponent)
        .collect::<Vec<_>>();
    let aggregates = observations
        .iter()
        .filter(|observation| observation.fact_kind == UsageFactKind::Aggregate)
        .collect::<Vec<_>>();
    if components.is_empty() && aggregates.len() > 1 {
        return Err(data_error(
            "managed usage contains ambiguous aggregate facts",
        ));
    }
    let token_facts = if components.is_empty() {
        aggregates
    } else {
        components
    };
    let token_unknown = token_facts.iter().any(|observation| {
        [
            UsageUnknownField::InputTokens,
            UsageUnknownField::OutputTokens,
            UsageUnknownField::CacheReadTokens,
            UsageUnknownField::CacheWriteTokens,
        ]
        .iter()
        .any(|field| observation.unknown_fields.contains(field))
    });
    let tokens = if token_facts.is_empty() || token_unknown {
        None
    } else {
        let mut total = TokenCounts::default();
        for observation in &token_facts {
            total
                .checked_accumulate(&observation.tokens)
                .map_err(|error| data_error(error.to_string()))?;
        }
        Some(
            total
                .checked_total()
                .map_err(|error| data_error(error.to_string()))?,
        )
    };

    let provider_costs = observations
        .iter()
        .filter_map(|observation| observation.provider_reported_micro_usd)
        .collect::<Vec<_>>();
    if provider_costs.len() > 1 {
        return Err(data_error(
            "managed usage contains multiple provider-reported turn costs",
        ));
    }
    let mut pricing_version_ids = BTreeSet::new();
    let (cost_micro_usd, cost_source) = if let Some(cost) = provider_costs.first().copied() {
        (Some(cost), BudgetCostSource::ProviderReported)
    } else if token_facts.is_empty() || token_unknown {
        (None, BudgetCostSource::Unknown)
    } else {
        let mut estimated = Some(0_u64);
        for observation in &token_facts {
            let Some((pricing_version_id, rates)) =
                pricing_for_usage_observation(conn, observation)?
            else {
                estimated = None;
                break;
            };
            let Some(fact_cost) = estimate_usage_micro_usd(&observation.tokens, &rates)
                .map_err(|error| data_error(error.to_string()))?
            else {
                estimated = None;
                break;
            };
            pricing_version_ids.insert(pricing_version_id);
            estimated = Some(
                estimated
                    .expect("known accumulated usage estimate")
                    .checked_add(fact_cost)
                    .ok_or_else(|| data_error("usage price estimate overflow"))?,
            );
        }
        if estimated.is_none() {
            pricing_version_ids.clear();
        }
        (
            estimated,
            if estimated.is_some() {
                BudgetCostSource::EffectiveDatedEstimate
            } else {
                BudgetCostSource::Unknown
            },
        )
    };
    let actual = BudgetUsage {
        tokens,
        cost_micro_usd,
        runtime_ms,
    };
    let observation_ids = observations
        .iter()
        .map(|observation| observation.id.clone())
        .collect::<BTreeSet<_>>();
    let source_bytes = serde_json::to_vec(&(
        &stored.run_id,
        &stored.reservation.id,
        &stored.reservation.attempt_id,
        &turn_id,
        turn_attempt_id,
        thread_id,
        selected_provider,
        &turn_model,
        turn_started_at_ms,
        turn_ended_at_ms,
        &observations,
        actual,
        cost_source,
        &pricing_version_ids,
    ))?;
    Ok(BudgetReconciliationEvidence {
        turn_id,
        turn_attempt_id: turn_attempt_id.clone(),
        usage_observation_ids: observation_ids,
        source_sha256: sha256_hex(&source_bytes),
        cost_source,
        pricing_version_ids,
        actual,
    })
}

fn pricing_for_usage_observation(
    conn: &rusqlite::Connection,
    observation: &UsageObservation,
) -> Result<Option<(PricingVersionId, UsagePricingRates)>, StoreError> {
    if observation
        .unknown_fields
        .contains(&UsageUnknownField::OccurredAt)
        || observation
            .unknown_fields
            .contains(&UsageUnknownField::Model)
    {
        return Ok(None);
    }
    let (Some(occurred_at_ms), Some(model)) = (observation.occurred_at_ms, &observation.model)
    else {
        return Ok(None);
    };
    let row = conn
        .query_row(
            "WITH selected_version AS (
                SELECT id
                  FROM usage_pricing_versions
                 WHERE provider = ?1 AND effective_from_ms <= ?2
                 ORDER BY effective_from_ms DESC, id DESC
                 LIMIT 1
             )
             SELECT selected_version.id,
                    rule.input_micro_usd_per_million,
                    rule.cached_input_micro_usd_per_million,
                    rule.cache_write_micro_usd_per_million,
                    rule.output_micro_usd_per_million,
                    rule.reasoning_micro_usd_per_million
               FROM selected_version
               LEFT JOIN usage_pricing_aliases model_alias
                 ON model_alias.pricing_version_id = selected_version.id
                AND model_alias.alias_model_id = ?3
               JOIN usage_pricing_rules rule
                 ON rule.pricing_version_id = selected_version.id
                AND rule.canonical_model_id = COALESCE(
                    model_alias.canonical_model_id,
                    ?3
                )",
            params![
                observation.provider.as_str(),
                checked_i64(occurred_at_ms, "usage occurrence time")?,
                model,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .optional()?;
    row.map(|row| {
        Ok((
            PricingVersionId::from(row.0),
            UsagePricingRates {
                input_micro_usd_per_million: row
                    .1
                    .map(|value| checked_u64(value, "pricing input rate"))
                    .transpose()?,
                cached_input_micro_usd_per_million: row
                    .2
                    .map(|value| checked_u64(value, "pricing cached-input rate"))
                    .transpose()?,
                cache_write_micro_usd_per_million: row
                    .3
                    .map(|value| checked_u64(value, "pricing cache-write rate"))
                    .transpose()?,
                output_micro_usd_per_million: row
                    .4
                    .map(|value| checked_u64(value, "pricing output rate"))
                    .transpose()?,
                reasoning_micro_usd_per_million: row
                    .5
                    .map(|value| checked_u64(value, "pricing reasoning rate"))
                    .transpose()?,
            },
        ))
    })
    .transpose()
}

fn budget_conservation_report(
    conn: &rusqlite::Connection,
    run: &Run,
    reservations: &[StoredBudgetReservation],
) -> Result<BudgetConservationReport, StoreError> {
    let mut mismatches = Vec::new();
    let mut expected_reserved = BudgetAmount::default();
    let mut expected_reconciled = BudgetAmount::default();
    for reservation in run.reservations.values() {
        if matches!(
            reservation.state,
            BudgetReservationState::Reserved | BudgetReservationState::PartiallyReconciled
        ) {
            if reservation.actual.tokens.is_none() {
                expected_reserved.tokens = expected_reserved
                    .tokens
                    .checked_add(reservation.amount.tokens)
                    .ok_or_else(|| data_error("conservation reserved token overflow"))?;
            }
            if reservation.actual.cost_micro_usd.is_none() {
                expected_reserved.cost_micro_usd = expected_reserved
                    .cost_micro_usd
                    .checked_add(reservation.amount.cost_micro_usd)
                    .ok_or_else(|| data_error("conservation reserved cost overflow"))?;
            }
            if reservation.actual.runtime_ms.is_none() {
                expected_reserved.runtime_ms = expected_reserved
                    .runtime_ms
                    .checked_add(reservation.amount.runtime_ms)
                    .ok_or_else(|| data_error("conservation reserved runtime overflow"))?;
            }
        }
        expected_reconciled = checked_add_budget(
            expected_reconciled,
            BudgetAmount {
                tokens: reservation.actual.tokens.unwrap_or_default(),
                cost_micro_usd: reservation.actual.cost_micro_usd.unwrap_or_default(),
                runtime_ms: reservation.actual.runtime_ms.unwrap_or_default(),
            },
        )?;
        match reservations
            .iter()
            .find(|stored| stored.reservation.id == reservation.id)
        {
            Some(stored) if stored.reservation == *reservation => {
                if matches!(
                    reservation.state,
                    BudgetReservationState::PartiallyReconciled
                        | BudgetReservationState::Reconciled
                ) && stored.reconciliation.is_none()
                {
                    mismatches.push(BudgetConservationMismatch {
                        scope: format!("reservation:{}:source", reservation.id.as_str()),
                        dimension: BudgetDimension::Tokens,
                        expected: 1,
                        actual: 0,
                    });
                }
            }
            _ => mismatches.push(BudgetConservationMismatch {
                scope: format!("reservation:{}:snapshot", reservation.id.as_str()),
                dimension: BudgetDimension::Tokens,
                expected: 1,
                actual: 0,
            }),
        }
    }
    for stored in reservations {
        if !run.reservations.contains_key(&stored.reservation.id) {
            mismatches.push(BudgetConservationMismatch {
                scope: format!("reservation:{}:orphan", stored.reservation.id.as_str()),
                dimension: BudgetDimension::Tokens,
                expected: 0,
                actual: 1,
            });
        }
    }
    push_amount_mismatches(
        &mut mismatches,
        "run:reserved",
        expected_reserved,
        run.budget.reserved,
    );
    push_amount_mismatches(
        &mut mismatches,
        "run:reconciled",
        expected_reconciled,
        run.budget.reconciled,
    );

    let mut scope_overages = Vec::new();
    if amount_nonzero_server(run.budget.overage) {
        scope_overages.push(BudgetScopeOverage {
            scope: "run_or_task".to_string(),
            resource_id: run.id.as_str().to_string(),
            amount: run.budget.overage,
        });
    }
    let agent_ids = reservations
        .iter()
        .map(|record| record.agent_id.clone())
        .collect::<BTreeSet<_>>();
    for agent_id in agent_ids {
        let representative = reservations
            .iter()
            .find(|record| record.agent_id == agent_id)
            .expect("agent identity came from a reservation");
        let overage = agent_budget_overage(conn, representative)?;
        if amount_nonzero_server(overage) {
            scope_overages.push(BudgetScopeOverage {
                scope: "agent".to_string(),
                resource_id: agent_id.as_str().to_string(),
                amount: overage,
            });
        }
    }

    for daily in load_daily_budgets(conn, reservations)? {
        let all_scope_reservations = load_budget_reservations_for_daily_scope(
            conn,
            &daily.organization_id,
            &daily.ledger_utc_date,
        )?;
        let mut expected_daily_reserved = 0_u64;
        let mut expected_daily_reconciled = 0_u64;
        let mut expected_daily_granted = 0_u64;
        for record in &all_scope_reservations {
            if record.reservation.state != BudgetReservationState::Released {
                expected_daily_granted = expected_daily_granted
                    .checked_add(grant_amount(&record.grants)?.cost_micro_usd)
                    .ok_or_else(|| data_error("conservation daily grant overflow"))?;
            }
            if matches!(
                record.reservation.state,
                BudgetReservationState::Reserved | BudgetReservationState::PartiallyReconciled
            ) && record.reservation.actual.cost_micro_usd.is_none()
            {
                expected_daily_reserved = expected_daily_reserved
                    .checked_add(record.reservation.amount.cost_micro_usd)
                    .ok_or_else(|| data_error("conservation daily reserved overflow"))?;
            }
            expected_daily_reconciled = expected_daily_reconciled
                .checked_add(record.reservation.actual.cost_micro_usd.unwrap_or_default())
                .ok_or_else(|| data_error("conservation daily reconciled overflow"))?;
        }
        let scope = format!(
            "daily:{}:{}",
            daily.organization_id.as_str(),
            daily.ledger_utc_date
        );
        push_budget_mismatch(
            &mut mismatches,
            &format!("{scope}:reserved"),
            BudgetDimension::CostMicroUsd,
            expected_daily_reserved,
            daily.reserved_micro_usd,
        );
        push_budget_mismatch(
            &mut mismatches,
            &format!("{scope}:reconciled"),
            BudgetDimension::CostMicroUsd,
            expected_daily_reconciled,
            daily.reconciled_micro_usd,
        );
        push_budget_mismatch(
            &mut mismatches,
            &format!("{scope}:granted"),
            BudgetDimension::CostMicroUsd,
            expected_daily_granted,
            daily.granted_micro_usd,
        );
        let overage = daily_budget_overage(&daily)?;
        if overage != 0 {
            scope_overages.push(BudgetScopeOverage {
                scope: "daily".to_string(),
                resource_id: format!(
                    "{}:{}",
                    daily.organization_id.as_str(),
                    daily.ledger_utc_date
                ),
                amount: BudgetAmount {
                    cost_micro_usd: overage,
                    ..BudgetAmount::default()
                },
            });
        }
    }
    mismatches
        .sort_by(|left, right| (&left.scope, left.dimension).cmp(&(&right.scope, right.dimension)));
    scope_overages.sort_by(|left, right| {
        (&left.scope, &left.resource_id).cmp(&(&right.scope, &right.resource_id))
    });
    Ok(BudgetConservationReport {
        run_id: run.id.clone(),
        verdict: if mismatches.is_empty() {
            BudgetConservationVerdict::BalancedSourceUnverified
        } else {
            BudgetConservationVerdict::Mismatch
        },
        mismatches,
        incomplete: run.budget.incomplete,
        run_overage: run.budget.overage,
        scope_overages,
    })
}

fn load_budget_reservations_for_daily_scope(
    conn: &rusqlite::Connection,
    organization_id: &OrganizationId,
    ledger_utc_date: &str,
) -> Result<Vec<StoredBudgetReservation>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT run_id
           FROM orchestration_budget_reservations
          WHERE organization_id = ?1 AND ledger_utc_date = ?2
          ORDER BY run_id",
    )?;
    let rows = statement.query_map(params![organization_id.as_str(), ledger_utc_date], |row| {
        row.get::<_, String>(0)
    })?;
    let mut run_ids = Vec::new();
    for row in rows {
        run_ids.push(RunId::from(row?));
    }
    drop(statement);
    let mut reservations = Vec::new();
    for run_id in run_ids {
        reservations.extend(load_budget_reservations(conn, &run_id)?.into_iter().filter(
            |record| {
                record.organization_id == *organization_id
                    && record.reservation.ledger_utc_date == ledger_utc_date
            },
        ));
    }
    Ok(reservations)
}

fn push_amount_mismatches(
    mismatches: &mut Vec<BudgetConservationMismatch>,
    scope: &str,
    expected: BudgetAmount,
    actual: BudgetAmount,
) {
    for (dimension, expected, actual) in [
        (BudgetDimension::Tokens, expected.tokens, actual.tokens),
        (
            BudgetDimension::CostMicroUsd,
            expected.cost_micro_usd,
            actual.cost_micro_usd,
        ),
        (
            BudgetDimension::RuntimeMs,
            expected.runtime_ms,
            actual.runtime_ms,
        ),
    ] {
        push_budget_mismatch(mismatches, scope, dimension, expected, actual);
    }
}

fn push_budget_mismatch(
    mismatches: &mut Vec<BudgetConservationMismatch>,
    scope: &str,
    dimension: BudgetDimension,
    expected: u64,
    actual: u64,
) {
    if expected != actual {
        mismatches.push(BudgetConservationMismatch {
            scope: scope.to_string(),
            dimension,
            expected,
            actual,
        });
    }
}

fn load_worktree_by_id(
    conn: &rusqlite::Connection,
    worktree_id: &AttemptWorktreeId,
) -> Result<Option<OwnedAttemptWorktreeRecord>, StoreError> {
    let run_id = conn
        .query_row(
            "SELECT run_id FROM orchestration_worktrees WHERE worktree_id = ?1",
            [worktree_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(run_id) = run_id else {
        return Ok(None);
    };
    Ok(load_worktrees(conn, &RunId::from(run_id))?
        .into_iter()
        .find(|record| record.worktree.id == *worktree_id))
}

fn validate_active_worktree_lease(
    record: &OwnedAttemptWorktreeRecord,
    lease: &ExecutionLease,
    at_ms: u64,
) -> Result<(), StoreError> {
    validate_worktree_lease_identity(record, lease)?;
    if at_ms < lease.last_heartbeat_ms || at_ms >= lease.expires_at_ms {
        return Err(data_error(
            "owned attempt worktree lease is not active for this operation",
        ));
    }
    Ok(())
}

fn validate_worktree_lease_identity(
    record: &OwnedAttemptWorktreeRecord,
    lease: &ExecutionLease,
) -> Result<(), StoreError> {
    if lease.id != record.worktree.lease_id
        || lease.resource_kind != LeaseResourceKind::Worktree
        || lease.resource_id != record.worktree.id.as_str()
        || lease.revision == 0
    {
        return Err(data_error(
            "owned attempt worktree lease identity does not match its resource",
        ));
    }
    Ok(())
}

fn update_worktree_state(
    tx: &Transaction<'_>,
    current: &OwnedAttemptWorktreeRecord,
    dirty_state: WorktreeDirtyState,
    cleanup_eligibility: WorktreeCleanupEligibility,
    updated_at_ms: u64,
) -> Result<(), StoreError> {
    let changed = tx.execute(
        "UPDATE orchestration_worktrees
            SET dirty_state = ?1, cleanup_eligibility = ?2, updated_at_ms = ?3
          WHERE worktree_id = ?4 AND lease_id = ?5 AND dirty_state = ?6
            AND cleanup_eligibility = ?7 AND updated_at_ms = ?8",
        params![
            enum_text(dirty_state)?,
            enum_text(cleanup_eligibility)?,
            checked_i64(updated_at_ms, "worktree state timestamp")?,
            current.worktree.id.as_str(),
            current.worktree.lease_id.as_str(),
            enum_text(current.worktree.dirty_state)?,
            enum_text(current.worktree.cleanup_eligibility)?,
            checked_i64(current.updated_at_ms, "prior worktree timestamp")?,
        ],
    )?;
    if changed != 1 {
        return Err(data_error(
            "owned attempt worktree state lost its compare-and-swap",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_runtime_bindings(
    snapshot: &Run,
    effects: &[StoredOrchestrationEffect],
    attempts: &[AttemptDispatchRecord],
    queue_records: &[OrchestrationQueueRecord],
    worktrees: &[OwnedAttemptWorktreeRecord],
    reservations: &[StoredBudgetReservation],
    artifacts: &[BlackboardEntry],
    progress: &[AttemptProgressRecord],
    reviews: &[RecoveryReview],
    receipts: &[NodeReceiptRecord],
) -> Result<(), StoreError> {
    for attempt in attempts {
        let effect = effects
            .iter()
            .find(|effect| effect.effect.id == attempt.effect_id)
            .ok_or_else(|| data_error("attempt dispatch references a missing effect"))?;
        if !effect_matches_attempt(
            &effect.effect,
            &attempt.run_id,
            &attempt.task_id,
            &attempt.attempt_id,
            Some(&attempt.route),
        ) || snapshot.bundle_sha != attempt.bundle_sha
            || snapshot
                .attempts
                .get(&attempt.attempt_id)
                .is_none_or(|canonical| canonical.route != attempt.route)
        {
            return Err(data_error(
                "attempt dispatch differs from its canonical run or provider effect",
            ));
        }
    }
    for record in queue_records {
        let original_route = snapshot
            .routes
            .get(&record.task_id)
            .ok_or_else(|| data_error("queue row has no canonical task route"))?;
        let route = snapshot
            .attempts
            .get(&record.attempt_id)
            .map(|attempt| &attempt.route)
            .unwrap_or(original_route);
        if !snapshot.tasks.contains_key(&record.task_id)
            || route.selected_agent_id.as_ref() != Some(&record.agent_id)
            || route.provider.selected_provider_instance_id.as_ref()
                != Some(&record.provider_instance_id)
        {
            return Err(data_error(
                "orchestration queue row differs from the canonical run",
            ));
        }
    }
    for record in worktrees {
        if snapshot
            .attempts
            .get(&record.worktree.attempt_id)
            .is_none_or(|attempt| {
                attempt.task_id != record.worktree.task_id
                    || attempt.worktree_id.as_ref() != Some(&record.worktree.id)
                    || !attempt
                        .checkpoint_ids
                        .is_subset(&record.worktree.checkpoint_ids)
            })
        {
            return Err(data_error(
                "owned worktree differs from the canonical attempt owner",
            ));
        }
    }
    for reservation in reservations {
        if snapshot.organization_id != reservation.organization_id
            || snapshot.reservations.get(&reservation.reservation.id)
                != Some(&reservation.reservation)
            || reservation
                .grants
                .iter()
                .any(|(id, grant)| snapshot.budget_grants.get(id) != Some(grant))
        {
            return Err(data_error(
                "budget reservation differs from the canonical run snapshot",
            ));
        }
    }
    validate_loaded_artifact_set(snapshot, artifacts)?;
    for record in progress {
        if snapshot
            .attempts
            .get(&record.attempt_id)
            .is_none_or(|attempt| attempt.task_id != record.task_id)
        {
            return Err(data_error(
                "attempt progress differs from the canonical attempt owner",
            ));
        }
    }
    for review in reviews {
        if snapshot
            .attempts
            .get(&review.attempt_id)
            .is_none_or(|attempt| attempt.task_id != review.task_id)
            || review.effect_id.as_ref().is_some_and(|effect_id| {
                effects.iter().all(|effect| {
                    effect.effect.id != *effect_id
                        || !effect_matches_attempt(
                            &effect.effect,
                            &review.run_id,
                            &review.task_id,
                            &review.attempt_id,
                            None,
                        )
                })
            })
        {
            return Err(data_error(
                "recovery review differs from its canonical attempt or effect",
            ));
        }
    }
    for receipt in receipts {
        if effects.iter().all(|effect| {
            effect.effect.id != receipt.effect_id
                || !effect_matches_attempt(
                    &effect.effect,
                    &receipt.run_id,
                    &receipt.task_id,
                    &receipt.attempt_id,
                    None,
                )
        }) {
            return Err(data_error(
                "node receipt differs from its canonical provider effect",
            ));
        }
    }
    Ok(())
}

fn grants_for_attempt_at(
    snapshot: &Run,
    attempt_id: &RunAttemptId,
    at_ms: Option<u64>,
) -> BTreeMap<BudgetGrantId, SupplementalBudgetGrant> {
    snapshot
        .budget_grants
        .iter()
        .filter(|(_, grant)| {
            grant.attempt_id == *attempt_id && at_ms.is_none_or(|at_ms| grant.expires_at_ms > at_ms)
        })
        .map(|(id, grant)| (id.clone(), grant.clone()))
        .collect()
}

fn effect_matches_attempt(
    effect: &OrchestrationEffect,
    run_id: &RunId,
    task_id: &RunTaskId,
    attempt_id: &RunAttemptId,
    route: Option<&TaskRouteDecision>,
) -> bool {
    match &effect.kind {
        pandamux_core::OrchestrationEffectKind::DispatchTask {
            run_id: effect_run_id,
            task_id: effect_task_id,
            attempt_id: effect_attempt_id,
            route: effect_route,
        } => {
            effect_run_id == run_id
                && effect_task_id == task_id
                && effect_attempt_id == attempt_id
                && route.is_none_or(|route| route == effect_route)
        }
        pandamux_core::OrchestrationEffectKind::CancelAttempt {
            run_id: effect_run_id,
            task_id: effect_task_id,
            attempt_id: effect_attempt_id,
        } => {
            route.is_none()
                && effect_run_id == run_id
                && effect_task_id == task_id
                && effect_attempt_id == attempt_id
        }
        _ => false,
    }
}

fn load_attempt_dispatches(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Vec<AttemptDispatchRecord>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT attempt_id, task_id, effect_id, thread_id, turn_id, turn_attempt_id,
                provider_instance_id, route_bytes, route_sha256, bundle_sha, state,
                last_boundary, last_progress_at_ms, cancel_requested_at_ms,
                ambiguity_reason, outcome_digest, created_at_ms, updated_at_ms
           FROM orchestration_attempt_dispatch
          WHERE run_id = ?1 ORDER BY created_at_ms, attempt_id",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Vec<u8>>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, i64>(12)?,
            row.get::<_, Option<i64>>(13)?,
            row.get::<_, Option<String>>(14)?,
            row.get::<_, Option<String>>(15)?,
            row.get::<_, i64>(16)?,
            row.get::<_, i64>(17)?,
        ))
    })?;
    let mut records = Vec::new();
    for row in rows {
        let (
            attempt_id,
            task_id,
            effect_id,
            thread_id,
            turn_id,
            turn_attempt_id,
            provider_instance_id,
            route_bytes,
            route_sha256,
            bundle_sha,
            state,
            last_boundary,
            last_progress_at_ms,
            cancel_requested_at_ms,
            ambiguity_reason,
            outcome_digest,
            created_at_ms,
            updated_at_ms,
        ) = row?;
        if route_sha256 != sha256_hex(&route_bytes) {
            return Err(data_error("attempt route digest mismatch"));
        }
        let record = AttemptDispatchRecord {
            attempt_id: RunAttemptId::from(attempt_id),
            run_id: run_id.clone(),
            task_id: RunTaskId::from(task_id),
            effect_id: OrchestrationEffectId::from(effect_id),
            thread_id: ThreadId::from(thread_id),
            turn_id: TurnId::from(turn_id),
            turn_attempt_id: TurnAttemptId::from(turn_attempt_id),
            provider_instance_id: pandamux_core::ProviderInstanceId::from(provider_instance_id),
            route: serde_json::from_slice(&route_bytes)?,
            bundle_sha,
            state: parse_enum(&state, "attempt dispatch state")?,
            last_boundary: parse_attempt_boundary(&last_boundary)?,
            last_progress_at_ms: checked_u64(last_progress_at_ms, "attempt progress timestamp")?,
            cancel_requested_at_ms: cancel_requested_at_ms
                .map(|value| checked_u64(value, "attempt cancellation timestamp"))
                .transpose()?,
            ambiguity_reason,
            outcome_digest,
            created_at_ms: checked_u64(created_at_ms, "attempt dispatch created at")?,
            updated_at_ms: checked_u64(updated_at_ms, "attempt dispatch updated at")?,
        };
        validate_attempt_dispatch_record(&record)?;
        records.push(record);
    }
    Ok(records)
}

fn validate_attempt_dispatch_record(record: &AttemptDispatchRecord) -> Result<(), StoreError> {
    let selected_provider = record.route.provider.selected_provider_instance_id.as_ref();
    if record.route.task_id != record.task_id
        || selected_provider != Some(&record.provider_instance_id)
        || !is_sha256(&record.bundle_sha)
        || record.updated_at_ms < record.created_at_ms
        || record.last_progress_at_ms < record.created_at_ms
        || record
            .cancel_requested_at_ms
            .is_some_and(|value| value < record.created_at_ms)
        || (record.state == AttemptDispatchState::Ambiguous
            && record.ambiguity_reason.as_deref().is_none_or(str::is_empty))
        || (record.state == AttemptDispatchState::Terminal
            && record
                .outcome_digest
                .as_deref()
                .is_none_or(|value| !is_sha256(value)))
        || record
            .outcome_digest
            .as_deref()
            .is_some_and(|value| !is_sha256(value))
    {
        return Err(data_error("attempt dispatch row is malformed"));
    }
    Ok(())
}

fn load_queue_records(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Vec<OrchestrationQueueRecord>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT queue_id, task_id, attempt_id, agent_id, organization_id,
                agent_concurrency_limit, provider_instance_id, environment_id,
                candidate_sha256, priority, task_enqueued_at_ms, enqueued_at_ms,
                not_before_ms, state, owner_incarnation_id, lease_id,
                claim_revision, interactive, claimed_at_ms, claim_order,
                interactive_streak
           FROM orchestration_queue WHERE run_id = ?1
          ORDER BY enqueued_at_ms, task_id, attempt_id",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, i64>(10)?,
            row.get::<_, i64>(11)?,
            row.get::<_, i64>(12)?,
            row.get::<_, String>(13)?,
            row.get::<_, Option<String>>(14)?,
            row.get::<_, Option<String>>(15)?,
            row.get::<_, i64>(16)?,
            row.get::<_, bool>(17)?,
            row.get::<_, Option<i64>>(18)?,
            row.get::<_, Option<i64>>(19)?,
            row.get::<_, Option<i64>>(20)?,
        ))
    })?;
    let mut records = Vec::new();
    for row in rows {
        let (
            queue_id,
            task_id,
            attempt_id,
            agent_id,
            organization_id,
            agent_concurrency_limit,
            provider_instance_id,
            environment_id,
            candidate_sha256,
            priority,
            task_enqueued_at_ms,
            enqueued_at_ms,
            not_before_ms,
            state,
            owner_incarnation_id,
            lease_id,
            claim_revision,
            interactive,
            claimed_at_ms,
            claim_order,
            interactive_streak,
        ) = row?;
        let record = OrchestrationQueueRecord {
            queue_id,
            run_id: run_id.clone(),
            task_id: RunTaskId::from(task_id),
            attempt_id: RunAttemptId::from(attempt_id),
            agent_id: AgentId::from(agent_id),
            organization_id: organization_id.map(OrganizationId::from),
            agent_concurrency_limit: agent_concurrency_limit
                .map(|value| checked_u32(value, "agent concurrency limit"))
                .transpose()?,
            provider_instance_id: pandamux_core::ProviderInstanceId::from(provider_instance_id),
            environment_id: pandamux_core::EnvironmentId::from(environment_id),
            candidate_sha256,
            priority,
            task_enqueued_at_ms: checked_u64(task_enqueued_at_ms, "queue task age")?,
            enqueued_at_ms: checked_u64(enqueued_at_ms, "queue timestamp")?,
            not_before_ms: checked_u64(not_before_ms, "queue not-before")?,
            state: parse_enum(&state, "queue state")?,
            owner_incarnation_id: owner_incarnation_id.map(ServerIncarnationId::from),
            lease_id: lease_id.map(ExecutionLeaseId::from),
            claim_revision: checked_u64(claim_revision, "queue claim revision")?,
            interactive,
            claimed_at_ms: claimed_at_ms
                .map(|value| checked_u64(value, "queue claimed timestamp"))
                .transpose()?,
            claim_order: claim_order
                .map(|value| checked_u64(value, "queue claim order"))
                .transpose()?,
            interactive_streak: interactive_streak
                .map(|value| {
                    u8::try_from(value)
                        .map_err(|_| data_error("queue interactive streak is out of range"))
                })
                .transpose()?,
        };
        if !is_sha256(&record.candidate_sha256)
            || record.enqueued_at_ms < record.task_enqueued_at_ms
            || record.not_before_ms < record.enqueued_at_ms
            || (record.owner_incarnation_id.is_none() != record.lease_id.is_none())
            || record.interactive == record.agent_concurrency_limit.is_some()
            || (!record.interactive && record.organization_id.is_none())
            || record
                .agent_concurrency_limit
                .is_some_and(|limit| limit == 0)
            || (record.claimed_at_ms.is_none() != record.claim_order.is_none())
            || (record.claimed_at_ms.is_none() != record.interactive_streak.is_none())
            || record
                .claimed_at_ms
                .is_some_and(|claimed| claimed < record.enqueued_at_ms)
            || record.claim_order == Some(0)
            || record.interactive_streak.is_some_and(|streak| streak > 4)
            || match record.state {
                QueueState::Queued | QueueState::Cancelled => {
                    record.owner_incarnation_id.is_some() || record.lease_id.is_some()
                }
                QueueState::Claimed => {
                    record.owner_incarnation_id.is_none()
                        || record.lease_id.is_none()
                        || record.claimed_at_ms.is_none()
                }
                QueueState::Dispatched => {
                    record.owner_incarnation_id.is_some() && record.claimed_at_ms.is_none()
                }
            }
        {
            return Err(data_error("orchestration queue row is malformed"));
        }
        records.push(record);
    }
    Ok(records)
}

fn load_lease(
    conn: &rusqlite::Connection,
    lease_id: &ExecutionLeaseId,
) -> Result<Option<ExecutionLease>, StoreError> {
    conn.query_row(
        "SELECT resource_kind, resource_id, owner_incarnation_id,
                expires_at_ms, last_heartbeat_ms, revision
           FROM orchestration_leases WHERE lease_id = ?1",
        [lease_id.as_str()],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )
    .optional()?
    .map(|(kind, resource_id, owner, expiry, heartbeat, revision)| {
        let lease = ExecutionLease {
            id: lease_id.clone(),
            resource_kind: parse_enum(&kind, "lease resource kind")?,
            resource_id,
            owner_incarnation_id: ServerIncarnationId::from(owner),
            expires_at_ms: checked_u64(expiry, "lease expiry")?,
            last_heartbeat_ms: checked_u64(heartbeat, "lease heartbeat")?,
            revision: checked_u64(revision, "lease revision")?,
        };
        if lease.resource_id.trim().is_empty()
            || lease.owner_incarnation_id.as_str().is_empty()
            || lease.revision == 0
            || lease.expires_at_ms < lease.last_heartbeat_ms
        {
            return Err(data_error("execution lease row is malformed"));
        }
        Ok(lease)
    })
    .transpose()
}

fn load_worktrees(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Vec<OwnedAttemptWorktreeRecord>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT worktree_id, repository_id, task_id, attempt_id, environment_id,
                git_common_dir_identity, baseline_commit, branch_ref, path, lease_id,
                checkpoint_ids_json, dirty_state, cleanup_eligibility,
                created_at_ms, updated_at_ms
           FROM orchestration_worktrees WHERE run_id = ?1 ORDER BY created_at_ms, worktree_id",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, String>(12)?,
            row.get::<_, i64>(13)?,
            row.get::<_, i64>(14)?,
        ))
    })?;
    let mut records = Vec::new();
    for row in rows {
        let (
            worktree_id,
            repository_id,
            task_id,
            attempt_id,
            environment_id,
            git_common_dir_identity,
            baseline_commit,
            branch_ref,
            path,
            lease_id,
            checkpoint_ids_json,
            dirty_state,
            cleanup_eligibility,
            created_at_ms,
            updated_at_ms,
        ) = row?;
        let checkpoints =
            match serde_json::from_str::<StoredWorktreeCheckpointsCompat>(&checkpoint_ids_json)? {
                StoredWorktreeCheckpointsCompat::Current(checkpoints) => checkpoints,
                StoredWorktreeCheckpointsCompat::Legacy(ids) => StoredWorktreeCheckpoints {
                    ids,
                    intent_sha256: BTreeMap::new(),
                    verified_process_death_sha256: None,
                    retained_evidence_sha256: None,
                },
            };
        let record = OwnedAttemptWorktreeRecord {
            worktree: OwnedAttemptWorktree {
                id: AttemptWorktreeId::from(worktree_id),
                repository_id: ProjectRepositoryId::from(repository_id),
                run_id: run_id.clone(),
                task_id: RunTaskId::from(task_id),
                attempt_id: RunAttemptId::from(attempt_id),
                environment_id: pandamux_core::EnvironmentId::from(environment_id),
                git_common_dir_identity,
                baseline_commit,
                branch_ref,
                path,
                lease_id: ExecutionLeaseId::from(lease_id),
                checkpoint_ids: checkpoints.ids,
                dirty_state: parse_enum(&dirty_state, "worktree dirty state")?,
                cleanup_eligibility: parse_enum(
                    &cleanup_eligibility,
                    "worktree cleanup eligibility",
                )?,
            },
            checkpoint_intent_sha256: checkpoints.intent_sha256,
            verified_process_death_sha256: checkpoints.verified_process_death_sha256,
            retained_evidence_sha256: checkpoints.retained_evidence_sha256,
            created_at_ms: checked_u64(created_at_ms, "worktree created at")?,
            updated_at_ms: checked_u64(updated_at_ms, "worktree updated at")?,
        };
        if record.worktree.git_common_dir_identity.trim().is_empty()
            || record.worktree.baseline_commit.trim().is_empty()
            || record.worktree.branch_ref.trim().is_empty()
            || record.worktree.path.trim().is_empty()
            || record.updated_at_ms < record.created_at_ms
            || record
                .checkpoint_intent_sha256
                .iter()
                .any(|(checkpoint_id, digest)| {
                    !record.worktree.checkpoint_ids.contains(checkpoint_id) || !is_sha256(digest)
                })
            || record
                .verified_process_death_sha256
                .as_deref()
                .is_some_and(|digest| !is_sha256(digest))
            || record
                .retained_evidence_sha256
                .as_deref()
                .is_some_and(|digest| !is_sha256(digest))
        {
            return Err(data_error("owned worktree row is malformed"));
        }
        records.push(record);
    }
    Ok(records)
}

fn load_budget_reservations(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Vec<StoredBudgetReservation>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT reservation_id, task_id, attempt_id, agent_id, organization_id,
                agent_limit_tokens, agent_limit_micro_usd, agent_limit_runtime_ms,
                ledger_utc_date, reserved_tokens, reserved_micro_usd, reserved_runtime_ms,
                actual_tokens, actual_micro_usd, actual_runtime_ms, state, grants_json,
                reconciliation_json, created_at_ms, updated_at_ms
           FROM orchestration_budget_reservations WHERE run_id = ?1
          ORDER BY created_at_ms, reservation_id",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, i64>(10)?,
            row.get::<_, i64>(11)?,
            row.get::<_, Option<i64>>(12)?,
            row.get::<_, Option<i64>>(13)?,
            row.get::<_, Option<i64>>(14)?,
            row.get::<_, String>(15)?,
            row.get::<_, String>(16)?,
            row.get::<_, Option<String>>(17)?,
            row.get::<_, i64>(18)?,
            row.get::<_, i64>(19)?,
        ))
    })?;
    let mut records = Vec::new();
    for row in rows {
        let (
            reservation_id,
            task_id,
            attempt_id,
            agent_id,
            organization_id,
            agent_limit_tokens,
            agent_limit_micro_usd,
            agent_limit_runtime_ms,
            ledger_utc_date,
            reserved_tokens,
            reserved_micro_usd,
            reserved_runtime_ms,
            actual_tokens,
            actual_micro_usd,
            actual_runtime_ms,
            state,
            grants_json,
            reconciliation_json,
            created_at_ms,
            updated_at_ms,
        ) = row?;
        let task_id = RunTaskId::from(task_id);
        let attempt_id = RunAttemptId::from(attempt_id);
        let grants: BTreeMap<BudgetGrantId, SupplementalBudgetGrant> =
            serde_json::from_str(&grants_json)?;
        let record = StoredBudgetReservation {
            run_id: run_id.clone(),
            agent_id: AgentId::from(agent_id),
            organization_id: OrganizationId::from(organization_id),
            agent_budget_limit: BudgetAmount {
                tokens: checked_u64(agent_limit_tokens, "agent token limit")?,
                cost_micro_usd: checked_u64(agent_limit_micro_usd, "agent cost limit")?,
                runtime_ms: checked_u64(agent_limit_runtime_ms, "agent runtime limit")?,
            },
            reservation: BudgetReservation {
                id: pandamux_core::BudgetReservationId::from(reservation_id),
                task_id,
                attempt_id,
                amount: pandamux_core::BudgetAmount {
                    tokens: checked_u64(reserved_tokens, "reserved tokens")?,
                    cost_micro_usd: checked_u64(reserved_micro_usd, "reserved cost")?,
                    runtime_ms: checked_u64(reserved_runtime_ms, "reserved runtime")?,
                },
                actual: pandamux_core::BudgetUsage {
                    tokens: actual_tokens
                        .map(|value| checked_u64(value, "actual tokens"))
                        .transpose()?,
                    cost_micro_usd: actual_micro_usd
                        .map(|value| checked_u64(value, "actual cost"))
                        .transpose()?,
                    runtime_ms: actual_runtime_ms
                        .map(|value| checked_u64(value, "actual runtime"))
                        .transpose()?,
                },
                ledger_utc_date,
                state: parse_enum(&state, "budget reservation state")?,
            },
            grants,
            reconciliation: reconciliation_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?,
            created_at_ms: checked_u64(created_at_ms, "budget reservation created at")?,
            updated_at_ms: checked_u64(updated_at_ms, "budget reservation updated at")?,
        };
        if !valid_utc_date(&record.reservation.ledger_utc_date)
            || amount_exceeds_sqlite(&record.agent_budget_limit)
            || record.updated_at_ms < record.created_at_ms
            || !valid_budget_reconciliation(&record)
            || record.grants.iter().any(|(grant_id, grant)| {
                grant_id != &grant.id
                    || grant.run_id != record.run_id
                    || grant.task_id != record.reservation.task_id
                    || grant.attempt_id != record.reservation.attempt_id
                    || grant.amount == 0
                    || !is_sha256(&grant.action_digest)
            })
        {
            return Err(data_error("budget reservation row is malformed"));
        }
        records.push(record);
    }
    Ok(records)
}

fn load_daily_budgets(
    conn: &rusqlite::Connection,
    reservations: &[StoredBudgetReservation],
) -> Result<Vec<DailyBudgetRecord>, StoreError> {
    let identities = reservations
        .iter()
        .map(|record| {
            (
                record.organization_id.clone(),
                record.reservation.ledger_utc_date.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    identities
        .into_iter()
        .map(|(organization_id, ledger_utc_date)| {
            conn.query_row(
                "SELECT limit_micro_usd, reserved_micro_usd, reconciled_micro_usd,
                        granted_micro_usd, revision, updated_at_ms
                   FROM orchestration_daily_budgets
                  WHERE organization_id = ?1 AND ledger_utc_date = ?2",
                params![organization_id.as_str(), ledger_utc_date],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?
            .map(
                |(limit, reserved, reconciled, granted, revision, updated)| {
                    let record = DailyBudgetRecord {
                        organization_id,
                        ledger_utc_date,
                        limit_micro_usd: checked_u64(limit, "daily budget limit")?,
                        reserved_micro_usd: checked_u64(reserved, "daily budget reserved")?,
                        reconciled_micro_usd: checked_u64(reconciled, "daily budget reconciled")?,
                        granted_micro_usd: checked_u64(granted, "daily budget granted")?,
                        revision: checked_u64(revision, "daily budget revision")?,
                        updated_at_ms: checked_u64(updated, "daily budget updated at")?,
                    };
                    if !valid_utc_date(&record.ledger_utc_date) || record.revision == 0 {
                        return Err(data_error("daily budget row is malformed"));
                    }
                    Ok(record)
                },
            )
            .transpose()?
            .ok_or_else(|| data_error("budget reservation has no UTC daily ledger"))
        })
        .collect()
}

fn load_artifacts(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Vec<BlackboardEntry>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT entry_id, artifact_key, version, previous_entry_id, producer_task_id,
                consumers_json, output_contract_json, schema_sha256, canonical_json,
                byte_len, content_sha256, created_event_id, provenance_json
           FROM orchestration_blackboard_artifacts WHERE run_id = ?1
          ORDER BY artifact_key, version",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, Vec<u8>>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, Option<String>>(12)?,
        ))
    })?;
    let mut artifacts = Vec::new();
    for row in rows {
        let (
            entry_id,
            artifact_key,
            version,
            previous_entry_id,
            producer_task_id,
            consumers_json,
            output_contract_json,
            schema_sha256,
            canonical_json,
            byte_len,
            content_sha256,
            created_event_id,
            provenance_json,
        ) = row?;
        let artifact = BlackboardEntry {
            id: pandamux_core::BlackboardEntryId::from(entry_id),
            run_id: run_id.clone(),
            artifact_key: ArtifactKey::from(artifact_key),
            version: checked_u64(version, "artifact version")?,
            previous_entry_id: previous_entry_id.map(pandamux_core::BlackboardEntryId::from),
            producer: RunTaskId::from(producer_task_id),
            consumers: serde_json::from_str(&consumers_json)?,
            output_contract: serde_json::from_str(&output_contract_json)?,
            schema_sha256,
            canonical_json,
            byte_len: checked_u64(byte_len, "artifact byte length")?,
            content_sha256,
            created_event_id: RunEventId::from(created_event_id),
            provenance: provenance_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?,
        };
        validate_loaded_artifact(&artifact)?;
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

fn validate_loaded_artifact(artifact: &BlackboardEntry) -> Result<(), StoreError> {
    if artifact.artifact_key
        != pandamux_orchestrator::artifact_key(&artifact.producer, &artifact.output_contract)
        || artifact.schema_sha256
            != pandamux_orchestrator::output_contract_schema_sha256(&artifact.output_contract)
        || artifact.byte_len != u64::try_from(artifact.canonical_json.len()).unwrap_or(u64::MAX)
        || artifact.content_sha256 != sha256_hex(&artifact.canonical_json)
        || artifact.canonical_json.len() > MAX_ARTIFACT_BYTES
    {
        return Err(data_error("blackboard artifact row is malformed"));
    }
    let content: serde_json::Value = serde_json::from_slice(&artifact.canonical_json)?;
    if pandamux_orchestrator::canonical_artifact_json(&content) != artifact.canonical_json {
        return Err(data_error("blackboard artifact row is not canonical JSON"));
    }
    validate_json_bounds(&content, 0)?;
    pandamux_orchestrator::validate_output_value(&artifact.output_contract, &content)
        .map_err(|reason| data_error(format!("blackboard artifact row violates schema: {reason}")))
}

fn validate_loaded_artifact_set(
    snapshot: &Run,
    artifacts: &[BlackboardEntry],
) -> Result<(), StoreError> {
    let mut task_counts = std::collections::BTreeMap::<RunTaskId, usize>::new();
    let mut run_bytes = 0_u64;
    let mut prior: Option<(&ArtifactKey, u64, &BlackboardEntryId)> = None;
    for artifact in artifacts {
        if snapshot.blackboard.get(&artifact.id) != Some(artifact) {
            return Err(data_error(
                "blackboard artifact differs from the canonical run snapshot",
            ));
        }
        let producer = snapshot
            .tasks
            .get(&artifact.producer)
            .ok_or_else(|| data_error("blackboard producer does not exist in the run"))?;
        let expected_consumers = snapshot
            .tasks
            .values()
            .filter(|consumer| {
                consumer.inputs.iter().any(|input| {
                    input.producer_task_id.as_ref() == Some(&artifact.producer)
                        && input.contract == artifact.output_contract
                })
            })
            .map(|consumer| consumer.id.clone())
            .collect::<BTreeSet<_>>();
        if !producer.outputs.contains(&artifact.output_contract)
            || artifact.consumers != expected_consumers
        {
            return Err(data_error(
                "blackboard artifact scope differs from the run plan",
            ));
        }
        let count = task_counts.entry(artifact.producer.clone()).or_default();
        *count = count
            .checked_add(1)
            .ok_or_else(|| data_error("task artifact count overflow"))?;
        run_bytes = run_bytes
            .checked_add(artifact.byte_len)
            .ok_or_else(|| data_error("run artifact byte count overflow"))?;
        if *count > MAX_ARTIFACTS_PER_TASK
            || artifacts.len() > MAX_ARTIFACTS_PER_RUN
            || run_bytes > MAX_RUN_ARTIFACT_BYTES as u64
        {
            return Err(data_error("blackboard artifact limits are exceeded"));
        }
        match prior {
            Some((prior_key, prior_version, prior_id))
                if prior_key == &artifact.artifact_key
                    && prior_version.checked_add(1) == Some(artifact.version)
                    && artifact.previous_entry_id.as_ref() == Some(prior_id) => {}
            Some((prior_key, _, _))
                if prior_key != &artifact.artifact_key
                    && artifact.version == 1
                    && artifact.previous_entry_id.is_none() => {}
            None if artifact.version == 1 && artifact.previous_entry_id.is_none() => {}
            _ => return Err(data_error("blackboard artifact version chain is invalid")),
        }
        prior = Some((&artifact.artifact_key, artifact.version, &artifact.id));
    }
    Ok(())
}

fn load_progress(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Vec<AttemptProgressRecord>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT progress_id, task_id, attempt_id, kind, payload_digest, at_ms,
                cumulative_stream_bytes, terminal
           FROM orchestration_attempt_progress WHERE run_id = ?1
          ORDER BY at_ms, progress_id",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, bool>(7)?,
        ))
    })?;
    let mut records = Vec::new();
    for row in rows {
        let (id, task_id, attempt_id, kind, digest, at_ms, stream_bytes, terminal) = row?;
        let record = AttemptProgressRecord {
            id: AttemptProgressId::from(id),
            run_id: run_id.clone(),
            task_id: RunTaskId::from(task_id),
            attempt_id: RunAttemptId::from(attempt_id),
            kind: parse_enum(&kind, "attempt progress kind")?,
            payload_digest: digest,
            at_ms: checked_u64(at_ms, "attempt progress timestamp")?,
            cumulative_stream_bytes: stream_bytes
                .map(|value| checked_u64(value, "attempt progress bytes"))
                .transpose()?,
            terminal,
        };
        if !is_sha256(&record.payload_digest)
            || (record.kind == AttemptProgressKind::StreamBytes)
                != record.cumulative_stream_bytes.is_some()
            || (record.kind == AttemptProgressKind::TerminalOutcome) != record.terminal
        {
            return Err(data_error("attempt progress row is malformed"));
        }
        records.push(record);
    }
    Ok(records)
}

fn load_recovery_reviews(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Vec<RecoveryReview>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT review_id, task_id, attempt_id, effect_id, reason_code,
                evidence_digest, status, created_at_ms, resolved_at_ms, resolution
           FROM orchestration_recovery_reviews WHERE run_id = ?1
          ORDER BY created_at_ms, review_id",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, Option<i64>>(8)?,
            row.get::<_, Option<String>>(9)?,
        ))
    })?;
    let mut reviews = Vec::new();
    for row in rows {
        let (
            review_id,
            task_id,
            attempt_id,
            effect_id,
            reason_code,
            evidence_digest,
            status,
            created_at_ms,
            resolved_at_ms,
            resolution,
        ) = row?;
        let review = RecoveryReview {
            id: pandamux_core::RecoveryReviewId::from(review_id),
            run_id: run_id.clone(),
            task_id: RunTaskId::from(task_id),
            attempt_id: RunAttemptId::from(attempt_id),
            effect_id: effect_id.map(OrchestrationEffectId::from),
            reason_code,
            evidence_digest,
            status: parse_enum(&status, "recovery review status")?,
            created_at_ms: checked_u64(created_at_ms, "recovery review created at")?,
            resolved_at_ms: resolved_at_ms
                .map(|value| checked_u64(value, "recovery review resolved at"))
                .transpose()?,
            resolution,
        };
        if !valid_reason_code(&review.reason_code)
            || !is_sha256(&review.evidence_digest)
            || match review.status {
                RecoveryReviewStatus::Pending => {
                    review.resolved_at_ms.is_some() || review.resolution.is_some()
                }
                RecoveryReviewStatus::Resolved => {
                    review.resolved_at_ms.is_none()
                        || review.resolution.as_deref().is_none_or(str::is_empty)
                }
            }
        {
            return Err(data_error("recovery review row is malformed"));
        }
        reviews.push(review);
    }
    Ok(reviews)
}

fn load_recovery_review_by_id(
    conn: &rusqlite::Connection,
    review_id: &pandamux_core::RecoveryReviewId,
) -> Result<Option<RecoveryReview>, StoreError> {
    let run_id = conn
        .query_row(
            "SELECT run_id FROM orchestration_recovery_reviews WHERE review_id = ?1",
            [review_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(run_id) = run_id else {
        return Ok(None);
    };
    Ok(load_recovery_reviews(conn, &RunId::from(run_id))?
        .into_iter()
        .find(|review| review.id == *review_id))
}

fn load_recovery_review_by_identity(
    conn: &rusqlite::Connection,
    review: &RecoveryReview,
) -> Result<Option<RecoveryReview>, StoreError> {
    let review_id = conn
        .query_row(
            "SELECT review_id FROM orchestration_recovery_reviews
              WHERE run_id = ?1 AND task_id = ?2 AND attempt_id = ?3
                AND ifnull(effect_id, '') = ?4 AND reason_code = ?5
                AND evidence_digest = ?6",
            params![
                review.run_id.as_str(),
                review.task_id.as_str(),
                review.attempt_id.as_str(),
                review
                    .effect_id
                    .as_ref()
                    .map(OrchestrationEffectId::as_str)
                    .unwrap_or(""),
                review.reason_code,
                review.evidence_digest,
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    review_id
        .map(|review_id| {
            load_recovery_review_by_id(conn, &pandamux_core::RecoveryReviewId::from(review_id))?
                .ok_or_else(|| data_error("recovery review identity disappeared"))
        })
        .transpose()
}

fn load_node_receipts(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Vec<NodeReceiptRecord>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT receipt_id, node_store_id, task_id, attempt_id, effect_id,
                request_kind, idempotency_key, request_sha256, result_sha256,
                result_bytes, created_at_ms, updated_at_ms
           FROM orchestration_node_receipts WHERE run_id = ?1
          ORDER BY created_at_ms, receipt_id",
    )?;
    let rows = statement.query_map([run_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, Vec<u8>>(9)?,
            row.get::<_, i64>(10)?,
            row.get::<_, i64>(11)?,
        ))
    })?;
    let mut receipts = Vec::new();
    for row in rows {
        let (
            receipt_id,
            node_store_id,
            task_id,
            attempt_id,
            effect_id,
            request_kind,
            idempotency_key,
            request_sha256,
            result_sha256,
            result_bytes,
            created_at_ms,
            updated_at_ms,
        ) = row?;
        let receipt = NodeReceiptRecord {
            receipt_id,
            node_store_id: NodeStoreId::from(node_store_id),
            run_id: run_id.clone(),
            task_id: RunTaskId::from(task_id),
            attempt_id: RunAttemptId::from(attempt_id),
            effect_id: OrchestrationEffectId::from(effect_id),
            request_kind: parse_enum(&request_kind, "node receipt kind")?,
            idempotency_key,
            request_sha256,
            result_sha256,
            result_bytes,
            created_at_ms: checked_u64(created_at_ms, "node receipt created at")?,
            updated_at_ms: checked_u64(updated_at_ms, "node receipt updated at")?,
        };
        if receipt.receipt_id.trim().is_empty()
            || receipt.idempotency_key.trim().is_empty()
            || !is_sha256(&receipt.request_sha256)
            || receipt.result_sha256 != sha256_hex(&receipt.result_bytes)
            || receipt.result_bytes.is_empty()
            || receipt.updated_at_ms < receipt.created_at_ms
        {
            return Err(data_error("node receipt row is malformed"));
        }
        receipts.push(receipt);
    }
    Ok(receipts)
}

fn load_node_receipt_by_key(
    conn: &rusqlite::Connection,
    node_store_id: &NodeStoreId,
    request_kind: NodeReceiptKind,
    idempotency_key: &str,
) -> Result<Option<NodeReceiptRecord>, StoreError> {
    let kind = enum_text(request_kind)?;
    let row = conn
        .query_row(
            "SELECT receipt_id, run_id, task_id, attempt_id, effect_id,
                    request_sha256, result_sha256, result_bytes,
                    created_at_ms, updated_at_ms
               FROM orchestration_node_receipts
              WHERE node_store_id = ?1 AND request_kind = ?2
                AND idempotency_key = ?3",
            params![node_store_id.as_str(), kind, idempotency_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Vec<u8>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )
        .optional()?;
    row.map(|row| {
        node_receipt_from_parts(
            row,
            node_store_id.clone(),
            request_kind,
            idempotency_key.to_string(),
        )
    })
    .transpose()
}

fn load_node_receipts_for_attempt(
    conn: &rusqlite::Connection,
    attempt_id: &RunAttemptId,
) -> Result<Vec<NodeReceiptRecord>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT receipt_id, node_store_id, run_id, task_id, effect_id,
                request_kind, idempotency_key, request_sha256, result_sha256,
                result_bytes, created_at_ms, updated_at_ms
           FROM orchestration_node_receipts WHERE attempt_id = ?1
          ORDER BY created_at_ms, receipt_id",
    )?;
    let rows = statement.query_map([attempt_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, Vec<u8>>(9)?,
            row.get::<_, i64>(10)?,
            row.get::<_, i64>(11)?,
        ))
    })?;
    let mut receipts = Vec::new();
    for row in rows {
        let (
            receipt_id,
            node_store_id,
            run_id,
            task_id,
            effect_id,
            request_kind,
            idempotency_key,
            request_sha256,
            result_sha256,
            result_bytes,
            created_at_ms,
            updated_at_ms,
        ) = row?;
        receipts.push(node_receipt_from_parts(
            (
                receipt_id,
                run_id,
                task_id,
                attempt_id.as_str().to_string(),
                effect_id,
                request_sha256,
                result_sha256,
                result_bytes,
                created_at_ms,
                updated_at_ms,
            ),
            NodeStoreId::from(node_store_id),
            parse_enum(&request_kind, "node receipt kind")?,
            idempotency_key,
        )?);
    }
    Ok(receipts)
}

#[allow(clippy::type_complexity)]
fn node_receipt_from_parts(
    row: (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Vec<u8>,
        i64,
        i64,
    ),
    node_store_id: NodeStoreId,
    request_kind: NodeReceiptKind,
    idempotency_key: String,
) -> Result<NodeReceiptRecord, StoreError> {
    let record = NodeReceiptRecord {
        receipt_id: row.0,
        node_store_id,
        run_id: RunId::from(row.1),
        task_id: RunTaskId::from(row.2),
        attempt_id: RunAttemptId::from(row.3),
        effect_id: OrchestrationEffectId::from(row.4),
        request_kind,
        idempotency_key,
        request_sha256: row.5,
        result_sha256: row.6,
        result_bytes: row.7,
        created_at_ms: checked_u64(row.8, "node receipt created at")?,
        updated_at_ms: checked_u64(row.9, "node receipt updated at")?,
    };
    if record.receipt_id.trim().is_empty()
        || record.idempotency_key.trim().is_empty()
        || !is_sha256(&record.request_sha256)
        || record.result_sha256 != sha256_hex(&record.result_bytes)
        || record.result_bytes.is_empty()
        || record.updated_at_ms < record.created_at_ms
    {
        return Err(data_error("node receipt row is malformed"));
    }
    Ok(record)
}

fn load_snapshot(conn: &rusqlite::Connection, run_id: &RunId) -> Result<Run, StoreError> {
    load_snapshot_optional(conn, run_id)?
        .ok_or_else(|| data_error("orchestration run snapshot was not found"))
}

fn load_snapshot_optional(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Option<Run>, StoreError> {
    let row = conn
        .query_row(
            "SELECT snapshot_bytes, snapshot_sha256 FROM orchestration_runs WHERE run_id = ?1",
            [run_id.as_str()],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((bytes, digest)) = row else {
        return Ok(None);
    };
    if digest != sha256_hex(&bytes) {
        return Err(data_error("orchestration snapshot digest mismatch"));
    }
    let run: Run = serde_json::from_slice(&bytes)?;
    if run.id != *run_id {
        return Err(data_error("orchestration snapshot identity mismatch"));
    }
    Ok(Some(run))
}

fn validate_json_bounds(value: &serde_json::Value, depth: usize) -> Result<(), StoreError> {
    match value {
        serde_json::Value::String(text) if text.len() > MAX_ARTIFACT_STRING_BYTES => {
            Err(data_error("artifact JSON contains an oversized string"))
        }
        serde_json::Value::Array(items) => {
            let next = depth
                .checked_add(1)
                .ok_or_else(|| data_error("artifact JSON nesting overflow"))?;
            if next > MAX_ARTIFACT_JSON_DEPTH || items.len() > MAX_ARTIFACT_CONTAINER_ITEMS {
                return Err(data_error("artifact JSON array exceeds a hard bound"));
            }
            for item in items {
                validate_json_bounds(item, next)?;
            }
            Ok(())
        }
        serde_json::Value::Object(entries) => {
            let next = depth
                .checked_add(1)
                .ok_or_else(|| data_error("artifact JSON nesting overflow"))?;
            if next > MAX_ARTIFACT_JSON_DEPTH || entries.len() > MAX_ARTIFACT_CONTAINER_ITEMS {
                return Err(data_error("artifact JSON object exceeds a hard bound"));
            }
            for (key, value) in entries {
                if key.len() > MAX_ARTIFACT_STRING_BYTES {
                    return Err(data_error("artifact JSON key exceeds the string bound"));
                }
                validate_json_bounds(value, next)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn enum_text<T: Serialize>(value: T) -> Result<String, StoreError> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| data_error("enum did not serialize to a string"))
}

fn attempt_boundary_text(boundary: AttemptBoundary) -> &'static str {
    match boundary {
        AttemptBoundary::SafeToRetry => "safe_to_retry",
        AttemptBoundary::VisibleOutputBegun => "visible_output_begun",
        AttemptBoundary::ToolSideEffectBegun => "tool_started",
        AttemptBoundary::ExternallyCommitted => "external_commit",
    }
}

fn parse_attempt_boundary(value: &str) -> Result<AttemptBoundary, StoreError> {
    match value {
        "safe_to_retry" => Ok(AttemptBoundary::SafeToRetry),
        // Schema 14 retained the earlier request-submitted spelling. Treat it
        // as visible output so an old row can never become automatically
        // retryable under the narrower Phase 4 boundary model.
        "request_submitted" | "visible_output_begun" => Ok(AttemptBoundary::VisibleOutputBegun),
        "tool_started" => Ok(AttemptBoundary::ToolSideEffectBegun),
        "external_commit" => Ok(AttemptBoundary::ExternallyCommitted),
        _ => Err(data_error("attempt boundary is invalid")),
    }
}

fn parse_enum<T: DeserializeOwned>(value: &str, label: &str) -> Result<T, StoreError> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(|_| data_error(format!("{label} is invalid")))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_reason_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

fn valid_utc_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return false;
    }
    let year = value[0..4].parse::<u32>().ok();
    let month = value[5..7].parse::<u32>().ok();
    let day = value[8..10].parse::<u32>().ok();
    let (Some(year), Some(month), Some(day)) = (year, month, day) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

fn amount_exceeds_sqlite(amount: &BudgetAmount) -> bool {
    amount.tokens > i64::MAX as u64
        || amount.cost_micro_usd > i64::MAX as u64
        || amount.runtime_ms > i64::MAX as u64
}

fn valid_budget_reconciliation(record: &StoredBudgetReservation) -> bool {
    let actual = record.reservation.actual;
    let has_any =
        actual.tokens.is_some() || actual.cost_micro_usd.is_some() || actual.runtime_ms.is_some();
    let complete =
        actual.tokens.is_some() && actual.cost_micro_usd.is_some() && actual.runtime_ms.is_some();
    let state_matches = match record.reservation.state {
        BudgetReservationState::Reserved | BudgetReservationState::Released => {
            !has_any && record.reconciliation.is_none()
        }
        BudgetReservationState::PartiallyReconciled => {
            has_any && !complete && record.reconciliation.is_some()
        }
        BudgetReservationState::Reconciled => complete && record.reconciliation.is_some(),
    };
    state_matches
        && record.reconciliation.as_ref().is_none_or(|evidence| {
            let cost_matches = match evidence.cost_source {
                BudgetCostSource::ProviderReported => {
                    evidence.actual.cost_micro_usd.is_some()
                        && evidence.pricing_version_ids.is_empty()
                }
                BudgetCostSource::EffectiveDatedEstimate => {
                    evidence.actual.cost_micro_usd.is_some()
                        && !evidence.pricing_version_ids.is_empty()
                }
                BudgetCostSource::Unknown => {
                    evidence.actual.cost_micro_usd.is_none()
                        && evidence.pricing_version_ids.is_empty()
                }
            };
            is_sha256(&evidence.source_sha256)
                && evidence.actual == actual
                && !evidence.turn_id.as_str().is_empty()
                && !evidence.turn_attempt_id.as_str().is_empty()
                && evidence
                    .usage_observation_ids
                    .iter()
                    .all(|id| !id.as_str().is_empty())
                && evidence
                    .pricing_version_ids
                    .iter()
                    .all(|id| !id.as_str().is_empty())
                && cost_matches
        })
}

fn data_error(message: impl Into<String>) -> StoreError {
    StoreError::Data(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{
        AccessMode, AgentTask, ApprovalDecisionProvenance, ApprovalId, ApprovalRequirement,
        Attempt, AttemptExecutionEvidence, AttemptStatus, BlackboardArtifact, BudgetAmount,
        BudgetReservationId, BudgetReservationState, BudgetUsage, CleanupState,
        ConfigurationRevision, EffectReceipt, LeaseResourceKind, OrchestrationMode, OutputContract,
        PricingVersionId, ProjectRepositoryRecord, ProviderCapabilities, ProviderInstanceId,
        ProviderKind, RoutingDecision, RunApprovalKind, RunApprovalStatus, RunCommand, RunSeed,
        RunTaskStatus, RunTrigger, SubscriptionKey, Thread, ThreadOrigin, ThreadStatus,
        ThreadWorkspace, Turn, TurnAttempt, TurnAttemptStatus, TurnInput, TurnStatus,
        UsagePricingRule, UsagePricingVersion, UsageProvider, UsageSubscription, ValidatedPlan,
        WorktreeCleanupEligibility, WorktreeDirtyState,
    };
    use std::collections::BTreeMap;
    use std::sync::{Arc, Barrier};

    fn output_contract() -> OutputContract {
        OutputContract {
            kind: "change_set".to_string(),
            schema: None,
        }
    }

    fn seed() -> RunSeed {
        RunSeed {
            trigger: RunTrigger::Manual {
                client_id: "phase5c-test".to_string(),
            },
            policy_revision: "policy-phase5c".to_string(),
            bundle_sha: "a".repeat(64),
            organization_id: OrganizationId::new("org-phase5c"),
            mode: OrchestrationMode::Auto,
            budget_limit: BudgetAmount {
                tokens: 1_000,
                cost_micro_usd: 2_000,
                runtime_ms: 3_000,
            },
            max_parallel_tasks: 2,
        }
    }

    fn task() -> AgentTask {
        AgentTask {
            id: RunTaskId::new("task-phase5c"),
            summary: "Persist Phase 5C state".to_string(),
            class: "build".to_string(),
            project_id: ProjectId::new("project-phase5c"),
            organization_id: OrganizationId::new("org-phase5c"),
            required_capabilities: BTreeSet::new(),
            access_ceiling: AccessMode::WorkspaceOnly,
            inputs: Vec::new(),
            outputs: vec![output_contract()],
            required_verification: BTreeSet::new(),
            selected_agent_id: None,
            route: None,
            status: RunTaskStatus::Ready,
            max_attempts: 2,
            approval_ids: BTreeSet::new(),
            budget_limit: BudgetAmount {
                tokens: 100,
                cost_micro_usd: 200,
                runtime_ms: 300,
            },
            budget_reservation_id: None,
        }
    }

    fn plan() -> ValidatedPlan {
        let task = task();
        ValidatedPlan {
            schema_version: 1,
            source_digest: "b".repeat(64),
            tasks: BTreeMap::from([(task.id.clone(), task)]),
            edges: BTreeMap::new(),
            topological_order: vec![RunTaskId::new("task-phase5c")],
            total_budget: BudgetAmount {
                tokens: 100,
                cost_micro_usd: 200,
                runtime_ms: 300,
            },
        }
    }

    fn route() -> TaskRouteDecision {
        TaskRouteDecision {
            task_id: RunTaskId::new("task-phase5c"),
            selected_agent_id: Some(AgentId::new("agent-phase5c")),
            agent_evaluations: Vec::new(),
            provider: RoutingDecision {
                selected_provider_instance_id: Some(ProviderInstanceId::new("provider-phase5c")),
                selected_subscription_key: Some(SubscriptionKey::new("subscription-phase5c")),
                selected_model: Some("model-phase5c".to_string()),
                evaluations: Vec::new(),
            },
        }
    }

    fn reservation() -> BudgetReservation {
        BudgetReservation {
            id: BudgetReservationId::new("budget-phase5c"),
            task_id: RunTaskId::new("task-phase5c"),
            attempt_id: RunAttemptId::new("run-attempt-phase5c"),
            amount: BudgetAmount {
                tokens: 100,
                cost_micro_usd: 200,
                runtime_ms: 300,
            },
            actual: BudgetUsage::default(),
            ledger_utc_date: "2026-10-09".to_string(),
            state: BudgetReservationState::Reserved,
        }
    }

    fn attempt() -> Attempt {
        Attempt {
            id: RunAttemptId::new("run-attempt-phase5c"),
            task_id: RunTaskId::new("task-phase5c"),
            ordinal: 1,
            route: route(),
            thread_id: None,
            turn_attempt_id: None,
            status: AttemptStatus::DispatchRequested,
            boundary: AttemptBoundary::SafeToRetry,
            started_at_ms: 20_000,
            ended_at_ms: None,
            failure: None,
            usage_reconciled: false,
            ambiguous_effect: false,
            ambiguity_reason: None,
            worktree_id: None,
            checkpoint_ids: BTreeSet::new(),
            cleanup_state: CleanupState::NotStarted,
        }
    }

    fn execute(run: &mut Run, events: &mut Vec<RunEvent>, command: RunCommand) {
        let kinds = pandamux_orchestrator::decide(run, command).expect("legal fixture command");
        for kind in kinds {
            let seq = run.last_seq + 1;
            let event = RunEvent {
                event_id: RunEventId::new(format!("run-event-phase5c-{seq}")),
                run_id: run.id.clone(),
                seq,
                at_ms: 10_000 + seq,
                idempotency_key: format!("phase5c-{seq}"),
                kind,
            };
            *run = pandamux_orchestrator::apply(run, &event)
                .expect("fixture event applies")
                .run;
            events.push(event);
        }
    }

    fn fixture_lifecycle(complete: bool) -> (Run, Vec<RunEvent>) {
        let seed = seed();
        let mut run = Run::draft(RunId::new("run-phase5c"), seed.clone(), 10_001);
        let mut events = Vec::new();
        execute(
            &mut run,
            &mut events,
            RunCommand::AcceptPlan { seed, plan: plan() },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::SelectRoute { decision: route() },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::ReserveBudget {
                reservation: reservation(),
            },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::RequestDispatch { attempt: attempt() },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::AssignAttemptWorktree {
                attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                worktree_id: AttemptWorktreeId::new("attempt-worktree-phase5c"),
            },
        );
        if complete {
            execute(
                &mut run,
                &mut events,
                RunCommand::StartAttempt {
                    attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                    thread_id: ThreadId::new("thread-phase5c-orchestration"),
                    turn_attempt_id: TurnAttemptId::new("attempt-phase5c-orchestration"),
                },
            );
            execute(
                &mut run,
                &mut events,
                RunCommand::SettleAttempt {
                    attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                    status: AttemptStatus::Succeeded,
                    failure: None,
                    usage_reconciled: false,
                },
            );
            execute(
                &mut run,
                &mut events,
                RunCommand::ReconcileBudget {
                    reservation_id: BudgetReservationId::new("budget-phase5c"),
                    actual: BudgetUsage {
                        tokens: Some(80),
                        cost_micro_usd: Some(150),
                        runtime_ms: Some(250),
                    },
                },
            );
            let content = serde_json::json!({"files": ["src/lib.rs"]});
            let canonical_json = pandamux_orchestrator::canonical_artifact_json(&content);
            let contract = output_contract();
            execute(
                &mut run,
                &mut events,
                RunCommand::CompleteTask {
                    task_id: RunTaskId::new("task-phase5c"),
                    attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                    artifacts: vec![BlackboardArtifact {
                        id: BlackboardEntryId::new("blackboard-phase5c"),
                        run_id: RunId::new("run-phase5c"),
                        artifact_key: pandamux_orchestrator::artifact_key(
                            &RunTaskId::new("task-phase5c"),
                            &contract,
                        ),
                        version: 1,
                        previous_entry_id: None,
                        producer: RunTaskId::new("task-phase5c"),
                        consumers: BTreeSet::new(),
                        output_contract: contract.clone(),
                        schema_sha256: pandamux_orchestrator::output_contract_schema_sha256(
                            &contract,
                        ),
                        byte_len: canonical_json.len() as u64,
                        content_sha256: sha256_hex(&canonical_json),
                        canonical_json,
                        provenance: None,
                    }],
                    evidence_ids: BTreeSet::new(),
                    summary: Some("stored".to_string()),
                },
            );
        }
        (run, events)
    }

    fn fixture_before_dispatch() -> (Run, Vec<RunEvent>) {
        let seed = seed();
        let mut run = Run::draft(RunId::new("run-phase5c"), seed.clone(), 10_001);
        let mut events = Vec::new();
        execute(
            &mut run,
            &mut events,
            RunCommand::AcceptPlan { seed, plan: plan() },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::SelectRoute { decision: route() },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::ReserveBudget {
                reservation: reservation(),
            },
        );
        (run, events)
    }

    fn dispatch_transition(run: &Run) -> RunTransitionRequest {
        RunTransitionRequest {
            run_id: run.id.clone(),
            expected_last_seq: run.last_seq,
            expected_snapshot_sha256: orchestration_snapshot_sha256(run),
            command: RunCommand::RequestDispatch { attempt: attempt() },
            assigned_events: vec![AssignedRunEvent {
                event_id: RunEventId::new("run-event-phase5c-dispatch-transition"),
                idempotency_key: "phase5c-dispatch-transition".to_string(),
                at_ms: 20_000,
            }],
            rows: RunTransitionRows::default(),
        }
    }

    fn effect_claim(effect: &StoredOrchestrationEffect, owner_suffix: &str) -> EffectClaimRequest {
        let owner = ServerIncarnationId::new(format!("server-owner-{owner_suffix}"));
        EffectClaimRequest {
            effect_id: effect.effect.id.clone(),
            expected_payload_sha256: effect.effect.payload_sha256.clone(),
            owner_incarnation_id: owner.clone(),
            lease: ExecutionLease {
                id: ExecutionLeaseId::new(format!("effect-lease-{owner_suffix}")),
                resource_kind: LeaseResourceKind::Effect,
                resource_id: effect.effect.id.as_str().to_string(),
                owner_incarnation_id: owner,
                expires_at_ms: 50_000,
                last_heartbeat_ms: 29_000,
                revision: effect.delivery.claim_revision + 1,
            },
            claimed_at_ms: 30_000,
        }
    }

    fn effect_ownership(request: &EffectClaimRequest) -> EffectOwnership {
        EffectOwnership {
            effect_id: request.effect_id.clone(),
            expected_payload_sha256: request.expected_payload_sha256.clone(),
            owner_incarnation_id: request.owner_incarnation_id.clone(),
            lease_id: request.lease.id.clone(),
            claim_revision: request.lease.revision,
        }
    }

    fn repository() -> ProjectRepositoryRecord {
        ProjectRepositoryRecord {
            id: ProjectRepositoryId::new("project-repository-phase5c"),
            project_id: ProjectId::new("project-phase5c"),
            organization_id: OrganizationId::new("org-phase5c"),
            canonical_remote_identity: "remote-phase5c".to_string(),
            canonical_repository_root_identity: "root-phase5c".to_string(),
            display_path: "PandaMUX".to_string(),
            revision: ConfigurationRevision::INITIAL,
            created_at_ms: 1_000,
            updated_at_ms: 1_000,
        }
    }

    fn run_record(run: Run) -> OrchestrationRunRecord {
        OrchestrationRunRecord {
            run,
            project_id: ProjectId::new("project-phase5c"),
            repository_id: ProjectRepositoryId::new("project-repository-phase5c"),
            settings_revision: 1,
        }
    }

    fn store_with_repository() -> Store {
        let store = Store::in_memory().expect("in-memory store");
        store
            .save_project_repository(&repository())
            .expect("save repository");
        store
    }

    fn fixture_settled_unreconciled() -> (Run, Vec<RunEvent>) {
        let seed = seed();
        let mut run = Run::draft(RunId::new("run-phase5c"), seed.clone(), 10_001);
        let mut events = Vec::new();
        execute(
            &mut run,
            &mut events,
            RunCommand::AcceptPlan { seed, plan: plan() },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::SelectRoute { decision: route() },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::ReserveBudget {
                reservation: reservation(),
            },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::RequestDispatch { attempt: attempt() },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::AssignAttemptWorktree {
                attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                worktree_id: AttemptWorktreeId::new("attempt-worktree-phase5c"),
            },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::StartAttempt {
                attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                thread_id: ThreadId::new("thread-phase5c-orchestration"),
                turn_attempt_id: TurnAttemptId::new("attempt-phase5c-orchestration"),
            },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::SettleAttempt {
                attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                status: AttemptStatus::Succeeded,
                failure: None,
                usage_reconciled: false,
            },
        );
        (run, events)
    }

    fn settled_budget_store() -> Store {
        let store = store_with_repository();
        let (run, events) = fixture_settled_unreconciled();
        store
            .insert_orchestration_run(&run_record(run), &events)
            .expect("insert settled budget run");
        store
            .insert_orchestration_runtime_records(&OrchestrationRuntimeRecords {
                budget_reservations: vec![StoredBudgetReservation {
                    run_id: RunId::new("run-phase5c"),
                    agent_id: AgentId::new("agent-phase5c"),
                    organization_id: OrganizationId::new("org-phase5c"),
                    agent_budget_limit: BudgetAmount {
                        tokens: 1_000,
                        cost_micro_usd: 2_000,
                        runtime_ms: 3_000,
                    },
                    reservation: reservation(),
                    grants: BTreeMap::new(),
                    reconciliation: None,
                    created_at_ms: 30_000,
                    updated_at_ms: 30_000,
                }],
                daily_budgets: vec![DailyBudgetRecord {
                    organization_id: OrganizationId::new("org-phase5c"),
                    ledger_utc_date: "2026-10-09".to_string(),
                    limit_micro_usd: 10_000,
                    reserved_micro_usd: 200,
                    reconciled_micro_usd: 0,
                    granted_micro_usd: 0,
                    revision: 1,
                    updated_at_ms: 30_000,
                }],
                ..OrchestrationRuntimeRecords::default()
            })
            .expect("insert settled budget ledger");
        store
            .save_thread(&Thread {
                id: ThreadId::new("thread-phase5c-orchestration"),
                project_id: Some(ProjectId::new("project-phase5c")),
                environment_id: EnvironmentId::new("env-phase5c"),
                parent_thread_id: None,
                title: "Budget reconciliation".to_string(),
                provider_instance_id: ProviderInstanceId::new("provider-phase5c"),
                model: "model-phase5c".to_string(),
                effort: None,
                access_mode: AccessMode::WorkspaceOnly,
                workspace: ThreadWorkspace {
                    cwd: "/fixture".to_string(),
                    worktree: None,
                },
                status: ThreadStatus::Idle,
                agent: None,
                origin: ThreadOrigin::Orchestrator {
                    run_id: RunId::new("run-phase5c"),
                    task_id: "task-phase5c".to_string(),
                },
                created_at_ms: 31_000,
                updated_at_ms: 31_100,
            })
            .expect("save budget thread");
        store
            .save_turn(&Turn {
                id: TurnId::new("turn-phase5c-orchestration"),
                thread_id: ThreadId::new("thread-phase5c-orchestration"),
                environment_id: EnvironmentId::new("env-phase5c"),
                provider_instance_id: ProviderInstanceId::new("provider-phase5c"),
                seq: 1,
                input: TurnInput {
                    text: "fixture".to_string(),
                    attachments: Vec::new(),
                    attachment_ids: Vec::new(),
                    model: Some("model-phase5c".to_string()),
                    effort: None,
                },
                status: TurnStatus::Completed,
                started_at_ms: 31_000,
                ended_at_ms: Some(31_250),
                usage: Some(pandamux_core::TurnUsage {
                    cost_usd: Some(999.0),
                    ..pandamux_core::TurnUsage::default()
                }),
                checkpoint_before: None,
                checkpoint_after: None,
            })
            .expect("save budget turn");
        store
            .save_turn_attempt(&TurnAttempt {
                id: TurnAttemptId::new("attempt-phase5c-orchestration"),
                turn_id: TurnId::new("turn-phase5c-orchestration"),
                thread_id: ThreadId::new("thread-phase5c-orchestration"),
                ordinal: 1,
                environment_id: EnvironmentId::new("env-phase5c"),
                provider_instance_id: ProviderInstanceId::new("provider-phase5c"),
                subscription_key: Some(SubscriptionKey::new("subscription-phase5c")),
                model: "model-phase5c".to_string(),
                boundary: AttemptBoundary::VisibleOutputBegun,
                status: TurnAttemptStatus::Completed,
                routing: route().provider,
                execution: Some(AttemptExecutionEvidence {
                    provider_kind: ProviderKind::Codex,
                    profile_dir: "/fixture/profile".to_string(),
                    binary_path: Some("/fixture/codex".to_string()),
                    capabilities: ProviderCapabilities::default(),
                    provider_version: Some("fixture".to_string()),
                }),
                checkpoint: None,
                partial_output: String::new(),
                capacity_evidence: None,
                started_at_ms: 31_000,
                ended_at_ms: Some(31_250),
            })
            .expect("save budget turn attempt");
        store
            .save_usage_subscription_mapping(
                &UsageSubscription {
                    key: SubscriptionKey::new("subscription-phase5c"),
                    provider: UsageProvider::Codex,
                    display_name: "Budget fixture".to_string(),
                    masked_account_label: None,
                },
                &EnvironmentId::new("env-phase5c"),
                &ProviderInstanceId::new("provider-phase5c"),
                31_000,
            )
            .expect("save budget usage mapping");
        store
    }

    fn managed_usage(
        id: &str,
        tokens: TokenCounts,
        provider_reported_micro_usd: Option<u64>,
    ) -> UsageObservation {
        UsageObservation {
            id: UsageObservationId::new(id),
            source_kind: UsageSourceKind::ManagedTurn,
            parser_version: 1,
            provider_native_session_id: Some("native-session-phase5c".to_string()),
            provider_native_turn_id: Some("native-turn-phase5c".to_string()),
            occurred_at_ms: Some(31_200),
            provider: UsageProvider::Codex,
            provider_instance_id: ProviderInstanceId::new("provider-phase5c"),
            subscription_key: SubscriptionKey::new("subscription-phase5c"),
            model: Some("model-phase5c".to_string()),
            tokens,
            unknown_fields: BTreeSet::new(),
            provider_reported_micro_usd,
            source_record_count: 1,
            thread_id: Some(ThreadId::new("thread-phase5c-orchestration")),
            turn_id: Some(TurnId::new("turn-phase5c-orchestration")),
            agent_id: Some(AgentId::new("agent-phase5c")),
            schedule_id: None,
            fact_kind: UsageFactKind::Aggregate,
        }
    }

    fn save_pricing(store: &Store, rate: u64) {
        store
            .save_usage_pricing_version(&UsagePricingVersion {
                id: PricingVersionId::new("pricing-phase5c"),
                provider: UsageProvider::Codex,
                source_published_at_ms: 30_000,
                effective_from_ms: 30_000,
                rules: vec![UsagePricingRule {
                    canonical_model_id: "model-phase5c".to_string(),
                    aliases: BTreeSet::new(),
                    rates: UsagePricingRates {
                        input_micro_usd_per_million: Some(rate),
                        cached_input_micro_usd_per_million: Some(rate),
                        cache_write_micro_usd_per_million: Some(rate),
                        output_micro_usd_per_million: Some(rate),
                        reasoning_micro_usd_per_million: Some(rate),
                    },
                }],
            })
            .expect("save budget pricing");
    }

    fn approved_budget_grant_store(
        decision: RunApprovalStatus,
    ) -> (Store, SupplementalBudgetGrant) {
        let store = store_with_repository();
        let seed = seed();
        let mut run = Run::draft(RunId::new("run-phase5c"), seed.clone(), 10_001);
        let mut events = Vec::new();
        execute(
            &mut run,
            &mut events,
            RunCommand::AcceptPlan { seed, plan: plan() },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::SelectRoute { decision: route() },
        );
        let mut grant = SupplementalBudgetGrant {
            id: BudgetGrantId::new("budget-grant-phase5c"),
            run_id: RunId::new("run-phase5c"),
            task_id: RunTaskId::new("task-phase5c"),
            attempt_id: RunAttemptId::new("run-attempt-phase5c"),
            approval_id: ApprovalId::new("approval-budget-phase5c"),
            dimension: BudgetDimension::CostMicroUsd,
            amount: 75,
            policy_revision: "policy-phase5c".to_string(),
            action_digest: String::new(),
            expires_at_ms: 80_000,
        };
        grant.action_digest = supplemental_budget_action_digest(&grant);
        execute(
            &mut run,
            &mut events,
            RunCommand::RequestApproval {
                approval: ApprovalRequirement {
                    id: grant.approval_id.clone(),
                    run_id: grant.run_id.clone(),
                    task_id: grant.task_id.clone(),
                    kind: RunApprovalKind::Budget,
                    policy_revision: grant.policy_revision.clone(),
                    action_digest: grant.action_digest.clone(),
                    safe_summary: "Increase exact attempt budget".to_string(),
                    reason_codes: vec!["budget_limit".to_string()],
                    expires_at_ms: 90_000,
                    status: RunApprovalStatus::Pending,
                    decision: None,
                },
            },
        );
        execute(
            &mut run,
            &mut events,
            RunCommand::ResolveApproval {
                approval_id: grant.approval_id.clone(),
                action_digest: grant.action_digest.clone(),
                status: decision,
                provenance: ApprovalDecisionProvenance {
                    actor: "local-control".to_string(),
                    decided_at_ms: 20_000,
                    decision_id: "decision-budget-phase5c".to_string(),
                },
            },
        );
        store
            .insert_orchestration_run(&run_record(run), &events)
            .expect("insert budget approval run");
        (store, grant)
    }

    fn runtime_records(dispatch_effect_id: OrchestrationEffectId) -> OrchestrationRuntimeRecords {
        let capacity_lease = ExecutionLease {
            id: ExecutionLeaseId::new("execution-lease-capacity-phase5c"),
            resource_kind: LeaseResourceKind::Capacity,
            resource_id: "provider:provider-phase5c".to_string(),
            owner_incarnation_id: ServerIncarnationId::new("server-incarnation-phase5c"),
            expires_at_ms: 40_000,
            last_heartbeat_ms: 30_000,
            revision: 1,
        };
        let worktree_lease = ExecutionLease {
            id: ExecutionLeaseId::new("execution-lease-worktree-phase5c"),
            resource_kind: LeaseResourceKind::Worktree,
            resource_id: "attempt-worktree-phase5c".to_string(),
            owner_incarnation_id: ServerIncarnationId::new("server-incarnation-phase5c"),
            expires_at_ms: 40_000,
            last_heartbeat_ms: 30_000,
            revision: 1,
        };
        let result_bytes = br#"{"state":"prepared"}"#.to_vec();
        OrchestrationRuntimeRecords {
            attempt_dispatches: vec![AttemptDispatchRecord {
                attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                run_id: RunId::new("run-phase5c"),
                task_id: RunTaskId::new("task-phase5c"),
                effect_id: dispatch_effect_id.clone(),
                thread_id: ThreadId::new("thread-phase5c-orchestration"),
                turn_id: TurnId::new("turn-phase5c-orchestration"),
                turn_attempt_id: TurnAttemptId::new("attempt-phase5c-orchestration"),
                provider_instance_id: ProviderInstanceId::new("provider-phase5c"),
                route: route(),
                bundle_sha: "a".repeat(64),
                state: AttemptDispatchState::Prepared,
                last_boundary: AttemptBoundary::SafeToRetry,
                last_progress_at_ms: 30_000,
                cancel_requested_at_ms: None,
                ambiguity_reason: None,
                outcome_digest: None,
                created_at_ms: 30_000,
                updated_at_ms: 30_000,
            }],
            queue_records: vec![OrchestrationQueueRecord {
                queue_id: "queue-phase5c".to_string(),
                run_id: RunId::new("run-phase5c"),
                task_id: RunTaskId::new("task-phase5c"),
                attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                agent_id: AgentId::new("agent-phase5c"),
                organization_id: Some(OrganizationId::new("org-phase5c")),
                agent_concurrency_limit: Some(2),
                provider_instance_id: ProviderInstanceId::new("provider-phase5c"),
                environment_id: pandamux_core::EnvironmentId::new("env-phase5c"),
                candidate_sha256: "c".repeat(64),
                priority: 4,
                task_enqueued_at_ms: 25_000,
                enqueued_at_ms: 25_001,
                not_before_ms: 25_001,
                state: QueueState::Claimed,
                owner_incarnation_id: Some(ServerIncarnationId::new(
                    "server-incarnation-phase5c",
                )),
                lease_id: Some(capacity_lease.id.clone()),
                claim_revision: 1,
                interactive: false,
                claimed_at_ms: Some(30_000),
                claim_order: Some(1),
                interactive_streak: Some(0),
            }],
            leases: vec![capacity_lease, worktree_lease.clone()],
            worktrees: vec![OwnedAttemptWorktreeRecord {
                worktree: OwnedAttemptWorktree {
                    id: AttemptWorktreeId::new("attempt-worktree-phase5c"),
                    repository_id: ProjectRepositoryId::new("project-repository-phase5c"),
                    run_id: RunId::new("run-phase5c"),
                    task_id: RunTaskId::new("task-phase5c"),
                    attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                    environment_id: pandamux_core::EnvironmentId::new("env-phase5c"),
                    git_common_dir_identity: "git-common-phase5c".to_string(),
                    baseline_commit: "d".repeat(40),
                    branch_ref: "refs/heads/pandamux/run/run-phase5c/task-phase5c/1".to_string(),
                    path: "/private/worktrees/project-repository-phase5c/run-phase5c/run-attempt-phase5c"
                        .to_string(),
                    lease_id: worktree_lease.id,
                    checkpoint_ids: BTreeSet::new(),
                    dirty_state: WorktreeDirtyState::Clean,
                    cleanup_eligibility: WorktreeCleanupEligibility::Ineligible,
                },
                checkpoint_intent_sha256: BTreeMap::new(),
                verified_process_death_sha256: None,
                retained_evidence_sha256: None,
                created_at_ms: 30_000,
                updated_at_ms: 30_000,
            }],
            budget_reservations: vec![StoredBudgetReservation {
                run_id: RunId::new("run-phase5c"),
                agent_id: AgentId::new("agent-phase5c"),
                organization_id: OrganizationId::new("org-phase5c"),
                agent_budget_limit: BudgetAmount {
                    tokens: 1_000,
                    cost_micro_usd: 2_000,
                    runtime_ms: 3_000,
                },
                reservation: reservation(),
                grants: BTreeMap::new(),
                reconciliation: None,
                created_at_ms: 30_000,
                updated_at_ms: 30_000,
            }],
            daily_budgets: vec![DailyBudgetRecord {
                organization_id: OrganizationId::new("org-phase5c"),
                ledger_utc_date: "2026-10-09".to_string(),
                limit_micro_usd: 10_000,
                reserved_micro_usd: 200,
                reconciled_micro_usd: 0,
                granted_micro_usd: 0,
                revision: 1,
                updated_at_ms: 30_000,
            }],
            artifacts: Vec::new(),
            progress: vec![AttemptProgressRecord {
                id: AttemptProgressId::new("attempt-progress-phase5c"),
                run_id: RunId::new("run-phase5c"),
                task_id: RunTaskId::new("task-phase5c"),
                attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                kind: AttemptProgressKind::BoundaryAdvance,
                payload_digest: "e".repeat(64),
                at_ms: 30_001,
                cumulative_stream_bytes: None,
                terminal: false,
            }],
            recovery_reviews: vec![RecoveryReview {
                id: pandamux_core::RecoveryReviewId::new("recovery-review-phase5c"),
                run_id: RunId::new("run-phase5c"),
                task_id: RunTaskId::new("task-phase5c"),
                attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                effect_id: Some(dispatch_effect_id.clone()),
                reason_code: "dispatch_response_unknown".to_string(),
                evidence_digest: "f".repeat(64),
                status: RecoveryReviewStatus::Pending,
                created_at_ms: 30_002,
                resolved_at_ms: None,
                resolution: None,
            }],
            node_receipts: vec![NodeReceiptRecord {
                receipt_id: "node-receipt-phase5c".to_string(),
                node_store_id: NodeStoreId::new("store-phase5c"),
                run_id: RunId::new("run-phase5c"),
                task_id: RunTaskId::new("task-phase5c"),
                attempt_id: RunAttemptId::new("run-attempt-phase5c"),
                effect_id: dispatch_effect_id,
                request_kind: NodeReceiptKind::AttemptPrepare,
                idempotency_key: "node-prepare-phase5c".to_string(),
                request_sha256: "1".repeat(64),
                result_sha256: sha256_hex(&result_bytes),
                result_bytes,
                created_at_ms: 30_003,
                updated_at_ms: 30_003,
            }],
        }
    }

    #[test]
    fn canonical_run_replay_and_every_runtime_mapping_round_trip() {
        let store = store_with_repository();
        let (run, events) = fixture_lifecycle(false);
        let stored_effects = store
            .insert_orchestration_run(&run_record(run.clone()), &events)
            .expect("insert verified run");
        let loaded = store
            .load_verified_orchestration_run(&run.id)
            .expect("load verified run")
            .expect("stored run");
        assert_eq!(loaded, run_record(run.clone()));
        assert_eq!(store.load_orchestration_events(&run.id).unwrap(), events);
        assert_eq!(
            store.load_orchestration_effects(&run.id).unwrap(),
            stored_effects
        );

        let dispatch_effect = stored_effects
            .iter()
            .find(|record| {
                matches!(
                    record.effect.kind,
                    pandamux_core::OrchestrationEffectKind::DispatchTask { .. }
                )
            })
            .expect("dispatch effect")
            .effect
            .id
            .clone();
        let records = runtime_records(dispatch_effect);
        store
            .insert_orchestration_runtime_records(&records)
            .expect("insert runtime records");
        assert_eq!(
            store
                .load_orchestration_runtime_records(&run.id)
                .expect("load runtime records"),
            records
        );
    }

    #[test]
    fn runtime_transaction_rolls_back_every_prior_insert_on_a_late_invalid_receipt() {
        let store = store_with_repository();
        let (run, events) = fixture_lifecycle(false);
        let effects = store
            .insert_orchestration_run(&run_record(run.clone()), &events)
            .expect("insert verified run");
        let dispatch_effect = effects
            .iter()
            .find(|record| {
                matches!(
                    record.effect.kind,
                    pandamux_core::OrchestrationEffectKind::DispatchTask { .. }
                )
            })
            .expect("dispatch effect")
            .effect
            .id
            .clone();
        let mut records = runtime_records(dispatch_effect);
        records.node_receipts[0].result_sha256 = "0".repeat(64);
        assert!(
            store
                .insert_orchestration_runtime_records(&records)
                .is_err()
        );

        assert_eq!(
            store
                .load_orchestration_runtime_records(&run.id)
                .expect("load rolled-back runtime"),
            OrchestrationRuntimeRecords::default()
        );
    }

    #[test]
    fn artifact_rows_match_the_atomic_snapshot_and_remain_immutable() {
        let store = store_with_repository();
        let (run, events) = fixture_lifecycle(true);
        store
            .insert_orchestration_run(&run_record(run.clone()), &events)
            .expect("insert completed run");
        let artifact = run
            .blackboard
            .values()
            .next()
            .expect("atomic artifact")
            .clone();
        store
            .insert_orchestration_runtime_records(&OrchestrationRuntimeRecords {
                artifacts: vec![artifact.clone()],
                ..OrchestrationRuntimeRecords::default()
            })
            .expect("insert canonical artifact");
        assert_eq!(
            store
                .load_orchestration_runtime_records(&run.id)
                .expect("load artifact")
                .artifacts,
            vec![artifact]
        );

        let conn = store.conn.lock().unwrap();
        assert!(
            conn.execute(
                "UPDATE orchestration_blackboard_artifacts
                    SET content_sha256 = ?1 WHERE entry_id = 'blackboard-phase5c'",
                ["0".repeat(64)],
            )
            .is_err(),
            "immutable artifact trigger rejects updates"
        );
    }

    #[test]
    fn verified_load_rejects_a_digest_valid_snapshot_that_disagrees_with_replay() {
        let store = store_with_repository();
        let (run, events) = fixture_lifecycle(false);
        store
            .insert_orchestration_run(&run_record(run.clone()), &events)
            .expect("insert verified run");
        let mut corrupted = run.clone();
        corrupted.policy_revision = "different-policy".to_string();
        let bytes = pandamux_orchestrator::canonical_run_bytes(&corrupted);
        let digest = sha256_hex(&bytes);
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE orchestration_runs
                    SET snapshot_bytes = ?1, snapshot_sha256 = ?2,
                        policy_revision = 'different-policy'
                  WHERE run_id = ?3",
                params![bytes, digest, run.id.as_str()],
            )
            .expect("inject internally consistent corruption");

        let error = store
            .load_verified_orchestration_run(&run.id)
            .expect_err("event replay remains authoritative");
        assert!(error.to_string().contains("replay differs"));
    }

    #[test]
    fn runtime_load_rejects_digest_valid_rows_that_change_canonical_identity() {
        let store = store_with_repository();
        let (run, events) = fixture_lifecycle(false);
        let effects = store
            .insert_orchestration_run(&run_record(run.clone()), &events)
            .expect("insert verified run");
        let dispatch_effect = effects
            .iter()
            .find(|record| {
                matches!(
                    record.effect.kind,
                    pandamux_core::OrchestrationEffectKind::DispatchTask { .. }
                )
            })
            .expect("dispatch effect")
            .effect
            .id
            .clone();
        let records = runtime_records(dispatch_effect);
        store
            .insert_orchestration_runtime_records(&records)
            .expect("insert runtime records");

        let mut changed_route = records.attempt_dispatches[0].route.clone();
        changed_route.provider.selected_model = Some("changed-model".to_string());
        let changed_route_bytes = serde_json::to_vec(&changed_route).expect("changed route bytes");
        let changed_route_sha = sha256_hex(&changed_route_bytes);
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE orchestration_attempt_dispatch
                    SET route_bytes = ?1, route_sha256 = ?2
                  WHERE attempt_id = 'run-attempt-phase5c'",
                params![changed_route_bytes, changed_route_sha],
            )
            .expect("inject digest-valid route mismatch");

        let error = store
            .load_orchestration_runtime_records(&run.id)
            .expect_err("changed canonical route must fail closed");
        assert!(
            error
                .to_string()
                .contains("canonical run or provider effect")
        );
    }

    #[test]
    fn runtime_load_rejects_budget_and_progress_rows_with_changed_owners() {
        let store = store_with_repository();
        let (run, events) = fixture_lifecycle(false);
        let effects = store
            .insert_orchestration_run(&run_record(run.clone()), &events)
            .expect("insert verified run");
        let dispatch_effect = effects
            .iter()
            .find(|record| {
                matches!(
                    record.effect.kind,
                    pandamux_core::OrchestrationEffectKind::DispatchTask { .. }
                )
            })
            .expect("dispatch effect")
            .effect
            .id
            .clone();
        let records = runtime_records(dispatch_effect);
        store
            .insert_orchestration_runtime_records(&records)
            .expect("insert runtime records");

        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE orchestration_budget_reservations
                    SET reserved_tokens = reserved_tokens - 1
                  WHERE reservation_id = 'budget-phase5c'",
                [],
            )
            .expect("inject in-range budget mismatch");
        assert!(
            store
                .load_orchestration_runtime_records(&run.id)
                .expect_err("changed reservation must fail closed")
                .to_string()
                .contains("canonical run snapshot")
        );

        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE orchestration_budget_reservations
                    SET reserved_tokens = reserved_tokens + 1
                  WHERE reservation_id = 'budget-phase5c'",
                [],
            )
            .expect("restore reservation");
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE orchestration_attempt_progress
                    SET task_id = 'task-foreign'
                  WHERE progress_id = 'attempt-progress-phase5c'",
                [],
            )
            .expect("inject progress owner mismatch");
        assert!(
            store
                .load_orchestration_runtime_records(&run.id)
                .expect_err("changed progress owner must fail closed")
                .to_string()
                .contains("canonical attempt owner")
        );
    }

    #[test]
    fn independent_connections_racing_the_same_run_leave_one_canonical_identity() {
        let temp = tempfile::tempdir().expect("temporary store");
        let database = temp.path().join("pandamux.db");
        let first = Store::open(&database).expect("first store connection");
        first
            .save_project_repository(&repository())
            .expect("save repository");
        let second = Store::open(&database).expect("second store connection");
        let (run, events) = fixture_lifecycle(false);
        let record = run_record(run.clone());
        let barrier = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let left_barrier = barrier.clone();
            let left_record = record.clone();
            let left_events = events.clone();
            let left = scope.spawn(move || {
                left_barrier.wait();
                first.insert_orchestration_run(&left_record, &left_events)
            });
            let right_barrier = barrier.clone();
            let right_record = record.clone();
            let right_events = events.clone();
            let right = scope.spawn(move || {
                right_barrier.wait();
                second.insert_orchestration_run(&right_record, &right_events)
            });
            [
                left.join().expect("left join"),
                right.join().expect("right join"),
            ]
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);

        let reopened = Store::open(&database).expect("reopen raced store");
        assert_eq!(
            reopened
                .load_verified_orchestration_run(&run.id)
                .expect("verify raced run")
                .expect("one canonical run"),
            record
        );
    }

    #[test]
    fn immutable_event_constraints_and_malformed_runtime_values_fail_closed() {
        let store = store_with_repository();
        let (run, events) = fixture_lifecycle(false);
        store
            .insert_orchestration_run(&run_record(run.clone()), &events)
            .expect("insert verified run");
        let conn = store.conn.lock().unwrap();
        assert!(
            conn.execute(
                "UPDATE orchestration_run_events SET seq = 99 WHERE run_id = ?1 AND seq = 1",
                [run.id.as_str()],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO orchestration_leases (
                    lease_id, resource_kind, resource_id, owner_incarnation_id,
                    expires_at_ms, last_heartbeat_ms, revision
                 ) VALUES ('bad-lease', 'capacity', 'provider:x', 'owner', 1, 2, 0)",
                [],
            )
            .is_err()
        );
    }

    #[test]
    fn run_transition_crash_replays_exactly_rejects_changed_bytes_and_broadcasts_after_commit() {
        let store = store_with_repository();
        let (base, events) = fixture_before_dispatch();
        store
            .insert_orchestration_run(&run_record(base.clone()), &events)
            .expect("insert transition base");
        let service = crate::RunExecutionService::new(store.clone());
        let mut notices = service.subscribe();
        let request = dispatch_transition(&base);

        assert!(
            service
                .commit_transition(
                    &request,
                    Some(OrchestrationExecutionFailpoint::AfterEventCommit),
                )
                .expect_err("post-commit failpoint")
                .to_string()
                .contains("after_event_commit")
        );
        assert_eq!(
            notices.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty),
            "a failed caller observes no pre-commit or speculative broadcast"
        );

        let duplicate = service
            .commit_transition(&request, None)
            .expect("database-only transition retry");
        assert!(duplicate.duplicate);
        let notice = notices.try_recv().expect("post-commit projection notice");
        assert!(notice.duplicate);
        assert_eq!(notice.run_id, base.id);
        assert_eq!(notice.last_seq, base.last_seq + 1);
        assert_eq!(
            notice.event_ids,
            vec![request.assigned_events[0].event_id.clone()]
        );
        assert_eq!(notice.effect_ids, duplicate.effect_ids);
        let safe_debug = format!("{notice:?}");
        assert!(!safe_debug.contains("Persist Phase 5C state"));
        assert!(!safe_debug.contains("model-phase5c"));

        let audit = service.audit_replay(&base.id).expect("replay audit");
        assert_eq!(audit.last_seq, base.last_seq + 1);
        assert_eq!(audit.snapshot_sha256, duplicate.snapshot_sha256);
        let stored_events = store
            .load_orchestration_events(&base.id)
            .expect("load committed events");
        let full = pandamux_orchestrator::replay(stored_events.iter()).expect("full replay");
        for prefix_len in 1..=stored_events.len() {
            let prefix = pandamux_orchestrator::replay(stored_events[..prefix_len].iter())
                .expect("prefix replay");
            let rebuilt =
                pandamux_orchestrator::replay_from(&prefix, stored_events[prefix_len..].iter())
                    .expect("suffix replay");
            assert_eq!(
                pandamux_orchestrator::canonical_run_bytes(&rebuilt),
                pandamux_orchestrator::canonical_run_bytes(&full)
            );
        }

        let mut changed = request.clone();
        changed.assigned_events[0].at_ms += 1;
        assert!(
            service
                .commit_transition(&changed, None)
                .expect_err("changed event identity must conflict")
                .to_string()
                .contains("reused with different bytes")
        );
        assert_eq!(
            store
                .load_orchestration_events(&base.id)
                .expect("unchanged event log")
                .len(),
            events.len() + 1
        );
    }

    #[test]
    fn effect_failpoints_resume_database_only_and_never_redispatch() {
        let store = store_with_repository();
        let (run, events) = fixture_lifecycle(false);
        let effects = store
            .insert_orchestration_run(&run_record(run.clone()), &events)
            .expect("insert effect fixture");
        let effect = effects
            .iter()
            .find(|record| {
                matches!(
                    record.effect.kind,
                    pandamux_core::OrchestrationEffectKind::DispatchTask { .. }
                )
            })
            .expect("dispatch effect")
            .clone();
        let claim = effect_claim(&effect, "crash");

        assert!(
            store
                .claim_orchestration_effect(
                    &claim,
                    Some(OrchestrationExecutionFailpoint::BeforeEffectClaim),
                )
                .expect_err("before-claim failpoint")
                .to_string()
                .contains("before_effect_claim")
        );
        assert_eq!(
            load_stored_effect_by_id(&store.conn.lock().unwrap(), &effect.effect.id)
                .unwrap()
                .unwrap()
                .delivery
                .state,
            EffectDeliveryState::Pending
        );
        assert!(
            store
                .claim_orchestration_effect(
                    &claim,
                    Some(OrchestrationExecutionFailpoint::AfterEffectClaim),
                )
                .expect_err("after-claim failpoint")
                .to_string()
                .contains("after_effect_claim")
        );
        let retry = store
            .claim_orchestration_effect(&claim, None)
            .expect("exact claim retry");
        assert!(!retry.acquired);
        let competing = effect_claim(&effect, "foreign");
        assert!(
            store
                .claim_orchestration_effect(&competing, None)
                .expect_err("foreign owner cannot steal claim")
                .to_string()
                .contains("already owned")
        );

        let ownership = effect_ownership(&claim);
        assert!(
            store
                .mark_orchestration_effect_prepared(&ownership, &"c".repeat(64), 31_000)
                .expect("prepare effect")
                .prepared
        );
        let dispatch = EffectDispatchRequest {
            ownership: ownership.clone(),
            dispatch_started_at_ms: 32_000,
        };
        assert!(
            store
                .mark_orchestration_effect_dispatch_started(
                    &dispatch,
                    Some(OrchestrationExecutionFailpoint::BeforeDispatchMarker),
                )
                .expect_err("before-dispatch-marker failpoint")
                .to_string()
                .contains("before_dispatch_marker")
        );
        assert!(
            store
                .mark_orchestration_effect_dispatch_started(
                    &dispatch,
                    Some(OrchestrationExecutionFailpoint::AfterDispatchMarker),
                )
                .expect_err("after-dispatch-marker failpoint")
                .to_string()
                .contains("after_dispatch_marker")
        );
        assert!(
            !store
                .mark_orchestration_effect_dispatch_started(&dispatch, None)
                .expect("dispatch marker retry")
                .dispatch_allowed,
            "a committed marker must never authorize a second external dispatch"
        );

        let outcome = EffectOutcomeRequest {
            ownership: ownership.clone(),
            result_digest: "d".repeat(64),
            safe_error_code: None,
            recorded_at_ms: 33_000,
        };
        assert!(
            store
                .record_orchestration_effect_outcome(
                    &outcome,
                    Some(OrchestrationExecutionFailpoint::BeforeOutcomeCommit),
                )
                .expect_err("before-outcome failpoint")
                .to_string()
                .contains("before_outcome_commit")
        );
        assert!(
            store
                .record_orchestration_effect_outcome(
                    &outcome,
                    Some(OrchestrationExecutionFailpoint::AfterOutcomeCommit),
                )
                .expect_err("after-outcome failpoint")
                .to_string()
                .contains("after_outcome_commit")
        );
        let mut changed_outcome = outcome.clone();
        changed_outcome.result_digest = "e".repeat(64);
        assert!(
            store
                .record_orchestration_effect_outcome(&changed_outcome, None)
                .expect_err("changed outcome retry")
                .to_string()
                .contains("conflicts")
        );
        assert!(
            !store
                .record_orchestration_effect_outcome(&outcome, None)
                .expect("exact outcome retry")
                .recorded
        );

        let acknowledged_at_ms = 34_000;
        let acknowledgement = RunTransitionRequest {
            run_id: run.id.clone(),
            expected_last_seq: run.last_seq,
            expected_snapshot_sha256: orchestration_snapshot_sha256(&run),
            command: RunCommand::AcknowledgeEffect {
                effect_id: effect.effect.id.clone(),
                receipt: EffectReceipt {
                    effect_id: effect.effect.id.clone(),
                    acknowledged_at_ms,
                    result_digest: outcome.result_digest.clone(),
                },
            },
            assigned_events: vec![AssignedRunEvent {
                event_id: RunEventId::new("run-event-phase5c-acknowledge-effect"),
                idempotency_key: "phase5c-acknowledge-effect".to_string(),
                at_ms: acknowledged_at_ms,
            }],
            rows: RunTransitionRows::default(),
        };
        assert!(
            store
                .commit_run_transition(
                    &acknowledgement,
                    Some(OrchestrationExecutionFailpoint::AfterEventCommit),
                )
                .expect_err("acknowledgement post-commit failpoint")
                .to_string()
                .contains("after_event_commit")
        );
        let settled = load_stored_effect_by_id(&store.conn.lock().unwrap(), &effect.effect.id)
            .unwrap()
            .unwrap();
        assert_eq!(settled.delivery.state, EffectDeliveryState::Settled);
        assert!(settled.delivery.owner_incarnation_id.is_none());
        assert!(settled.delivery.lease_id.is_none());
        assert!(
            load_lease(&store.conn.lock().unwrap(), &claim.lease.id)
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .commit_run_transition(&acknowledgement, None)
                .expect("database-only acknowledgement retry")
                .duplicate
        );
        assert!(
            store
                .list_pending_orchestration_effects(256)
                .expect("pending effects")
                .iter()
                .all(|record| record.effect.id != effect.effect.id)
        );
    }

    #[test]
    fn bud_05_independent_connections_cannot_overreserve_one_daily_ledger() {
        let temp = tempfile::tempdir().expect("temporary store");
        let database = temp.path().join("budget-race.db");
        let first = Store::open(&database).expect("first connection");
        let second = Store::open(&database).expect("second connection");
        let record = |suffix: &str| StoredBudgetReservation {
            run_id: RunId::new(format!("run-{suffix}")),
            agent_id: AgentId::new(format!("agent-{suffix}")),
            organization_id: OrganizationId::new("org-race"),
            agent_budget_limit: BudgetAmount {
                tokens: 10_000,
                cost_micro_usd: 10_000,
                runtime_ms: 10_000,
            },
            reservation: BudgetReservation {
                id: BudgetReservationId::new(format!("budget-{suffix}")),
                task_id: RunTaskId::new(format!("task-{suffix}")),
                attempt_id: RunAttemptId::new(format!("attempt-{suffix}")),
                amount: BudgetAmount {
                    tokens: 1,
                    cost_micro_usd: 600,
                    runtime_ms: 1,
                },
                actual: BudgetUsage::default(),
                ledger_utc_date: "2026-10-09".to_string(),
                state: BudgetReservationState::Reserved,
            },
            grants: BTreeMap::new(),
            reconciliation: None,
            created_at_ms: 40_000,
            updated_at_ms: 40_000,
        };
        let left_record = record("left");
        let right_record = record("right");
        let barrier = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let left_barrier = barrier.clone();
            let left_store = first.clone();
            let left = scope.spawn(move || {
                left_barrier.wait();
                let mut conn = left_store.conn.lock().unwrap();
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .expect("left transaction");
                reserve_daily_budget_for_admission(&tx, &left_record, 1_000, 40_000)
                    .and_then(|()| tx.commit().map_err(StoreError::from))
            });
            let right_barrier = barrier.clone();
            let right_store = second.clone();
            let right = scope.spawn(move || {
                right_barrier.wait();
                let mut conn = right_store.conn.lock().unwrap();
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .expect("right transaction");
                reserve_daily_budget_for_admission(&tx, &right_record, 1_000, 40_000)
                    .and_then(|()| tx.commit().map_err(StoreError::from))
            });
            [
                left.join().expect("left join"),
                right.join().expect("right join"),
            ]
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        let daily = load_daily_budget(
            &first.conn.lock().unwrap(),
            &OrganizationId::new("org-race"),
            "2026-10-09",
        )
        .expect("load raced daily ledger")
        .expect("daily ledger");
        assert_eq!(daily.reserved_micro_usd, 600);
    }

    #[test]
    fn bud_08_to_10_grants_require_exact_live_local_approval_and_replay_once() {
        let (disconnected_store, disconnected_grant) =
            approved_budget_grant_store(RunApprovalStatus::Approved);
        assert!(
            disconnected_store
                .grant_orchestration_budget(&disconnected_grant, 40_000, false, None)
                .expect_err("disconnected local control")
                .to_string()
                .contains("connected local control")
        );

        let (rejected_store, rejected_grant) =
            approved_budget_grant_store(RunApprovalStatus::Rejected);
        assert!(
            rejected_store
                .grant_orchestration_budget(&rejected_grant, 40_000, true, None)
                .is_err()
        );

        let (store, grant) = approved_budget_grant_store(RunApprovalStatus::Approved);
        assert!(
            store
                .grant_orchestration_budget(&grant, grant.expires_at_ms, true, None)
                .expect_err("expired grant")
                .to_string()
                .contains("transition rejected")
        );
        let mut changed_amount = grant.clone();
        changed_amount.amount += 1;
        assert!(
            store
                .grant_orchestration_budget(&changed_amount, 40_000, true, None)
                .expect_err("changed amount with stale digest")
                .to_string()
                .contains("action digest")
        );
        let mut changed_policy = grant.clone();
        changed_policy.policy_revision = "policy-drift".to_string();
        changed_policy.action_digest = supplemental_budget_action_digest(&changed_policy);
        assert!(
            store
                .grant_orchestration_budget(&changed_policy, 40_000, true, None)
                .expect_err("policy drift")
                .to_string()
                .contains("transition rejected")
        );
        let initial_events = store
            .load_orchestration_events(&grant.run_id)
            .expect("load initial grant events")
            .len();
        assert!(
            store
                .grant_orchestration_budget(
                    &grant,
                    40_000,
                    true,
                    Some(BudgetExecutionFailpoint::BeforeGrantCommit),
                )
                .expect_err("before-grant failpoint")
                .to_string()
                .contains("before_budget_grant_commit")
        );
        assert_eq!(
            store
                .load_orchestration_events(&grant.run_id)
                .expect("load rolled-back grant events")
                .len(),
            initial_events
        );
        assert!(
            store
                .grant_orchestration_budget(
                    &grant,
                    40_000,
                    true,
                    Some(BudgetExecutionFailpoint::AfterGrantCommit),
                )
                .expect_err("after-grant failpoint")
                .to_string()
                .contains("after_budget_grant_commit")
        );
        let replay = store
            .grant_orchestration_budget(&grant, 40_000, true, None)
            .expect("exact grant replay");
        assert!(replay.duplicate);
        assert_eq!(
            store
                .load_orchestration_events(&grant.run_id)
                .expect("load committed grant events")
                .len(),
            initial_events + 1
        );
    }

    #[test]
    fn bud_11_provider_reported_cost_wins_and_reconciliation_recovers_exactly() {
        let store = settled_budget_store();
        save_pricing(&store, 2_000_000);
        let observation = managed_usage(
            "usage-provider-cost",
            TokenCounts {
                input: 80,
                output: 20,
                ..TokenCounts::default()
            },
            Some(123),
        );
        store
            .record_usage_batch(&EnvironmentId::new("env-phase5c"), &[observation], None)
            .expect("record provider-reported usage");
        assert!(
            store
                .reconcile_orchestration_budget(
                    &RunId::new("run-phase5c"),
                    &BudgetReservationId::new("budget-phase5c"),
                    40_000,
                    Some(BudgetExecutionFailpoint::BeforeReconciliationCommit),
                )
                .expect_err("before-reconciliation failpoint")
                .to_string()
                .contains("before_budget_reconciliation_commit")
        );
        let before = store
            .load_orchestration_runtime_records(&RunId::new("run-phase5c"))
            .expect("load rolled-back reconciliation");
        assert_eq!(
            before.budget_reservations[0].reservation.state,
            BudgetReservationState::Reserved
        );
        assert!(
            store
                .reconcile_orchestration_budget(
                    &RunId::new("run-phase5c"),
                    &BudgetReservationId::new("budget-phase5c"),
                    40_000,
                    Some(BudgetExecutionFailpoint::AfterReconciliationCommit),
                )
                .expect_err("after-reconciliation failpoint")
                .to_string()
                .contains("after_budget_reconciliation_commit")
        );
        let replay = store
            .reconcile_orchestration_budget(
                &RunId::new("run-phase5c"),
                &BudgetReservationId::new("budget-phase5c"),
                40_000,
                None,
            )
            .expect("exact reconciliation replay");
        assert!(replay.duplicate);
        assert_eq!(replay.reservation.reservation.actual.tokens, Some(100));
        assert_eq!(
            replay.reservation.reservation.actual.cost_micro_usd,
            Some(123)
        );
        assert_eq!(replay.reservation.reservation.actual.runtime_ms, Some(250));
        assert_eq!(
            replay
                .reservation
                .reconciliation
                .as_ref()
                .expect("reconciliation evidence")
                .cost_source,
            BudgetCostSource::ProviderReported
        );
        assert_eq!(replay.daily_budget.reserved_micro_usd, 0);
        assert_eq!(replay.daily_budget.reconciled_micro_usd, 123);
        assert_eq!(replay.daily_budget.ledger_utc_date, "2026-10-09");
    }

    #[test]
    fn bud_12_effective_dated_integer_pricing_reconciles_without_legacy_float() {
        let store = settled_budget_store();
        save_pricing(&store, 1_000_000);
        let observation = managed_usage(
            "usage-estimated-cost",
            TokenCounts {
                input: 70,
                output: 30,
                ..TokenCounts::default()
            },
            None,
        );
        store
            .record_usage_batch(&EnvironmentId::new("env-phase5c"), &[observation], None)
            .expect("record estimated usage");
        let outcome = store
            .reconcile_orchestration_budget(
                &RunId::new("run-phase5c"),
                &BudgetReservationId::new("budget-phase5c"),
                40_000,
                None,
            )
            .expect("reconcile estimated usage");
        assert_eq!(
            outcome.reservation.reservation.actual.cost_micro_usd,
            Some(100)
        );
        let evidence = outcome
            .reservation
            .reconciliation
            .expect("estimated reconciliation evidence");
        assert_eq!(
            evidence.cost_source,
            BudgetCostSource::EffectiveDatedEstimate
        );
        assert_eq!(
            evidence.pricing_version_ids,
            BTreeSet::from([PricingVersionId::new("pricing-phase5c")])
        );
    }

    #[test]
    fn bud_13_unknown_cost_retains_reservation_and_ignores_legacy_float() {
        let store = settled_budget_store();
        let observation = managed_usage(
            "usage-unknown-cost",
            TokenCounts {
                input: 60,
                output: 20,
                ..TokenCounts::default()
            },
            None,
        );
        store
            .record_usage_batch(&EnvironmentId::new("env-phase5c"), &[observation], None)
            .expect("record unknown-cost usage");
        let outcome = store
            .reconcile_orchestration_budget(
                &RunId::new("run-phase5c"),
                &BudgetReservationId::new("budget-phase5c"),
                100_000_000,
                None,
            )
            .expect("reconcile unknown cost");
        assert_eq!(
            outcome.reservation.reservation.state,
            BudgetReservationState::PartiallyReconciled
        );
        assert_eq!(outcome.reservation.reservation.actual.tokens, Some(80));
        assert_eq!(outcome.reservation.reservation.actual.cost_micro_usd, None);
        assert_eq!(outcome.run.budget.reserved.cost_micro_usd, 200);
        assert!(outcome.run.budget.incomplete.cost_micro_usd);
        assert_eq!(outcome.run.status, RunStatus::Paused);
        assert_eq!(outcome.daily_budget.reserved_micro_usd, 200);
        assert_eq!(outcome.daily_budget.reconciled_micro_usd, 0);
        assert_eq!(outcome.daily_budget.ledger_utc_date, "2026-10-09");
        assert_eq!(
            outcome
                .reservation
                .reconciliation
                .expect("unknown-cost evidence")
                .cost_source,
            BudgetCostSource::Unknown
        );
    }

    #[test]
    fn bud_14_actual_overage_is_recorded_and_pauses_before_more_dispatch() {
        let store = settled_budget_store();
        let observation = managed_usage(
            "usage-overage",
            TokenCounts {
                input: 120,
                output: 30,
                ..TokenCounts::default()
            },
            Some(250),
        );
        store
            .record_usage_batch(&EnvironmentId::new("env-phase5c"), &[observation], None)
            .expect("record over-budget usage");
        let outcome = store
            .reconcile_orchestration_budget(
                &RunId::new("run-phase5c"),
                &BudgetReservationId::new("budget-phase5c"),
                40_000,
                None,
            )
            .expect("reconcile over-budget usage");
        assert_eq!(outcome.reservation.reservation.actual.tokens, Some(150));
        assert_eq!(
            outcome.reservation.reservation.actual.cost_micro_usd,
            Some(250)
        );
        assert_eq!(outcome.run.budget.overage.tokens, 50);
        assert_eq!(outcome.run.budget.overage.cost_micro_usd, 50);
        assert_eq!(outcome.run.status, RunStatus::Paused);
        assert_eq!(outcome.daily_budget.reconciled_micro_usd, 250);
    }

    #[test]
    fn bud_15_conservation_reports_mismatch_without_certifying_source_truth() {
        let store = settled_budget_store();
        let balanced = store
            .audit_budget_conservation(&RunId::new("run-phase5c"))
            .expect("audit balanced ledger");
        assert_eq!(
            balanced.verdict,
            BudgetConservationVerdict::BalancedSourceUnverified
        );
        assert!(balanced.mismatches.is_empty());
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE orchestration_daily_budgets
                    SET reserved_micro_usd = reserved_micro_usd + 1
                  WHERE organization_id = ?1 AND ledger_utc_date = ?2",
                params!["org-phase5c", "2026-10-09"],
            )
            .expect("inject conservation mismatch");
        let mismatch = store
            .audit_budget_conservation(&RunId::new("run-phase5c"))
            .expect("audit mismatched ledger");
        assert_eq!(mismatch.verdict, BudgetConservationVerdict::Mismatch);
        assert!(mismatch.mismatches.iter().any(|item| {
            item.scope.contains("daily:org-phase5c:2026-10-09:reserved")
                && item.dimension == BudgetDimension::CostMicroUsd
                && item.expected == 200
                && item.actual == 201
        }));
    }

    #[test]
    fn life_01_and_02_progress_is_coalesced_deduplicated_and_bounded() {
        let store = store_with_repository();
        let (run, events) = fixture_lifecycle(false);
        let effects = store
            .insert_orchestration_run(&run_record(run.clone()), &events)
            .expect("insert progress run");
        let dispatch_effect = effects
            .iter()
            .find(|record| {
                matches!(
                    record.effect.kind,
                    pandamux_core::OrchestrationEffectKind::DispatchTask { .. }
                )
            })
            .expect("progress dispatch effect")
            .effect
            .id
            .clone();
        let mut runtime = runtime_records(dispatch_effect);
        runtime.progress.clear();
        runtime.recovery_reviews.clear();
        store
            .insert_orchestration_runtime_records(&runtime)
            .expect("insert progress runtime");

        let progress = |id: String,
                        kind: AttemptProgressKind,
                        payload_digest: String,
                        at_ms: u64,
                        cumulative_stream_bytes: Option<u64>,
                        terminal: bool| AttemptProgressRecord {
            id: AttemptProgressId::new(id),
            run_id: RunId::new("run-phase5c"),
            task_id: RunTaskId::new("task-phase5c"),
            attempt_id: RunAttemptId::new("run-attempt-phase5c"),
            kind,
            payload_digest,
            at_ms,
            cumulative_stream_bytes,
            terminal,
        };

        let first_stream = progress(
            "progress-stream-first".to_string(),
            AttemptProgressKind::StreamBytes,
            "1".repeat(64),
            30_001,
            Some(0),
            false,
        );
        assert_eq!(
            store
                .record_attempt_progress(&first_stream)
                .expect("first stream progress")
                .disposition,
            AttemptProgressDisposition::Persisted
        );
        let coalesced = progress(
            "progress-stream-coalesced".to_string(),
            AttemptProgressKind::StreamBytes,
            "2".repeat(64),
            30_500,
            Some(1_000),
            false,
        );
        let coalesced = store
            .record_attempt_progress(&coalesced)
            .expect("sub-threshold stream progress");
        assert_eq!(coalesced.disposition, AttemptProgressDisposition::Coalesced);
        assert_eq!(coalesced.last_progress_at_ms, 30_001);

        let byte_boundary = progress(
            "progress-stream-byte-boundary".to_string(),
            AttemptProgressKind::StreamBytes,
            "3".repeat(64),
            30_501,
            Some(4_096),
            false,
        );
        assert_eq!(
            store
                .record_attempt_progress(&byte_boundary)
                .expect("four KiB stream boundary")
                .disposition,
            AttemptProgressDisposition::Persisted
        );
        let time_boundary = progress(
            "progress-stream-time-boundary".to_string(),
            AttemptProgressKind::StreamBytes,
            "4".repeat(64),
            31_501,
            Some(4_097),
            false,
        );
        assert_eq!(
            store
                .record_attempt_progress(&time_boundary)
                .expect("one second stream boundary")
                .disposition,
            AttemptProgressDisposition::Persisted
        );
        let semantic_duplicate = progress(
            "progress-stream-semantic-duplicate".to_string(),
            AttemptProgressKind::StreamBytes,
            "4".repeat(64),
            40_000,
            Some(4_097),
            false,
        );
        let duplicate = store
            .record_attempt_progress(&semantic_duplicate)
            .expect("semantic duplicate");
        assert_eq!(duplicate.disposition, AttemptProgressDisposition::Duplicate);
        assert_eq!(duplicate.last_progress_at_ms, 31_501);
        let backwards = progress(
            "progress-backwards".to_string(),
            AttemptProgressKind::ToolLifecycle,
            "5".repeat(64),
            31_500,
            None,
            false,
        );
        assert!(
            store
                .record_attempt_progress(&backwards)
                .expect_err("backward progress clock rejected")
                .to_string()
                .contains("moved backwards")
        );

        for index in 0..260_u64 {
            let payload = sha256_hex(format!("tool-transition-{index}").as_bytes());
            let record = progress(
                format!("progress-tool-{index:03}"),
                AttemptProgressKind::ToolLifecycle,
                payload,
                32_000 + index,
                None,
                false,
            );
            assert_eq!(
                store
                    .record_attempt_progress(&record)
                    .expect("bounded tool progress")
                    .disposition,
                AttemptProgressDisposition::Persisted
            );
        }

        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE orchestration_attempt_dispatch
                    SET state = 'terminal', outcome_digest = ?1
                  WHERE attempt_id = 'run-attempt-phase5c'",
                ["6".repeat(64)],
            )
            .expect("mark progress fixture terminal");
        let terminal = progress(
            "progress-terminal".to_string(),
            AttemptProgressKind::TerminalOutcome,
            "6".repeat(64),
            32_260,
            None,
            true,
        );
        assert_eq!(
            store
                .record_attempt_progress(&terminal)
                .expect("terminal progress")
                .disposition,
            AttemptProgressDisposition::Persisted
        );
        for (id, kind, digest, at_ms) in [
            (
                "progress-post-terminal-checkpoint",
                AttemptProgressKind::CheckpointCreated,
                "7".repeat(64),
                32_261,
            ),
            (
                "progress-post-terminal-artifact",
                AttemptProgressKind::ArtifactPublished,
                "8".repeat(64),
                32_262,
            ),
        ] {
            assert_eq!(
                store
                    .record_attempt_progress(&progress(
                        id.to_string(),
                        kind,
                        digest,
                        at_ms,
                        None,
                        false,
                    ))
                    .expect("post-terminal retained evidence")
                    .disposition,
                AttemptProgressDisposition::Persisted
            );
        }
        assert!(
            store
                .record_attempt_progress(&progress(
                    "progress-post-terminal-tool".to_string(),
                    AttemptProgressKind::ToolLifecycle,
                    "9".repeat(64),
                    32_263,
                    None,
                    false,
                ))
                .expect_err("post-terminal tool progress rejected")
                .to_string()
                .contains("post-terminal evidence")
        );

        let retained = store
            .load_attempt_progress(&RunAttemptId::new("run-attempt-phase5c"))
            .expect("load retained progress");
        assert_eq!(retained.iter().filter(|record| record.terminal).count(), 1);
        assert_eq!(
            retained.iter().filter(|record| !record.terminal).count(),
            256
        );
        assert_eq!(retained.len(), 257);
    }

    #[test]
    fn independent_connections_racing_effect_claims_choose_exactly_one_owner() {
        let temp = tempfile::tempdir().expect("temporary store");
        let database = temp.path().join("pandamux.db");
        let first = Store::open(&database).expect("first connection");
        first
            .save_project_repository(&repository())
            .expect("save repository");
        let (run, events) = fixture_lifecycle(false);
        let effects = first
            .insert_orchestration_run(&run_record(run.clone()), &events)
            .expect("insert run");
        let effect = effects
            .iter()
            .find(|record| {
                matches!(
                    record.effect.kind,
                    pandamux_core::OrchestrationEffectKind::DispatchTask { .. }
                )
            })
            .expect("dispatch effect")
            .clone();
        let second = Store::open(&database).expect("second connection");
        let left_request = effect_claim(&effect, "left");
        let right_request = effect_claim(&effect, "right");
        let barrier = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let left_barrier = barrier.clone();
            let left = scope.spawn(move || {
                left_barrier.wait();
                first.claim_orchestration_effect(&left_request, None)
            });
            let right_barrier = barrier.clone();
            let right = scope.spawn(move || {
                right_barrier.wait();
                second.claim_orchestration_effect(&right_request, None)
            });
            [
                left.join().expect("left join"),
                right.join().expect("right join"),
            ]
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);

        let reopened = Store::open(&database).expect("reopen claimed store");
        let claimed = reopened
            .load_orchestration_effects(&run.id)
            .expect("load raced effects")
            .into_iter()
            .find(|record| record.effect.id == effect.effect.id)
            .expect("claimed effect");
        assert_eq!(claimed.delivery.state, EffectDeliveryState::Claimed);
        assert_eq!(claimed.delivery.claim_revision, 1);
        assert!(claimed.delivery.owner_incarnation_id.is_some());
        assert!(claimed.delivery.lease_id.is_some());
    }
}
