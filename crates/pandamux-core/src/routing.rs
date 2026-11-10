use crate::{
    AccessMode, CapacityConfidence, CapacityFreshness, ContinuationId, EnvironmentId,
    ProviderCapabilities, ProviderInstanceId, ProviderKind, SubscriptionKey, ThreadId,
    TurnAttemptId, TurnId,
};
use serde::{Deserialize, Serialize};

/// Capability contract shared by every member of one explicit failover pool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingCapabilityRequirements {
    #[serde(default)]
    pub requires_instructions: bool,
    #[serde(default)]
    pub requires_tool_calls: bool,
    #[serde(default)]
    pub requires_approvals: bool,
    #[serde(default)]
    pub requires_reasoning: bool,
    #[serde(default)]
    pub access_mode: AccessMode,
}

impl RoutingCapabilityRequirements {
    pub fn validate_capabilities(self, capabilities: ProviderCapabilities) -> Result<(), String> {
        if self.requires_instructions && !capabilities.supports_system_prompt {
            return Err("required instruction delivery is not declared".to_string());
        }
        if self.requires_tool_calls && !capabilities.supports_tool_calls {
            return Err("required tool calls are not declared".to_string());
        }
        if self.requires_approvals && !capabilities.supports_approvals {
            return Err("required approval handling is not declared".to_string());
        }
        if self.requires_reasoning && !capabilities.supports_reasoning {
            return Err("required reasoning is not declared".to_string());
        }
        if !capabilities.supports_access_mode(self.access_mode) {
            return Err(format!(
                "required {:?} access exceeds the declared {:?} ceiling",
                self.access_mode, capabilities.max_access_mode
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingAuthentication {
    Authenticated,
    Configured,
    Unauthenticated,
    #[default]
    Unknown,
}

impl RoutingAuthentication {
    fn is_eligible(self) -> bool {
        matches!(self, Self::Authenticated | Self::Configured)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RoutingHealth {
    Healthy,
    Degraded {
        reason: String,
    },
    Unavailable {
        reason: String,
    },
    #[default]
    Unknown,
}

/// Fully resolved candidate input for the pure deterministic router.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingCandidate {
    pub provider_instance_id: ProviderInstanceId,
    pub subscription_key: SubscriptionKey,
    pub provider_kind: ProviderKind,
    pub model: String,
    pub capabilities: ProviderCapabilities,
    pub policy_allowed: bool,
    pub subscription_allowed: bool,
    pub model_compatible: bool,
    pub installed: bool,
    pub authentication: RoutingAuthentication,
    pub health: RoutingHealth,
    pub capacity_confidence: CapacityConfidence,
    pub capacity_freshness: CapacityFreshness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_millionths: Option<u32>,
    #[serde(default)]
    pub hard_limited: bool,
    pub active_concurrency: u32,
    pub concurrency_limit: u32,
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub user_preference: u16,
    #[serde(default)]
    pub local_hard_blocks: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingScore {
    pub capacity: i64,
    pub health: i64,
    pub concurrency: i64,
    pub locality: i64,
    pub preference: i64,
    pub total: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingEvaluation {
    pub provider_instance_id: ProviderInstanceId,
    pub subscription_key: SubscriptionKey,
    pub model: String,
    pub eligible: bool,
    #[serde(default)]
    pub rejection_codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<RoutingScore>,
    pub explanation: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingDecision {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_provider_instance_id: Option<ProviderInstanceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_subscription_key: Option<SubscriptionKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_model: Option<String>,
    #[serde(default)]
    pub evaluations: Vec<RoutingEvaluation>,
}

/// Apply every hard filter before scoring any eligible candidate.
pub fn select_route(
    requirements: RoutingCapabilityRequirements,
    candidates: impl IntoIterator<Item = RoutingCandidate>,
) -> RoutingDecision {
    let mut evaluations = candidates
        .into_iter()
        .map(|candidate| evaluate_candidate(requirements, candidate))
        .collect::<Vec<_>>();
    evaluations.sort_by(|left, right| {
        left.provider_instance_id
            .cmp(&right.provider_instance_id)
            .then_with(|| left.subscription_key.cmp(&right.subscription_key))
    });

    let selected = evaluations
        .iter()
        .filter(|evaluation| evaluation.eligible)
        .max_by(|left, right| {
            left.score
                .as_ref()
                .map(|score| score.total)
                .cmp(&right.score.as_ref().map(|score| score.total))
                .then_with(|| right.provider_instance_id.cmp(&left.provider_instance_id))
                .then_with(|| right.subscription_key.cmp(&left.subscription_key))
        })
        .cloned();

    RoutingDecision {
        selected_provider_instance_id: selected
            .as_ref()
            .map(|evaluation| evaluation.provider_instance_id.clone()),
        selected_subscription_key: selected
            .as_ref()
            .map(|evaluation| evaluation.subscription_key.clone()),
        selected_model: selected.as_ref().map(|evaluation| evaluation.model.clone()),
        evaluations,
    }
}

fn evaluate_candidate(
    requirements: RoutingCapabilityRequirements,
    candidate: RoutingCandidate,
) -> RoutingEvaluation {
    let mut rejection_codes = Vec::new();
    if !candidate.policy_allowed {
        rejection_codes.push("organization_policy_denied".to_string());
    }
    if !candidate.subscription_allowed {
        rejection_codes.push("subscription_policy_denied".to_string());
    }
    if let Err(reason) = requirements.validate_capabilities(candidate.capabilities) {
        rejection_codes.push(format!("capability_mismatch:{reason}"));
    }
    if !candidate.model_compatible || candidate.model.trim().is_empty() {
        rejection_codes.push("model_incompatible".to_string());
    }
    if !candidate.installed {
        rejection_codes.push("provider_not_installed".to_string());
    }
    if !candidate.authentication.is_eligible() {
        rejection_codes.push("provider_authentication_unverified".to_string());
    }
    if let RoutingHealth::Unavailable { reason } = &candidate.health {
        rejection_codes.push(format!("provider_unavailable:{reason}"));
    }
    if candidate.hard_limited || candidate.capacity_freshness == CapacityFreshness::HardLimited {
        rejection_codes.push("capacity_hard_limit".to_string());
    }
    if candidate.concurrency_limit == 0
        || candidate.active_concurrency >= candidate.concurrency_limit
    {
        rejection_codes.push("concurrency_limit_reached".to_string());
    }
    rejection_codes.extend(
        candidate
            .local_hard_blocks
            .iter()
            .map(|reason| format!("local_hard_block:{reason}")),
    );

    if !rejection_codes.is_empty() {
        return RoutingEvaluation {
            provider_instance_id: candidate.provider_instance_id,
            subscription_key: candidate.subscription_key,
            model: candidate.model,
            eligible: false,
            explanation: format!("rejected by {}", rejection_codes.join(", ")),
            rejection_codes,
            score: None,
        };
    }

    let capacity_confidence = match candidate.capacity_confidence {
        CapacityConfidence::Reported => 300,
        CapacityConfidence::Estimated => 180,
        CapacityConfidence::Unknown => 50,
    };
    let capacity_freshness = match candidate.capacity_freshness {
        CapacityFreshness::Fresh => 100,
        CapacityFreshness::Aging => 50,
        CapacityFreshness::Stale
        | CapacityFreshness::Reset
        | CapacityFreshness::ClockSkew
        | CapacityFreshness::HardLimited => 0,
    };
    let capacity_remaining = i64::from(candidate.remaining_millionths.unwrap_or(0)) / 1_000;
    let capacity = capacity_confidence + capacity_freshness + capacity_remaining;
    let health = match candidate.health {
        RoutingHealth::Healthy => 500,
        RoutingHealth::Degraded { .. } => 200,
        RoutingHealth::Unknown => 50,
        RoutingHealth::Unavailable { .. } => 0,
    };
    let concurrency = i64::from(
        candidate
            .concurrency_limit
            .saturating_sub(candidate.active_concurrency),
    ) * 100
        / i64::from(candidate.concurrency_limit);
    let locality = if candidate.local { 100 } else { 0 };
    let preference = i64::from(candidate.user_preference);
    let total = capacity + health + concurrency + locality + preference;
    let score = RoutingScore {
        capacity,
        health,
        concurrency,
        locality,
        preference,
        total,
    };
    RoutingEvaluation {
        provider_instance_id: candidate.provider_instance_id,
        subscription_key: candidate.subscription_key,
        model: candidate.model,
        eligible: true,
        rejection_codes,
        explanation: format!(
            "eligible with score {total}: capacity {capacity}, health {health}, concurrency {concurrency}, locality {locality}, preference {preference}"
        ),
        score: Some(score),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptBoundary {
    #[default]
    SafeToRetry,
    VisibleOutputBegun,
    ToolSideEffectBegun,
    ExternallyCommitted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnAttemptStatus {
    #[default]
    Running,
    Retrying,
    InterruptedForCapacity,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityInterruptionEvidence {
    pub environment_id: EnvironmentId,
    pub provider_instance_id: ProviderInstanceId,
    pub provider_kind: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_key: Option<SubscriptionKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_version: Option<String>,
    pub details: serde_json::Value,
    pub observed_at_ms: u64,
}

/// Immutable provider configuration and probe evidence used by one route attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptExecutionEvidence {
    pub provider_kind: ProviderKind,
    pub profile_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    pub capabilities: ProviderCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnAttempt {
    pub id: TurnAttemptId,
    pub turn_id: TurnId,
    pub thread_id: ThreadId,
    pub ordinal: u32,
    pub environment_id: EnvironmentId,
    pub provider_instance_id: ProviderInstanceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_key: Option<SubscriptionKey>,
    pub model: String,
    pub boundary: AttemptBoundary,
    pub status: TurnAttemptStatus,
    pub routing: RoutingDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<AttemptExecutionEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
    #[serde(default)]
    pub partial_output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_evidence: Option<CapacityInterruptionEvidence>,
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolLedgerStatus {
    #[default]
    Started,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolLedgerEntry {
    pub attempt_id: TurnAttemptId,
    pub turn_id: TurnId,
    pub provider_item_id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub status: ToolLedgerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationState {
    #[default]
    Offered,
    Claimed,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinuationRecord {
    pub id: ContinuationId,
    pub idempotency_key: String,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub source_attempt_id: TurnAttemptId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_provider_instance_id: Option<ProviderInstanceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_turn_id: Option<TurnId>,
    pub state: ContinuationState,
    pub handoff: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationDeliveryKind {
    Approval,
    ToolResult,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationDeliveryState {
    #[default]
    Pending,
    Delivered,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinuationDelivery {
    pub delivery_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_id: Option<ContinuationId>,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub kind: ContinuationDeliveryKind,
    pub item_id: String,
    pub payload_sha256: String,
    pub payload_json: String,
    pub state: ContinuationDeliveryState,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities() -> ProviderCapabilities {
        ProviderCapabilities {
            supports_streaming: true,
            supports_tool_calls: true,
            supports_sub_agents: false,
            supports_system_prompt: true,
            supports_reasoning: true,
            supports_approvals: true,
            supports_rate_limits: true,
            supports_resume: false,
            supports_images: false,
            max_access_mode: AccessMode::AutoEdit,
        }
    }

    fn candidate(id: &str) -> RoutingCandidate {
        RoutingCandidate {
            provider_instance_id: ProviderInstanceId::new(id),
            subscription_key: SubscriptionKey::new(format!("subkey-{id}")),
            provider_kind: ProviderKind::Claude,
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
            concurrency_limit: 2,
            local: true,
            user_preference: 0,
            local_hard_blocks: Vec::new(),
        }
    }

    #[test]
    fn hard_filters_run_before_scores_and_explain_every_candidate() {
        let mut disallowed = candidate("only-healthy");
        disallowed.policy_allowed = false;
        disallowed.remaining_millionths = Some(1_000_000);
        disallowed.user_preference = 1_000;
        let mut allowed = candidate("allowed");
        allowed.health = RoutingHealth::Degraded {
            reason: "slow".to_string(),
        };
        allowed.remaining_millionths = Some(100_000);

        let decision = select_route(
            RoutingCapabilityRequirements {
                requires_instructions: true,
                requires_tool_calls: true,
                requires_approvals: true,
                requires_reasoning: false,
                access_mode: AccessMode::WorkspaceOnly,
            },
            [disallowed, allowed],
        );

        assert_eq!(
            decision.selected_provider_instance_id,
            Some(ProviderInstanceId::new("allowed"))
        );
        assert_eq!(decision.evaluations.len(), 2);
        let rejected = decision
            .evaluations
            .iter()
            .find(|evaluation| evaluation.provider_instance_id.as_str() == "only-healthy")
            .expect("rejected candidate");
        assert!(!rejected.eligible);
        assert!(
            rejected
                .rejection_codes
                .contains(&"organization_policy_denied".to_string())
        );
        assert!(
            decision
                .evaluations
                .iter()
                .all(|item| !item.explanation.is_empty())
        );
    }

    #[test]
    fn selection_is_deterministic_for_equal_scores() {
        let forward = select_route(
            RoutingCapabilityRequirements::default(),
            [candidate("bravo"), candidate("alpha")],
        );
        let reverse = select_route(
            RoutingCapabilityRequirements::default(),
            [candidate("alpha"), candidate("bravo")],
        );
        assert_eq!(forward, reverse);
        assert_eq!(
            forward.selected_provider_instance_id,
            Some(ProviderInstanceId::new("alpha"))
        );
    }

    #[test]
    fn capability_and_access_mismatches_are_hard_rejections() {
        let mut value = candidate("limited");
        value.capabilities.supports_system_prompt = false;
        value.capabilities.max_access_mode = AccessMode::ReadOnly;
        let decision = select_route(
            RoutingCapabilityRequirements {
                requires_instructions: true,
                access_mode: AccessMode::AutoEdit,
                ..RoutingCapabilityRequirements::default()
            },
            [value],
        );
        assert!(decision.selected_provider_instance_id.is_none());
        assert!(
            decision.evaluations[0]
                .rejection_codes
                .iter()
                .any(|code| code.starts_with("capability_mismatch:"))
        );
    }
}
