use super::{Store, StoreError, checked_i64, checked_u32, checked_u64};
use pandamux_core::{
    AgentId, ArtifactKey, Attempt, AttemptBoundary, AttemptProgressId, AttemptProgressKind,
    AttemptProgressRecord, AttemptWorktreeId, BlackboardEntry, BlackboardEntryId, BudgetGrantId,
    BudgetReservation, CheckpointId, EffectDeliveryRecord, EffectDeliveryState, EnvironmentId,
    ExecutionLease, ExecutionLeaseId, LeaseResourceKind, MAX_ARTIFACT_BYTES,
    MAX_ARTIFACT_CONTAINER_ITEMS, MAX_ARTIFACT_JSON_DEPTH, MAX_ARTIFACT_STRING_BYTES,
    MAX_ARTIFACTS_PER_RUN, MAX_ARTIFACTS_PER_TASK, MAX_RUN_ARTIFACT_BYTES, NodeStoreId,
    OrchestrationEffect, OrchestrationEffectId, OrganizationId, OwnedAttemptWorktree, ProjectId,
    ProjectRepositoryId, ProviderInstanceId, RecoveryReview, RecoveryReviewStatus, Run,
    RunAttemptId, RunCommand, RunEvent, RunEventId, RunId, RunStatus, RunTaskId,
    ServerIncarnationId, SupplementalBudgetGrant, TaskRouteDecision, ThreadId, TurnAttemptId,
    TurnId, WorktreeCleanupEligibility, WorktreeDirtyState,
};
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
    pub reservation: BudgetReservation,
    pub grants: BTreeMap<BudgetGrantId, SupplementalBudgetGrant>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
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
        for artifact in &records.artifacts {
            insert_artifact(&tx, artifact)?;
        }
        for progress in &records.progress {
            insert_progress(&tx, progress)?;
        }
        for review in &records.recovery_reviews {
            insert_recovery_review(&tx, review)?;
        }
        for receipt in &records.node_receipts {
            insert_node_receipt(&tx, receipt)?;
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
    for artifact in &rows.artifacts {
        insert_artifact(tx, artifact)?;
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
                && record.delivery.claim_revision == 0
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
        || snapshot.routes.get(&record.task_id) != Some(&record.route)
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
            enum_text(record.last_boundary)?,
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
        let snapshot = load_snapshot(tx, &record.run_id)?;
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
        if !task_state_matches
            || Some(&snapshot.organization_id) != record.organization_id.as_ref()
            || route.selected_agent_id.as_ref() != Some(&record.agent_id)
            || route.provider.selected_provider_instance_id.as_ref()
                != Some(&record.provider_instance_id)
        {
            return Err(data_error(
                "orchestration queue record differs from the canonical run",
            ));
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
        || record.updated_at_ms < record.created_at_ms
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
        || grants_for_attempt(&snapshot, &reservation.attempt_id) != record.grants
    {
        return Err(data_error(
            "budget reservation differs from the canonical run snapshot",
        ));
    }
    tx.execute(
        "INSERT INTO orchestration_budget_reservations (
            reservation_id, run_id, task_id, attempt_id, agent_id, organization_id,
            ledger_utc_date, reserved_tokens, reserved_micro_usd, reserved_runtime_ms,
            actual_tokens, actual_micro_usd, actual_runtime_ms, state, grants_json,
            created_at_ms, updated_at_ms
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17
         )",
        params![
            reservation.id.as_str(),
            record.run_id.as_str(),
            reservation.task_id.as_str(),
            reservation.attempt_id.as_str(),
            record.agent_id.as_str(),
            record.organization_id.as_str(),
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

fn insert_artifact(tx: &Transaction<'_>, artifact: &BlackboardEntry) -> Result<(), StoreError> {
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
    let snapshot = load_snapshot(tx, &artifact.run_id)?;
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
    let snapshot = load_snapshot(tx, &review.run_id)?;
    if snapshot
        .attempts
        .get(&review.attempt_id)
        .is_none_or(|attempt| attempt.task_id != review.task_id)
    {
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
        validate_active_worktree_lease(&current, &lease, retained_at_ms)?;
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
        if current.interactive != request.orchestration.is_none() {
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
        validate_admission_ownership(ownership)?;
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
        Ok(updated)
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
            let run = load_snapshot(conn, &record.run_id)?;
            counts
                .run_limits
                .insert(record.run_id.clone(), run.max_parallel_tasks);
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
    let run = load_snapshot(conn, &record.run_id)?;
    Ok(counts.runs.get(&record.run_id).copied().unwrap_or(0) < run.max_parallel_tasks)
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
    if limit_micro_usd == 0 || limit_micro_usd > i64::MAX as u64 {
        return Err(data_error(
            "execution admission daily budget limit is malformed",
        ));
    }
    let amount = reservation.reservation.amount.cost_micro_usd;
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
        let revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| data_error("daily budget revision overflow"))?;
        let available = current
            .limit_micro_usd
            .checked_add(current.granted_micro_usd)
            .ok_or_else(|| data_error("daily budget available amount overflow"))?;
        let committed = reserved_micro_usd
            .checked_add(current.reconciled_micro_usd)
            .ok_or_else(|| data_error("daily budget committed amount overflow"))?;
        if committed > available {
            return Err(data_error("daily budget limit would be exceeded"));
        }
        let changed = tx.execute(
            "UPDATE orchestration_daily_budgets
                SET reserved_micro_usd = ?1, revision = ?2, updated_at_ms = ?3
              WHERE organization_id = ?4 AND ledger_utc_date = ?5
                AND limit_micro_usd = ?6 AND reserved_micro_usd = ?7
                AND reconciled_micro_usd = ?8 AND granted_micro_usd = ?9
                AND revision = ?10 AND updated_at_ms = ?11",
            params![
                checked_i64(reserved_micro_usd, "daily budget reserved")?,
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
            revision,
            updated_at_ms,
            ..current
        }
    } else {
        if amount > limit_micro_usd {
            return Err(data_error("daily budget limit would be exceeded"));
        }
        let record = DailyBudgetRecord {
            organization_id: reservation.organization_id.clone(),
            ledger_utc_date: reservation.reservation.ledger_utc_date.clone(),
            limit_micro_usd,
            reserved_micro_usd: amount,
            reconciled_micro_usd: 0,
            granted_micro_usd: 0,
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
    if lease.id != record.worktree.lease_id
        || lease.resource_kind != LeaseResourceKind::Worktree
        || lease.resource_id != record.worktree.id.as_str()
        || lease.revision == 0
        || at_ms < lease.last_heartbeat_ms
        || at_ms >= lease.expires_at_ms
    {
        return Err(data_error(
            "owned attempt worktree lease is not active for this operation",
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
            || snapshot.routes.get(&attempt.task_id) != Some(&attempt.route)
        {
            return Err(data_error(
                "attempt dispatch differs from its canonical run or provider effect",
            ));
        }
    }
    for record in queue_records {
        let route = snapshot
            .routes
            .get(&record.task_id)
            .ok_or_else(|| data_error("queue row has no canonical task route"))?;
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
            || grants_for_attempt(snapshot, &reservation.reservation.attempt_id)
                != reservation.grants
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

fn grants_for_attempt(
    snapshot: &Run,
    attempt_id: &RunAttemptId,
) -> BTreeMap<BudgetGrantId, SupplementalBudgetGrant> {
    snapshot
        .budget_grants
        .iter()
        .filter(|(_, grant)| grant.attempt_id == *attempt_id)
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
            last_boundary: parse_enum(&last_boundary, "attempt boundary")?,
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
                ledger_utc_date, reserved_tokens, reserved_micro_usd, reserved_runtime_ms,
                actual_tokens, actual_micro_usd, actual_runtime_ms, state, grants_json,
                created_at_ms, updated_at_ms
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
            row.get::<_, String>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
            row.get::<_, Option<i64>>(9)?,
            row.get::<_, Option<i64>>(10)?,
            row.get::<_, Option<i64>>(11)?,
            row.get::<_, String>(12)?,
            row.get::<_, String>(13)?,
            row.get::<_, i64>(14)?,
            row.get::<_, i64>(15)?,
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
            ledger_utc_date,
            reserved_tokens,
            reserved_micro_usd,
            reserved_runtime_ms,
            actual_tokens,
            actual_micro_usd,
            actual_runtime_ms,
            state,
            grants_json,
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
            created_at_ms: checked_u64(created_at_ms, "budget reservation created at")?,
            updated_at_ms: checked_u64(updated_at_ms, "budget reservation updated at")?,
        };
        if !valid_utc_date(&record.reservation.ledger_utc_date)
            || record.updated_at_ms < record.created_at_ms
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

fn load_snapshot(conn: &rusqlite::Connection, run_id: &RunId) -> Result<Run, StoreError> {
    let (bytes, digest) = conn.query_row(
        "SELECT snapshot_bytes, snapshot_sha256 FROM orchestration_runs WHERE run_id = ?1",
        [run_id.as_str()],
        |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
    )?;
    if digest != sha256_hex(&bytes) {
        return Err(data_error("orchestration snapshot digest mismatch"));
    }
    let run: Run = serde_json::from_slice(&bytes)?;
    if run.id != *run_id {
        return Err(data_error("orchestration snapshot identity mismatch"));
    }
    Ok(run)
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

fn data_error(message: impl Into<String>) -> StoreError {
    StoreError::Data(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{
        AccessMode, AgentTask, Attempt, AttemptStatus, BlackboardArtifact, BudgetAmount,
        BudgetReservationId, BudgetReservationState, BudgetUsage, CleanupState,
        ConfigurationRevision, EffectReceipt, LeaseResourceKind, OrchestrationMode, OutputContract,
        ProjectRepositoryRecord, ProviderInstanceId, RoutingDecision, RunCommand, RunSeed,
        RunTaskStatus, RunTrigger, SubscriptionKey, ValidatedPlan, WorktreeCleanupEligibility,
        WorktreeDirtyState,
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
                reservation: reservation(),
                grants: BTreeMap::new(),
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
