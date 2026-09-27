use crate::ids::{AgentId, ProjectId, RunId};
use serde::{Deserialize, Serialize};

/// Canonical agent definition resolved from layered configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub id: AgentId,
    pub name: String,
    pub scope: AgentScope,
    pub author: AgentAuthor,
    #[serde(default)]
    pub version: String,
    pub description: String,
    pub system_prompt: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub max_turns: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentScope {
    Team,
    Project,
    Personal,
    BuiltIn,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentAuthor {
    User,
    Orchestrator { run_id: RunId },
}

/// A proposed or applied modification to an agent definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChange {
    pub agent_id: AgentId,
    pub scope: AgentScope,
    pub diff: String,
    pub author: AgentAuthor,
    pub widens_access: bool,
    pub status: AgentChangeStatus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentChangeStatus {
    #[default]
    Proposed,
    Approved,
    Applied,
    Rejected,
    Reverted,
}

/// Agent memory entry (vector-indexed and markdown mirrored).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: String,
    pub agent_id: AgentId,
    pub scope: MemoryScope,
    pub text: String,
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub source_run_id: Option<RunId>,
    #[serde(default)]
    pub tombstone: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemoryScope {
    Project { project_id: ProjectId },
    AgentWide,
}
