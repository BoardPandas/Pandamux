use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 3;

/// Parameters for initial `system.hello` negotiation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloParams {
    pub protocol_version: u32,
    pub client_kind: String,
    pub client_version: String,
}

/// Server reply to `system.hello`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloResult {
    pub server_version: String,
    pub protocol_version: u32,
    pub role: ServerRole,
    pub capabilities: ServerCapabilities,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerRole {
    Hub,
    Node,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub streaming: bool,
    pub attachments: bool,
    pub worktrees: bool,
    pub mcp: bool,
    pub schedules: bool,
}

/// Result of `system.ping`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub pong: bool,
    pub timestamp_ms: u64,
}

/// Result of `system.identify`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentifyResult {
    pub server_version: String,
    pub protocol_version: u32,
    pub role: ServerRole,
    pub platform: String,
    pub environment_id: String,
}
