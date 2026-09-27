use crate::ids::{EnvironmentId, SshProfileId};
use serde::{Deserialize, Serialize};

/// Target machine or execution host where threads, worktrees, and schedules run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub id: EnvironmentId,
    pub kind: EnvironmentKind,
    pub display_name: String,
    #[serde(default)]
    pub status: EnvironmentStatus,
    #[serde(default)]
    pub platform: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EnvironmentKind {
    Local,
    Ssh { profile_id: SshProfileId },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EnvironmentStatus {
    Connected,
    Connecting,
    #[default]
    Disconnected,
    Unreachable,
    Error {
        message: String,
    },
}
