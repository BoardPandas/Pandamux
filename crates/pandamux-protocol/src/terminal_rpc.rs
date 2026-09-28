use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Parameters for `terminal.open`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOpenParams {
    #[serde(default)]
    pub terminal_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub rows: Option<u16>,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
}

/// Result returned from `terminal.open`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOpenResult {
    pub terminal_id: String,
    pub cwd: String,
    pub rows: u16,
    pub cols: u16,
}

/// Metadata describing an active or recently finished terminal session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub terminal_id: String,
    pub cwd: String,
    pub rows: u16,
    pub cols: u16,
    pub running: bool,
    pub head_offset: u64,
}

/// Result returned from `terminal.list`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalListResult {
    pub terminals: Vec<TerminalInfo>,
}

/// Parameters for `terminal.attach`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalAttachParams {
    pub terminal_id: String,
    #[serde(default)]
    pub since_offset: Option<u64>,
}

/// Result returned from `terminal.attach`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalAttachResult {
    pub terminal_id: String,
    pub rows: u16,
    pub cols: u16,
    pub offset: u64,
    pub data: String,
    #[serde(default)]
    pub truncated: bool,
}

/// Parameters for `terminal.input`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInputParams {
    pub terminal_id: String,
    pub data: String,
}

/// Parameters for `terminal.resize`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalResizeParams {
    pub terminal_id: String,
    pub rows: u16,
    pub cols: u16,
}

/// Parameters for `terminal.close`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCloseParams {
    pub terminal_id: String,
}

/// Notification event broadcast when a terminal produces output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputEvent {
    pub terminal_id: String,
    pub offset: u64,
    pub data: String,
    #[serde(default)]
    pub truncated: bool,
}

/// Notification event broadcast when a terminal process exits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExitedEvent {
    pub terminal_id: String,
    pub exit_code: Option<u32>,
}
