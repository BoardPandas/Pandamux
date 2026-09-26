use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodexSandboxPolicy {
    #[serde(rename = "read-only")]
    ReadOnly,
    #[serde(rename = "workspace-write")]
    WorkspaceWrite,
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodexApprovalPolicy {
    #[serde(rename = "never")]
    Never,
    #[serde(rename = "on-write")]
    OnWrite,
    #[serde(rename = "always")]
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitEntry {
    pub limit_name: String,
    pub used_percent: f64,
    pub window_duration_mins: u32,
    pub resets_at: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum CodexNotificationEvent {
    TextDelta(String),
    ApprovalRequest {
        approval_id: String,
        command: String,
        reason: String,
    },
    CommandOutput {
        stdout: String,
        exit_code: i32,
    },
    TurnCompleted {
        total_tokens: u64,
    },
    Other(String),
}

/// Builds a `thread/start` request injecting developer instructions and security policies.
pub fn build_thread_start_request(
    id: u64,
    cwd: &str,
    model: &str,
    developer_instructions: Option<&str>,
    sandbox_policy: CodexSandboxPolicy,
    approval_policy: CodexApprovalPolicy,
) -> JsonRpcRequest {
    let mut params_map = serde_json::Map::new();
    params_map.insert("cwd".into(), Value::String(cwd.into()));
    params_map.insert("model".into(), Value::String(model.into()));
    params_map.insert(
        "sandboxPolicy".into(),
        serde_json::to_value(sandbox_policy).unwrap(),
    );
    params_map.insert(
        "approvalPolicy".into(),
        serde_json::to_value(approval_policy).unwrap(),
    );

    if let Some(instructions) = developer_instructions {
        params_map.insert(
            "developerInstructions".into(),
            Value::String(instructions.into()),
        );
    }

    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id,
        method: "thread/start".into(),
        params: Value::Object(params_map),
    }
}

/// Builds a `turn/start` request for initiating an agent turn.
pub fn build_turn_start_request(id: u64, thread_id: &str, prompt: &str) -> JsonRpcRequest {
    let input_item = serde_json::json!({
        "type": "text",
        "text": prompt
    });

    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id,
        method: "turn/start".into(),
        params: serde_json::json!({
            "threadId": thread_id,
            "input": [input_item]
        }),
    }
}

/// Builds a `turn/approvalResponse` responding to an `approvalRequest`.
pub fn build_approval_response(id: u64, approval_id: &str, approved: bool) -> JsonRpcRequest {
    let decision = if approved { "approved" } else { "rejected" };
    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id,
        method: "turn/approvalResponse".into(),
        params: serde_json::json!({
            "approvalId": approval_id,
            "decision": decision
        }),
    }
}

/// Parses an `account/rateLimits/read` response payload into structured rate limit entries.
pub fn parse_rate_limits_response(resp: &JsonRpcResponse) -> Result<Vec<RateLimitEntry>> {
    if let Some(err) = &resp.error {
        bail!("Rate limits query returned error: {:?}", err);
    }

    let Some(result) = &resp.result else {
        bail!("Rate limits query response missing result field");
    };

    let limits_val = result
        .get("rateLimits")
        .context("Missing 'rateLimits' array in result")?;

    let entries: Vec<RateLimitEntry> = serde_json::from_value(limits_val.clone())
        .context("Failed to deserialize rate limit entries")?;

    Ok(entries)
}

/// Parses a Codex JSON-RPC notification line.
pub fn parse_codex_notification(line: &str) -> Result<CodexNotificationEvent> {
    let notif: JsonRpcNotification = serde_json::from_str(line)
        .with_context(|| format!("Failed to parse line as JsonRpcNotification: {}", line))?;

    match notif.method.as_str() {
        "turn/event" => {
            let event = notif.params.get("event").context("Missing event object")?;
            let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or_default();
            match event_type {
                "text_delta" => {
                    let text = event.get("text").and_then(|v| v.as_str()).unwrap_or_default();
                    Ok(CodexNotificationEvent::TextDelta(text.into()))
                }
                "command_output" => {
                    let stdout = event.get("stdout").and_then(|v| v.as_str()).unwrap_or_default();
                    let exit_code = event.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    Ok(CodexNotificationEvent::CommandOutput {
                        stdout: stdout.into(),
                        exit_code,
                    })
                }
                _ => Ok(CodexNotificationEvent::Other(event_type.into())),
            }
        }
        "turn/approvalRequest" => {
            let approval_id = notif.params.get("approvalId").and_then(|v| v.as_str()).unwrap_or_default();
            let command = notif.params.get("command").and_then(|v| v.as_str()).unwrap_or_default();
            let reason = notif.params.get("reason").and_then(|v| v.as_str()).unwrap_or_default();
            Ok(CodexNotificationEvent::ApprovalRequest {
                approval_id: approval_id.into(),
                command: command.into(),
                reason: reason.into(),
            })
        }
        "turn/completed" => {
            let usage = notif.params.get("usage");
            let total = usage.and_then(|u| u.get("totalTokens")).and_then(|v| v.as_u64()).unwrap_or(0);
            Ok(CodexNotificationEvent::TurnCompleted {
                total_tokens: total,
            })
        }
        other => Ok(CodexNotificationEvent::Other(other.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thread_start_request_serialization() {
        let req = build_thread_start_request(
            1,
            "C:\\Project",
            "o3-mini",
            Some("System instructions for tester"),
            CodexSandboxPolicy::WorkspaceWrite,
            CodexApprovalPolicy::OnWrite,
        );

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"developerInstructions\":\"System instructions for tester\""));
        assert!(json.contains("\"sandboxPolicy\":\"workspace-write\""));
        assert!(json.contains("\"approvalPolicy\":\"on-write\""));
    }

    #[test]
    fn test_rate_limits_parsing() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: 42,
            result: Some(serde_json::json!({
                "rateLimits": [
                    {
                        "limitName": "tokens",
                        "usedPercent": 55.4,
                        "windowDurationMins": 300,
                        "resetsAt": "2026-09-26T04:00:00Z"
                    }
                ]
            })),
            error: None,
        };

        let limits = parse_rate_limits_response(&resp).unwrap();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].limit_name, "tokens");
        assert_eq!(limits[0].used_percent, 55.4);
    }
}
