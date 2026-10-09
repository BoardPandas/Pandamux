use pandamux_core::ids::{EnvironmentId, ProviderInstanceId};
use pandamux_core::provider_config::{ProviderDisabledReason, ProviderKind};
use pandamux_core::{ConfigurationRevision, UserSettings};
use serde::{Deserialize, Serialize};

use crate::provider_rpc::ProviderLoginGuidance;

pub const DEFAULT_PROVIDER_HEALTH_PAGE_SIZE: u32 = 100;
pub const MAX_PROVIDER_HEALTH_PAGE_SIZE: u32 = 500;
pub const DEFAULT_PROVIDER_PROBE_TIMEOUT_MS: u64 = 5_000;
pub const MIN_PROVIDER_PROBE_TIMEOUT_MS: u64 = 25;
pub const MAX_PROVIDER_PROBE_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_PROVIDER_PROBE_CONCURRENCY: u32 = 8;
pub const MAX_PROVIDER_PROBE_CONCURRENCY: u32 = 32;

fn default_provider_health_page_size() -> u32 {
    DEFAULT_PROVIDER_HEALTH_PAGE_SIZE
}

fn default_provider_probe_timeout_ms() -> u64 {
    DEFAULT_PROVIDER_PROBE_TIMEOUT_MS
}

fn default_provider_probe_concurrency() -> u32 {
    DEFAULT_PROVIDER_PROBE_CONCURRENCY
}

/// Parameters for retrieving application user settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsGetParams {
    /// Optional dotted camelCase key (e.g. "terminal.scrollbackLines").
    #[serde(default)]
    pub key: Option<String>,
}

/// Result returned from settings.get.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsGetResult {
    pub settings: UserSettings,
    pub revision: ConfigurationRevision,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

/// Parameters for updating user settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSetParams {
    pub expected_revision: ConfigurationRevision,
    /// Full updated settings object.
    #[serde(default)]
    pub settings: Option<UserSettings>,
    /// Or a specific dotted-path key.
    #[serde(default)]
    pub key: Option<String>,
    /// Value for the dotted-path key if provided.
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

/// Result returned from settings.set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSetResult {
    pub success: bool,
    pub revision: ConfigurationRevision,
}

/// Parameters for checking provider health.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthParams {
    /// Optional specific provider instance to check.
    #[serde(default)]
    pub instance_id: Option<ProviderInstanceId>,
    /// Environment whose instance overrides must be applied.
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    /// Stable keyset cursor. Only instance ids after this value are returned.
    #[serde(default)]
    pub after_instance_id: Option<ProviderInstanceId>,
    /// Maximum number of instances to probe and return.
    #[serde(default = "default_provider_health_page_size")]
    pub limit: u32,
    /// Deadline applied after a probe acquires a global concurrency permit.
    #[serde(default = "default_provider_probe_timeout_ms")]
    pub timeout_ms: u64,
    /// Maximum number of provider probes executing concurrently.
    #[serde(default = "default_provider_probe_concurrency")]
    pub max_concurrency: u32,
}

impl Default for ProviderHealthParams {
    fn default() -> Self {
        Self {
            instance_id: None,
            environment_id: None,
            after_instance_id: None,
            limit: DEFAULT_PROVIDER_HEALTH_PAGE_SIZE,
            timeout_ms: DEFAULT_PROVIDER_PROBE_TIMEOUT_MS,
            max_concurrency: DEFAULT_PROVIDER_PROBE_CONCURRENCY,
        }
    }
}

impl ProviderHealthParams {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_PROVIDER_HEALTH_PAGE_SIZE).contains(&self.limit) {
            return Err(format!(
                "limit must be between 1 and {MAX_PROVIDER_HEALTH_PAGE_SIZE}"
            ));
        }
        if !(MIN_PROVIDER_PROBE_TIMEOUT_MS..=MAX_PROVIDER_PROBE_TIMEOUT_MS)
            .contains(&self.timeout_ms)
        {
            return Err(format!(
                "timeoutMs must be between {MIN_PROVIDER_PROBE_TIMEOUT_MS} and {MAX_PROVIDER_PROBE_TIMEOUT_MS}"
            ));
        }
        if !(1..=MAX_PROVIDER_PROBE_CONCURRENCY).contains(&self.max_concurrency) {
            return Err(format!(
                "maxConcurrency must be between 1 and {MAX_PROVIDER_PROBE_CONCURRENCY}"
            ));
        }
        for (field, value) in [
            (
                "instanceId",
                self.instance_id.as_ref().map(ProviderInstanceId::as_str),
            ),
            (
                "environmentId",
                self.environment_id.as_ref().map(EnvironmentId::as_str),
            ),
            (
                "afterInstanceId",
                self.after_instance_id
                    .as_ref()
                    .map(ProviderInstanceId::as_str),
            ),
        ] {
            if value.is_some_and(|value| value.trim().is_empty()) {
                return Err(format!("{field} must be absent or non-empty"));
            }
        }
        Ok(())
    }
}

/// Health report for an individual provider instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthReport {
    pub environment_id: EnvironmentId,
    pub instance_id: ProviderInstanceId,
    pub provider: ProviderKind,
    pub display_name: String,
    pub enabled: bool,
    pub installed: bool,
    #[serde(default)]
    pub resolved_executable: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub status: String,
    pub authentication_state: String,
    pub authentication_confidence: String,
    pub runtime_status: String,
    #[serde(default)]
    pub runtime_code: Option<String>,
    #[serde(default)]
    pub account_badge: Option<String>,
    #[serde(default)]
    pub subscription: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<ProviderDisabledReason>,
    pub checked_at_ms: u64,
    pub expires_at_ms: u64,
    #[serde(default)]
    pub last_success_at_ms: Option<u64>,
    #[serde(default)]
    pub login_guidance: Option<ProviderLoginGuidance>,
    #[serde(default)]
    pub retryable: bool,
}

/// Result of provider health checks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthResult {
    pub reports: Vec<ProviderHealthReport>,
    #[serde(default)]
    pub next_instance_id: Option<ProviderInstanceId>,
    #[serde(default)]
    pub has_more: bool,
}

/// One model returned by a configured provider instance's bounded discovery call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelInfo {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    pub supports_reasoning: bool,
}

/// Model-discovery result for one configured provider instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelsReport {
    pub environment_id: EnvironmentId,
    pub instance_id: ProviderInstanceId,
    pub provider: ProviderKind,
    pub display_name: String,
    pub enabled: bool,
    #[serde(default)]
    pub models: Vec<ProviderModelInfo>,
    pub checked_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub retryable: bool,
}

/// Bounded model-discovery page. It intentionally reuses
/// [`ProviderHealthParams`] so health and model discovery have identical
/// pagination, timeout, and concurrency limits.
pub type ProviderModelsParams = ProviderHealthParams;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelsResult {
    pub reports: Vec<ProviderModelsReport>,
    #[serde(default)]
    pub next_instance_id: Option<ProviderInstanceId>,
    #[serde(default)]
    pub has_more: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_rpc_serialization() {
        let set_params = SettingsSetParams {
            expected_revision: ConfigurationRevision::INITIAL,
            settings: Some(UserSettings::default()),
            key: None,
            value: None,
        };
        let json = serde_json::to_string(&set_params).unwrap();
        let parsed: SettingsSetParams = serde_json::from_str(&json).unwrap();
        assert!(parsed.settings.is_some());
        assert_eq!(parsed.expected_revision, ConfigurationRevision::INITIAL);

        let missing_revision = serde_json::json!({
            "settings": UserSettings::default()
        });
        assert!(serde_json::from_value::<SettingsSetParams>(missing_revision).is_err());
    }

    #[test]
    fn test_provider_health_report_serialization() {
        let report = ProviderHealthReport {
            environment_id: EnvironmentId::new("env-local"),
            instance_id: ProviderInstanceId::new("claude-default"),
            provider: ProviderKind::Claude,
            display_name: "Claude Code".to_string(),
            enabled: true,
            installed: true,
            resolved_executable: Some("C:\\Tools\\claude.exe".to_string()),
            version: Some("0.2.29".to_string()),
            status: "healthy".to_string(),
            authentication_state: "authenticated".to_string(),
            authentication_confidence: "reported".to_string(),
            runtime_status: "healthy".to_string(),
            runtime_code: Some("runtime_ready".to_string()),
            account_badge: Some("Claude Max".to_string()),
            subscription: Some("Max".to_string()),
            reason: None,
            disabled_reason: None,
            checked_at_ms: 12345678,
            expires_at_ms: 12645678,
            last_success_at_ms: Some(12345678),
            login_guidance: None,
            retryable: true,
        };

        let json = serde_json::to_string(&report).unwrap();
        let parsed: ProviderHealthReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, "healthy");
        assert_eq!(parsed.account_badge.as_deref(), Some("Claude Max"));
    }

    #[test]
    fn provider_health_page_contract_is_stable_and_bounded() {
        let params: ProviderHealthParams = serde_json::from_str("{}").unwrap();
        assert_eq!(params.limit, DEFAULT_PROVIDER_HEALTH_PAGE_SIZE);
        assert!(params.validate().is_ok());

        let fixture = ProviderHealthParams {
            instance_id: None,
            environment_id: Some(EnvironmentId::new("env-local")),
            after_instance_id: Some(ProviderInstanceId::new("claude-default")),
            limit: 25,
            timeout_ms: 2_000,
            max_concurrency: 4,
        };
        assert_eq!(
            serde_json::to_value(fixture).unwrap(),
            serde_json::json!({
                "instanceId": null,
                "environmentId": "env-local",
                "afterInstanceId": "claude-default",
                "limit": 25,
                "timeoutMs": 2000,
                "maxConcurrency": 4
            })
        );

        let invalid = ProviderHealthParams {
            limit: MAX_PROVIDER_HEALTH_PAGE_SIZE + 1,
            ..ProviderHealthParams::default()
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn provider_models_report_round_trips_without_fabricating_models() {
        let report = ProviderModelsReport {
            environment_id: EnvironmentId::new("env-local"),
            instance_id: ProviderInstanceId::new("codex-default"),
            provider: ProviderKind::Codex,
            display_name: "Codex CLI".to_string(),
            enabled: true,
            models: vec![ProviderModelInfo {
                id: "gpt-test".to_string(),
                display_name: "GPT Test".to_string(),
                context_window: None,
                max_output_tokens: None,
                supports_reasoning: true,
            }],
            checked_at_ms: 123,
            error: None,
            retryable: false,
        };

        let json = serde_json::to_string(&report).expect("serialize model report");
        let parsed: ProviderModelsReport =
            serde_json::from_str(&json).expect("deserialize model report");
        assert_eq!(parsed, report);

        let unavailable: ProviderModelsReport = serde_json::from_value(serde_json::json!({
            "environmentId": "env-local",
            "instanceId": "codex-default",
            "provider": "codex",
            "displayName": "Codex CLI",
            "enabled": true,
            "checkedAtMs": 124,
            "error": "not authenticated"
        }))
        .expect("deserialize unavailable model report");
        assert!(unavailable.models.is_empty());
        assert!(!unavailable.retryable);
    }
}
