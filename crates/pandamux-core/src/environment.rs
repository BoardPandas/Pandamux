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
    #[serde(default)]
    pub server_version: Option<String>,
    #[serde(default)]
    pub schedules_count: Option<usize>,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl Environment {
    pub fn local_default() -> Self {
        Self {
            id: EnvironmentId::new("env_local"),
            kind: EnvironmentKind::Local,
            display_name: "Local Machine".to_string(),
            status: EnvironmentStatus::Ready,
            platform: Some(format!(
                "{} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )),
            server_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            schedules_count: Some(0),
            last_error: None,
        }
    }
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
    Bootstrapping,
    Ready,
    Degraded,
    Offline,
    #[default]
    Disconnected,
    Unreachable,
    Error {
        message: String,
    },
}
