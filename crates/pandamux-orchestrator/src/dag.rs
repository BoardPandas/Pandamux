use crate::diagnostics::{
    ConfigurationIssue, ConfigurationIssueCode, ConfigurationStage, DiagnosticSet,
    RelativeConfigPath, issue,
};
use crate::policy::canonical_json_bytes;
use pandamux_core::{
    AccessMode, AgentDefinition, AgentId, AgentScope, AgentTask, BudgetAmount, DependencyEdge,
    DependencyEdgeId, DependencyRequirement, OrchestrationMode, OrganizationId, OutputContract,
    ProjectId, RunTaskId, RunTaskStatus, TaskInput, ValidatedPlan,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PROPOSED_PLAN_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_MAX_PLAN_TASKS: usize = 6;
pub const HARD_MAX_PLAN_TASKS: usize = 64;
pub const HARD_MAX_PLAN_EDGES: usize = 256;
pub const HARD_MAX_TASK_ATTEMPTS: u32 = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedPlanV1 {
    pub schema_version: u32,
    #[serde(default)]
    pub tasks: Vec<ProposedTaskV1>,
    #[serde(default)]
    pub edges: Vec<ProposedDependencyEdgeV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedTaskV1 {
    pub id: RunTaskId,
    pub summary: String,
    pub class: String,
    pub project_id: ProjectId,
    #[serde(default)]
    pub required_capabilities: BTreeSet<String>,
    pub access_ceiling: AccessMode,
    #[serde(default)]
    pub inputs: Vec<ProposedTaskInputV1>,
    #[serde(default)]
    pub outputs: Vec<OutputContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_agent_id: Option<AgentId>,
    pub max_attempts: u32,
    pub budget_limit: BudgetAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedTaskInputV1 {
    pub name: String,
    pub contract: OutputContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_task_id: Option<RunTaskId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedDependencyEdgeV1 {
    pub id: DependencyEdgeId,
    pub source: RunTaskId,
    pub target: RunTaskId,
    pub requirement: DependencyRequirement,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectPolicySnapshot {
    pub organization_id: OrganizationId,
    pub orchestration_allowed: bool,
    pub cross_project_allowed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentFeasibilitySnapshot {
    pub agent: AgentDefinition,
    pub available_capabilities: BTreeSet<String>,
    pub route_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanValidationContext {
    pub run_organization_id: OrganizationId,
    pub root_project_id: ProjectId,
    pub projects: BTreeMap<ProjectId, ProjectPolicySnapshot>,
    pub agents: BTreeMap<AgentId, AgentFeasibilitySnapshot>,
    pub budget_limit: BudgetAmount,
    pub max_tasks_per_run: usize,
    pub mode: OrchestrationMode,
    pub approvals_available: bool,
    pub auto_access_ceiling: AccessMode,
}

struct NormalizedGraph {
    tasks: BTreeMap<RunTaskId, AgentTask>,
    edges: BTreeMap<DependencyEdgeId, DependencyEdge>,
}

pub fn validate_plan(
    proposal: &ProposedPlanV1,
    context: &PlanValidationContext,
) -> Result<ValidatedPlan, DiagnosticSet> {
    validate_bounds(proposal, context)?;

    let normalized = normalize_unique_records(proposal)?;
    let mut tasks = normalized.tasks;
    let edges = normalized.edges;
    validate_edge_endpoints(proposal, &tasks)?;
    let topological_order = stable_topological_order(&tasks, &edges)?;
    validate_project_permissions(&tasks, context)?;
    validate_organizations(&mut tasks, context)?;
    validate_agent_and_output_references(&tasks, context)?;
    validate_producer_compatibility(&tasks, &edges)?;
    let total_budget = validate_budget(&tasks, context.budget_limit)?;
    validate_capability_feasibility(&tasks, context)?;
    validate_approval_feasibility(&tasks, context)?;

    let readiness_snapshot = tasks.clone();
    for task in tasks.values_mut() {
        task.status = if is_task_ready(task, &readiness_snapshot, &edges) {
            RunTaskStatus::Ready
        } else {
            RunTaskStatus::Blocked
        };
    }

    let mut plan = ValidatedPlan {
        schema_version: PROPOSED_PLAN_SCHEMA_VERSION,
        source_digest: String::new(),
        tasks,
        edges,
        topological_order,
        total_budget,
    };
    plan.source_digest = validated_plan_digest(&plan);
    Ok(plan)
}

/// The single dependency readiness predicate used by validation and replay state.
pub fn is_task_ready(
    task: &AgentTask,
    tasks: &BTreeMap<RunTaskId, AgentTask>,
    edges: &BTreeMap<DependencyEdgeId, DependencyEdge>,
) -> bool {
    if !matches!(
        task.status,
        RunTaskStatus::Pending | RunTaskStatus::Blocked | RunTaskStatus::Ready
    ) {
        return false;
    }
    edges
        .values()
        .filter(|edge| edge.target == task.id)
        .all(|edge| {
            let Some(source) = tasks.get(&edge.source) else {
                return false;
            };
            match edge.requirement {
                DependencyRequirement::RequiresSuccess => source.status == RunTaskStatus::Succeeded,
                DependencyRequirement::RequiresSettlement => source.status.is_terminal(),
            }
        })
}

fn validate_bounds(
    proposal: &ProposedPlanV1,
    context: &PlanValidationContext,
) -> Result<(), DiagnosticSet> {
    let mut issues = Vec::new();
    if proposal.schema_version != PROPOSED_PLAN_SCHEMA_VERSION {
        issues.push(plan_issue(
            ConfigurationIssueCode::SchemaVersionUnsupported,
            Some("/schemaVersion"),
            "The proposed plan schema version is unsupported",
            "Use proposed plan schemaVersion 1",
        ));
    }
    let configured_limit = context.max_tasks_per_run.min(HARD_MAX_PLAN_TASKS);
    if proposal.tasks.is_empty() || proposal.tasks.len() > configured_limit {
        issues.push(plan_issue(
            ConfigurationIssueCode::PlanTaskLimit,
            Some("/tasks"),
            "The plan has no tasks or exceeds the trusted per-run task limit",
            "Provide at least one task and stay within the configured limit and hard maximum of 64",
        ));
    }
    if proposal.edges.len() > HARD_MAX_PLAN_EDGES {
        issues.push(plan_issue(
            ConfigurationIssueCode::PlanEdgeLimit,
            Some("/edges"),
            "The plan exceeds the hard maximum of 256 dependency edges",
            "Reduce the plan to 256 dependency edges or fewer",
        ));
    }
    for (index, task) in proposal.tasks.iter().enumerate() {
        if task.max_attempts == 0 || task.max_attempts > HARD_MAX_TASK_ATTEMPTS {
            issues.push(plan_issue(
                ConfigurationIssueCode::SchemaInvalidValue,
                Some(&format!("/tasks/{index}/maxAttempts")),
                "A task attempt limit is outside the supported range of 1 through 8",
                "Set maxAttempts to a value from 1 through 8",
            ));
        }
    }
    finish(issues)
}

fn normalize_unique_records(proposal: &ProposedPlanV1) -> Result<NormalizedGraph, DiagnosticSet> {
    let mut issues = Vec::new();
    let mut tasks = BTreeMap::new();
    for proposed in &proposal.tasks {
        let normalized = AgentTask {
            id: proposed.id.clone(),
            summary: proposed.summary.clone(),
            class: proposed.class.clone(),
            project_id: proposed.project_id.clone(),
            organization_id: OrganizationId::new("<unresolved>"),
            required_capabilities: proposed.required_capabilities.clone(),
            access_ceiling: proposed.access_ceiling,
            inputs: proposed
                .inputs
                .iter()
                .map(|input| TaskInput {
                    name: input.name.clone(),
                    contract: input.contract.clone(),
                    producer_task_id: input.producer_task_id.clone(),
                    blackboard_entry_id: None,
                })
                .collect(),
            outputs: proposed.outputs.clone(),
            selected_agent_id: proposed.selected_agent_id.clone(),
            route: None,
            status: RunTaskStatus::Pending,
            max_attempts: proposed.max_attempts,
            approval_ids: BTreeSet::new(),
            budget_limit: proposed.budget_limit,
            budget_reservation_id: None,
        };
        if tasks.insert(proposed.id.clone(), normalized).is_some() {
            issues.push(plan_issue(
                ConfigurationIssueCode::PlanDuplicateTask,
                Some("/tasks"),
                "The plan reuses a task id",
                "Assign a unique server-supplied id to every task",
            ));
        }
    }

    let mut edges = BTreeMap::new();
    let mut relationships = BTreeSet::new();
    for proposed in &proposal.edges {
        let normalized = DependencyEdge {
            id: proposed.id.clone(),
            source: proposed.source.clone(),
            target: proposed.target.clone(),
            requirement: proposed.requirement,
        };
        let duplicate_id = edges.insert(proposed.id.clone(), normalized).is_some();
        let duplicate_relationship =
            !relationships.insert((proposed.source.clone(), proposed.target.clone()));
        if duplicate_id || duplicate_relationship {
            issues.push(plan_issue(
                ConfigurationIssueCode::PlanDuplicateEdge,
                Some("/edges"),
                "The plan reuses an edge id or dependency relationship",
                "Keep exactly one uniquely identified edge between two tasks",
            ));
        }
    }
    finish(issues)?;
    Ok(NormalizedGraph { tasks, edges })
}

fn validate_edge_endpoints(
    proposal: &ProposedPlanV1,
    tasks: &BTreeMap<RunTaskId, AgentTask>,
) -> Result<(), DiagnosticSet> {
    let mut issues = Vec::new();
    for edge in &proposal.edges {
        if edge.source == edge.target {
            issues.push(plan_issue(
                ConfigurationIssueCode::PlanSelfEdge,
                Some("/edges"),
                "A task cannot depend on itself",
                "Remove the self-edge or point it at a distinct task",
            ));
        }
        if !tasks.contains_key(&edge.source) || !tasks.contains_key(&edge.target) {
            issues.push(plan_issue(
                ConfigurationIssueCode::PlanMissingTask,
                Some("/edges"),
                "A dependency edge references a task that is not in the plan",
                "Add the referenced task or remove the edge",
            ));
        }
    }
    for task in &proposal.tasks {
        for input in &task.inputs {
            if input
                .producer_task_id
                .as_ref()
                .is_some_and(|producer| !tasks.contains_key(producer))
            {
                issues.push(plan_issue(
                    ConfigurationIssueCode::PlanMissingTask,
                    Some("/tasks/inputs/producerTaskId"),
                    "A typed input references a producer that is not in the plan",
                    "Add the producer task or remove the producer reference",
                ));
            }
        }
    }
    finish(issues)
}

fn stable_topological_order(
    tasks: &BTreeMap<RunTaskId, AgentTask>,
    edges: &BTreeMap<DependencyEdgeId, DependencyEdge>,
) -> Result<Vec<RunTaskId>, DiagnosticSet> {
    let mut indegree = tasks
        .keys()
        .cloned()
        .map(|id| (id, 0usize))
        .collect::<BTreeMap<_, _>>();
    let mut outgoing = tasks
        .keys()
        .cloned()
        .map(|id| (id, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for edge in edges.values() {
        *indegree.get_mut(&edge.target).expect("endpoints validated") += 1;
        outgoing
            .get_mut(&edge.source)
            .expect("endpoints validated")
            .insert(edge.target.clone());
    }
    let mut ready = indegree
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(tasks.len());
    while let Some(next) = ready.pop_first() {
        order.push(next.clone());
        for target in outgoing.get(&next).expect("task adjacency exists") {
            let count = indegree.get_mut(target).expect("target indegree exists");
            *count -= 1;
            if *count == 0 {
                ready.insert(target.clone());
            }
        }
    }
    if order.len() == tasks.len() {
        return Ok(order);
    }

    let remaining = indegree
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();
    let witness = deterministic_cycle_witness(&outgoing, &remaining)
        .into_iter()
        .map(|id| id.as_str().to_string())
        .collect::<Vec<_>>()
        .join(" -> ");
    Err(DiagnosticSet::one(plan_issue(
        ConfigurationIssueCode::PlanCycle,
        Some("/edges"),
        &format!("The dependency graph contains the stable cycle witness: {witness}"),
        "Remove at least one edge from the reported cycle",
    )))
}

fn deterministic_cycle_witness(
    outgoing: &BTreeMap<RunTaskId, BTreeSet<RunTaskId>>,
    remaining: &BTreeSet<RunTaskId>,
) -> Vec<RunTaskId> {
    let mut colors = BTreeMap::<RunTaskId, u8>::new();
    let mut stack = Vec::new();
    for start in remaining {
        if colors.get(start).copied().unwrap_or_default() == 0
            && let Some(witness) = visit_cycle(start, outgoing, remaining, &mut colors, &mut stack)
        {
            return witness;
        }
    }
    Vec::new()
}

fn visit_cycle(
    current: &RunTaskId,
    outgoing: &BTreeMap<RunTaskId, BTreeSet<RunTaskId>>,
    remaining: &BTreeSet<RunTaskId>,
    colors: &mut BTreeMap<RunTaskId, u8>,
    stack: &mut Vec<RunTaskId>,
) -> Option<Vec<RunTaskId>> {
    colors.insert(current.clone(), 1);
    stack.push(current.clone());
    for next in outgoing.get(current).into_iter().flatten() {
        if !remaining.contains(next) {
            continue;
        }
        match colors.get(next).copied().unwrap_or_default() {
            0 => {
                if let Some(witness) = visit_cycle(next, outgoing, remaining, colors, stack) {
                    return Some(witness);
                }
            }
            1 => {
                let start = stack.iter().position(|item| item == next).unwrap_or(0);
                let mut witness = stack[start..].to_vec();
                witness.push(next.clone());
                return Some(witness);
            }
            _ => {}
        }
    }
    stack.pop();
    colors.insert(current.clone(), 2);
    None
}

fn validate_project_permissions(
    tasks: &BTreeMap<RunTaskId, AgentTask>,
    context: &PlanValidationContext,
) -> Result<(), DiagnosticSet> {
    let mut issues = Vec::new();
    for task in tasks.values() {
        let Some(project) = context.projects.get(&task.project_id) else {
            issues.push(plan_issue(
                ConfigurationIssueCode::PlanProjectDenied,
                Some("/tasks/projectId"),
                "A task references a project absent from the trusted snapshot",
                "Choose a project present in the server-supplied policy snapshot",
            ));
            continue;
        };
        if !project.orchestration_allowed
            || (task.project_id != context.root_project_id && !project.cross_project_allowed)
        {
            issues.push(plan_issue(
                ConfigurationIssueCode::PlanProjectDenied,
                Some("/tasks/projectId"),
                "Trusted project policy does not allow this orchestration task",
                "Remove the task or obtain explicit same-organization cross-project permission",
            ));
        }
    }
    finish(issues)
}

fn validate_organizations(
    tasks: &mut BTreeMap<RunTaskId, AgentTask>,
    context: &PlanValidationContext,
) -> Result<(), DiagnosticSet> {
    let mut issues = Vec::new();
    for task in tasks.values_mut() {
        let project_organization = context
            .projects
            .get(&task.project_id)
            .map(|project| &project.organization_id);
        if project_organization != Some(&context.run_organization_id) {
            issues.push(plan_issue(
                ConfigurationIssueCode::PlanCrossOrganization,
                Some("/tasks/projectId"),
                "A task project belongs to a different organization",
                "Keep every task within the run organization",
            ));
        } else {
            task.organization_id = context.run_organization_id.clone();
        }
    }
    finish(issues)
}

fn validate_agent_and_output_references(
    tasks: &BTreeMap<RunTaskId, AgentTask>,
    context: &PlanValidationContext,
) -> Result<(), DiagnosticSet> {
    let mut issues = Vec::new();
    for task in tasks.values() {
        if task.summary.trim().is_empty() || task.class.trim().is_empty() {
            issues.push(plan_issue(
                ConfigurationIssueCode::SchemaInvalidValue,
                Some("/tasks"),
                "Task summary and class must be nonempty",
                "Provide a bounded safe summary and class id",
            ));
        }
        for output in &task.outputs {
            if output.kind.trim().is_empty() {
                issues.push(plan_issue(
                    ConfigurationIssueCode::PlanOutputIncompatible,
                    Some("/tasks/outputs"),
                    "A task output contract has an empty kind",
                    "Name the typed output contract",
                ));
            }
        }
        for input in &task.inputs {
            if input.name.trim().is_empty() || input.contract.kind.trim().is_empty() {
                issues.push(plan_issue(
                    ConfigurationIssueCode::PlanOutputIncompatible,
                    Some("/tasks/inputs"),
                    "A task input name or contract kind is empty",
                    "Name every typed input and output contract",
                ));
            }
        }
        if let Some(agent_id) = &task.selected_agent_id {
            let Some(snapshot) = context.agents.get(agent_id) else {
                issues.push(plan_issue(
                    ConfigurationIssueCode::PlanAgentNotFound,
                    Some("/tasks/selectedAgentId"),
                    "A selected agent is absent from the trusted bundle snapshot",
                    "Select a compiled agent or leave routing undecided",
                ));
                continue;
            };
            if !task.outputs.is_empty()
                && task
                    .outputs
                    .iter()
                    .any(|output| !contracts_compatible(output, &snapshot.agent.output))
            {
                issues.push(plan_issue(
                    ConfigurationIssueCode::PlanOutputIncompatible,
                    Some("/tasks/outputs"),
                    "The selected agent cannot produce the requested output contract",
                    "Select a compatible agent or change the output contract",
                ));
            }
        }
    }
    finish(issues)
}

fn validate_producer_compatibility(
    tasks: &BTreeMap<RunTaskId, AgentTask>,
    edges: &BTreeMap<DependencyEdgeId, DependencyEdge>,
) -> Result<(), DiagnosticSet> {
    let mut issues = Vec::new();
    for consumer in tasks.values() {
        for input in &consumer.inputs {
            let Some(producer_id) = &input.producer_task_id else {
                continue;
            };
            let producer = tasks
                .get(producer_id)
                .expect("producer existence validated");
            let compatible_outputs = producer
                .outputs
                .iter()
                .filter(|output| contracts_compatible(output, &input.contract))
                .count();
            let success_edges = edges
                .values()
                .filter(|edge| {
                    edge.source == *producer_id
                        && edge.target == consumer.id
                        && edge.requirement == DependencyRequirement::RequiresSuccess
                })
                .count();
            if compatible_outputs != 1 || success_edges != 1 {
                issues.push(plan_issue(
                    ConfigurationIssueCode::PlanOutputIncompatible,
                    Some("/tasks/inputs"),
                    "Typed output consumption requires one compatible producer output and one success dependency",
                    "Make the producer contract unambiguous and add exactly one RequiresSuccess edge",
                ));
            }
        }
    }
    finish(issues)
}

fn contracts_compatible(left: &OutputContract, right: &OutputContract) -> bool {
    left.kind == right.kind && left.schema == right.schema
}

fn validate_budget(
    tasks: &BTreeMap<RunTaskId, AgentTask>,
    limit: BudgetAmount,
) -> Result<BudgetAmount, DiagnosticSet> {
    let mut total = BudgetAmount::default();
    for task in tasks.values() {
        let Some(tokens) = total.tokens.checked_add(task.budget_limit.tokens) else {
            return Err(budget_overflow_issue());
        };
        let Some(cost_micro_usd) = total
            .cost_micro_usd
            .checked_add(task.budget_limit.cost_micro_usd)
        else {
            return Err(budget_overflow_issue());
        };
        let Some(runtime_ms) = total.runtime_ms.checked_add(task.budget_limit.runtime_ms) else {
            return Err(budget_overflow_issue());
        };
        total = BudgetAmount {
            tokens,
            cost_micro_usd,
            runtime_ms,
        };
    }
    if total.tokens > limit.tokens
        || total.cost_micro_usd > limit.cost_micro_usd
        || total.runtime_ms > limit.runtime_ms
    {
        return Err(DiagnosticSet::one(plan_issue(
            ConfigurationIssueCode::PlanBudgetExceeded,
            Some("/tasks/budgetLimit"),
            "The checked plan budget exceeds the trusted run limit",
            "Reduce task budgets or obtain a higher trusted run limit",
        )));
    }
    Ok(total)
}

fn budget_overflow_issue() -> DiagnosticSet {
    DiagnosticSet::one(plan_issue(
        ConfigurationIssueCode::PlanBudgetOverflow,
        Some("/tasks/budgetLimit"),
        "Summing task budgets overflowed an unsigned 64-bit integer",
        "Reduce task budget values before validation",
    ))
}

fn validate_capability_feasibility(
    tasks: &BTreeMap<RunTaskId, AgentTask>,
    context: &PlanValidationContext,
) -> Result<(), DiagnosticSet> {
    let mut issues = Vec::new();
    for task in tasks.values() {
        let candidates = match &task.selected_agent_id {
            Some(id) => context.agents.get(id).into_iter().collect::<Vec<_>>(),
            None => context.agents.values().collect::<Vec<_>>(),
        };
        if !candidates
            .into_iter()
            .any(|snapshot| agent_is_feasible(task, snapshot, context))
        {
            issues.push(plan_issue(
                ConfigurationIssueCode::PlanCapabilityImpossible,
                Some("/tasks/requiredCapabilities"),
                "No trusted agent and route can satisfy the task's hard requirements",
                "Reduce the requirements or restore a compatible enabled agent and provider route",
            ));
        }
    }
    finish(issues)
}

fn agent_is_feasible(
    task: &AgentTask,
    snapshot: &AgentFeasibilitySnapshot,
    context: &PlanValidationContext,
) -> bool {
    let agent = &snapshot.agent;
    if !agent.enabled
        || !snapshot.route_available
        || agent.organization_id.as_ref() != Some(&context.run_organization_id)
        || agent.access_mode.security_rank() < task.access_ceiling.security_rank()
        || !snapshot
            .available_capabilities
            .is_superset(&task.required_capabilities)
        || !snapshot
            .available_capabilities
            .is_superset(&agent.required_capabilities)
        || (!task.outputs.is_empty()
            && task
                .outputs
                .iter()
                .any(|output| !contracts_compatible(output, &agent.output)))
    {
        return false;
    }
    match agent.scope {
        AgentScope::Project | AgentScope::Personal => {
            agent.project_id.as_ref() == Some(&task.project_id)
        }
        AgentScope::Team | AgentScope::BuiltIn => true,
    }
}

fn validate_approval_feasibility(
    tasks: &BTreeMap<RunTaskId, AgentTask>,
    context: &PlanValidationContext,
) -> Result<(), DiagnosticSet> {
    let requires_approval = context.mode == OrchestrationMode::ApproveFirst
        || tasks.values().any(|task| {
            task.access_ceiling.security_rank() > context.auto_access_ceiling.security_rank()
        });
    if requires_approval && !context.approvals_available {
        return Err(DiagnosticSet::one(plan_issue(
            ConfigurationIssueCode::PlanApprovalImpossible,
            Some("/tasks/accessCeiling"),
            "Trusted policy requires approval but no approval channel is available",
            "Enable approvals or reduce the plan to the trusted automatic access ceiling",
        )));
    }
    Ok(())
}

fn validated_plan_digest(plan: &ValidatedPlan) -> String {
    let mut unsigned = plan.clone();
    unsigned.source_digest.clear();
    let bytes = canonical_json_bytes(&unsigned);
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn plan_issue(
    code: ConfigurationIssueCode,
    field: Option<&str>,
    explanation: &str,
    action: &str,
) -> ConfigurationIssue {
    issue(
        code,
        RelativeConfigPath::new("plan.json").expect("static path is valid"),
        field,
        ConfigurationStage::Plan,
        explanation,
        action,
        None,
    )
}

fn finish(issues: Vec<ConfigurationIssue>) -> Result<(), DiagnosticSet> {
    if issues.is_empty() {
        Ok(())
    } else {
        Err(DiagnosticSet::from_issues(issues))
    }
}
