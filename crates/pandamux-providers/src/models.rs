use pandamux_core::{
    AttemptBoundary, CapacityConfidence,
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
    AttemptBoundaryAdvanced {
        boundary: AttemptBoundary,
        reason: String,
    },
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
    CapacityExhausted {
        details: serde_json::Value,
    },
    Notice {
        level: String,
        message: String,
    },
}

impl ProviderEvent {
    pub fn required_attempt_boundary(&self) -> Option<(AttemptBoundary, &'static str)> {
        match self {
            Self::TextDelta { .. } | Self::ReasoningDelta { .. } => Some((
                AttemptBoundary::VisibleOutputBegun,
                "provider visible output",
            )),
            Self::ToolCallStarted { .. } | Self::ToolCallFinished { .. } => Some((
                AttemptBoundary::ToolSideEffectBegun,
                "provider tool activity",
            )),
            Self::ApprovalRequested { .. } => Some((
                AttemptBoundary::ExternallyCommitted,
                "provider approval interaction",
            )),
            Self::SubAgentSpawned { .. } => Some((
                AttemptBoundary::ToolSideEffectBegun,
                "provider subagent activity",
            )),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderEventForwardError;

/// Forward one provider event while emitting the side-effect boundary first.
pub async fn forward_provider_event(
    events: &tokio::sync::mpsc::Sender<ProviderEvent>,
    event: ProviderEvent,
) -> Result<(), ProviderEventForwardError> {
    if let Some((boundary, reason)) = event.required_attempt_boundary() {
        events
            .send(ProviderEvent::AttemptBoundaryAdvanced {
                boundary,
                reason: reason.to_string(),
            })
            .await
            .map_err(|_| ProviderEventForwardError)?;
    }
    if let Some(details) = capacity_exhaustion_details(&event) {
        events
            .send(ProviderEvent::CapacityExhausted { details })
            .await
            .map_err(|_| ProviderEventForwardError)?;
    }
    events
        .send(event)
        .await
        .map_err(|_| ProviderEventForwardError)
}

fn capacity_exhaustion_details(event: &ProviderEvent) -> Option<serde_json::Value> {
    match event {
        ProviderEvent::Error {
            message,
            recoverable: true,
        } if message_is_capacity_exhaustion(message) => Some(serde_json::json!({
            "source": "provider_error",
            "message": message,
        })),
        ProviderEvent::RateLimitObserved { details }
            if details_indicate_capacity_exhaustion(details) =>
        {
            Some(serde_json::json!({
                "source": "provider_rate_limit",
                "report": details,
            }))
        }
        _ => None,
    }
}

pub(crate) fn message_is_capacity_exhaustion(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "rate limit",
        "rate_limit",
        "quota exhausted",
        "quota exceeded",
        "usage limit",
        "too many requests",
        "http 429",
        "status 429",
        "capacity exhausted",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

pub(crate) fn rate_limit_snapshot_reports_hard_block(
    limits: &serde_json::Value,
    ordinary_usage_allowed: Option<bool>,
) -> bool {
    if ordinary_usage_allowed == Some(true) {
        return false;
    }
    if limits
        .get("spendControlReached")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return true;
    }

    let reached_type = limits
        .get("rateLimitReachedType")
        .filter(|value| !value.is_null());
    if reached_type.is_some_and(|value| value.as_str() != Some("rate_limit_reached")) {
        return true;
    }

    let credits = limits.get("credits").filter(|value| value.is_object());
    let credits_available = credits.is_some_and(|credits| {
        credits
            .get("hasCredits")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
            || credits
                .get("unlimited")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    });
    if credits_available {
        return false;
    }
    if reached_type.is_some() {
        return true;
    }

    let credits_explicitly_unavailable = credits.is_some_and(|credits| {
        credits
            .get("hasCredits")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
            && credits
                .get("unlimited")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
    });
    ordinary_usage_allowed == Some(false) && credits_explicitly_unavailable
}

fn details_indicate_capacity_exhaustion(details: &serde_json::Value) -> bool {
    let limits = details.get("rateLimits").unwrap_or(details);
    if rate_limit_snapshot_reports_hard_block(
        limits,
        details
            .get("ordinaryUsageAllowed")
            .and_then(serde_json::Value::as_bool),
    ) {
        return true;
    }
    let text = details.to_string().to_ascii_lowercase();
    (text.contains("\"hardlimit\":true") || text.contains("\"hard_limit\":true"))
        && (text.contains("\"remainingmillionths\":0")
            || text.contains("\"remaining_millionths\":0"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn visible_output_is_preceded_by_boundary_transition() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        forward_provider_event(
            &tx,
            ProviderEvent::TextDelta {
                delta: "visible".to_string(),
            },
        )
        .await
        .expect("forward event");
        assert!(matches!(
            rx.recv().await,
            Some(ProviderEvent::AttemptBoundaryAdvanced {
                boundary: AttemptBoundary::VisibleOutputBegun,
                ..
            })
        ));
        assert!(matches!(
            rx.recv().await,
            Some(ProviderEvent::TextDelta { delta }) if delta == "visible"
        ));
    }

    #[tokio::test]
    async fn recoverable_rate_limit_is_promoted_before_original_error() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        forward_provider_event(
            &tx,
            ProviderEvent::Error {
                message: "HTTP 429 rate limit".to_string(),
                recoverable: true,
            },
        )
        .await
        .expect("forward event");
        assert!(matches!(
            rx.recv().await,
            Some(ProviderEvent::CapacityExhausted { .. })
        ));
        assert!(matches!(
            rx.recv().await,
            Some(ProviderEvent::Error {
                recoverable: true,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn credit_serviceable_full_window_is_not_promoted_to_capacity_exhaustion() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let details = serde_json::json!({
            "primary": {
                "usedPercent": 100,
                "windowDurationMins": 10080,
                "resetsAt": 1791948542
            },
            "credits": {
                "hasCredits": true,
                "unlimited": false,
                "balance": "100"
            },
            "spendControlReached": false,
            "rateLimitReachedType": "rate_limit_reached"
        });
        forward_provider_event(
            &tx,
            ProviderEvent::RateLimitObserved {
                details: details.clone(),
            },
        )
        .await
        .expect("forward event");
        assert!(matches!(
            rx.recv().await,
            Some(ProviderEvent::RateLimitObserved { details: observed }) if observed == details
        ));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn explicit_backend_block_is_promoted_before_rate_limit_observation() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let details = serde_json::json!({
            "primary": { "usedPercent": 100 },
            "credits": {
                "hasCredits": false,
                "unlimited": false,
                "balance": null
            },
            "spendControlReached": true,
            "rateLimitReachedType": "workspace_owner_usage_limit_reached"
        });
        forward_provider_event(
            &tx,
            ProviderEvent::RateLimitObserved {
                details: details.clone(),
            },
        )
        .await
        .expect("forward event");
        assert!(matches!(
            rx.recv().await,
            Some(ProviderEvent::CapacityExhausted { .. })
        ));
        assert!(matches!(
            rx.recv().await,
            Some(ProviderEvent::RateLimitObserved { details: observed }) if observed == details
        ));
    }
}
