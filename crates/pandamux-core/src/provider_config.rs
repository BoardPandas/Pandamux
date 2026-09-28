use crate::ids::{EnvironmentId, ProviderInstanceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Supported agent provider backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Claude,
    Codex,
    Antigravity,
    Cursor,
    Grok,
    OpenCode,
    Custom,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Antigravity => "antigravity",
            Self::Cursor => "cursor",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
            Self::Custom => "custom",
        }
    }
}

/// Optional per-environment overrides for a provider instance.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEnvironmentOverride {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub binary_path: Option<String>,
    #[serde(default)]
    pub concurrency_limit: Option<u32>,
    #[serde(default)]
    pub env_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub settings: Option<serde_json::Value>,
}

/// Configuration for a specific provider instance profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstanceConfig {
    pub id: ProviderInstanceId,
    pub provider: ProviderKind,
    pub display_name: String,
    pub profile_dir: String,
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    #[serde(default)]
    pub binary_path: Option<String>,
    #[serde(default = "default_concurrency")]
    pub concurrency_limit: u32,
    #[serde(default)]
    pub env_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub settings: serde_json::Value,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl ProviderInstanceConfig {
    /// Resolves the effective configuration for a target environment.
    pub fn resolve_for_environment(
        &self,
        env_id: &EnvironmentId,
        overrides: Option<&ProviderEnvironmentOverride>,
    ) -> Self {
        let mut resolved = self.clone();
        if let Some(ovr) = overrides {
            if let Some(en) = ovr.enabled {
                resolved.enabled = en;
            }
            if let Some(bp) = &ovr.binary_path {
                resolved.binary_path = Some(bp.clone());
            }
            if let Some(limit) = ovr.concurrency_limit {
                resolved.concurrency_limit = limit;
            }
            for (k, v) in &ovr.env_overrides {
                resolved.env_overrides.insert(k.clone(), v.clone());
            }
            if let Some(st) = &ovr.settings {
                resolved.settings = st.clone();
            }
        }
        resolved.environment_id = Some(env_id.clone());
        resolved
    }
}

fn default_concurrency() -> u32 {
    2
}

fn default_enabled() -> bool {
    true
}

/// Dynamic capabilities reported by or configured for a provider driver.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub supports_streaming: bool,
    pub supports_tool_calls: bool,
    pub supports_sub_agents: bool,
    pub supports_system_prompt: bool,
    pub supports_reasoning: bool,
    #[serde(default)]
    pub supports_approvals: bool,
    #[serde(default)]
    pub supports_rate_limits: bool,
    #[serde(default)]
    pub supports_resume: bool,
    #[serde(default)]
    pub supports_images: bool,
}
