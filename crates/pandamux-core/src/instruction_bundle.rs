use crate::agent_def::AgentDefinition;
use crate::ids::{AgentId, OrganizationId, ProjectId};
use crate::organization::TierMapping;
use crate::run::OrchestrationMode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const COMPILED_BUNDLE_SCHEMA_VERSION: u32 = 1;

/// Source identities supplied by trusted server-owned snapshots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleSourceManifest {
    pub instructions_commit_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_layer_content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_commit_sha: Option<String>,
    pub settings_revision: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledPrompt {
    pub id: String,
    pub purpose: PromptPurpose,
    pub content: String,
    pub source_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptPurpose {
    Intake,
    Planner,
    Handoff,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledKnowledge {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub tags: BTreeSet<String>,
    pub content: String,
    pub source_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleReference {
    pub path: String,
    pub expected_kind: String,
    pub resolved_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueLayer {
    Instructions,
    Project,
    Personal,
    Settings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueOrigin {
    pub layer: ValueLayer,
    pub logical_path: String,
    pub field: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledClassRule {
    pub id: String,
    /// Agent order is semantic and follows the authored preference order.
    pub agents: Vec<AgentId>,
    #[serde(default)]
    pub match_keywords: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_files_hint: Option<u32>,
    #[serde(default)]
    pub decompose: bool,
    #[serde(default)]
    pub require_approval: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_agent: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_when: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitFallback {
    NextProvider,
    Queue,
    Ask,
    Fail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoProviderFallback {
    Queue,
    Ask,
    Fail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureFallback {
    Ask,
    Fail,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledOrchestrationPolicy {
    pub mode: OrchestrationMode,
    pub default_agent_id: AgentId,
    pub max_parallel_tasks: u32,
    pub max_tasks_per_run: u32,
    pub max_attempts_per_task: u32,
    pub min_remaining_millionths: u32,
    #[serde(default)]
    pub provider_concurrency: BTreeMap<String, u32>,
    pub on_rate_limit: RateLimitFallback,
    pub on_no_provider: NoProviderFallback,
    pub on_failure: FailureFallback,
    pub use_project_commands: bool,
    /// Command order is semantic and preserved from the trusted project document.
    #[serde(default)]
    pub project_verification: Vec<String>,
    pub tier_mapping: TierMapping,
}

/// Canonical JSON envelope. `payload_sha256` hashes the unsigned form with this
/// field and every nested agent `bundle_sha` empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledInstructionBundleV1 {
    pub schema_version: u32,
    pub kind: String,
    pub compiler_version: String,
    pub source_manifest: BundleSourceManifest,
    pub project_id: ProjectId,
    pub organization_id: OrganizationId,
    #[serde(default)]
    pub agents: BTreeMap<AgentId, AgentDefinition>,
    #[serde(default)]
    pub prompts: BTreeMap<String, CompiledPrompt>,
    #[serde(default)]
    pub knowledge: BTreeMap<String, CompiledKnowledge>,
    #[serde(default)]
    pub references: BTreeMap<String, Vec<BundleReference>>,
    #[serde(default)]
    pub classes: Vec<CompiledClassRule>,
    pub policy: CompiledOrchestrationPolicy,
    #[serde(default)]
    pub value_origins: BTreeMap<String, ValueOrigin>,
    pub payload_sha256: String,
}
