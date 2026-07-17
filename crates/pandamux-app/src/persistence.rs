//! Session persistence ported from the Electron `session-persistence.ts`.
//!
//! - `session.json`: the auto-restored layout (atomic write; overwritten on
//!   every autosave). Cleared on a version change so a new build starts clean.
//! - `saved/<name>.json`: explicitly named sessions the user chose to keep.
//!   These survive version changes.
//! - `last-session.txt`: pointer to the most recently saved named session.
//! - `app-version.txt`: the version that last wrote `session.json`.
//!
//! [`SessionStore`] takes its base directory by value so it is unit-testable
//! against a temp dir; [`SessionStore::default_dir`] resolves the real location.

use pandamux_core::{AppState, SshHostProfile, SshProfileId, SshProfiles};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A named session snapshot: the app state plus its save metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedSession {
    pub name: String,
    pub saved_at_ms: u64,
    pub state: AppState,
}

/// Summary row for the named-session list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedSessionSummary {
    pub name: String,
    pub saved_at_ms: u64,
    pub workspace_count: usize,
}

pub struct SessionStore {
    base: PathBuf,
}

const SSH_PROFILE_SCHEMA_VERSION: u32 = 1;

/// Secretless SSH connection settings. Credentials intentionally have no field
/// in this schema, so they cannot leak through a future save call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshProfileConfig {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub profiles: Vec<SshHostProfile>,
    #[serde(default)]
    pub last_selected_folder_by_profile: BTreeMap<SshProfileId, String>,
    #[serde(default)]
    pub last_selected_local_folder: Option<String>,
}

impl Default for SshProfileConfig {
    fn default() -> Self {
        Self {
            version: SSH_PROFILE_SCHEMA_VERSION,
            profiles: Vec::new(),
            last_selected_folder_by_profile: BTreeMap::new(),
            last_selected_local_folder: None,
        }
    }
}

impl SshProfileConfig {
    pub fn registry(&self) -> SshProfiles {
        SshProfiles {
            profiles: self.profiles.clone(),
        }
    }

    pub fn set_registry(&mut self, profiles: &SshProfiles) {
        self.profiles = profiles.profiles.clone();
    }
}

#[derive(Debug)]
pub enum SshProfileStoreError {
    Io(io::Error),
    Corrupt { path: PathBuf, message: String },
    UnsupportedVersion(u32),
}

impl fmt::Display for SshProfileStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Corrupt { path, message } => {
                write!(
                    formatter,
                    "SSH profile file {} is corrupt: {message}",
                    path.display()
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported SSH profile schema version {version}"
                )
            }
        }
    }
}

impl std::error::Error for SshProfileStoreError {}

impl From<io::Error> for SshProfileStoreError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub struct SshProfileStore {
    base: PathBuf,
}

impl SshProfileStore {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    pub fn default_dir() -> PathBuf {
        SessionStore::default_dir().join("config")
    }

    pub fn path(&self) -> PathBuf {
        self.base.join("ssh-profiles.json")
    }

    pub fn save(&self, config: &SshProfileConfig) -> Result<(), SshProfileStoreError> {
        fs::create_dir_all(&self.base)?;
        let mut config = config.clone();
        config.version = SSH_PROFILE_SCHEMA_VERSION;
        let json = serde_json::to_string_pretty(&config).map_err(io::Error::other)?;
        atomic_write(&self.path(), &json)?;
        Ok(())
    }

    /// Load and, when needed, migrate with a version-stamped backup written
    /// before the original file is replaced. Invalid JSON is left untouched.
    pub fn load(&self) -> Result<SshProfileConfig, SshProfileStoreError> {
        let path = self.path();
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(SshProfileConfig::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut config: SshProfileConfig =
            serde_json::from_str(&raw).map_err(|error| SshProfileStoreError::Corrupt {
                path: path.clone(),
                message: error.to_string(),
            })?;
        if config.version > SSH_PROFILE_SCHEMA_VERSION {
            return Err(SshProfileStoreError::UnsupportedVersion(config.version));
        }
        if config.version < SSH_PROFILE_SCHEMA_VERSION {
            let backup = self
                .base
                .join(format!("ssh-profiles.v{}.bak.json", config.version));
            fs::create_dir_all(&self.base)?;
            fs::copy(&path, backup)?;
            config.version = SSH_PROFILE_SCHEMA_VERSION;
            self.save(&config)?;
        }
        Ok(config)
    }
}

#[derive(Debug)]
pub enum SettingsStoreError {
    Io(io::Error),
    Corrupt { path: PathBuf, message: String },
    UnsupportedVersion(u32),
}

impl fmt::Display for SettingsStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Corrupt { path, message } => {
                write!(
                    formatter,
                    "settings file {} is corrupt: {message}",
                    path.display()
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported settings schema version {version}")
            }
        }
    }
}

impl std::error::Error for SettingsStoreError {}

impl From<io::Error> for SettingsStoreError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// `config/settings.json`: the persistent [`UserSettings`] behind the Settings
/// UI and the `config.get` / `config.set` pipe methods. Mirrors
/// [`SshProfileStore`]: versioned schema, migration writes a version-stamped
/// backup, corrupt files are preserved untouched (the caller refuses saves so
/// a broken file is never clobbered).
pub struct SettingsStore {
    base: PathBuf,
}

impl SettingsStore {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    pub fn default_dir() -> PathBuf {
        SessionStore::default_dir().join("config")
    }

    pub fn path(&self) -> PathBuf {
        self.base.join("settings.json")
    }

    pub fn save(&self, settings: &pandamux_core::UserSettings) -> Result<(), SettingsStoreError> {
        fs::create_dir_all(&self.base)?;
        let mut settings = settings.clone();
        settings.version = pandamux_core::SETTINGS_SCHEMA_VERSION;
        let json = serde_json::to_string_pretty(&settings).map_err(io::Error::other)?;
        atomic_write(&self.path(), &json)?;
        Ok(())
    }

    /// Load and, when needed, migrate with a version-stamped backup written
    /// before the original file is replaced. Invalid JSON is left untouched.
    pub fn load(&self) -> Result<pandamux_core::UserSettings, SettingsStoreError> {
        let path = self.path();
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(pandamux_core::UserSettings::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut settings: pandamux_core::UserSettings =
            serde_json::from_str(&raw).map_err(|error| SettingsStoreError::Corrupt {
                path: path.clone(),
                message: error.to_string(),
            })?;
        if settings.version > pandamux_core::SETTINGS_SCHEMA_VERSION {
            return Err(SettingsStoreError::UnsupportedVersion(settings.version));
        }
        if settings.version < pandamux_core::SETTINGS_SCHEMA_VERSION {
            let backup = self
                .base
                .join(format!("settings.v{}.bak.json", settings.version));
            fs::create_dir_all(&self.base)?;
            fs::copy(&path, backup)?;
            settings.version = pandamux_core::SETTINGS_SCHEMA_VERSION;
            self.save(&settings)?;
        }
        settings.normalize();
        Ok(settings)
    }
}

impl SessionStore {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    /// The real per-user data directory (`%APPDATA%/pandamux` on Windows).
    pub fn default_dir() -> PathBuf {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("pandamux");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".pandamux");
        }
        std::env::temp_dir().join("pandamux")
    }

    fn sessions_dir(&self) -> PathBuf {
        self.base.join("sessions")
    }
    fn session_file(&self) -> PathBuf {
        self.sessions_dir().join("session.json")
    }
    fn saved_dir(&self) -> PathBuf {
        self.sessions_dir().join("saved")
    }
    fn version_file(&self) -> PathBuf {
        self.base.join("app-version.txt")
    }
    fn last_session_file(&self) -> PathBuf {
        self.sessions_dir().join("last-session.txt")
    }

    pub fn ensure_dirs(&self) -> io::Result<()> {
        fs::create_dir_all(self.sessions_dir())
    }

    /// Atomically persist the auto-restore session (temp file + rename).
    pub fn save_session(&self, state: &AppState) -> io::Result<()> {
        self.ensure_dirs()?;
        let json = serde_json::to_string_pretty(state)?;
        atomic_write(&self.session_file(), &json)
    }

    /// Load the auto-restore session, or `None` if missing/corrupt.
    pub fn load_session(&self) -> Option<AppState> {
        read_json(&self.session_file())
    }

    /// Save a named session (and update the last-session pointer).
    pub fn save_named(&self, name: &str, saved_at_ms: u64, state: &AppState) -> io::Result<()> {
        fs::create_dir_all(self.saved_dir())?;
        let session = NamedSession {
            name: name.to_string(),
            saved_at_ms,
            state: state.clone(),
        };
        let json = serde_json::to_string_pretty(&session)?;
        atomic_write(&self.named_file(name), &json)?;
        self.set_last_session_name(name)
    }

    pub fn load_named(&self, name: &str) -> Option<NamedSession> {
        read_json(&self.named_file(name))
    }

    /// List named sessions, newest first.
    pub fn list_named(&self) -> Vec<NamedSessionSummary> {
        let Ok(entries) = fs::read_dir(self.saved_dir()) else {
            return Vec::new();
        };
        let mut sessions: Vec<NamedSessionSummary> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| read_json::<NamedSession>(&entry.path()))
            .map(|session| NamedSessionSummary {
                name: session.name,
                saved_at_ms: session.saved_at_ms,
                workspace_count: session.state.workspaces.len(),
            })
            .collect();
        sessions.sort_by(|a, b| b.saved_at_ms.cmp(&a.saved_at_ms));
        sessions
    }

    pub fn delete_named(&self, name: &str) -> bool {
        fs::remove_file(self.named_file(name)).is_ok()
    }

    pub fn last_session_name(&self) -> Option<String> {
        let raw = fs::read_to_string(self.last_session_file()).ok()?;
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    pub fn set_last_session_name(&self, name: &str) -> io::Result<()> {
        fs::create_dir_all(self.saved_dir())?;
        fs::write(self.last_session_file(), name)
    }

    /// Returns true if the app version changed (or first launch), clearing ONLY
    /// the volatile auto-restore `session.json`. Named sessions and the
    /// last-session pointer are intentionally preserved across updates.
    pub fn handle_version_change(&self, current_version: &str) -> bool {
        if self.ensure_dirs().is_err() {
            return false;
        }
        let saved = fs::read_to_string(self.version_file())
            .map(|raw| raw.trim().to_string())
            .unwrap_or_default();
        if saved == current_version {
            return false;
        }
        let _ = fs::remove_file(self.session_file());
        let _ = fs::write(self.version_file(), current_version);
        true
    }

    fn named_file(&self, name: &str) -> PathBuf {
        self.saved_dir()
            .join(format!("{}.json", sanitize_name(name)))
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ' ') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect()
}

/// Atomic write: write to a temp sibling, remove any existing target (Windows
/// rename will not overwrite), then rename into place.
fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, contents)?;
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp, path)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{AppIntent, PaneIntent, SplitDirection, SplitPaneParams, SurfaceType};

    fn temp_store(tag: &str) -> SessionStore {
        let dir = std::env::temp_dir().join(format!("pandamux-persist-test-{tag}"));
        let _ = fs::remove_dir_all(&dir);
        SessionStore::new(dir)
    }

    fn temp_profile_store(tag: &str) -> SshProfileStore {
        let dir = std::env::temp_dir().join(format!("pandamux-profile-test-{tag}"));
        let _ = fs::remove_dir_all(&dir);
        SshProfileStore::new(dir)
    }

    fn temp_settings_store(tag: &str) -> SettingsStore {
        let dir = std::env::temp_dir().join(format!("pandamux-settings-test-{tag}"));
        let _ = fs::remove_dir_all(&dir);
        SettingsStore::new(dir)
    }

    #[test]
    fn settings_roundtrip_and_missing_file_defaults() {
        let store = temp_settings_store("roundtrip");
        assert_eq!(
            store.load().expect("missing file loads defaults"),
            pandamux_core::UserSettings::default()
        );
        let mut settings = pandamux_core::UserSettings::default();
        settings.terminal.scrollback_lines = 50_000;
        settings.ui.theme = "light".to_string();
        store.save(&settings).expect("save");
        assert_eq!(store.load().expect("load"), settings);
    }

    #[test]
    fn settings_migration_writes_versioned_backup() {
        let store = temp_settings_store("migrate");
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(
            store.path(),
            r#"{"version":0,"terminal":{"scrollbackLines":20000}}"#,
        )
        .unwrap();
        let settings = store.load().expect("migrated load");
        assert_eq!(settings.version, pandamux_core::SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.terminal.scrollback_lines, 20_000);
        assert!(
            store
                .path()
                .parent()
                .unwrap()
                .join("settings.v0.bak.json")
                .exists(),
            "migration must back up the original file"
        );
    }

    #[test]
    fn corrupt_settings_are_preserved_and_error() {
        let store = temp_settings_store("corrupt");
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), "not json {").unwrap();
        assert!(matches!(
            store.load(),
            Err(SettingsStoreError::Corrupt { .. })
        ));
        // The broken file was not clobbered.
        assert_eq!(fs::read_to_string(store.path()).unwrap(), "not json {");
    }

    #[test]
    fn newer_settings_schema_is_refused() {
        let store = temp_settings_store("newer");
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), r#"{"version":99}"#).unwrap();
        assert!(matches!(
            store.load(),
            Err(SettingsStoreError::UnsupportedVersion(99))
        ));
    }

    fn split_state() -> AppState {
        let mut state = AppState::default();
        state
            .apply(AppIntent::Pane(PaneIntent::Split(SplitPaneParams {
                workspace_id: None,
                target_pane_id: Some(pandamux_core::PaneId::from("pane-default")),
                target_surface_id: None,
                direction: SplitDirection::Horizontal,
                surface_type: SurfaceType::Terminal,
            })))
            .expect("split should apply");
        state
    }

    #[test]
    fn auto_session_roundtrips() {
        let store = temp_store("auto");
        let state = split_state();
        store.save_session(&state).expect("save");
        let loaded = store.load_session().expect("load");
        assert_eq!(loaded, state);
    }

    #[test]
    fn missing_session_loads_none() {
        let store = temp_store("missing");
        assert!(store.load_session().is_none());
    }

    #[test]
    fn named_sessions_save_list_and_delete() {
        let store = temp_store("named");
        // `split_state()` mints fresh ids each call, so capture the instance we
        // save and compare the reload against it.
        let other = split_state();
        store
            .save_named("My Layout", 100, &AppState::default())
            .expect("save a");
        store.save_named("Other", 200, &other).expect("save b");

        let list = store.list_named();
        assert_eq!(list.len(), 2);
        // Newest first.
        assert_eq!(list[0].name, "Other");
        assert_eq!(list[0].saved_at_ms, 200);
        assert_eq!(store.last_session_name().as_deref(), Some("Other"));

        let loaded = store.load_named("Other").expect("load named");
        assert_eq!(loaded.state, other);

        assert!(store.delete_named("Other"));
        assert_eq!(store.list_named().len(), 1);
    }

    #[test]
    fn version_change_clears_auto_session_but_keeps_named() {
        let store = temp_store("version");
        store.save_session(&AppState::default()).expect("save auto");
        store
            .save_named("keep", 100, &AppState::default())
            .expect("save named");

        // First call for a new version reports change and clears session.json.
        assert!(store.handle_version_change("0.17.0"));
        assert!(store.load_session().is_none());
        // Named session survives.
        assert!(store.load_named("keep").is_some());
        // Same version again reports no change.
        assert!(!store.handle_version_change("0.17.0"));
    }

    #[test]
    fn names_are_sanitized() {
        assert_eq!(sanitize_name("a/b:c*?"), "a_b_c__");
    }

    #[test]
    fn ssh_profiles_roundtrip_and_rename_by_id() {
        let store = temp_profile_store("roundtrip");
        let first = SshHostProfile::new("One", "one.example", "chaz");
        let second = SshHostProfile::new("Two", "two.example", "chaz");
        let first_id = first.id.clone();
        let mut config = SshProfileConfig {
            profiles: vec![first, second],
            ..SshProfileConfig::default()
        };
        config
            .last_selected_folder_by_profile
            .insert(first_id.clone(), "/home/chaz/one".to_string());
        store.save(&config).expect("save profiles");

        let mut loaded = store.load().expect("load profiles");
        loaded
            .profiles
            .iter_mut()
            .find(|profile| profile.id == first_id)
            .expect("first profile")
            .name = "Renamed".to_string();
        store.save(&loaded).expect("save rename");
        let reloaded = store.load().expect("reload profiles");
        assert_eq!(reloaded.profiles.len(), 2);
        assert_eq!(reloaded.profiles[0].name, "Renamed");
    }

    #[test]
    fn profile_migration_backs_up_before_rewrite() {
        let store = temp_profile_store("migration");
        fs::create_dir_all(&store.base).expect("create config dir");
        fs::write(
            store.path(),
            r#"{"version":0,"profiles":[],"lastSelectedFolderByProfile":{}}"#,
        )
        .expect("write v0");
        let loaded = store.load().expect("migrate");
        assert_eq!(loaded.version, SSH_PROFILE_SCHEMA_VERSION);
        assert!(store.base.join("ssh-profiles.v0.bak.json").is_file());
        let disk: SshProfileConfig = read_json(&store.path()).expect("rewritten json");
        assert_eq!(disk.version, SSH_PROFILE_SCHEMA_VERSION);
    }

    #[test]
    fn corrupt_profile_file_is_preserved() {
        let store = temp_profile_store("corrupt");
        fs::create_dir_all(&store.base).expect("create config dir");
        fs::write(store.path(), "{not-json").expect("write corrupt file");
        assert!(matches!(
            store.load(),
            Err(SshProfileStoreError::Corrupt { .. })
        ));
        assert_eq!(fs::read_to_string(store.path()).unwrap(), "{not-json");
    }

    #[test]
    fn persisted_profile_schema_has_no_secret_fields() {
        let store = temp_profile_store("secretless");
        let mut profile = SshHostProfile::new("Password", "host", "user");
        profile.auth = pandamux_core::SshAuthConfig::Password;
        store
            .save(&SshProfileConfig {
                profiles: vec![profile],
                ..SshProfileConfig::default()
            })
            .expect("save password mode");
        let raw = fs::read_to_string(store.path()).unwrap();
        assert!(!raw.contains("passphrase"));
        assert!(!raw.contains("\"password\":"));
    }
}
