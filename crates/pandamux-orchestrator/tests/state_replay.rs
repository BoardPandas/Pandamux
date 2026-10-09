use pandamux_core::{
    AccessMode, AgentEvaluation, AgentSelectionScore, AgentTask, ApprovalDecisionProvenance,
    ApprovalId, ApprovalRequirement, Attempt, AttemptBoundary, AttemptFailure, AttemptStatus,
    BlackboardEntry, BlackboardEntryId, BudgetAmount, BudgetReservation, BudgetReservationId,
    BudgetReservationState, DependencyEdge, DependencyEdgeId, DependencyRequirement, EffectReceipt,
    FailureClass, OrchestrationMode, OrganizationId, OutputContract, ProjectId, ProviderInstanceId,
    RoutingDecision, Run, RunApprovalKind, RunApprovalStatus, RunAttemptId, RunCommand, RunEvent,
    RunEventId, RunEventKind, RunId, RunSeed, RunStatus, RunTaskId, RunTaskStatus, RunTrigger,
    SubscriptionKey, TaskRouteDecision, ThreadId, TurnAttemptId, ValidatedPlan,
};
use pandamux_orchestrator::{
    OrchestrationErrorCode, apply, canonical_blackboard_sha256, canonical_run_bytes, decide,
    replay, replay_from,
};
use serde_json::json;
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

fn reservation(task_id: &str) -> BudgetReservation {
    BudgetReservation {
        id: BudgetReservationId::new(format!("budget-{task_id}")),
        task_id: RunTaskId::new(task_id),
        attempt_id: None,
        amount: BudgetAmount {
            tokens: 100,
            cost_micro_usd: 200,
            runtime_ms: 300,
        },
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
            reservation: reservation("task-one"),
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
            usage_reconciled: true,
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::SettleTask {
            task_id: RunTaskId::new("task-one"),
            status: RunTaskStatus::Succeeded,
            summary: Some("done".to_string()),
        },
    );
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReconcileBudget {
            reservation_id: BudgetReservationId::new("budget-task-one"),
            actual: BudgetAmount {
                tokens: 90,
                cost_micro_usd: 180,
                runtime_ms: 270,
            },
        },
    );
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
            reservation: reservation("task-a"),
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
            usage_reconciled: true,
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
    execute_command(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation("task-one"),
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
    let mut run =
        replay(lifecycle.events[..lifecycle.events.len() - 1].iter()).expect("pre-settlement run");
    let mut events = lifecycle.events[..lifecycle.events.len() - 1].to_vec();
    let content = json!({
        "accessMode": "full_access",
        "organizationId": "org-other",
        "tools": ["shell"]
    });
    let mut entry = BlackboardEntry {
        id: BlackboardEntryId::new("blackboard-one"),
        version: 1,
        producer: RunTaskId::new("task-one"),
        consumers: BTreeSet::new(),
        output_contract: contract(),
        content_sha256: canonical_blackboard_sha256(&content),
        content,
        created_event_id: RunEventId::new("event-blackboard-source"),
    };
    let access_before = run.tasks[&RunTaskId::new("task-one")].access_ceiling;
    let mut event = next_event(
        &run,
        events.len() + 1,
        RunEventKind::BlackboardPublished {
            entry: entry.clone(),
        },
    );
    entry.created_event_id = event.event_id.clone();
    event.kind = RunEventKind::BlackboardPublished {
        entry: entry.clone(),
    };
    let outcome = apply(&run, &event).expect("blackboard event applies");
    run = outcome.run;
    events.push(event);
    assert_eq!(
        run.tasks[&RunTaskId::new("task-one")].access_ceiling,
        access_before
    );
    assert_eq!(run.blackboard[&entry.id], entry);

    let mut bad = entry;
    bad.id = BlackboardEntryId::new("blackboard-bad");
    bad.content_sha256 = "0".repeat(64);
    let mut bad_event = next_event(
        &run,
        1_000,
        RunEventKind::BlackboardPublished { entry: bad },
    );
    let RunEventKind::BlackboardPublished { entry: bad } = &mut bad_event.kind else {
        unreachable!("blackboard event")
    };
    bad.created_event_id = bad_event.event_id.clone();
    assert_eq!(
        apply(&run, &bad_event).expect_err("bad hash").code,
        OrchestrationErrorCode::HashMismatch
    );

    let mut wrong_source = bad_event;
    let RunEventKind::BlackboardPublished { entry } = &mut wrong_source.kind else {
        unreachable!("blackboard event")
    };
    entry.id = BlackboardEntryId::new("blackboard-wrong-source");
    entry.content_sha256 = canonical_blackboard_sha256(&entry.content);
    entry.created_event_id = RunEventId::new("different-event");
    assert_eq!(
        apply(&run, &wrong_source)
            .expect_err("creation event mismatch")
            .code,
        OrchestrationErrorCode::InvalidEvent
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
                reservation: reservation(task_id),
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
    let dummy_entry = BlackboardEntry {
        id: BlackboardEntryId::new("blackboard-x"),
        version: 1,
        producer: RunTaskId::new("task-one"),
        consumers: BTreeSet::new(),
        output_contract: contract(),
        content: json!({}),
        content_sha256: canonical_blackboard_sha256(&json!({})),
        created_event_id: RunEventId::new("event-source"),
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
            reservation: reservation("task-one"),
        },
        RunEventKind::BudgetReconciled {
            reservation_id: BudgetReservationId::new("budget-task-one"),
            actual: BudgetAmount::default(),
        },
        RunEventKind::BudgetReleased {
            reservation_id: BudgetReservationId::new("budget-task-one"),
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
        RunEventKind::AttemptSettled {
            attempt_id: RunAttemptId::new("attempt-task-one-1"),
            status: AttemptStatus::Succeeded,
            failure: None,
            usage_reconciled: true,
        },
        RunEventKind::BlackboardPublished { entry: dummy_entry },
        RunEventKind::TaskSettled {
            task_id: RunTaskId::new("task-one"),
            status: RunTaskStatus::Succeeded,
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
