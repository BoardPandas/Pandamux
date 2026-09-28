use pandamux_core::{EnvironmentId, ProjectId, ThreadId};
use serde::{Deserialize, Serialize};

/// Parameters for `worktree.sync_delta` or `worktree.syncDelta`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSyncDeltaParams {
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub thread_id: Option<ThreadId>,
    pub local_path: String,
    pub remote_path: String,
    pub rev_range: String,
    pub target_branch: String,
    #[serde(default)]
    pub bundle_bytes_base64: Option<String>,
}

/// Result of `worktree.sync_delta` or `worktree.syncDelta`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSyncDeltaResult {
    pub synced: bool,
    pub target_branch: String,
    pub bundle_size_bytes: usize,
    pub message: String,
}
