use pandamux_core::{
    AccessMode, AgentDefinition, AgentEvaluation, AgentId, AgentScope, AgentSelectionScore,
    AgentTask, AttemptBoundary, CompiledClassRule, EnvironmentId, OrganizationId, OutputContract,
    RoutingCandidate, RoutingCapabilityRequirements, RoutingDecision, TaskRouteDecision,
    TierMapping, select_route,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type ValidatedAgentTask = AgentTask;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCandidateSnapshot {
    pub environment_id: EnvironmentId,
    pub organization_id: OrganizationId,
    pub declared_capabilities: BTreeSet<String>,
    pub candidate: RoutingCandidate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingContext {
    pub environment_id: EnvironmentId,
    pub environment_kind: String,
    pub organization_id: OrganizationId,
    pub agents: BTreeMap<AgentId, AgentDefinition>,
    pub classes: Vec<CompiledClassRule>,
    pub tier_mapping: TierMapping,
    pub provider_candidates: Vec<ProviderCandidateSnapshot>,
    pub active_agent_instances: BTreeMap<AgentId, u32>,
    pub min_remaining_millionths: u32,
    pub provider_concurrency: BTreeMap<String, u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingFailure {
    pub code: String,
    pub agent_evaluations: Vec<AgentEvaluation>,
    #[serde(default)]
    pub provider_evaluations: BTreeMap<AgentId, RoutingDecision>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackEligibility {
    Allowed,
    AttemptsExhausted,
    UnsafeBoundary,
    AmbiguousEffect,
}

pub fn route_task(
    task: &ValidatedAgentTask,
    context: &RoutingContext,
) -> Result<TaskRouteDecision, RoutingFailure> {
    if let Some(recorded) = &task.route {
        return Ok(recorded.clone());
    }

    let mut provider_decisions = BTreeMap::new();
    let mut evaluations = context
        .agents
        .values()
        .map(|agent| evaluate_agent(task, agent, context, &mut provider_decisions))
        .collect::<Vec<_>>();
    evaluations.sort_by(|left, right| left.agent_id.cmp(&right.agent_id));

    let selected = evaluations
        .iter()
        .filter(|evaluation| evaluation.eligible)
        .max_by(|left, right| {
            left.score
                .as_ref()
                .map(|score| score.total)
                .cmp(&right.score.as_ref().map(|score| score.total))
                .then_with(|| right.agent_id.cmp(&left.agent_id))
        })
        .map(|evaluation| evaluation.agent_id.clone());
    let Some(selected_agent_id) = selected else {
        return Err(RoutingFailure {
            code: "no_eligible_agent".to_string(),
            agent_evaluations: evaluations,
            provider_evaluations: provider_decisions,
        });
    };
    let provider = provider_decisions
        .remove(&selected_agent_id)
        .expect("eligible agent has a provider decision");
    if provider.selected_provider_instance_id.is_none() {
        return Err(RoutingFailure {
            code: "no_eligible_provider".to_string(),
            agent_evaluations: evaluations,
            provider_evaluations: provider_decisions,
        });
    }
    Ok(TaskRouteDecision {
        task_id: task.id.clone(),
        selected_agent_id: Some(selected_agent_id),
        agent_evaluations: evaluations,
        provider,
    })
}

pub fn fallback_eligibility(
    boundary: AttemptBoundary,
    ambiguous_effect: bool,
    completed_attempts: u32,
    max_attempts: u32,
) -> FallbackEligibility {
    if completed_attempts >= max_attempts {
        FallbackEligibility::AttemptsExhausted
    } else if ambiguous_effect {
        FallbackEligibility::AmbiguousEffect
    } else if boundary != AttemptBoundary::SafeToRetry {
        FallbackEligibility::UnsafeBoundary
    } else {
        FallbackEligibility::Allowed
    }
}

fn evaluate_agent(
    task: &ValidatedAgentTask,
    agent: &AgentDefinition,
    context: &RoutingContext,
    provider_decisions: &mut BTreeMap<AgentId, RoutingDecision>,
) -> AgentEvaluation {
    let mut rejection_codes = Vec::new();
    if !agent.enabled {
        rejection_codes.push("agent_disabled".to_string());
    }
    if agent.organization_id.as_ref() != Some(&context.organization_id)
        || task.organization_id != context.organization_id
    {
        rejection_codes.push("organization_mismatch".to_string());
    }
    if !agent_scope_matches(agent, task) {
        rejection_codes.push("project_scope_mismatch".to_string());
    }
    if agent.access_mode.security_rank() < task.access_ceiling.security_rank() {
        rejection_codes.push("agent_access_ceiling".to_string());
    }
    if !agent_environment_matches(agent, context) {
        rejection_codes.push("environment_mismatch".to_string());
    }
    if !task.outputs.is_empty()
        && task
            .outputs
            .iter()
            .any(|output| !contracts_compatible(output, &agent.output))
    {
        rejection_codes.push("output_contract_mismatch".to_string());
    }
    let active = context
        .active_agent_instances
        .get(&agent.id)
        .copied()
        .unwrap_or_default();
    if agent.concurrency == 0 || active >= agent.concurrency {
        rejection_codes.push("agent_concurrency_limit".to_string());
    }
    if task
        .selected_agent_id
        .as_ref()
        .is_some_and(|selected| selected != &agent.id)
    {
        rejection_codes.push("validated_plan_selected_other_agent".to_string());
    }
    if !context.tier_mapping.tiers.contains_key(&agent.tier) {
        rejection_codes.push("tier_unmapped".to_string());
    }

    let provider = route_provider(task, agent, context);
    if provider.selected_provider_instance_id.is_none() {
        rejection_codes.push("no_eligible_provider".to_string());
    }
    provider_decisions.insert(agent.id.clone(), provider);
    rejection_codes.sort();
    rejection_codes.dedup();
    if !rejection_codes.is_empty() {
        return AgentEvaluation {
            agent_id: agent.id.clone(),
            eligible: false,
            rejection_codes,
            explanation_codes: Vec::new(),
            score: None,
        };
    }

    let (class_rule, mut explanation_codes) = class_rule_score(task, agent, &context.classes);
    let specialty = if agent.specialties.contains(&task.class) {
        explanation_codes.push("specialty_exact".to_string());
        1_000
    } else {
        0
    };
    let provider_preference = provider_preference_score(agent);
    if provider_preference > 0 {
        explanation_codes.push("provider_preference".to_string());
    }
    if task.selected_agent_id.as_ref() == Some(&agent.id) {
        explanation_codes.push("validated_plan_selection".to_string());
    }
    explanation_codes.sort();
    explanation_codes.dedup();
    let total = class_rule + specialty + provider_preference;
    AgentEvaluation {
        agent_id: agent.id.clone(),
        eligible: true,
        rejection_codes,
        explanation_codes,
        score: Some(AgentSelectionScore {
            class_rule,
            specialty,
            provider_preference,
            total,
        }),
    }
}

fn route_provider(
    task: &ValidatedAgentTask,
    agent: &AgentDefinition,
    context: &RoutingContext,
) -> RoutingDecision {
    let required_capabilities = task
        .required_capabilities
        .union(&agent.required_capabilities)
        .cloned()
        .collect::<BTreeSet<_>>();
    let requirements = core_requirements(&required_capabilities, task.access_ceiling);
    let candidates = context
        .provider_candidates
        .iter()
        .map(|snapshot| adapt_provider_candidate(snapshot, agent, context, &required_capabilities))
        .collect::<Vec<_>>();
    select_route(requirements, candidates)
}

fn adapt_provider_candidate(
    snapshot: &ProviderCandidateSnapshot,
    agent: &AgentDefinition,
    context: &RoutingContext,
    required_capabilities: &BTreeSet<String>,
) -> RoutingCandidate {
    let mut candidate = snapshot.candidate.clone();
    if snapshot.environment_id != context.environment_id {
        candidate
            .local_hard_blocks
            .push("environment_identity_mismatch".to_string());
    }
    if snapshot.organization_id != context.organization_id {
        candidate
            .local_hard_blocks
            .push("organization_mismatch".to_string());
    }
    for capability in required_capabilities {
        if !declared_capability_supported(capability, snapshot) {
            candidate
                .local_hard_blocks
                .push(format!("missing_capability:{capability}"));
        }
    }
    if candidate
        .remaining_millionths
        .is_none_or(|remaining| remaining < context.min_remaining_millionths)
    {
        candidate
            .local_hard_blocks
            .push("below_minimum_remaining".to_string());
    }
    if let Some(limit) = context
        .provider_concurrency
        .get(candidate.provider_kind.as_str())
    {
        candidate.concurrency_limit = candidate.concurrency_limit.min(*limit);
    }
    if agent.providers.avoid.contains(&candidate.provider_kind) {
        candidate.user_preference /= 2;
    }
    if let Some(position) = agent
        .providers
        .prefer
        .iter()
        .position(|provider| provider == &candidate.provider_kind)
    {
        let bonus = u16::try_from(1_000usize.saturating_sub(position)).unwrap_or_default();
        candidate.user_preference = candidate.user_preference.saturating_add(bonus);
    }
    candidate.local_hard_blocks.sort();
    candidate.local_hard_blocks.dedup();
    candidate
}

fn declared_capability_supported(capability: &str, snapshot: &ProviderCandidateSnapshot) -> bool {
    let capabilities = snapshot.candidate.capabilities;
    match capability {
        "images" => capabilities.supports_images,
        "reasoning" => capabilities.supports_reasoning,
        "approvals" => capabilities.supports_approvals,
        "resume" => capabilities.supports_resume,
        "rateLimits" => capabilities.supports_rate_limits,
        "subAgents" => capabilities.supports_sub_agents,
        "toolCalls" | "toolFiltering" => capabilities.supports_tool_calls,
        "instructions" | "instructionInjection" => capabilities.supports_system_prompt,
        other => snapshot.declared_capabilities.contains(other),
    }
}

fn core_requirements(
    capabilities: &BTreeSet<String>,
    access_mode: AccessMode,
) -> RoutingCapabilityRequirements {
    RoutingCapabilityRequirements {
        requires_instructions: capabilities.contains("instructions")
            || capabilities.contains("instructionInjection"),
        requires_tool_calls: capabilities.contains("toolCalls")
            || capabilities.contains("toolFiltering"),
        requires_approvals: capabilities.contains("approvals"),
        requires_reasoning: capabilities.contains("reasoning"),
        access_mode,
    }
}

fn agent_scope_matches(agent: &AgentDefinition, task: &ValidatedAgentTask) -> bool {
    match agent.scope {
        AgentScope::Project | AgentScope::Personal => {
            agent.project_id.as_ref() == Some(&task.project_id)
        }
        AgentScope::Team | AgentScope::BuiltIn => true,
    }
}

fn agent_environment_matches(agent: &AgentDefinition, context: &RoutingContext) -> bool {
    agent.environments.contains("any")
        || agent.environments.contains(&context.environment_kind)
        || agent.environments.contains(context.environment_id.as_str())
}

fn contracts_compatible(left: &OutputContract, right: &OutputContract) -> bool {
    left.kind == right.kind && left.schema == right.schema
}

fn class_rule_score(
    task: &ValidatedAgentTask,
    agent: &AgentDefinition,
    classes: &[CompiledClassRule],
) -> (i64, Vec<String>) {
    let Some(class) = classes.iter().find(|class| class.id == task.class) else {
        return (0, Vec::new());
    };
    let Some(position) = class
        .agents
        .iter()
        .position(|candidate| candidate == &agent.id)
    else {
        return (0, Vec::new());
    };
    let position = i64::try_from(position).unwrap_or(i64::MAX);
    (
        100_000_i64.saturating_sub(position),
        vec![format!("class_rule:{}", class.id)],
    )
}

fn provider_preference_score(agent: &AgentDefinition) -> i64 {
    if agent.providers.prefer.is_empty() {
        0
    } else {
        100
    }
}
