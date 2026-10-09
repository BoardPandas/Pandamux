use crate::ids::{AgentId, OrganizationId, ProjectId, RunId};
use crate::provider_config::ProviderKind;
use crate::thread::AccessMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Canonical provider-neutral agent produced by the instruction compiler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub id: AgentId,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub scope: AgentScope,
    pub author: AgentAuthor,
    pub version: String,
    pub bundle_sha: String,
    pub source_path: String,
    #[serde(default)]
    pub organization_id: Option<OrganizationId>,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub specialties: BTreeSet<String>,
    pub tier: String,
    #[serde(default)]
    pub effort: Option<String>,
    pub access_mode: AccessMode,
    #[serde(default)]
    pub full_access_opt_in: bool,
    #[serde(default)]
    pub required_capabilities: BTreeSet<String>,
    #[serde(default)]
    pub providers: AgentProviderPolicy,
    #[serde(default)]
    pub environments: BTreeSet<String>,
    #[serde(default)]
    pub knowledge: BTreeSet<String>,
    #[serde(default)]
    pub tools: AgentToolPolicy,
    pub output: OutputContract,
    #[serde(default)]
    pub verification: Vec<String>,
    pub concurrency: u32,
    pub budget: AgentBudget,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
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
    InstructionsRepository,
    ProjectRepository,
}

/// Provider order is semantic for `prefer`; `avoid` is canonicalized by the compiler.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProviderPolicy {
    #[serde(default)]
    pub prefer: Vec<ProviderKind>,
    #[serde(default)]
    pub avoid: Vec<ProviderKind>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolPolicy {
    #[serde(default)]
    pub allow: BTreeSet<String>,
    #[serde(default)]
    pub deny: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBudget {
    pub max_tokens: u64,
    pub max_cost_micro_usd: u64,
    pub max_runtime_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputContract {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
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

/// Legacy Phase 1 memory input retained only for the explicit Phase 5E importer.
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
