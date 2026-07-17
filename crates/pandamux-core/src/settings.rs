//! Persistent user settings (spec 2.6 supporting pipeline).
//!
//! [`UserSettings`] is the single schema behind `config/settings.json`, the
//! Settings UI, and the `config.get` / `config.set` pipe methods. Every field
//! is `#[serde(default)]` so files written by older builds keep loading, and
//! the dotted-path helpers ([`settings_get`] / [`settings_set`]) give the RPC
//! and future tooling one shared access path whose keys are exactly the
//! camelCase names in the file.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

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
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_SCHEMA_VERSION,
            ui: UiSettings::default(),
            terminal: TerminalSettings::default(),
            keyboard: KeyboardSettings::default(),
        }
    }
}

impl UserSettings {
    /// Clamp values into their valid ranges (applied after load and set).
    pub fn normalize(&mut self) {
        self.terminal.scrollback_lines = self
            .terminal
            .scrollback_lines
            .clamp(SCROLLBACK_LINES_MIN, SCROLLBACK_LINES_MAX);
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
    /// "teal" | "gold" | "blue" | "mauve"
    pub accent: String,
    pub show_status_bar: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            accent: "teal".to_string(),
            show_status_bar: true,
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
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            scrollback_lines: 10_000,
            welcome_prompt_enabled: true,
            right_click_paste_optin: false,
            confirm_close_on_running: true,
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
    }

    #[test]
    fn legacy_empty_json_loads_with_defaults() {
        let settings: UserSettings = serde_json::from_str("{}").expect("defaults");
        assert_eq!(settings.terminal.scrollback_lines, 10_000);
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
