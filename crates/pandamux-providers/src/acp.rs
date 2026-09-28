use pandamux_core::{
    event::{ApprovalDecision, ApprovalKind, TurnOutcome},
    thread::{AccessMode, TurnInput, TurnUsage},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use crate::error::ProviderError;
use crate::models::ProviderEvent;
use crate::supervision::SupervisedChild;
use crate::traits::{BoxFuture, ProviderSession};

pub const ERROR_AUTH_REQUIRED: i64 = -32000;
pub const ERROR_INTERNAL_FAILURE: i64 = -32603;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcpMode {
    Default,
    AutoEdit,
    Yolo,
}

impl AcpMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AutoEdit => "auto_edit",
            Self::Yolo => "yolo",
        }
    }
}

/// Maps abstract AccessMode to ACP mode string.
pub fn map_access_mode_to_acp(mode: AccessMode) -> AcpMode {
    match mode {
        AccessMode::FullAccess => AcpMode::Yolo,
        AccessMode::AutoEdit => AcpMode::AutoEdit,
        AccessMode::ReadOnly | AccessMode::Ask | AccessMode::WorkspaceOnly => AcpMode::Default,
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
pub struct AcpInitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(rename = "agentInfo")]
    pub agent_info: AcpAgentInfo,
    pub capabilities: AcpCapabilities,
    #[serde(rename = "authMethods")]
    pub auth_methods: Vec<String>,
}

/// Validates that an ACP initialize response satisfies Antigravity requirements.
pub fn validate_antigravity_initialize(res: &AcpInitializeResult) -> Result<(), ProviderError> {
    if res.agent_info.name != "antigravity-acp" {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: format!(
                "Expected agentInfo.name 'antigravity-acp', got '{}'",
                res.agent_info.name
            ),
        });
    }
    if res.protocol_version != 1 {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: format!("Expected protocolVersion 1, got {}", res.protocol_version),
        });
    }
    if !res.capabilities.load_session && !res.capabilities.resume_session {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: "ACP server missing session load/resume capabilities".into(),
        });
    }
    if !res.auth_methods.iter().any(|m| m == "oauth-personal") {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: "ACP server missing 'oauth-personal' auth method".into(),
        });
    }

    Ok(())
}

/// Constructs a `session/new` JSON-RPC request configuring client capabilities.
/// Enforces: terminal=false, fs.readTextFile=true, fs.writeTextFile=true.
pub fn build_session_new_request(id: u64, mode: AcpMode) -> Value {
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

/// Constructs an `initialize` JSON-RPC request.
pub fn build_initialize_request(id: u64) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientInfo": {
                "name": "pandamux",
                "version": "1.0.0"
            }
        }
    })
}

/// Permission request received from ACP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpPermissionRequest {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "permissionType")]
    pub permission_type: String,
    pub resource: String,
    pub options: Vec<String>,
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
}

impl AcpPermissionRequest {
    /// Extracts security warning from `_meta["agy.security.warning"]` if present.
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

/// Heuristically classifies whether an ACP tool call represents a sub-agent invocation.
pub fn is_heuristic_subagent(tool_name: &str, tool_title: Option<&str>) -> bool {
    if tool_name.contains("subagent") || tool_name.contains("start_subagent") {
        return true;
    }
    if let Some(title) = tool_title {
        let lower = title.to_lowercase();
        if lower.contains("subagent") || lower.contains("start_subagent") {
            return true;
        }
    }
    false
}

/// Parses an ACP JSON-RPC line into a ProviderEvent if relevant.
pub fn parse_acp_line(line: &str) -> Option<ProviderEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let val: Value = serde_json::from_str(line).ok()?;

    if let Some(method) = val.get("method").and_then(|v| v.as_str()) {
        let params = val.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "session/promptUpdate" | "session/update" => {
                if let Some(text) = params.get("delta").and_then(|v| v.as_str()) {
                    return Some(ProviderEvent::TextDelta {
                        delta: text.to_string(),
                    });
                }
                if let Some(reasoning) = params.get("reasoningDelta").and_then(|v| v.as_str()) {
                    return Some(ProviderEvent::ReasoningDelta {
                        delta: reasoning.to_string(),
                    });
                }
            }
            "request_permission" | "session/request_permission" => {
                let req_id = val.get("id").map(|v| v.to_string()).unwrap_or_default();
                let perm_type = params.get("permissionType").and_then(|v| v.as_str());

                let tool_call_tool = params
                    .get("toolCall")
                    .and_then(|t| t.get("tool"))
                    .and_then(|t| t.as_str());

                let resource = params
                    .get("resource")
                    .and_then(|v| v.as_str())
                    .or(tool_call_tool)
                    .unwrap_or_default();

                let kind = if let Some(pt) = perm_type {
                    match pt {
                        "bash" | "command" => ApprovalKind::CommandExecution,
                        "fs_write" | "file_write" => ApprovalKind::FileEdit,
                        _ => ApprovalKind::ToolCall,
                    }
                } else if let Some(tc) = tool_call_tool {
                    match tc {
                        "run_command" | "bash" | "execute_command" => {
                            ApprovalKind::CommandExecution
                        }
                        "edit_file" | "write_file" | "replace_content" => ApprovalKind::FileEdit,
                        _ => ApprovalKind::ToolCall,
                    }
                } else {
                    ApprovalKind::ToolCall
                };

                return Some(ProviderEvent::ApprovalRequested {
                    request_id: req_id,
                    kind,
                    detail: params.clone(),
                    command: Some(resource.to_string()),
                });
            }
            "session/toolCall" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let title = params.get("title").and_then(|v| v.as_str());

                if is_heuristic_subagent(&name, title) {
                    return Some(ProviderEvent::SubAgentSpawned {
                        tool_use_id: id,
                        parent_tool_use_id: None,
                        agent_type: "subagent".to_string(),
                        description: title.unwrap_or(&name).to_string(),
                        model: None,
                    });
                } else {
                    return Some(ProviderEvent::ToolCallStarted {
                        id,
                        name,
                        input: params,
                    });
                }
            }
            "session/completed" => {
                let tokens = params
                    .get("usage")
                    .and_then(|u| u.get("totalTokens"))
                    .or_else(|| params.get("totalTokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                return Some(ProviderEvent::TurnCompleted {
                    outcome: TurnOutcome::Success,
                    usage: Some(TurnUsage {
                        input_tokens: tokens,
                        output_tokens: 0,
                        ..Default::default()
                    }),
                });
            }
            _ => {}
        }
    }

    None
}

/// An active ACP session running over stdio.
pub struct AcpSession {
    child: SupervisedChild,
    stdin: tokio::process::ChildStdin,
    events_rx: Option<mpsc::Receiver<ProviderEvent>>,
    next_id: Arc<AtomicU64>,
    session_id: String,
    preamble: Option<String>,
}

impl AcpSession {
    pub fn new(
        child: SupervisedChild,
        stdin: tokio::process::ChildStdin,
        events_rx: mpsc::Receiver<ProviderEvent>,
        session_id: String,
    ) -> Self {
        Self {
            child,
            stdin,
            events_rx: Some(events_rx),
            next_id: Arc::new(AtomicU64::new(10)),
            session_id,
            preamble: None,
        }
    }

    pub fn with_preamble(mut self, preamble: Option<String>) -> Self {
        self.preamble = preamble;
        self
    }
}

impl ProviderSession for AcpSession {
    fn send_turn<'a>(&'a mut self, input: TurnInput) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let id = self.next_id.fetch_add(1, Ordering::SeqCst);
            let mut prompt_text = String::new();
            if let Some(preamble) = self.preamble.take() {
                prompt_text.push_str(&preamble);
                prompt_text.push_str("\n\n");
            }
            prompt_text.push_str(&input.text);
            if !input.attachments.is_empty() {
                prompt_text.push_str(&pandamux_core::format_attachments_summary(
                    &input.attachments,
                ));
            }

            let mut params_map = serde_json::Map::new();
            params_map.insert(
                "sessionId".into(),
                serde_json::Value::String(self.session_id.clone()),
            );
            params_map.insert("prompt".into(), serde_json::Value::String(prompt_text));

            if !input.attachments.is_empty() {
                let att_list: Vec<serde_json::Value> = input
                    .attachments
                    .iter()
                    .map(|a| {
                        serde_json::json!({
                            "id": a.id,
                            "name": a.file_name,
                            "mimeType": a.mime_type,
                            "size": a.size_bytes,
                            "path": a.file_path,
                        })
                    })
                    .collect();
                params_map.insert("attachments".into(), serde_json::Value::Array(att_list));
            }

            let req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "session/prompt",
                "params": serde_json::Value::Object(params_map)
            });
            let mut line = serde_json::to_string(&req)?;
            line.push('\n');
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await?;
            Ok(())
        })
    }

    fn events(&mut self) -> &mut mpsc::Receiver<ProviderEvent> {
        self.events_rx
            .as_mut()
            .expect("events receiver already taken")
    }

    fn take_event_receiver(&mut self) -> Option<mpsc::Receiver<ProviderEvent>> {
        self.events_rx.take()
    }

    fn respond_approval<'a>(
        &'a mut self,
        request_id: String,
        decision: ApprovalDecision,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let id_val: Value =
                serde_json::from_str(&request_id).unwrap_or(Value::String(request_id));
            let outcome = match decision {
                ApprovalDecision::Approved => "allow_once",
                ApprovalDecision::Denied | ApprovalDecision::TimedOut => "reject_once",
            };

            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id_val,
                "result": {
                    "outcome": outcome
                }
            });

            let mut line = serde_json::to_string(&resp)?;
            line.push('\n');
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await?;
            Ok(())
        })
    }

    fn interrupt<'a>(&'a mut self) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let id = self.next_id.fetch_add(1, Ordering::SeqCst);
            let req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "session/cancel",
                "params": {
                    "sessionId": self.session_id
                }
            });
            let mut line = serde_json::to_string(&req)?;
            line.push('\n');
            let _ = self.stdin.write_all(line.as_bytes()).await;
            let _ = self.stdin.flush().await;
            Ok(())
        })
    }

    fn resume_token(&self) -> Option<String> {
        Some(self.session_id.clone())
    }

    fn shutdown(mut self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let _ = self.child.kill().await;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_acp_cursor_permission_line() {
        let line = r#"{"jsonrpc":"2.0","id":105,"method":"session/request_permission","params":{"sessionId":"sess-1","permissionId":"p-1","toolCall":{"id":"tc-1","tool":"edit_file","parameters":{"path":"src/main.rs"}}}}"#;
        let event = parse_acp_line(line).expect("Should parse permission line");
        match event {
            ProviderEvent::ApprovalRequested {
                request_id,
                kind,
                command,
                ..
            } => {
                assert_eq!(request_id, "105");
                assert_eq!(kind, ApprovalKind::FileEdit);
                assert_eq!(command, Some("edit_file".to_string()));
            }
            _ => panic!("Expected ApprovalRequested"),
        }
    }

    #[test]
    fn test_parse_acp_bash_permission_line() {
        let line = r#"{"jsonrpc":"2.0","id":106,"method":"session/request_permission","params":{"sessionId":"sess-1","permissionId":"p-2","toolCall":{"id":"tc-2","tool":"run_command","parameters":{"command":"cargo test"}}}}"#;
        let event = parse_acp_line(line).expect("Should parse bash permission line");
        match event {
            ProviderEvent::ApprovalRequested {
                request_id,
                kind,
                command,
                ..
            } => {
                assert_eq!(request_id, "106");
                assert_eq!(kind, ApprovalKind::CommandExecution);
                assert_eq!(command, Some("run_command".to_string()));
            }
            _ => panic!("Expected ApprovalRequested"),
        }
    }
}
