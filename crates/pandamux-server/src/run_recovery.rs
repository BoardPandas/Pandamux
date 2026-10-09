use crate::attempt_worktree::{AttemptWorktreeAuditStatus, AttemptWorktreeService};
use crate::bundle_store::BundleStore;
use crate::data_paths::DataPaths;
use crate::environment_router::EnvironmentRouter;
use crate::execution_admission::{ExecutionAdmissionClaim, ExecutionAdmissionService};
use crate::remote_execution::{remote_identity_sha256, remote_request_sha256};
use crate::store::{
    AdmissionOwnership, AssignedRunEvent, AttemptDispatchRecord, AttemptDispatchState,
    EffectOwnership, LocalAttemptOutcomeRequest, NodeReceiptKind, NodeReceiptRecord,
    OrchestrationRuntimeRecords, QueueState, RecoveryLeaseDisposition, RunTransitionRequest,
    RunTransitionRows, Store, StoreError, orchestration_snapshot_sha256,
};
use crate::thread_manager::ThreadManager;
use fs4::FileExt;
use pandamux_core::{
    AttemptFailure, AttemptProgressId, AttemptProgressKind, AttemptProgressRecord, AttemptStatus,
    BundleSha, CleanupState, ContentSha256, EffectDeliveryState, EffectReceipt, FailureClass,
    LeaseResourceKind, OrchestrationEffectId, RecoveryReview, RecoveryReviewId,
    RecoveryReviewStatus, RunAttemptId, RunCommand, RunEventId, RunId, RunStatus, RunTaskId,
    ServerIncarnationId,
};
use pandamux_protocol::{
    ORCHESTRATION_EXECUTION_SCHEMA_VERSION, OrchestrationAttemptPrepareResult,
    OrchestrationAttemptReceipt, OrchestrationAttemptStatusParams, OrchestrationExecutionIdentity,
    OrchestrationNodeAttemptState, OrchestrationRepositoryPrepareResult,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Process-lifetime proof that no second server owns this data root. The file
/// is advisory-locked by the OS and the handle is intentionally retained until
/// `Server` drops.
pub struct ServerIncarnationLock {
    owner: ServerIncarnationId,
    _file: File,
}

impl ServerIncarnationLock {
    pub fn acquire(paths: &DataPaths) -> io::Result<Self> {
        paths.ensure_private_roots()?;
        let lock_path = paths.recovery().join("server-incarnation.lock");
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)?;
        FileExt::try_lock(&file).map_err(|error| match error {
            fs4::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::AlreadyExists,
                "another PandaMUX server incarnation owns this data root",
            ),
            fs4::TryLockError::Error(error) => error,
        })?;
        let owner = ServerIncarnationId::generate();
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(owner.as_str().as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(Self { owner, _file: file })
    }

    pub fn owner(&self) -> &ServerIncarnationId {
        &self.owner
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryDisposition {
    Verified,
    PendingReady,
    LeaseReclaimed,
    ExactAttemptReattached,
    RecordedOutcomeReconciled,
    RetainedLiveOwner,
    RemoteOwnerQueriedOnly,
    ReviewRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryAction {
    pub run_id: RunId,
    pub attempt_id: Option<RunAttemptId>,
    pub disposition: RecoveryDisposition,
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub runs_examined: usize,
    pub runs_verified: usize,
    pub effects_ready: usize,
    pub leases_reclaimed: usize,
    pub attempts_reattached: usize,
    pub reviews_created: usize,
    pub reviews_deduplicated: usize,
    pub actions: Vec<RecoveryAction>,
    pub intake_open: bool,
}

#[derive(Debug)]
pub enum RunRecoveryError {
    Store(StoreError),
    Invalid(&'static str),
}

impl std::fmt::Display for RunRecoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "{error}"),
            Self::Invalid(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for RunRecoveryError {}

impl From<StoreError> for RunRecoveryError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

#[derive(Clone)]
pub struct RunRecoveryService {
    store: Store,
    bundle_store: Option<BundleStore>,
    worktrees: Option<AttemptWorktreeService>,
    thread_manager: Option<ThreadManager>,
    environment_router: Option<Arc<EnvironmentRouter>>,
    local_environment_id: Option<pandamux_core::EnvironmentId>,
    live_owners: Arc<std::sync::RwLock<BTreeSet<ServerIncarnationId>>>,
    intake_open: Arc<AtomicBool>,
}

impl RunRecoveryService {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            bundle_store: None,
            worktrees: None,
            thread_manager: None,
            environment_router: None,
            local_environment_id: None,
            live_owners: Arc::new(std::sync::RwLock::new(BTreeSet::new())),
            intake_open: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_environment_router(mut self, environment_router: Arc<EnvironmentRouter>) -> Self {
        if self.local_environment_id.is_none() {
            self.local_environment_id = Some(environment_router.local_environment_id().clone());
        }
        self.environment_router = Some(environment_router);
        self
    }

    pub fn with_local_resources(
        mut self,
        bundle_store: BundleStore,
        worktrees: AttemptWorktreeService,
        thread_manager: ThreadManager,
        local_environment_id: pandamux_core::EnvironmentId,
    ) -> Self {
        self.bundle_store = Some(bundle_store);
        self.worktrees = Some(worktrees);
        self.thread_manager = Some(thread_manager);
        self.local_environment_id = Some(local_environment_id);
        self
    }

    pub fn set_live_owners(&self, owners: BTreeSet<ServerIncarnationId>) {
        *self.live_owners.write().expect("live owner set lock") = owners;
    }

    pub fn accepting_work(&self) -> bool {
        self.intake_open.load(Ordering::Acquire)
    }

    pub fn close_intake(&self) {
        self.intake_open.store(false, Ordering::Release);
    }

    pub async fn recover_nonterminal_runs(
        &self,
        owner: ServerIncarnationId,
        now_ms: u64,
    ) -> Result<RecoveryReport, RunRecoveryError> {
        self.close_intake();
        let mut live_owners = self
            .live_owners
            .read()
            .expect("live owner set lock")
            .clone();
        live_owners.insert(owner.clone());
        let run_ids = self.store.list_nonterminal_orchestration_run_ids()?;
        let mut report = RecoveryReport {
            runs_examined: run_ids.len(),
            ..RecoveryReport::default()
        };

        for run_id in run_ids {
            let verified = match self.store.load_verified_orchestration_run(&run_id) {
                Ok(Some(record)) => Some(record),
                Ok(None) => continue,
                Err(error) => {
                    let reason = replay_failure_reason(&error);
                    let attempts = self.store.load_attempt_dispatches_for_recovery(&run_id)?;
                    if attempts.is_empty() {
                        return Err(RunRecoveryError::Invalid(reason));
                    }
                    self.persist_unverified_run_reviews(
                        &run_id,
                        &attempts,
                        reason,
                        now_ms,
                        &mut report,
                    )?;
                    continue;
                }
            };
            let record = verified.expect("verified run exists");
            report.runs_verified += 1;
            let runtime = self.store.load_orchestration_runtime_records(&run_id)?;
            let mut issues = Vec::new();

            if let Some(bundle_store) = self.bundle_store.as_ref() {
                let bundle_valid = BundleSha::parse(record.run.bundle_sha.clone())
                    .ok()
                    .and_then(|bundle_sha| {
                        bundle_store.load_exact_compiled_bundle(&bundle_sha).ok()
                    })
                    .is_some();
                if !bundle_valid {
                    for attempt in attempts_requiring_resource_audit(&record.run) {
                        issues.push(RecoveryIssue::new(
                            &run_id,
                            &attempt.task_id,
                            &attempt.id,
                            None,
                            "recovery_bundle_missing",
                            &[record.run.bundle_sha.as_str()],
                        ));
                    }
                }
            }

            self.audit_worktrees(&record.run, &runtime, &mut issues)
                .await?;
            classify_terminal_cleanup_debt(&record.run, &runtime, &mut issues);
            self.recover_leases(
                &record.run,
                &runtime,
                now_ms,
                &live_owners,
                &mut report,
                &mut issues,
            )?;
            self.classify_attempts(
                &record.run,
                &runtime,
                &owner,
                now_ms,
                &mut report,
                &mut issues,
            )
            .await;
            self.reconcile_recorded_outcomes(&record.run, &runtime, now_ms, &mut issues)?;

            let derived_review_ids = issues
                .iter()
                .map(|issue| issue.review(now_ms).id)
                .collect::<BTreeSet<_>>();
            let pending_reviews = runtime
                .recovery_reviews
                .iter()
                .filter(|review| review.status == RecoveryReviewStatus::Pending)
                .collect::<Vec<_>>();
            for review in &pending_reviews {
                if derived_review_ids.contains(&review.id) {
                    continue;
                }
                report.reviews_deduplicated += 1;
                report.actions.push(RecoveryAction {
                    run_id: review.run_id.clone(),
                    attempt_id: Some(review.attempt_id.clone()),
                    disposition: RecoveryDisposition::ReviewRequired,
                    reason_code: Some(review.reason_code.clone()),
                });
            }

            if issues.is_empty() && pending_reviews.is_empty() {
                report.actions.push(RecoveryAction {
                    run_id: run_id.clone(),
                    attempt_id: None,
                    disposition: RecoveryDisposition::Verified,
                    reason_code: None,
                });
            } else if !issues.is_empty() {
                self.persist_verified_run_issues(&record.run, issues, now_ms, &mut report)?;
            }
        }

        self.intake_open.store(true, Ordering::Release);
        report.intake_open = true;
        Ok(report)
    }

    async fn audit_worktrees(
        &self,
        run: &pandamux_core::Run,
        runtime: &OrchestrationRuntimeRecords,
        issues: &mut Vec<RecoveryIssue>,
    ) -> Result<(), RunRecoveryError> {
        for attempt in attempts_requiring_resource_audit(run) {
            let Some(worktree_id) = attempt.worktree_id.as_ref() else {
                continue;
            };
            let Some(worktree) = runtime
                .worktrees
                .iter()
                .find(|record| record.worktree.id == *worktree_id)
            else {
                issues.push(RecoveryIssue::new(
                    &run.id,
                    &attempt.task_id,
                    &attempt.id,
                    None,
                    "recovery_worktree_missing",
                    &[worktree_id.as_str()],
                ));
                continue;
            };
            if self
                .local_environment_id
                .as_ref()
                .is_some_and(|local| worktree.worktree.environment_id != *local)
            {
                // A Hub never applies its filesystem clock or path checks to a
                // node-owned worktree. The node's authenticated receipt is the
                // only authority for that resource.
                continue;
            }
            if worktree.worktree.checkpoint_ids.is_empty() {
                issues.push(RecoveryIssue::new(
                    &run.id,
                    &attempt.task_id,
                    &attempt.id,
                    None,
                    "recovery_checkpoint_missing",
                    &[worktree.worktree.id.as_str()],
                ));
                continue;
            }
            let Some(service) = self.worktrees.as_ref() else {
                continue;
            };
            let audit = service
                .audit(worktree.clone())
                .await
                .map_err(|_| RunRecoveryError::Invalid("recovery_worktree_audit_failed"))?;
            match audit.status {
                AttemptWorktreeAuditStatus::Exact { .. } => {}
                AttemptWorktreeAuditStatus::MissingWorktree => issues.push(RecoveryIssue::new(
                    &run.id,
                    &attempt.task_id,
                    &attempt.id,
                    None,
                    "recovery_worktree_missing",
                    &[worktree.worktree.id.as_str()],
                )),
                AttemptWorktreeAuditStatus::ChangedBaseline => issues.push(RecoveryIssue::new(
                    &run.id,
                    &attempt.task_id,
                    &attempt.id,
                    None,
                    "recovery_worktree_baseline_changed",
                    &[
                        worktree.worktree.id.as_str(),
                        &worktree.worktree.baseline_commit,
                    ],
                )),
                AttemptWorktreeAuditStatus::MissingCheckpoint { checkpoint_id } => {
                    issues.push(RecoveryIssue::new(
                        &run.id,
                        &attempt.task_id,
                        &attempt.id,
                        None,
                        "recovery_checkpoint_missing",
                        &[checkpoint_id.as_str()],
                    ));
                }
            }
        }
        Ok(())
    }

    fn recover_leases(
        &self,
        run: &pandamux_core::Run,
        runtime: &OrchestrationRuntimeRecords,
        now_ms: u64,
        live_owners: &BTreeSet<ServerIncarnationId>,
        report: &mut RecoveryReport,
        issues: &mut Vec<RecoveryIssue>,
    ) -> Result<(), RunRecoveryError> {
        for effect in self.store.load_orchestration_effects(&run.id)? {
            if effect.delivery.state == EffectDeliveryState::Pending {
                report.effects_ready += 1;
                report.actions.push(RecoveryAction {
                    run_id: run.id.clone(),
                    attempt_id: effect_attempt_id(&effect.effect),
                    disposition: RecoveryDisposition::PendingReady,
                    reason_code: None,
                });
                continue;
            }
            let (Some(owner), Some(lease_id)) = (
                effect.delivery.owner_incarnation_id.clone(),
                effect.delivery.lease_id.clone(),
            ) else {
                continue;
            };
            let ownership = EffectOwnership {
                effect_id: effect.effect.id.clone(),
                expected_payload_sha256: effect.effect.payload_sha256.clone(),
                owner_incarnation_id: owner,
                lease_id,
                claim_revision: effect.delivery.claim_revision,
            };
            match self
                .store
                .reclaim_expired_effect_claim(&ownership, now_ms, live_owners)?
            {
                RecoveryLeaseDisposition::Reclaimed => {
                    report.leases_reclaimed += 1;
                    report.effects_ready += 1;
                    report.actions.push(RecoveryAction {
                        run_id: run.id.clone(),
                        attempt_id: effect_attempt_id(&effect.effect),
                        disposition: RecoveryDisposition::LeaseReclaimed,
                        reason_code: None,
                    });
                }
                RecoveryLeaseDisposition::ClockAnomaly => {
                    push_effect_issue(
                        run,
                        &effect.effect.id,
                        "recovery_clock_moved_backwards",
                        issues,
                    );
                }
                RecoveryLeaseDisposition::LiveOwner => {
                    report.actions.push(RecoveryAction {
                        run_id: run.id.clone(),
                        attempt_id: effect_attempt_id(&effect.effect),
                        disposition: RecoveryDisposition::RetainedLiveOwner,
                        reason_code: None,
                    });
                }
                RecoveryLeaseDisposition::Unexpired
                | RecoveryLeaseDisposition::ForeignOwner
                | RecoveryLeaseDisposition::Ineligible => {}
            }
        }

        for queue in &runtime.queue_records {
            if queue.state != QueueState::Claimed {
                continue;
            }
            let (Some(owner), Some(lease_id)) =
                (queue.owner_incarnation_id.clone(), queue.lease_id.clone())
            else {
                continue;
            };
            let ownership = AdmissionOwnership {
                queue_id: queue.queue_id.clone(),
                expected_candidate_sha256: queue.candidate_sha256.clone(),
                owner_incarnation_id: owner,
                lease_id,
                claim_revision: queue.claim_revision,
            };
            match self
                .store
                .reclaim_expired_admission(&ownership, now_ms, live_owners)?
            {
                RecoveryLeaseDisposition::Reclaimed => {
                    report.leases_reclaimed += 1;
                    report.actions.push(RecoveryAction {
                        run_id: run.id.clone(),
                        attempt_id: Some(queue.attempt_id.clone()),
                        disposition: RecoveryDisposition::LeaseReclaimed,
                        reason_code: None,
                    });
                }
                RecoveryLeaseDisposition::ClockAnomaly => issues.push(RecoveryIssue::new(
                    &run.id,
                    &queue.task_id,
                    &queue.attempt_id,
                    None,
                    "recovery_clock_moved_backwards",
                    &[queue.queue_id.as_str()],
                )),
                RecoveryLeaseDisposition::LiveOwner => report.actions.push(RecoveryAction {
                    run_id: run.id.clone(),
                    attempt_id: Some(queue.attempt_id.clone()),
                    disposition: RecoveryDisposition::RetainedLiveOwner,
                    reason_code: None,
                }),
                RecoveryLeaseDisposition::Unexpired
                | RecoveryLeaseDisposition::ForeignOwner
                | RecoveryLeaseDisposition::Ineligible => {}
            }
        }

        for lease in &runtime.leases {
            if now_ms < lease.last_heartbeat_ms {
                if let Some((task_id, attempt_id)) = lease_attempt(runtime, lease) {
                    issues.push(RecoveryIssue::new(
                        &run.id,
                        task_id,
                        attempt_id,
                        None,
                        "recovery_clock_moved_backwards",
                        &[lease.id.as_str()],
                    ));
                }
            } else if lease.resource_kind == LeaseResourceKind::RemoteAttempt
                && now_ms >= lease.expires_at_ms
            {
                report.actions.push(RecoveryAction {
                    run_id: run.id.clone(),
                    attempt_id: lease_attempt(runtime, lease).map(|(_, attempt)| attempt.clone()),
                    disposition: RecoveryDisposition::RemoteOwnerQueriedOnly,
                    reason_code: Some("recovery_remote_owner_clock_authoritative".to_string()),
                });
            }
        }
        Ok(())
    }

    async fn classify_attempts(
        &self,
        run: &pandamux_core::Run,
        runtime: &OrchestrationRuntimeRecords,
        recovery_owner: &ServerIncarnationId,
        now_ms: u64,
        report: &mut RecoveryReport,
        issues: &mut Vec<RecoveryIssue>,
    ) {
        for dispatch in &runtime.attempt_dispatches {
            if dispatch.state == AttemptDispatchState::Prepared {
                issues.push(RecoveryIssue::new(
                    &run.id,
                    &dispatch.task_id,
                    &dispatch.attempt_id,
                    Some(&dispatch.effect_id),
                    "recovery_prepared_attempt_requires_resume",
                    &[
                        dispatch.effect_id.as_str(),
                        dispatch.thread_id.as_str(),
                        dispatch.turn_attempt_id.as_str(),
                    ],
                ));
                continue;
            }
            if !matches!(
                dispatch.state,
                AttemptDispatchState::DispatchStarted
                    | AttemptDispatchState::Accepted
                    | AttemptDispatchState::CancelRequested
                    | AttemptDispatchState::Ambiguous
            ) {
                continue;
            }
            let is_local = self.local_environment_id.as_ref().is_none_or(|local| {
                runtime.worktrees.iter().any(|worktree| {
                    worktree.worktree.attempt_id == dispatch.attempt_id
                        && worktree.worktree.environment_id == *local
                })
            });
            if is_local
                && let Some(manager) = self.thread_manager.as_ref()
                && manager
                    .prepared_turn_prompt_accepted(
                        &dispatch.thread_id,
                        &dispatch.turn_id,
                        &dispatch.turn_attempt_id,
                    )
                    .await
            {
                report.attempts_reattached += 1;
                report.actions.push(RecoveryAction {
                    run_id: run.id.clone(),
                    attempt_id: Some(dispatch.attempt_id.clone()),
                    disposition: RecoveryDisposition::ExactAttemptReattached,
                    reason_code: None,
                });
                continue;
            }
            if !is_local && self.environment_router.is_some() {
                match self
                    .query_exact_remote_status(run, runtime, dispatch, now_ms)
                    .await
                {
                    Ok(receipt)
                        if matches!(
                            receipt.state,
                            OrchestrationNodeAttemptState::DispatchStarted
                                | OrchestrationNodeAttemptState::Accepted
                        ) =>
                    {
                        let (ownership, _) = match self
                            .reattach_remote_ownership(dispatch, recovery_owner, now_ms)
                            .await
                        {
                            Ok(ownership) => ownership,
                            Err(_) => {
                                issues.push(RecoveryIssue::new(
                                    &run.id,
                                    &dispatch.task_id,
                                    &dispatch.attempt_id,
                                    Some(&dispatch.effect_id),
                                    "recovery_remote_ownership_reattach_failed",
                                    &[receipt.receipt_sha256.as_str()],
                                ));
                                continue;
                            }
                        };
                        if self
                            .reconcile_remote_live_attempt(
                                run, dispatch, &receipt, &ownership, now_ms,
                            )
                            .is_err()
                        {
                            issues.push(RecoveryIssue::new(
                                &run.id,
                                &dispatch.task_id,
                                &dispatch.attempt_id,
                                Some(&dispatch.effect_id),
                                "recovery_remote_live_reconcile_failed",
                                &[receipt.receipt_sha256.as_str()],
                            ));
                            continue;
                        }
                        report.attempts_reattached += 1;
                        report.actions.push(RecoveryAction {
                            run_id: run.id.clone(),
                            attempt_id: Some(dispatch.attempt_id.clone()),
                            disposition: RecoveryDisposition::ExactAttemptReattached,
                            reason_code: None,
                        });
                        continue;
                    }
                    Ok(receipt)
                        if matches!(
                            receipt.state,
                            OrchestrationNodeAttemptState::Terminal
                                | OrchestrationNodeAttemptState::Cancelled
                                | OrchestrationNodeAttemptState::NotStarted
                                | OrchestrationNodeAttemptState::Ambiguous
                        ) =>
                    {
                        let (ownership, admission_claim) = match self
                            .reattach_remote_ownership(dispatch, recovery_owner, now_ms)
                            .await
                        {
                            Ok(ownership) => ownership,
                            Err(_) => {
                                issues.push(RecoveryIssue::new(
                                    &run.id,
                                    &dispatch.task_id,
                                    &dispatch.attempt_id,
                                    Some(&dispatch.effect_id),
                                    "recovery_remote_ownership_reattach_failed",
                                    &[receipt.receipt_sha256.as_str()],
                                ));
                                continue;
                            }
                        };
                        match self
                            .reconcile_remote_outcome(
                                run,
                                dispatch,
                                &receipt,
                                &ownership,
                                &admission_claim,
                                now_ms,
                            )
                            .await
                        {
                            Ok(Some(issue)) => issues.push(issue),
                            Ok(None) => report.actions.push(RecoveryAction {
                                run_id: run.id.clone(),
                                attempt_id: Some(dispatch.attempt_id.clone()),
                                disposition: RecoveryDisposition::RecordedOutcomeReconciled,
                                reason_code: None,
                            }),
                            Err(reason) => issues.push(RecoveryIssue::new(
                                &run.id,
                                &dispatch.task_id,
                                &dispatch.attempt_id,
                                Some(&dispatch.effect_id),
                                reason,
                                &[receipt.receipt_sha256.as_str()],
                            )),
                        }
                        continue;
                    }
                    Ok(receipt) => {
                        issues.push(RecoveryIssue::new(
                            &run.id,
                            &dispatch.task_id,
                            &dispatch.attempt_id,
                            Some(&dispatch.effect_id),
                            "recovery_dispatch_marker_ambiguous",
                            &[receipt.receipt_sha256.as_str()],
                        ));
                        continue;
                    }
                    Err(reason) => {
                        issues.push(RecoveryIssue::new(
                            &run.id,
                            &dispatch.task_id,
                            &dispatch.attempt_id,
                            Some(&dispatch.effect_id),
                            reason,
                            &[
                                dispatch.effect_id.as_str(),
                                dispatch.turn_attempt_id.as_str(),
                            ],
                        ));
                        continue;
                    }
                }
            }
            let reason = match dispatch.state {
                AttemptDispatchState::Ambiguous => "recovery_attempt_already_ambiguous",
                AttemptDispatchState::DispatchStarted => "recovery_dispatch_marker_ambiguous",
                AttemptDispatchState::Accepted | AttemptDispatchState::CancelRequested => {
                    if is_local {
                        "recovery_local_attempt_unavailable"
                    } else {
                        "recovery_remote_status_required"
                    }
                }
                _ => unreachable!("filtered active dispatch state"),
            };
            issues.push(RecoveryIssue::new(
                &run.id,
                &dispatch.task_id,
                &dispatch.attempt_id,
                Some(&dispatch.effect_id),
                reason,
                &[
                    dispatch.effect_id.as_str(),
                    dispatch.thread_id.as_str(),
                    dispatch.turn_attempt_id.as_str(),
                ],
            ));
        }
    }

    async fn query_exact_remote_status(
        &self,
        run: &pandamux_core::Run,
        runtime: &OrchestrationRuntimeRecords,
        dispatch: &AttemptDispatchRecord,
        now_ms: u64,
    ) -> Result<OrchestrationAttemptReceipt, &'static str> {
        const MAX_RECOVERABLE_LEASE_GENERATION: u64 = 100_000;

        let router = self
            .environment_router
            .as_ref()
            .ok_or("recovery_remote_status_required")?;
        let effect = self
            .store
            .get_orchestration_effect(&dispatch.effect_id)
            .map_err(|_| "recovery_remote_identity_unavailable")?
            .ok_or("recovery_remote_identity_unavailable")?;
        if effect.delivery.claim_revision == 0
            || effect.delivery.claim_revision > MAX_RECOVERABLE_LEASE_GENERATION
        {
            return Err("recovery_remote_identity_unavailable");
        }
        let prepare_record = runtime
            .node_receipts
            .iter()
            .find(|record| {
                record.attempt_id == dispatch.attempt_id
                    && record.effect_id == dispatch.effect_id
                    && record.request_kind == NodeReceiptKind::AttemptPrepare
            })
            .ok_or("recovery_remote_prepare_receipt_missing")?;
        let prepare: OrchestrationAttemptPrepareResult =
            serde_json::from_slice(&prepare_record.result_bytes)
                .map_err(|_| "recovery_remote_prepare_receipt_invalid")?;
        let repository_record = runtime
            .node_receipts
            .iter()
            .find(|record| {
                record.attempt_id == dispatch.attempt_id
                    && record.effect_id == dispatch.effect_id
                    && record.request_kind == NodeReceiptKind::RepositoryPrepare
            })
            .ok_or("recovery_remote_repository_receipt_missing")?;
        let repository: OrchestrationRepositoryPrepareResult =
            serde_json::from_slice(&repository_record.result_bytes)
                .map_err(|_| "recovery_remote_repository_receipt_invalid")?;
        if prepare.receipt.environment_id != repository.environment_id
            || prepare.receipt.node_store_id != repository.node_store_id
            || prepare.receipt.run_id != run.id
            || prepare.receipt.task_id != dispatch.task_id
            || prepare.receipt.attempt_id != dispatch.attempt_id
            || prepare.receipt.effect_id != dispatch.effect_id
            || prepare.receipt.thread_id != dispatch.thread_id
            || prepare.receipt.turn_id != dispatch.turn_id
            || prepare.receipt.turn_attempt_id != dispatch.turn_attempt_id
            || repository.repository_id.as_str().trim().is_empty()
        {
            return Err("recovery_remote_prepare_receipt_invalid");
        }

        let payload_sha256 = ContentSha256::parse(effect.effect.payload_sha256.clone())
            .map_err(|_| "recovery_remote_identity_unavailable")?;
        let bundle_sha = BundleSha::parse(dispatch.bundle_sha.clone())
            .map_err(|_| "recovery_remote_identity_unavailable")?;
        let idempotency_key = format!("recovery-status:{}:{now_ms}", dispatch.attempt_id.as_str());
        let mut exact_identity = None;
        for lease_generation in (1..=effect.delivery.claim_revision).rev() {
            let candidate = OrchestrationExecutionIdentity {
                schema_version: ORCHESTRATION_EXECUTION_SCHEMA_VERSION,
                run_id: run.id.clone(),
                task_id: dispatch.task_id.clone(),
                attempt_id: dispatch.attempt_id.clone(),
                effect_id: dispatch.effect_id.clone(),
                payload_sha256: payload_sha256.clone(),
                repository_id: repository.repository_id.clone(),
                baseline_commit: repository.baseline_commit.clone(),
                bundle_sha: bundle_sha.clone(),
                route: dispatch.route.clone(),
                lease_generation,
                idempotency_key: idempotency_key.clone(),
            };
            if remote_identity_sha256(&candidate).ok().as_ref()
                == Some(&prepare.receipt.identity_sha256)
            {
                exact_identity = Some(candidate);
                break;
            }
        }
        let identity = exact_identity.ok_or("recovery_remote_identity_unavailable")?;
        let params = OrchestrationAttemptStatusParams {
            identity: identity.clone(),
        };
        let request_sha256 =
            remote_request_sha256(&params).map_err(|_| "recovery_remote_status_receipt_invalid")?;
        let result = router
            .orchestration_attempt_status(&prepare.receipt.environment_id, &params)
            .await
            .map_err(|_| "recovery_remote_status_unavailable")?;
        let receipt = &result.receipt;
        let mut unsigned = receipt.clone();
        unsigned.receipt_sha256 =
            ContentSha256::parse("0".repeat(64)).expect("zero content digest is valid");
        let sealed = remote_request_sha256(&unsigned)
            .map_err(|_| "recovery_remote_status_receipt_invalid")?;
        if receipt.schema_version != ORCHESTRATION_EXECUTION_SCHEMA_VERSION
            || receipt.request_sha256 != request_sha256
            || receipt.receipt_sha256 != sealed
            || receipt.environment_id != prepare.receipt.environment_id
            || receipt.node_store_id != prepare.receipt.node_store_id
            || receipt.identity_sha256 != prepare.receipt.identity_sha256
            || receipt.prepared_execution_sha256 != prepare.receipt.prepared_execution_sha256
            || receipt.run_id != run.id
            || receipt.task_id != dispatch.task_id
            || receipt.attempt_id != dispatch.attempt_id
            || receipt.effect_id != dispatch.effect_id
            || receipt.thread_id != dispatch.thread_id
            || receipt.turn_id != dispatch.turn_id
            || receipt.turn_attempt_id != dispatch.turn_attempt_id
        {
            return Err("recovery_remote_status_receipt_invalid");
        }
        let result_bytes =
            serde_json::to_vec(&result).map_err(|_| "recovery_remote_status_receipt_invalid")?;
        let result_sha256 = format!("{:x}", Sha256::digest(&result_bytes));
        let receipt_id = format!(
            "node-receipt-recovery-status-{}",
            &stable_digest(&[
                dispatch.attempt_id.as_str(),
                request_sha256.as_str(),
                &result_sha256,
            ])[..32]
        );
        self.store
            .record_hub_node_receipt(&NodeReceiptRecord {
                receipt_id,
                node_store_id: receipt.node_store_id.clone(),
                run_id: run.id.clone(),
                task_id: dispatch.task_id.clone(),
                attempt_id: dispatch.attempt_id.clone(),
                effect_id: dispatch.effect_id.clone(),
                request_kind: NodeReceiptKind::AttemptStatus,
                idempotency_key,
                request_sha256: request_sha256.as_str().to_string(),
                result_sha256,
                result_bytes,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            })
            .map_err(|_| "recovery_remote_status_receipt_invalid")?;
        Ok(result.receipt)
    }

    async fn reattach_remote_ownership(
        &self,
        dispatch: &AttemptDispatchRecord,
        recovery_owner: &ServerIncarnationId,
        now_ms: u64,
    ) -> Result<(EffectOwnership, ExecutionAdmissionClaim), &'static str> {
        let effect = self
            .store
            .get_orchestration_effect(&dispatch.effect_id)
            .map_err(|_| "recovery_remote_ownership_reattach_failed")?
            .ok_or("recovery_remote_ownership_reattach_failed")?;
        let ownership = self
            .store
            .reattach_remote_attempt_ownership(
                &dispatch.attempt_id,
                &dispatch.effect_id,
                &effect.effect.payload_sha256,
                recovery_owner,
                now_ms,
                crate::attempt_watchdog::DEFAULT_LEASE_TTL_MS,
            )
            .map_err(|_| "recovery_remote_ownership_reattach_failed")?;
        let admission =
            ExecutionAdmissionService::with_owner(self.store.clone(), recovery_owner.clone());
        let claim = admission
            .load_owned_attempt_claim(&dispatch.run_id, &dispatch.attempt_id)
            .await
            .map_err(|_| "recovery_remote_ownership_reattach_failed")?;
        let claim = admission
            .mark_dispatched(&claim)
            .await
            .map_err(|_| "recovery_remote_ownership_reattach_failed")?;
        Ok((ownership, claim))
    }

    fn reconcile_remote_live_attempt(
        &self,
        run: &pandamux_core::Run,
        dispatch: &AttemptDispatchRecord,
        receipt: &OrchestrationAttemptReceipt,
        ownership: &EffectOwnership,
        now_ms: u64,
    ) -> Result<(), &'static str> {
        if receipt.state == OrchestrationNodeAttemptState::DispatchStarted {
            return Ok(());
        }
        if receipt.state != OrchestrationNodeAttemptState::Accepted {
            return Err("recovery_remote_status_receipt_invalid");
        }
        let recorded_at_ms = now_ms.max(dispatch.updated_at_ms).max(
            self.store
                .get_orchestration_effect(&dispatch.effect_id)
                .map_err(|_| "recovery_remote_live_reconcile_failed")?
                .ok_or("recovery_remote_live_reconcile_failed")?
                .updated_at_ms,
        );
        let current_dispatch = self
            .store
            .get_attempt_dispatch(&dispatch.attempt_id)
            .map_err(|_| "recovery_remote_live_reconcile_failed")?
            .ok_or("recovery_remote_live_reconcile_failed")?;
        if current_dispatch.state == AttemptDispatchState::DispatchStarted {
            self.store
                .mark_local_attempt_accepted(ownership, &dispatch.attempt_id, recorded_at_ms)
                .map_err(|_| "recovery_remote_live_reconcile_failed")?;
        } else if current_dispatch.state != AttemptDispatchState::Accepted {
            return Err("recovery_remote_live_reconcile_failed");
        }

        let current = self
            .store
            .load_verified_orchestration_run(&run.id)
            .map_err(|_| "recovery_remote_live_reconcile_failed")?
            .ok_or("recovery_remote_live_reconcile_failed")?;
        match current.run.attempts.get(&dispatch.attempt_id) {
            Some(attempt) if attempt.status == AttemptStatus::DispatchRequested => {
                commit_recovery_command(
                    &self.store,
                    &run.id,
                    RunCommand::StartAttempt {
                        attempt_id: dispatch.attempt_id.clone(),
                        thread_id: receipt.thread_id.clone(),
                        turn_attempt_id: receipt.turn_attempt_id.clone(),
                    },
                    recorded_at_ms,
                    "remote-start",
                )
                .map_err(|_| "recovery_remote_live_reconcile_failed")?;
            }
            Some(attempt) if attempt.status == AttemptStatus::Running => {}
            _ => return Err("recovery_remote_live_reconcile_failed"),
        }

        let current_dispatch = self
            .store
            .get_attempt_dispatch(&dispatch.attempt_id)
            .map_err(|_| "recovery_remote_live_reconcile_failed")?
            .ok_or("recovery_remote_live_reconcile_failed")?;
        if receipt.boundary > current_dispatch.last_boundary {
            self.store
                .record_local_attempt_boundary(
                    ownership,
                    &dispatch.attempt_id,
                    receipt.boundary,
                    recorded_at_ms,
                )
                .map_err(|_| "recovery_remote_live_reconcile_failed")?;
            commit_recovery_command(
                &self.store,
                &run.id,
                RunCommand::AdvanceAttemptBoundary {
                    attempt_id: dispatch.attempt_id.clone(),
                    boundary: receipt.boundary,
                },
                recorded_at_ms,
                "remote-boundary",
            )
            .map_err(|_| "recovery_remote_live_reconcile_failed")?;
        }
        Ok(())
    }

    async fn reconcile_remote_outcome(
        &self,
        run: &pandamux_core::Run,
        dispatch: &AttemptDispatchRecord,
        receipt: &OrchestrationAttemptReceipt,
        ownership: &EffectOwnership,
        admission_claim: &ExecutionAdmissionClaim,
        now_ms: u64,
    ) -> Result<Option<RecoveryIssue>, &'static str> {
        let recorded_at_ms = now_ms.max(dispatch.updated_at_ms).max(
            self.store
                .get_orchestration_effect(&dispatch.effect_id)
                .map_err(|_| "recovery_remote_outcome_store_failed")?
                .ok_or("recovery_remote_outcome_store_failed")?
                .updated_at_ms,
        );
        if dispatch.state == AttemptDispatchState::DispatchStarted
            && matches!(
                receipt.state,
                OrchestrationNodeAttemptState::Accepted
                    | OrchestrationNodeAttemptState::Terminal
                    | OrchestrationNodeAttemptState::Cancelled
            )
        {
            self.store
                .mark_local_attempt_accepted(ownership, &dispatch.attempt_id, recorded_at_ms)
                .map_err(|_| "recovery_remote_outcome_store_failed")?;
        }
        let current_dispatch = self
            .store
            .get_attempt_dispatch(&dispatch.attempt_id)
            .map_err(|_| "recovery_remote_outcome_store_failed")?
            .ok_or("recovery_remote_outcome_store_failed")?;
        if receipt.boundary > current_dispatch.last_boundary
            && current_dispatch.state == AttemptDispatchState::Accepted
        {
            self.store
                .record_local_attempt_boundary(
                    ownership,
                    &dispatch.attempt_id,
                    receipt.boundary,
                    recorded_at_ms,
                )
                .map_err(|_| "recovery_remote_outcome_store_failed")?;
            commit_recovery_command(
                &self.store,
                &run.id,
                RunCommand::AdvanceAttemptBoundary {
                    attempt_id: dispatch.attempt_id.clone(),
                    boundary: receipt.boundary,
                },
                recorded_at_ms,
                "remote-boundary",
            )
            .map_err(|_| "recovery_remote_outcome_store_failed")?;
        }

        let current_dispatch = self
            .store
            .get_attempt_dispatch(&dispatch.attempt_id)
            .map_err(|_| "recovery_remote_outcome_store_failed")?
            .ok_or("recovery_remote_outcome_store_failed")?;
        let outcome_digest = receipt
            .result_sha256
            .as_ref()
            .unwrap_or(&receipt.receipt_sha256)
            .as_str()
            .to_string();
        let (dispatch_state, safe_error_code, ambiguity_reason) = match receipt.state {
            OrchestrationNodeAttemptState::NotStarted => (
                AttemptDispatchState::NotStarted,
                Some(
                    receipt
                        .safe_error_code
                        .clone()
                        .unwrap_or_else(|| "remote_dispatch_not_started".to_string()),
                ),
                None,
            ),
            OrchestrationNodeAttemptState::Ambiguous => (
                AttemptDispatchState::Ambiguous,
                None,
                Some("recovery_remote_attempt_ambiguous".to_string()),
            ),
            OrchestrationNodeAttemptState::Cancelled
                if current_dispatch.state == AttemptDispatchState::CancelRequested =>
            {
                (AttemptDispatchState::Cancelled, None, None)
            }
            OrchestrationNodeAttemptState::Terminal | OrchestrationNodeAttemptState::Cancelled => {
                (AttemptDispatchState::Terminal, None, None)
            }
            _ => return Err("recovery_remote_status_receipt_invalid"),
        };
        if !matches!(
            current_dispatch.state,
            AttemptDispatchState::Terminal
                | AttemptDispatchState::Cancelled
                | AttemptDispatchState::NotStarted
                | AttemptDispatchState::Ambiguous
        ) {
            self.store
                .record_local_attempt_outcome(
                    &LocalAttemptOutcomeRequest {
                        ownership: ownership.clone(),
                        attempt_id: dispatch.attempt_id.clone(),
                        state: dispatch_state,
                        boundary: receipt.boundary,
                        result_digest: outcome_digest.clone(),
                        safe_error_code,
                        ambiguity_reason: ambiguity_reason.clone(),
                        recorded_at_ms,
                    },
                    None,
                )
                .map_err(|_| "recovery_remote_outcome_store_failed")?;
        }

        if dispatch_state == AttemptDispatchState::Ambiguous {
            let current = self
                .store
                .load_verified_orchestration_run(&run.id)
                .map_err(|_| "recovery_remote_outcome_store_failed")?
                .ok_or("recovery_remote_outcome_store_failed")?;
            if current
                .run
                .attempts
                .get(&dispatch.attempt_id)
                .is_some_and(|attempt| !attempt.ambiguous_effect)
            {
                commit_recovery_command(
                    &self.store,
                    &run.id,
                    RunCommand::MarkAttemptAmbiguous {
                        attempt_id: dispatch.attempt_id.clone(),
                        reason_code: "recovery_remote_attempt_ambiguous".to_string(),
                    },
                    recorded_at_ms,
                    "remote-ambiguous",
                )
                .map_err(|_| "recovery_remote_outcome_store_failed")?;
            }
            record_recovery_terminal_progress(
                &self.store,
                dispatch,
                &outcome_digest,
                recorded_at_ms,
            )
            .map_err(|_| "recovery_remote_outcome_store_failed")?;
            return Ok(Some(RecoveryIssue::new(
                &run.id,
                &dispatch.task_id,
                &dispatch.attempt_id,
                Some(&dispatch.effect_id),
                "recovery_remote_attempt_ambiguous",
                &[receipt.receipt_sha256.as_str()],
            )));
        }

        let terminal_status = match receipt.state {
            OrchestrationNodeAttemptState::NotStarted => AttemptStatus::Failed,
            _ => receipt
                .terminal_status
                .ok_or("recovery_remote_status_receipt_invalid")?,
        };
        let usage_reconciled = receipt.state == OrchestrationNodeAttemptState::NotStarted;
        if usage_reconciled {
            self.store
                .release_verified_not_started_budget(&run.id, &dispatch.attempt_id, recorded_at_ms)
                .map_err(|_| "recovery_remote_budget_release_failed")?;
            ExecutionAdmissionService::new(self.store.clone())
                .release(admission_claim)
                .await
                .map_err(|_| "recovery_remote_capacity_release_failed")?;
        }
        let current = self
            .store
            .load_verified_orchestration_run(&run.id)
            .map_err(|_| "recovery_remote_outcome_store_failed")?
            .ok_or("recovery_remote_outcome_store_failed")?;
        if current
            .run
            .attempts
            .get(&dispatch.attempt_id)
            .is_some_and(|attempt| {
                !matches!(
                    attempt.status,
                    AttemptStatus::Succeeded
                        | AttemptStatus::Failed
                        | AttemptStatus::Cancelled
                        | AttemptStatus::Interrupted
                )
            })
        {
            let failure = match terminal_status {
                AttemptStatus::Succeeded => None,
                AttemptStatus::Cancelled => Some(AttemptFailure {
                    class: FailureClass::Cancelled,
                    code: "remote_attempt_cancelled".to_string(),
                    safe_message: "The remote provider attempt was cancelled".to_string(),
                }),
                AttemptStatus::Interrupted => Some(AttemptFailure {
                    class: FailureClass::Provider,
                    code: "remote_attempt_interrupted".to_string(),
                    safe_message: "The remote provider attempt was interrupted".to_string(),
                }),
                AttemptStatus::Failed => Some(AttemptFailure {
                    class: FailureClass::Provider,
                    code: receipt
                        .safe_error_code
                        .clone()
                        .unwrap_or_else(|| "remote_provider_failed".to_string()),
                    safe_message: "The remote provider attempt failed".to_string(),
                }),
                AttemptStatus::DispatchRequested | AttemptStatus::Running => {
                    return Err("recovery_remote_status_receipt_invalid");
                }
            };
            commit_recovery_command(
                &self.store,
                &run.id,
                RunCommand::SettleAttempt {
                    attempt_id: dispatch.attempt_id.clone(),
                    status: terminal_status,
                    failure,
                    usage_reconciled,
                },
                recorded_at_ms,
                "remote-settle",
            )
            .map_err(|_| "recovery_remote_outcome_store_failed")?;
        }
        let current = self
            .store
            .load_verified_orchestration_run(&run.id)
            .map_err(|_| "recovery_remote_outcome_store_failed")?
            .ok_or("recovery_remote_outcome_store_failed")?;
        acknowledge_recorded_effect(
            &self.store,
            &current.run,
            &dispatch.effect_id,
            &outcome_digest,
            recorded_at_ms,
        )
        .map_err(|_| "recovery_remote_outcome_store_failed")?;
        record_recovery_terminal_progress(&self.store, dispatch, &outcome_digest, recorded_at_ms)
            .map_err(|_| "recovery_remote_outcome_store_failed")?;
        if usage_reconciled {
            Ok(None)
        } else {
            Ok(Some(RecoveryIssue::new(
                &run.id,
                &dispatch.task_id,
                &dispatch.attempt_id,
                Some(&dispatch.effect_id),
                "recovery_usage_reconciliation_unknown",
                &[receipt.receipt_sha256.as_str()],
            )))
        }
    }

    fn reconcile_recorded_outcomes(
        &self,
        run: &pandamux_core::Run,
        runtime: &OrchestrationRuntimeRecords,
        now_ms: u64,
        issues: &mut Vec<RecoveryIssue>,
    ) -> Result<(), RunRecoveryError> {
        for effect in self.store.load_orchestration_effects(&run.id)? {
            if effect.delivery.state != EffectDeliveryState::OutcomeRecorded {
                continue;
            }
            let Some(attempt_id) = effect_attempt_id(&effect.effect) else {
                continue;
            };
            let current = self
                .store
                .load_verified_orchestration_run(&run.id)?
                .ok_or(RunRecoveryError::Invalid("recovery_run_disappeared"))?;
            let Some(attempt) = current.run.attempts.get(&attempt_id) else {
                continue;
            };
            let Some(result_digest) = effect.delivery.result_digest.as_ref() else {
                continue;
            };
            if matches!(
                attempt.status,
                pandamux_core::AttemptStatus::Succeeded
                    | pandamux_core::AttemptStatus::Failed
                    | pandamux_core::AttemptStatus::Cancelled
                    | pandamux_core::AttemptStatus::Interrupted
            ) {
                acknowledge_recorded_effect(
                    &self.store,
                    &current.run,
                    &effect.effect.id,
                    result_digest,
                    now_ms,
                )?;
            } else {
                let dispatch = runtime
                    .attempt_dispatches
                    .iter()
                    .find(|dispatch| dispatch.attempt_id == attempt_id);
                issues.push(RecoveryIssue::new(
                    &run.id,
                    &attempt.task_id,
                    &attempt.id,
                    Some(&effect.effect.id),
                    "recovery_outcome_requires_settlement",
                    &[
                        effect.effect.id.as_str(),
                        dispatch
                            .and_then(|record| record.outcome_digest.as_deref())
                            .unwrap_or(result_digest),
                    ],
                ));
            }
        }
        Ok(())
    }

    fn persist_verified_run_issues(
        &self,
        run: &pandamux_core::Run,
        issues: Vec<RecoveryIssue>,
        now_ms: u64,
        report: &mut RecoveryReport,
    ) -> Result<(), RunRecoveryError> {
        let current = self
            .store
            .load_verified_orchestration_run(&run.id)?
            .ok_or(RunRecoveryError::Invalid("recovery_run_disappeared"))?;
        let run = &current.run;
        let reviews = issues
            .iter()
            .map(|issue| issue.review(now_ms))
            .collect::<Vec<_>>();
        if matches!(run.status, RunStatus::Ready | RunStatus::Running) {
            let reason = issues[0].reason_code.clone();
            let event_digest = stable_digest(&[
                run.id.as_str(),
                reason.as_str(),
                reviews[0].evidence_digest.as_str(),
            ]);
            self.store.commit_run_transition(
                &RunTransitionRequest {
                    run_id: run.id.clone(),
                    expected_last_seq: run.last_seq,
                    expected_snapshot_sha256: orchestration_snapshot_sha256(run),
                    command: RunCommand::Pause {
                        reason: reason.clone(),
                    },
                    assigned_events: vec![AssignedRunEvent {
                        event_id: RunEventId::new(format!(
                            "run-event-recovery-pause-{event_digest}"
                        )),
                        idempotency_key: format!("recovery-pause-{event_digest}"),
                        at_ms: now_ms.max(run.updated_at_ms),
                    }],
                    rows: RunTransitionRows {
                        recovery_reviews: reviews.clone(),
                        ..RunTransitionRows::default()
                    },
                },
                None,
            )?;
            for review in &reviews {
                if self.store.get_recovery_review(&review.id)?.is_some() {
                    report.reviews_created += 1;
                }
            }
        } else {
            for review in &reviews {
                let existed = self.store.get_recovery_review(&review.id)?.is_some();
                let stored = self.store.upsert_recovery_review(review)?;
                if existed {
                    report.reviews_deduplicated += 1;
                } else if stored.id == review.id {
                    report.reviews_created += 1;
                }
            }
        }
        for issue in issues {
            report.actions.push(RecoveryAction {
                run_id: issue.run_id,
                attempt_id: Some(issue.attempt_id),
                disposition: RecoveryDisposition::ReviewRequired,
                reason_code: Some(issue.reason_code),
            });
        }
        Ok(())
    }

    fn persist_unverified_run_reviews(
        &self,
        run_id: &RunId,
        attempts: &[AttemptDispatchRecord],
        reason: &'static str,
        now_ms: u64,
        report: &mut RecoveryReport,
    ) -> Result<(), RunRecoveryError> {
        for attempt in attempts {
            let issue = RecoveryIssue::new(
                run_id,
                &attempt.task_id,
                &attempt.attempt_id,
                Some(&attempt.effect_id),
                reason,
                &[
                    attempt.effect_id.as_str(),
                    attempt.thread_id.as_str(),
                    &attempt.bundle_sha,
                ],
            );
            let review = issue.review(now_ms);
            let existed = self.store.get_recovery_review(&review.id)?.is_some();
            let stored = self.store.upsert_recovery_review(&review)?;
            if existed {
                report.reviews_deduplicated += 1;
            } else if stored.id == review.id {
                report.reviews_created += 1;
            }
            report.actions.push(RecoveryAction {
                run_id: run_id.clone(),
                attempt_id: Some(attempt.attempt_id.clone()),
                disposition: RecoveryDisposition::ReviewRequired,
                reason_code: Some(reason.to_string()),
            });
        }
        Ok(())
    }
}

#[derive(Clone)]
struct RecoveryIssue {
    run_id: RunId,
    task_id: RunTaskId,
    attempt_id: RunAttemptId,
    effect_id: Option<OrchestrationEffectId>,
    reason_code: String,
    evidence_digest: String,
}

impl RecoveryIssue {
    fn new(
        run_id: &RunId,
        task_id: &RunTaskId,
        attempt_id: &RunAttemptId,
        effect_id: Option<&OrchestrationEffectId>,
        reason_code: &str,
        evidence: &[&str],
    ) -> Self {
        let mut fields = vec![
            run_id.as_str(),
            task_id.as_str(),
            attempt_id.as_str(),
            effect_id.map(OrchestrationEffectId::as_str).unwrap_or(""),
            reason_code,
        ];
        fields.extend_from_slice(evidence);
        Self {
            run_id: run_id.clone(),
            task_id: task_id.clone(),
            attempt_id: attempt_id.clone(),
            effect_id: effect_id.cloned(),
            reason_code: reason_code.to_string(),
            evidence_digest: stable_digest(&fields),
        }
    }

    fn review(&self, created_at_ms: u64) -> RecoveryReview {
        let identity = stable_digest(&[
            self.run_id.as_str(),
            self.task_id.as_str(),
            self.attempt_id.as_str(),
            self.effect_id
                .as_ref()
                .map(OrchestrationEffectId::as_str)
                .unwrap_or(""),
            &self.reason_code,
            &self.evidence_digest,
        ]);
        RecoveryReview {
            id: RecoveryReviewId::new(format!("recovery-review-{identity}")),
            run_id: self.run_id.clone(),
            task_id: self.task_id.clone(),
            attempt_id: self.attempt_id.clone(),
            effect_id: self.effect_id.clone(),
            reason_code: self.reason_code.clone(),
            evidence_digest: self.evidence_digest.clone(),
            status: RecoveryReviewStatus::Pending,
            created_at_ms,
            resolved_at_ms: None,
            resolution: None,
        }
    }
}

fn attempts_requiring_resource_audit(
    run: &pandamux_core::Run,
) -> impl Iterator<Item = &pandamux_core::Attempt> {
    run.attempts.values().filter(|attempt| {
        matches!(
            attempt.status,
            AttemptStatus::DispatchRequested | AttemptStatus::Running
        ) || (attempt_is_terminal(attempt.status)
            && attempt.cleanup_state != CleanupState::Verified)
    })
}

fn classify_terminal_cleanup_debt(
    run: &pandamux_core::Run,
    runtime: &OrchestrationRuntimeRecords,
    issues: &mut Vec<RecoveryIssue>,
) {
    for attempt in run.attempts.values().filter(|attempt| {
        attempt_is_terminal(attempt.status) && attempt.cleanup_state != CleanupState::Verified
    }) {
        if runtime.recovery_reviews.iter().any(|review| {
            review.status == RecoveryReviewStatus::Pending
                && review.run_id == run.id
                && review.task_id == attempt.task_id
                && review.attempt_id == attempt.id
        }) {
            continue;
        }
        let dispatch = runtime
            .attempt_dispatches
            .iter()
            .find(|dispatch| dispatch.attempt_id == attempt.id);
        let reason = match attempt.cleanup_state {
            CleanupState::NotStarted | CleanupState::Pending => "recovery_cleanup_pending",
            CleanupState::Quarantined => "recovery_cleanup_quarantined",
            CleanupState::Verified => unreachable!("verified cleanup was filtered"),
        };
        issues.push(RecoveryIssue::new(
            &run.id,
            &attempt.task_id,
            &attempt.id,
            dispatch.map(|dispatch| &dispatch.effect_id),
            reason,
            &[
                attempt
                    .worktree_id
                    .as_ref()
                    .map(pandamux_core::AttemptWorktreeId::as_str)
                    .unwrap_or(""),
                cleanup_state_name(attempt.cleanup_state),
            ],
        ));
    }
}

fn attempt_is_terminal(status: AttemptStatus) -> bool {
    matches!(
        status,
        AttemptStatus::Succeeded
            | AttemptStatus::Failed
            | AttemptStatus::Cancelled
            | AttemptStatus::Interrupted
    )
}

fn cleanup_state_name(state: CleanupState) -> &'static str {
    match state {
        CleanupState::NotStarted => "not_started",
        CleanupState::Pending => "pending",
        CleanupState::Verified => "verified",
        CleanupState::Quarantined => "quarantined",
    }
}

fn effect_attempt_id(effect: &pandamux_core::OrchestrationEffect) -> Option<RunAttemptId> {
    match &effect.kind {
        pandamux_core::OrchestrationEffectKind::DispatchTask { attempt_id, .. }
        | pandamux_core::OrchestrationEffectKind::CancelAttempt { attempt_id, .. } => {
            Some(attempt_id.clone())
        }
        _ => None,
    }
}

fn push_effect_issue(
    run: &pandamux_core::Run,
    effect_id: &OrchestrationEffectId,
    reason: &str,
    issues: &mut Vec<RecoveryIssue>,
) {
    let Some(effect) = run.pending_effects.get(effect_id) else {
        return;
    };
    let (task_id, attempt_id) = match &effect.kind {
        pandamux_core::OrchestrationEffectKind::DispatchTask {
            task_id,
            attempt_id,
            ..
        }
        | pandamux_core::OrchestrationEffectKind::CancelAttempt {
            task_id,
            attempt_id,
            ..
        } => (task_id, attempt_id),
        _ => return,
    };
    issues.push(RecoveryIssue::new(
        &run.id,
        task_id,
        attempt_id,
        Some(effect_id),
        reason,
        &[effect_id.as_str()],
    ));
}

fn lease_attempt<'a>(
    runtime: &'a OrchestrationRuntimeRecords,
    lease: &pandamux_core::ExecutionLease,
) -> Option<(&'a RunTaskId, &'a RunAttemptId)> {
    match lease.resource_kind {
        LeaseResourceKind::Effect => runtime
            .attempt_dispatches
            .iter()
            .find(|attempt| attempt.effect_id.as_str() == lease.resource_id)
            .map(|attempt| (&attempt.task_id, &attempt.attempt_id)),
        LeaseResourceKind::Capacity => runtime
            .queue_records
            .iter()
            .find(|queue| queue.queue_id == lease.resource_id)
            .map(|queue| (&queue.task_id, &queue.attempt_id)),
        LeaseResourceKind::Worktree => runtime
            .worktrees
            .iter()
            .find(|worktree| worktree.worktree.id.as_str() == lease.resource_id)
            .map(|worktree| (&worktree.worktree.task_id, &worktree.worktree.attempt_id)),
        LeaseResourceKind::RemoteAttempt => None,
    }
}

fn acknowledge_recorded_effect(
    store: &Store,
    run: &pandamux_core::Run,
    effect_id: &OrchestrationEffectId,
    result_digest: &str,
    at_ms: u64,
) -> Result<(), StoreError> {
    if run.settled_effects.contains_key(effect_id) {
        return Ok(());
    }
    let command = RunCommand::AcknowledgeEffect {
        effect_id: effect_id.clone(),
        receipt: EffectReceipt {
            effect_id: effect_id.clone(),
            acknowledged_at_ms: at_ms.max(run.updated_at_ms),
            result_digest: result_digest.to_string(),
        },
    };
    let kinds = pandamux_orchestrator::decide(run, command.clone())
        .map_err(|error| StoreError::Data(error.to_string()))?;
    let digest = stable_digest(&[run.id.as_str(), effect_id.as_str(), result_digest]);
    let assigned_events = kinds
        .iter()
        .enumerate()
        .map(|(index, _)| AssignedRunEvent {
            event_id: RunEventId::new(format!("run-event-recovery-ack-{digest}-{index}")),
            idempotency_key: format!("recovery-ack-{digest}-{index}"),
            at_ms: at_ms.max(run.updated_at_ms),
        })
        .collect();
    store.commit_run_transition(
        &RunTransitionRequest {
            run_id: run.id.clone(),
            expected_last_seq: run.last_seq,
            expected_snapshot_sha256: orchestration_snapshot_sha256(run),
            command,
            assigned_events,
            rows: RunTransitionRows::default(),
        },
        None,
    )?;
    Ok(())
}

fn commit_recovery_command(
    store: &Store,
    run_id: &RunId,
    command: RunCommand,
    at_ms: u64,
    operation: &str,
) -> Result<(), StoreError> {
    for _ in 0..8 {
        let record = store
            .load_verified_orchestration_run(run_id)?
            .ok_or_else(|| StoreError::Data("recovery run disappeared".to_string()))?;
        let kinds = pandamux_orchestrator::decide(&record.run, command.clone())
            .map_err(|error| StoreError::Data(error.to_string()))?;
        if kinds.is_empty() {
            return Ok(());
        }
        let command_bytes = serde_json::to_vec(&command)?;
        let command_digest = format!("{:x}", Sha256::digest(&command_bytes));
        let next_seq =
            record.run.last_seq.checked_add(1).ok_or_else(|| {
                StoreError::Data("recovery event sequence overflowed".to_string())
            })?;
        let assigned_events = kinds
            .iter()
            .enumerate()
            .map(|(index, _)| AssignedRunEvent {
                event_id: RunEventId::new(format!(
                    "run-event-recovery-{operation}-{next_seq}-{}-{index}",
                    &command_digest[..16]
                )),
                idempotency_key: format!(
                    "recovery-{operation}:{}:{}:{index}",
                    run_id.as_str(),
                    &command_digest[..16]
                ),
                at_ms: at_ms.max(record.run.updated_at_ms),
            })
            .collect();
        match store.commit_run_transition(
            &RunTransitionRequest {
                run_id: run_id.clone(),
                expected_last_seq: record.run.last_seq,
                expected_snapshot_sha256: orchestration_snapshot_sha256(&record.run),
                command: command.clone(),
                assigned_events,
                rows: RunTransitionRows::default(),
            },
            None,
        ) {
            Ok(_) => return Ok(()),
            Err(StoreError::Data(message))
                if message.contains("changed before the transition could commit") => {}
            Err(error) => return Err(error),
        }
    }
    Err(StoreError::Data(
        "recovery transition remained contended".to_string(),
    ))
}

fn record_recovery_terminal_progress(
    store: &Store,
    dispatch: &AttemptDispatchRecord,
    outcome_digest: &str,
    at_ms: u64,
) -> Result<(), StoreError> {
    let current = store
        .get_attempt_dispatch(&dispatch.attempt_id)?
        .ok_or_else(|| StoreError::Data("recovery progress owner disappeared".to_string()))?;
    store
        .record_attempt_progress(&AttemptProgressRecord {
            id: AttemptProgressId::new(format!(
                "attempt-progress-terminal-{}",
                &stable_digest(&[dispatch.attempt_id.as_str(), outcome_digest])[..32]
            )),
            run_id: dispatch.run_id.clone(),
            task_id: dispatch.task_id.clone(),
            attempt_id: dispatch.attempt_id.clone(),
            kind: AttemptProgressKind::TerminalOutcome,
            payload_digest: outcome_digest.to_string(),
            at_ms: at_ms.max(current.last_progress_at_ms),
            cumulative_stream_bytes: None,
            terminal: true,
        })
        .map(|_| ())
}

fn replay_failure_reason(error: &StoreError) -> &'static str {
    let message = error.to_string();
    if message.contains("sequence") || message.contains("event") {
        "recovery_event_gap"
    } else {
        "recovery_snapshot_mismatch"
    }
}

fn stable_digest(fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    format!("{:x}", digest.finalize())
}
