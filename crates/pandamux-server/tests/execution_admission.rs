use pandamux_core::{
    AccessMode, AgentId, AgentTask, Attempt, AttemptBoundary, AttemptStatus, BroadsidePolicy,
    BudgetAmount, BudgetReservation, BudgetReservationId, BudgetReservationState, BudgetUsage,
    CleanupState, ConfigurationRevision, Environment, EnvironmentId, EnvironmentKind,
    EnvironmentStatus, ExecutionLeaseId, OrchestrationMode, OrganizationId, OrganizationPolicy,
    OutputContract, ProjectId, ProjectRepositoryId, ProjectRepositoryRecord,
    ProviderInstanceConfig, ProviderInstanceId, ProviderKind, RoutingDecision, Run, RunAttemptId,
    RunCommand, RunEvent, RunEventId, RunId, RunSeed, RunTaskId, RunTaskStatus, RunTrigger,
    ServerIncarnationId, SubscriptionKey, TaskRouteDecision, ThreadId, TurnId, UserSettings,
    ValidatedPlan,
};
use pandamux_protocol::{RpcRequest, ServerRole};
use pandamux_providers::MockProviderDriver;
use pandamux_server::{
    AdmissionOwnership, AdmissionRenewStoreRequest, AdmissionRequeueStoreRequest,
    BudgetExecutionFailpoint, DriverRegistry, EnqueueManualAdmissionRequest,
    EnqueueOrchestratedAdmissionRequest, ExecutionAdmissionClaim, ExecutionAdmissionClaimRequest,
    ExecutionAdmissionService, OrchestratedAdmissionTransition, OrchestrationRunRecord, QueueState,
    Router, Store, StoredBudgetReservation, bounded_execution_retry_delay_ms,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tokio::sync::Barrier;

const BASE_TIME: u64 = 100_000;
const ORG: &str = "org-admission";

fn provider_config(id: &str, limit: u32) -> ProviderInstanceConfig {
    ProviderInstanceConfig {
        id: ProviderInstanceId::new(id),
        provider: ProviderKind::Custom,
        display_name: format!("Provider {id}"),
        profile_dir: format!("profiles/{id}"),
        environment_id: None,
        binary_path: Some(format!("/test/{id}")),
        concurrency_limit: limit,
        env_overrides: BTreeMap::new(),
        settings: serde_json::json!({}),
        enabled: true,
        disabled_reason: None,
    }
}

fn environment(id: &str, limit: u32) -> Environment {
    Environment {
        id: EnvironmentId::new(id),
        kind: EnvironmentKind::Local,
        display_name: id.to_string(),
        status: EnvironmentStatus::Ready,
        platform: None,
        server_version: None,
        schedules_count: Some(0),
        last_error: None,
        execution_limit: limit,
        provider_overrides: BTreeMap::new(),
    }
}

fn organization(limit: Option<u32>) -> OrganizationPolicy {
    OrganizationPolicy {
        id: OrganizationId::new(ORG),
        display_name: "Admission organization".to_string(),
        project_ids: Vec::new(),
        allowed_provider_instances: Vec::new(),
        allowed_subscriptions: Vec::new(),
        failover_pool_ids: Vec::new(),
        default_tier_mapping: None,
        max_concurrency: limit,
        daily_budget_micro_usd: None,
        broadside: BroadsidePolicy::default(),
        revision: 1,
    }
}

fn configure(
    store: &Store,
    global_limit: u32,
    environments: &[(&str, u32)],
    providers: &[(&str, u32)],
    organization_limit: Option<u32>,
) {
    let settings = UserSettings {
        environments: environments
            .iter()
            .map(|(id, limit)| environment(id, *limit))
            .collect(),
        providers: providers
            .iter()
            .map(|(id, limit)| provider_config(id, *limit))
            .collect(),
        organizations: vec![organization(organization_limit)],
        advanced: pandamux_core::AdvancedSettings {
            global_concurrency_limit: global_limit,
            ..pandamux_core::AdvancedSettings::default()
        },
        ..UserSettings::default()
    };
    store
        .save_settings(&settings)
        .expect("save admission settings");
}

async fn enqueue_manual(
    service: &ExecutionAdmissionService,
    name: &str,
    provider: &str,
    environment: &str,
    organization: Option<&str>,
    at_ms: u64,
) -> pandamux_server::OrchestrationQueueRecord {
    service
        .enqueue_manual(EnqueueManualAdmissionRequest {
            thread_id: ThreadId::new(format!("thread-{name}")),
            turn_id: TurnId::new(format!("turn-{name}")),
            agent_id: None,
            organization_id: organization.map(OrganizationId::new),
            provider_instance_id: ProviderInstanceId::new(provider),
            environment_id: EnvironmentId::new(environment),
            enqueued_at_ms: at_ms,
        })
        .await
        .expect("enqueue manual admission")
}

async fn claim_manual(
    service: &ExecutionAdmissionService,
    record: &pandamux_server::OrchestrationQueueRecord,
    at_ms: u64,
) -> Option<ExecutionAdmissionClaim> {
    service
        .try_claim_record(record, at_ms, None)
        .await
        .expect("claim manual admission")
}

#[derive(Clone)]
struct OrchestratedTaskFixture {
    run_id: RunId,
    task_id: RunTaskId,
    attempt_id: RunAttemptId,
    agent_id: AgentId,
    organization_id: OrganizationId,
    provider_instance_id: ProviderInstanceId,
    environment_id: EnvironmentId,
    route: TaskRouteDecision,
}

fn apply(run: &mut Run, events: &mut Vec<RunEvent>, command: RunCommand, at_ms: u64) {
    let kinds = pandamux_orchestrator::decide(run, command).expect("legal fixture command");
    for kind in kinds {
        let next = run.last_seq + 1;
        let event = RunEvent {
            event_id: RunEventId::new(format!("fixture-event-{}-{next}", run.id.as_str())),
            run_id: run.id.clone(),
            seq: next,
            at_ms,
            idempotency_key: format!("fixture:{}:{next}", run.id.as_str()),
            kind,
        };
        *run = pandamux_orchestrator::apply(run, &event)
            .expect("apply fixture event")
            .run;
        events.push(event);
    }
}

fn insert_ready_run(
    store: &Store,
    name: &str,
    agents: &[&str],
    providers: &[&str],
    environment: &str,
    max_parallel_tasks: u32,
) -> Vec<OrchestratedTaskFixture> {
    assert_eq!(agents.len(), providers.len());
    let run_id = RunId::new(format!("run-{name}"));
    let project_id = ProjectId::new(format!("project-{name}"));
    let repository_id = ProjectRepositoryId::new(format!("repository-{name}"));
    let organization_id = OrganizationId::new(ORG);
    store
        .save_project_repository(&ProjectRepositoryRecord {
            id: repository_id.clone(),
            project_id: project_id.clone(),
            organization_id: organization_id.clone(),
            canonical_remote_identity: format!("remote-{name}"),
            canonical_repository_root_identity: format!("root-{name}"),
            display_path: name.to_string(),
            revision: ConfigurationRevision::INITIAL,
            created_at_ms: BASE_TIME,
            updated_at_ms: BASE_TIME,
        })
        .expect("save fixture repository");

    let tasks = agents
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let task_id = RunTaskId::new(format!("task-{name}-{index}"));
            (
                task_id.clone(),
                AgentTask {
                    id: task_id,
                    summary: format!("Task {index}"),
                    class: "build".to_string(),
                    project_id: project_id.clone(),
                    organization_id: organization_id.clone(),
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
                    max_attempts: 2,
                    approval_ids: BTreeSet::new(),
                    budget_limit: BudgetAmount {
                        tokens: 1_000,
                        cost_micro_usd: 1_000,
                        runtime_ms: 10_000,
                    },
                    budget_reservation_id: None,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let total = u64::try_from(tasks.len()).expect("task count") * 1_000;
    let seed = RunSeed {
        trigger: RunTrigger::Manual {
            client_id: format!("client-{name}"),
        },
        policy_revision: format!("policy-{name}"),
        bundle_sha: "a".repeat(64),
        organization_id: organization_id.clone(),
        mode: OrchestrationMode::Auto,
        budget_limit: BudgetAmount {
            tokens: total,
            cost_micro_usd: total,
            runtime_ms: total.saturating_mul(10),
        },
        max_parallel_tasks,
    };
    let plan = ValidatedPlan {
        schema_version: 1,
        source_digest: "b".repeat(64),
        topological_order: tasks.keys().cloned().collect(),
        tasks,
        edges: BTreeMap::new(),
        total_budget: BudgetAmount {
            tokens: total,
            cost_micro_usd: total,
            runtime_ms: total.saturating_mul(10),
        },
    };
    let mut run = Run::draft(run_id.clone(), seed.clone(), BASE_TIME);
    let mut events = Vec::new();
    apply(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan { seed, plan },
        BASE_TIME,
    );

    let fixtures = agents
        .iter()
        .zip(providers)
        .enumerate()
        .map(|(index, (agent, provider))| {
            let task_id = RunTaskId::new(format!("task-{name}-{index}"));
            let route = TaskRouteDecision {
                task_id: task_id.clone(),
                selected_agent_id: Some(AgentId::new(*agent)),
                agent_evaluations: Vec::new(),
                provider: RoutingDecision {
                    selected_provider_instance_id: Some(ProviderInstanceId::new(*provider)),
                    selected_subscription_key: Some(SubscriptionKey::new(format!(
                        "subscription-{provider}"
                    ))),
                    selected_model: Some("fixture-model".to_string()),
                    evaluations: Vec::new(),
                },
            };
            apply(
                &mut run,
                &mut events,
                RunCommand::SelectRoute {
                    decision: route.clone(),
                },
                BASE_TIME + 1 + index as u64,
            );
            OrchestratedTaskFixture {
                run_id: run_id.clone(),
                task_id,
                attempt_id: RunAttemptId::new(format!("run-attempt-{name}-{index}")),
                agent_id: AgentId::new(*agent),
                organization_id: organization_id.clone(),
                provider_instance_id: ProviderInstanceId::new(*provider),
                environment_id: EnvironmentId::new(environment),
                route,
            }
        })
        .collect::<Vec<_>>();
    store
        .insert_orchestration_run(
            &OrchestrationRunRecord {
                run,
                project_id,
                repository_id,
                settings_revision: 1,
            },
            &events,
        )
        .expect("insert ready run");
    fixtures
}

async fn enqueue_orchestrated(
    service: &ExecutionAdmissionService,
    fixture: &OrchestratedTaskFixture,
    agent_limit: u32,
    enqueued_at_ms: u64,
) -> pandamux_server::OrchestrationQueueRecord {
    service
        .enqueue_orchestrated(EnqueueOrchestratedAdmissionRequest {
            queue_id: format!("queue-{}", fixture.attempt_id.as_str()),
            run_id: fixture.run_id.clone(),
            task_id: fixture.task_id.clone(),
            attempt_id: fixture.attempt_id.clone(),
            agent_id: fixture.agent_id.clone(),
            organization_id: fixture.organization_id.clone(),
            agent_concurrency_limit: agent_limit,
            provider_instance_id: fixture.provider_instance_id.clone(),
            environment_id: fixture.environment_id.clone(),
            priority: 0,
            task_enqueued_at_ms: enqueued_at_ms,
            enqueued_at_ms,
            not_before_ms: enqueued_at_ms,
        })
        .await
        .expect("enqueue orchestrated admission")
}

fn transition(fixture: &OrchestratedTaskFixture, at_ms: u64) -> OrchestratedAdmissionTransition {
    OrchestratedAdmissionTransition {
        reservation: StoredBudgetReservation {
            run_id: fixture.run_id.clone(),
            agent_id: fixture.agent_id.clone(),
            organization_id: fixture.organization_id.clone(),
            agent_budget_limit: BudgetAmount {
                tokens: 1_000,
                cost_micro_usd: 1_000,
                runtime_ms: 10_000,
            },
            reservation: BudgetReservation {
                id: BudgetReservationId::new(format!("budget-{}", fixture.attempt_id.as_str())),
                task_id: fixture.task_id.clone(),
                attempt_id: fixture.attempt_id.clone(),
                amount: BudgetAmount {
                    tokens: 10,
                    cost_micro_usd: 20,
                    runtime_ms: 30,
                },
                actual: BudgetUsage::default(),
                ledger_utc_date: "1970-01-01".to_string(),
                state: BudgetReservationState::Reserved,
            },
            grants: BTreeMap::new(),
            reconciliation: None,
            created_at_ms: at_ms,
            updated_at_ms: at_ms,
        },
        attempt: Attempt {
            id: fixture.attempt_id.clone(),
            task_id: fixture.task_id.clone(),
            ordinal: 1,
            route: fixture.route.clone(),
            thread_id: None,
            turn_attempt_id: None,
            status: AttemptStatus::DispatchRequested,
            boundary: AttemptBoundary::SafeToRetry,
            started_at_ms: at_ms,
            ended_at_ms: None,
            failure: None,
            usage_reconciled: false,
            ambiguous_effect: false,
            ambiguity_reason: None,
            worktree_id: None,
            checkpoint_ids: BTreeSet::new(),
            cleanup_state: CleanupState::NotStarted,
        },
        daily_budget_limit_micro_usd: 1_000_000,
    }
}

async fn claim_orchestrated(
    service: &ExecutionAdmissionService,
    record: &pandamux_server::OrchestrationQueueRecord,
    fixture: &OrchestratedTaskFixture,
    at_ms: u64,
) -> Option<ExecutionAdmissionClaim> {
    service
        .try_claim_record(record, at_ms, Some(transition(fixture, at_ms)))
        .await
        .expect("claim orchestrated admission")
}

#[tokio::test]
async fn que_01_run_cap_is_independent() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 8, &[("env-a", 8)], &[("provider-a", 8)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let fixtures = insert_ready_run(
        &store,
        "run-cap",
        &["agent-a", "agent-b"],
        &["provider-a", "provider-a"],
        "env-a",
        1,
    );
    let first = enqueue_orchestrated(&service, &fixtures[0], 8, BASE_TIME + 10).await;
    let second = enqueue_orchestrated(&service, &fixtures[1], 8, BASE_TIME + 11).await;
    assert!(
        claim_orchestrated(&service, &first, &fixtures[0], BASE_TIME + 20)
            .await
            .is_some()
    );
    assert!(
        claim_orchestrated(&service, &second, &fixtures[1], BASE_TIME + 21)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn que_02_agent_cap_is_independent() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 8, &[("env-a", 8)], &[("provider-a", 8)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let first_fixture = insert_ready_run(
        &store,
        "agent-cap-a",
        &["agent-shared"],
        &["provider-a"],
        "env-a",
        2,
    )
    .remove(0);
    let second_fixture = insert_ready_run(
        &store,
        "agent-cap-b",
        &["agent-shared"],
        &["provider-a"],
        "env-a",
        2,
    )
    .remove(0);
    let first = enqueue_orchestrated(&service, &first_fixture, 1, BASE_TIME + 10).await;
    let second = enqueue_orchestrated(&service, &second_fixture, 1, BASE_TIME + 11).await;
    assert!(
        claim_orchestrated(&service, &first, &first_fixture, BASE_TIME + 20)
            .await
            .is_some()
    );
    assert!(
        claim_orchestrated(&service, &second, &second_fixture, BASE_TIME + 21)
            .await
            .is_none()
    );
}

async fn assert_second_manual_blocked(
    global_limit: u32,
    environments: &[(&str, u32)],
    providers: &[(&str, u32)],
    organization_limit: Option<u32>,
    first: (&str, &str, Option<&str>),
    second: (&str, &str, Option<&str>),
) {
    let store = Store::in_memory().expect("open store");
    configure(
        &store,
        global_limit,
        environments,
        providers,
        organization_limit,
    );
    let service = ExecutionAdmissionService::new(store);
    let first_record =
        enqueue_manual(&service, "first", first.0, first.1, first.2, BASE_TIME).await;
    let second_record = enqueue_manual(
        &service,
        "second",
        second.0,
        second.1,
        second.2,
        BASE_TIME + 1,
    )
    .await;
    assert!(
        claim_manual(&service, &first_record, BASE_TIME + 10)
            .await
            .is_some()
    );
    assert!(
        claim_manual(&service, &second_record, BASE_TIME + 11)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn que_03_provider_cap_is_independent() {
    assert_second_manual_blocked(
        8,
        &[("env-a", 8)],
        &[("provider-a", 1)],
        None,
        ("provider-a", "env-a", None),
        ("provider-a", "env-a", None),
    )
    .await;
}

#[tokio::test]
async fn que_04_environment_cap_is_independent() {
    assert_second_manual_blocked(
        8,
        &[("env-a", 1)],
        &[("provider-a", 8), ("provider-b", 8)],
        None,
        ("provider-a", "env-a", None),
        ("provider-b", "env-a", None),
    )
    .await;
}

#[tokio::test]
async fn que_05_organization_cap_includes_manual_admission() {
    assert_second_manual_blocked(
        8,
        &[("env-a", 8), ("env-b", 8)],
        &[("provider-a", 8), ("provider-b", 8)],
        Some(1),
        ("provider-a", "env-a", Some(ORG)),
        ("provider-b", "env-b", Some(ORG)),
    )
    .await;
}

#[tokio::test]
async fn que_06_global_cap_is_independent() {
    assert_second_manual_blocked(
        1,
        &[("env-a", 1), ("env-b", 1)],
        &[("provider-a", 8), ("provider-b", 8)],
        None,
        ("provider-a", "env-a", None),
        ("provider-b", "env-b", None),
    )
    .await;
}

#[tokio::test]
async fn que_07_separate_connections_do_not_overclaim() {
    let temp = tempfile::tempdir().expect("temporary database directory");
    let database = temp.path().join("admission.db");
    let store_a = Store::open(&database).expect("open first store");
    configure(&store_a, 1, &[("env-a", 1)], &[("provider-a", 2)], None);
    let store_b = Store::open(&database).expect("open second store");
    let service_a =
        ExecutionAdmissionService::with_owner(store_a, ServerIncarnationId::new("owner-a"));
    let service_b =
        ExecutionAdmissionService::with_owner(store_b, ServerIncarnationId::new("owner-b"));
    let first = enqueue_manual(&service_a, "race-a", "provider-a", "env-a", None, BASE_TIME).await;
    let second = enqueue_manual(
        &service_b,
        "race-b",
        "provider-a",
        "env-a",
        None,
        BASE_TIME + 1,
    )
    .await;
    let barrier = Arc::new(Barrier::new(3));
    let first_worker = {
        let service = service_a.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            claim_manual(&service, &first, BASE_TIME + 10).await
        })
    };
    let second_worker = {
        let service = service_b.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            claim_manual(&service, &second, BASE_TIME + 10).await
        })
    };
    barrier.wait().await;
    let claims = [
        first_worker.await.expect("join first claimant"),
        second_worker.await.expect("join second claimant"),
    ];
    assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
}

#[tokio::test]
async fn que_08_dynamic_cap_decrease_blocks_without_revoking() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 2, &[("env-a", 2)], &[("provider-a", 3)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let first = enqueue_manual(
        &service,
        "decrease-a",
        "provider-a",
        "env-a",
        None,
        BASE_TIME,
    )
    .await;
    let second = enqueue_manual(
        &service,
        "decrease-b",
        "provider-a",
        "env-a",
        None,
        BASE_TIME + 1,
    )
    .await;
    let first_claim = claim_manual(&service, &first, BASE_TIME + 10)
        .await
        .expect("first claim");
    let second_claim = claim_manual(&service, &second, BASE_TIME + 11)
        .await
        .expect("second claim");

    configure(&store, 1, &[("env-a", 1)], &[("provider-a", 3)], None);
    let reconciliation = service
        .reconcile_capacity()
        .await
        .expect("reconcile decreased capacity");
    assert_eq!(reconciliation.active_claims, 2);
    assert!(reconciliation.overages.iter().any(|overage| {
        overage.dimension == "global" && overage.active == 2 && overage.limit == 1
    }));
    assert!(
        store
            .list_execution_queue_records()
            .expect("list claims")
            .iter()
            .filter(|record| record.state == QueueState::Claimed)
            .count()
            == 2
    );
    let third = enqueue_manual(
        &service,
        "decrease-c",
        "provider-a",
        "env-a",
        None,
        BASE_TIME + 2,
    )
    .await;
    assert!(
        claim_manual(&service, &third, BASE_TIME + 12)
            .await
            .is_none()
    );
    service.release(&first_claim).await.expect("release first");
    service
        .release(&second_claim)
        .await
        .expect("release second");
}

#[tokio::test]
async fn que_09_orchestration_cannot_borrow_interactive_reserve() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 2, &[("env-a", 2)], &[("provider-a", 3)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let first_fixture = insert_ready_run(
        &store,
        "reserve-a",
        &["agent-a"],
        &["provider-a"],
        "env-a",
        2,
    )
    .remove(0);
    let second_fixture = insert_ready_run(
        &store,
        "reserve-b",
        &["agent-b"],
        &["provider-a"],
        "env-a",
        2,
    )
    .remove(0);
    let first = enqueue_orchestrated(&service, &first_fixture, 2, BASE_TIME + 10).await;
    let second = enqueue_orchestrated(&service, &second_fixture, 2, BASE_TIME + 11).await;
    assert!(
        claim_orchestrated(&service, &first, &first_fixture, BASE_TIME + 20)
            .await
            .is_some()
    );
    assert!(
        claim_orchestrated(&service, &second, &second_fixture, BASE_TIME + 21)
            .await
            .is_none()
    );
    let interactive = enqueue_manual(
        &service,
        "reserved-lane",
        "provider-a",
        "env-a",
        None,
        BASE_TIME + 12,
    )
    .await;
    assert!(
        claim_manual(&service, &interactive, BASE_TIME + 22)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn que_10_cap_one_admits_orchestration_after_four_interactive_claims() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 1, &[("env-a", 1)], &[("provider-a", 2)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let fixture = insert_ready_run(
        &store,
        "weighted",
        &["agent-a"],
        &["provider-a"],
        "env-a",
        1,
    )
    .remove(0);
    let orchestrated = enqueue_orchestrated(&service, &fixture, 1, BASE_TIME).await;
    let mut interactive = Vec::new();
    for index in 0..5 {
        interactive.push(
            enqueue_manual(
                &service,
                &format!("weighted-{index}"),
                "provider-a",
                "env-a",
                None,
                BASE_TIME + 1 + index,
            )
            .await,
        );
    }
    for record in interactive.iter().take(4) {
        let claim = claim_manual(&service, record, BASE_TIME + 20)
            .await
            .expect("interactive claim before weight boundary");
        service.release(&claim).await.expect("release interactive");
    }
    assert!(
        claim_manual(&service, &interactive[4], BASE_TIME + 21)
            .await
            .is_none()
    );
    assert!(
        claim_orchestrated(&service, &orchestrated, &fixture, BASE_TIME + 21)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn que_11_fifo_ties_use_run_task_attempt_identity() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 1, &[("env-a", 1)], &[("provider-a", 2)], None);
    let service = ExecutionAdmissionService::new(store);
    let later_identity =
        enqueue_manual(&service, "zulu", "provider-a", "env-a", None, BASE_TIME).await;
    let earlier_identity =
        enqueue_manual(&service, "alpha", "provider-a", "env-a", None, BASE_TIME).await;
    assert!(
        claim_manual(&service, &later_identity, BASE_TIME + 1)
            .await
            .is_none()
    );
    assert!(
        claim_manual(&service, &earlier_identity, BASE_TIME + 1)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn que_12_provider_blocked_head_does_not_block_eligible_work() {
    let store = Store::in_memory().expect("open store");
    configure(
        &store,
        3,
        &[("env-a", 3)],
        &[("provider-a", 1), ("provider-b", 2)],
        None,
    );
    let service = ExecutionAdmissionService::new(store);
    let occupying = enqueue_manual(
        &service,
        "occupying",
        "provider-a",
        "env-a",
        None,
        BASE_TIME,
    )
    .await;
    assert!(
        claim_manual(&service, &occupying, BASE_TIME + 1)
            .await
            .is_some()
    );
    let blocked_head = enqueue_manual(
        &service,
        "blocked-head",
        "provider-a",
        "env-a",
        None,
        BASE_TIME + 2,
    )
    .await;
    let eligible = enqueue_manual(
        &service,
        "eligible-tail",
        "provider-b",
        "env-a",
        None,
        BASE_TIME + 3,
    )
    .await;
    assert!(
        claim_manual(&service, &blocked_head, BASE_TIME + 4)
            .await
            .is_none()
    );
    assert!(
        claim_manual(&service, &eligible, BASE_TIME + 4)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn que_13_bounded_load_cannot_starve_old_orchestration() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 1, &[("env-a", 1)], &[("provider-a", 2)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let mut orchestrated = Vec::new();
    for round in 0..3 {
        let fixture = insert_ready_run(
            &store,
            &format!("starvation-{round}"),
            &["agent-a"],
            &["provider-a"],
            "env-a",
            1,
        )
        .remove(0);
        let record = enqueue_orchestrated(&service, &fixture, 1, BASE_TIME + round as u64).await;
        orchestrated.push((record, fixture));
    }
    let mut interactive = Vec::new();
    for index in 0..12 {
        interactive.push(
            enqueue_manual(
                &service,
                &format!("load-{index:02}"),
                "provider-a",
                "env-a",
                None,
                BASE_TIME + 10 + index,
            )
            .await,
        );
    }

    for round in 0..3 {
        for record in &interactive[(round * 4)..(round * 4 + 4)] {
            let claim = claim_manual(&service, record, BASE_TIME + 100 + round as u64)
                .await
                .expect("bounded interactive claim");
            service.release(&claim).await.expect("release interactive");
        }
        let (record, fixture) = &orchestrated[round];
        let claim = claim_orchestrated(&service, record, fixture, BASE_TIME + 100 + round as u64)
            .await
            .expect("orchestration admitted at weighted boundary");
        let dispatched = service
            .mark_dispatched(&claim)
            .await
            .expect("mark orchestration dispatched");
        service
            .release(&dispatched)
            .await
            .expect("settle orchestration capacity claim");
    }
}

#[tokio::test]
async fn que_14_manual_wins_race_against_orchestration_at_shared_cap() {
    let temp = tempfile::tempdir().expect("temporary database directory");
    let database = temp.path().join("admission.db");
    let store_a = Store::open(&database).expect("open first store");
    configure(&store_a, 1, &[("env-a", 1)], &[("provider-a", 2)], None);
    let fixture = insert_ready_run(
        &store_a,
        "manual-race",
        &["agent-a"],
        &["provider-a"],
        "env-a",
        1,
    )
    .remove(0);
    let store_b = Store::open(&database).expect("open second store");
    let orchestrator = ExecutionAdmissionService::with_owner(
        store_a,
        ServerIncarnationId::new("owner-orchestrator"),
    );
    let interactive = ExecutionAdmissionService::with_owner(
        store_b,
        ServerIncarnationId::new("owner-interactive"),
    );
    let orchestrated_record = enqueue_orchestrated(&orchestrator, &fixture, 1, BASE_TIME).await;
    let manual_record = enqueue_manual(
        &interactive,
        "manual-race",
        "provider-a",
        "env-a",
        None,
        BASE_TIME + 1,
    )
    .await;
    let barrier = Arc::new(Barrier::new(3));
    let orchestrated_worker = {
        let service = orchestrator.clone();
        let barrier = barrier.clone();
        let fixture = fixture.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            claim_orchestrated(&service, &orchestrated_record, &fixture, BASE_TIME + 10).await
        })
    };
    let manual_worker = {
        let service = interactive.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            claim_manual(&service, &manual_record, BASE_TIME + 10).await
        })
    };
    barrier.wait().await;
    let orchestrated_claim = orchestrated_worker.await.expect("join orchestrated worker");
    let manual_claim = manual_worker.await.expect("join manual worker");
    assert!(orchestrated_claim.is_none());
    assert!(manual_claim.is_some());
}

#[tokio::test]
async fn que_14_thread_manager_never_starts_provider_without_shared_admission() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 1, &[("env-a", 1)], &[("provider-a", 2)], None);
    let fixture = insert_ready_run(
        &store,
        "thread-manager",
        &["agent-a"],
        &["provider-a"],
        "env-a",
        1,
    )
    .remove(0);
    let orchestrator = ExecutionAdmissionService::new(store.clone());
    let record = enqueue_orchestrated(&orchestrator, &fixture, 1, BASE_TIME).await;
    assert!(
        claim_orchestrated(&orchestrator, &record, &fixture, BASE_TIME + 1)
            .await
            .is_some()
    );

    let drivers = Arc::new(DriverRegistry::new());
    let mock = Arc::new(MockProviderDriver::new(
        ProviderKind::Custom,
        "Admission Mock",
    ));
    drivers.register(ProviderKind::Custom, mock.clone());
    let router = Router::with_drivers(store.clone(), ServerRole::Hub, "env-a".to_string(), drivers);
    let workspace = tempfile::tempdir().expect("temporary thread workspace");
    let create = router
        .handle_request(RpcRequest::new(
            1,
            "thread.create",
            Some(json!({
                "title": "Admission blocked",
                "environmentId": "env-a",
                "providerInstanceId": "provider-a",
                "model": "fixture-model",
                "cwd": workspace.path().to_string_lossy(),
            })),
        ))
        .await
        .expect("thread create response");
    assert!(create.is_success(), "thread create failed: {create:?}");
    let thread_id = create.result.expect("thread create result")["id"]
        .as_str()
        .expect("thread id")
        .to_string();
    let response = router
        .handle_request(RpcRequest::new(
            2,
            "thread.send_turn",
            Some(json!({ "threadId": thread_id, "text": "must not start" })),
        ))
        .await
        .expect("thread send response");
    let error = response.error.expect("capacity must block ThreadManager");
    assert_eq!(error.code, -32023);
    assert_eq!(
        error.data.expect("typed capacity error")["reason"],
        "provider_capacity_reached"
    );
    assert_eq!(mock.session_start_count(), 0);
    let records = store
        .list_execution_queue_records()
        .expect("list shared queue");
    assert_eq!(
        records
            .iter()
            .filter(|record| record.state == QueueState::Claimed)
            .count(),
        1
    );
    assert!(
        records
            .iter()
            .any(|record| { record.interactive && record.state == QueueState::Cancelled })
    );
}

#[tokio::test]
async fn lease_renewal_owned_expiry_reclaim_and_foreign_refusal_are_exact() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 1, &[("env-a", 1)], &[("provider-a", 2)], None);
    let service = ExecutionAdmissionService::with_owner(
        store.clone(),
        ServerIncarnationId::new("owner-lease"),
    );
    let record = enqueue_manual(&service, "lease", "provider-a", "env-a", None, BASE_TIME).await;
    let claim = service
        .try_claim(ExecutionAdmissionClaimRequest {
            queue_id: record.queue_id.clone(),
            expected_candidate_sha256: record.candidate_sha256.clone(),
            expected_claim_revision: 0,
            lease_id: ExecutionLeaseId::new("lease-exact"),
            claimed_at_ms: BASE_TIME + 10,
            ttl_ms: 1_000,
            orchestration: None,
        })
        .await
        .expect("initial lease claim")
        .expect("claim acquired");
    let renewed = service
        .renew(&claim, BASE_TIME + 500, 1_000)
        .await
        .expect("renew owned lease");
    assert_eq!(renewed.record.claim_revision, 2);

    let foreign = AdmissionOwnership {
        owner_incarnation_id: ServerIncarnationId::new("owner-foreign"),
        ..renewed.ownership()
    };
    assert!(
        store
            .renew_execution_admission(&AdmissionRenewStoreRequest {
                ownership: foreign,
                renewed_at_ms: BASE_TIME + 600,
                lease_expires_at_ms: BASE_TIME + 1_600,
            })
            .is_err()
    );
    assert!(
        store
            .requeue_expired_execution_admission(&AdmissionRequeueStoreRequest {
                ownership: renewed.ownership(),
                reclaimed_at_ms: BASE_TIME + 1_499,
                not_before_ms: BASE_TIME + 2_000,
            })
            .is_err()
    );
    let requeued = service
        .requeue_expired(&renewed, BASE_TIME + 1_500, 3)
        .await
        .expect("requeue owned expired lease");
    assert_eq!(requeued.state, QueueState::Queued);
    assert_eq!(requeued.task_enqueued_at_ms, BASE_TIME);
    assert_eq!(
        requeued.not_before_ms,
        BASE_TIME + 1_500 + bounded_execution_retry_delay_ms(3)
    );
    assert!(
        claim_manual(&service, &requeued, requeued.not_before_ms - 1)
            .await
            .is_none()
    );
    assert!(
        claim_manual(&service, &requeued, requeued.not_before_ms)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn atomic_capacity_budget_dispatch_rolls_back_and_replays_exactly() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 2, &[("env-a", 2)], &[("provider-a", 2)], None);
    let service = ExecutionAdmissionService::with_owner(
        store.clone(),
        ServerIncarnationId::new("owner-atomic"),
    );
    let fixture =
        insert_ready_run(&store, "atomic", &["agent-a"], &["provider-a"], "env-a", 1).remove(0);
    let record = enqueue_orchestrated(&service, &fixture, 1, BASE_TIME + 1).await;
    let initial_events = store
        .load_orchestration_events(&fixture.run_id)
        .expect("load initial events")
        .len();
    let mut malformed = transition(&fixture, BASE_TIME + 10);
    malformed.reservation.agent_id = AgentId::new("changed-agent");
    let request = ExecutionAdmissionClaimRequest {
        queue_id: record.queue_id.clone(),
        expected_candidate_sha256: record.candidate_sha256.clone(),
        expected_claim_revision: 0,
        lease_id: ExecutionLeaseId::new("lease-atomic"),
        claimed_at_ms: BASE_TIME + 10,
        ttl_ms: 1_000,
        orchestration: Some(malformed),
    };
    assert!(service.try_claim(request).await.is_err());
    let rolled_back = store
        .load_orchestration_runtime_records(&fixture.run_id)
        .expect("load rolled back runtime");
    assert!(rolled_back.leases.is_empty());
    assert_eq!(rolled_back.queue_records[0].state, QueueState::Queued);
    assert!(rolled_back.budget_reservations.is_empty());
    assert_eq!(
        store
            .load_orchestration_events(&fixture.run_id)
            .expect("load rolled back events")
            .len(),
        initial_events
    );

    let valid_request = ExecutionAdmissionClaimRequest {
        queue_id: record.queue_id.clone(),
        expected_candidate_sha256: record.candidate_sha256.clone(),
        expected_claim_revision: 0,
        lease_id: ExecutionLeaseId::new("lease-atomic"),
        claimed_at_ms: BASE_TIME + 10,
        ttl_ms: 1_000,
        orchestration: Some(transition(&fixture, BASE_TIME + 10)),
    };
    let first = service
        .try_claim(valid_request.clone())
        .await
        .expect("commit atomic admission")
        .expect("atomic admission selected");
    assert!(first.acquired);
    let replay = service
        .try_claim(valid_request)
        .await
        .expect("replay atomic admission")
        .expect("replayed claim returned");
    assert!(!replay.acquired);
    assert_eq!(first.record, replay.record);
    assert_eq!(first.lease, replay.lease);

    let runtime = store
        .load_orchestration_runtime_records(&fixture.run_id)
        .expect("load committed runtime");
    assert_eq!(runtime.leases.len(), 1);
    assert_eq!(runtime.budget_reservations.len(), 1);
    assert_eq!(runtime.queue_records[0].state, QueueState::Claimed);
    assert_eq!(
        store
            .load_orchestration_events(&fixture.run_id)
            .expect("load committed events")
            .len(),
        initial_events + 2
    );
    assert_eq!(
        store
            .load_orchestration_effects(&fixture.run_id)
            .expect("load dispatch effect")
            .into_iter()
            .filter(|effect| matches!(
                effect.effect.kind,
                pandamux_core::OrchestrationEffectKind::DispatchTask { .. }
            ))
            .count(),
        1
    );

    let changed_replay = service
        .enqueue_orchestrated(EnqueueOrchestratedAdmissionRequest {
            queue_id: record.queue_id,
            run_id: fixture.run_id,
            task_id: fixture.task_id,
            attempt_id: fixture.attempt_id,
            agent_id: fixture.agent_id,
            organization_id: fixture.organization_id,
            agent_concurrency_limit: 1,
            provider_instance_id: fixture.provider_instance_id,
            environment_id: fixture.environment_id,
            priority: 1,
            task_enqueued_at_ms: BASE_TIME + 1,
            enqueued_at_ms: BASE_TIME + 1,
            not_before_ms: BASE_TIME + 1,
        })
        .await;
    assert!(changed_replay.is_err());
}

#[tokio::test]
async fn bud_04_compiled_agent_ceiling_is_checked_inside_the_claim_transaction() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 8, &[("env-a", 8)], &[("provider-a", 8)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let fixtures = insert_ready_run(
        &store,
        "budget-agent",
        &["agent-shared", "agent-shared"],
        &["provider-a", "provider-a"],
        "env-a",
        2,
    );
    let first = enqueue_orchestrated(&service, &fixtures[0], 2, BASE_TIME + 1).await;
    let second = enqueue_orchestrated(&service, &fixtures[1], 2, BASE_TIME + 2).await;
    let budget = BudgetAmount {
        tokens: 600,
        cost_micro_usd: 600,
        runtime_ms: 600,
    };
    let mut first_transition = transition(&fixtures[0], BASE_TIME + 10);
    first_transition.reservation.reservation.amount = budget;
    first_transition.reservation.agent_budget_limit = BudgetAmount {
        tokens: 1_000,
        cost_micro_usd: 1_000,
        runtime_ms: 1_000,
    };
    assert!(
        service
            .try_claim_record(&first, BASE_TIME + 10, Some(first_transition))
            .await
            .expect("first agent-budget claim")
            .is_some()
    );

    let mut second_transition = transition(&fixtures[1], BASE_TIME + 11);
    second_transition.reservation.reservation.amount = budget;
    second_transition.reservation.agent_budget_limit = BudgetAmount {
        tokens: 1_000,
        cost_micro_usd: 1_000,
        runtime_ms: 1_000,
    };
    let error = service
        .try_claim_record(&second, BASE_TIME + 11, Some(second_transition))
        .await
        .expect_err("second claim exceeds the compiled agent budget");
    assert!(error.to_string().contains("agent budget limit"));
    let runtime = store
        .load_orchestration_runtime_records(&fixtures[0].run_id)
        .expect("load agent-budget runtime");
    assert_eq!(runtime.budget_reservations.len(), 1);
    assert_eq!(runtime.queue_records[1].state, QueueState::Queued);
}

#[tokio::test]
async fn bud_05_daily_ceiling_rolls_back_capacity_and_budget_together() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 8, &[("env-a", 8)], &[("provider-a", 8)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let fixtures = insert_ready_run(
        &store,
        "budget-daily",
        &["agent-a", "agent-b"],
        &["provider-a", "provider-a"],
        "env-a",
        2,
    );
    let first = enqueue_orchestrated(&service, &fixtures[0], 2, BASE_TIME + 1).await;
    let second = enqueue_orchestrated(&service, &fixtures[1], 2, BASE_TIME + 2).await;
    let mut first_transition = transition(&fixtures[0], BASE_TIME + 10);
    first_transition
        .reservation
        .reservation
        .amount
        .cost_micro_usd = 600;
    first_transition.daily_budget_limit_micro_usd = 1_000;
    service
        .try_claim_record(&first, BASE_TIME + 10, Some(first_transition))
        .await
        .expect("first daily-budget claim")
        .expect("first daily-budget admission");

    let event_count = store
        .load_orchestration_events(&fixtures[0].run_id)
        .expect("load pre-failure events")
        .len();
    let mut second_transition = transition(&fixtures[1], BASE_TIME + 11);
    second_transition
        .reservation
        .reservation
        .amount
        .cost_micro_usd = 600;
    second_transition.daily_budget_limit_micro_usd = 1_000;
    let error = service
        .try_claim_record(&second, BASE_TIME + 11, Some(second_transition))
        .await
        .expect_err("second claim exceeds the UTC-day budget");
    assert!(error.to_string().contains("daily budget limit"));
    let runtime = store
        .load_orchestration_runtime_records(&fixtures[0].run_id)
        .expect("load daily-budget runtime");
    assert_eq!(runtime.budget_reservations.len(), 1);
    assert_eq!(runtime.daily_budgets[0].reserved_micro_usd, 600);
    assert_eq!(runtime.queue_records[1].state, QueueState::Queued);
    assert_eq!(
        store
            .load_orchestration_events(&fixtures[0].run_id)
            .expect("load post-failure events")
            .len(),
        event_count
    );
}

#[tokio::test]
async fn bud_07_midnight_claims_stay_on_their_fixed_utc_ledgers() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 8, &[("env-a", 8)], &[("provider-a", 8)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let fixtures = insert_ready_run(
        &store,
        "budget-midnight",
        &["agent-a", "agent-b"],
        &["provider-a", "provider-a"],
        "env-a",
        2,
    );
    let first = enqueue_orchestrated(&service, &fixtures[0], 2, BASE_TIME + 1).await;
    let second = enqueue_orchestrated(&service, &fixtures[1], 2, BASE_TIME + 2).await;
    let before_midnight = 86_399_999;
    let after_midnight = 86_400_000;
    let mut first_transition = transition(&fixtures[0], before_midnight);
    first_transition.reservation.reservation.ledger_utc_date = "1970-01-01".to_string();
    service
        .try_claim_record(&first, before_midnight, Some(first_transition))
        .await
        .expect("claim before UTC midnight")
        .expect("before-midnight admission");
    let mut second_transition = transition(&fixtures[1], after_midnight);
    second_transition.reservation.reservation.ledger_utc_date = "1970-01-02".to_string();
    service
        .try_claim_record(&second, after_midnight, Some(second_transition))
        .await
        .expect("claim after UTC midnight")
        .expect("after-midnight admission");
    let runtime = store
        .load_orchestration_runtime_records(&fixtures[0].run_id)
        .expect("load midnight runtime");
    assert_eq!(
        runtime
            .daily_budgets
            .iter()
            .map(|record| record.ledger_utc_date.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["1970-01-01", "1970-01-02"])
    );
}

#[tokio::test]
async fn budget_release_failpoints_rollback_or_resume_without_leaking_a_claim() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 8, &[("env-a", 8)], &[("provider-a", 8)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let fixture = insert_ready_run(
        &store,
        "budget-release-crash",
        &["agent-a"],
        &["provider-a"],
        "env-a",
        1,
    )
    .remove(0);
    let record = enqueue_orchestrated(&service, &fixture, 1, BASE_TIME + 1).await;
    let claim = claim_orchestrated(&service, &record, &fixture, BASE_TIME + 10)
        .await
        .expect("claim budget before release crash");

    assert!(
        service
            .release_at(
                &claim,
                BASE_TIME + 20,
                Some(BudgetExecutionFailpoint::BeforeReleaseCommit),
            )
            .await
            .expect_err("pre-commit release failpoint")
            .to_string()
            .contains("before_budget_release_commit")
    );
    let before = store
        .load_orchestration_runtime_records(&fixture.run_id)
        .expect("load rolled-back release");
    assert_eq!(before.queue_records[0].state, QueueState::Claimed);
    assert_eq!(
        before.budget_reservations[0].reservation.state,
        BudgetReservationState::Reserved
    );
    assert_eq!(before.daily_budgets[0].reserved_micro_usd, 20);

    assert!(
        service
            .release_at(
                &claim,
                BASE_TIME + 21,
                Some(BudgetExecutionFailpoint::AfterReleaseCommit),
            )
            .await
            .expect_err("post-commit release failpoint")
            .to_string()
            .contains("after_budget_release_commit")
    );
    let replay = service
        .release_at(&claim, BASE_TIME + 22, None)
        .await
        .expect("database-only release retry");
    assert_eq!(replay.state, QueueState::Cancelled);
    let after = store
        .load_orchestration_runtime_records(&fixture.run_id)
        .expect("load committed release");
    assert_eq!(
        after.budget_reservations[0].reservation.state,
        BudgetReservationState::Released
    );
    assert_eq!(after.daily_budgets[0].reserved_micro_usd, 0);
    assert!(after.leases.is_empty());
}

#[tokio::test]
async fn orchestrated_claim_cannot_dispatch_without_an_active_complete_reservation() {
    let store = Store::in_memory().expect("open store");
    configure(&store, 8, &[("env-a", 8)], &[("provider-a", 8)], None);
    let service = ExecutionAdmissionService::new(store.clone());
    let fixture = insert_ready_run(
        &store,
        "budget-required",
        &["agent-a"],
        &["provider-a"],
        "env-a",
        1,
    )
    .remove(0);
    let record = enqueue_orchestrated(&service, &fixture, 1, BASE_TIME + 1).await;
    let error = service
        .try_claim_record(&record, BASE_TIME + 10, None)
        .await
        .expect_err("orchestration claim omitted its budget transition");
    assert!(
        error
            .to_string()
            .contains("class does not match its transition")
    );
    let runtime = store
        .load_orchestration_runtime_records(&fixture.run_id)
        .expect("load refused admission");
    assert!(runtime.budget_reservations.is_empty());
    assert_eq!(runtime.queue_records[0].state, QueueState::Queued);
}
