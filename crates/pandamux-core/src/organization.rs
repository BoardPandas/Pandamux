use crate::ids::ProviderInstanceId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Organization-level policies restricting provider usage and defining tier defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPolicy {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub allowed_provider_instances: Vec<ProviderInstanceId>,
    #[serde(default)]
    pub default_tier_mapping: Option<TierMapping>,
}

/// Mapping of abstract capability tiers (fast, smart, reasoning, orchestrator)
/// to concrete provider instances and models.
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

/// Organization subscription access and budget boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationSubscription {
    pub org_id: String,
    pub display_name: String,
    #[serde(default)]
    pub allowed_provider_instances: Vec<ProviderInstanceId>,
    #[serde(default)]
    pub max_concurrency: u32,
    #[serde(default)]
    pub daily_budget_usd: Option<f64>,
}
