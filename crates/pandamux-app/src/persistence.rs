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

use pandamux_core::AppState;
use serde::{Deserialize, Serialize};
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
}
