use pandamux_core::{EnvironmentId, ProviderInstanceId, ProviderKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderLoginFlow {
    Terminal,
    Browser,
    Manual,
    Unavailable,
}

/// Side-effect-free summary included with health reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLoginGuidance {
    pub flow: ProviderLoginFlow,
    pub available: bool,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLoginPreviewParams {
    pub provider_instance_id: ProviderInstanceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<EnvironmentId>,
}

/// One-time preview that must be consumed before a login terminal is opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLoginPreview {
    pub preview_id: String,
    pub environment_id: EnvironmentId,
    pub provider_instance_id: ProviderInstanceId,
    pub provider: ProviderKind,
    pub profile: String,
    pub flow: ProviderLoginFlow,
    pub display_command: String,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLoginLaunchParams {
    pub preview_id: String,
    pub provider_instance_id: ProviderInstanceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<EnvironmentId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLoginLaunchResult {
    pub environment_id: EnvironmentId,
    pub provider_instance_id: ProviderInstanceId,
    pub terminal_id: String,
    pub display_command: String,
    pub retryable: bool,
}
