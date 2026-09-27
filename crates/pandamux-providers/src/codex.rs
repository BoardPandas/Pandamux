use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use pandamux_core::{
    event::{ApprovalDecision, ApprovalKind, TurnOutcome},
    provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind},
    thread::{AccessMode, TurnInput, TurnUsage},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::error::ProviderError;
use crate::models::{
    ModelInfo, ProviderAuthStatus, ProviderEvent, ProviderHealth, ProviderMetadata,
    ProviderSnapshot, RateLimitEntry, SessionSpec, UsageLimits,
};
use crate::profiles::{apply_profile_environment, ensure_profile_dir, resolve_profile_dir};
use crate::shim_resolver::resolve_command_shim;
use crate::supervision::{create_supervised_command, SupervisedChild};
use crate::traits::{BoxFuture, ProviderDriver, ProviderSession};

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

#[derive(Debug, Clone)]
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

/// Builds a `turn/start` request with prompt text.
pub fn build_turn_start_request(id: u64, thread_id: &str, text: &str) -> JsonRpcRequest {
    let mut params_map = serde_json::Map::new();
    params_map.insert("threadId".into(), Value::String(thread_id.into()));
    params_map.insert(
        "input".into(),
        serde_json::json!({
            "type": "text",
            "content": text
        }),
    );

    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id,
        method: "turn/start".into(),
        params: Value::Object(params_map),
    }
}

/// Builds an approval response for Codex.
pub fn build_approval_response(id: u64, approval_id: &str, approved: bool) -> JsonRpcRequest {
    let mut params_map = serde_json::Map::new();
    params_map.insert("approvalId".into(), Value::String(approval_id.into()));
    params_map.insert(
        "decision".into(),
        Value::String(if approved { "allow" } else { "deny" }.into()),
    );

    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id,
        method: "turn/approvalResponse".into(),
        params: Value::Object(params_map),
    }
}

/// Parses a line from Codex JSON-RPC output stream.
pub fn parse_codex_notification(line: &str) -> Result<CodexNotificationEvent, ProviderError> {
    let val: Value = serde_json::from_str(line).map_err(|e| ProviderError::ProtocolError {
        provider: "codex".into(),
        message: format!("Invalid JSON line: {e}"),
    })?;

    if let Some(method) = val.get("method").and_then(|v| v.as_str()) {
        let params = val.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "turn/textDelta" => {
                let delta = params
                    .get("delta")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                Ok(CodexNotificationEvent::TextDelta(delta))
            }
            "turn/approvalRequest" => {
                let approval_id = params
                    .get("approvalId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let command = params
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let reason = params
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                Ok(CodexNotificationEvent::ApprovalRequest {
                    approval_id,
                    command,
                    reason,
                })
            }
            "turn/commandOutput" => {
                let stdout = params
                    .get("stdout")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let exit_code = params
                    .get("exitCode")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;

                Ok(CodexNotificationEvent::CommandOutput { stdout, exit_code })
            }
            "turn/completed" => {
                let total_tokens = params
                    .get("usage")
                    .and_then(|u| u.get("totalTokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                Ok(CodexNotificationEvent::TurnCompleted { total_tokens })
            }
            _ => Ok(CodexNotificationEvent::Other(method.to_string())),
        }
    } else {
        Ok(CodexNotificationEvent::Other("response".to_string()))
    }
}

/// Parses rate limits returned by `account/rateLimits/read`.
pub fn parse_rate_limits_response(
    resp: &JsonRpcResponse,
) -> Result<Vec<RateLimitEntry>, ProviderError> {
    let result = resp.result.as_ref().ok_or_else(|| ProviderError::ProtocolError {
        provider: "codex".into(),
        message: "No result object in rate limits response".into(),
    })?;

    let limits_arr = result
        .get("rateLimits")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ProviderError::ProtocolError {
            provider: "codex".into(),
            message: "Missing 'rateLimits' array in result".into(),
        })?;

    let mut entries = Vec::new();
    for item in limits_arr {
        let limit_name = item
            .get("limitName")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let used_percent = item
            .get("usedPercent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let window_duration_mins = item
            .get("windowDurationMins")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let resets_at = item
            .get("resetsAt")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        entries.push(RateLimitEntry {
            limit_name,
            used_percent,
            window_duration_mins,
            resets_at,
        });
    }

    Ok(entries)
}

/// The Codex provider driver.
pub struct CodexDriver {
    pub base_data_dir: PathBuf,
}

impl CodexDriver {
    pub fn new(base_data_dir: PathBuf) -> Self {
        Self { base_data_dir }
    }

    fn resolve_codex_binary(&self, cfg: &ProviderInstanceConfig) -> Option<PathBuf> {
        if let Some(explicit_path) = cfg.settings.get("binaryPath").and_then(|v| v.as_str()) {
            let p = PathBuf::from(explicit_path);
            if p.is_file() {
                return Some(p);
            }
        }

        resolve_command_shim("codex").map(|r| r.program)
    }
}

impl ProviderDriver for CodexDriver {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            id: ProviderKind::Codex,
            display_name: "Codex".to_string(),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calls: true,
                supports_sub_agents: false,
                supports_system_prompt: true,
                supports_reasoning: true,
                supports_approvals: true,
                supports_rate_limits: true,
                supports_resume: true,
                supports_images: false,
            },
            supported_models: vec![
                ModelInfo {
                    id: "o3-mini".to_string(),
                    display_name: "o3-mini".to_string(),
                    context_window: Some(200_000),
                    max_output_tokens: Some(100_000),
                    supports_reasoning: true,
                },
                ModelInfo {
                    id: "o1".to_string(),
                    display_name: "o1".to_string(),
                    context_window: Some(200_000),
                    max_output_tokens: Some(100_000),
                    supports_reasoning: true,
                },
            ],
        }
    }

    fn probe<'a>(&'a self, cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, ProviderSnapshot> {
        Box::pin(async move {
            let binary = self.resolve_codex_binary(cfg);
            if let Some(_bin) = binary {
                let profile_dir = resolve_profile_dir(&self.base_data_dir, ProviderKind::Codex, &cfg.id);
                // Check if profile directory contains auth credentials
                let auth_file = profile_dir.join("auth.json");
                let auth = if auth_file.exists() {
                    ProviderAuthStatus::Authenticated {
                        account_label: Some("Codex Account".to_string()),
                        email: None,
                        subscription: None,
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
            } else {
                ProviderSnapshot {
                    installed: false,
                    version: None,
                    auth: ProviderAuthStatus::Unknown,
                    health: ProviderHealth::Unavailable {
                        reason: "Codex binary not found on PATH".to_string(),
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

    fn usage_limits<'a>(&'a self, _cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, Option<UsageLimits>> {
        Box::pin(async move { None })
    }

    fn start_session<'a>(
        &'a self,
        cfg: &'a ProviderInstanceConfig,
        spec: SessionSpec,
    ) -> BoxFuture<'a, Result<Box<dyn ProviderSession>, ProviderError>> {
        Box::pin(async move {
            let binary = self.resolve_codex_binary(cfg).ok_or_else(|| {
                ProviderError::ProcessFailed {
                    program: "codex".to_string(),
                    message: "Executable not found in PATH".to_string(),
                }
            })?;

            let profile_dir = resolve_profile_dir(&self.base_data_dir, ProviderKind::Codex, &cfg.id);
            ensure_profile_dir(&profile_dir)?;

            let mut cmd = create_supervised_command(&binary);
            cmd.arg("app-server");
            cmd.current_dir(&spec.cwd);
            apply_profile_environment(&mut cmd, ProviderKind::Codex, &profile_dir, &cfg.env_overrides);

            let mut child = SupervisedChild::spawn(cmd)?;
            let mut stdin = child.take_stdin().ok_or_else(|| ProviderError::SupervisionError {
                message: "Failed to open child stdin".to_string(),
            })?;
            let stdout = child.take_stdout().ok_or_else(|| ProviderError::SupervisionError {
                message: "Failed to open child stdout".to_string(),
            })?;

            let sandbox = match spec.access {
                AccessMode::ReadOnly => CodexSandboxPolicy::ReadOnly,
                AccessMode::Ask | AccessMode::WorkspaceOnly | AccessMode::AutoEdit => {
                    CodexSandboxPolicy::WorkspaceWrite
                }
                AccessMode::FullAccess => CodexSandboxPolicy::DangerFullAccess,
            };

            let approval_policy = match spec.access {
                AccessMode::ReadOnly | AccessMode::Ask | AccessMode::WorkspaceOnly => {
                    CodexApprovalPolicy::OnWrite
                }
                AccessMode::AutoEdit | AccessMode::FullAccess => CodexApprovalPolicy::Never,
            };

            let preamble = spec.instructions.as_ref().map(|i| i.format_preamble());
            let start_req = build_thread_start_request(
                1,
                &spec.cwd.to_string_lossy(),
                &spec.model,
                preamble.as_deref(),
                sandbox,
                approval_policy,
            );

            let mut start_line = serde_json::to_string(&start_req)?;
            start_line.push('\n');
            stdin.write_all(start_line.as_bytes()).await?;
            stdin.flush().await?;

            let (tx, rx) = mpsc::channel(128);

            // Background stdout parser
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if let Ok(event) = parse_codex_notification(&line) {
                        let provider_event = match event {
                            CodexNotificationEvent::TextDelta(delta) => {
                                Some(ProviderEvent::TextDelta { delta })
                            }
                            CodexNotificationEvent::ApprovalRequest {
                                approval_id,
                                command,
                                reason,
                            } => Some(ProviderEvent::ApprovalRequested {
                                request_id: approval_id,
                                kind: ApprovalKind::CommandExecution,
                                detail: serde_json::json!({
                                    "command": command,
                                    "reason": reason,
                                }),
                                command: Some(command),
                            }),
                            CodexNotificationEvent::TurnCompleted { total_tokens } => {
                                Some(ProviderEvent::TurnCompleted {
                                    outcome: TurnOutcome::Success,
                                    usage: Some(TurnUsage {
                                        input_tokens: total_tokens,
                                        output_tokens: 0,
                                        ..Default::default()
                                    }),
                                })
                            }
                            _ => None,
                        };

                        if let Some(pe) = provider_event {
                            if tx.send(pe).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });

            Ok(Box::new(CodexSession {
                child,
                stdin,
                events_rx: Some(rx),
                next_id: Arc::new(AtomicU64::new(2)),
                thread_id: "thread-codex".to_string(),
            }) as Box<dyn ProviderSession>)
        })
    }
}

/// An active Codex app-server session.
pub struct CodexSession {
    child: SupervisedChild,
    stdin: tokio::process::ChildStdin,
    events_rx: Option<mpsc::Receiver<ProviderEvent>>,
    next_id: Arc<AtomicU64>,
    thread_id: String,
}

impl ProviderSession for CodexSession {
    fn send_turn<'a>(&'a mut self, input: TurnInput) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let id = self.next_id.fetch_add(1, Ordering::SeqCst);
            let req = build_turn_start_request(id, &self.thread_id, &input.text);
            let mut line = serde_json::to_string(&req)?;
            line.push('\n');
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await?;
            Ok(())
        })
    }

    fn events(&mut self) -> &mut mpsc::Receiver<ProviderEvent> {
        self.events_rx.as_mut().expect("events receiver already taken")
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
            let id = self.next_id.fetch_add(1, Ordering::SeqCst);
            let approved = matches!(decision, ApprovalDecision::Approved);
            let req = build_approval_response(id, &request_id, approved);
            let mut line = serde_json::to_string(&req)?;
            line.push('\n');
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await?;
            Ok(())
        })
    }

    fn interrupt<'a>(&'a mut self) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let id = self.next_id.fetch_add(1, Ordering::SeqCst);
            let req = JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id,
                method: "turn/cancel".into(),
                params: serde_json::json!({ "threadId": self.thread_id }),
            };
            let mut line = serde_json::to_string(&req)?;
            line.push('\n');
            let _ = self.stdin.write_all(line.as_bytes()).await;
            let _ = self.stdin.flush().await;
            Ok(())
        })
    }

    fn resume_token(&self) -> Option<String> {
        Some(self.thread_id.clone())
    }

    fn shutdown(mut self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let _ = self.child.kill().await;
        })
    }
}
