//! Backend-owned sidebar state: the status key/values, a progress bar, and a
//! capped activity log that the CLI (`set-status`/`set-progress`/`log`/
//! `sidebar-state`) and the pandamux-orchestrator plugin write to report
//! progress. The UI surfaces the progress value in the status bar and can read
//! the full state via `sidebar.get_state`.

use serde::{Deserialize, Serialize};

/// A determinate progress reading (0-100) with an optional label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub value: u8,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub level: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarState {
    pub statuses: Vec<StatusEntry>,
    pub progress: Option<Progress>,
    pub logs: Vec<LogEntry>,
}

impl SidebarState {
    /// Cap on retained log lines (oldest dropped first).
    pub const MAX_LOGS: usize = 200;

    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or replace) a status key. An empty value clears the key.
    pub fn set_status(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        if value.is_empty() {
            self.statuses.retain(|entry| entry.key != key);
            return;
        }
        match self.statuses.iter_mut().find(|entry| entry.key == key) {
            Some(entry) => entry.value = value,
            None => self.statuses.push(StatusEntry { key, value }),
        }
    }

    /// Set the progress bar (value is clamped to 0-100).
    pub fn set_progress(&mut self, value: u8, label: Option<String>) {
        self.progress = Some(Progress {
            value: value.min(100),
            label,
        });
    }

    pub fn clear_progress(&mut self) {
        self.progress = None;
    }

    /// Append a log line, dropping the oldest when over the cap.
    pub fn log(&mut self, level: impl Into<String>, message: impl Into<String>) {
        self.logs.push(LogEntry {
            level: level.into(),
            message: message.into(),
        });
        if self.logs.len() > Self::MAX_LOGS {
            let overflow = self.logs.len() - Self::MAX_LOGS;
            self.logs.drain(0..overflow);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_status_updates_and_clears() {
        let mut sidebar = SidebarState::new();
        sidebar.set_status("branch", "master");
        sidebar.set_status("branch", "dev");
        assert_eq!(sidebar.statuses.len(), 1);
        assert_eq!(sidebar.statuses[0].value, "dev");
        sidebar.set_status("branch", "");
        assert!(sidebar.statuses.is_empty());
    }

    #[test]
    fn progress_clamps_to_100() {
        let mut sidebar = SidebarState::new();
        sidebar.set_progress(150, Some("building".to_string()));
        assert_eq!(sidebar.progress.as_ref().unwrap().value, 100);
    }

    #[test]
    fn logs_are_capped() {
        let mut sidebar = SidebarState::new();
        for i in 0..(SidebarState::MAX_LOGS + 10) {
            sidebar.log("info", format!("line {i}"));
        }
        assert_eq!(sidebar.logs.len(), SidebarState::MAX_LOGS);
        assert_eq!(sidebar.logs[0].message, "line 10");
    }
}
