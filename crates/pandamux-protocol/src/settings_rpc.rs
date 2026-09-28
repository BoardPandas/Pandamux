use pandamux_core::UserSettings;
use pandamux_core::ids::ProviderInstanceId;
use pandamux_core::provider_config::ProviderKind;
use serde::{Deserialize, Serialize};

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
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

/// Parameters for updating user settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSetParams {
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSetResult {
    pub success: bool,
}

/// Parameters for checking provider health.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthParams {
    /// Optional specific provider instance to check.
    #[serde(default)]
    pub instance_id: Option<ProviderInstanceId>,
}

/// Health report for an individual provider instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthReport {
    pub instance_id: ProviderInstanceId,
    pub provider: ProviderKind,
    pub display_name: String,
    pub enabled: bool,
    pub installed: bool,
    #[serde(default)]
    pub version: Option<String>,
    pub status: String,
    #[serde(default)]
    pub account_badge: Option<String>,
    #[serde(default)]
    pub subscription: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    pub checked_at_ms: u64,
}

/// Result of provider health checks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthResult {
    pub reports: Vec<ProviderHealthReport>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_rpc_serialization() {
        let set_params = SettingsSetParams {
            settings: Some(UserSettings::default()),
            key: None,
            value: None,
        };
        let json = serde_json::to_string(&set_params).unwrap();
        let parsed: SettingsSetParams = serde_json::from_str(&json).unwrap();
        assert!(parsed.settings.is_some());
    }

    #[test]
    fn test_provider_health_report_serialization() {
        let report = ProviderHealthReport {
            instance_id: ProviderInstanceId::new("claude-default"),
            provider: ProviderKind::Claude,
            display_name: "Claude Code".to_string(),
            enabled: true,
            installed: true,
            version: Some("0.2.29".to_string()),
            status: "healthy".to_string(),
            account_badge: Some("Claude Max".to_string()),
            subscription: Some("Max".to_string()),
            reason: None,
            checked_at_ms: 12345678,
        };

        let json = serde_json::to_string(&report).unwrap();
        let parsed: ProviderHealthReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, "healthy");
        assert_eq!(parsed.account_badge.as_deref(), Some("Claude Max"));
    }
}
