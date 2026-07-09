//! Backend-owned content for non-terminal surfaces (markdown / diff).
//!
//! Terminal surfaces are backed by a live PTY grid and never appear here.
//! Markdown/diff surfaces hold text set over the pipe (`markdown.set_content`,
//! `markdown.load_file`, `diff.refresh`) and rendered by the UI. This is a thin
//! projection-friendly store keyed by surface id, owned alongside `AppState` on
//! the single-writer path so both the pipe server and the live UI share it.

use crate::ids::SurfaceId;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SurfaceContents {
    contents: HashMap<SurfaceId, String>,
}

impl SurfaceContents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the content for a surface.
    pub fn set(&mut self, surface_id: SurfaceId, content: String) {
        self.contents.insert(surface_id, content);
    }

    pub fn get(&self, surface_id: &SurfaceId) -> Option<&str> {
        self.contents.get(surface_id).map(String::as_str)
    }

    pub fn remove(&mut self, surface_id: &SurfaceId) {
        self.contents.remove(surface_id);
    }

    pub fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }

    /// A cheap owned snapshot for the UI view model.
    pub fn snapshot(&self) -> HashMap<SurfaceId, String> {
        self.contents.clone()
    }

    /// Drop content for surfaces that no longer exist (called after a mutation
    /// that can close surfaces, so closed markdown/diff panes do not leak).
    pub fn retain_live(&mut self, live: &HashSet<SurfaceId>) {
        self.contents.retain(|id, _| live.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_remove() {
        let mut contents = SurfaceContents::new();
        let id = SurfaceId::from("surf-1");
        assert_eq!(contents.get(&id), None);
        contents.set(id.clone(), "# Title".to_string());
        assert_eq!(contents.get(&id), Some("# Title"));
        contents.remove(&id);
        assert!(contents.is_empty());
    }

    #[test]
    fn retain_live_drops_missing() {
        let mut contents = SurfaceContents::new();
        contents.set(SurfaceId::from("surf-1"), "a".to_string());
        contents.set(SurfaceId::from("surf-2"), "b".to_string());
        let live = HashSet::from([SurfaceId::from("surf-1")]);
        contents.retain_live(&live);
        assert_eq!(contents.get(&SurfaceId::from("surf-1")), Some("a"));
        assert_eq!(contents.get(&SurfaceId::from("surf-2")), None);
    }
}
