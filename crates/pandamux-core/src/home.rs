//! The Home dashboard layout (spec 2.5): a persistent, user-built view whose
//! panes reference LIVE sessions from ANY project by surface id. This is the
//! deliberate exception to project-scoped tab views. Unpinning removes a pane
//! from Home without touching the underlying session; a pane whose session no
//! longer exists renders a placeholder that can relaunch from its pinned
//! configuration (project + session type). The layout persists inside
//! `AppState`, so it restores with the session file.
//!
//! v1 arranges panes as an ordered balanced grid (the renderer chunks the
//! list into columns); reordering is a positional move. A free-form split
//! tree can replace the ordering later without breaking old files (the field
//! is additive).

use crate::ids::{PaneId, SurfaceId};
use crate::project_registry::LaunchConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HomeLayout {
    pub panes: Vec<HomePane>,
    pub focused_pane_id: Option<PaneId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomePane {
    pub id: PaneId,
    /// What this pane relaunches as when its session is gone.
    #[serde(default)]
    pub pinned: Option<LaunchConfig>,
    /// The live session shown here, when one exists.
    #[serde(default)]
    pub surface_id: Option<SurfaceId>,
}

impl HomeLayout {
    pub fn pane(&self, pane_id: &PaneId) -> Option<&HomePane> {
        self.panes.iter().find(|pane| &pane.id == pane_id)
    }

    fn pane_mut(&mut self, pane_id: &PaneId) -> Option<&mut HomePane> {
        self.panes.iter_mut().find(|pane| &pane.id == pane_id)
    }

    /// Pin a live session onto Home. A session already pinned just refocuses;
    /// otherwise a new pane appends and takes focus.
    pub fn pin(&mut self, surface_id: SurfaceId, pinned: Option<LaunchConfig>) -> PaneId {
        if let Some(pane) = self
            .panes
            .iter()
            .find(|pane| pane.surface_id.as_ref() == Some(&surface_id))
        {
            let pane_id = pane.id.clone();
            self.focused_pane_id = Some(pane_id.clone());
            return pane_id;
        }
        let pane = HomePane {
            id: PaneId::generate(),
            pinned,
            surface_id: Some(surface_id),
        };
        let pane_id = pane.id.clone();
        self.panes.push(pane);
        self.focused_pane_id = Some(pane_id.clone());
        pane_id
    }

    /// Point an existing pane at a (new) live session.
    pub fn assign(
        &mut self,
        pane_id: &PaneId,
        surface_id: SurfaceId,
        pinned: Option<LaunchConfig>,
    ) -> bool {
        let Some(pane) = self.pane_mut(pane_id) else {
            return false;
        };
        pane.surface_id = Some(surface_id);
        if pinned.is_some() {
            pane.pinned = pinned;
        }
        self.focused_pane_id = Some(pane_id.clone());
        true
    }

    /// Remove a pane from Home. The underlying session is untouched.
    pub fn unpin(&mut self, pane_id: &PaneId) -> bool {
        let before = self.panes.len();
        self.panes.retain(|pane| &pane.id != pane_id);
        if self.focused_pane_id.as_ref() == Some(pane_id) {
            self.focused_pane_id = self.panes.first().map(|pane| pane.id.clone());
        }
        self.panes.len() != before
    }

    /// Move a pane earlier/later in the arrangement (clamped).
    pub fn move_by(&mut self, pane_id: &PaneId, delta: i32) -> bool {
        let Some(index) = self.panes.iter().position(|pane| &pane.id == pane_id) else {
            return false;
        };
        let target = (index as i64 + delta as i64).clamp(0, self.panes.len() as i64 - 1) as usize;
        if target == index {
            return false;
        }
        let pane = self.panes.remove(index);
        self.panes.insert(target, pane);
        true
    }

    pub fn focus(&mut self, pane_id: &PaneId) -> bool {
        if self.pane(pane_id).is_none() {
            return false;
        }
        self.focused_pane_id = Some(pane_id.clone());
        true
    }

    /// Drop dead surface references (their sessions closed). Panes stay, as
    /// placeholders that can relaunch from their pinned configuration.
    pub fn release_dead_surfaces(&mut self, alive: &dyn Fn(&SurfaceId) -> bool) -> bool {
        let mut changed = false;
        for pane in &mut self.panes {
            if let Some(surface_id) = &pane.surface_id
                && !alive(surface_id)
            {
                pane.surface_id = None;
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ProjectId;
    use crate::split_tree::SessionType;

    fn config() -> LaunchConfig {
        LaunchConfig {
            project_id: ProjectId::from("proj-a"),
            session: SessionType::Claude,
        }
    }

    #[test]
    fn pin_focus_unpin_and_reorder() {
        let mut home = HomeLayout::default();
        let first = home.pin(SurfaceId::from("surf-1"), Some(config()));
        let second = home.pin(SurfaceId::from("surf-2"), None);
        assert_eq!(home.panes.len(), 2);
        assert_eq!(home.focused_pane_id, Some(second.clone()));

        // Re-pinning an existing session focuses its pane instead of duplicating.
        let again = home.pin(SurfaceId::from("surf-1"), None);
        assert_eq!(again, first);
        assert_eq!(home.panes.len(), 2);

        // Reorder moves within bounds and clamps at the edges.
        assert!(home.move_by(&second, -1));
        assert_eq!(home.panes[0].id, second);
        assert!(!home.move_by(&second, -1));

        // Unpin removes the pane and refocuses; sessions are untouched by design.
        assert!(home.unpin(&second));
        assert_eq!(home.panes.len(), 1);
        assert_eq!(home.focused_pane_id, Some(first));
    }

    #[test]
    fn dead_sessions_become_relaunchable_placeholders() {
        let mut home = HomeLayout::default();
        let pane_id = home.pin(SurfaceId::from("surf-dead"), Some(config()));
        assert!(home.release_dead_surfaces(&|_| false));
        let pane = home.pane(&pane_id).unwrap();
        assert_eq!(pane.surface_id, None);
        assert_eq!(pane.pinned, Some(config()));

        // Relaunch assigns the fresh surface back into the same pane.
        assert!(home.assign(&pane_id, SurfaceId::from("surf-new"), None));
        assert_eq!(
            home.pane(&pane_id).unwrap().surface_id,
            Some(SurfaceId::from("surf-new"))
        );
    }
}
