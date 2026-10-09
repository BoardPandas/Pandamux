//! Persistent user settings (spec 2.6 supporting pipeline).
//!
//! [`UserSettings`] is the single schema behind `config/settings.json`, the
//! Settings UI, and the `config.get` / `config.set` pipe methods. Every field
//! is `#[serde(default)]` so files written by older builds keep loading, and
//! the dotted-path helpers ([`settings_get`] / [`settings_set`]) give the RPC
//! and future tooling one shared access path whose keys are exactly the
//! camelCase names in the file.

use crate::AgentId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const SETTINGS_SCHEMA_VERSION: u32 = 8;

/// Bounds for `terminal.scrollbackLines` (clamped on load and set).
pub const SCROLLBACK_LINES_MIN: u32 = 1_000;
pub const SCROLLBACK_LINES_MAX: u32 = 200_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserSettings {
    pub version: u32,
    pub ui: UiSettings,
    pub terminal: TerminalSettings,
    pub keyboard: KeyboardSettings,
    #[serde(default)]
    pub organizations: Vec<crate::organization::OrganizationPolicy>,
    #[serde(default)]
    pub failover_pools: Vec<crate::organization::FailoverPool>,
    #[serde(default = "default_tier_mapping_opt")]
    pub tier_mapping: Option<crate::organization::TierMapping>,
    #[serde(default = "default_providers")]
    pub providers: Vec<crate::provider_config::ProviderInstanceConfig>,
    #[serde(default = "default_environments")]
    pub environments: Vec<crate::environment::Environment>,
    #[serde(default)]
    pub trusted_configuration: TrustedConfigurationSettings,
    #[serde(default)]
    pub advanced: AdvancedSettings,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct TrustedConfigurationSettings {
    #[serde(default)]
    pub disabled_agent_ids: BTreeSet<AgentId>,
    #[serde(default)]
    pub full_access_agent_ids: BTreeSet<AgentId>,
}

pub fn default_environments() -> Vec<crate::environment::Environment> {
    vec![crate::environment::Environment::local_default()]
}

fn default_tier_mapping_opt() -> Option<crate::organization::TierMapping> {
    Some(default_tier_mapping())
}

pub fn default_tier_mapping() -> crate::organization::TierMapping {
    use crate::ids::ProviderInstanceId;
    use crate::organization::{ModelTarget, TierMapping};

    let mut tiers = BTreeMap::new();
    tiers.insert(
        "fast".to_string(),
        ModelTarget {
            provider_instance_id: ProviderInstanceId::new("claude-default"),
            model: "claude-3-5-haiku-latest".to_string(),
            effort: None,
        },
    );
    tiers.insert(
        "smart".to_string(),
        ModelTarget {
            provider_instance_id: ProviderInstanceId::new("claude-default"),
            model: "claude-3-5-sonnet-latest".to_string(),
            effort: None,
        },
    );
    tiers.insert(
        "reasoning".to_string(),
        ModelTarget {
            provider_instance_id: ProviderInstanceId::new("codex-default"),
            model: "default".to_string(),
            effort: Some("high".to_string()),
        },
    );
    tiers.insert(
        "orchestrator".to_string(),
        ModelTarget {
            provider_instance_id: ProviderInstanceId::new("claude-default"),
            model: "claude-3-7-sonnet-latest".to_string(),
            effort: None,
        },
    );
    TierMapping { tiers }
}

pub fn default_providers() -> Vec<crate::provider_config::ProviderInstanceConfig> {
    use crate::ids::ProviderInstanceId;
    use crate::provider_config::{ProviderDisabledReason, ProviderInstanceConfig, ProviderKind};

    vec![
        ProviderInstanceConfig {
            id: ProviderInstanceId::new("claude-default"),
            provider: ProviderKind::Claude,
            display_name: "Claude Code".to_string(),
            profile_dir: "profiles/claude/default".to_string(),
            environment_id: None,
            binary_path: None,
            concurrency_limit: 3,
            env_overrides: BTreeMap::new(),
            settings: serde_json::json!({
                "defaultModel": "claude-3-7-sonnet-latest"
            }),
            enabled: true,
            disabled_reason: None,
        },
        ProviderInstanceConfig {
            id: ProviderInstanceId::new("codex-default"),
            provider: ProviderKind::Codex,
            display_name: "Codex CLI".to_string(),
            profile_dir: "profiles/codex/default".to_string(),
            environment_id: None,
            binary_path: None,
            concurrency_limit: 3,
            env_overrides: BTreeMap::new(),
            settings: serde_json::json!({
                "defaultModel": "default"
            }),
            enabled: true,
            disabled_reason: None,
        },
        ProviderInstanceConfig {
            id: ProviderInstanceId::new("antigravity-default"),
            provider: ProviderKind::Antigravity,
            display_name: "Google Antigravity".to_string(),
            profile_dir: "profiles/antigravity/default".to_string(),
            environment_id: None,
            binary_path: None,
            concurrency_limit: 2,
            env_overrides: BTreeMap::new(),
            settings: serde_json::json!({
                "defaultModel": "gemini-2.0-flash"
            }),
            enabled: true,
            disabled_reason: None,
        },
        ProviderInstanceConfig {
            id: ProviderInstanceId::new("cursor-default"),
            provider: ProviderKind::Cursor,
            display_name: "Cursor Agent".to_string(),
            profile_dir: "profiles/cursor/default".to_string(),
            environment_id: None,
            binary_path: None,
            concurrency_limit: 2,
            env_overrides: BTreeMap::new(),
            settings: serde_json::json!({ "defaultModel": "cursor-fast" }),
            enabled: false,
            disabled_reason: Some(ProviderDisabledReason {
                code: "live_acceptance_incomplete".to_string(),
                message: "Cursor is disabled until its ACP fixture and live smoke pass"
                    .to_string(),
                remediation: Some(
                    "Install and authenticate cursor-agent, then run the common provider acceptance smoke"
                        .to_string(),
                ),
            }),
        },
        ProviderInstanceConfig {
            id: ProviderInstanceId::new("grok-default"),
            provider: ProviderKind::Grok,
            display_name: "xAI Grok".to_string(),
            profile_dir: "profiles/grok/default".to_string(),
            environment_id: None,
            binary_path: None,
            concurrency_limit: 2,
            env_overrides: BTreeMap::new(),
            settings: serde_json::json!({ "defaultModel": "grok-3" }),
            enabled: false,
            disabled_reason: Some(ProviderDisabledReason {
                code: "live_acceptance_incomplete".to_string(),
                message: "Grok is disabled until its ACP entry point and live smoke are verified"
                    .to_string(),
                remediation: Some(
                    "Configure a verified Grok ACP binary, authenticate it, and run the common provider acceptance smoke"
                        .to_string(),
                ),
            }),
        },
        ProviderInstanceConfig {
            id: ProviderInstanceId::new("opencode-default"),
            provider: ProviderKind::OpenCode,
            display_name: "OpenCode".to_string(),
            profile_dir: "profiles/opencode/default".to_string(),
            environment_id: None,
            binary_path: None,
            concurrency_limit: 2,
            env_overrides: BTreeMap::new(),
            settings: serde_json::json!({ "defaultModel": "opencode-coder" }),
            enabled: false,
            disabled_reason: Some(ProviderDisabledReason {
                code: "live_acceptance_incomplete".to_string(),
                message: "OpenCode is disabled until its ACP or serve-mode live smoke passes"
                    .to_string(),
                remediation: Some(
                    "Install and authenticate OpenCode, select its verified transport, and run the common provider acceptance smoke"
                        .to_string(),
                ),
            }),
        },
    ]
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_SCHEMA_VERSION,
            ui: UiSettings::default(),
            terminal: TerminalSettings::default(),
            keyboard: KeyboardSettings::default(),
            organizations: Vec::new(),
            failover_pools: Vec::new(),
            tier_mapping: Some(default_tier_mapping()),
            providers: default_providers(),
            environments: default_environments(),
            trusted_configuration: TrustedConfigurationSettings::default(),
            advanced: AdvancedSettings::default(),
        }
    }
}

impl UserSettings {
    /// Clamp values into their valid ranges (applied after load and set).
    pub fn normalize(&mut self) {
        if self.version < 3 {
            for provider in default_providers().into_iter().filter(|provider| {
                matches!(
                    provider.provider,
                    crate::provider_config::ProviderKind::Cursor
                        | crate::provider_config::ProviderKind::Grok
                        | crate::provider_config::ProviderKind::OpenCode
                )
            }) {
                if !self
                    .providers
                    .iter()
                    .any(|existing| existing.id == provider.id)
                {
                    self.providers.push(provider);
                }
            }
            self.version = 3;
        }
        if self.version < 4 {
            self.version = 4;
        }
        if self.version < 5 {
            self.version = 5;
        }
        if self.version < 6 {
            self.version = 6;
        }
        if self.version < 7 {
            for provider in &mut self.providers {
                if provider.provider == crate::provider_config::ProviderKind::Codex
                    && provider
                        .settings
                        .get("defaultModel")
                        .and_then(Value::as_str)
                        == Some("o3-mini")
                    && let Some(settings) = provider.settings.as_object_mut()
                {
                    settings.insert("defaultModel".to_string(), Value::String("default".into()));
                }
            }
            if let Some(target) = self
                .tier_mapping
                .as_mut()
                .and_then(|mapping| mapping.tiers.get_mut("reasoning"))
                && target.provider_instance_id.as_str() == "codex-default"
                && target.model == "o3-mini"
            {
                target.model = "default".to_string();
            }
            self.version = 7;
        }
        if self.version < 8 {
            self.version = 8;
        }
        for provider in &mut self.providers {
            if provider.enabled {
                provider.disabled_reason = None;
            }
        }
        self.terminal.scrollback_lines = self
            .terminal
            .scrollback_lines
            .clamp(SCROLLBACK_LINES_MIN, SCROLLBACK_LINES_MAX);
        if self.advanced.global_concurrency_limit == 0 {
            self.advanced.global_concurrency_limit = 1;
        }
        if self.advanced.max_checkpoints_per_thread == 0 {
            self.advanced.max_checkpoints_per_thread = 10;
        }
    }
}

/// Chrome preferences. Theme and accent are strings (not UI-crate enums) so
/// the schema stays framework-free; unknown values fall back to defaults at
/// the mapping layer in pandamux-app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiSettings {
    /// "dark" | "light"
    pub theme: String,
    /// "teal" | "gold" | "blue" | "purple"
    pub accent: String,
    pub show_status_bar: bool,
    pub sound_effects_enabled: bool,
    pub notifications_enabled: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            accent: "teal".to_string(),
            show_status_bar: true,
            sound_effects_enabled: true,
            notifications_enabled: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminalSettings {
    /// Scrollback history retained per session.
    pub scrollback_lines: u32,
    /// Show the tool chooser in fresh bare terminals (spec 2.7).
    pub welcome_prompt_enabled: bool,
    /// Opt-in classic right-click paste. Off by default: right-click opens
    /// the context menu (spec 1.3, explicitly decided).
    pub right_click_paste_optin: bool,
    /// Confirm closing a tab whose shell is still running (spec 2.6).
    pub confirm_close_on_running: bool,
    /// Preferred shell to launch in new bare terminals.
    pub preferred_shell: Option<String>,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            scrollback_lines: 10_000,
            welcome_prompt_enabled: true,
            right_click_paste_optin: false,
            confirm_close_on_running: true,
            preferred_shell: None,
        }
    }
}

/// Advanced engine, storage, and concurrency settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdvancedSettings {
    pub global_concurrency_limit: u32,
    pub max_checkpoints_per_thread: usize,
    pub max_image_size_bytes: u64,
    pub max_file_size_bytes: u64,
    pub max_turn_attachments_bytes: u64,
    pub usage_scanning_enabled: bool,
    pub usage_hub_sync_enabled: bool,
    pub capacity_routing_enabled: bool,
    pub automatic_failover_enabled: bool,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self {
            global_concurrency_limit: 8,
            max_checkpoints_per_thread: 50,
            max_image_size_bytes: crate::attachment::MAX_IMAGE_SIZE_BYTES,
            max_file_size_bytes: crate::attachment::MAX_FILE_SIZE_BYTES,
            max_turn_attachments_bytes: crate::attachment::MAX_TOTAL_ATTACHMENTS_PER_TURN_BYTES,
            usage_scanning_enabled: true,
            usage_hub_sync_enabled: true,
            capacity_routing_enabled: true,
            automatic_failover_enabled: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeyboardSettings {
    /// Chords always passed through to the terminal even when bound
    /// (e.g. "ctrl+1"). Consumed by the keymap.
    pub pass_through: Vec<String>,
    /// Keymap overrides: action id -> chord string, or null to unbind.
    /// Unknown ids and unparseable chords warn and keep the default.
    pub overrides: BTreeMap<String, Option<String>>,
}

/// Read a settings value by dotted camelCase path ("terminal.scrollbackLines").
/// An empty path returns the whole settings object.
pub fn settings_get(settings: &UserSettings, key: &str) -> Result<Value, String> {
    let root = serde_json::to_value(settings).map_err(|error| error.to_string())?;
    if key.is_empty() {
        return Ok(root);
    }
    let mut node = &root;
    for segment in key.split('.') {
        node = node
            .get(segment)
            .ok_or_else(|| format!("unknown settings key: {key}"))?;
    }
    Ok(node.clone())
}

/// Write a settings value by dotted camelCase path. The path must exist in the
/// schema (except under `keyboard.overrides`, where insertion is allowed) and
/// the value must deserialize back into [`UserSettings`], so type mismatches
/// are rejected with the serde error.
pub fn settings_set(settings: &mut UserSettings, key: &str, value: Value) -> Result<(), String> {
    if key.is_empty() || key == "version" {
        return Err(format!("settings key cannot be set: {key:?}"));
    }
    let mut root = serde_json::to_value(&*settings).map_err(|error| error.to_string())?;
    {
        let segments: Vec<&str> = key.split('.').collect();
        let (last, parents) = segments.split_last().expect("non-empty key");
        let mut node = &mut root;
        for segment in parents {
            node = node
                .get_mut(*segment)
                .ok_or_else(|| format!("unknown settings key: {key}"))?;
        }
        let map = node
            .as_object_mut()
            .ok_or_else(|| format!("settings key has no children: {key}"))?;
        let insertable = parents == ["keyboard", "overrides"];
        if !insertable && !map.contains_key(*last) {
            return Err(format!("unknown settings key: {key}"));
        }
        map.insert((*last).to_string(), value);
    }
    let mut updated: UserSettings = serde_json::from_value(root)
        .map_err(|error| format!("invalid value for {key}: {error}"))?;
    updated.version = settings.version;
    updated.normalize();
    *settings = updated;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_config::ProviderKind;
    use serde_json::json;

    #[test]
    fn defaults_match_the_spec() {
        let settings = UserSettings::default();
        assert_eq!(settings.terminal.scrollback_lines, 10_000);
        assert!(settings.terminal.welcome_prompt_enabled);
        assert!(!settings.terminal.right_click_paste_optin);
        assert!(settings.terminal.confirm_close_on_running);
        assert_eq!(settings.ui.theme, "dark");
        assert!(settings.ui.show_status_bar);
        assert!(settings.keyboard.pass_through.is_empty());
        assert!(settings.advanced.usage_scanning_enabled);
        assert!(settings.advanced.usage_hub_sync_enabled);
        assert!(settings.advanced.capacity_routing_enabled);
        assert!(settings.advanced.automatic_failover_enabled);
        assert!(settings.trusted_configuration.disabled_agent_ids.is_empty());
        assert!(
            settings
                .trusted_configuration
                .full_access_agent_ids
                .is_empty()
        );
        let disabled_best_effort = settings
            .providers
            .iter()
            .filter(|provider| {
                matches!(
                    provider.provider,
                    ProviderKind::Cursor | ProviderKind::Grok | ProviderKind::OpenCode
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(disabled_best_effort.len(), 3);
        for provider in disabled_best_effort {
            assert!(!provider.enabled);
            assert_eq!(
                provider
                    .disabled_reason
                    .as_ref()
                    .map(|reason| reason.code.as_str()),
                Some("live_acceptance_incomplete")
            );
        }
    }

    #[test]
    fn legacy_empty_json_loads_with_defaults() {
        let settings: UserSettings = serde_json::from_str("{}").expect("defaults");
        assert_eq!(settings.terminal.scrollback_lines, 10_000);
        assert!(settings.advanced.usage_scanning_enabled);
        assert!(settings.advanced.usage_hub_sync_enabled);
        assert!(settings.advanced.capacity_routing_enabled);
        assert!(settings.advanced.automatic_failover_enabled);
    }

    #[test]
    fn version_two_settings_gain_disabled_best_effort_providers() {
        let mut settings = UserSettings {
            version: 2,
            ..UserSettings::default()
        };
        settings.providers.retain(|provider| {
            !matches!(
                provider.provider,
                ProviderKind::Cursor | ProviderKind::Grok | ProviderKind::OpenCode
            )
        });

        settings.normalize();

        assert_eq!(settings.version, SETTINGS_SCHEMA_VERSION);
        for kind in [
            ProviderKind::Cursor,
            ProviderKind::Grok,
            ProviderKind::OpenCode,
        ] {
            let provider = settings
                .providers
                .iter()
                .find(|provider| provider.provider == kind)
                .expect("migrated best-effort provider");
            assert!(!provider.enabled);
            assert!(provider.disabled_reason.is_some());
        }

        let cursor = settings
            .providers
            .iter_mut()
            .find(|provider| provider.provider == ProviderKind::Cursor)
            .expect("migrated Cursor provider");
        cursor.enabled = true;
        settings.normalize();
        assert!(
            settings
                .providers
                .iter()
                .find(|provider| provider.provider == ProviderKind::Cursor)
                .expect("enabled Cursor provider")
                .disabled_reason
                .is_none()
        );
    }

    #[test]
    fn version_six_migrates_only_the_shipped_codex_model_default() {
        let mut settings = UserSettings {
            version: 6,
            ..UserSettings::default()
        };
        let codex = settings
            .providers
            .iter_mut()
            .find(|provider| provider.provider == ProviderKind::Codex)
            .expect("Codex default provider");
        codex.settings["defaultModel"] = Value::String("o3-mini".to_string());
        settings
            .tier_mapping
            .as_mut()
            .expect("default tier mapping")
            .tiers
            .get_mut("reasoning")
            .expect("reasoning tier")
            .model = "o3-mini".to_string();

        settings.normalize();

        assert_eq!(settings.version, SETTINGS_SCHEMA_VERSION);
        let codex = settings
            .providers
            .iter()
            .find(|provider| provider.provider == ProviderKind::Codex)
            .expect("migrated Codex provider");
        assert_eq!(codex.settings["defaultModel"], "default");
        assert_eq!(
            settings
                .tier_mapping
                .as_ref()
                .expect("migrated tier mapping")
                .tiers["reasoning"]
                .model,
            "default"
        );

        settings.version = 6;
        let codex = settings
            .providers
            .iter_mut()
            .find(|provider| provider.provider == ProviderKind::Codex)
            .expect("Codex provider");
        codex.settings["defaultModel"] = Value::String("gpt-custom".to_string());
        settings
            .tier_mapping
            .as_mut()
            .expect("tier mapping")
            .tiers
            .get_mut("reasoning")
            .expect("reasoning tier")
            .model = "gpt-custom-tier".to_string();
        settings.normalize();
        assert_eq!(
            settings
                .providers
                .iter()
                .find(|provider| provider.provider == ProviderKind::Codex)
                .expect("custom Codex provider")
                .settings["defaultModel"],
            "gpt-custom"
        );
        assert_eq!(
            settings
                .tier_mapping
                .as_ref()
                .expect("custom tier mapping")
                .tiers["reasoning"]
                .model,
            "gpt-custom-tier"
        );
    }

    #[test]
    fn get_and_set_round_trip_by_dotted_path() {
        let mut settings = UserSettings::default();
        assert_eq!(
            settings_get(&settings, "terminal.scrollbackLines").unwrap(),
            json!(10_000)
        );
        settings_set(&mut settings, "terminal.scrollbackLines", json!(50_000)).unwrap();
        assert_eq!(settings.terminal.scrollback_lines, 50_000);
        settings_set(&mut settings, "ui.theme", json!("light")).unwrap();
        assert_eq!(settings.ui.theme, "light");
        settings_set(
            &mut settings,
            "trustedConfiguration.disabledAgentIds",
            json!(["builder"]),
        )
        .unwrap();
        assert!(
            settings
                .trusted_configuration
                .disabled_agent_ids
                .contains(&AgentId::new("builder"))
        );
    }

    #[test]
    fn set_rejects_unknown_keys_and_wrong_types() {
        let mut settings = UserSettings::default();
        assert!(settings_set(&mut settings, "terminal.scrollbak", json!(1)).is_err());
        assert!(settings_set(&mut settings, "nope", json!(1)).is_err());
        assert!(settings_set(&mut settings, "version", json!(9)).is_err());
        assert!(
            settings_set(&mut settings, "terminal.scrollbackLines", json!("many")).is_err(),
            "type mismatch must be rejected"
        );
        // The failed sets left everything untouched.
        assert_eq!(settings, UserSettings::default());
    }

    #[test]
    fn set_allows_inserting_keyboard_overrides() {
        let mut settings = UserSettings::default();
        settings_set(
            &mut settings,
            "keyboard.overrides.palette.open",
            json!("ctrl+j"),
        )
        .unwrap_err();
        // Dots inside action ids are not paths; overrides are set as a map.
        settings_set(
            &mut settings,
            "keyboard.overrides",
            json!({"tab.close": "ctrl+shift+w", "find.open": null}),
        )
        .unwrap();
        assert_eq!(
            settings.keyboard.overrides.get("tab.close"),
            Some(&Some("ctrl+shift+w".to_string()))
        );
        assert_eq!(settings.keyboard.overrides.get("find.open"), Some(&None));
    }

    #[test]
    fn scrollback_clamps_on_set() {
        let mut settings = UserSettings::default();
        settings_set(&mut settings, "terminal.scrollbackLines", json!(5)).unwrap();
        assert_eq!(settings.terminal.scrollback_lines, SCROLLBACK_LINES_MIN);
        settings_set(&mut settings, "terminal.scrollbackLines", json!(10_000_000)).unwrap();
        assert_eq!(settings.terminal.scrollback_lines, SCROLLBACK_LINES_MAX);
    }
}
