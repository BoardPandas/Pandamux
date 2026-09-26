use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ERROR_AUTH_REQUIRED: i64 = -32000;
pub const ERROR_INTERNAL_FAILURE: i64 = -32603;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntigravityMode {
    Default,
    AutoEdit,
    Yolo,
}

impl AntigravityMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AutoEdit => "auto_edit",
            Self::Yolo => "yolo",
        }
    }
}

/// Maps PandaMUX abstract AccessMode onto native Antigravity mode.
pub fn map_access_mode_to_antigravity(access_mode: &str) -> AntigravityMode {
    match access_mode.to_lowercase().as_str() {
        "fullaccess" | "full_access" | "yolo" => AntigravityMode::Yolo,
        "autoedit" | "auto_edit" => AntigravityMode::AutoEdit,
        _ => AntigravityMode::Default,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpAgentInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpCapabilities {
    #[serde(rename = "loadSession", default)]
    pub load_session: bool,
    #[serde(rename = "resumeSession", default)]
    pub resume_session: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(rename = "agentInfo")]
    pub agent_info: AcpAgentInfo,
    pub capabilities: AcpCapabilities,
    #[serde(rename = "authMethods")]
    pub auth_methods: Vec<String>,
}

/// Validates that an ACP `initialize` response satisfies the required Antigravity capabilities.
pub fn validate_antigravity_initialize(res: &InitializeResult) -> Result<()> {
    if res.agent_info.name != "antigravity-acp" {
        bail!("Expected agentInfo.name 'antigravity-acp', got '{}'", res.agent_info.name);
    }
    if res.protocol_version != 1 {
        bail!("Expected protocolVersion 1, got {}", res.protocol_version);
    }
    if !res.capabilities.load_session && !res.capabilities.resume_session {
        bail!("ACP server missing required session load/resume capabilities");
    }
    if !res.auth_methods.iter().any(|m| m == "oauth-personal") {
        bail!("ACP server missing required 'oauth-personal' auth method");
    }

    Ok(())
}

/// Constructs a `session/new` JSON-RPC request configuring client capabilities.
/// Enforces: terminal=false, fs.readTextFile=true, fs.writeTextFile=true.
pub fn build_session_new_request(id: u64, mode: AntigravityMode) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "session/new",
        "params": {
            "mode": mode.as_str(),
            "clientCapabilities": {
                "terminal": false,
                "fs": {
                    "readTextFile": true,
                    "writeTextFile": true
                }
            }
        }
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequestParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "permissionType")]
    pub permission_type: String,
    pub resource: String,
    pub options: Vec<String>,
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
}

impl PermissionRequestParams {
    /// Extracts prompt injection security warning from `_meta["agy.security.warning"]` if present.
    pub fn security_warning(&self) -> Option<String> {
        self.meta
            .as_ref()?
            .get("agy.security.warning")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

/// Identifies if a tool call is a native interactive prompt question (prefixed with `interaction_`).
pub fn is_interaction_prompt(tool_name: &str) -> bool {
    tool_name.starts_with("interaction_")
}

/// Maps ACP error codes to domain outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpErrorOutcome {
    AuthRequired,
    InternalSessionFailure,
    Generic(i64),
}

pub fn map_acp_error_code(code: i64) -> AcpErrorOutcome {
    match code {
        ERROR_AUTH_REQUIRED => AcpErrorOutcome::AuthRequired,
        ERROR_INTERNAL_FAILURE => AcpErrorOutcome::InternalSessionFailure,
        other => AcpErrorOutcome::Generic(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_mapping() {
        assert_eq!(map_access_mode_to_antigravity("FullAccess"), AntigravityMode::Yolo);
        assert_eq!(map_access_mode_to_antigravity("auto_edit"), AntigravityMode::AutoEdit);
        assert_eq!(map_access_mode_to_antigravity("read_only"), AntigravityMode::Default);
    }

    #[test]
    fn test_interaction_detection() {
        assert!(is_interaction_prompt("interaction_confirm_delete"));
        assert!(!is_interaction_prompt("fs_write_file"));
    }
}
