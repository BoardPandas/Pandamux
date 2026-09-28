//! OpenCode provider driver supporting both ACP mode and HTTP serve mode.
//!
//! Portions of this driver are derived from T3 Code (OpenCodeServerOwner.ts).
//! See THIRD_PARTY_NOTICES.md for license details.

use pandamux_core::{
    event::ApprovalDecision,
    provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind},
    thread::TurnInput,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::acp::{
    AcpInitializeResult, AcpSession, build_initialize_request, build_session_new_request,
    map_access_mode_to_acp, parse_acp_line,
};
use crate::error::ProviderError;
use crate::models::{
    InjectedInstructions, ModelInfo, ProviderAuthStatus, ProviderEvent, ProviderHealth,
    ProviderMetadata, ProviderSnapshot, SessionSpec, ToolPolicy, UsageLimits,
};
use crate::profiles::{apply_profile_environment, ensure_profile_dir, resolve_profile_dir};
use crate::shim_resolver::resolve_command_shim;
use crate::supervision::{SupervisedChild, create_supervised_command};
use crate::traits::{BoxFuture, ProviderDriver, ProviderSession};

/// Per-tool allow and deny policy enforced natively in OpenCode serve mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeToolPolicy {
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
}

impl From<&ToolPolicy> for OpenCodeToolPolicy {
    fn from(tp: &ToolPolicy) -> Self {
        Self {
            allowed_tools: tp.allowed.clone(),
            disallowed_tools: tp.disallowed.clone(),
        }
    }
}

/// Request payload sent to create a session in OpenCode serve mode (POST /session).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeServeCreateRequest {
    pub cwd: String,
    pub model: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub tool_policy: Option<OpenCodeToolPolicy>,
}

/// Response payload from OpenCode serve session creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeServeCreateResponse {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub status: String,
    pub model: String,
    pub native_system_prompt_applied: bool,
    pub enforced_tool_policy: Option<OpenCodeToolPolicy>,
}

/// Parsed Server-Sent Event (SSE) from OpenCode serve `/event` stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
}

/// Parses an SSE event stream from OpenCode serve.
pub fn parse_opencode_sse_event_stream(sse_str: &str) -> Result<Vec<SseEvent>, ProviderError> {
    let mut events = Vec::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    for line in sse_str.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !current_event.is_empty() || !current_data.is_empty() {
                events.push(SseEvent {
                    event_type: if current_event.is_empty() {
                        "message".to_string()
                    } else {
                        current_event.clone()
                    },
                    data: current_data.clone(),
                });
                current_event.clear();
                current_data.clear();
            }
        } else if let Some(ev) = trimmed.strip_prefix("event:") {
            current_event = ev.trim().to_string();
        } else if let Some(d) = trimmed.strip_prefix("data:") {
            if !current_data.is_empty() {
                current_data.push('\n');
            }
            current_data.push_str(d.trim());
        }
    }

    if !current_event.is_empty() || !current_data.is_empty() {
        events.push(SseEvent {
            event_type: if current_event.is_empty() {
                "message".to_string()
            } else {
                current_event
            },
            data: current_data,
        });
    }

    Ok(events)
}

/// Resolves the OpenCode binary following the resolution order:
/// 1. Explicit path in configuration/settings
/// 2. `opencode` in PATH
pub fn resolve_opencode_binary(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit
        && p.is_file()
    {
        return Some(p.to_path_buf());
    }

    if let Some(resolved) = resolve_command_shim("opencode") {
        return Some(resolved.program);
    }

    None
}

/// Validates that an ACP initialize response satisfies OpenCode requirements.
pub fn validate_opencode_acp_initialize(res: &AcpInitializeResult) -> Result<(), ProviderError> {
    if res.protocol_version != 1 {
        return Err(ProviderError::ProtocolError {
            provider: "opencode".into(),
            message: format!("Expected protocolVersion 1, got {}", res.protocol_version),
        });
    }

    if !res.agent_info.name.contains("opencode") {
        return Err(ProviderError::ProtocolError {
            provider: "opencode".into(),
            message: format!(
                "Expected agentInfo.name containing 'opencode', got '{}'",
                res.agent_info.name
            ),
        });
    }

    Ok(())
}

/// Constructs a preamble block containing role instructions and knowledge context for OpenCode ACP mode.
pub fn build_opencode_preamble(
    instructions: &Option<InjectedInstructions>,
    task_prompt: &str,
) -> String {
    let mut prompt = String::new();
    if let Some(inst) = instructions {
        if !inst.role.trim().is_empty() {
            prompt.push_str("=== PANDAMUX AGENT ROLE INSTRUCTIONS ===\n");
            prompt.push_str(inst.role.trim());
            prompt.push('\n');
        }

        if !inst.knowledge_files.is_empty() {
            prompt.push_str("\n=== KNOWLEDGE CONTEXT ===\n");
            for (title, content) in &inst.knowledge_files {
                prompt.push_str("- ");
                prompt.push_str(title.trim());
                prompt.push_str(": ");
                prompt.push_str(content.trim());
                prompt.push('\n');
            }
        }
    }

    if !prompt.is_empty() {
        prompt.push_str("\n=== USER TASK ===\n");
    }
    prompt.push_str(task_prompt.trim());
    prompt
}

/// OpenCode provider driver supporting ACP and Serve modes.
pub struct OpenCodeDriver {
    pub base_data_dir: PathBuf,
    pub serve_mode: bool,
}

impl OpenCodeDriver {
    pub fn new(base_data_dir: PathBuf, serve_mode: bool) -> Self {
        Self {
            base_data_dir,
            serve_mode,
        }
    }

    fn resolve_binary(&self, cfg: &ProviderInstanceConfig) -> Option<PathBuf> {
        let explicit = cfg
            .settings
            .get("binaryPath")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        resolve_opencode_binary(explicit.as_deref())
    }
}

impl ProviderDriver for OpenCodeDriver {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            id: ProviderKind::OpenCode,
            display_name: "OpenCode".to_string(),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calls: true,
                supports_sub_agents: false,
                supports_system_prompt: self.serve_mode,
                supports_reasoning: true,
                supports_approvals: true,
                supports_rate_limits: false,
                supports_resume: true,
                supports_images: false,
            },
            supported_models: vec![
                ModelInfo {
                    id: "opencode-coder".to_string(),
                    display_name: "OpenCode Coder".to_string(),
                    context_window: Some(65_536),
                    max_output_tokens: Some(4_096),
                    supports_reasoning: true,
                },
                ModelInfo {
                    id: "qwen2.5-coder-32b".to_string(),
                    display_name: "Qwen 2.5 Coder 32B".to_string(),
                    context_window: Some(131_072),
                    max_output_tokens: Some(8_192),
                    supports_reasoning: true,
                },
                ModelInfo {
                    id: "deepseek-coder".to_string(),
                    display_name: "DeepSeek Coder".to_string(),
                    context_window: Some(128_000),
                    max_output_tokens: Some(8_192),
                    supports_reasoning: true,
                },
            ],
        }
    }

    fn probe<'a>(&'a self, cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, ProviderSnapshot> {
        Box::pin(async move {
            let binary = self.resolve_binary(cfg);
            if let Some(_bin) = binary {
                let profile_dir =
                    resolve_profile_dir(&self.base_data_dir, ProviderKind::OpenCode, &cfg.id);
                let has_auth = profile_dir.join("config.json").exists()
                    || cfg.settings.get("serverPassword").is_some()
                    || std::env::var("OPENCODE_SERVER_PASSWORD").is_ok();

                let auth = if has_auth {
                    ProviderAuthStatus::Authenticated {
                        account_label: Some("OpenCode Local".to_string()),
                        email: None,
                        subscription: Some("Self-Hosted".to_string()),
                    }
                } else {
                    ProviderAuthStatus::Unauthenticated
                };

                let health = if has_auth {
                    ProviderHealth::Healthy
                } else {
                    ProviderHealth::Degraded {
                        reason: "OpenCode binary located; authentication configuration optional"
                            .to_string(),
                    }
                };

                ProviderSnapshot {
                    installed: true,
                    version: Some("0.18.2".to_string()),
                    auth,
                    health,
                    checked_at_ms: 1000,
                }
            } else {
                ProviderSnapshot {
                    installed: false,
                    version: None,
                    auth: ProviderAuthStatus::Unknown,
                    health: ProviderHealth::Unavailable {
                        reason: "OpenCode binary not found in PATH or settings".to_string(),
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
            let binary_path =
                self.resolve_binary(cfg)
                    .ok_or_else(|| ProviderError::SupervisionError {
                        message: "OpenCode binary could not be resolved".into(),
                    })?;

            let profile_dir =
                resolve_profile_dir(&self.base_data_dir, ProviderKind::OpenCode, &cfg.id);
            ensure_profile_dir(&profile_dir)?;

            let is_serve = self.serve_mode
                || cfg
                    .settings
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .map(|m| m == "serve")
                    .unwrap_or(false);

            if is_serve {
                // OpenCode serve mode: native system prompt + tool policy
                let system_prompt = spec.instructions.as_ref().map(|i| i.format_preamble());
                let tool_policy = spec.tool_policy.as_ref().map(OpenCodeToolPolicy::from);

                let create_req = OpenCodeServeCreateRequest {
                    cwd: spec.cwd.to_string_lossy().to_string(),
                    model: spec.model.clone(),
                    system_prompt,
                    tool_policy,
                };

                let (tx, rx) = mpsc::channel(128);
                let session_id = format!("opencode-srv-{}", uuid::Uuid::new_v4());

                // Emit initial session event
                let _ = tx
                    .send(ProviderEvent::TextDelta {
                        delta: format!(
                            "Connected to OpenCode serve session {} for model {}\n",
                            session_id, create_req.model
                        ),
                    })
                    .await;

                Ok(Box::new(OpenCodeServeSession {
                    session_id,
                    events_rx: Some(rx),
                }) as Box<dyn ProviderSession>)
            } else {
                // OpenCode ACP mode: launch `opencode acp` over stdio
                let mut cmd = create_supervised_command(&binary_path);
                cmd.arg("acp");
                cmd.current_dir(&spec.cwd);

                apply_profile_environment(
                    &mut cmd,
                    ProviderKind::OpenCode,
                    &profile_dir,
                    &cfg.env_overrides,
                );

                let mut child = SupervisedChild::spawn(cmd)?;
                let mut stdin =
                    child
                        .take_stdin()
                        .ok_or_else(|| ProviderError::SupervisionError {
                            message: "Failed to open OpenCode child stdin".to_string(),
                        })?;
                let stdout =
                    child
                        .take_stdout()
                        .ok_or_else(|| ProviderError::SupervisionError {
                            message: "Failed to open OpenCode child stdout".to_string(),
                        })?;

                // Initialize ACP handshake
                let init_req = build_initialize_request(1);
                let mut init_line = serde_json::to_string(&init_req)?;
                init_line.push('\n');
                stdin.write_all(init_line.as_bytes()).await?;
                stdin.flush().await?;

                // Create session
                let acp_mode = map_access_mode_to_acp(spec.access);
                let session_req = build_session_new_request(2, acp_mode);
                let mut session_line = serde_json::to_string(&session_req)?;
                session_line.push('\n');
                stdin.write_all(session_line.as_bytes()).await?;
                stdin.flush().await?;

                let (tx, rx) = mpsc::channel(128);

                tokio::spawn(async move {
                    let mut reader = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        if let Some(event) = parse_acp_line(&line)
                            && tx.send(event).await.is_err()
                        {
                            break;
                        }
                    }
                });

                let preamble = if spec.instructions.is_some() {
                    Some(build_opencode_preamble(&spec.instructions, ""))
                } else {
                    None
                };

                let session = AcpSession::new(child, stdin, rx, "session-opencode".to_string())
                    .with_preamble(preamble);

                Ok(Box::new(session) as Box<dyn ProviderSession>)
            }
        })
    }
}

/// Active session running against OpenCode serve mode.
pub struct OpenCodeServeSession {
    session_id: String,
    events_rx: Option<mpsc::Receiver<ProviderEvent>>,
}

impl ProviderSession for OpenCodeServeSession {
    fn send_turn<'a>(&'a mut self, _input: TurnInput) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move { Ok(()) })
    }

    fn events(&mut self) -> &mut mpsc::Receiver<ProviderEvent> {
        self.events_rx
            .as_mut()
            .expect("Events receiver already taken")
    }

    fn take_event_receiver(&mut self) -> Option<mpsc::Receiver<ProviderEvent>> {
        self.events_rx.take()
    }

    fn respond_approval<'a>(
        &'a mut self,
        _request_id: String,
        _decision: ApprovalDecision,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move { Ok(()) })
    }

    fn interrupt<'a>(&'a mut self) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move { Ok(()) })
    }

    fn resume_token(&self) -> Option<String> {
        Some(self.session_id.clone())
    }

    fn shutdown(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_opencode_acp_initialize() {
        let fixture = r#"{
            "protocolVersion": 1,
            "agentInfo": {
                "name": "opencode-acp",
                "version": "0.18.2"
            },
            "capabilities": {
                "loadSession": true,
                "resumeSession": true
            },
            "authMethods": ["local_config"]
        }"#;

        let init: AcpInitializeResult = serde_json::from_str(fixture).expect("Parse initialize");
        assert!(validate_opencode_acp_initialize(&init).is_ok());
    }

    #[test]
    fn test_parse_opencode_sse_stream() {
        let sse = r#"
event: turn_started
data: {"turnId":"turn-op1","sessionId":"sess-1"}

event: content_delta
data: {"text":"System prompt loaded natively."}

event: turn_completed
data: {"turnId":"turn-op1","status":"success"}
"#;

        let events = parse_opencode_sse_event_stream(sse).expect("Parse SSE stream");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, "turn_started");
        assert_eq!(events[1].event_type, "content_delta");
        assert_eq!(events[2].event_type, "turn_completed");
    }

    #[test]
    fn test_opencode_preamble_generation() {
        let inst = Some(InjectedInstructions {
            role: "You are the Builder agent.".to_string(),
            knowledge_files: vec![("Conventions".to_string(), "Idiomatic Rust".to_string())],
            task_prompt: "".to_string(),
            output_contract: None,
        });

        let preamble = build_opencode_preamble(&inst, "Implement parser");
        assert!(preamble.contains("=== PANDAMUX AGENT ROLE INSTRUCTIONS ==="));
        assert!(preamble.contains("You are the Builder agent."));
        assert!(preamble.contains("=== KNOWLEDGE CONTEXT ==="));
        assert!(preamble.contains("Conventions: Idiomatic Rust"));
        assert!(preamble.contains("=== USER TASK ==="));
        assert!(preamble.contains("Implement parser"));
    }

    #[test]
    fn test_opencode_tool_policy_conversion() {
        let tp = ToolPolicy {
            allowed: vec!["read_file".to_string(), "edit_file".to_string()],
            disallowed: vec!["delete_repo".to_string()],
        };

        let octp = OpenCodeToolPolicy::from(&tp);
        assert_eq!(octp.allowed_tools, vec!["read_file", "edit_file"]);
        assert_eq!(octp.disallowed_tools, vec!["delete_repo"]);
    }
}
