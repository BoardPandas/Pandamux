//! SSH host profiles, `~/.ssh/config` import, and clipboard policy config
//! (plan F1/F2). These are the persistent, UI-facing description of a
//! connection; the actual russh work lives in `pandamux-term::ssh`. Passwords
//! are never stored here: a `Password` profile records only that a prompt is
//! needed.

use crate::SshProfileId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// How a host profile authenticates. A `Password` profile deliberately stores no
/// secret; the connect path prompts for it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SshAuthConfig {
    /// The Windows OpenSSH-compatible agent named pipe. This is the default and
    /// needs no stored material.
    #[default]
    Agent,
    /// A private key file (`IdentityFile`).
    KeyFile { path: String },
    /// Password auth; the secret is prompted for at connect time, never stored.
    Password,
}

/// A saved SSH host, imported from `~/.ssh/config` or entered by the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostProfile {
    /// Stable identity used by Projects. Friendly names can be edited safely.
    #[serde(default = "SshProfileId::generate")]
    pub id: SshProfileId,
    /// Display name (the `Host` alias).
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuthConfig,
    /// `ProxyJump` target (host alias), if any. Dialing through it is deferred
    /// glue work (plan Section 3); recorded here so nothing is lost on import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jump: Option<String>,
}

impl SshHostProfile {
    pub fn new(name: impl Into<String>, host: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            id: SshProfileId::generate(),
            name: name.into(),
            host: host.into(),
            port: 22,
            user: user.into(),
            auth: SshAuthConfig::default(),
            jump: None,
        }
    }
}

/// A registry of secretless host profiles, keyed by stable id.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshProfiles {
    pub profiles: Vec<SshHostProfile>,
}

impl SshProfiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a profile by stable id.
    pub fn upsert(&mut self, profile: SshHostProfile) {
        if let Some(existing) = self.profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile;
        } else {
            self.profiles.push(profile);
        }
    }

    pub fn get(&self, id: &SshProfileId) -> Option<&SshHostProfile> {
        self.profiles.iter().find(|p| &p.id == id)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&SshHostProfile> {
        self.profiles
            .iter()
            .find(|profile| profile.name.eq_ignore_ascii_case(name))
    }

    pub fn has_duplicate_name(&self, name: &str, except: Option<&SshProfileId>) -> bool {
        self.profiles
            .iter()
            .any(|profile| profile.name.eq_ignore_ascii_case(name) && except != Some(&profile.id))
    }

    pub fn remove(&mut self, id: &SshProfileId) -> bool {
        let before = self.profiles.len();
        self.profiles.retain(|profile| &profile.id != id);
        self.profiles.len() != before
    }

    /// Compatibility helper for the historical name-based RPC.
    pub fn remove_by_name(&mut self, name: &str) -> bool {
        let Some(id) = self.get_by_name(name).map(|profile| profile.id.clone()) else {
            return false;
        };
        self.remove(&id)
    }

    pub fn list(&self) -> &[SshHostProfile] {
        &self.profiles
    }

    /// Merge parsed `~/.ssh/config` entries in, upserting by name. Returns the
    /// names imported.
    pub fn import_config(&mut self, text: &str) -> Vec<String> {
        let parsed = parse_ssh_config(text);
        let names = parsed.iter().map(|p| p.name.clone()).collect();
        for mut profile in parsed {
            if let Some(existing) = self.get_by_name(&profile.name) {
                profile.id = existing.id.clone();
            }
            self.upsert(profile);
        }
        names
    }
}

/// Parse an OpenSSH `~/.ssh/config` into host profiles. Wildcard hosts (`Host *`)
/// are skipped (they are defaults, not connectable targets). `IdentityFile`
/// selects key-file auth; otherwise the agent is assumed.
pub fn parse_ssh_config(text: &str) -> Vec<SshHostProfile> {
    let mut profiles: Vec<SshHostProfile> = Vec::new();
    let mut current: Option<SshHostProfile> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (keyword, value) = match split_config_line(line) {
            Some(pair) => pair,
            None => continue,
        };

        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                if let Some(profile) = current.take() {
                    profiles.push(profile);
                }
                // Take the first alias; skip pure-wildcard patterns.
                let alias = value.split_whitespace().next().unwrap_or("");
                if alias.is_empty() || alias.contains('*') || alias.contains('?') {
                    current = None;
                } else {
                    current = Some(SshHostProfile::new(alias, alias, default_user()));
                }
            }
            "hostname" => {
                if let Some(profile) = current.as_mut() {
                    profile.host = value.to_string();
                }
            }
            "user" => {
                if let Some(profile) = current.as_mut() {
                    profile.user = value.to_string();
                }
            }
            "port" => {
                if let Some(profile) = current.as_mut()
                    && let Ok(port) = value.parse::<u16>()
                {
                    profile.port = port;
                }
            }
            "identityfile" => {
                if let Some(profile) = current.as_mut() {
                    profile.auth = SshAuthConfig::KeyFile {
                        path: expand_tilde(&value),
                    };
                }
            }
            "proxyjump" => {
                if let Some(profile) = current.as_mut() {
                    profile.jump = Some(value.to_string());
                }
            }
            _ => {}
        }
    }

    if let Some(profile) = current.take() {
        profiles.push(profile);
    }
    profiles
}

/// Split a config line into `(keyword, value)`, honoring `key value` and
/// `key=value` forms and stripping surrounding quotes on the value.
fn split_config_line(line: &str) -> Option<(&str, String)> {
    let (keyword, rest) = if let Some((k, v)) = line.split_once('=') {
        (k.trim(), v.trim())
    } else {
        let mut parts = line.splitn(2, char::is_whitespace);
        let keyword = parts.next()?.trim();
        let rest = parts.next().unwrap_or("").trim();
        (keyword, rest)
    };
    if keyword.is_empty() {
        return None;
    }
    Some((keyword, rest.trim_matches('"').to_string()))
}

fn expand_tilde(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return format!("{home}/{stripped}");
    }
    path.to_string()
}

fn home_dir() -> Option<String> {
    std::env::var("USERPROFILE")
        .ok()
        .or_else(|| std::env::var("HOME").ok())
}

fn default_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "root".to_string())
}

/// Persistent clipboard policy: the OSC 52 store size cap plus the set of hosts
/// allowed to *read* the local clipboard (the per-host load opt-in from the
/// plan). Deny-by-default: a host must be added explicitly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardConfig {
    pub max_store_bytes: usize,
    pub load_allowed_hosts: BTreeSet<String>,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            max_store_bytes: 1024 * 1024,
            load_allowed_hosts: BTreeSet::new(),
        }
    }
}

impl ClipboardConfig {
    /// Whether `host` may read the local clipboard (OSC 52 load).
    pub fn load_allowed(&self, host: &str) -> bool {
        self.load_allowed_hosts.contains(host)
    }

    pub fn allow_load(&mut self, host: impl Into<String>) {
        self.load_allowed_hosts.insert(host.into());
    }

    pub fn deny_load(&mut self, host: &str) {
        self.load_allowed_hosts.remove(host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hosts_with_hostname_user_port_and_identity() {
        let config = "\
Host galahad
    HostName 10.55.88.48
    User chaz
    Port 2222
    IdentityFile ~/.ssh/id_ed25519

Host *
    ForwardAgent yes

Host jumpbox
    HostName jump.example.com
    ProxyJump galahad
";
        let profiles = parse_ssh_config(config);
        assert_eq!(profiles.len(), 2, "wildcard host is skipped");

        let galahad = &profiles[0];
        assert_eq!(galahad.name, "galahad");
        assert_eq!(galahad.host, "10.55.88.48");
        assert_eq!(galahad.user, "chaz");
        assert_eq!(galahad.port, 2222);
        assert!(matches!(galahad.auth, SshAuthConfig::KeyFile { .. }));

        let jumpbox = &profiles[1];
        assert_eq!(jumpbox.host, "jump.example.com");
        assert_eq!(jumpbox.jump.as_deref(), Some("galahad"));
        assert_eq!(jumpbox.auth, SshAuthConfig::Agent);
    }

    #[test]
    fn parses_equals_form() {
        let profiles = parse_ssh_config("Host=web\nHostName=web.internal\n");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].host, "web.internal");
    }

    #[test]
    fn profiles_upsert_by_id_and_allow_rename() {
        let mut store = SshProfiles::new();
        let mut profile = SshHostProfile::new("a", "a.com", "root");
        let id = profile.id.clone();
        store.upsert(profile.clone());
        profile.name = "renamed".to_string();
        profile.host = "a2.com".to_string();
        store.upsert(profile);
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.get(&id).unwrap().name, "renamed");
        assert_eq!(store.get(&id).unwrap().host, "a2.com");
        assert!(store.remove(&id));
        assert!(store.list().is_empty());
    }

    #[test]
    fn import_config_merges_and_reports_names() {
        let mut store = SshProfiles::new();
        let names = store.import_config("Host box\n  HostName box.local\n");
        assert_eq!(names, vec!["box".to_string()]);
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn clipboard_config_denies_load_by_default() {
        let mut config = ClipboardConfig::default();
        assert!(!config.load_allowed("galahad"));
        config.allow_load("galahad");
        assert!(config.load_allowed("galahad"));
        config.deny_load("galahad");
        assert!(!config.load_allowed("galahad"));
    }
}
