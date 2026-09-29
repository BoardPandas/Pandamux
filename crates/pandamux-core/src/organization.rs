use crate::ids::{FailoverPoolId, OrganizationId, ProjectId, ProviderInstanceId, SubscriptionKey};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Mapping of abstract capability tiers to concrete provider instances and models.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TierMapping {
    #[serde(default)]
    pub tiers: BTreeMap<String, ModelTarget>,
}

/// A target provider instance and optional model identifier for a tier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTarget {
    pub provider_instance_id: ProviderInstanceId,
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
}

/// Explicit sharing policy for the optional Broadside integration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadsidePolicy {
    #[serde(default)]
    pub allow_artifacts: bool,
    #[serde(default)]
    pub allow_transcripts: bool,
}

/// One eligible provider and quota boundary within a failover pool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailoverPoolMember {
    pub provider_instance_id: ProviderInstanceId,
    pub subscription_key: SubscriptionKey,
}

/// Explicit provider routing pool. Phase 4F adds capability-aware editing and scoring.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailoverPool {
    pub id: FailoverPoolId,
    pub display_name: String,
    #[serde(default)]
    pub members: Vec<FailoverPoolMember>,
    #[serde(default = "default_revision")]
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailoverPoolError {
    EmptyIdentifier(&'static str),
    EmptyDisplayName,
    EmptyMembers,
    ZeroRevision,
    DuplicateProviderInstance(ProviderInstanceId),
}

impl fmt::Display for FailoverPoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyIdentifier(field) => write!(f, "{field} must not be empty"),
            Self::EmptyDisplayName => f.write_str("failover pool display name must not be empty"),
            Self::EmptyMembers => f.write_str("failover pool must contain at least one member"),
            Self::ZeroRevision => f.write_str("failover pool revision must be greater than zero"),
            Self::DuplicateProviderInstance(instance_id) => {
                write!(f, "duplicate failover provider instance: {instance_id}")
            }
        }
    }
}

impl std::error::Error for FailoverPoolError {}

impl FailoverPool {
    pub fn validate(&self) -> Result<(), FailoverPoolError> {
        if self.id.as_str().trim().is_empty() {
            return Err(FailoverPoolError::EmptyIdentifier("failover pool id"));
        }
        if self.display_name.trim().is_empty() {
            return Err(FailoverPoolError::EmptyDisplayName);
        }
        if self.members.is_empty() {
            return Err(FailoverPoolError::EmptyMembers);
        }
        if self.revision == 0 {
            return Err(FailoverPoolError::ZeroRevision);
        }

        let mut instances = BTreeSet::new();
        for member in &self.members {
            if member.provider_instance_id.as_str().trim().is_empty() {
                return Err(FailoverPoolError::EmptyIdentifier(
                    "failover provider instance id",
                ));
            }
            if member.subscription_key.as_str().trim().is_empty() {
                return Err(FailoverPoolError::EmptyIdentifier(
                    "failover subscription key",
                ));
            }
            if !instances.insert(member.provider_instance_id.as_str()) {
                return Err(FailoverPoolError::DuplicateProviderInstance(
                    member.provider_instance_id.clone(),
                ));
            }
        }
        Ok(())
    }
}

/// Canonical organization policy and budget boundary.
///
/// This replaces the prior overlapping policy and subscription records. Provider
/// instances identify runtimes, while subscription keys identify quota boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPolicy {
    pub id: OrganizationId,
    pub display_name: String,
    #[serde(default)]
    pub project_ids: Vec<ProjectId>,
    #[serde(default)]
    pub allowed_provider_instances: Vec<ProviderInstanceId>,
    #[serde(default)]
    pub allowed_subscriptions: Vec<SubscriptionKey>,
    #[serde(default)]
    pub failover_pool_ids: Vec<FailoverPoolId>,
    #[serde(default)]
    pub default_tier_mapping: Option<TierMapping>,
    #[serde(default)]
    pub max_concurrency: Option<u32>,
    #[serde(default)]
    pub daily_budget_micro_usd: Option<u64>,
    #[serde(default)]
    pub broadside: BroadsidePolicy,
    #[serde(default = "default_revision")]
    pub revision: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrganizationPolicyWire {
    #[serde(alias = "orgId")]
    id: OrganizationId,
    display_name: String,
    #[serde(default)]
    project_ids: Vec<ProjectId>,
    #[serde(default)]
    allowed_provider_instances: Vec<ProviderInstanceId>,
    #[serde(default)]
    allowed_subscriptions: Vec<SubscriptionKey>,
    #[serde(default)]
    failover_pool_ids: Vec<FailoverPoolId>,
    #[serde(default)]
    default_tier_mapping: Option<TierMapping>,
    #[serde(default)]
    max_concurrency: Option<u32>,
    #[serde(default)]
    daily_budget_micro_usd: Option<u64>,
    #[serde(default)]
    daily_budget_usd: Option<f64>,
    #[serde(default)]
    broadside: BroadsidePolicy,
    #[serde(default = "default_revision")]
    revision: u64,
}

impl<'de> Deserialize<'de> for OrganizationPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = OrganizationPolicyWire::deserialize(deserializer)?;
        if wire.daily_budget_micro_usd.is_some() && wire.daily_budget_usd.is_some() {
            return Err(serde::de::Error::custom(
                "organization budget must use one representation",
            ));
        }
        let daily_budget_micro_usd = match wire.daily_budget_micro_usd {
            Some(value) => Some(value),
            None => wire
                .daily_budget_usd
                .map(legacy_usd_to_micro)
                .transpose()
                .map_err(serde::de::Error::custom)?,
        };
        Ok(Self {
            id: wire.id,
            display_name: wire.display_name,
            project_ids: wire.project_ids,
            allowed_provider_instances: wire.allowed_provider_instances,
            allowed_subscriptions: wire.allowed_subscriptions,
            failover_pool_ids: wire.failover_pool_ids,
            default_tier_mapping: wire.default_tier_mapping,
            max_concurrency: wire.max_concurrency,
            daily_budget_micro_usd,
            broadside: wire.broadside,
            revision: wire.revision,
        })
    }
}

fn default_revision() -> u64 {
    1
}

fn legacy_usd_to_micro(value: f64) -> Result<u64, String> {
    if !value.is_finite() || value.is_sign_negative() {
        return Err("legacy organization budget must be finite and non-negative".to_string());
    }
    let scaled = value * 1_000_000.0;
    let rounded = scaled.round();
    if rounded > u64::MAX as f64 || (scaled - rounded).abs() > 0.000_001 {
        return Err("legacy organization budget cannot be represented in micro-USD".to_string());
    }
    Ok(rounded as u64)
}

/// Compatibility name for settings written before the canonical policy model.
pub type OrganizationSubscription = OrganizationPolicy;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrganizationPolicyError {
    EmptyIdentifier(&'static str),
    EmptyDisplayName,
    ZeroRevision,
    ZeroConcurrency,
    DuplicateReference {
        field: &'static str,
        value: String,
    },
    EmptyTierName,
    EmptyTierModel(String),
    TierProviderNotAllowed(ProviderInstanceId),
    MissingProjectAssignment(ProjectId),
    AmbiguousProjectAssignment(ProjectId),
    InvalidPolicy {
        organization_id: OrganizationId,
        reason: String,
    },
}

impl fmt::Display for OrganizationPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyIdentifier(field) => write!(f, "{field} must not be empty"),
            Self::EmptyDisplayName => f.write_str("organization display name must not be empty"),
            Self::ZeroRevision => f.write_str("organization revision must be greater than zero"),
            Self::ZeroConcurrency => {
                f.write_str("organization max concurrency must be greater than zero")
            }
            Self::DuplicateReference { field, value } => {
                write!(f, "duplicate {field} reference: {value}")
            }
            Self::EmptyTierName => f.write_str("tier name must not be empty"),
            Self::EmptyTierModel(tier) => write!(f, "tier {tier} must name a model"),
            Self::TierProviderNotAllowed(provider) => {
                write!(
                    f,
                    "tier provider is not allowed by organization: {provider}"
                )
            }
            Self::MissingProjectAssignment(project) => {
                write!(f, "project has no organization assignment: {project}")
            }
            Self::AmbiguousProjectAssignment(project) => {
                write!(
                    f,
                    "project has multiple organization assignments: {project}"
                )
            }
            Self::InvalidPolicy {
                organization_id,
                reason,
            } => write!(f, "organization {organization_id} is invalid: {reason}"),
        }
    }
}

impl std::error::Error for OrganizationPolicyError {}

impl OrganizationPolicy {
    pub fn validate(&self) -> Result<(), OrganizationPolicyError> {
        if self.id.as_str().trim().is_empty() {
            return Err(OrganizationPolicyError::EmptyIdentifier("organization id"));
        }
        if self.display_name.trim().is_empty() {
            return Err(OrganizationPolicyError::EmptyDisplayName);
        }
        if self.revision == 0 {
            return Err(OrganizationPolicyError::ZeroRevision);
        }
        if self.max_concurrency == Some(0) {
            return Err(OrganizationPolicyError::ZeroConcurrency);
        }
        ensure_unique("project", self.project_ids.iter().map(ProjectId::as_str))?;
        ensure_unique(
            "provider instance",
            self.allowed_provider_instances
                .iter()
                .map(ProviderInstanceId::as_str),
        )?;
        ensure_unique(
            "subscription",
            self.allowed_subscriptions
                .iter()
                .map(SubscriptionKey::as_str),
        )?;
        ensure_unique(
            "failover pool",
            self.failover_pool_ids.iter().map(FailoverPoolId::as_str),
        )?;

        if let Some(mapping) = &self.default_tier_mapping {
            let allowed: BTreeSet<&str> = self
                .allowed_provider_instances
                .iter()
                .map(ProviderInstanceId::as_str)
                .collect();
            for (tier, target) in &mapping.tiers {
                if tier.trim().is_empty() {
                    return Err(OrganizationPolicyError::EmptyTierName);
                }
                if target.model.trim().is_empty() {
                    return Err(OrganizationPolicyError::EmptyTierModel(tier.clone()));
                }
                if !allowed.contains(target.provider_instance_id.as_str()) {
                    return Err(OrganizationPolicyError::TierProviderNotAllowed(
                        target.provider_instance_id.clone(),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn ensure_unique<'a>(
    field: &'static str,
    values: impl Iterator<Item = &'a str>,
) -> Result<(), OrganizationPolicyError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() {
            return Err(OrganizationPolicyError::EmptyIdentifier(field));
        }
        if !seen.insert(value) {
            return Err(OrganizationPolicyError::DuplicateReference {
                field,
                value: value.to_string(),
            });
        }
    }
    Ok(())
}

/// Resolve exactly one organization for a policy-sensitive project action.
pub fn resolve_project_organization<'a>(
    policies: &'a [OrganizationPolicy],
    project_id: &ProjectId,
) -> Result<&'a OrganizationPolicy, OrganizationPolicyError> {
    let mut matches = policies
        .iter()
        .filter(|policy| policy.project_ids.iter().any(|id| id == project_id));
    let policy = matches
        .next()
        .ok_or_else(|| OrganizationPolicyError::MissingProjectAssignment(project_id.clone()))?;
    if matches.next().is_some() {
        return Err(OrganizationPolicyError::AmbiguousProjectAssignment(
            project_id.clone(),
        ));
    }
    policy
        .validate()
        .map_err(|error| OrganizationPolicyError::InvalidPolicy {
            organization_id: policy.id.clone(),
            reason: error.to_string(),
        })?;
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(id: &str, project: &str) -> OrganizationPolicy {
        OrganizationPolicy {
            id: OrganizationId::new(id),
            display_name: "Example Organization".to_string(),
            project_ids: vec![ProjectId::new(project)],
            allowed_provider_instances: vec![ProviderInstanceId::new("codex-work")],
            allowed_subscriptions: vec![SubscriptionKey::new("subkey-work")],
            failover_pool_ids: vec![FailoverPoolId::new("pool-work")],
            default_tier_mapping: Some(TierMapping {
                tiers: BTreeMap::from([(
                    "reasoning".to_string(),
                    ModelTarget {
                        provider_instance_id: ProviderInstanceId::new("codex-work"),
                        model: "gpt-5".to_string(),
                        effort: Some("high".to_string()),
                    },
                )]),
            }),
            max_concurrency: Some(4),
            daily_budget_micro_usd: Some(25_000_000),
            broadside: BroadsidePolicy::default(),
            revision: 1,
        }
    }

    #[test]
    fn canonical_policy_validates_and_resolves_exactly_once() {
        let policy = policy("org-work", "proj-one");
        policy.validate().expect("valid policy");
        assert_eq!(
            resolve_project_organization(&[policy], &ProjectId::new("proj-one"))
                .expect("resolved policy")
                .id
                .as_str(),
            "org-work"
        );
    }

    #[test]
    fn missing_and_ambiguous_project_assignments_fail_closed() {
        let project_id = ProjectId::new("proj-one");
        assert!(matches!(
            resolve_project_organization(&[], &project_id),
            Err(OrganizationPolicyError::MissingProjectAssignment(_))
        ));
        let policies = vec![policy("org-one", "proj-one"), policy("org-two", "proj-one")];
        assert!(matches!(
            resolve_project_organization(&policies, &project_id),
            Err(OrganizationPolicyError::AmbiguousProjectAssignment(_))
        ));
    }

    #[test]
    fn tier_mapping_cannot_escape_allowed_provider_instances() {
        let mut value = policy("org-work", "proj-one");
        value.default_tier_mapping.as_mut().unwrap().tiers.insert(
            "fast".to_string(),
            ModelTarget {
                provider_instance_id: ProviderInstanceId::new("personal-provider"),
                model: "fast-model".to_string(),
                effort: None,
            },
        );
        assert!(matches!(
            value.validate(),
            Err(OrganizationPolicyError::TierProviderNotAllowed(_))
        ));
    }

    #[test]
    fn legacy_subscription_deserializes_into_integer_policy() {
        let json = r#"{
            "orgId":"org-legacy",
            "displayName":"Legacy",
            "allowedProviderInstances":["claude-work"],
            "maxConcurrency":3,
            "dailyBudgetUsd":12.5
        }"#;
        let value: OrganizationPolicy = serde_json::from_str(json).expect("legacy policy");
        assert_eq!(value.id.as_str(), "org-legacy");
        assert_eq!(value.daily_budget_micro_usd, Some(12_500_000));
        assert_eq!(value.max_concurrency, Some(3));
        assert_eq!(value.revision, 1);
        assert!(
            !serde_json::to_string(&value)
                .expect("serialize policy")
                .contains("dailyBudgetUsd")
        );
    }

    #[test]
    fn failover_pool_rejects_duplicate_or_empty_candidates() {
        let member = FailoverPoolMember {
            provider_instance_id: ProviderInstanceId::new("codex-work"),
            subscription_key: SubscriptionKey::new("subkey-work"),
        };
        let mut pool = FailoverPool {
            id: FailoverPoolId::new("pool-work"),
            display_name: "Work pool".to_string(),
            members: vec![member.clone(), member],
            revision: 1,
        };
        assert!(matches!(
            pool.validate(),
            Err(FailoverPoolError::DuplicateProviderInstance(_))
        ));

        pool.members.clear();
        assert_eq!(pool.validate(), Err(FailoverPoolError::EmptyMembers));
    }
}
