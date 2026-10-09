use pandamux_core::{
    AccessMode, AgentAuthor, AgentBudget, AgentDefinition, AgentId, AgentProviderPolicy,
    AgentScope, AgentToolPolicy, BudgetAmount, DependencyEdge, DependencyEdgeId,
    DependencyRequirement, OrchestrationMode, OrganizationId, OutputContract, ProjectId, RunTaskId,
    RunTaskStatus,
};
use pandamux_orchestrator::{
    AgentFeasibilitySnapshot, ConfigurationIssueCode, PlanValidationContext, ProjectPolicySnapshot,
    ProposedDependencyEdgeV1, ProposedPlanV1, ProposedTaskInputV1, ProposedTaskV1, is_task_ready,
    validate_plan,
};
use std::collections::{BTreeMap, BTreeSet};

fn contract(kind: &str) -> OutputContract {
    OutputContract {
        kind: kind.to_string(),
        schema: None,
    }
}

fn proposed_task(id: &str) -> ProposedTaskV1 {
    ProposedTaskV1 {
        id: RunTaskId::new(id),
        summary: format!("Task {id}"),
        class: "build".to_string(),
        project_id: ProjectId::new("project-root"),
        required_capabilities: BTreeSet::from(["longContext".to_string()]),
        access_ceiling: AccessMode::ReadOnly,
        inputs: Vec::new(),
        outputs: vec![contract("patch")],
        selected_agent_id: None,
        max_attempts: 2,
        budget_limit: BudgetAmount {
            tokens: 100,
            cost_micro_usd: 200,
            runtime_ms: 300,
        },
    }
}

fn edge(
    id: &str,
    source: &str,
    target: &str,
    requirement: DependencyRequirement,
) -> ProposedDependencyEdgeV1 {
    ProposedDependencyEdgeV1 {
        id: DependencyEdgeId::new(id),
        source: RunTaskId::new(source),
        target: RunTaskId::new(target),
        requirement,
    }
}

fn agent(id: &str) -> AgentDefinition {
    AgentDefinition {
        id: AgentId::new(id),
        name: "Builder".to_string(),
        description: "Builds".to_string(),
        instructions: "Build".to_string(),
        scope: AgentScope::Team,
        author: AgentAuthor::InstructionsRepository,
        version: "1".to_string(),
        bundle_sha: "bundle".to_string(),
        source_path: "instructions:agents/builder.md".to_string(),
        organization_id: Some(OrganizationId::new("org-one")),
        project_id: None,
        specialties: BTreeSet::from(["build".to_string()]),
        tier: "reasoning".to_string(),
        effort: Some("high".to_string()),
        access_mode: AccessMode::AutoEdit,
        full_access_opt_in: false,
        required_capabilities: BTreeSet::new(),
        providers: AgentProviderPolicy::default(),
        environments: BTreeSet::from(["any".to_string()]),
        knowledge: BTreeSet::new(),
        tools: AgentToolPolicy::default(),
        output: contract("patch"),
        verification: Vec::new(),
        concurrency: 2,
        budget: AgentBudget::default(),
        enabled: true,
    }
}

fn context() -> PlanValidationContext {
    PlanValidationContext {
        run_organization_id: OrganizationId::new("org-one"),
        root_project_id: ProjectId::new("project-root"),
        projects: BTreeMap::from([
            (
                ProjectId::new("project-root"),
                ProjectPolicySnapshot {
                    organization_id: OrganizationId::new("org-one"),
                    orchestration_allowed: true,
                    cross_project_allowed: false,
                },
            ),
            (
                ProjectId::new("project-peer"),
                ProjectPolicySnapshot {
                    organization_id: OrganizationId::new("org-one"),
                    orchestration_allowed: true,
                    cross_project_allowed: true,
                },
            ),
            (
                ProjectId::new("project-foreign"),
                ProjectPolicySnapshot {
                    organization_id: OrganizationId::new("org-two"),
                    orchestration_allowed: true,
                    cross_project_allowed: true,
                },
            ),
        ]),
        agents: BTreeMap::from([(
            AgentId::new("builder"),
            AgentFeasibilitySnapshot {
                agent: agent("builder"),
                available_capabilities: BTreeSet::from(["longContext".to_string()]),
                route_available: true,
            },
        )]),
        budget_limit: BudgetAmount {
            tokens: 10_000,
            cost_micro_usd: 20_000,
            runtime_ms: 30_000,
        },
        max_tasks_per_run: 6,
        mode: OrchestrationMode::Auto,
        approvals_available: true,
        auto_access_ceiling: AccessMode::WorkspaceOnly,
    }
}

fn codes(
    result: Result<pandamux_core::ValidatedPlan, pandamux_orchestrator::DiagnosticSet>,
) -> Vec<ConfigurationIssueCode> {
    result
        .expect_err("plan must fail")
        .issues()
        .iter()
        .map(|issue| issue.code)
        .collect()
}

fn assert_code(
    result: Result<pandamux_core::ValidatedPlan, pandamux_orchestrator::DiagnosticSet>,
    expected: ConfigurationIssueCode,
) {
    let actual = codes(result);
    assert!(
        actual.contains(&expected),
        "expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn normalizes_maps_topology_readiness_and_digest_independent_of_input_order() {
    let mut consumer = proposed_task("task-b");
    consumer.inputs.push(ProposedTaskInputV1 {
        name: "change".to_string(),
        contract: contract("patch"),
        producer_task_id: Some(RunTaskId::new("task-a")),
    });
    let first = ProposedPlanV1 {
        schema_version: 1,
        tasks: vec![consumer.clone(), proposed_task("task-a")],
        edges: vec![edge(
            "edge-a-b",
            "task-a",
            "task-b",
            DependencyRequirement::RequiresSuccess,
        )],
    };
    let second = ProposedPlanV1 {
        schema_version: 1,
        tasks: vec![proposed_task("task-a"), consumer],
        edges: first.edges.clone(),
    };

    let first = validate_plan(&first, &context()).expect("first plan validates");
    let second = validate_plan(&second, &context()).expect("second plan validates");
    assert_eq!(first.source_digest, second.source_digest);
    assert_eq!(
        serde_json::to_vec(&first).expect("first serializes"),
        serde_json::to_vec(&second).expect("second serializes")
    );
    assert_eq!(
        first.topological_order,
        [RunTaskId::new("task-a"), RunTaskId::new("task-b")]
    );
    assert_eq!(
        first.tasks[&RunTaskId::new("task-a")].status,
        RunTaskStatus::Ready
    );
    assert_eq!(
        first.tasks[&RunTaskId::new("task-b")].status,
        RunTaskStatus::Blocked
    );
    assert!(
        first
            .tasks
            .values()
            .all(|task| task.organization_id == OrganizationId::new("org-one"))
    );
}

#[test]
fn enforces_task_edge_and_attempt_bounds_before_graph_validation() {
    let mut capped_context = context();
    capped_context.max_tasks_per_run = 64;
    let too_many_tasks = ProposedPlanV1 {
        schema_version: 1,
        tasks: (0..65)
            .map(|index| proposed_task(&format!("task-{index:02}")))
            .collect(),
        edges: Vec::new(),
    };
    assert_code(
        validate_plan(&too_many_tasks, &capped_context),
        ConfigurationIssueCode::PlanTaskLimit,
    );

    let too_many_edges = ProposedPlanV1 {
        schema_version: 1,
        tasks: vec![proposed_task("task-a"), proposed_task("task-b")],
        edges: (0..257)
            .map(|index| {
                edge(
                    &format!("edge-{index}"),
                    "task-a",
                    "task-b",
                    DependencyRequirement::RequiresSuccess,
                )
            })
            .collect(),
    };
    assert_code(
        validate_plan(&too_many_edges, &context()),
        ConfigurationIssueCode::PlanEdgeLimit,
    );

    let mut attempts = proposed_task("task-a");
    attempts.max_attempts = 9;
    assert_code(
        validate_plan(
            &ProposedPlanV1 {
                schema_version: 1,
                tasks: vec![attempts],
                edges: Vec::new(),
            },
            &context(),
        ),
        ConfigurationIssueCode::SchemaInvalidValue,
    );
}

#[test]
fn rejects_duplicate_missing_and_self_edges_in_fixed_order() {
    let duplicate_tasks = ProposedPlanV1 {
        schema_version: 1,
        tasks: vec![proposed_task("task-a"), proposed_task("task-a")],
        edges: Vec::new(),
    };
    assert_code(
        validate_plan(&duplicate_tasks, &context()),
        ConfigurationIssueCode::PlanDuplicateTask,
    );

    let duplicate_edges = ProposedPlanV1 {
        schema_version: 1,
        tasks: vec![proposed_task("task-a"), proposed_task("task-b")],
        edges: vec![
            edge(
                "edge-one",
                "task-a",
                "task-b",
                DependencyRequirement::RequiresSuccess,
            ),
            edge(
                "edge-two",
                "task-a",
                "task-b",
                DependencyRequirement::RequiresSettlement,
            ),
        ],
    };
    assert_code(
        validate_plan(&duplicate_edges, &context()),
        ConfigurationIssueCode::PlanDuplicateEdge,
    );

    let missing = ProposedPlanV1 {
        schema_version: 1,
        tasks: vec![proposed_task("task-a")],
        edges: vec![edge(
            "edge-missing",
            "task-a",
            "task-missing",
            DependencyRequirement::RequiresSuccess,
        )],
    };
    assert_code(
        validate_plan(&missing, &context()),
        ConfigurationIssueCode::PlanMissingTask,
    );

    let self_edge = ProposedPlanV1 {
        schema_version: 1,
        tasks: vec![proposed_task("task-a")],
        edges: vec![edge(
            "edge-self",
            "task-a",
            "task-a",
            DependencyRequirement::RequiresSuccess,
        )],
    };
    assert_code(
        validate_plan(&self_edge, &context()),
        ConfigurationIssueCode::PlanSelfEdge,
    );
}

#[test]
fn reports_a_stable_cycle_witness_across_edge_permutations() {
    let edges = vec![
        edge(
            "edge-a-b",
            "task-a",
            "task-b",
            DependencyRequirement::RequiresSuccess,
        ),
        edge(
            "edge-b-c",
            "task-b",
            "task-c",
            DependencyRequirement::RequiresSuccess,
        ),
        edge(
            "edge-c-a",
            "task-c",
            "task-a",
            DependencyRequirement::RequiresSuccess,
        ),
    ];
    let plan = |edges| ProposedPlanV1 {
        schema_version: 1,
        tasks: vec![
            proposed_task("task-c"),
            proposed_task("task-a"),
            proposed_task("task-b"),
        ],
        edges,
    };
    let first = validate_plan(&plan(edges.clone()), &context()).expect_err("cycle fails");
    let second = validate_plan(&plan(edges.into_iter().rev().collect()), &context())
        .expect_err("cycle fails");
    assert_eq!(first, second);
    assert_eq!(first.issues()[0].code, ConfigurationIssueCode::PlanCycle);
    assert!(
        first.issues()[0]
            .explanation
            .as_str()
            .contains("task-a -> task-b -> task-c -> task-a")
    );
}

#[test]
fn permits_same_organization_cross_project_only_with_explicit_permission() {
    let mut peer_task = proposed_task("task-peer");
    peer_task.project_id = ProjectId::new("project-peer");
    let plan = ProposedPlanV1 {
        schema_version: 1,
        tasks: vec![peer_task.clone()],
        edges: Vec::new(),
    };
    validate_plan(&plan, &context()).expect("allowed peer project validates");

    let mut denied_context = context();
    denied_context
        .projects
        .get_mut(&ProjectId::new("project-peer"))
        .expect("peer")
        .cross_project_allowed = false;
    assert_code(
        validate_plan(&plan, &denied_context),
        ConfigurationIssueCode::PlanProjectDenied,
    );

    peer_task.project_id = ProjectId::new("project-foreign");
    assert_code(
        validate_plan(
            &ProposedPlanV1 {
                schema_version: 1,
                tasks: vec![peer_task],
                edges: Vec::new(),
            },
            &context(),
        ),
        ConfigurationIssueCode::PlanCrossOrganization,
    );
}

#[test]
fn requires_one_compatible_producer_and_success_edge() {
    let mut consumer = proposed_task("task-b");
    consumer.inputs = vec![ProposedTaskInputV1 {
        name: "patch".to_string(),
        contract: contract("patch"),
        producer_task_id: Some(RunTaskId::new("task-a")),
    }];
    let plan = |producer: ProposedTaskV1, requirement| ProposedPlanV1 {
        schema_version: 1,
        tasks: vec![producer, consumer.clone()],
        edges: vec![edge("edge-a-b", "task-a", "task-b", requirement)],
    };
    validate_plan(
        &plan(
            proposed_task("task-a"),
            DependencyRequirement::RequiresSuccess,
        ),
        &context(),
    )
    .expect("one compatible producer validates");

    assert_code(
        validate_plan(
            &plan(
                proposed_task("task-a"),
                DependencyRequirement::RequiresSettlement,
            ),
            &context(),
        ),
        ConfigurationIssueCode::PlanOutputIncompatible,
    );

    let mut ambiguous = proposed_task("task-a");
    ambiguous.outputs.push(contract("patch"));
    assert_code(
        validate_plan(
            &plan(ambiguous, DependencyRequirement::RequiresSuccess),
            &context(),
        ),
        ConfigurationIssueCode::PlanOutputIncompatible,
    );
}

#[test]
fn rejects_missing_agents_incompatible_outputs_and_impossible_capabilities() {
    let mut selected = proposed_task("task-a");
    selected.selected_agent_id = Some(AgentId::new("missing"));
    assert_code(
        validate_plan(
            &ProposedPlanV1 {
                schema_version: 1,
                tasks: vec![selected],
                edges: Vec::new(),
            },
            &context(),
        ),
        ConfigurationIssueCode::PlanAgentNotFound,
    );

    let mut wrong_output = proposed_task("task-a");
    wrong_output.selected_agent_id = Some(AgentId::new("builder"));
    wrong_output.outputs = vec![contract("report")];
    assert_code(
        validate_plan(
            &ProposedPlanV1 {
                schema_version: 1,
                tasks: vec![wrong_output],
                edges: Vec::new(),
            },
            &context(),
        ),
        ConfigurationIssueCode::PlanOutputIncompatible,
    );

    let mut impossible = proposed_task("task-a");
    impossible
        .required_capabilities
        .insert("images".to_string());
    assert_code(
        validate_plan(
            &ProposedPlanV1 {
                schema_version: 1,
                tasks: vec![impossible],
                edges: Vec::new(),
            },
            &context(),
        ),
        ConfigurationIssueCode::PlanCapabilityImpossible,
    );

    let mut unavailable_context = context();
    unavailable_context
        .agents
        .get_mut(&AgentId::new("builder"))
        .expect("builder")
        .route_available = false;
    assert_code(
        validate_plan(
            &ProposedPlanV1 {
                schema_version: 1,
                tasks: vec![proposed_task("task-a")],
                edges: Vec::new(),
            },
            &unavailable_context,
        ),
        ConfigurationIssueCode::PlanCapabilityImpossible,
    );
}

#[test]
fn checks_each_budget_dimension_for_overflow_and_limit() {
    let mut unlimited = context();
    unlimited.budget_limit = BudgetAmount {
        tokens: u64::MAX,
        cost_micro_usd: u64::MAX,
        runtime_ms: u64::MAX,
    };
    for dimension in ["tokens", "cost", "runtime"] {
        let mut first = proposed_task("task-a");
        let mut second = proposed_task("task-b");
        match dimension {
            "tokens" => {
                first.budget_limit.tokens = u64::MAX;
                second.budget_limit.tokens = 1;
            }
            "cost" => {
                first.budget_limit.cost_micro_usd = u64::MAX;
                second.budget_limit.cost_micro_usd = 1;
            }
            "runtime" => {
                first.budget_limit.runtime_ms = u64::MAX;
                second.budget_limit.runtime_ms = 1;
            }
            _ => unreachable!(),
        }
        assert_code(
            validate_plan(
                &ProposedPlanV1 {
                    schema_version: 1,
                    tasks: vec![first, second],
                    edges: Vec::new(),
                },
                &unlimited,
            ),
            ConfigurationIssueCode::PlanBudgetOverflow,
        );
    }

    for dimension in ["tokens", "cost", "runtime"] {
        let mut task = proposed_task("task-a");
        match dimension {
            "tokens" => task.budget_limit.tokens = 10_001,
            "cost" => task.budget_limit.cost_micro_usd = 20_001,
            "runtime" => task.budget_limit.runtime_ms = 30_001,
            _ => unreachable!(),
        }
        assert_code(
            validate_plan(
                &ProposedPlanV1 {
                    schema_version: 1,
                    tasks: vec![task],
                    edges: Vec::new(),
                },
                &context(),
            ),
            ConfigurationIssueCode::PlanBudgetExceeded,
        );
    }
}

#[test]
fn approval_policy_and_shared_readiness_fail_closed() {
    let mut no_approvals = context();
    no_approvals.mode = OrchestrationMode::ApproveFirst;
    no_approvals.approvals_available = false;
    assert_code(
        validate_plan(
            &ProposedPlanV1 {
                schema_version: 1,
                tasks: vec![proposed_task("task-a")],
                edges: Vec::new(),
            },
            &no_approvals,
        ),
        ConfigurationIssueCode::PlanApprovalImpossible,
    );

    let validated = validate_plan(
        &ProposedPlanV1 {
            schema_version: 1,
            tasks: vec![proposed_task("task-a"), proposed_task("task-b")],
            edges: vec![edge(
                "edge-a-b",
                "task-a",
                "task-b",
                DependencyRequirement::RequiresSuccess,
            )],
        },
        &context(),
    )
    .expect("valid graph");
    let mut tasks = validated.tasks.clone();
    let dependent = tasks[&RunTaskId::new("task-b")].clone();
    assert!(!is_task_ready(&dependent, &tasks, &validated.edges));
    tasks
        .get_mut(&RunTaskId::new("task-a"))
        .expect("source")
        .status = RunTaskStatus::Succeeded;
    assert!(is_task_ready(&dependent, &tasks, &validated.edges));

    tasks
        .get_mut(&RunTaskId::new("task-a"))
        .expect("source")
        .status = RunTaskStatus::Failed;
    assert!(!is_task_ready(&dependent, &tasks, &validated.edges));
    let settlement_edges = BTreeMap::from([(
        DependencyEdgeId::new("edge-a-b"),
        DependencyEdge {
            id: DependencyEdgeId::new("edge-a-b"),
            source: RunTaskId::new("task-a"),
            target: RunTaskId::new("task-b"),
            requirement: DependencyRequirement::RequiresSettlement,
        },
    )]);
    assert!(is_task_ready(&dependent, &tasks, &settlement_edges));
}
