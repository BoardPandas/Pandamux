use pandamux_core::ThreadId;
use serde::{Deserialize, Serialize};

/// Parameters for querying Git status in a thread's worktree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusParams {
    pub thread_id: ThreadId,
}

/// Status of an individual file in the Git worktree.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileStatus {
    pub path: String,
    pub status: String,
}

/// Result of querying Git status.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusResult {
    pub is_repo: bool,
    pub branch: String,
    #[serde(default)]
    pub tracking_branch: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub files: Vec<GitFileStatus>,
    pub drafted_commit_message: String,
}

/// Parameters for creating a Git commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub message: Option<String>,
}

/// Result of a Git commit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitResult {
    pub success: bool,
    pub commit_hash: String,
    pub message: String,
    pub files_committed: usize,
}

/// Parameters for pushing Git commits to a remote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPushParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

/// Result of a Git push operation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPushResult {
    pub success: bool,
    pub message: String,
}

/// Parameters for creating a Pull Request via gh or compare URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCreatePrParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

/// Result of creating a Pull Request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCreatePrResult {
    pub success: bool,
    pub url: String,
    pub method: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_rpc_serialization() {
        let status = GitStatusResult {
            is_repo: true,
            branch: "feature/settings".to_string(),
            tracking_branch: Some("origin/feature/settings".to_string()),
            ahead: 1,
            behind: 0,
            files: vec![GitFileStatus {
                path: "src/main.rs".to_string(),
                status: "modified".to_string(),
            }],
            drafted_commit_message: "feat: update main.rs".to_string(),
        };

        let json = serde_json::to_string(&status).unwrap();
        let parsed: GitStatusResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.branch, "feature/settings");
        assert_eq!(parsed.ahead, 1);
        assert_eq!(parsed.files.len(), 1);
    }
}
