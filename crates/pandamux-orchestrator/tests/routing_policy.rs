use pandamux_core::{
    AccessMode, AgentAuthor, AgentBudget, AgentDefinition, AgentId, AgentProviderPolicy,
    AgentScope, AgentToolPolicy, ApprovalDecisionProvenance, ApprovalId, ApprovalRequirement,
    AttemptBoundary, BudgetAmount, CapacityConfidence, CapacityFreshness, CompiledClassRule,
    EnvironmentId, ModelTarget, OrganizationId, OutputContract, ProjectId, ProviderCapabilities,
    ProviderInstanceId, ProviderKind, RoutingAuthentication, RoutingCandidate, RoutingHealth,
    RunApprovalKind, RunApprovalStatus, RunTaskId, RunTaskStatus, SubscriptionKey, TierMapping,
};
use pandamux_orchestrator::{
    ApprovalAction, ApprovalReconciliation, ApprovalSafetyClass, FallbackEligibility,
    ProviderCandidateSnapshot, RoutingContext, SafeText, canonical_action_digest,
    fallback_eligibility, reconcile_approval, route_task,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn contract(kind: &str) -> OutputContract {
    OutputContract {
        kind: kind.to_string(),
        schema: None,
    }
}

fn agent(id: &str) -> AgentDefinition {
    AgentDefinition {
        id: AgentId::new(id),
        name: id.to_string(),
        description: "Routes deterministically".to_string(),
        instructions: "Do the task".to_string(),
        scope: AgentScope::Team,
        author: AgentAuthor::InstructionsRepository,
        version: "1".to_string(),
        bundle_sha: "bundle".to_string(),
        source_path: format!("instructions:agents/{id}.md"),
        organization_id: Some(OrganizationId::new("org-one")),
        project_id: None,
        specialties: BTreeSet::new(),
        tier: "reasoning".to_string(),
        effort: Some("high".to_string()),
        access_mode: AccessMode::AutoEdit,
        full_access_opt_in: false,
        required_capabilities: BTreeSet::from(["longContext".to_string()]),
        providers: AgentProviderPolicy::default(),
        environments: BTreeSet::from(["local".to_string()]),
        knowledge: BTreeSet::new(),
        tools: AgentToolPolicy::default(),
        output: contract("patch"),
        verification: Vec::new(),
        concurrency: 2,
        budget: AgentBudget::default(),
        enabled: true,
    }
}

fn task() -> pandamux_core::AgentTask {
    pandamux_core::AgentTask {
        id: RunTaskId::new("task-one"),
        summary: "Build the change".to_string(),
        class: "build".to_string(),
        project_id: ProjectId::new("project-one"),
        organization_id: OrganizationId::new("org-one"),
        required_capabilities: BTreeSet::new(),
        access_ceiling: AccessMode::WorkspaceOnly,
        inputs: Vec::new(),
        outputs: vec![contract("patch")],
        required_verification: BTreeSet::new(),
        selected_agent_id: None,
        route: None,
        status: RunTaskStatus::Ready,
        max_attempts: 2,
        approval_ids: BTreeSet::new(),
        budget_limit: BudgetAmount::default(),
        budget_reservation_id: None,
    }
}

fn capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        supports_streaming: true,
        supports_tool_calls: true,
        supports_sub_agents: true,
        supports_system_prompt: true,
        supports_reasoning: true,
        supports_approvals: true,
        supports_rate_limits: true,
        supports_resume: true,
        supports_images: true,
        max_access_mode: AccessMode::AutoEdit,
    }
}

fn provider(id: &str) -> ProviderCandidateSnapshot {
    ProviderCandidateSnapshot {
        environment_id: EnvironmentId::new("env-one"),
        organization_id: OrganizationId::new("org-one"),
        declared_capabilities: BTreeSet::from(["longContext".to_string()]),
        candidate: RoutingCandidate {
            provider_instance_id: ProviderInstanceId::new(id),
            subscription_key: SubscriptionKey::new(format!("subscription-{id}")),
            provider_kind: ProviderKind::Codex,
            model: "model".to_string(),
            capabilities: capabilities(),
            policy_allowed: true,
            subscription_allowed: true,
            model_compatible: true,
            installed: true,
            authentication: RoutingAuthentication::Authenticated,
            health: RoutingHealth::Healthy,
            capacity_confidence: CapacityConfidence::Reported,
            capacity_freshness: CapacityFreshness::Fresh,
            remaining_millionths: Some(500_000),
            hard_limited: false,
            active_concurrency: 0,
            concurrency_limit: 4,
            local: true,
            user_preference: 0,
            local_hard_blocks: Vec::new(),
        },
    }
}

fn context(
    agents: Vec<AgentDefinition>,
    providers: Vec<ProviderCandidateSnapshot>,
) -> RoutingContext {
    RoutingContext {
        environment_id: EnvironmentId::new("env-one"),
        environment_kind: "local".to_string(),
        organization_id: OrganizationId::new("org-one"),
        agents: agents
            .into_iter()
            .map(|agent| (agent.id.clone(), agent))
            .collect(),
        classes: Vec::new(),
        tier_mapping: TierMapping {
            tiers: BTreeMap::from([(
                "reasoning".to_string(),
                ModelTarget {
                    provider_instance_id: ProviderInstanceId::new("provider-good"),
                    model: "model".to_string(),
                    effort: Some("high".to_string()),
                },
            )]),
        },
        provider_candidates: providers,
        active_agent_instances: BTreeMap::new(),
        min_remaining_millionths: 100_000,
        provider_concurrency: BTreeMap::from([("codex".to_string(), 4)]),
    }
}

#[test]
fn hard_filters_every_agent_before_scoring_and_records_codes() {
    let good = agent("agent-good");
    let mut disabled = agent("agent-disabled");
    disabled.enabled = false;
    let mut wrong_org = agent("agent-org");
    wrong_org.organization_id = Some(OrganizationId::new("org-two"));
    let mut wrong_project = agent("agent-project");
    wrong_project.scope = AgentScope::Project;
    wrong_project.project_id = Some(ProjectId::new("project-two"));
    let mut access = agent("agent-access");
    access.access_mode = AccessMode::ReadOnly;
    let mut environment = agent("agent-environment");
    environment.environments = BTreeSet::from(["remote".to_string()]);
    let mut output = agent("agent-output");
    output.output = contract("report");
    let concurrency = agent("agent-concurrency");
    let mut tier = agent("agent-tier");
    tier.tier = "missing".to_string();

    let mut context = context(
        vec![
            good,
            disabled,
            wrong_org,
            wrong_project,
            access,
            environment,
            output,
            concurrency.clone(),
            tier,
        ],
        vec![provider("provider-good")],
    );
    context
        .active_agent_instances
        .insert(concurrency.id.clone(), concurrency.concurrency);
    let decision = route_task(&task(), &context).expect("good agent routes");
    assert_eq!(decision.selected_agent_id, Some(AgentId::new("agent-good")));
    let expected = [
        ("agent-access", "agent_access_ceiling"),
        ("agent-concurrency", "agent_concurrency_limit"),
        ("agent-disabled", "agent_disabled"),
        ("agent-environment", "environment_mismatch"),
        ("agent-org", "organization_mismatch"),
        ("agent-output", "output_contract_mismatch"),
        ("agent-project", "project_scope_mismatch"),
        ("agent-tier", "tier_unmapped"),
    ];
    for (agent_id, code) in expected {
        let evaluation = decision
            .agent_evaluations
            .iter()
            .find(|evaluation| evaluation.agent_id.as_str() == agent_id)
            .expect("evaluation exists");
        assert!(!evaluation.eligible);
        assert!(evaluation.score.is_none());
        assert!(evaluation.rejection_codes.contains(&code.to_string()));
    }
}

#[test]
fn existing_provider_router_applies_every_hard_filter_before_score() {
    let mut candidates = Vec::new();
    let mut add = |id: &str, mutate: &dyn Fn(&mut ProviderCandidateSnapshot)| {
        let mut candidate = provider(id);
        mutate(&mut candidate);
        candidates.push(candidate);
    };
    add("policy", &|value| value.candidate.policy_allowed = false);
    add("subscription", &|value| {
        value.candidate.subscription_allowed = false
    });
    add("model", &|value| value.candidate.model_compatible = false);
    add("installed", &|value| value.candidate.installed = false);
    add("auth", &|value| {
        value.candidate.authentication = RoutingAuthentication::Unauthenticated
    });
    add("health", &|value| {
        value.candidate.health = RoutingHealth::Unavailable {
            reason: "offline".to_string(),
        }
    });
    add("capacity", &|value| value.candidate.hard_limited = true);
    add("concurrency", &|value| {
        value.candidate.active_concurrency = value.candidate.concurrency_limit
    });
    add("environment", &|value| {
        value.environment_id = EnvironmentId::new("env-two")
    });
    add("organization", &|value| {
        value.organization_id = OrganizationId::new("org-two")
    });
    add("minimum", &|value| {
        value.candidate.remaining_millionths = Some(1)
    });
    add("capability", &|value| {
        value.declared_capabilities.clear();
    });
    candidates.push(provider("provider-good"));

    let decision = route_task(&task(), &context(vec![agent("builder")], candidates))
        .expect("one provider remains");
    assert_eq!(
        decision.provider.selected_provider_instance_id,
        Some(ProviderInstanceId::new("provider-good"))
    );
    assert_eq!(decision.provider.evaluations.len(), 13);
    for evaluation in &decision.provider.evaluations {
        if evaluation.provider_instance_id.as_str() == "provider-good" {
            assert!(evaluation.eligible);
            assert!(evaluation.score.is_some());
        } else {
            assert!(!evaluation.eligible, "{}", evaluation.provider_instance_id);
            assert!(evaluation.score.is_none());
            assert!(!evaluation.rejection_codes.is_empty());
        }
    }
}

#[test]
fn class_rules_beat_specialties_and_ties_use_stable_agent_ids() {
    let mut specialist = agent("agent-alpha");
    specialist.specialties.insert("build".to_string());
    let explicit = agent("agent-zulu");
    let mut routing = context(
        vec![specialist.clone(), explicit.clone()],
        vec![provider("provider-good")],
    );
    routing.classes = vec![CompiledClassRule {
        id: "build".to_string(),
        agents: vec![explicit.id.clone()],
        match_keywords: BTreeSet::new(),
        max_files_hint: None,
        decompose: false,
        require_approval: false,
        review_agent: None,
        review_when: None,
    }];
    assert_eq!(
        route_task(&task(), &routing)
            .expect("class routes")
            .selected_agent_id,
        Some(explicit.id)
    );

    routing.classes.clear();
    specialist.specialties.clear();
    routing.agents.insert(specialist.id.clone(), specialist);
    assert_eq!(
        route_task(&task(), &routing)
            .expect("tie routes")
            .selected_agent_id,
        Some(AgentId::new("agent-alpha"))
    );
}

#[test]
fn route_and_explanations_are_stable_under_input_permutations() {
    let first = context(
        vec![agent("agent-bravo"), agent("agent-alpha")],
        vec![provider("provider-bravo"), provider("provider-alpha")],
    );
    let second = context(
        vec![agent("agent-alpha"), agent("agent-bravo")],
        vec![provider("provider-alpha"), provider("provider-bravo")],
    );
    assert_eq!(
        route_task(&task(), &first).expect("first routes"),
        route_task(&task(), &second).expect("second routes")
    );
}

#[test]
fn recorded_route_is_immutable_when_capacity_changes() {
    let original = route_task(
        &task(),
        &context(vec![agent("builder")], vec![provider("provider-original")]),
    )
    .expect("original route");
    let mut routed_task = task();
    routed_task.route = Some(original.clone());
    routed_task.selected_agent_id = original.selected_agent_id.clone();

    let mut replacement = provider("provider-replacement");
    replacement.candidate.remaining_millionths = Some(1_000_000);
    let rerouted = route_task(
        &routed_task,
        &context(vec![agent("builder")], vec![replacement]),
    )
    .expect("recorded route is returned");
    assert_eq!(rerouted, original);
}

#[test]
fn one_task_one_node_still_runs_the_complete_pipeline() {
    let decision = route_task(
        &task(),
        &context(vec![agent("builder")], vec![provider("provider-only")]),
    )
    .expect("single candidate routes");
    assert_eq!(decision.agent_evaluations.len(), 1);
    assert_eq!(decision.provider.evaluations.len(), 1);
    assert!(decision.agent_evaluations[0].score.is_some());
    assert!(decision.provider.evaluations[0].score.is_some());
}

#[test]
fn fallback_requires_attempt_budget_safe_boundary_and_unambiguous_effects() {
    assert_eq!(
        fallback_eligibility(AttemptBoundary::SafeToRetry, false, 1, 2),
        FallbackEligibility::Allowed
    );
    assert_eq!(
        fallback_eligibility(AttemptBoundary::SafeToRetry, false, 2, 2),
        FallbackEligibility::AttemptsExhausted
    );
    assert_eq!(
        fallback_eligibility(AttemptBoundary::SafeToRetry, true, 1, 2),
        FallbackEligibility::AmbiguousEffect
    );
    for boundary in [
        AttemptBoundary::VisibleOutputBegun,
        AttemptBoundary::ToolSideEffectBegun,
        AttemptBoundary::ExternallyCommitted,
    ] {
        assert_eq!(
            fallback_eligibility(boundary, false, 1, 2),
            FallbackEligibility::UnsafeBoundary
        );
    }
}

fn action() -> ApprovalAction {
    ApprovalAction {
        approval_id: ApprovalId::new("approval-one"),
        run_id: pandamux_core::RunId::new("run-one"),
        task_id: RunTaskId::new("task-one"),
        kind: RunApprovalKind::Plan,
        policy_revision: "policy-one".to_string(),
        canonical_action: json!({"b": 2, "a": {"z": 1, "y": 0}}),
        safe_summary: SafeText::new("Approve the exact plan"),
        reason_codes: Vec::new(),
        expires_at_ms: 2_000,
        safety_class: ApprovalSafetyClass::Ordinary,
        trusted_auto_permitted: true,
        hard_denial_codes: Vec::new(),
    }
}

fn requirement(action: &ApprovalAction, status: RunApprovalStatus) -> ApprovalRequirement {
    ApprovalRequirement {
        id: action.approval_id.clone(),
        run_id: action.run_id.clone(),
        task_id: action.task_id.clone(),
        kind: action.kind,
        policy_revision: action.policy_revision.clone(),
        action_digest: canonical_action_digest(&action.canonical_action),
        safe_summary: action.safe_summary.as_str().to_string(),
        reason_codes: action.reason_codes.clone(),
        expires_at_ms: action.expires_at_ms,
        status,
        decision: (status != RunApprovalStatus::Pending).then(|| ApprovalDecisionProvenance {
            actor: "reviewer".to_string(),
            decided_at_ms: 1_100,
            decision_id: "decision-one".to_string(),
        }),
    }
}

#[test]
fn canonical_approval_identity_and_one_mode_reconciler_deduplicate_exact_actions() {
    let mut reordered = action();
    reordered.canonical_action = json!({"a": {"y": 0, "z": 1}, "b": 2});
    assert_eq!(
        canonical_action_digest(&action().canonical_action),
        canonical_action_digest(&reordered.canonical_action)
    );
    assert_eq!(
        canonical_action_digest(&action().canonical_action),
        "6f8d1c3d05d97eed01c2075d8b214890582453321b8597acedb3158391128712"
    );

    assert!(matches!(
        reconcile_approval(
            pandamux_core::OrchestrationMode::Auto,
            &action(),
            &BTreeMap::new(),
            1_000
        ),
        ApprovalReconciliation::Authorized {
            approval_id: None,
            ..
        }
    ));
    let pending = reconcile_approval(
        pandamux_core::OrchestrationMode::ApproveFirst,
        &action(),
        &BTreeMap::new(),
        1_000,
    );
    let ApprovalReconciliation::Pending { approval } = pending else {
        panic!("approve-first must create a requirement")
    };
    let existing = BTreeMap::from([(approval.id.clone(), approval.clone())]);
    assert_eq!(
        reconcile_approval(
            pandamux_core::OrchestrationMode::ApproveFirst,
            &action(),
            &existing,
            1_000,
        ),
        ApprovalReconciliation::Pending { approval }
    );
}

#[test]
fn auto_never_skips_forbidden_safety_classes() {
    for safety_class in [
        ApprovalSafetyClass::AccessWidening,
        ApprovalSafetyClass::CrossOrganization,
        ApprovalSafetyClass::HumanAgentEdit,
        ApprovalSafetyClass::FullAccess,
        ApprovalSafetyClass::McpMutation,
        ApprovalSafetyClass::ProjectLayerTrust,
        ApprovalSafetyClass::ProjectIdentityChange,
    ] {
        let mut candidate = action();
        candidate.safety_class = safety_class;
        assert!(matches!(
            reconcile_approval(
                pandamux_core::OrchestrationMode::Auto,
                &candidate,
                &BTreeMap::new(),
                1_000,
            ),
            ApprovalReconciliation::Pending { .. }
        ));
    }
}

#[test]
fn only_current_exact_unexpired_approved_identity_authorizes_dispatch() {
    let candidate = action();
    let approved = requirement(&candidate, RunApprovalStatus::Approved);
    assert!(matches!(
        reconcile_approval(
            pandamux_core::OrchestrationMode::ApproveFirst,
            &candidate,
            &BTreeMap::from([(approved.id.clone(), approved)]),
            1_000,
        ),
        ApprovalReconciliation::Authorized {
            approval_id: Some(_),
            ..
        }
    ));

    for status in [RunApprovalStatus::Rejected, RunApprovalStatus::Expired] {
        let existing = requirement(&candidate, status);
        assert!(matches!(
            reconcile_approval(
                pandamux_core::OrchestrationMode::ApproveFirst,
                &candidate,
                &BTreeMap::from([(existing.id.clone(), existing)]),
                1_000,
            ),
            ApprovalReconciliation::Denied { .. }
        ));
    }

    let mut expired = requirement(&candidate, RunApprovalStatus::Approved);
    expired.expires_at_ms = 999;
    assert!(matches!(
        reconcile_approval(
            pandamux_core::OrchestrationMode::ApproveFirst,
            &candidate,
            &BTreeMap::from([(expired.id.clone(), expired)]),
            1_000,
        ),
        ApprovalReconciliation::Denied { .. }
    ));

    let mut replacement = candidate.clone();
    replacement.canonical_action = json!({"replacement": true});
    let stale = requirement(&candidate, RunApprovalStatus::Approved);
    assert!(matches!(
        reconcile_approval(
            pandamux_core::OrchestrationMode::ApproveFirst,
            &replacement,
            &BTreeMap::from([(stale.id.clone(), stale)]),
            1_000,
        ),
        ApprovalReconciliation::Denied { .. }
    ));

    replacement.approval_id = ApprovalId::new("approval-two");
    let stale = requirement(&candidate, RunApprovalStatus::Approved);
    assert!(matches!(
        reconcile_approval(
            pandamux_core::OrchestrationMode::ApproveFirst,
            &replacement,
            &BTreeMap::from([(stale.id.clone(), stale)]),
            1_000,
        ),
        ApprovalReconciliation::Pending { .. }
    ));
}

#[test]
fn hard_policy_denial_never_becomes_an_approval_request() {
    let mut denied = action();
    denied.hard_denial_codes = vec!["organization_denied".to_string()];
    assert!(matches!(
        reconcile_approval(
            pandamux_core::OrchestrationMode::ApproveFirst,
            &denied,
            &BTreeMap::new(),
            1_000,
        ),
        ApprovalReconciliation::Denied { .. }
    ));
}
