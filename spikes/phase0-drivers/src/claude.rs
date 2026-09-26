use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAuthStatus {
    pub logged_in: bool,
    pub auth_method: Option<String>,
    pub api_provider: Option<String>,
    pub email: Option<String>,
    pub subscription_type: Option<String>,
    pub analytics_disabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    pub request_id: String,
    pub tool_name: String,
    pub command: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlResponse {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub request_id: String,
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ControlResponse {
    pub fn allow(request_id: &str) -> Self {
        Self {
            msg_type: "control_response".into(),
            request_id: request_id.into(),
            decision: "allow".into(),
            message: None,
        }
    }

    pub fn deny(request_id: &str, reason: &str) -> Self {
        Self {
            msg_type: "control_response".into(),
            request_id: request_id.into(),
            decision: "deny".into(),
            message: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentSpawn {
    pub tool_use_id: String,
    pub parent_tool_use_id: Option<String>,
    pub subagent_type: String,
    pub description: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamTurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
}

/// Executes a zero-session auth probe using `claude auth status --json`.
/// Does not start a session, allocate quota, or alter state.
pub fn probe_claude_auth(
    claude_bin: &Path,
    config_dir: Option<&Path>,
) -> Result<ClaudeAuthStatus> {
    let mut cmd = Command::new(claude_bin);
    cmd.arg("auth").arg("status").arg("--json");

    if let Some(dir) = config_dir {
        cmd.env("CLAUDE_CONFIG_DIR", dir);
    }

    let output = cmd
        .output()
        .with_context(|| format!("Failed to execute {:?}", claude_bin))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let status: ClaudeAuthStatus = serde_json::from_str(&stdout)
        .with_context(|| format!("Failed to parse auth status JSON: {}", stdout))?;

    Ok(status)
}

/// Builds invocation arguments for Claude Code in non-interactive streaming mode.
pub fn build_claude_launch_args(
    append_system_prompt: Option<&str>,
    allowed_tools: &[String],
    disallowed_tools: &[String],
    permission_mode: Option<&str>,
    session_id: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--verbose".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--permission-prompt-tool".to_string(),
        "stdio".to_string(),
    ];

    if let Some(prompt) = append_system_prompt {
        args.push("--append-system-prompt".to_string());
        args.push(prompt.to_string());
    }

    if let Some(mode) = permission_mode {
        args.push("--permission-mode".to_string());
        args.push(mode.to_string());
    }

    if let Some(sid) = session_id {
        args.push("--session-id".to_string());
        args.push(sid.to_string());
    }

    if !allowed_tools.is_empty() {
        args.push("--allowedTools".to_string());
        args.push(allowed_tools.join(","));
    }

    if !disallowed_tools.is_empty() {
        args.push("--disallowedTools".to_string());
        args.push(disallowed_tools.join(","));
    }

    args
}

/// Parses NDJSON events from Claude Code stream-json output.
pub fn parse_stream_json_lines(ndjson: &str) -> Result<ParsedClaudeStream> {
    let mut parsed = ParsedClaudeStream::default();

    for line in ndjson.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let val: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("Failed to parse line as JSON: {}", line))?;

        let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or_default();

        match event_type {
            "system" => {
                if let Some(sid) = val.get("session_id").and_then(|v| v.as_str()) {
                    parsed.session_id = Some(sid.to_string());
                }
            }
            "control_request" => {
                let req: ControlRequest = serde_json::from_value(val.clone())?;
                parsed.control_requests.push(req);
            }
            "tool_use" => {
                let name = val.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                if name == "Task" {
                    let tool_use_id = val.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let parent_tool_use_id = val.get("parent_tool_use_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let input = val.get("input").cloned().unwrap_or_default();
                    let subagent_type = input.get("subagent_type").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let description = input.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let model = input.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());

                    parsed.subagents.push(SubAgentSpawn {
                        tool_use_id,
                        parent_tool_use_id,
                        subagent_type,
                        description,
                        model,
                    });
                }
            }
            "result" => {
                if let Some(usage_val) = val.get("usage") {
                    let input_tokens = usage_val.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or_default();
                    let output_tokens = usage_val.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or_default();
                    let cost_usd = usage_val.get("cost_usd").and_then(|v| v.as_f64());

                    parsed.usage = Some(StreamTurnUsage {
                        input_tokens,
                        output_tokens,
                        cost_usd,
                    });
                }
            }
            _ => {}
        }
    }

    Ok(parsed)
}

#[derive(Debug, Default, Clone)]
pub struct ParsedClaudeStream {
    pub session_id: Option<String>,
    pub control_requests: Vec<ControlRequest>,
    pub subagents: Vec<SubAgentSpawn>,
    pub usage: Option<StreamTurnUsage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_status_parsing() {
        let raw = r#"{"loggedIn":false,"authMethod":"none","apiProvider":"firstParty","analyticsDisabled":false}"#;
        let status: ClaudeAuthStatus = serde_json::from_str(raw).unwrap();
        assert!(!status.logged_in);
        assert_eq!(status.auth_method.as_deref(), Some("none"));
    }

    #[test]
    fn test_control_response_generation() {
        let resp = ControlResponse::allow("req_123");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"decision\":\"allow\""));
        assert!(json.contains("\"request_id\":\"req_123\""));

        let denied = ControlResponse::deny("req_456", "Forbidden by security policy");
        let json_denied = serde_json::to_string(&denied).unwrap();
        assert!(json_denied.contains("\"decision\":\"deny\""));
        assert!(json_denied.contains("Forbidden by security policy"));
    }

    #[test]
    fn test_launch_args_building() {
        let args = build_claude_launch_args(
            Some("You are the Architect."),
            &["Read".into(), "Edit".into()],
            &["Bash".into()],
            Some("manual"),
            Some("123e4567-e89b-12d3-a456-426614174000"),
        );

        assert!(args.contains(&"--permission-prompt-tool".into()));
        assert!(args.contains(&"stdio".into()));
        assert!(args.contains(&"--append-system-prompt".into()));
        assert!(args.contains(&"You are the Architect.".into()));
        assert!(args.contains(&"--allowedTools".into()));
        assert!(args.contains(&"Read,Edit".into()));
        assert!(args.contains(&"--disallowedTools".into()));
        assert!(args.contains(&"Bash".into()));
    }
}
