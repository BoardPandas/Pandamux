use pandamux_core::{
    AccessMode, AgentId, AgentTask, Attempt, AttemptBoundary, AttemptFailure, AttemptStatus,
    BudgetAmount, BudgetReservation, BudgetReservationId, BudgetReservationState, BudgetUsage,
    CheckpointId, CheckpointKind, CheckpointRef, CleanupRecord, CleanupRecordId, CleanupState,
    ConfigurationRevision, ExecutionLease, ExecutionLeaseId, FailureClass, LeaseResourceKind,
    OrchestrationMode, OutputContract, ProjectId, ProjectRepositoryId, ProjectRepositoryRecord,
    ProviderInstanceId, RoutingDecision, Run, RunAttemptId, RunCommand, RunEvent, RunEventId,
    RunId, RunSeed, RunTaskId, RunTaskStatus, RunTrigger, ServerIncarnationId, SubscriptionKey,
    TaskRouteDecision, ThreadId, TurnAttemptId, ValidatedPlan,
};
use pandamux_server::{
    AssignedRunEvent, DataPaths, EffectClaimRequest, EffectOwnership, RecoveryLeaseDisposition,
    RunExecutionService, RunRecoveryError, RunRecoveryService, RunTransitionRequest,
    RunTransitionRows, ServerIncarnationLock, Store, orchestration_snapshot_sha256,
};
use std::collections::{BTreeMap, BTreeSet};

const RUN_ID: &str = "run-recovery-cj";
const TASK_ID: &str = "task-recovery-cj";
const ATTEMPT_ID: &str = "attempt-recovery-cj";

struct RecoveryFixture {
    store: Store,
    ownership: EffectOwnership,
    old_owner: ServerIncarnationId,
}

impl RecoveryFixture {
    fn in_memory(last_heartbeat_ms: u64, expires_at_ms: u64) -> Self {
        Self::new(
            Store::in_memory().expect("in-memory recovery store"),
            last_heartbeat_ms,
            expires_at_ms,
        )
    }

    fn new(store: Store, last_heartbeat_ms: u64, expires_at_ms: u64) -> Self {
        store
            .save_project_repository(&ProjectRepositoryRecord {
                id: ProjectRepositoryId::new("repository-recovery-cj"),
                project_id: ProjectId::new("project-recovery-cj"),
                organization_id: pandamux_core::OrganizationId::new("org-recovery-cj"),
                canonical_remote_identity: "remote-recovery-cj".to_string(),
                canonical_repository_root_identity: "root-recovery-cj".to_string(),
                display_path: "PandaMUX".to_string(),
                revision: ConfigurationRevision::INITIAL,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .expect("recovery repository");
        let (run, events) = base_run();
        store
            .insert_orchestration_run(
                &pandamux_server::OrchestrationRunRecord {
                    run: run.clone(),
                    project_id: ProjectId::new("project-recovery-cj"),
                    repository_id: ProjectRepositoryId::new("repository-recovery-cj"),
                    settings_revision: 1,
                },
                &events,
            )
            .expect("insert recovery run");
        RunExecutionService::new(store.clone())
            .commit_transition(
                &RunTransitionRequest {
                    run_id: run.id.clone(),
                    expected_last_seq: run.last_seq,
                    expected_snapshot_sha256: orchestration_snapshot_sha256(&run),
                    command: RunCommand::RequestDispatch { attempt: attempt() },
                    assigned_events: vec![AssignedRunEvent {
                        event_id: RunEventId::new("event-recovery-dispatch"),
                        idempotency_key: "recovery-dispatch".to_string(),
                        at_ms: 200,
                    }],
                    rows: RunTransitionRows::default(),
                },
                None,
            )
            .expect("request recovery dispatch");
        let effect = store
            .load_orchestration_effects(&RunId::new(RUN_ID))
            .expect("load recovery effect")
            .into_iter()
            .find(|effect| {
                matches!(
                    effect.effect.kind,
                    pandamux_core::OrchestrationEffectKind::DispatchTask { .. }
                )
            })
            .expect("dispatch effect");
        let old_owner = ServerIncarnationId::new("server-recovery-old");
        let lease_id = ExecutionLeaseId::new("lease-recovery-effect");
        let revision = effect.delivery.claim_revision + 1;
        store
            .claim_orchestration_effect(
                &EffectClaimRequest {
                    effect_id: effect.effect.id.clone(),
                    expected_payload_sha256: effect.effect.payload_sha256.clone(),
                    owner_incarnation_id: old_owner.clone(),
                    lease: ExecutionLease {
                        id: lease_id.clone(),
                        resource_kind: LeaseResourceKind::Effect,
                        resource_id: effect.effect.id.as_str().to_string(),
                        owner_incarnation_id: old_owner.clone(),
                        expires_at_ms,
                        last_heartbeat_ms,
                        revision,
                    },
                    claimed_at_ms: last_heartbeat_ms,
                },
                None,
            )
            .expect("claim recovery effect");
        Self {
            store,
            ownership: EffectOwnership {
                effect_id: effect.effect.id,
                expected_payload_sha256: effect.effect.payload_sha256,
                owner_incarnation_id: old_owner.clone(),
                lease_id,
                claim_revision: revision,
            },
            old_owner,
        }
    }
}

#[tokio::test]
async fn rec_01_expired_exact_undispatched_effect_is_reclaimed_once() {
    let fixture = RecoveryFixture::in_memory(210, 260);
    let recovery = RunRecoveryService::new(fixture.store.clone());
    let report = recovery
        .recover_nonterminal_runs(ServerIncarnationId::new("server-recovery-new"), 300)
        .await
        .expect("recover expired effect");
    assert_eq!(report.leases_reclaimed, 1);
    assert!(report.intake_open);
    let effects = fixture
        .store
        .load_orchestration_effects(&RunId::new(RUN_ID))
        .expect("load recovered effects");
    assert_eq!(
        report.effects_ready,
        effects
            .iter()
            .filter(|effect| {
                effect.delivery.state == pandamux_core::EffectDeliveryState::Pending
            })
            .count()
    );
    let effect = effects
        .iter()
        .find(|effect| effect.effect.id == fixture.ownership.effect_id)
        .expect("recovered dispatch effect");
    assert_eq!(
        effect.delivery.state,
        pandamux_core::EffectDeliveryState::Pending
    );

    let replay = recovery
        .recover_nonterminal_runs(ServerIncarnationId::new("server-recovery-newer"), 301)
        .await
        .expect("repeat recovery");
    assert_eq!(replay.leases_reclaimed, 0);
    assert_eq!(replay.effects_ready, report.effects_ready);
}

#[tokio::test]
async fn rec_02_and_clk_02_unexpired_or_live_owner_is_never_reclaimed() {
    let unexpired = RecoveryFixture::in_memory(210, 400);
    let report = RunRecoveryService::new(unexpired.store.clone())
        .recover_nonterminal_runs(ServerIncarnationId::new("server-new"), 300)
        .await
        .expect("unexpired recovery");
    assert_eq!(report.leases_reclaimed, 0);

    let live = RecoveryFixture::in_memory(210, 260);
    let recovery = RunRecoveryService::new(live.store.clone());
    recovery.set_live_owners(BTreeSet::from([live.old_owner.clone()]));
    let report = recovery
        .recover_nonterminal_runs(ServerIncarnationId::new("server-new"), 10_000)
        .await
        .expect("live owner wins over forward clock jump");
    assert_eq!(report.leases_reclaimed, 0);
    assert!(report.actions.iter().any(|action| {
        action.disposition == pandamux_server::RecoveryDisposition::RetainedLiveOwner
    }));
}

#[test]
fn rec_03_and_04_foreign_or_changed_ownership_is_untouched() {
    let fixture = RecoveryFixture::in_memory(210, 260);
    let mut foreign = fixture.ownership.clone();
    foreign.owner_incarnation_id = ServerIncarnationId::new("foreign-owner");
    assert_eq!(
        fixture
            .store
            .reclaim_expired_effect_claim(&foreign, 300, &BTreeSet::new())
            .expect("foreign disposition"),
        RecoveryLeaseDisposition::ForeignOwner
    );
    let mut changed = fixture.ownership.clone();
    changed.claim_revision += 1;
    assert_eq!(
        fixture
            .store
            .reclaim_expired_effect_claim(&changed, 300, &BTreeSet::new())
            .expect("changed disposition"),
        RecoveryLeaseDisposition::ForeignOwner
    );
}

#[tokio::test]
async fn clk_01_and_rec_12_backward_clock_deduplicates_review() {
    let fixture = RecoveryFixture::in_memory(500, 560);
    let recovery = RunRecoveryService::new(fixture.store.clone());
    let first = recovery
        .recover_nonterminal_runs(ServerIncarnationId::new("server-new"), 400)
        .await
        .expect("clock anomaly classified");
    assert_eq!(first.reviews_created, 1);
    assert_eq!(first.reviews_deduplicated, 0);
    let second = recovery
        .recover_nonterminal_runs(ServerIncarnationId::new("server-newer"), 400)
        .await
        .expect("duplicate clock anomaly classified");
    assert_eq!(second.reviews_created, 0);
    assert_eq!(second.reviews_deduplicated, 1);
    assert_eq!(
        fixture.store.list_pending_recovery_reviews().unwrap().len(),
        1
    );
}

#[tokio::test]
async fn rec_13_review_resolution_is_audited_and_immutable() {
    let fixture = RecoveryFixture::in_memory(500, 560);
    RunRecoveryService::new(fixture.store.clone())
        .recover_nonterminal_runs(ServerIncarnationId::new("server-new"), 400)
        .await
        .expect("create recovery review");
    let review = fixture
        .store
        .list_pending_recovery_reviews()
        .unwrap()
        .pop()
        .expect("pending review");
    let resolved = fixture
        .store
        .resolve_recovery_review(&review.id, "retain_for_manual_review", 600)
        .expect("resolve review");
    assert_eq!(
        resolved.status,
        pandamux_core::RecoveryReviewStatus::Resolved
    );
    assert_eq!(
        fixture
            .store
            .resolve_recovery_review(&review.id, "retain_for_manual_review", 600)
            .expect("exact resolution replay"),
        resolved
    );
    assert!(
        fixture
            .store
            .resolve_recovery_review(&review.id, "retry", 601)
            .is_err()
    );
}

#[tokio::test]
async fn rec_terminal_cleanup_debt_is_reviewed_once_instead_of_verified() {
    let store = Store::in_memory().expect("in-memory cleanup recovery store");
    store
        .save_project_repository(&ProjectRepositoryRecord {
            id: ProjectRepositoryId::new("repository-recovery-cj"),
            project_id: ProjectId::new("project-recovery-cj"),
            organization_id: pandamux_core::OrganizationId::new("org-recovery-cj"),
            canonical_remote_identity: "remote-recovery-cj".to_string(),
            canonical_repository_root_identity: "root-recovery-cj".to_string(),
            display_path: "PandaMUX".to_string(),
            revision: ConfigurationRevision::INITIAL,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .expect("cleanup recovery repository");
    let (run, events) = cleanup_debt_run();
    store
        .insert_orchestration_run(
            &pandamux_server::OrchestrationRunRecord {
                run,
                project_id: ProjectId::new("project-recovery-cj"),
                repository_id: ProjectRepositoryId::new("repository-recovery-cj"),
                settings_revision: 1,
            },
            &events,
        )
        .expect("insert cleanup recovery run");
    let recovery = RunRecoveryService::new(store.clone());

    let first = recovery
        .recover_nonterminal_runs(ServerIncarnationId::new("server-cleanup-first"), 300)
        .await
        .expect("classify terminal cleanup debt");
    assert_eq!(first.reviews_created, 1);
    assert!(first.actions.iter().any(|action| {
        action
            .attempt_id
            .as_ref()
            .is_some_and(|id| id.as_str() == ATTEMPT_ID)
            && action.disposition == pandamux_server::RecoveryDisposition::ReviewRequired
            && action.reason_code.as_deref() == Some("recovery_cleanup_pending")
    }));
    assert!(!first.actions.iter().any(|action| {
        action.run_id.as_str() == RUN_ID
            && action.disposition == pandamux_server::RecoveryDisposition::Verified
    }));

    let second = recovery
        .recover_nonterminal_runs(ServerIncarnationId::new("server-cleanup-second"), 301)
        .await
        .expect("repeat terminal cleanup debt classification");
    assert_eq!(second.reviews_created, 0);
    assert_eq!(second.reviews_deduplicated, 1);
    assert!(!second.actions.iter().any(|action| {
        action.run_id.as_str() == RUN_ID
            && action.disposition == pandamux_server::RecoveryDisposition::Verified
    }));
    let reviews = store.list_pending_recovery_reviews().unwrap();
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].reason_code, "recovery_cleanup_pending");
}

#[tokio::test]
async fn rec_08_snapshot_mismatch_keeps_intake_closed() {
    let temp = tempfile::tempdir().expect("recovery tempdir");
    let database = temp.path().join("pandamux.db");
    let store = Store::open(&database).expect("open recovery store");
    let fixture = RecoveryFixture::new(store, 210, 400);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE orchestration_runs SET snapshot_sha256 = ?1 WHERE run_id = ?2",
            rusqlite::params!["0".repeat(64), RUN_ID],
        )
        .expect("corrupt snapshot digest");
    let recovery = RunRecoveryService::new(fixture.store);
    let error = recovery
        .recover_nonterminal_runs(ServerIncarnationId::new("server-new"), 300)
        .await
        .expect_err("snapshot mismatch fails startup closed");
    assert!(matches!(
        error,
        RunRecoveryError::Invalid("recovery_snapshot_mismatch")
    ));
    assert!(!recovery.accepting_work());
}

#[test]
fn rec_incarnation_lock_is_exclusive_and_recoverable_after_drop() {
    let temp = tempfile::tempdir().expect("incarnation tempdir");
    let paths = DataPaths::from_root(temp.path()).expect("incarnation paths");
    let first = ServerIncarnationLock::acquire(&paths).expect("first lock");
    let error = match ServerIncarnationLock::acquire(&paths) {
        Ok(_) => panic!("second lock was not refused"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    drop(first);
    ServerIncarnationLock::acquire(&paths).expect("lock recovers after owner drop");
}

fn base_run() -> (Run, Vec<RunEvent>) {
    let seed = seed();
    let mut run = Run::draft(RunId::new(RUN_ID), seed.clone(), 100);
    let mut events = Vec::new();
    apply(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan { seed, plan: plan() },
        100,
    );
    apply(
        &mut run,
        &mut events,
        RunCommand::SelectRoute { decision: route() },
        120,
    );
    apply(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation(),
        },
        130,
    );
    (run, events)
}

fn cleanup_debt_run() -> (Run, Vec<RunEvent>) {
    let (mut run, mut events) = base_run();
    apply(
        &mut run,
        &mut events,
        RunCommand::RequestDispatch { attempt: attempt() },
        200,
    );
    apply(
        &mut run,
        &mut events,
        RunCommand::StartAttempt {
            attempt_id: RunAttemptId::new(ATTEMPT_ID),
            thread_id: ThreadId::new("thread-recovery-cleanup"),
            turn_attempt_id: TurnAttemptId::new("turn-attempt-recovery-cleanup"),
        },
        210,
    );
    apply(&mut run, &mut events, RunCommand::Cancel, 220);
    apply(
        &mut run,
        &mut events,
        RunCommand::RecordAttemptCleanup {
            cleanup: CleanupRecord {
                id: CleanupRecordId::new("cleanup-recovery-pending"),
                run_id: RunId::new(RUN_ID),
                task_id: RunTaskId::new(TASK_ID),
                attempt_id: RunAttemptId::new(ATTEMPT_ID),
                state: CleanupState::Pending,
                reason_code: "attempt_cleanup_pending".to_string(),
                evidence_digest: "c".repeat(64),
                updated_at_ms: 230,
            },
        },
        230,
    );
    apply(
        &mut run,
        &mut events,
        RunCommand::RecordCheckpoint {
            checkpoint: CheckpointRef {
                id: CheckpointId::new("checkpoint-recovery-cancelled"),
                run_id: RunId::new(RUN_ID),
                value: "retained cancellation output".to_string(),
                task_id: RunTaskId::new(TASK_ID),
                attempt_id: RunAttemptId::new(ATTEMPT_ID),
                thread_id: Some(ThreadId::new("thread-recovery-cleanup")),
                kind: CheckpointKind::AfterAttempt,
                created_at_ms: 240,
            },
        },
        240,
    );
    apply(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("reservation-recovery-cj"),
            actual: BudgetUsage {
                tokens: Some(10),
                cost_micro_usd: Some(10),
                runtime_ms: Some(10),
            },
        },
        250,
    );
    apply(
        &mut run,
        &mut events,
        RunCommand::SettleAttempt {
            attempt_id: RunAttemptId::new(ATTEMPT_ID),
            status: AttemptStatus::Cancelled,
            failure: Some(AttemptFailure {
                class: FailureClass::Cancelled,
                code: "attempt_cancelled".to_string(),
                safe_message: "The provider attempt was cancelled".to_string(),
            }),
            usage_reconciled: true,
        },
        260,
    );
    (run, events)
}

fn seed() -> RunSeed {
    RunSeed {
        trigger: RunTrigger::Manual {
            client_id: "recovery-cj".to_string(),
        },
        policy_revision: "policy-recovery-cj".to_string(),
        bundle_sha: "a".repeat(64),
        organization_id: pandamux_core::OrganizationId::new("org-recovery-cj"),
        mode: OrchestrationMode::Auto,
        budget_limit: BudgetAmount {
            tokens: 1_000,
            cost_micro_usd: 1_000,
            runtime_ms: 1_000,
        },
        max_parallel_tasks: 1,
    }
}

fn plan() -> ValidatedPlan {
    let task = AgentTask {
        id: RunTaskId::new(TASK_ID),
        summary: "Recover one attempt".to_string(),
        class: "build".to_string(),
        project_id: ProjectId::new("project-recovery-cj"),
        organization_id: pandamux_core::OrganizationId::new("org-recovery-cj"),
        required_capabilities: BTreeSet::new(),
        access_ceiling: AccessMode::WorkspaceOnly,
        inputs: Vec::new(),
        outputs: vec![OutputContract {
            kind: "change_set".to_string(),
            schema: None,
        }],
        required_verification: BTreeSet::new(),
        selected_agent_id: None,
        route: None,
        status: RunTaskStatus::Ready,
        max_attempts: 1,
        approval_ids: BTreeSet::new(),
        budget_limit: BudgetAmount {
            tokens: 100,
            cost_micro_usd: 100,
            runtime_ms: 100,
        },
        budget_reservation_id: None,
    };
    ValidatedPlan {
        schema_version: 1,
        source_digest: "b".repeat(64),
        tasks: BTreeMap::from([(task.id.clone(), task)]),
        edges: BTreeMap::new(),
        topological_order: vec![RunTaskId::new(TASK_ID)],
        total_budget: BudgetAmount {
            tokens: 100,
            cost_micro_usd: 100,
            runtime_ms: 100,
        },
    }
}

fn route() -> TaskRouteDecision {
    TaskRouteDecision {
        task_id: RunTaskId::new(TASK_ID),
        selected_agent_id: Some(AgentId::new("agent-recovery-cj")),
        agent_evaluations: Vec::new(),
        provider: RoutingDecision {
            selected_provider_instance_id: Some(ProviderInstanceId::new("provider-recovery-cj")),
            selected_subscription_key: Some(SubscriptionKey::new("subscription-recovery-cj")),
            selected_model: Some("model-recovery-cj".to_string()),
            evaluations: Vec::new(),
        },
    }
}

fn reservation() -> BudgetReservation {
    BudgetReservation {
        id: BudgetReservationId::new("reservation-recovery-cj"),
        task_id: RunTaskId::new(TASK_ID),
        attempt_id: RunAttemptId::new(ATTEMPT_ID),
        amount: BudgetAmount {
            tokens: 100,
            cost_micro_usd: 100,
            runtime_ms: 100,
        },
        actual: BudgetUsage::default(),
        ledger_utc_date: "2026-10-09".to_string(),
        state: BudgetReservationState::Reserved,
    }
}

fn attempt() -> Attempt {
    Attempt {
        id: RunAttemptId::new(ATTEMPT_ID),
        task_id: RunTaskId::new(TASK_ID),
        ordinal: 1,
        route: route(),
        thread_id: None,
        turn_attempt_id: None,
        status: AttemptStatus::DispatchRequested,
        boundary: AttemptBoundary::SafeToRetry,
        started_at_ms: 200,
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

fn apply(run: &mut Run, events: &mut Vec<RunEvent>, command: RunCommand, at_ms: u64) {
    let kinds = pandamux_orchestrator::decide(run, command).expect("recovery fixture command");
    for kind in kinds {
        let event = RunEvent {
            event_id: RunEventId::new(format!("event-recovery-{}", run.last_seq + 1)),
            run_id: run.id.clone(),
            seq: run.last_seq + 1,
            at_ms,
            idempotency_key: format!("recovery-fixture-{}", run.last_seq + 1),
            kind,
        };
        *run = pandamux_orchestrator::apply(run, &event)
            .expect("recovery fixture event")
            .run;
        events.push(event);
    }
}
