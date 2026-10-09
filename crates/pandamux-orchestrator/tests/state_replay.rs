use pandamux_core::{
    AccessMode, AgentEvaluation, AgentSelectionScore, AgentTask, ApprovalDecisionProvenance,
    ApprovalId, ApprovalRequirement, Attempt, AttemptBoundary, AttemptFailure, AttemptStatus,
    AttemptWorktreeId, BlackboardArtifact, BlackboardEntryId, BudgetAmount, BudgetDimension,
    BudgetGrantId, BudgetReservation, BudgetReservationId, BudgetReservationState, BudgetUsage,
    CheckpointId, CheckpointKind, CheckpointRef, CleanupRecord, CleanupRecordId, CleanupState,
    DependencyEdge, DependencyEdgeId, DependencyRequirement, EffectReceipt, FailureClass,
    OrchestrationMode, OrganizationId, OutputContract, ProjectId, ProviderInstanceId,
    RoutingDecision, Run, RunApprovalKind, RunApprovalStatus, RunAttemptId, RunCommand, RunEvent,
    RunEventId, RunEventKind, RunId, RunSeed, RunStatus, RunTaskId, RunTaskStatus, RunTrigger,
    SubscriptionKey, SupplementalBudgetGrant, TaskInput, TaskRouteDecision, ThreadId,
    TurnAttemptId, ValidatedPlan, VerificationEvidence, VerificationEvidenceId,
    VerificationEvidenceKind, VerificationEvidenceOutcome,
};
use pandamux_orchestrator::{
    OrchestrationErrorCode, apply, artifact_key, canonical_artifact_json,
    canonical_blackboard_sha256, canonical_run_bytes, decide, output_contract_schema_sha256,
    replay, replay_from,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn contract() -> OutputContract {
    OutputContract {
        kind: "patch".to_string(),
        schema: None,
    }
}

fn task(id: &str, status: RunTaskStatus) -> AgentTask {
    AgentTask {
        id: RunTaskId::new(id),
        summary: format!("Task {id}"),
        class: "build".to_string(),
        project_id: ProjectId::new("project-one"),
        organization_id: OrganizationId::new("org-one"),
        required_capabilities: BTreeSet::new(),
        access_ceiling: AccessMode::WorkspaceOnly,
        inputs: Vec::new(),
        outputs: vec![contract()],
        required_verification: BTreeSet::new(),
        selected_agent_id: None,
        route: None,
        status,
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

fn seed(mode: OrchestrationMode) -> RunSeed {
    RunSeed {
        trigger: RunTrigger::Manual {
            client_id: "client-one".to_string(),
        },
        policy_revision: "policy-one".to_string(),
        bundle_sha: "bundle-one".to_string(),
        organization_id: OrganizationId::new("org-one"),
        mode,
        budget_limit: BudgetAmount {
            tokens: 1_000,
            cost_micro_usd: 2_000,
            runtime_ms: 3_000,
        },
        max_parallel_tasks: 2,
    }
}

fn one_task_plan() -> ValidatedPlan {
    let task = task("task-one", RunTaskStatus::Ready);
    ValidatedPlan {
        schema_version: 1,
        source_digest: "plan-one".to_string(),
        tasks: BTreeMap::from([(task.id.clone(), task)]),
        edges: BTreeMap::new(),
        topological_order: vec![RunTaskId::new("task-one")],
        total_budget: BudgetAmount {
            tokens: 100,
            cost_micro_usd: 200,
            runtime_ms: 300,
        },
    }
}

fn route(task_id: &str, provider: &str) -> TaskRouteDecision {
    TaskRouteDecision {
        task_id: RunTaskId::new(task_id),
        selected_agent_id: Some(pandamux_core::AgentId::new("builder")),
        agent_evaluations: vec![AgentEvaluation {
            agent_id: pandamux_core::AgentId::new("builder"),
            eligible: true,
            rejection_codes: Vec::new(),
            explanation_codes: vec!["class_rule:build".to_string()],
            score: Some(AgentSelectionScore {
                class_rule: 100,
                specialty: 0,
                provider_preference: 0,
                total: 100,
            }),
        }],
        provider: RoutingDecision {
            selected_provider_instance_id: Some(ProviderInstanceId::new(provider)),
            selected_subscription_key: Some(SubscriptionKey::new("subscription-one")),
            selected_model: Some("model-one".to_string()),
            evaluations: Vec::new(),
        },
    }
}

fn reservation(task_id: &str, ordinal: u32) -> BudgetReservation {
    BudgetReservation {
        id: BudgetReservationId::new(format!("budget-{task_id}-{ordinal}")),
        task_id: RunTaskId::new(task_id),
        attempt_id: RunAttemptId::new(format!("attempt-{task_id}-{ordinal}")),
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

fn attempt(task_id: &str, ordinal: u32, provider: &str) -> Attempt {
    Attempt {
        id: RunAttemptId::new(format!("attempt-{task_id}-{ordinal}")),
        task_id: RunTaskId::new(task_id),
        ordinal,
        route: route(task_id, provider),
        thread_id: None,
        turn_attempt_id: None,
        status: AttemptStatus::DispatchRequested,
        boundary: AttemptBoundary::SafeToRetry,
        started_at_ms: 2_000 + u64::from(ordinal),
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

fn artifact(run_id: &RunId, task_id: &str, content: serde_json::Value) -> BlackboardArtifact {
    let output_contract = contract();
    let canonical_json = canonical_artifact_json(&content);
    BlackboardArtifact {
        id: BlackboardEntryId::new(format!("blackboard-{task_id}")),
        run_id: run_id.clone(),
        artifact_key: artifact_key(&RunTaskId::new(task_id), &output_contract),
        version: 1,
        previous_entry_id: None,
        producer: RunTaskId::new(task_id),
        consumers: BTreeSet::new(),
        schema_sha256: output_contract_schema_sha256(&output_contract),
        output_contract,
        byte_len: u64::try_from(canonical_json.len()).expect("artifact bytes fit"),
        content_sha256: canonical_blackboard_sha256(&content),
        canonical_json,
        provenance: None,
    }
}

fn next_event(run: &Run, index: usize, kind: RunEventKind) -> RunEvent {
    let seq = run.last_seq + 1;
    RunEvent {
        event_id: RunEventId::new(format!("event-{index}-{seq}")),
        run_id: run.id.clone(),
        seq,
        at_ms: 1_000 + seq,
        idempotency_key: format!("idempotency-{index}-{seq}"),
        kind,
    }
}

fn execute_kind(
    run: &mut Run,
    events: &mut Vec<RunEvent>,
    kind: RunEventKind,
) -> pandamux_orchestrator::ApplyOutcome {
    let event = next_event(run, events.len() + 1, kind);
    let outcome = apply(run, &event).expect("event applies");
    *run = outcome.run.clone();
    events.push(event);
    outcome
}

fn execute_command(
    run: &mut Run,
    events: &mut Vec<RunEvent>,
    command: RunCommand,
) -> pandamux_orchestrator::ApplyOutcome {
    let kinds = decide(run, command).expect("command is legal");
    assert_eq!(kinds.len(), 1);
    execute_kind(run, events, kinds.into_iter().next().expect("one event"))
}

fn acknowledge_all(run: &mut Run, events: &mut Vec<RunEvent>) {
    let pending = run.pending_effects.keys().cloned().collect::<Vec<_>>();
    for effect_id in pending {
        let receipt = EffectReceipt {
            effect_id: effect_id.clone(),
            acknowledged_at_ms: 5_000 + run.last_seq,
            result_digest: "result-ok".to_string(),
        };
        execute_command(
            run,
            events,
            RunCommand::AcknowledgeEffect { effect_id, receipt },
        );
    }
}

struct Lifecycle {
    initial: Run,
    events: Vec<RunEvent>,
    final_run: Run,
}

fn successful_lifecycle() -> Lifecycle {
    let seed = seed(OrchestrationMode::Auto);
    let initial = Run::draft(RunId::new("run-one"), seed.clone(), 1_001);
    let mut run = initial.clone();
    let mut events = Vec::new();
    execute_command(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan {
            seed,
            plan: one_task_plan(),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SelectRoute {
            decision: route("task-one", "provider-one"),
        },
    );
    assert_eq!(
        run.routes[&RunTaskId::new("task-one")],
        route("task-one", "provider-one")
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-one", 1),
        },
    );
    let dispatch = execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestDispatch {
            attempt: attempt("task-one", 1, "provider-one"),
        },
    );
    assert_eq!(dispatch.new_effects.len(), 1);
    assert!(
        run.replay_receipts
            .contains_key(&events.last().expect("dispatch event").event_id)
    );
    assert_eq!(
        run.pending_effects
            .values()
            .next()
            .expect("persisted dispatch effect")
            .source_event_id,
        events.last().expect("dispatch event").event_id
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::AssignAttemptWorktree {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            worktree_id: AttemptWorktreeId::new("attempt-worktree-one"),
        },
    );
    assert!(
        decide(
            &run,
            RunCommand::AssignAttemptWorktree {
                attempt_id: RunAttemptId::new("attempt-task-one-1"),
                worktree_id: AttemptWorktreeId::new("attempt-worktree-one"),
            },
        )
        .expect("exact worktree replay")
        .is_empty()
    );
    assert_eq!(
        decide(
            &run,
            RunCommand::AssignAttemptWorktree {
                attempt_id: RunAttemptId::new("attempt-task-one-1"),
                worktree_id: AttemptWorktreeId::new("attempt-worktree-other"),
            },
        )
        .expect_err("worktree assignment is immutable")
        .code,
        OrchestrationErrorCode::ReplayConflict
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::StartAttempt {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            thread_id: ThreadId::new("thread-one"),
            turn_attempt_id: TurnAttemptId::new("turn-attempt-one"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleAttempt {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            status: AttemptStatus::Succeeded,
            failure: None,
            usage_reconciled: false,
        },
    );
    let after_checkpoint = CheckpointRef {
        id: CheckpointId::new("checkpoint-after-one"),
        run_id: run.id.clone(),
        value: "commit-after".to_string(),
        task_id: RunTaskId::new("task-one"),
        attempt_id: RunAttemptId::new("attempt-task-one-1"),
        thread_id: Some(ThreadId::new("thread-one")),
        kind: CheckpointKind::AfterAttempt,
        created_at_ms: 1_000 + run.last_seq + 1,
    };
    let mut missing_thread = after_checkpoint.clone();
    missing_thread.id = CheckpointId::new("checkpoint-after-missing-thread");
    missing_thread.thread_id = None;
    assert_eq!(
        decide(
            &run,
            RunCommand::RecordCheckpoint {
                checkpoint: missing_thread,
            },
        )
        .expect_err("after-attempt checkpoints require the exact thread")
        .code,
        OrchestrationErrorCode::IllegalTransition
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RecordCheckpoint {
            checkpoint: after_checkpoint,
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
            actual: BudgetUsage {
                tokens: Some(90),
                cost_micro_usd: Some(180),
                runtime_ms: Some(270),
            },
        },
    );
    let completed_artifact = artifact(
        &run.id,
        "task-one",
        json!({
            "accessMode": "full_access",
            "organizationId": "org-other",
            "tools": ["shell"]
        }),
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::CompleteTask {
            task_id: RunTaskId::new("task-one"),
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            artifacts: vec![completed_artifact],
            evidence_ids: BTreeSet::new(),
            summary: Some("done".to_string()),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleRun {
            status: RunStatus::Succeeded,
            outcome: "all tasks succeeded".to_string(),
        },
    );
    Lifecycle {
        initial,
        events,
        final_run: run,
    }
}

fn successful_attempt_before_reconciliation() -> (Run, Vec<RunEvent>) {
    let lifecycle = successful_lifecycle();
    let reconciliation_index = lifecycle
        .events
        .iter()
        .position(|event| matches!(event.kind, RunEventKind::BudgetReconciled { .. }))
        .expect("budget reconciliation event");
    let events = lifecycle.events[..reconciliation_index].to_vec();
    let run = replay(events.iter()).expect("successful attempt before reconciliation");
    (run, events)
}

#[test]
fn full_replay_snapshot_suffix_and_every_prefix_are_byte_identical() {
    let lifecycle = successful_lifecycle();
    let full = replay(lifecycle.events.iter()).expect("full replay");
    assert_eq!(
        canonical_run_bytes(&full),
        canonical_run_bytes(&lifecycle.final_run)
    );

    for prefix in 0..=lifecycle.events.len() {
        let snapshot = if prefix == 0 {
            lifecycle.initial.clone()
        } else {
            replay(lifecycle.events[..prefix].iter()).expect("prefix replay")
        };
        let rebuilt = replay_from(&snapshot, lifecycle.events[prefix..].iter())
            .expect("snapshot plus suffix replay");
        assert_eq!(
            canonical_run_bytes(&rebuilt),
            canonical_run_bytes(&full),
            "prefix {prefix}"
        );
        assert_eq!(
            rebuilt.pending_effects, full.pending_effects,
            "prefix {prefix}"
        );
        assert_eq!(
            rebuilt.settled_effects, full.settled_effects,
            "prefix {prefix}"
        );
    }
}

#[test]
fn adjacent_and_nonadjacent_identical_events_are_noops_without_duplicate_effects() {
    let lifecycle = successful_lifecycle();
    let expected = replay(lifecycle.events.iter()).expect("baseline replay");
    let mut duplicated = lifecycle.events.clone();
    duplicated.insert(1, duplicated[0].clone());
    let later = duplicated
        .iter()
        .find(|event| matches!(event.kind, RunEventKind::DispatchRequested { .. }))
        .expect("dispatch")
        .clone();
    duplicated.push(later);
    let actual = replay(duplicated.iter()).expect("duplicates replay");
    assert_eq!(canonical_run_bytes(&actual), canonical_run_bytes(&expected));
    assert_eq!(actual.pending_effects, expected.pending_effects);
    assert_eq!(actual.settled_effects, expected.settled_effects);
}

#[test]
fn sequence_identity_and_idempotency_conflicts_fail_hard() {
    let lifecycle = successful_lifecycle();
    let first = &lifecycle.events[0];
    let state = apply(&lifecycle.initial, first).expect("first event").run;

    let mut changed = first.clone();
    let RunEventKind::PlanAccepted { plan, .. } = &mut changed.kind else {
        panic!("plan event")
    };
    plan.source_digest = "changed".to_string();
    assert_eq!(
        apply(&state, &changed)
            .expect_err("identity collision")
            .code,
        OrchestrationErrorCode::ReplayConflict
    );

    let mut idempotency = lifecycle.events[1].clone();
    idempotency.event_id = RunEventId::new("event-other");
    idempotency.idempotency_key = first.idempotency_key.clone();
    assert_eq!(
        apply(&state, &idempotency)
            .expect_err("idempotency collision")
            .code,
        OrchestrationErrorCode::IdempotencyConflict
    );

    let mut gap = lifecycle.events[1].clone();
    gap.seq = state.last_seq + 2;
    assert_eq!(
        apply(&state, &gap).expect_err("gap").code,
        OrchestrationErrorCode::EventGap
    );
}

#[test]
fn terminal_state_absorbs_new_work_but_accepts_identical_replay() {
    let lifecycle = successful_lifecycle();
    assert!(lifecycle.final_run.is_terminal());
    assert!(
        apply(
            &lifecycle.final_run,
            lifecycle.events.last().expect("terminal event")
        )
        .expect("identical replay")
        .duplicate
    );
    let new_event = next_event(
        &lifecycle.final_run,
        999,
        RunEventKind::Paused {
            reason: "late".to_string(),
        },
    );
    assert_eq!(
        apply(&lifecycle.final_run, &new_event)
            .expect_err("terminal absorbs")
            .code,
        OrchestrationErrorCode::TerminalRun
    );
}

fn graph_plan() -> ValidatedPlan {
    let mut task_a = task("task-a", RunTaskStatus::Ready);
    task_a.max_attempts = 2;
    let task_b = task("task-b", RunTaskStatus::Blocked);
    let task_c = task("task-c", RunTaskStatus::Blocked);
    let task_d = task("task-d", RunTaskStatus::Ready);
    let edges = [
        DependencyEdge {
            id: DependencyEdgeId::new("edge-a-b"),
            source: task_a.id.clone(),
            target: task_b.id.clone(),
            requirement: DependencyRequirement::RequiresSuccess,
        },
        DependencyEdge {
            id: DependencyEdgeId::new("edge-a-c"),
            source: task_a.id.clone(),
            target: task_c.id.clone(),
            requirement: DependencyRequirement::RequiresSettlement,
        },
    ];
    ValidatedPlan {
        schema_version: 1,
        source_digest: "graph-plan".to_string(),
        tasks: [task_a, task_b, task_c, task_d]
            .into_iter()
            .map(|task| (task.id.clone(), task))
            .collect(),
        edges: edges
            .into_iter()
            .map(|edge| (edge.id.clone(), edge))
            .collect(),
        topological_order: vec![
            RunTaskId::new("task-a"),
            RunTaskId::new("task-b"),
            RunTaskId::new("task-c"),
            RunTaskId::new("task-d"),
        ],
        total_budget: BudgetAmount {
            tokens: 400,
            cost_micro_usd: 800,
            runtime_ms: 1_200,
        },
    }
}

fn run_to_failed_attempt(boundary: AttemptBoundary) -> (Run, Vec<RunEvent>) {
    let seed = seed(OrchestrationMode::Auto);
    let mut run = Run::draft(RunId::new("run-graph"), seed.clone(), 1_001);
    let mut events = Vec::new();
    execute_command(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan {
            seed,
            plan: graph_plan(),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SelectRoute {
            decision: route("task-a", "provider-one"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-a", 1),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestDispatch {
            attempt: attempt("task-a", 1, "provider-one"),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::StartAttempt {
            attempt_id: RunAttemptId::new("attempt-task-a-1"),
            thread_id: ThreadId::new("thread-a"),
            turn_attempt_id: TurnAttemptId::new("turn-a"),
        },
    );
    if boundary != AttemptBoundary::SafeToRetry {
        execute_command(
            &mut run,
            &mut events,
            RunCommand::AdvanceAttemptBoundary {
                attempt_id: RunAttemptId::new("attempt-task-a-1"),
                boundary,
            },
        );
    }
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleAttempt {
            attempt_id: RunAttemptId::new("attempt-task-a-1"),
            status: AttemptStatus::Failed,
            failure: Some(AttemptFailure {
                class: FailureClass::Provider,
                code: "provider_failed".to_string(),
                safe_message: "Provider failed".to_string(),
            }),
            usage_reconciled: false,
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-a-1"),
            actual: BudgetUsage {
                tokens: Some(0),
                cost_micro_usd: Some(0),
                runtime_ms: Some(0),
            },
        },
    );
    (run, events)
}

#[test]
fn retry_requires_safe_pre_output_boundary_and_remaining_attempts() {
    let (mut safe, mut events) = run_to_failed_attempt(AttemptBoundary::SafeToRetry);
    assert_eq!(
        safe.tasks[&RunTaskId::new("task-a")].status,
        RunTaskStatus::Ready
    );
    execute_command(
        &mut safe,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-a", 2),
        },
    );
    execute_command(
        &mut safe,
        &mut events,
        RunCommand::RequestDispatch {
            attempt: attempt("task-a", 2, "provider-fallback"),
        },
    );

    let (unsafe_run, _) = run_to_failed_attempt(AttemptBoundary::VisibleOutputBegun);
    assert_eq!(
        unsafe_run.tasks[&RunTaskId::new("task-a")].status,
        RunTaskStatus::Blocked
    );
    assert_eq!(
        decide(
            &unsafe_run,
            RunCommand::RequestDispatch {
                attempt: attempt("task-a", 2, "provider-fallback"),
            },
        )
        .expect_err("unsafe retry refused")
        .code,
        OrchestrationErrorCode::IllegalTransition
    );
}

#[test]
fn failed_task_skips_success_descendants_and_continues_independent_and_settlement_branches() {
    let (mut run, mut events) = run_to_failed_attempt(AttemptBoundary::VisibleOutputBegun);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleTask {
            task_id: RunTaskId::new("task-a"),
            status: RunTaskStatus::Failed,
            summary: Some("failed".to_string()),
        },
    );
    assert_eq!(
        run.tasks[&RunTaskId::new("task-b")].status,
        RunTaskStatus::Skipped
    );
    assert_eq!(
        run.tasks[&RunTaskId::new("task-c")].status,
        RunTaskStatus::Ready
    );
    assert_eq!(
        run.tasks[&RunTaskId::new("task-d")].status,
        RunTaskStatus::Ready
    );
}

#[test]
fn cancellation_emits_one_effect_per_active_attempt_and_cancels_undispatched_tasks() {
    let (mut run, mut events) = run_to_failed_attempt(AttemptBoundary::SafeToRetry);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-a", 2),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestDispatch {
            attempt: attempt("task-a", 2, "provider-two"),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::StartAttempt {
            attempt_id: RunAttemptId::new("attempt-task-a-2"),
            thread_id: ThreadId::new("thread-a-2"),
            turn_attempt_id: TurnAttemptId::new("turn-a-2"),
        },
    );
    let outcome = execute_command(&mut run, &mut events, RunCommand::Cancel);
    assert_eq!(run.status, RunStatus::Cancelling);
    assert_eq!(
        outcome
            .new_effects
            .iter()
            .filter(|effect| matches!(
                effect.kind,
                pandamux_core::OrchestrationEffectKind::CancelAttempt { .. }
            ))
            .count(),
        1
    );
    for id in ["task-b", "task-c", "task-d"] {
        assert_eq!(
            run.tasks[&RunTaskId::new(id)].status,
            RunTaskStatus::Cancelled
        );
    }
}

fn run_to_running_cancellation_attempt() -> (Run, Vec<RunEvent>) {
    let seed = seed(OrchestrationMode::Auto);
    let mut run = Run::draft(RunId::new("run-cancellation-order"), seed.clone(), 1_001);
    let mut events = Vec::new();
    execute_command(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan {
            seed,
            plan: one_task_plan(),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SelectRoute {
            decision: route("task-one", "provider-one"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-one", 1),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestDispatch {
            attempt: attempt("task-one", 1, "provider-one"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::AssignAttemptWorktree {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            worktree_id: AttemptWorktreeId::new("worktree-cancellation-order"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::StartAttempt {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            thread_id: ThreadId::new("thread-cancellation-order"),
            turn_attempt_id: TurnAttemptId::new("turn-cancellation-order"),
        },
    );
    (run, events)
}

fn cancellation_checkpoint(run: &Run, id: &str, thread_id: &str) -> CheckpointRef {
    CheckpointRef {
        id: CheckpointId::new(id),
        run_id: run.id.clone(),
        value: "retained-cancellation-output".to_string(),
        task_id: RunTaskId::new("task-one"),
        attempt_id: RunAttemptId::new("attempt-task-one-1"),
        thread_id: Some(ThreadId::new(thread_id)),
        kind: CheckpointKind::AfterAttempt,
        created_at_ms: 1_000 + run.last_seq + 1,
    }
}

#[test]
fn cancellation_checkpoints_and_reconciles_before_terminal_attempt_state() {
    let (mut run, mut events) = run_to_running_cancellation_attempt();
    let pre_cancel = decide(
        &run,
        RunCommand::RecordCheckpoint {
            checkpoint: cancellation_checkpoint(
                &run,
                "checkpoint-cancellation-too-early",
                "thread-cancellation-order",
            ),
        },
    );
    assert_eq!(
        pre_cancel
            .expect_err("active non-cancelling attempt rejects after checkpoint")
            .code,
        OrchestrationErrorCode::IllegalTransition
    );

    execute_command(&mut run, &mut events, RunCommand::Cancel);
    let premature_reconciliation = decide(
        &run,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
            actual: BudgetUsage {
                tokens: Some(10),
                cost_micro_usd: Some(20),
                runtime_ms: Some(30),
            },
        },
    );
    assert_eq!(
        premature_reconciliation
            .expect_err("cancellation usage requires its retained checkpoint")
            .code,
        OrchestrationErrorCode::IllegalTransition
    );
    let wrong_thread = decide(
        &run,
        RunCommand::RecordCheckpoint {
            checkpoint: cancellation_checkpoint(
                &run,
                "checkpoint-cancellation-wrong-thread",
                "thread-other",
            ),
        },
    );
    assert_eq!(
        wrong_thread
            .expect_err("cancellation checkpoint requires the exact thread")
            .code,
        OrchestrationErrorCode::InvalidEvent
    );

    let checkpoint = cancellation_checkpoint(
        &run,
        "checkpoint-cancellation-after",
        "thread-cancellation-order",
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RecordCheckpoint { checkpoint },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
            actual: BudgetUsage {
                tokens: Some(10),
                cost_micro_usd: Some(20),
                runtime_ms: Some(30),
            },
        },
    );
    assert_eq!(
        run.attempts[&RunAttemptId::new("attempt-task-one-1")].status,
        AttemptStatus::Running
    );
    assert!(run.attempts[&RunAttemptId::new("attempt-task-one-1")].usage_reconciled);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleAttempt {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            status: AttemptStatus::Cancelled,
            failure: Some(AttemptFailure {
                class: FailureClass::Cancelled,
                code: "attempt_cancelled".to_string(),
                safe_message: "The attempt was cancelled".to_string(),
            }),
            usage_reconciled: true,
        },
    );
    assert_eq!(
        run.tasks[&RunTaskId::new("task-one")].status,
        RunTaskStatus::Cancelled
    );
    assert_eq!(
        canonical_run_bytes(&replay(events.iter()).expect("replay cancellation ordering")),
        canonical_run_bytes(&run)
    );
}

fn cleanup_record(run: &Run, id: &str, state: CleanupState) -> CleanupRecord {
    CleanupRecord {
        id: CleanupRecordId::new(id),
        run_id: run.id.clone(),
        task_id: RunTaskId::new("task-one"),
        attempt_id: RunAttemptId::new("attempt-task-one-1"),
        state,
        reason_code: format!("cleanup_{state:?}").to_ascii_lowercase(),
        evidence_digest: "a".repeat(64),
        updated_at_ms: 1_000 + run.last_seq + 1,
    }
}

#[test]
fn cleanup_transitions_are_terminal_bound_and_replay_safe() {
    let (mut run, mut events) = run_to_running_cancellation_attempt();
    let direct_verified = decide(
        &run,
        RunCommand::RecordAttemptCleanup {
            cleanup: cleanup_record(&run, "cleanup-direct-verified", CleanupState::Verified),
        },
    );
    assert_eq!(
        direct_verified
            .expect_err("cleanup cannot skip pending")
            .code,
        OrchestrationErrorCode::IllegalTransition
    );
    let pending_cleanup = cleanup_record(&run, "cleanup-pending", CleanupState::Pending);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RecordAttemptCleanup {
            cleanup: pending_cleanup,
        },
    );
    execute_command(&mut run, &mut events, RunCommand::Cancel);
    let active_verified = decide(
        &run,
        RunCommand::RecordAttemptCleanup {
            cleanup: cleanup_record(&run, "cleanup-active-verified", CleanupState::Verified),
        },
    );
    assert_eq!(
        active_verified
            .expect_err("verified cleanup requires terminal attempt")
            .code,
        OrchestrationErrorCode::IllegalTransition
    );
    let checkpoint = cancellation_checkpoint(
        &run,
        "checkpoint-cleanup-after",
        "thread-cancellation-order",
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RecordCheckpoint { checkpoint },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
            actual: BudgetUsage {
                tokens: Some(10),
                cost_micro_usd: Some(20),
                runtime_ms: Some(30),
            },
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleAttempt {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            status: AttemptStatus::Cancelled,
            failure: Some(AttemptFailure {
                class: FailureClass::Cancelled,
                code: "attempt_cancelled".to_string(),
                safe_message: "The attempt was cancelled".to_string(),
            }),
            usage_reconciled: true,
        },
    );
    let verified_cleanup = cleanup_record(&run, "cleanup-verified", CleanupState::Verified);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RecordAttemptCleanup {
            cleanup: verified_cleanup,
        },
    );
    assert_eq!(
        run.attempts[&RunAttemptId::new("attempt-task-one-1")].cleanup_state,
        CleanupState::Verified
    );
    let verified_event = events.last().expect("verified cleanup event").clone();
    assert!(
        apply(&run, &verified_event)
            .expect("exact cleanup replay")
            .duplicate
    );
    let mut changed = verified_event;
    let RunEventKind::AttemptCleanupRecorded { cleanup } = &mut changed.kind else {
        panic!("cleanup event")
    };
    cleanup.reason_code = "cleanup_changed".to_string();
    assert_eq!(
        apply(&run, &changed)
            .expect_err("changed cleanup replay conflicts")
            .code,
        OrchestrationErrorCode::ReplayConflict
    );
}

#[test]
fn approvals_are_digest_revision_and_expiry_bound_before_dispatch() {
    let seed = seed(OrchestrationMode::ApproveFirst);
    let mut run = Run::draft(RunId::new("run-approval"), seed.clone(), 1_001);
    let mut events = Vec::new();
    execute_command(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan {
            seed,
            plan: one_task_plan(),
        },
    );
    acknowledge_all(&mut run, &mut events);
    let approval = ApprovalRequirement {
        id: ApprovalId::new("approval-plan"),
        run_id: run.id.clone(),
        task_id: RunTaskId::new("task-one"),
        kind: RunApprovalKind::Plan,
        policy_revision: run.policy_revision.clone(),
        action_digest: "a".repeat(64),
        safe_summary: "Approve plan".to_string(),
        reason_codes: vec!["approve_first".to_string()],
        expires_at_ms: 10_000,
        status: RunApprovalStatus::Pending,
        decision: None,
    };
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestApproval {
            approval: approval.clone(),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_kind(
        &mut run,
        &mut events,
        RunEventKind::ApprovalResolved {
            approval_id: approval.id.clone(),
            action_digest: "b".repeat(64),
            status: RunApprovalStatus::Approved,
            provenance: ApprovalDecisionProvenance {
                actor: "reviewer".to_string(),
                decided_at_ms: 2_000,
                decision_id: "decision-stale".to_string(),
            },
        },
    );
    assert_eq!(
        run.approvals[&approval.id].status,
        RunApprovalStatus::Pending
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ResolveApproval {
            approval_id: approval.id.clone(),
            action_digest: approval.action_digest.clone(),
            status: RunApprovalStatus::Approved,
            provenance: ApprovalDecisionProvenance {
                actor: "reviewer".to_string(),
                decided_at_ms: 2_100,
                decision_id: "decision-current".to_string(),
            },
        },
    );
    assert_eq!(run.status, RunStatus::Ready);
    assert_eq!(
        run.tasks[&RunTaskId::new("task-one")].status,
        RunTaskStatus::Ready
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SelectRoute {
            decision: route("task-one", "provider-one"),
        },
    );
    let mut invalid_date = reservation("task-one", 1);
    invalid_date.ledger_utc_date = "2026-02-30".to_string();
    assert_eq!(
        decide(
            &run,
            RunCommand::ReserveBudget {
                reservation: invalid_date,
            },
        )
        .expect_err("the UTC ledger date must be a real calendar date")
        .code,
        OrchestrationErrorCode::InvalidEvent
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-one", 1),
        },
    );
    let mut late_event = next_event(
        &run,
        999,
        RunEventKind::DispatchRequested {
            attempt: attempt("task-one", 1, "provider-one"),
        },
    );
    late_event.at_ms = approval.expires_at_ms;
    assert_eq!(
        apply(&run, &late_event)
            .expect_err("expired approval blocks dispatch")
            .code,
        OrchestrationErrorCode::IllegalTransition
    );
}

#[test]
fn canonical_blackboard_content_is_hash_bound_and_cannot_change_authority() {
    let lifecycle = successful_lifecycle();
    let run = &lifecycle.final_run;
    let entry = run
        .blackboard
        .values()
        .next()
        .expect("atomic task success stored the artifact");
    assert_eq!(
        run.tasks[&RunTaskId::new("task-one")].access_ceiling,
        AccessMode::WorkspaceOnly
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&entry.canonical_json)
            .expect("canonical artifact JSON")["accessMode"],
        "full_access"
    );
    let notification = run
        .settled_effects
        .keys()
        .find_map(|effect_id| {
            lifecycle.events.iter().find_map(|event| {
                let RunEventKind::EffectAcknowledged {
                    effect_id: acknowledged,
                    ..
                } = &event.kind
                else {
                    return None;
                };
                (acknowledged == effect_id).then_some(acknowledged)
            })
        })
        .expect("artifact notification was acknowledged");
    assert!(!notification.as_str().is_empty());

    let success_index = lifecycle
        .events
        .iter()
        .position(|event| matches!(event.kind, RunEventKind::TaskSucceeded { .. }))
        .expect("task success event");
    let pre_success = replay(lifecycle.events[..success_index].iter()).expect("pre-success state");
    let mut bad = artifact(&pre_success.id, "task-one", json!({"ok": true}));
    bad.content_sha256 = "0".repeat(64);
    assert_eq!(
        decide(
            &pre_success,
            RunCommand::CompleteTask {
                task_id: RunTaskId::new("task-one"),
                attempt_id: RunAttemptId::new("attempt-task-one-1"),
                artifacts: vec![bad],
                evidence_ids: BTreeSet::new(),
                summary: None,
            },
        )
        .expect_err("bad hash")
        .code,
        OrchestrationErrorCode::HashMismatch
    );

    assert_eq!(
        decide(
            &pre_success,
            RunCommand::SettleTask {
                task_id: RunTaskId::new("task-one"),
                status: RunTaskStatus::Succeeded,
                summary: None,
            },
        )
        .expect_err("two-step success is forbidden")
        .code,
        OrchestrationErrorCode::IllegalTransition
    );
}

#[test]
fn settlement_requires_reconciled_budget_and_matching_latest_attempt() {
    let lifecycle = successful_lifecycle();
    let reconciliation_index = lifecycle
        .events
        .iter()
        .position(|event| matches!(event.kind, RunEventKind::BudgetReconciled { .. }))
        .expect("budget reconciliation event");
    let before_reconciliation = replay(lifecycle.events[..reconciliation_index].iter())
        .expect("state before reconciliation");
    assert_eq!(
        decide(
            &before_reconciliation,
            RunCommand::SettleRun {
                status: RunStatus::Succeeded,
                outcome: "premature".to_string(),
            },
        )
        .expect_err("reserved budget blocks settlement")
        .code,
        OrchestrationErrorCode::IllegalTransition
    );

    let attempt_settled_index = lifecycle
        .events
        .iter()
        .position(|event| matches!(event.kind, RunEventKind::AttemptSettled { .. }))
        .expect("attempt settlement event");
    let running = replay(lifecycle.events[..=attempt_settled_index].iter())
        .expect("state after attempt settlement");
    assert_eq!(
        decide(
            &running,
            RunCommand::SettleTask {
                task_id: RunTaskId::new("task-one"),
                status: RunTaskStatus::Failed,
                summary: None,
            },
        )
        .expect_err("failed task cannot follow successful attempt")
        .code,
        OrchestrationErrorCode::IllegalTransition
    );
}

#[test]
fn replacement_approval_makes_the_old_decision_id_stale() {
    let seed = seed(OrchestrationMode::Auto);
    let mut run = Run::draft(RunId::new("run-replacement"), seed.clone(), 1_001);
    let mut events = Vec::new();
    execute_command(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan {
            seed,
            plan: one_task_plan(),
        },
    );
    acknowledge_all(&mut run, &mut events);

    let approval_run_id = run.id.clone();
    let approval_policy_revision = run.policy_revision.clone();
    let approval = |id: &str, digest: char| ApprovalRequirement {
        id: ApprovalId::new(id),
        run_id: approval_run_id.clone(),
        task_id: RunTaskId::new("task-one"),
        kind: RunApprovalKind::Access,
        policy_revision: approval_policy_revision.clone(),
        action_digest: digest.to_string().repeat(64),
        safe_summary: "Approve exact access".to_string(),
        reason_codes: vec!["access_review".to_string()],
        expires_at_ms: 20_000,
        status: RunApprovalStatus::Pending,
        decision: None,
    };
    let first = approval("approval-first", 'a');
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestApproval {
            approval: first.clone(),
        },
    );
    acknowledge_all(&mut run, &mut events);
    let replacement = approval("approval-replacement", 'b');
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestApproval {
            approval: replacement.clone(),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ResolveApproval {
            approval_id: first.id.clone(),
            action_digest: first.action_digest.clone(),
            status: RunApprovalStatus::Approved,
            provenance: ApprovalDecisionProvenance {
                actor: "reviewer".to_string(),
                decided_at_ms: 2_000,
                decision_id: "stale-decision".to_string(),
            },
        },
    );
    assert_eq!(run.approvals[&first.id].status, RunApprovalStatus::Pending);
    assert_eq!(
        run.tasks[&RunTaskId::new("task-one")].status,
        RunTaskStatus::AwaitingApproval
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ResolveApproval {
            approval_id: replacement.id.clone(),
            action_digest: replacement.action_digest.clone(),
            status: RunApprovalStatus::Approved,
            provenance: ApprovalDecisionProvenance {
                actor: "reviewer".to_string(),
                decided_at_ms: 2_100,
                decision_id: "current-decision".to_string(),
            },
        },
    );
    assert_eq!(
        run.tasks[&RunTaskId::new("task-one")].status,
        RunTaskStatus::Ready
    );
}

#[test]
fn run_parallelism_and_plan_rejection_cannot_be_bypassed() {
    let mut plan = one_task_plan();
    let second = task("task-two", RunTaskStatus::Ready);
    plan.topological_order.push(second.id.clone());
    plan.tasks.insert(second.id.clone(), second);
    let mut limited_seed = seed(OrchestrationMode::Auto);
    limited_seed.max_parallel_tasks = 1;
    let mut run = Run::draft(
        RunId::new("run-parallel-limit"),
        limited_seed.clone(),
        1_001,
    );
    let mut events = Vec::new();
    execute_command(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan {
            seed: limited_seed,
            plan,
        },
    );
    acknowledge_all(&mut run, &mut events);
    for task_id in ["task-one", "task-two"] {
        execute_command(
            &mut run,
            &mut events,
            RunCommand::SelectRoute {
                decision: route(task_id, "provider-one"),
            },
        );
        execute_command(
            &mut run,
            &mut events,
            RunCommand::ReserveBudget {
                reservation: reservation(task_id, 1),
            },
        );
    }
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestDispatch {
            attempt: attempt("task-one", 1, "provider-one"),
        },
    );
    assert_eq!(
        decide(
            &run,
            RunCommand::RequestDispatch {
                attempt: attempt("task-two", 1, "provider-one"),
            },
        )
        .expect_err("parallel limit blocks second dispatch")
        .code,
        OrchestrationErrorCode::IllegalTransition
    );

    let approve_seed = seed(OrchestrationMode::ApproveFirst);
    let mut approval_run = Run::draft(RunId::new("run-rejected-plan"), approve_seed.clone(), 1_001);
    let mut approval_events = Vec::new();
    execute_command(
        &mut approval_run,
        &mut approval_events,
        RunCommand::AcceptPlan {
            seed: approve_seed,
            plan: one_task_plan(),
        },
    );
    acknowledge_all(&mut approval_run, &mut approval_events);
    let approval = ApprovalRequirement {
        id: ApprovalId::new("approval-reject-plan"),
        run_id: approval_run.id.clone(),
        task_id: RunTaskId::new("task-one"),
        kind: RunApprovalKind::Plan,
        policy_revision: approval_run.policy_revision.clone(),
        action_digest: "c".repeat(64),
        safe_summary: "Approve plan".to_string(),
        reason_codes: vec!["approve_first".to_string()],
        expires_at_ms: 20_000,
        status: RunApprovalStatus::Pending,
        decision: None,
    };
    execute_command(
        &mut approval_run,
        &mut approval_events,
        RunCommand::RequestApproval {
            approval: approval.clone(),
        },
    );
    acknowledge_all(&mut approval_run, &mut approval_events);
    execute_command(
        &mut approval_run,
        &mut approval_events,
        RunCommand::ResolveApproval {
            approval_id: approval.id,
            action_digest: approval.action_digest,
            status: RunApprovalStatus::Rejected,
            provenance: ApprovalDecisionProvenance {
                actor: "reviewer".to_string(),
                decided_at_ms: 2_000,
                decision_id: "reject-plan".to_string(),
            },
        },
    );
    assert_eq!(approval_run.status, RunStatus::Paused);
    assert_eq!(
        decide(&approval_run, RunCommand::Resume)
            .expect_err("resume cannot bypass rejected plan approval")
            .code,
        OrchestrationErrorCode::IllegalTransition
    );
}

#[test]
fn attempt_bound_reservations_refuse_mismatched_dispatch_and_clear_for_retry() {
    let seed = seed(OrchestrationMode::Auto);
    let mut run = Run::draft(RunId::new("run-attempt-budget"), seed.clone(), 1_001);
    let mut events = Vec::new();
    execute_command(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan {
            seed,
            plan: one_task_plan(),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SelectRoute {
            decision: route("task-one", "provider-one"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-one", 1),
        },
    );

    let mut wrong_attempt = attempt("task-one", 1, "provider-one");
    wrong_attempt.id = RunAttemptId::new("attempt-wrong");
    assert_eq!(
        decide(
            &run,
            RunCommand::RequestDispatch {
                attempt: wrong_attempt,
            },
        )
        .expect_err("reservation cannot authorize another attempt")
        .code,
        OrchestrationErrorCode::IllegalTransition
    );

    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReleaseBudget {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
        },
    );
    assert!(
        run.tasks[&RunTaskId::new("task-one")]
            .budget_reservation_id
            .is_none()
    );
    let mut replacement_reservation = reservation("task-one", 1);
    replacement_reservation.id = BudgetReservationId::new("budget-task-one-1-retry");
    replacement_reservation.attempt_id = RunAttemptId::new("attempt-task-one-retry");
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: replacement_reservation,
        },
    );
}

#[test]
fn unknown_and_over_budget_usage_remain_truthful_until_exact_grant_and_resume() {
    let (mut run, mut events) = successful_attempt_before_reconciliation();
    assert_eq!(
        decide(
            &run,
            RunCommand::ReleaseBudget {
                reservation_id: BudgetReservationId::new("budget-task-one-1"),
            },
        )
        .expect_err("a provider-started reservation must reconcile truthful usage")
        .code,
        OrchestrationErrorCode::IllegalTransition
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
            actual: BudgetUsage {
                tokens: Some(1_200),
                cost_micro_usd: None,
                runtime_ms: Some(270),
            },
        },
    );
    assert_eq!(run.budget.reconciled.tokens, 1_200);
    assert_eq!(run.budget.reserved.cost_micro_usd, 200);
    assert!(run.budget.incomplete.cost_micro_usd);
    assert_eq!(run.budget.overage.tokens, 1_100);
    assert_eq!(run.status, RunStatus::Paused);
    assert!(!run.attempts[&RunAttemptId::new("attempt-task-one-1")].usage_reconciled);

    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
            actual: BudgetUsage {
                tokens: Some(1_200),
                cost_micro_usd: Some(180),
                runtime_ms: Some(270),
            },
        },
    );
    assert!(!run.budget.incomplete.cost_micro_usd);
    assert_eq!(run.budget.reserved, BudgetAmount::default());
    assert_eq!(run.budget.overage.tokens, 1_100);
    assert!(
        run.tasks[&RunTaskId::new("task-one")]
            .budget_reservation_id
            .is_none()
    );
    assert!(run.attempts[&RunAttemptId::new("attempt-task-one-1")].usage_reconciled);
    assert_eq!(
        decide(&run, RunCommand::Resume)
            .expect_err("ungranted overage remains paused")
            .code,
        OrchestrationErrorCode::IllegalTransition
    );

    let approval = ApprovalRequirement {
        id: ApprovalId::new("approval-budget-overage"),
        run_id: run.id.clone(),
        task_id: RunTaskId::new("task-one"),
        kind: RunApprovalKind::Budget,
        policy_revision: run.policy_revision.clone(),
        action_digest: "d".repeat(64),
        safe_summary: "Grant 1100 tokens to this exact attempt".to_string(),
        reason_codes: vec!["actual_usage_overage".to_string()],
        expires_at_ms: 20_000,
        status: RunApprovalStatus::Pending,
        decision: None,
    };
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestApproval {
            approval: approval.clone(),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ResolveApproval {
            approval_id: approval.id.clone(),
            action_digest: approval.action_digest.clone(),
            status: RunApprovalStatus::Approved,
            provenance: ApprovalDecisionProvenance {
                actor: "local-reviewer".to_string(),
                decided_at_ms: 2_500,
                decision_id: "decision-budget-overage".to_string(),
            },
        },
    );
    let grant_run_id = run.id.clone();
    let grant_policy_revision = run.policy_revision.clone();
    execute_command(
        &mut run,
        &mut events,
        RunCommand::GrantSupplementalBudget {
            grant: SupplementalBudgetGrant {
                id: BudgetGrantId::new("grant-budget-overage"),
                run_id: grant_run_id,
                task_id: RunTaskId::new("task-one"),
                attempt_id: RunAttemptId::new("attempt-task-one-1"),
                approval_id: approval.id,
                dimension: BudgetDimension::Tokens,
                amount: 1_100,
                policy_revision: grant_policy_revision,
                action_digest: approval.action_digest,
                expires_at_ms: 20_000,
            },
        },
    );
    assert_eq!(run.budget.overage, BudgetAmount::default());
    execute_command(&mut run, &mut events, RunCommand::Resume);
    assert_eq!(run.status, RunStatus::Ready);
}

#[test]
fn checkpoints_and_ambiguity_are_replayable_and_forbid_fallback() {
    let lifecycle = successful_lifecycle();
    let start_index = lifecycle
        .events
        .iter()
        .position(|event| matches!(event.kind, RunEventKind::AttemptStarted { .. }))
        .expect("attempt start event");
    let mut events = lifecycle.events[..start_index].to_vec();
    let mut run = replay(events.iter()).expect("prepared attempt state");
    let checkpoint = CheckpointRef {
        id: CheckpointId::new("checkpoint-before-one"),
        run_id: run.id.clone(),
        value: "before-commit".to_string(),
        task_id: RunTaskId::new("task-one"),
        attempt_id: RunAttemptId::new("attempt-task-one-1"),
        thread_id: None,
        kind: CheckpointKind::BeforeAttempt,
        created_at_ms: 1_000 + run.last_seq + 1,
    };
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RecordCheckpoint {
            checkpoint: checkpoint.clone(),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::StartAttempt {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            thread_id: ThreadId::new("thread-one"),
            turn_attempt_id: TurnAttemptId::new("turn-attempt-one"),
        },
    );
    assert_eq!(
        decide(
            &run,
            RunCommand::MarkAttemptAmbiguous {
                attempt_id: RunAttemptId::new("attempt-task-one-1"),
                reason_code: "raw provider response".to_string(),
            },
        )
        .expect_err("ambiguity state stores a bounded code, not raw text")
        .code,
        OrchestrationErrorCode::IllegalTransition
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::MarkAttemptAmbiguous {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            reason_code: "provider_acceptance_unknown".to_string(),
        },
    );
    assert!(
        decide(
            &run,
            RunCommand::MarkAttemptAmbiguous {
                attempt_id: RunAttemptId::new("attempt-task-one-1"),
                reason_code: "provider_acceptance_unknown".to_string(),
            },
        )
        .expect("identical ambiguity is idempotent")
        .is_empty()
    );
    let replayed = replay(events.iter()).expect("checkpoint and ambiguity replay");
    assert_eq!(canonical_run_bytes(&replayed), canonical_run_bytes(&run));
    assert_eq!(run.checkpoints[&checkpoint.id], checkpoint);

    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleAttempt {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            status: AttemptStatus::Failed,
            failure: Some(AttemptFailure {
                class: FailureClass::Provider,
                code: "ambiguous_dispatch".to_string(),
                safe_message: "Provider acceptance is unknown".to_string(),
            }),
            usage_reconciled: false,
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
            actual: BudgetUsage {
                tokens: Some(0),
                cost_micro_usd: Some(0),
                runtime_ms: Some(0),
            },
        },
    );
    assert_eq!(
        run.tasks[&RunTaskId::new("task-one")].status,
        RunTaskStatus::Blocked
    );
}

#[test]
fn task_success_requires_server_fact_evidence_from_the_exact_attempt() {
    let mut plan = one_task_plan();
    plan.tasks
        .get_mut(&RunTaskId::new("task-one"))
        .expect("task exists")
        .required_verification = BTreeSet::from([VerificationEvidenceKind::Command]);
    let seed = seed(OrchestrationMode::Auto);
    let mut run = Run::draft(RunId::new("run-required-evidence"), seed.clone(), 1_001);
    let mut events = Vec::new();
    execute_command(&mut run, &mut events, RunCommand::AcceptPlan { seed, plan });
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SelectRoute {
            decision: route("task-one", "provider-one"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-one", 1),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestDispatch {
            attempt: attempt("task-one", 1, "provider-one"),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::StartAttempt {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            thread_id: ThreadId::new("thread-evidence"),
            turn_attempt_id: TurnAttemptId::new("turn-evidence"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleAttempt {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            status: AttemptStatus::Succeeded,
            failure: None,
            usage_reconciled: false,
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
            actual: BudgetUsage {
                tokens: Some(10),
                cost_micro_usd: Some(20),
                runtime_ms: Some(30),
            },
        },
    );

    let output = artifact(&run.id, "task-one", json!({"verified": true}));
    let completion = |evidence_ids| RunCommand::CompleteTask {
        task_id: RunTaskId::new("task-one"),
        attempt_id: RunAttemptId::new("attempt-task-one-1"),
        artifacts: vec![output.clone()],
        evidence_ids,
        summary: Some("verified".to_string()),
    };
    assert_eq!(
        decide(&run, completion(BTreeSet::new()))
            .expect_err("agent prose cannot substitute for required evidence")
            .code,
        OrchestrationErrorCode::IllegalTransition
    );

    let evidence_run_id = run.id.clone();
    let evidence = |id: &str, outcome, recorded_at_ms| VerificationEvidence {
        id: VerificationEvidenceId::new(id),
        run_id: evidence_run_id.clone(),
        task_id: RunTaskId::new("task-one"),
        attempt_id: RunAttemptId::new("attempt-task-one-1"),
        kind: VerificationEvidenceKind::Command,
        outcome,
        command_digest: Some("a".repeat(64)),
        arguments_digest: Some("b".repeat(64)),
        exit_status: Some(0),
        bounded_output_digest: "c".repeat(64),
        checkpoint_id: None,
        checkpoint_diff_digest: None,
        tool_item_id: None,
        usage_observation_id: None,
        provider_instance_id: ProviderInstanceId::new("provider-one"),
        thread_id: ThreadId::new("thread-evidence"),
        turn_attempt_id: TurnAttemptId::new("turn-evidence"),
        recorded_at_ms,
    };
    let mut wrong_owner = evidence(
        "evidence-wrong-owner",
        VerificationEvidenceOutcome::Passed,
        1_000 + run.last_seq + 1,
    );
    wrong_owner.thread_id = ThreadId::new("thread-other");
    assert_eq!(
        decide(
            &run,
            RunCommand::RecordVerificationEvidence {
                evidence: wrong_owner,
            },
        )
        .expect_err("evidence must belong to the exact managed turn")
        .code,
        OrchestrationErrorCode::InvalidEvent
    );

    let failed = evidence(
        "evidence-failed",
        VerificationEvidenceOutcome::Failed,
        1_000 + run.last_seq + 1,
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RecordVerificationEvidence {
            evidence: failed.clone(),
        },
    );
    assert_eq!(
        decide(&run, completion(BTreeSet::from([failed.id])))
            .expect_err("failed evidence cannot satisfy success")
            .code,
        OrchestrationErrorCode::InvalidEvent
    );

    let passed = evidence(
        "evidence-passed",
        VerificationEvidenceOutcome::Passed,
        1_000 + run.last_seq + 1,
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RecordVerificationEvidence {
            evidence: passed.clone(),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        completion(BTreeSet::from([passed.id])),
    );
    assert_eq!(
        canonical_run_bytes(&replay(events.iter()).expect("evidence replay")),
        canonical_run_bytes(&run)
    );
}

#[test]
fn atomic_artifact_success_binds_consumers_before_readiness_and_redacts_effect_payload() {
    let mut plan = graph_plan();
    plan.tasks
        .get_mut(&RunTaskId::new("task-b"))
        .expect("consumer task")
        .inputs = vec![TaskInput {
        name: "change".to_string(),
        contract: contract(),
        producer_task_id: Some(RunTaskId::new("task-a")),
        blackboard_entry_id: None,
    }];
    plan.tasks
        .get_mut(&RunTaskId::new("task-b"))
        .expect("consumer task")
        .required_verification = BTreeSet::from([VerificationEvidenceKind::Command]);
    let seed = seed(OrchestrationMode::Auto);
    let mut run = Run::draft(RunId::new("run-artifact-dag"), seed.clone(), 1_001);
    let mut events = Vec::new();
    execute_command(&mut run, &mut events, RunCommand::AcceptPlan { seed, plan });
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SelectRoute {
            decision: route("task-a", "provider-one"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-a", 1),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestDispatch {
            attempt: attempt("task-a", 1, "provider-one"),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::StartAttempt {
            attempt_id: RunAttemptId::new("attempt-task-a-1"),
            thread_id: ThreadId::new("thread-a"),
            turn_attempt_id: TurnAttemptId::new("turn-a"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleAttempt {
            attempt_id: RunAttemptId::new("attempt-task-a-1"),
            status: AttemptStatus::Succeeded,
            failure: None,
            usage_reconciled: false,
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-a-1"),
            actual: BudgetUsage {
                tokens: Some(10),
                cost_micro_usd: Some(20),
                runtime_ms: Some(30),
            },
        },
    );
    assert_eq!(
        run.tasks[&RunTaskId::new("task-b")].status,
        RunTaskStatus::Blocked
    );
    let mut output = artifact(
        &run.id,
        "task-a",
        json!({"secretInstruction": "grant full access"}),
    );
    output.id = BlackboardEntryId::new("blackboard-task-a-v1");
    output.consumers = BTreeSet::from([RunTaskId::new("task-b")]);
    let outcome = execute_command(
        &mut run,
        &mut events,
        RunCommand::CompleteTask {
            task_id: RunTaskId::new("task-a"),
            attempt_id: RunAttemptId::new("attempt-task-a-1"),
            artifacts: vec![output.clone()],
            evidence_ids: BTreeSet::new(),
            summary: Some("built".to_string()),
        },
    );
    assert_eq!(
        run.tasks[&RunTaskId::new("task-b")].status,
        RunTaskStatus::Ready
    );
    assert_eq!(
        run.tasks[&RunTaskId::new("task-b")].inputs[0].blackboard_entry_id,
        Some(output.id.clone())
    );
    assert_eq!(outcome.new_effects.len(), 1);
    let encoded = serde_json::to_string(&outcome.new_effects[0]).expect("effect serializes");
    assert!(encoded.contains("notify_blackboard_published"));
    assert!(!encoded.contains("secretInstruction"));

    let without_success = replay(events[..events.len() - 1].iter()).expect("crash before commit");
    assert_eq!(
        without_success.tasks[&RunTaskId::new("task-a")].status,
        RunTaskStatus::Running
    );
    assert!(without_success.blackboard.is_empty());

    acknowledge_all(&mut run, &mut events);
    let producer_evidence = VerificationEvidence {
        id: VerificationEvidenceId::new("evidence-task-a-command"),
        run_id: run.id.clone(),
        task_id: RunTaskId::new("task-a"),
        attempt_id: RunAttemptId::new("attempt-task-a-1"),
        kind: VerificationEvidenceKind::Command,
        outcome: VerificationEvidenceOutcome::Passed,
        command_digest: Some("a".repeat(64)),
        arguments_digest: Some("b".repeat(64)),
        exit_status: Some(0),
        bounded_output_digest: "c".repeat(64),
        checkpoint_id: None,
        checkpoint_diff_digest: None,
        tool_item_id: None,
        usage_observation_id: None,
        provider_instance_id: ProviderInstanceId::new("provider-one"),
        thread_id: ThreadId::new("thread-a"),
        turn_attempt_id: TurnAttemptId::new("turn-a"),
        recorded_at_ms: 1_000 + run.last_seq + 1,
    };
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RecordVerificationEvidence {
            evidence: producer_evidence.clone(),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SelectRoute {
            decision: route("task-b", "provider-two"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-b", 1),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::RequestDispatch {
            attempt: attempt("task-b", 1, "provider-two"),
        },
    );
    acknowledge_all(&mut run, &mut events);
    execute_command(
        &mut run,
        &mut events,
        RunCommand::StartAttempt {
            attempt_id: RunAttemptId::new("attempt-task-b-1"),
            thread_id: ThreadId::new("thread-b"),
            turn_attempt_id: TurnAttemptId::new("turn-b"),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleAttempt {
            attempt_id: RunAttemptId::new("attempt-task-b-1"),
            status: AttemptStatus::Succeeded,
            failure: None,
            usage_reconciled: false,
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-b-1"),
            actual: BudgetUsage {
                tokens: Some(10),
                cost_micro_usd: Some(20),
                runtime_ms: Some(30),
            },
        },
    );
    let reviewer_artifact = artifact(&run.id, "task-b", json!({"reviewed": true}));
    execute_command(
        &mut run,
        &mut events,
        RunCommand::CompleteTask {
            task_id: RunTaskId::new("task-b"),
            attempt_id: RunAttemptId::new("attempt-task-b-1"),
            artifacts: vec![reviewer_artifact],
            evidence_ids: BTreeSet::from([producer_evidence.id]),
            summary: Some("reviewed durable producer evidence".to_string()),
        },
    );
    assert_eq!(
        run.tasks[&RunTaskId::new("task-b")].status,
        RunTaskStatus::Succeeded
    );
}

#[test]
fn artifact_serialization_boundaries_are_enforced_before_atomic_success() {
    let lifecycle = successful_lifecycle();
    let success_index = lifecycle
        .events
        .iter()
        .position(|event| matches!(event.kind, RunEventKind::TaskSucceeded { .. }))
        .expect("task success event");
    let pre_success = replay(lifecycle.events[..success_index].iter()).expect("pre-success run");
    let complete = |content| RunCommand::CompleteTask {
        task_id: RunTaskId::new("task-one"),
        attempt_id: RunAttemptId::new("attempt-task-one-1"),
        artifacts: vec![artifact(&pre_success.id, "task-one", content)],
        evidence_ids: BTreeSet::new(),
        summary: None,
    };

    let boundary_string = "x".repeat(pandamux_core::MAX_ARTIFACT_STRING_BYTES);
    decide(&pre_success, complete(json!({"value": boundary_string})))
        .expect("the exact string boundary is accepted");
    let oversized = "x".repeat(pandamux_core::MAX_ARTIFACT_STRING_BYTES + 1);
    assert_eq!(
        decide(&pre_success, complete(json!({"value": oversized})))
            .expect_err("oversized string is rejected")
            .code,
        OrchestrationErrorCode::InvalidEvent
    );

    let boundary_array = Value::Array(
        (0..pandamux_core::MAX_ARTIFACT_CONTAINER_ITEMS)
            .map(|_| json!(0))
            .collect(),
    );
    decide(&pre_success, complete(boundary_array))
        .expect("the exact container item boundary is accepted");
    let oversized_array = Value::Array(
        (0..=pandamux_core::MAX_ARTIFACT_CONTAINER_ITEMS)
            .map(|_| json!(0))
            .collect(),
    );
    assert_eq!(
        decide(&pre_success, complete(oversized_array))
            .expect_err("oversized arrays are rejected")
            .code,
        OrchestrationErrorCode::InvalidEvent
    );

    let nested = |levels: usize| (0..levels).fold(json!(0), |value, _| Value::Array(vec![value]));
    decide(
        &pre_success,
        complete(nested(pandamux_core::MAX_ARTIFACT_JSON_DEPTH)),
    )
    .expect("the exact nesting boundary is accepted");
    assert_eq!(
        decide(
            &pre_success,
            complete(nested(pandamux_core::MAX_ARTIFACT_JSON_DEPTH + 1)),
        )
        .expect_err("excessive nesting is rejected")
        .code,
        OrchestrationErrorCode::InvalidEvent
    );

    let max_string = pandamux_core::MAX_ARTIFACT_STRING_BYTES;
    let exact_last = pandamux_core::MAX_ARTIFACT_BYTES - (3 * max_string) - 13;
    let exact_bytes = json!([
        "x".repeat(max_string),
        "x".repeat(max_string),
        "x".repeat(max_string),
        "x".repeat(exact_last)
    ]);
    assert_eq!(
        canonical_artifact_json(&exact_bytes).len(),
        pandamux_core::MAX_ARTIFACT_BYTES
    );
    decide(&pre_success, complete(exact_bytes))
        .expect("the exact canonical byte boundary is accepted");

    let oversized_bytes = json!([
        "x".repeat(max_string),
        "x".repeat(max_string),
        "x".repeat(max_string),
        "x".repeat(exact_last + 1)
    ]);
    assert_eq!(
        decide(&pre_success, complete(oversized_bytes))
            .expect_err("oversized canonical artifact bytes are rejected")
            .code,
        OrchestrationErrorCode::InvalidEvent
    );
}

#[test]
fn bounded_event_alphabet_is_total_and_deterministic() {
    let seed = seed(OrchestrationMode::Auto);
    let run = Run::draft(RunId::new("run-alphabet"), seed.clone(), 1_001);
    let approval = ApprovalRequirement {
        id: ApprovalId::new("approval-x"),
        run_id: run.id.clone(),
        task_id: RunTaskId::new("task-one"),
        kind: RunApprovalKind::Plan,
        policy_revision: run.policy_revision.clone(),
        action_digest: "a".repeat(64),
        safe_summary: "summary".to_string(),
        reason_codes: Vec::new(),
        expires_at_ms: 9_999,
        status: RunApprovalStatus::Pending,
        decision: None,
    };
    let dummy_receipt = EffectReceipt {
        effect_id: pandamux_core::OrchestrationEffectId::new("effect-x"),
        acknowledged_at_ms: 2_000,
        result_digest: "result".to_string(),
    };
    let dummy_artifact = artifact(&run.id, "task-one", json!({}));
    let dummy_checkpoint = CheckpointRef {
        id: CheckpointId::new("checkpoint-x"),
        run_id: run.id.clone(),
        value: "deadbeef".to_string(),
        task_id: RunTaskId::new("task-one"),
        attempt_id: RunAttemptId::new("attempt-task-one-1"),
        thread_id: None,
        kind: CheckpointKind::BeforeAttempt,
        created_at_ms: 2_000,
    };
    let dummy_evidence = VerificationEvidence {
        id: VerificationEvidenceId::new("evidence-x"),
        run_id: run.id.clone(),
        task_id: RunTaskId::new("task-one"),
        attempt_id: RunAttemptId::new("attempt-task-one-1"),
        kind: VerificationEvidenceKind::Command,
        outcome: VerificationEvidenceOutcome::Passed,
        command_digest: Some("a".repeat(64)),
        arguments_digest: Some("b".repeat(64)),
        exit_status: Some(0),
        bounded_output_digest: "c".repeat(64),
        checkpoint_id: None,
        checkpoint_diff_digest: None,
        tool_item_id: None,
        usage_observation_id: None,
        provider_instance_id: ProviderInstanceId::new("provider-one"),
        thread_id: ThreadId::new("thread-one"),
        turn_attempt_id: TurnAttemptId::new("turn-attempt-one"),
        recorded_at_ms: 2_000,
    };
    let kinds = vec![
        RunEventKind::PlanAccepted {
            seed,
            plan: one_task_plan(),
        },
        RunEventKind::ApprovalRequested { approval },
        RunEventKind::ApprovalResolved {
            approval_id: ApprovalId::new("approval-x"),
            action_digest: "a".repeat(64),
            status: RunApprovalStatus::Approved,
            provenance: ApprovalDecisionProvenance {
                actor: "actor".to_string(),
                decided_at_ms: 2_000,
                decision_id: "decision".to_string(),
            },
        },
        RunEventKind::BudgetReserved {
            reservation: reservation("task-one", 1),
        },
        RunEventKind::BudgetReconciled {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
            actual: BudgetUsage::default(),
        },
        RunEventKind::BudgetReleased {
            reservation_id: BudgetReservationId::new("budget-task-one-1"),
        },
        RunEventKind::SupplementalBudgetGranted {
            grant: SupplementalBudgetGrant {
                id: BudgetGrantId::new("grant-x"),
                run_id: run.id.clone(),
                task_id: RunTaskId::new("task-one"),
                attempt_id: RunAttemptId::new("attempt-task-one-1"),
                approval_id: ApprovalId::new("approval-x"),
                dimension: BudgetDimension::Tokens,
                amount: 1,
                policy_revision: run.policy_revision.clone(),
                action_digest: "a".repeat(64),
                expires_at_ms: 9_999,
            },
        },
        RunEventKind::RouteSelected {
            decision: route("task-one", "provider-one"),
        },
        RunEventKind::DispatchRequested {
            attempt: attempt("task-one", 1, "provider-one"),
        },
        RunEventKind::AttemptStarted {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            thread_id: ThreadId::new("thread"),
            turn_attempt_id: TurnAttemptId::new("turn"),
        },
        RunEventKind::AttemptBoundaryAdvanced {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            boundary: AttemptBoundary::VisibleOutputBegun,
        },
        RunEventKind::CheckpointRecorded {
            checkpoint: dummy_checkpoint,
        },
        RunEventKind::AttemptMarkedAmbiguous {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            reason_code: "dispatch_unknown".to_string(),
        },
        RunEventKind::AttemptSettled {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            status: AttemptStatus::Succeeded,
            failure: None,
            usage_reconciled: true,
        },
        RunEventKind::VerificationEvidenceRecorded {
            evidence: dummy_evidence,
        },
        RunEventKind::TaskSucceeded {
            task_id: RunTaskId::new("task-one"),
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            artifacts: vec![dummy_artifact],
            evidence_ids: BTreeSet::new(),
            summary: None,
        },
        RunEventKind::TaskSettled {
            task_id: RunTaskId::new("task-one"),
            status: RunTaskStatus::Failed,
            summary: None,
        },
        RunEventKind::CancellationRequested {
            task_ids: Vec::new(),
        },
        RunEventKind::Paused {
            reason: "pause".to_string(),
        },
        RunEventKind::Resumed,
        RunEventKind::EffectAcknowledged {
            effect_id: dummy_receipt.effect_id.clone(),
            receipt: dummy_receipt,
        },
        RunEventKind::RunSettled {
            status: RunStatus::Succeeded,
            outcome: "done".to_string(),
        },
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        let event = next_event(&run, index, kind);
        assert_eq!(
            apply(&run, &event),
            apply(&run, &event),
            "event variant {index}"
        );
    }
}
