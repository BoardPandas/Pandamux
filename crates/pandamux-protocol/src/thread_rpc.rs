use serde::{Deserialize, Serialize};
use pandamux_core::{
    AccessMode, AgentId, ApprovalDecision, EnvironmentId, ProjectId, ProviderInstanceId, Thread,
    ThreadId, ThreadStatus, Turn, TurnId,
};

/// Parameters for `thread.create`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCreateParams {
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub provider_instance_id: Option<ProviderInstanceId>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub access_mode: Option<AccessMode>,
    #[serde(default)]
    pub agent_id: Option<AgentId>,
}

/// Parameters for `thread.list`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListParams {
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub agent_id: Option<AgentId>,
    #[serde(default)]
    pub status: Option<ThreadStatus>,
    #[serde(default)]
    pub parent_thread_id: Option<ThreadId>,
}

/// Parameters for `thread.get`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGetParams {
    pub thread_id: ThreadId,
}

/// Result of `thread.get`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGetResult {
    pub thread: Thread,
    #[serde(default)]
    pub turns: Vec<Turn>,
}

/// Parameters for `thread.send_turn`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSendTurnParams {
    pub thread_id: ThreadId,
    pub text: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

/// Result of `thread.send_turn`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSendTurnResult {
    pub turn_id: TurnId,
    pub seq: u64,
}

/// Parameters for `thread.cancel_turn`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCancelTurnParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub turn_id: Option<TurnId>,
}

/// Parameters for `thread.respond_approval`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRespondApprovalParams {
    pub thread_id: ThreadId,
    pub request_id: String,
    pub decision: ApprovalDecision,
}
