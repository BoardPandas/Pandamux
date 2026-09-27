use pandamux_core::{
    event::{ApprovalDecision, ApprovalKind, TurnOutcome},
    provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind},
    thread::{AccessMode, TurnInput, TurnUsage},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::error::ProviderError;
use crate::models::{
    ModelInfo, ProviderAuthStatus, ProviderEvent, ProviderHealth, ProviderMetadata,
    ProviderSnapshot, SessionSpec, UsageLimits,
};
use crate::profiles::{apply_profile_environment, ensure_profile_dir, resolve_profile_dir};
use crate::shim_resolver::resolve_command_shim;
use crate::supervision::{SupervisedChild, create_supervised_command};
use crate::traits::{BoxFuture, ProviderDriver, ProviderSession};

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
}

/// Outbound control response answering a `control_request`.
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
    pub cost_usd: Option<f64>,
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

/// Executes a zero-session auth probe using `claude auth status --json`.
pub async fn probe_claude_auth(
    claude_bin: &Path,
    config_dir: Option<&Path>,
) -> Result<ClaudeAuthStatus, ProviderError> {
    let mut cmd = tokio::process::Command::new(claude_bin);
    cmd.arg("auth").arg("status").arg("--json");

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
pub fn parse_stream_json_lines(ndjson: &str) -> Result<ParsedClaudeStream, ProviderError> {
    let mut parsed = ParsedClaudeStream::default();

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
                if let Some(usage_val) = val.get("usage") {
                    let input_tokens = usage_val
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or_default();
                    let output_tokens = usage_val
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or_default();
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

/// Converts a single line of Claude NDJSON output into a ProviderEvent.
pub fn parse_stream_json_line(line: &str) -> Option<ProviderEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let val: serde_json::Value = serde_json::from_str(line).ok()?;
    let event_type = val.get("type").and_then(|v| v.as_str())?;

    match event_type {
        "control_request" => {
            let req_id = val.get("request_id")?.as_str()?.to_string();
            let tool_name = val
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let command = val
                .get("command")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let kind = match tool_name {
                "Bash" => ApprovalKind::CommandExecution,
                "Edit" | "Write" => ApprovalKind::FileEdit,
                _ => ApprovalKind::ToolCall,
            };

            Some(ProviderEvent::ApprovalRequested {
                request_id: req_id,
                kind,
                detail: val.clone(),
                command,
            })
        }
        "tool_use" => {
            let name = val.get("name")?.as_str()?;
            let id = val.get("id")?.as_str()?.to_string();
            let input = val.get("input").cloned().unwrap_or_default();

            if name == "Task" {
                let parent_tool_use_id = val
                    .get("parent_tool_use_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let subagent_type = input
                    .get("subagent_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent")
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

                Some(ProviderEvent::SubAgentSpawned {
                    tool_use_id: id,
                    parent_tool_use_id,
                    agent_type: subagent_type,
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
        "assistant" => {
            if let Some(content_arr) = val.get("content").and_then(|v| v.as_array()) {
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
        "result" => {
            let usage = val.get("usage").map(|u| TurnUsage {
                input_tokens: u
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_default(),
                output_tokens: u
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_default(),
                cost_usd: u.get("cost_usd").and_then(|v| v.as_f64()),
                ..Default::default()
            });

            Some(ProviderEvent::TurnCompleted {
                outcome: TurnOutcome::Success,
                usage,
            })
        }
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
            let profile_dir =
                resolve_profile_dir(&self.base_data_dir, ProviderKind::Claude, &cfg.id);
            let binary = self.resolve_claude_binary(cfg);

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
                            version: None,
                            auth,
                            health: ProviderHealth::Healthy,
                            checked_at_ms: 1000,
                        }
                    }
                    Err(e) => ProviderSnapshot {
                        installed: true,
                        version: None,
                        auth: ProviderAuthStatus::Unknown,
                        health: ProviderHealth::Degraded {
                            reason: e.to_string(),
                        },
                        checked_at_ms: 1000,
                    },
                }
            } else {
                ProviderSnapshot {
                    installed: false,
                    version: None,
                    auth: ProviderAuthStatus::Unknown,
                    health: ProviderHealth::Unavailable {
                        reason: "Claude binary not found on PATH".to_string(),
                    },
                    checked_at_ms: 1000,
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

            let profile_dir =
                resolve_profile_dir(&self.base_data_dir, ProviderKind::Claude, &cfg.id);
            ensure_profile_dir(&profile_dir)?;

            let perm_mode = match spec.access {
                AccessMode::ReadOnly => "ask",
                AccessMode::Ask | AccessMode::WorkspaceOnly => "ask",
                AccessMode::AutoEdit => "auto",
                AccessMode::FullAccess => "auto",
            };

            let preamble = spec.instructions.as_ref().map(|i| i.format_preamble());
            let (allowed_tools, disallowed_tools) = if let Some(tp) = &spec.tool_policy {
                (tp.allowed.clone(), tp.disallowed.clone())
            } else {
                (Vec::new(), Vec::new())
            };

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

            // Spawn background stdout parser
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if let Some(event) = parse_stream_json_line(&line)
                        && tx.send(event).await.is_err()
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
}

impl ProviderSession for ClaudeSession {
    fn send_turn<'a>(&'a mut self, input: TurnInput) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let msg = serde_json::json!({
                "type": "user_input",
                "text": input.text,
            });
            let mut line = serde_json::to_string(&msg)?;
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
            let resp = match decision {
                ApprovalDecision::Approved => ControlResponse::allow(&request_id),
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
            let resp = serde_json::json!({ "type": "interrupt" });
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
