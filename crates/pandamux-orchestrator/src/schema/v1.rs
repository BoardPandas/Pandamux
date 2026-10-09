use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA_VERSION_V1: u32 = 1;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum DocumentKind {
    #[serde(rename = "pandamux.orchestrator")]
    Orchestrator,
    #[serde(rename = "pandamux.project")]
    Project,
    #[serde(rename = "pandamux.agent")]
    Agent,
    #[serde(rename = "pandamux.knowledge")]
    Knowledge,
    #[serde(rename = "pandamux.prompt")]
    Prompt,
    #[serde(rename = "pandamux.compiled-bundle")]
    CompiledBundle,
}

impl DocumentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Orchestrator => "pandamux.orchestrator",
            Self::Project => "pandamux.project",
            Self::Agent => "pandamux.agent",
            Self::Knowledge => "pandamux.knowledge",
            Self::Prompt => "pandamux.prompt",
            Self::CompiledBundle => "pandamux.compiled-bundle",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredOrchestrationMode {
    Auto,
    #[default]
    ApproveFirst,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredAccessMode {
    ReadOnly,
    WorkspaceOnly,
    Ask,
    AutoEdit,
    FullAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitFallbackV1 {
    NextProvider,
    Queue,
    Ask,
    Fail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NoProviderFallbackV1 {
    Queue,
    Ask,
    Fail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureFallbackV1 {
    Ask,
    Fail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromptPurposeV1 {
    Intake,
    Planner,
    Handoff,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestratorDefaultsV1 {
    #[serde(default)]
    pub mode: AuthoredOrchestrationMode,
    pub default_agent: String,
    #[serde(default = "default_max_parallel_tasks")]
    pub max_parallel_tasks: u32,
    #[serde(default = "default_max_tasks_per_run")]
    pub max_tasks_per_run: u32,
    #[serde(default = "default_max_attempts_per_task")]
    pub max_attempts_per_task: u32,
}

const fn default_max_parallel_tasks() -> u32 {
    4
}

const fn default_max_tasks_per_run() -> u32 {
    6
}

const fn default_max_attempts_per_task() -> u32 {
    2
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptReferencesV1 {
    pub intake: String,
    pub planner: String,
    pub handoff: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassMatchV1 {
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_files_hint: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassReviewV1 {
    pub agent: String,
    pub when: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassRuleV1 {
    pub id: String,
    #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
    pub match_rule: Option<ClassMatchV1>,
    #[serde(default)]
    pub decompose: bool,
    #[serde(default)]
    pub require_approval: bool,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ClassReviewV1>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPolicyV1 {
    pub min_remaining_millionths: u32,
    #[serde(default)]
    pub concurrency: BTreeMap<String, u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FallbacksV1 {
    pub on_rate_limit: RateLimitFallbackV1,
    #[serde(default = "default_no_provider_fallback")]
    pub on_no_provider: NoProviderFallbackV1,
    #[serde(default = "default_failure_fallback")]
    pub on_failure: FailureFallbackV1,
}

const fn default_no_provider_fallback() -> NoProviderFallbackV1 {
    NoProviderFallbackV1::Queue
}

const fn default_failure_fallback() -> FailureFallbackV1 {
    FailureFallbackV1::Ask
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestratorVerificationV1 {
    #[serde(default)]
    pub use_project_commands: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestratorDocumentV1 {
    pub schema_version: u32,
    pub kind: DocumentKind,
    pub min_pandamux_version: String,
    pub defaults: OrchestratorDefaultsV1,
    pub prompts: PromptReferencesV1,
    #[serde(default)]
    pub classes: Vec<ClassRuleV1>,
    pub provider_policy: ProviderPolicyV1,
    pub fallbacks: FallbacksV1,
    pub verification: OrchestratorVerificationV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectDocumentV1 {
    pub schema_version: u32,
    pub kind: DocumentKind,
    #[serde(default)]
    pub class_overrides: Vec<ClassRuleV1>,
    #[serde(default)]
    pub verification: Vec<String>,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub knowledge: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentProviderPolicyV1 {
    #[serde(default)]
    pub prefer: Vec<String>,
    #[serde(default)]
    pub avoid: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentToolPolicyV1 {
    #[serde(default)]
    pub allow: BTreeSet<String>,
    #[serde(default)]
    pub deny: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputContractV1 {
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentBudgetV1 {
    pub max_tokens: u64,
    pub max_cost_micro_usd: u64,
    pub max_runtime_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDocumentV1 {
    pub schema_version: u32,
    pub kind: DocumentKind,
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub specialties: BTreeSet<String>,
    pub tier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub access_mode: AuthoredAccessMode,
    #[serde(default)]
    pub required_capabilities: BTreeSet<String>,
    #[serde(default)]
    pub providers: AgentProviderPolicyV1,
    #[serde(default)]
    pub environments: BTreeSet<String>,
    #[serde(default)]
    pub knowledge: BTreeSet<String>,
    #[serde(default)]
    pub tools: AgentToolPolicyV1,
    pub output: OutputContractV1,
    #[serde(default)]
    pub verification: Vec<String>,
    pub concurrency: u32,
    pub budget: AgentBudgetV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeDocumentV1 {
    pub schema_version: u32,
    pub kind: DocumentKind,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub tags: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptDocumentV1 {
    pub schema_version: u32,
    pub kind: DocumentKind,
    pub id: String,
    pub purpose: PromptPurposeV1,
}

#[derive(JsonSchema)]
#[allow(dead_code)]
pub(crate) struct AuthoredSchemaCatalog {
    pub orchestrator: OrchestratorDocumentV1,
    pub project: ProjectDocumentV1,
    pub agent: AgentDocumentV1,
    pub knowledge: KnowledgeDocumentV1,
    pub prompt: PromptDocumentV1,
}
