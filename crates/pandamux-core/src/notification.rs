//! Backend-owned notification store, ported from the Electron notification slice.
//!
//! Holds at most [`Notifications::MAX`] notifications, evicting the oldest *read*
//! ones first when full. Mirrors the renderer slice's add / mark-read /
//! mark-all-read / clear / jump-to-unread behavior, but as canonical backend
//! state (the UI holds a read-projection, per the rewrite's state model).

use crate::ids::{SurfaceId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Where a notification came from (drives the source dot color in the panel).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSource {
    Build,
    Agent,
    Deploy,
    Port,
    Generic,
}

/// A single notification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationInfo {
    pub id: String,
    pub workspace_id: Option<WorkspaceId>,
    pub surface_id: Option<SurfaceId>,
    pub title: String,
    pub body: String,
    pub source: NotificationSource,
    pub timestamp_ms: u64,
    pub read: bool,
}

/// The data needed to raise a notification (id/timestamp/read are assigned).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewNotification {
    pub workspace_id: Option<WorkspaceId>,
    pub surface_id: Option<SurfaceId>,
    pub title: String,
    pub body: String,
    pub source: NotificationSource,
}

impl NewNotification {
    pub fn generic(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            workspace_id: None,
            surface_id: None,
            title: title.into(),
            body: body.into(),
            source: NotificationSource::Generic,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notifications {
    items: Vec<NotificationInfo>,
}

impl Default for Notifications {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifications {
    pub const MAX: usize = 200;

    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn list(&self) -> &[NotificationInfo] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of unread notifications, optionally scoped to a workspace.
    pub fn unread_count(&self, workspace_id: Option<&WorkspaceId>) -> usize {
        self.items
            .iter()
            .filter(|note| !note.read)
            .filter(|note| workspace_id.is_none() || note.workspace_id.as_ref() == workspace_id)
            .count()
    }

    /// Add a notification with a caller-supplied id and timestamp (deterministic
    /// entry point; the runtime supplies a uuid id and wall-clock timestamp).
    pub fn push(&mut self, note: NewNotification, id: impl Into<String>, timestamp_ms: u64) {
        self.items.push(NotificationInfo {
            id: id.into(),
            workspace_id: note.workspace_id,
            surface_id: note.surface_id,
            title: note.title,
            body: note.body,
            source: note.source,
            timestamp_ms,
            read: false,
        });
        self.evict_overflow();
    }

    /// Mark every notification for `surface_id` as read.
    pub fn mark_read(&mut self, surface_id: &SurfaceId) {
        for note in &mut self.items {
            if note.surface_id.as_ref() == Some(surface_id) {
                note.read = true;
            }
        }
    }

    /// Mark all notifications read, optionally scoped to a workspace.
    pub fn mark_all_read(&mut self, workspace_id: Option<&WorkspaceId>) {
        for note in &mut self.items {
            if workspace_id.is_none() || note.workspace_id.as_ref() == workspace_id {
                note.read = true;
            }
        }
    }

    /// Remove a single notification by id. Returns whether one was removed.
    pub fn clear(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|note| note.id != id);
        self.items.len() != before
    }

    pub fn clear_all(&mut self) {
        self.items.clear();
    }

    /// The most recent unread notification, if any (what the bell jumps to).
    pub fn jump_to_unread(&self) -> Option<&NotificationInfo> {
        self.items.iter().rev().find(|note| !note.read)
    }

    /// When over [`Self::MAX`], drop the oldest *read* notifications first; only
    /// if there are not enough read ones does it drop the oldest overall.
    fn evict_overflow(&mut self) {
        if self.items.len() <= Self::MAX {
            return;
        }
        let mut to_evict = self.items.len() - Self::MAX;
        // First pass: remove oldest read entries.
        let mut index = 0;
        while to_evict > 0 && index < self.items.len() {
            if self.items[index].read {
                self.items.remove(index);
                to_evict -= 1;
            } else {
                index += 1;
            }
        }
        // Second pass: still over the cap (all remaining unread) — drop oldest.
        if to_evict > 0 {
            self.items.drain(0..to_evict);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(title: &str) -> NewNotification {
        NewNotification::generic(title, "body")
    }

    #[test]
    fn push_and_unread_count() {
        let mut notifications = Notifications::new();
        notifications.push(note("a"), "notif-1", 100);
        notifications.push(note("b"), "notif-2", 200);
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications.unread_count(None), 2);
    }

    #[test]
    fn mark_read_by_surface() {
        let mut notifications = Notifications::new();
        let surface = SurfaceId::from("surf-1");
        let mut with_surface = note("a");
        with_surface.surface_id = Some(surface.clone());
        notifications.push(with_surface, "notif-1", 100);
        notifications.push(note("b"), "notif-2", 200);

        notifications.mark_read(&surface);
        assert_eq!(notifications.unread_count(None), 1);
    }

    #[test]
    fn jump_returns_latest_unread() {
        let mut notifications = Notifications::new();
        notifications.push(note("old"), "notif-1", 100);
        notifications.push(note("new"), "notif-2", 200);
        assert_eq!(notifications.jump_to_unread().unwrap().id, "notif-2");
        notifications.mark_all_read(None);
        assert!(notifications.jump_to_unread().is_none());
    }

    #[test]
    fn clear_removes_by_id_and_all() {
        let mut notifications = Notifications::new();
        notifications.push(note("a"), "notif-1", 100);
        notifications.push(note("b"), "notif-2", 200);
        assert!(notifications.clear("notif-1"));
        assert!(!notifications.clear("missing"));
        assert_eq!(notifications.len(), 1);
        notifications.clear_all();
        assert!(notifications.is_empty());
    }

    #[test]
    fn eviction_drops_read_before_unread_at_the_cap() {
        let mut notifications = Notifications::new();
        // Fill to the cap; mark the first one read.
        for i in 0..Notifications::MAX {
            notifications.push(note(&format!("n{i}")), format!("notif-{i}"), i as u64);
        }
        notifications.items[0].read = true;
        // One more over the cap evicts the single read entry, keeping unread ones.
        notifications.push(note("overflow"), "notif-overflow", 9999);
        assert_eq!(notifications.len(), Notifications::MAX);
        assert!(!notifications.items.iter().any(|n| n.id == "notif-0"));
        assert!(notifications.items.iter().any(|n| n.id == "notif-overflow"));
    }
}
