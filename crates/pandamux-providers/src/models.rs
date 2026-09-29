use pandamux_core::{
    CapacityConfidence,
    event::{ApprovalKind, TurnOutcome},
    provider_config::{ProviderCapabilities, ProviderKind},
    thread::{AccessMode, TurnUsage},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROVIDER_SNAPSHOT_TTL_MS: u64 = 5 * 60 * 1000;

pub fn provider_snapshot_expiry(checked_at_ms: u64) -> u64 {
    checked_at_ms.saturating_add(PROVIDER_SNAPSHOT_TTL_MS)
}

pub fn provider_snapshot_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Metadata identifying a provider driver.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMetadata {
    pub id: ProviderKind,
    pub display_name: String,
    pub capabilities: ProviderCapabilities,
    pub supported_models: Vec<ModelInfo>,
}

/// Description of an AI model supported by a provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub supports_reasoning: bool,
}

/// Lightweight snapshot of provider status without creating sessions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSnapshot {
    pub installed: bool,
    #[serde(default)]
    pub resolved_executable: Option<String>,
    pub version: Option<String>,
    pub auth: ProviderAuthStatus,
    #[serde(default)]
    pub auth_confidence: ProviderAuthConfidence,
    pub health: ProviderHealth,
    #[serde(default)]
    pub runtime_code: Option<String>,
    pub checked_at_ms: u64,
    pub expires_at_ms: u64,
}

/// Strength of the evidence behind an authentication state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthConfidence {
    Reported,
    Detected,
    #[default]
    Unknown,
}

/// Authentication state detected for a provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProviderAuthStatus {
    Authenticated {
        account_label: Option<String>,
        email: Option<String>,
        subscription: Option<String>,
    },
    Configured,
    Unauthenticated,
    Unknown,
}

/// Health rating of a provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProviderHealth {
    Healthy,
    Degraded { reason: String },
    Unavailable { reason: String },
}

/// Parameters for starting a new provider session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSpec {
    pub cwd: PathBuf,
    pub model: String,
    pub effort: Option<String>,
    pub access: AccessMode,
    pub instructions: Option<InjectedInstructions>,
    pub tool_policy: Option<ToolPolicy>,
    pub resume: Option<String>,
}

/// Injected developer or agent instructions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectedInstructions {
    pub role: String,
    #[serde(default)]
    pub knowledge_files: Vec<(String, String)>,
    pub task_prompt: String,
    pub output_contract: Option<String>,
}

impl InjectedInstructions {
    /// Formats instructions into a single unified preamble string.
    pub fn format_preamble(&self) -> String {
        let mut parts = Vec::new();
        if !self.role.trim().is_empty() {
            parts.push(self.role.trim().to_string());
        }
        for (name, content) in &self.knowledge_files {
            parts.push(format!("# Knowledge: {name}\n{content}"));
        }
        if !self.task_prompt.trim().is_empty() {
            parts.push(format!("# Task\n{}", self.task_prompt.trim()));
        }
        if let Some(contract) = &self.output_contract
            && !contract.trim().is_empty()
        {
            parts.push(format!("# Output Contract\n{}", contract.trim()));
        }
        parts.join("\n\n")
    }
}

/// Allow and deny tool access policies.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicy {
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub disallowed: Vec<String>,
}

/// Account rate limit entry reported by a provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitEntry {
    pub limit_name: String,
    #[serde(default)]
    pub used_millionths: Option<u32>,
    #[serde(default)]
    pub remaining_millionths: Option<u32>,
    pub window_duration_mins: u32,
    #[serde(default)]
    pub resets_at_ms: Option<u64>,
    #[serde(default)]
    pub hard_limit: bool,
}

/// Quota and usage limits.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimits {
    pub source: String,
    pub confidence: CapacityConfidence,
    #[serde(default)]
    pub provider_version: Option<String>,
    pub as_of_ms: u64,
    pub limits: Vec<RateLimitEntry>,
}

/// Fine-grained event streamed by a provider during turn execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderEvent {
    TextDelta {
        delta: String,
    },
    ReasoningDelta {
        delta: String,
    },
    ToolCallStarted {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolCallFinished {
        id: String,
        output: Option<String>,
        error: Option<String>,
    },
    ApprovalRequested {
        request_id: String,
        kind: ApprovalKind,
        detail: serde_json::Value,
        command: Option<String>,
    },
    SubAgentSpawned {
        tool_use_id: String,
        parent_tool_use_id: Option<String>,
        agent_type: String,
        description: String,
        model: Option<String>,
    },
    TurnCompleted {
        outcome: TurnOutcome,
        usage: Option<TurnUsage>,
    },
    Error {
        message: String,
        recoverable: bool,
    },
    RateLimitObserved {
        details: serde_json::Value,
    },
    Notice {
        level: String,
        message: String,
    },
}
