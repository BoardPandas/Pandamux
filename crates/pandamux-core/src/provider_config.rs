use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use crate::ids::ProviderInstanceId;

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

/// Configuration for a specific provider instance profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstanceConfig {
    pub id: ProviderInstanceId,
    pub provider: ProviderKind,
    pub display_name: String,
    pub profile_dir: String,
    #[serde(default = "default_concurrency")]
    pub concurrency_limit: u32,
    #[serde(default)]
    pub env_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub settings: serde_json::Value,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
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
}
