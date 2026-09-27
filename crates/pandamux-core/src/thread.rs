use crate::ids::{
    AgentId, EnvironmentId, ProjectId, ProviderInstanceId, RunId, ScheduleId, ThreadId, TurnId,
};
use serde::{Deserialize, Serialize};

/// Core thread entity representing a conversation with an agent or provider.
/// Global Orchestrator threads have `project_id: None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: ThreadId,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    pub environment_id: EnvironmentId,
    #[serde(default)]
    pub parent_thread_id: Option<ThreadId>,
    pub title: String,
    pub provider_instance_id: ProviderInstanceId,
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub access_mode: AccessMode,
    pub workspace: ThreadWorkspace,
    #[serde(default)]
    pub status: ThreadStatus,
    #[serde(default)]
    pub agent: Option<AgentInstanceRef>,
    #[serde(default)]
    pub origin: ThreadOrigin,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
}

/// Access permission ceiling for a thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    #[default]
    WorkspaceOnly,
    ReadOnly,
    Ask,
    AutoEdit,
    FullAccess,
}

/// Thread workspace location and optional isolated git worktree.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadWorkspace {
    pub cwd: String,
    #[serde(default)]
    pub worktree: Option<WorktreeRef>,
}

impl ThreadWorkspace {
    /// Returns the active worktree path if present, otherwise the standard cwd.
    pub fn effective_path(&self) -> &str {
        if let Some(ref wt) = self.worktree {
            &wt.path
        } else {
            &self.cwd
        }
    }
}

/// A dedicated git worktree backing a thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRef {
    pub path: String,
    pub branch: String,
    pub base_ref: String,
}

/// Agent definition reference for agent instance threads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInstanceRef {
    pub agent_id: AgentId,
    pub bundle_sha: String,
}

/// High-level lifecycle status of a thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    #[default]
    Idle,
    Working,
    AwaitingApproval,
    Paused,
    Errored,
    Archived,
}

/// What initiated this thread.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThreadOrigin {
    #[default]
    Manual,
    Orchestrator {
        run_id: RunId,
        task_id: String,
    },
    Schedule {
        schedule_id: ScheduleId,
        run_id: RunId,
    },
}

/// A discrete user turn and provider response cycle within a thread.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub id: TurnId,
    pub thread_id: ThreadId,
    pub seq: u64,
    pub input: TurnInput,
    #[serde(default)]
    pub status: TurnStatus,
    #[serde(default)]
    pub started_at_ms: u64,
    #[serde(default)]
    pub ended_at_ms: Option<u64>,
    #[serde(default)]
    pub usage: Option<TurnUsage>,
    #[serde(default)]
    pub checkpoint_before: Option<String>,
    #[serde(default)]
    pub checkpoint_after: Option<String>,
}

/// Prompt input initiating a turn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    pub text: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<crate::attachment::AttachmentRecord>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

/// Lifecycle status of an individual turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    #[default]
    Requested,
    Running,
    AwaitingApproval,
    Completed,
    Failed,
    Interrupted,
}

/// Token and cost usage metrics recorded for a turn.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}
