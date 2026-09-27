use pandamux_core::ThreadId;
use serde::{Deserialize, Serialize};

/// Parameters for listing turn checkpoints in a thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointListParams {
    pub thread_id: ThreadId,
}

/// Parameters for computing diff statistics between two checkpoints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointDiffParams {
    pub thread_id: ThreadId,
    pub before_ref: String,
    pub after_ref: String,
    #[serde(default)]
    pub file_path: Option<String>,
}

/// Result of computing diff between two checkpoints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointDiffResult {
    pub changed_files: Vec<pandamux_core::FileChangeKind>,
    #[serde(default)]
    pub patch: Option<String>,
}

/// Parameters for rolling back a thread's worktree to a checkpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointRollbackParams {
    pub thread_id: ThreadId,
    pub checkpoint_ref: String,
}
