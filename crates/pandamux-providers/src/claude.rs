use pandamux_core::{
    UsageUnknownField,
    event::{ApprovalDecision, ApprovalKind, TurnOutcome},
    provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind},
    thread::{AccessMode, TurnInput, TurnUsage, TurnUsageComponent, TurnUsageProvenance},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::error::ProviderError;
use crate::models::{
    ModelInfo, ProviderAuthConfidence, ProviderAuthStatus, ProviderEvent, ProviderHealth,
    ProviderMetadata, ProviderSnapshot, SessionSpec, UsageLimits, provider_snapshot_expiry,
    provider_snapshot_now_ms,
};
use crate::profiles::{
    apply_profile_environment, ensure_profile_dir, resolve_instance_profile_dir,
};
use crate::shim_resolver::resolve_command_shim;
use crate::supervision::{SupervisedChild, create_supervised_command};
use crate::traits::{BoxFuture, ProviderDriver, ProviderSession};
use crate::usage_transcript::{parse_provider_reported_micro_usd, parse_provider_timestamp_ms};

/// Claude authentication status returned by `claude auth status --json`.
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

/// Inbound permission control request from Claude Code stdio protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    pub request_id: String,
    pub tool_name: String,
    pub command: Option<String>,
    pub reason: Option<String>,
    #[serde(default)]
    pub input: serde_json::Value,
}

/// Outbound control response answering a `control_request`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlResponse {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub response: serde_json::Value,
}

impl ControlResponse {
    pub fn allow(request_id: &str, updated_input: serde_json::Value) -> Self {
        Self {
            msg_type: "control_response".into(),
            response: serde_json::json!({
                "subtype": "success",
                "request_id": request_id,
                "response": {
                    "behavior": "allow",
                    "updatedInput": updated_input
                }
            }),
        }
    }

    pub fn deny(request_id: &str, reason: &str) -> Self {
        Self {
            msg_type: "control_response".into(),
            response: serde_json::json!({
                "subtype": "success",
                "request_id": request_id,
                "response": {
                    "behavior": "deny",
                    "message": reason
                }
            }),
        }
    }
}

/// Subagent task spawn event extracted from Claude tool use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentSpawn {
    pub tool_use_id: String,
    pub parent_tool_use_id: Option<String>,
    pub subagent_type: String,
    pub description: String,
    pub model: Option<String>,
}

/// Turn token usage metrics parsed from Claude stream output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamTurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub components: Vec<TurnUsageComponent>,
}

/// Accumulated stream lines parsed for testing and validation.
#[derive(Debug, Default, Clone)]
pub struct ParsedClaudeStream {
    pub session_id: Option<String>,
    pub control_requests: Vec<ControlRequest>,
    pub subagents: Vec<SubAgentSpawn>,
    pub usage: Option<StreamTurnUsage>,
    pub assistant_text: String,
}

#[derive(Debug, Default)]
struct ClaudeTurnUsageAccumulator {
    session_id: Option<String>,
    components: Vec<TurnUsageComponent>,
    component_indexes: HashMap<String, usize>,
}

impl ClaudeTurnUsageAccumulator {
    fn observe(&mut self, value: &serde_json::Value) {
        if let Some(session_id) = value
            .get("session_id")
            .or_else(|| value.get("sessionId"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            self.session_id = Some(session_id.to_string());
        }
        if value.get("type").and_then(|value| value.as_str()) != Some("assistant") {
            return;
        }
        let Some(component) = parse_assistant_usage_component(value) else {
            return;
        };
        if let Some(index) = self
            .component_indexes
            .get(&component.provider_native_message_id)
            .copied()
        {
            merge_assistant_usage_component(&mut self.components[index], component);
            return;
        }
        let index = self.components.len();
        self.component_indexes
            .insert(component.provider_native_message_id.clone(), index);
        self.components.push(component);
    }

    fn finish(&mut self, result: &serde_json::Value) -> Option<TurnUsage> {
        self.observe(result);
        let usage = result.get("usage");
        let input_tokens = usage
            .and_then(|usage| usage.get("input_tokens"))
            .and_then(|value| value.as_u64());
        let output_tokens = usage
            .and_then(|usage| usage.get("output_tokens"))
            .and_then(|value| value.as_u64());
        let cache_read_tokens = usage
            .and_then(|usage| {
                usage
                    .get("cache_read_input_tokens")
                    .or_else(|| usage.get("cached_input_tokens"))
            })
            .and_then(|value| value.as_u64());
        let cache_write_tokens = usage
            .and_then(|usage| {
                usage
                    .get("cache_creation_input_tokens")
                    .or_else(|| usage.get("cache_write_input_tokens"))
            })
            .and_then(|value| value.as_u64());
        let cost = result
            .get("total_cost_usd")
            .or_else(|| result.get("cost_usd"))
            .or_else(|| usage.and_then(|usage| usage.get("cost_usd")));
        let cost_usd = cost.and_then(|value| value.as_f64());
        let provider_reported_micro_usd = cost.and_then(parse_provider_reported_micro_usd);
        let components = std::mem::take(&mut self.components);
        self.component_indexes.clear();

        if usage.is_none() && components.is_empty() && cost.is_none() {
            return None;
        }

        let mut unknown_fields = BTreeSet::new();
        let fallback_input = checked_component_sum(&components, |component| component.input_tokens);
        let fallback_output =
            checked_component_sum(&components, |component| component.output_tokens);
        let fallback_cache_read =
            checked_optional_component_sum(&components, |component| component.cache_read_tokens);
        let fallback_cache_write =
            checked_optional_component_sum(&components, |component| component.cache_write_tokens);
        if input_tokens.is_none() && fallback_input.is_none() {
            unknown_fields.insert(UsageUnknownField::InputTokens);
        }
        if output_tokens.is_none() && fallback_output.is_none() {
            unknown_fields.insert(UsageUnknownField::OutputTokens);
        }

        Some(TurnUsage {
            input_tokens: input_tokens.or(fallback_input).unwrap_or_default(),
            output_tokens: output_tokens.or(fallback_output).unwrap_or_default(),
            cache_read_tokens: cache_read_tokens.or(fallback_cache_read),
            cache_write_tokens: cache_write_tokens.or(fallback_cache_write),
            cost_usd,
            unknown_fields,
            provider_reported_micro_usd,
            provenance: Some(TurnUsageProvenance {
                provider_native_session_id: result
                    .get("session_id")
                    .or_else(|| result.get("sessionId"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .or_else(|| self.session_id.clone()),
                provider_native_turn_id: result
                    .get("turn_id")
                    .or_else(|| result.get("turnId"))
                    .or_else(|| result.get("uuid"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                occurred_at_ms: result
                    .get("timestamp")
                    .and_then(parse_provider_timestamp_ms),
                model: result
                    .get("model")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            }),
            components,
        })
    }
}

fn parse_assistant_usage_component(value: &serde_json::Value) -> Option<TurnUsageComponent> {
    let message = value.get("message").unwrap_or(value);
    let usage = message.get("usage")?;
    let provider_native_message_id = message.get("id")?.as_str()?.trim().to_string();
    if provider_native_message_id.is_empty() {
        return None;
    }
    Some(TurnUsageComponent {
        provider_native_message_id,
        model: message
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        input_tokens: usage
            .get("input_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or_default(),
        output_tokens: usage
            .get("output_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or_default(),
        cache_read_tokens: usage
            .get("cache_read_input_tokens")
            .or_else(|| usage.get("cached_input_tokens"))
            .and_then(|value| value.as_u64()),
        cache_write_tokens: usage
            .get("cache_creation_input_tokens")
            .or_else(|| usage.get("cache_write_input_tokens"))
            .and_then(|value| value.as_u64()),
        occurred_at_ms: value
            .get("timestamp")
            .or_else(|| message.get("created_at"))
            .and_then(parse_provider_timestamp_ms),
    })
}

fn merge_assistant_usage_component(current: &mut TurnUsageComponent, incoming: TurnUsageComponent) {
    if current.model.is_none() {
        current.model = incoming.model;
    }
    current.input_tokens = current.input_tokens.max(incoming.input_tokens);
    current.output_tokens = current.output_tokens.max(incoming.output_tokens);
    current.cache_read_tokens = match (current.cache_read_tokens, incoming.cache_read_tokens) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (value @ Some(_), None) | (None, value @ Some(_)) => value,
        (None, None) => None,
    };
    current.cache_write_tokens = match (current.cache_write_tokens, incoming.cache_write_tokens) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (value @ Some(_), None) | (None, value @ Some(_)) => value,
        (None, None) => None,
    };
    if current.occurred_at_ms.is_none() {
        current.occurred_at_ms = incoming.occurred_at_ms;
    }
}

fn checked_component_sum(
    components: &[TurnUsageComponent],
    value: impl Fn(&TurnUsageComponent) -> u64,
) -> Option<u64> {
    (!components.is_empty())
        .then(|| {
            components
                .iter()
                .try_fold(0_u64, |sum, component| sum.checked_add(value(component)))
        })
        .flatten()
}

fn checked_optional_component_sum(
    components: &[TurnUsageComponent],
    value: impl Fn(&TurnUsageComponent) -> Option<u64>,
) -> Option<u64> {
    components
        .iter()
        .any(|component| value(component).is_some())
        .then(|| {
            components.iter().try_fold(0_u64, |sum, component| {
                sum.checked_add(value(component).unwrap_or_default())
            })
        })
        .flatten()
}

/// Stateful Claude stream parser. One instance must be retained for the life of
/// the CLI session so usage components can be finalized at each result event.
#[derive(Debug, Default)]
pub struct ClaudeStreamEventParser {
    usage: ClaudeTurnUsageAccumulator,
}

impl ClaudeStreamEventParser {
    pub fn parse_line(&mut self, line: &str) -> Option<ProviderEvent> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        self.usage.observe(&value);
        if value.get("type").and_then(|value| value.as_str()) == Some("result") {
            let is_error = value.get("is_error").and_then(|value| value.as_bool()) == Some(true)
                || value
                    .get("subtype")
                    .and_then(|value| value.as_str())
                    .is_some_and(|subtype| subtype.starts_with("error_"));
            return Some(ProviderEvent::TurnCompleted {
                outcome: if is_error {
                    TurnOutcome::Failed
                } else {
                    TurnOutcome::Success
                },
                usage: self.usage.finish(&value),
            });
        }
        parse_stream_json_value(&value)
    }
}

/// Executes a zero-session auth probe using `claude auth status --json`.
pub async fn probe_claude_auth(
    claude_bin: &Path,
    config_dir: Option<&Path>,
) -> Result<ClaudeAuthStatus, ProviderError> {
    let mut cmd = tokio::process::Command::new(claude_bin);
    cmd.arg("auth").arg("status").arg("--json");
    cmd.kill_on_drop(true);

    if let Some(dir) = config_dir {
        cmd.env("CLAUDE_CONFIG_DIR", dir);
    }

    #[cfg(windows)]
    {
        cmd.creation_flags(crate::supervision::CREATE_NO_WINDOW);
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| ProviderError::ProcessFailed {
            program: claude_bin.display().to_string(),
            message: e.to_string(),
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let status: ClaudeAuthStatus =
        serde_json::from_str(&stdout).map_err(|e| ProviderError::ProtocolError {
            provider: "claude".into(),
            message: format!("Failed to parse auth status JSON: {e} (stdout: {stdout})"),
        })?;

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
        if mode == "bypassPermissions" {
            args.push("--allow-dangerously-skip-permissions".to_string());
        }
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

/// Maps PandaMUX access policy to a supported Claude Code permission mode.
pub fn claude_permission_mode(access: AccessMode) -> &'static str {
    match access {
        AccessMode::ReadOnly => "plan",
        AccessMode::Ask | AccessMode::WorkspaceOnly => "manual",
        AccessMode::AutoEdit => "acceptEdits",
        AccessMode::FullAccess => "bypassPermissions",
    }
}

/// Merges PandaMUX access policy with configured Claude tool restrictions.
pub fn claude_disallowed_tools(access: AccessMode, configured: &[String]) -> Vec<String> {
    let mut disallowed = configured.to_vec();
    if access == AccessMode::ReadOnly && !disallowed.iter().any(|tool| tool == "ExitPlanMode") {
        disallowed.push("ExitPlanMode".to_string());
    }
    disallowed
}

/// Builds one Claude Code SDK streaming user message.
pub fn build_claude_user_message(input: &TurnInput) -> serde_json::Value {
    let mut prompt_text = input.text.clone();
    if !input.attachments.is_empty() {
        prompt_text.push_str(&pandamux_core::format_attachments_summary(
            &input.attachments,
        ));
    }

    serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "type": "text",
                "text": prompt_text
            }]
        }
    })
}

/// Builds one Claude Code SDK interrupt control request.
pub fn build_claude_interrupt_request(request_id: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "interrupt"}
    })
}

fn parse_control_request_value(val: &serde_json::Value) -> Option<ControlRequest> {
    if val.get("type").and_then(|value| value.as_str()) != Some("control_request") {
        return None;
    }

    let request_id = val.get("request_id")?.as_str()?.to_string();
    let request = val.get("request");
    let tool_name = request
        .and_then(|value| value.get("tool_name"))
        .or_else(|| val.get("tool_name"))?
        .as_str()?
        .to_string();
    let command = request
        .and_then(|value| value.pointer("/input/command"))
        .or_else(|| val.get("command"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let reason = request
        .and_then(|value| value.get("description"))
        .or_else(|| val.get("reason"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let input = request
        .and_then(|value| value.get("input"))
        .cloned()
        .unwrap_or_else(|| {
            command
                .as_ref()
                .map(|command| serde_json::json!({ "command": command }))
                .unwrap_or_else(|| serde_json::json!({}))
        });

    Some(ControlRequest {
        request_id,
        tool_name,
        command,
        reason,
        input,
    })
}

/// Parses NDJSON events from Claude Code stream-json output.
pub fn parse_stream_json_lines(ndjson: &str) -> Result<ParsedClaudeStream, ProviderError> {
    let mut parsed = ParsedClaudeStream::default();
    let mut usage_accumulator = ClaudeTurnUsageAccumulator::default();

    for line in ndjson.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let val: serde_json::Value =
            serde_json::from_str(line).map_err(|e| ProviderError::ProtocolError {
                provider: "claude".into(),
                message: format!("Failed to parse line as JSON: {e}"),
            })?;

        let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        usage_accumulator.observe(&val);

        match event_type {
            "system" => {
                if let Some(sid) = val.get("session_id").and_then(|v| v.as_str()) {
                    parsed.session_id = Some(sid.to_string());
                }
            }
            "control_request" => {
                let req = parse_control_request_value(&val).ok_or_else(|| {
                    ProviderError::ProtocolError {
                        provider: "claude".to_string(),
                        message: "Malformed Claude control request".to_string(),
                    }
                })?;
                parsed.control_requests.push(req);
            }
            "tool_use" => {
                let name = val.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                if name == "Task" {
                    let tool_use_id = val
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let parent_tool_use_id = val
                        .get("parent_tool_use_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let input = val.get("input").cloned().unwrap_or_default();
                    let subagent_type = input
                        .get("subagent_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let description = input
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let model = input
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    parsed.subagents.push(SubAgentSpawn {
                        tool_use_id,
                        parent_tool_use_id,
                        subagent_type,
                        description,
                        model,
                    });
                }
            }
            "assistant" => {
                if let Some(content_arr) = val.get("content").and_then(|v| v.as_array()) {
                    for item in content_arr {
                        if item.get("type").and_then(|v| v.as_str()) == Some("text")
                            && let Some(txt) = item.get("text").and_then(|v| v.as_str())
                        {
                            parsed.assistant_text.push_str(txt);
                        }
                    }
                }
            }
            "result" => {
                parsed.usage = usage_accumulator.finish(&val).map(|usage| StreamTurnUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cache_read_tokens: usage.cache_read_tokens,
                    cache_write_tokens: usage.cache_write_tokens,
                    cost_usd: usage.cost_usd,
                    components: usage.components,
                });
            }
            _ => {}
        }
    }

    Ok(parsed)
}

fn parse_tool_use_value(
    value: &serde_json::Value,
    parent_tool_use_id: Option<String>,
) -> Option<ProviderEvent> {
    let name = value.get("name")?.as_str()?;
    let id = value.get("id")?.as_str()?.to_string();
    let input = value.get("input").cloned().unwrap_or_default();

    if name == "Task" {
        let parent_tool_use_id = value
            .get("parent_tool_use_id")
            .and_then(|item| item.as_str())
            .map(str::to_string)
            .or(parent_tool_use_id);
        let agent_type = input
            .get("subagent_type")
            .and_then(|item| item.as_str())
            .unwrap_or("agent")
            .to_string();
        let description = input
            .get("description")
            .and_then(|item| item.as_str())
            .unwrap_or_default()
            .to_string();
        let model = input
            .get("model")
            .and_then(|item| item.as_str())
            .map(str::to_string);
        Some(ProviderEvent::SubAgentSpawned {
            tool_use_id: id,
            parent_tool_use_id,
            agent_type,
            description,
            model,
        })
    } else {
        Some(ProviderEvent::ToolCallStarted {
            id,
            name: name.to_string(),
            input,
        })
    }
}

fn parse_tool_result_value(value: &serde_json::Value) -> Option<ProviderEvent> {
    let id = value
        .get("tool_use_id")
        .or_else(|| value.get("id"))?
        .as_str()?
        .to_string();
    let output = value.get("content").map(|content| {
        content
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| content.to_string())
    });
    let is_error = value
        .get("is_error")
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    let error = is_error.then(|| {
        output
            .clone()
            .unwrap_or_else(|| "Claude tool execution failed".to_string())
    });
    Some(ProviderEvent::ToolCallFinished { id, output, error })
}

/// Converts a single line of Claude NDJSON output into a ProviderEvent.
pub fn parse_stream_json_line(line: &str) -> Option<ProviderEvent> {
    ClaudeStreamEventParser::default().parse_line(line)
}

fn parse_stream_json_value(val: &serde_json::Value) -> Option<ProviderEvent> {
    let event_type = val.get("type").and_then(|v| v.as_str())?;

    match event_type {
        "control_request" => {
            let request = parse_control_request_value(val)?;

            let kind = match request.tool_name.as_str() {
                "Bash" => ApprovalKind::CommandExecution,
                "Edit" | "Write" => ApprovalKind::FileEdit,
                _ => ApprovalKind::ToolCall,
            };

            Some(ProviderEvent::ApprovalRequested {
                request_id: request.request_id,
                kind,
                detail: val.clone(),
                command: request.command,
            })
        }
        "tool_use" => parse_tool_use_value(val, None),
        "tool_result" => parse_tool_result_value(val),
        "assistant" => {
            let content = val
                .pointer("/message/content")
                .or_else(|| val.get("content"));
            if let Some(content_arr) = content.and_then(|v| v.as_array()) {
                let parent_tool_use_id = val
                    .get("parent_tool_use_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
                if let Some(tool_use) = content_arr.iter().find(|item| {
                    item.get("type").and_then(|value| value.as_str()) == Some("tool_use")
                }) {
                    return parse_tool_use_value(tool_use, parent_tool_use_id);
                }
                let mut text = String::new();
                for item in content_arr {
                    if item.get("type").and_then(|v| v.as_str()) == Some("text")
                        && let Some(txt) = item.get("text").and_then(|v| v.as_str())
                    {
                        text.push_str(txt);
                    }
                }
                if !text.is_empty() {
                    return Some(ProviderEvent::TextDelta { delta: text });
                }
            }
            None
        }
        "user" => val
            .pointer("/message/content")
            .and_then(|value| value.as_array())
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("type").and_then(|value| value.as_str()) == Some("tool_result")
                })
            })
            .and_then(parse_tool_result_value),
        "content_block_delta" => {
            if let Some(delta) = val.get("delta")
                && delta.get("type").and_then(|v| v.as_str()) == Some("text_delta")
                && let Some(txt) = delta.get("text").and_then(|v| v.as_str())
            {
                return Some(ProviderEvent::TextDelta {
                    delta: txt.to_string(),
                });
            }
            None
        }
        "result" => None,
        _ => None,
    }
}

/// The Claude provider driver.
pub struct ClaudeDriver {
    pub base_data_dir: PathBuf,
}

impl ClaudeDriver {
    pub fn new(base_data_dir: PathBuf) -> Self {
        Self { base_data_dir }
    }

    fn resolve_claude_binary(&self, cfg: &ProviderInstanceConfig) -> Option<PathBuf> {
        if let Some(explicit_path) = cfg.binary_path.as_ref() {
            let path = PathBuf::from(explicit_path);
            if path.is_file() {
                return Some(path);
            }
        }
        if let Some(explicit_path) = cfg.settings.get("binaryPath").and_then(|v| v.as_str()) {
            let p = PathBuf::from(explicit_path);
            if p.is_file() {
                return Some(p);
            }
        }

        resolve_command_shim("claude").map(|r| r.program)
    }
}

impl ProviderDriver for ClaudeDriver {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            id: ProviderKind::Claude,
            display_name: "Claude Code".to_string(),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calls: true,
                supports_sub_agents: true,
                supports_system_prompt: true,
                supports_reasoning: true,
                supports_approvals: true,
                supports_rate_limits: false,
                supports_resume: true,
                supports_images: true,
                max_access_mode: AccessMode::FullAccess,
            },
            supported_models: vec![
                ModelInfo {
                    id: "claude-3-7-sonnet".to_string(),
                    display_name: "Claude 3.7 Sonnet".to_string(),
                    context_window: Some(200_000),
                    max_output_tokens: Some(64_000),
                    supports_reasoning: true,
                },
                ModelInfo {
                    id: "claude-3-5-sonnet".to_string(),
                    display_name: "Claude 3.5 Sonnet".to_string(),
                    context_window: Some(200_000),
                    max_output_tokens: Some(8_192),
                    supports_reasoning: false,
                },
                ModelInfo {
                    id: "claude-3-5-haiku".to_string(),
                    display_name: "Claude 3.5 Haiku".to_string(),
                    context_window: Some(200_000),
                    max_output_tokens: Some(8_192),
                    supports_reasoning: false,
                },
            ],
        }
    }

    fn probe<'a>(&'a self, cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, ProviderSnapshot> {
        Box::pin(async move {
            let profile_dir = resolve_instance_profile_dir(&self.base_data_dir, cfg);
            let binary = self.resolve_claude_binary(cfg);
            let checked_at_ms = provider_snapshot_now_ms();
            let expires_at_ms = provider_snapshot_expiry(checked_at_ms);

            if let Some(bin) = binary {
                match probe_claude_auth(&bin, Some(&profile_dir)).await {
                    Ok(status) => {
                        let auth = if status.logged_in {
                            ProviderAuthStatus::Authenticated {
                                account_label: status.subscription_type.clone(),
                                email: status.email,
                                subscription: status.subscription_type,
                            }
                        } else {
                            ProviderAuthStatus::Unauthenticated
                        };

                        ProviderSnapshot {
                            installed: true,
                            resolved_executable: Some(bin.to_string_lossy().to_string()),
                            version: None,
                            auth,
                            auth_confidence: ProviderAuthConfidence::Reported,
                            health: ProviderHealth::Healthy,
                            runtime_code: Some("runtime_ready".to_string()),
                            checked_at_ms,
                            expires_at_ms,
                        }
                    }
                    Err(e) => ProviderSnapshot {
                        installed: true,
                        resolved_executable: Some(bin.to_string_lossy().to_string()),
                        version: None,
                        auth: ProviderAuthStatus::Unknown,
                        auth_confidence: ProviderAuthConfidence::Unknown,
                        health: ProviderHealth::Degraded {
                            reason: e.to_string(),
                        },
                        runtime_code: Some("auth_probe_failed".to_string()),
                        checked_at_ms,
                        expires_at_ms,
                    },
                }
            } else {
                ProviderSnapshot {
                    installed: false,
                    resolved_executable: None,
                    version: None,
                    auth: ProviderAuthStatus::Unknown,
                    auth_confidence: ProviderAuthConfidence::Unknown,
                    health: ProviderHealth::Unavailable {
                        reason: "Claude binary not found on PATH".to_string(),
                    },
                    runtime_code: Some("binary_missing".to_string()),
                    checked_at_ms,
                    expires_at_ms,
                }
            }
        })
    }

    fn list_models<'a>(
        &'a self,
        _cfg: &'a ProviderInstanceConfig,
    ) -> BoxFuture<'a, Result<Vec<ModelInfo>, ProviderError>> {
        Box::pin(async move { Ok(self.metadata().supported_models) })
    }

    fn usage_limits<'a>(
        &'a self,
        _cfg: &'a ProviderInstanceConfig,
    ) -> BoxFuture<'a, Option<UsageLimits>> {
        Box::pin(async move { None })
    }

    fn start_session<'a>(
        &'a self,
        cfg: &'a ProviderInstanceConfig,
        spec: SessionSpec,
    ) -> BoxFuture<'a, Result<Box<dyn ProviderSession>, ProviderError>> {
        Box::pin(async move {
            let binary =
                self.resolve_claude_binary(cfg)
                    .ok_or_else(|| ProviderError::ProcessFailed {
                        program: "claude".to_string(),
                        message: "Executable not found in PATH".to_string(),
                    })?;

            let profile_dir = resolve_instance_profile_dir(&self.base_data_dir, cfg);
            ensure_profile_dir(&profile_dir)?;

            let perm_mode = claude_permission_mode(spec.access);

            let preamble = spec.instructions.as_ref().map(|i| i.format_preamble());
            let (allowed_tools, configured_disallowed_tools) = if let Some(tp) = &spec.tool_policy {
                (tp.allowed.clone(), tp.disallowed.clone())
            } else {
                (Vec::new(), Vec::new())
            };
            let disallowed_tools =
                claude_disallowed_tools(spec.access, &configured_disallowed_tools);

            let args = build_claude_launch_args(
                preamble.as_deref(),
                &allowed_tools,
                &disallowed_tools,
                Some(perm_mode),
                spec.resume.as_deref(),
            );

            let mut cmd = create_supervised_command(&binary);
            cmd.args(&args);
            cmd.current_dir(&spec.cwd);
            apply_profile_environment(
                &mut cmd,
                ProviderKind::Claude,
                &profile_dir,
                &cfg.env_overrides,
            );

            let mut child = SupervisedChild::spawn(cmd)?;
            let stdin = child
                .take_stdin()
                .ok_or_else(|| ProviderError::SupervisionError {
                    message: "Failed to open child stdin".to_string(),
                })?;
            let stdout = child
                .take_stdout()
                .ok_or_else(|| ProviderError::SupervisionError {
                    message: "Failed to open child stdout".to_string(),
                })?;

            let (tx, rx) = mpsc::channel(128);
            let pending_control_inputs = Arc::new(Mutex::new(HashMap::new()));
            let parser_pending_inputs = pending_control_inputs.clone();

            // Spawn background stdout parser
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                let mut stream_parser = ClaudeStreamEventParser::default();
                while let Ok(Some(line)) = reader.next_line().await {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                        && let Some(request) = parse_control_request_value(&value)
                        && let Ok(mut pending) = parser_pending_inputs.lock()
                    {
                        pending.insert(request.request_id, request.input);
                    }
                    if let Some(event) = stream_parser.parse_line(&line)
                        && crate::forward_provider_event(&tx, event).await.is_err()
                    {
                        break;
                    }
                }
            });

            Ok(Box::new(ClaudeSession {
                child,
                stdin,
                events_rx: Some(rx),
                session_token: spec.resume,
                pending_control_inputs,
            }) as Box<dyn ProviderSession>)
        })
    }
}

/// An active Claude Code session.
pub struct ClaudeSession {
    child: SupervisedChild,
    stdin: tokio::process::ChildStdin,
    events_rx: Option<mpsc::Receiver<ProviderEvent>>,
    session_token: Option<String>,
    pending_control_inputs: Arc<Mutex<HashMap<String, serde_json::Value>>>,
}

impl ProviderSession for ClaudeSession {
    fn send_turn<'a>(&'a mut self, input: TurnInput) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let mut line = serde_json::to_string(&build_claude_user_message(&input))?;
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
            let pending_input = self
                .pending_control_inputs
                .lock()
                .map_err(|_| ProviderError::ProtocolError {
                    provider: "claude".to_string(),
                    message: "Claude permission state lock is poisoned".to_string(),
                })?
                .remove(&request_id);
            let resp = match decision {
                ApprovalDecision::Approved => {
                    let input = pending_input.ok_or_else(|| ProviderError::ProtocolError {
                        provider: "claude".to_string(),
                        message: format!(
                            "Claude approval {request_id} has no matching pending tool input"
                        ),
                    })?;
                    ControlResponse::allow(&request_id, input)
                }
                ApprovalDecision::Denied | ApprovalDecision::TimedOut => {
                    ControlResponse::deny(&request_id, "User declined permission request")
                }
            };
            let mut line = serde_json::to_string(&resp)?;
            line.push('\n');
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await?;
            Ok(())
        })
    }

    fn interrupt<'a>(&'a mut self) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let request_id = uuid::Uuid::new_v4().to_string();
            let resp = build_claude_interrupt_request(&request_id);
            let mut line = serde_json::to_string(&resp)?;
            line.push('\n');
            let _ = self.stdin.write_all(line.as_bytes()).await;
            let _ = self.stdin.flush().await;
            Ok(())
        })
    }

    fn resume_token(&self) -> Option<String> {
        self.session_token.clone()
    }

    fn shutdown(mut self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let _ = self.child.kill().await;
        })
    }
}
