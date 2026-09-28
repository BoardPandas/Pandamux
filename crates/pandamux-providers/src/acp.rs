use pandamux_core::{
    event::{ApprovalDecision, ApprovalKind, TurnOutcome},
    thread::{AccessMode, TurnInput, TurnUsage},
};
use serde::{Deserialize, Deserializer, Serialize};
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
    #[serde(
        rename = "resumeSession",
        default,
        deserialize_with = "deserialize_capability_support"
    )]
    pub resume_session: bool,
    #[serde(rename = "sessionCapabilities", default)]
    pub session_capabilities: AcpSessionCapabilities,
    #[serde(default)]
    pub auth: AcpAuthCapabilities,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AcpSessionCapabilities {
    #[serde(default, deserialize_with = "deserialize_capability_support")]
    pub resume: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AcpAuthCapabilities {
    #[serde(default, deserialize_with = "deserialize_capability_support")]
    pub logout: bool,
}

fn deserialize_capability_support<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(matches!(value, Value::Bool(true) | Value::Object(_)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AcpAuthMethod {
    Id(String),
    Descriptor { id: String },
}

impl AcpAuthMethod {
    pub fn id(&self) -> &str {
        match self {
            Self::Id(id) | Self::Descriptor { id } => id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpInitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(rename = "agentInfo")]
    pub agent_info: AcpAgentInfo,
    #[serde(rename = "agentCapabilities", alias = "capabilities")]
    pub capabilities: AcpCapabilities,
    #[serde(rename = "authMethods", default)]
    pub auth_methods: Vec<AcpAuthMethod>,
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
    if !res.capabilities.load_session {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: "ACP server missing session load capability".into(),
        });
    }
    if !res.capabilities.session_capabilities.resume && !res.capabilities.resume_session {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: "ACP server missing session resume capability".into(),
        });
    }
    if !res.capabilities.auth.logout {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: "ACP server missing authentication logout capability".into(),
        });
    }
    if !res
        .auth_methods
        .iter()
        .any(|method| method.id() == "oauth-personal")
    {
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
            "clientCapabilities": {
                "fs": {
                    "readTextFile": false,
                    "writeTextFile": false
                },
                "terminal": false
            },
            "clientInfo": {
                "name": "pandamux",
                "version": env!("CARGO_PKG_VERSION")
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
    tool_name == "start_subagent"
        || matches!(
            tool_title,
            Some("Running start_subagent" | "Run start_subagent?")
        )
}

const MAX_RETAINED_TOOL_TEXT: usize = 8_000;
const MAX_RETAINED_TOOL_NODES: usize = 512;

fn bounded_text(value: &str) -> String {
    if value.len() <= MAX_RETAINED_TOOL_TEXT {
        value.to_string()
    } else {
        let mut start = value.len() - MAX_RETAINED_TOOL_TEXT;
        while !value.is_char_boundary(start) {
            start += 1;
        }
        format!("[Earlier output truncated]\n\n{}", &value[start..])
    }
}

fn sanitize_tool_value(value: &Value, nodes: &mut usize) -> Option<Value> {
    if *nodes == 0 {
        return None;
    }
    *nodes -= 1;
    match value {
        Value::String(text) if text.starts_with("data:image/") => None,
        Value::String(text) => Some(Value::String(bounded_text(text))),
        Value::Array(values) => Some(Value::Array(
            values
                .iter()
                .filter_map(|value| sanitize_tool_value(value, nodes))
                .collect(),
        )),
        Value::Object(values) => {
            let mut sanitized = serde_json::Map::new();
            for (key, value) in values {
                if *nodes == 0 {
                    break;
                }
                if (key == "data" || key == "blob")
                    && values.get("type").and_then(Value::as_str) == Some("image")
                {
                    continue;
                }
                if let Some(value) = sanitize_tool_value(value, nodes) {
                    sanitized.insert(key.clone(), value);
                }
            }
            Some(Value::Object(sanitized))
        }
        other => Some(other.clone()),
    }
}

fn sanitized_tool_value(value: &Value) -> Value {
    let mut nodes = MAX_RETAINED_TOOL_NODES;
    sanitize_tool_value(value, &mut nodes).unwrap_or(Value::Null)
}

fn content_text(content: &Value) -> Option<&str> {
    (content.get("type").and_then(Value::as_str) == Some("text"))
        .then(|| content.get("text").and_then(Value::as_str))
        .flatten()
}

fn permission_kind(params: &Value) -> ApprovalKind {
    let tool = params.get("toolCall").unwrap_or(&Value::Null);
    let kind = tool
        .get("kind")
        .and_then(Value::as_str)
        .or_else(|| tool.get("tool").and_then(Value::as_str))
        .or_else(|| params.get("permissionType").and_then(Value::as_str));
    let title = tool
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match kind {
        Some("execute" | "bash" | "command" | "run_command" | "execute_command") => {
            ApprovalKind::CommandExecution
        }
        Some(
            "edit" | "delete" | "move" | "fs_write" | "file_write" | "edit_file" | "write_file"
            | "replace_content",
        ) => ApprovalKind::FileEdit,
        _ if title.contains("command") || title.contains("shell") => ApprovalKind::CommandExecution,
        _ if title.contains("edit") || title.contains("write") => ApprovalKind::FileEdit,
        _ => ApprovalKind::ToolCall,
    }
}

fn permission_command(params: &Value) -> Option<String> {
    let tool = params.get("toolCall")?;
    let raw = tool.get("rawInput").or_else(|| tool.get("parameters"));
    let command = raw.and_then(|raw| {
        ["CommandLine", "command_line", "commandLine", "command"]
            .iter()
            .find_map(|name| raw.get(*name).and_then(Value::as_str))
    });
    command
        .or_else(|| tool.get("title").and_then(Value::as_str))
        .or_else(|| tool.get("tool").and_then(Value::as_str))
        .map(bounded_text)
}

fn turn_usage(value: &Value) -> Option<TurnUsage> {
    let usage = value.get("usage")?;
    let input_tokens = usage
        .get("inputTokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("outputTokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(TurnUsage {
        input_tokens,
        output_tokens,
        ..Default::default()
    })
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
            "session/promptUpdate" => {
                if let Some(text) = params.get("delta").and_then(Value::as_str) {
                    return Some(ProviderEvent::TextDelta {
                        delta: text.to_string(),
                    });
                }
                if let Some(reasoning) = params.get("reasoningDelta").and_then(Value::as_str) {
                    return Some(ProviderEvent::ReasoningDelta {
                        delta: reasoning.to_string(),
                    });
                }
            }
            "session/update" => {
                let update = params.get("update").unwrap_or(&params);
                match update.get("sessionUpdate").and_then(Value::as_str) {
                    Some("agent_message_chunk") => {
                        let text = update.get("content").and_then(content_text)?;
                        return Some(ProviderEvent::TextDelta {
                            delta: text.to_string(),
                        });
                    }
                    Some("agent_thought_chunk") => {
                        let text = update.get("content").and_then(content_text)?;
                        return Some(ProviderEvent::ReasoningDelta {
                            delta: text.to_string(),
                        });
                    }
                    Some("tool_call") => {
                        let id = update
                            .get("toolCallId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let title = update
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_string();
                        let kind = update
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("other");
                        let is_mcp = update
                            .get("_meta")
                            .and_then(|meta| meta.get("is_mcp_tool_call"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        if !is_mcp && is_heuristic_subagent("", Some(&title)) && kind == "other" {
                            return Some(ProviderEvent::SubAgentSpawned {
                                tool_use_id: id,
                                parent_tool_use_id: None,
                                agent_type: "subagent".to_string(),
                                description: title,
                                model: None,
                            });
                        }
                        return Some(ProviderEvent::ToolCallStarted {
                            id,
                            name: title,
                            input: sanitized_tool_value(update),
                        });
                    }
                    Some("tool_call_update") => {
                        let status = update
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if matches!(status, "completed" | "failed") {
                            let output = update
                                .get("rawOutput")
                                .map(sanitized_tool_value)
                                .and_then(|value| serde_json::to_string(&value).ok());
                            return Some(ProviderEvent::ToolCallFinished {
                                id: update
                                    .get("toolCallId")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                output,
                                error: (status == "failed").then(|| {
                                    update
                                        .get("title")
                                        .and_then(Value::as_str)
                                        .unwrap_or("Antigravity tool call failed")
                                        .to_string()
                                }),
                            });
                        }
                    }
                    Some("usage_update") => {
                        return Some(ProviderEvent::RateLimitObserved {
                            details: sanitized_tool_value(update),
                        });
                    }
                    _ => {}
                }
            }
            "request_permission" | "session/request_permission" => {
                let req_id = val.get("id").map(|v| v.to_string()).unwrap_or_default();
                return Some(ProviderEvent::ApprovalRequested {
                    request_id: req_id,
                    kind: permission_kind(&params),
                    detail: sanitized_tool_value(&params),
                    command: permission_command(&params),
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

    if let Some(result) = val.get("result")
        && let Some(stop_reason) = result.get("stopReason").and_then(Value::as_str)
    {
        let outcome = match stop_reason {
            "cancelled" => TurnOutcome::Interrupted,
            "refusal" | "error" => TurnOutcome::Failed,
            _ => TurnOutcome::Success,
        };
        return Some(ProviderEvent::TurnCompleted {
            outcome,
            usage: turn_usage(result),
        });
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
                assert_eq!(command, Some("cargo test".to_string()));
            }
            _ => panic!("Expected ApprovalRequested"),
        }
    }

    #[test]
    fn parses_standard_acp_v1_updates_and_prompt_completion() {
        let text = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}}}"#;
        assert_eq!(
            parse_acp_line(text),
            Some(ProviderEvent::TextDelta {
                delta: "hello".to_string()
            })
        );

        let tool = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"tool_call","toolCallId":"tool-1","title":"Run command","kind":"execute","status":"in_progress","rawInput":{"CommandLine":"cargo test"}}}}"#;
        match parse_acp_line(tool).expect("tool event") {
            ProviderEvent::ToolCallStarted { id, name, input } => {
                assert_eq!(id, "tool-1");
                assert_eq!(name, "Run command");
                assert_eq!(input["rawInput"]["CommandLine"], "cargo test");
            }
            event => panic!("unexpected event: {event:?}"),
        }

        let completed = r#"{"jsonrpc":"2.0","id":10,"result":{"stopReason":"end_turn","usage":{"inputTokens":7,"outputTokens":11}}}"#;
        assert_eq!(
            parse_acp_line(completed),
            Some(ProviderEvent::TurnCompleted {
                outcome: TurnOutcome::Success,
                usage: Some(TurnUsage {
                    input_tokens: 7,
                    output_tokens: 11,
                    ..Default::default()
                })
            })
        );
    }

    #[test]
    fn bounds_utf8_tool_payloads_without_splitting_a_character() {
        let value = Value::String("🦀".repeat(MAX_RETAINED_TOOL_TEXT));
        let sanitized = sanitized_tool_value(&value);
        let text = sanitized.as_str().expect("sanitized string");
        assert!(text.starts_with("[Earlier output truncated]"));
        assert!(text.ends_with('🦀'));
    }
}
