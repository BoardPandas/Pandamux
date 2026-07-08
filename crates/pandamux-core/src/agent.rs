//! Backend-owned agent registry, ported from the Electron agent-manager.
//!
//! An *agent* is a terminal surface running a specific command (typically
//! `claude ...`) rather than the plain workspace shell. The registry tracks each
//! spawned agent's identity, its placement (workspace/pane/surface), and its
//! status so the CLI (`agent status/list`) and the pandamux-orchestrator plugin
//! can coordinate. Spawning is done by the backend, which creates the surface,
//! starts the PTY with the agent command, and registers the agent here.

use crate::ids::{PaneId, SurfaceId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Distribution strategy for a batch spawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpawnStrategy {
    /// Round-robin new tabs across the existing panes.
    #[default]
    Distribute,
    /// All agents as tabs in the focused pane.
    Stack,
    /// One new split pane per agent.
    Split,
}

impl SpawnStrategy {
    pub fn parse(value: &str) -> Self {
        match value {
            "stack" => SpawnStrategy::Stack,
            "split" => SpawnStrategy::Split,
            _ => SpawnStrategy::Distribute,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Starting,
    Running,
    Exited,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub id: String,
    pub label: String,
    pub command: String,
    pub cwd: Option<String>,
    pub workspace_id: WorkspaceId,
    pub pane_id: PaneId,
    pub surface_id: SurfaceId,
    pub status: AgentStatus,
}

/// The set of live agents. Ids are monotonic (`agent-1`, `agent-2`, ...).
#[derive(Clone, Debug, Default)]
pub struct AgentRegistry {
    agents: Vec<AgentInfo>,
    seq: u64,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint the next agent id.
    pub fn next_id(&mut self) -> String {
        self.seq += 1;
        format!("agent-{}", self.seq)
    }

    pub fn add(&mut self, info: AgentInfo) {
        self.agents.push(info);
    }

    pub fn get(&self, id: &str) -> Option<&AgentInfo> {
        self.agents.iter().find(|agent| agent.id == id)
    }

    pub fn list(&self) -> &[AgentInfo] {
        &self.agents
    }

    pub fn len(&self) -> usize {
        self.agents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    /// Remove an agent by id, returning it (so the caller can close its surface).
    pub fn remove(&mut self, id: &str) -> Option<AgentInfo> {
        let index = self.agents.iter().position(|agent| agent.id == id)?;
        Some(self.agents.remove(index))
    }

    pub fn set_status(&mut self, id: &str, status: AgentStatus) {
        if let Some(agent) = self.agents.iter_mut().find(|agent| agent.id == id) {
            agent.status = status;
        }
    }

    /// The agent backing a given surface, if any.
    pub fn by_surface(&self, surface_id: &SurfaceId) -> Option<&AgentInfo> {
        self.agents
            .iter()
            .find(|agent| &agent.surface_id == surface_id)
    }

    /// Drop registry entries whose surface is no longer live (its id is not in
    /// `live_surface_ids`); returns the removed agents.
    pub fn prune_missing(&mut self, live_surface_ids: &[SurfaceId]) -> Vec<AgentInfo> {
        let mut removed = Vec::new();
        self.agents.retain(|agent| {
            if live_surface_ids.contains(&agent.surface_id) {
                true
            } else {
                removed.push(agent.clone());
                false
            }
        });
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, surface: &str) -> AgentInfo {
        AgentInfo {
            id: id.to_string(),
            label: "worker".to_string(),
            command: "claude".to_string(),
            cwd: None,
            workspace_id: WorkspaceId::from("ws-1"),
            pane_id: PaneId::from("pane-1"),
            surface_id: SurfaceId::from(surface),
            status: AgentStatus::Starting,
        }
    }

    #[test]
    fn ids_are_monotonic() {
        let mut registry = AgentRegistry::new();
        assert_eq!(registry.next_id(), "agent-1");
        assert_eq!(registry.next_id(), "agent-2");
    }

    #[test]
    fn add_get_status_and_remove() {
        let mut registry = AgentRegistry::new();
        registry.add(agent("agent-1", "surf-1"));
        assert_eq!(
            registry.get("agent-1").unwrap().status,
            AgentStatus::Starting
        );
        registry.set_status("agent-1", AgentStatus::Running);
        assert_eq!(
            registry.get("agent-1").unwrap().status,
            AgentStatus::Running
        );
        assert!(registry.by_surface(&SurfaceId::from("surf-1")).is_some());
        let removed = registry.remove("agent-1").unwrap();
        assert_eq!(removed.surface_id, SurfaceId::from("surf-1"));
        assert!(registry.is_empty());
    }

    #[test]
    fn prune_drops_agents_without_live_surfaces() {
        let mut registry = AgentRegistry::new();
        registry.add(agent("agent-1", "surf-1"));
        registry.add(agent("agent-2", "surf-2"));
        let removed = registry.prune_missing(&[SurfaceId::from("surf-1")]);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].id, "agent-2");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn strategy_parses() {
        assert_eq!(SpawnStrategy::parse("stack"), SpawnStrategy::Stack);
        assert_eq!(SpawnStrategy::parse("split"), SpawnStrategy::Split);
        assert_eq!(
            SpawnStrategy::parse("distribute"),
            SpawnStrategy::Distribute
        );
        assert_eq!(SpawnStrategy::parse("weird"), SpawnStrategy::Distribute);
    }
}
