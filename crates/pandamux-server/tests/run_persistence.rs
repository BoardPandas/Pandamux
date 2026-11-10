use pandamux_core::{
    AccessMode, AgentId, AgentTask, Attempt, AttemptBoundary, AttemptStatus, BudgetAmount,
    BudgetReservation, BudgetReservationId, BudgetReservationState, BudgetUsage, CleanupState,
    ConfigurationRevision, OrchestrationMode, OutputContract, ProjectId, ProjectRepositoryId,
    ProjectRepositoryRecord, ProviderInstanceId, RoutingDecision, Run, RunAttemptId, RunCommand,
    RunEvent, RunEventId, RunId, RunSeed, RunTaskId, RunTaskStatus, RunTrigger, SubscriptionKey,
    TaskRouteDecision, ValidatedPlan,
};
use pandamux_server::{
    AssignedRunEvent, OrchestrationExecutionFailpoint, OrchestrationRunRecord, RunExecutionService,
    RunTransitionRequest, RunTransitionRows, Store, orchestration_snapshot_sha256,
};
use std::collections::{BTreeMap, BTreeSet};

fn output_contract() -> OutputContract {
    OutputContract {
        kind: "change_set".to_string(),
        schema: None,
    }
}

fn seed() -> RunSeed {
    RunSeed {
        trigger: RunTrigger::Manual {
            client_id: "run-persistence-test".to_string(),
        },
        policy_revision: "policy-run-persistence".to_string(),
        bundle_sha: "a".repeat(64),
        organization_id: pandamux_core::OrganizationId::new("org-run-persistence"),
        mode: OrchestrationMode::Auto,
        budget_limit: BudgetAmount {
            tokens: 1_000,
            cost_micro_usd: 2_000,
            runtime_ms: 3_000,
        },
        max_parallel_tasks: 1,
    }
}

fn task() -> AgentTask {
    AgentTask {
        id: RunTaskId::new("task-run-persistence"),
        summary: "Persist a run transition".to_string(),
        class: "build".to_string(),
        project_id: ProjectId::new("project-run-persistence"),
        organization_id: pandamux_core::OrganizationId::new("org-run-persistence"),
        required_capabilities: BTreeSet::new(),
        access_ceiling: AccessMode::WorkspaceOnly,
        inputs: Vec::new(),
        outputs: vec![output_contract()],
        required_verification: BTreeSet::new(),
        selected_agent_id: None,
        route: None,
        status: RunTaskStatus::Ready,
        max_attempts: 1,
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
        topological_order: vec![RunTaskId::new("task-run-persistence")],
        total_budget: BudgetAmount {
            tokens: 100,
            cost_micro_usd: 200,
            runtime_ms: 300,
        },
    }
}

fn route() -> TaskRouteDecision {
    TaskRouteDecision {
        task_id: RunTaskId::new("task-run-persistence"),
        selected_agent_id: Some(AgentId::new("agent-run-persistence")),
        agent_evaluations: Vec::new(),
        provider: RoutingDecision {
            selected_provider_instance_id: Some(ProviderInstanceId::new(
                "provider-run-persistence",
            )),
            selected_subscription_key: Some(SubscriptionKey::new("subscription-run-persistence")),
            selected_model: Some("model-run-persistence".to_string()),
            evaluations: Vec::new(),
        },
    }
}

fn reservation() -> BudgetReservation {
    BudgetReservation {
        id: BudgetReservationId::new("budget-run-persistence"),
        task_id: RunTaskId::new("task-run-persistence"),
        attempt_id: RunAttemptId::new("attempt-run-persistence"),
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
        id: RunAttemptId::new("attempt-run-persistence"),
        task_id: RunTaskId::new("task-run-persistence"),
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

fn apply(run: &mut Run, events: &mut Vec<RunEvent>, command: RunCommand) {
    let kinds = pandamux_orchestrator::decide(run, command).expect("fixture command");
    for kind in kinds {
        let event = RunEvent {
            event_id: RunEventId::new(format!("run-persistence-event-{}", run.last_seq + 1)),
            run_id: run.id.clone(),
            seq: run.last_seq + 1,
            at_ms: 10_000 + run.last_seq + 1,
            idempotency_key: format!("run-persistence-{}", run.last_seq + 1),
            kind,
        };
        *run = pandamux_orchestrator::apply(run, &event)
            .expect("fixture event")
            .run;
        events.push(event);
    }
}

fn base_run() -> (Run, Vec<RunEvent>) {
    let seed = seed();
    let mut run = Run::draft(RunId::new("run-persistence"), seed.clone(), 10_001);
    let mut events = Vec::new();
    apply(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan { seed, plan: plan() },
    );
    apply(
        &mut run,
        &mut events,
        RunCommand::SelectRoute { decision: route() },
    );
    apply(
        &mut run,
        &mut events,
        RunCommand::ReserveBudget {
            reservation: reservation(),
        },
    );
    (run, events)
}

#[test]
fn event_commit_survives_reopen_and_retries_without_duplicate_effects() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let database = temp.path().join("pandamux.db");
    let store = Store::open(&database).expect("open store");
    store
        .save_project_repository(&ProjectRepositoryRecord {
            id: ProjectRepositoryId::new("repository-run-persistence"),
            project_id: ProjectId::new("project-run-persistence"),
            organization_id: pandamux_core::OrganizationId::new("org-run-persistence"),
            canonical_remote_identity: "remote-run-persistence".to_string(),
            canonical_repository_root_identity: "root-run-persistence".to_string(),
            display_path: "PandaMUX".to_string(),
            revision: ConfigurationRevision::INITIAL,
            created_at_ms: 1_000,
            updated_at_ms: 1_000,
        })
        .expect("save repository");
    let (run, events) = base_run();
    store
        .insert_orchestration_run(
            &OrchestrationRunRecord {
                run: run.clone(),
                project_id: ProjectId::new("project-run-persistence"),
                repository_id: ProjectRepositoryId::new("repository-run-persistence"),
                settings_revision: 1,
            },
            &events,
        )
        .expect("insert base run");
    let request = RunTransitionRequest {
        run_id: run.id.clone(),
        expected_last_seq: run.last_seq,
        expected_snapshot_sha256: orchestration_snapshot_sha256(&run),
        command: RunCommand::RequestDispatch { attempt: attempt() },
        assigned_events: vec![AssignedRunEvent {
            event_id: RunEventId::new("run-persistence-dispatch-event"),
            idempotency_key: "run-persistence-dispatch".to_string(),
            at_ms: 20_000,
        }],
        rows: RunTransitionRows::default(),
    };
    let service = RunExecutionService::new(store);
    assert!(
        service
            .commit_transition(
                &request,
                Some(OrchestrationExecutionFailpoint::AfterEventCommit),
            )
            .is_err()
    );
    drop(service);

    let reopened = Store::open(&database).expect("reopen after injected crash");
    let service = RunExecutionService::new(reopened.clone());
    let retry = service
        .commit_transition(&request, None)
        .expect("retry committed transition");
    assert!(retry.duplicate);
    assert_eq!(retry.events.len(), 1);
    assert_eq!(retry.effect_ids.len(), 1);
    assert_eq!(
        reopened
            .load_orchestration_events(&run.id)
            .expect("load durable events")
            .len(),
        events.len() + 1
    );
    assert_eq!(
        reopened
            .load_orchestration_effects(&run.id)
            .expect("load durable effects")
            .iter()
            .filter(|record| {
                matches!(
                    record.effect.kind,
                    pandamux_core::OrchestrationEffectKind::DispatchTask { .. }
                )
            })
            .count(),
        1
    );
    let audit = service.audit_replay(&run.id).expect("audit reopened run");
    assert_eq!(audit.last_seq, retry.run.last_seq);
    assert_eq!(audit.snapshot_sha256, retry.snapshot_sha256);
}
