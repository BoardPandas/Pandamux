use pandamux_core::{
    CapacityConfidence, UsageUnknownField,
    event::{ApprovalDecision, ApprovalKind, TurnOutcome},
    provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind},
    thread::{AccessMode, TurnInput, TurnUsage, TurnUsageProvenance},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::ChildStdout;
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::error::ProviderError;
use crate::models::{
    ModelInfo, ProviderAuthConfidence, ProviderAuthStatus, ProviderEvent, ProviderHealth,
    ProviderMetadata, ProviderSnapshot, RateLimitEntry, SessionSpec, UsageLimits,
    provider_snapshot_expiry,
};
use crate::profiles::{
    apply_profile_environment, ensure_profile_dir, resolve_instance_profile_dir,
};
use crate::shim_resolver::resolve_command_shim;
use crate::supervision::{SupervisedChild, create_supervised_command};
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
    #[serde(rename = "on-request")]
    OnRequest,
    #[serde(rename = "untrusted")]
    Untrusted,
}

/// Maps PandaMUX's access ceiling to the closest Codex sandbox and approval pair.
///
/// `Ask` must use Codex's fail-closed policy. `on-request` lets Codex decide that a
/// command is safe enough to run without consulting the user, which violates the
/// PandaMUX contract for an explicitly approval-gated thread.
pub fn codex_session_policies(access: AccessMode) -> (CodexSandboxPolicy, CodexApprovalPolicy) {
    match access {
        AccessMode::ReadOnly => (CodexSandboxPolicy::ReadOnly, CodexApprovalPolicy::Never),
        AccessMode::Ask => (
            CodexSandboxPolicy::WorkspaceWrite,
            CodexApprovalPolicy::Untrusted,
        ),
        AccessMode::WorkspaceOnly => (
            CodexSandboxPolicy::WorkspaceWrite,
            CodexApprovalPolicy::OnRequest,
        ),
        AccessMode::AutoEdit => (
            CodexSandboxPolicy::WorkspaceWrite,
            CodexApprovalPolicy::Never,
        ),
        AccessMode::FullAccess => (
            CodexSandboxPolicy::DangerFullAccess,
            CodexApprovalPolicy::Never,
        ),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub id: u64,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jsonrpc: Option<String>,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone)]
pub enum CodexNotificationEvent {
    TextDelta(String),
    ApprovalRequest {
        request_id: String,
        command: String,
        reason: String,
        kind: ApprovalKind,
    },
    CommandOutput {
        stdout: String,
        exit_code: i32,
    },
    TurnCompleted {
        outcome: TurnOutcome,
    },
    Other(String),
}

/// Builds a `thread/start` request using the stable Codex App Server contract.
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
    if !model.trim().is_empty() && model != "default" {
        params_map.insert("model".into(), Value::String(model.into()));
    }
    params_map.insert(
        "sandbox".into(),
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
        id,
        method: "thread/start".into(),
        params: Value::Object(params_map),
    }
}

/// Builds a `turn/start` request with prompt text.
pub fn build_turn_start_request(id: u64, thread_id: &str, text: &str) -> JsonRpcRequest {
    build_turn_start_request_with_attachments(id, thread_id, text, &[])
}

/// Builds a `turn/start` request with prompt text and optional attachments.
pub fn build_turn_start_request_with_attachments(
    id: u64,
    thread_id: &str,
    text: &str,
    attachments: &[pandamux_core::AttachmentRecord],
) -> JsonRpcRequest {
    let mut effective_text = text.to_string();
    if !attachments.is_empty() {
        effective_text.push_str(&pandamux_core::format_attachments_summary(attachments));
    }

    let mut params_map = serde_json::Map::new();
    params_map.insert("threadId".into(), Value::String(thread_id.into()));
    let mut input = vec![serde_json::json!({
        "type": "text",
        "text": effective_text
    })];
    for attachment in attachments
        .iter()
        .filter(|attachment| attachment.mime_type.starts_with("image/"))
    {
        input.push(serde_json::json!({
            "type": "localImage",
            "path": attachment.file_path
        }));
    }
    params_map.insert("input".into(), Value::Array(input));

    JsonRpcRequest {
        id,
        method: "turn/start".into(),
        params: Value::Object(params_map),
    }
}

/// Builds the result object for a server-initiated approval request.
pub fn build_approval_response(approved: bool) -> Value {
    serde_json::json!({
        "decision": if approved { "accept" } else { "decline" }
    })
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
            "item/agentMessage/delta" => {
                let delta = params
                    .get("delta")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                Ok(CodexNotificationEvent::TextDelta(delta))
            }
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                let request_id = val
                    .get("id")
                    .and_then(|id| {
                        id.as_str()
                            .map(str::to_string)
                            .or_else(|| id.as_u64().map(|id| id.to_string()))
                    })
                    .unwrap_or_default();
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

                let kind = if method == "item/fileChange/requestApproval" {
                    ApprovalKind::FileEdit
                } else if params
                    .get("networkApprovalContext")
                    .is_some_and(|value| !value.is_null())
                {
                    ApprovalKind::NetworkAccess
                } else {
                    ApprovalKind::CommandExecution
                };

                Ok(CodexNotificationEvent::ApprovalRequest {
                    request_id,
                    command,
                    reason,
                    kind,
                })
            }
            "item/commandExecution/outputDelta" => {
                let stdout = params
                    .get("delta")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let exit_code = params.get("exitCode").and_then(Value::as_i64).unwrap_or(0) as i32;

                Ok(CodexNotificationEvent::CommandOutput { stdout, exit_code })
            }
            "turn/completed" => {
                let outcome = match params
                    .get("turn")
                    .and_then(|turn| turn.get("status"))
                    .and_then(Value::as_str)
                {
                    Some("completed") => TurnOutcome::Success,
                    Some("interrupted") => TurnOutcome::Interrupted,
                    _ => TurnOutcome::Failed,
                };

                Ok(CodexNotificationEvent::TurnCompleted { outcome })
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
    let result = resp
        .result
        .as_ref()
        .ok_or_else(|| ProviderError::ProtocolError {
            provider: "codex".into(),
            message: "No result object in rate limits response".into(),
        })?;

    let limits = result
        .get("rateLimits")
        .ok_or_else(|| ProviderError::ProtocolError {
            provider: "codex".into(),
            message: "Missing 'rateLimits' in result".into(),
        })?;

    parse_rate_limits_value(limits)
}

/// Parses a Codex rate-limit snapshot with exact integer fraction conversion.
pub fn parse_rate_limits_value(limits: &Value) -> Result<Vec<RateLimitEntry>, ProviderError> {
    let mut entries = Vec::new();
    if let Some(legacy_limits) = limits.as_array() {
        for item in legacy_limits {
            entries.push(parse_rate_limit_window(
                item.get("limitName")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                item,
            )?);
        }
    } else if limits.is_object() {
        let name = limits
            .get("limitName")
            .and_then(Value::as_str)
            .unwrap_or("codex");
        for window_name in ["primary", "secondary"] {
            if let Some(window) = limits.get(window_name).filter(|window| window.is_object()) {
                entries.push(parse_rate_limit_window(
                    &format!("{name} {window_name}"),
                    window,
                )?);
            }
        }
    } else {
        return Err(codex_protocol_error(
            "Codex rateLimits value was neither a snapshot nor an array",
        ));
    }

    Ok(entries)
}

fn parse_rate_limit_window(
    limit_name: &str,
    window: &Value,
) -> Result<RateLimitEntry, ProviderError> {
    let used_millionths = window
        .get("usedPercent")
        .map(parse_percent_millionths)
        .transpose()?;
    let remaining_millionths = used_millionths.map(|used| 1_000_000_u32.saturating_sub(used));
    Ok(RateLimitEntry {
        limit_name: limit_name.to_string(),
        used_millionths,
        remaining_millionths,
        window_duration_mins: window
            .get("windowDurationMins")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .try_into()
            .unwrap_or(u32::MAX),
        resets_at_ms: window.get("resetsAt").and_then(parse_epoch_ms),
        hard_limit: remaining_millionths == Some(0),
    })
}

fn parse_percent_millionths(value: &Value) -> Result<u32, ProviderError> {
    let raw = match value {
        Value::Number(number) => number.to_string(),
        Value::String(value) => value.trim().to_string(),
        _ => return Err(codex_protocol_error("Codex usedPercent was not numeric")),
    };
    if raw.starts_with('-') || raw.starts_with('+') || raw.is_empty() {
        return Err(codex_protocol_error(
            "Codex usedPercent was outside 0 through 100",
        ));
    }
    let (whole, fraction) = raw.split_once('.').unwrap_or((&raw, ""));
    if !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 9
    {
        return Err(codex_protocol_error(
            "Codex usedPercent had invalid precision",
        ));
    }
    let whole = whole
        .parse::<u128>()
        .map_err(|_| codex_protocol_error("Codex usedPercent overflowed"))?;
    let scale = 10_u128
        .checked_pow(u32::try_from(fraction.len()).unwrap_or(u32::MAX))
        .ok_or_else(|| codex_protocol_error("Codex usedPercent precision overflowed"))?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u128>()
            .map_err(|_| codex_protocol_error("Codex usedPercent overflowed"))?
    };
    let numerator = whole
        .checked_mul(scale)
        .and_then(|value| value.checked_add(fraction))
        .and_then(|value| value.checked_mul(1_000_000))
        .ok_or_else(|| codex_protocol_error("Codex usedPercent overflowed"))?;
    let denominator = scale
        .checked_mul(100)
        .ok_or_else(|| codex_protocol_error("Codex usedPercent overflowed"))?;
    let rounded = numerator
        .checked_add(denominator / 2)
        .ok_or_else(|| codex_protocol_error("Codex usedPercent overflowed"))?
        / denominator;
    if rounded > 1_000_000 {
        return Err(codex_protocol_error(
            "Codex usedPercent was outside 0 through 100",
        ));
    }
    u32::try_from(rounded).map_err(|_| codex_protocol_error("Codex usedPercent overflowed"))
}

fn parse_epoch_ms(value: &Value) -> Option<u64> {
    let raw = value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))?;
    if raw >= 1_000_000_000_000 {
        Some(raw)
    } else {
        raw.checked_mul(1000)
    }
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

        resolve_command_shim("codex").map(|r| r.program)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn codex_protocol_error(message: impl Into<String>) -> ProviderError {
    ProviderError::ProtocolError {
        provider: "codex".to_string(),
        message: message.into(),
    }
}

async fn write_codex_message<T: Serialize>(
    stdin: &mut tokio::process::ChildStdin,
    message: &T,
) -> Result<(), ProviderError> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_codex_response(
    lines: &mut Lines<BufReader<ChildStdout>>,
    expected_id: u64,
) -> Result<Value, ProviderError> {
    tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(line) = lines.next_line().await? {
            let message: Value = serde_json::from_str(&line)?;
            if message.get("id").and_then(Value::as_u64) != Some(expected_id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                let detail = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex App Server returned an error");
                return Err(codex_protocol_error(detail));
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| codex_protocol_error("Codex response omitted its result"));
        }
        Err(codex_protocol_error(
            "Codex App Server closed before replying",
        ))
    })
    .await
    .map_err(|_| ProviderError::Timeout { timeout_secs: 30 })?
}

fn initialize_request(id: u64) -> JsonRpcRequest {
    JsonRpcRequest {
        id,
        method: "initialize".to_string(),
        params: serde_json::json!({
            "clientInfo": {
                "name": "pandamux",
                "title": "PandaMUX",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    }
}

fn initialized_notification() -> JsonRpcNotification {
    JsonRpcNotification {
        method: "initialized".to_string(),
        params: serde_json::json!({}),
    }
}

fn build_thread_resume_request(
    id: u64,
    thread_id: &str,
    cwd: &str,
    model: &str,
    developer_instructions: Option<&str>,
    sandbox_policy: CodexSandboxPolicy,
    approval_policy: CodexApprovalPolicy,
) -> JsonRpcRequest {
    let mut request = build_thread_start_request(
        id,
        cwd,
        model,
        developer_instructions,
        sandbox_policy,
        approval_policy,
    );
    request.method = "thread/resume".to_string();
    if let Some(params) = request.params.as_object_mut() {
        params.insert("threadId".to_string(), Value::String(thread_id.to_string()));
    }
    request
}

async fn spawn_codex_app_server(
    binary: &Path,
    profile_dir: &std::path::Path,
    cwd: &std::path::Path,
    env_overrides: &std::collections::BTreeMap<String, String>,
) -> Result<
    (
        SupervisedChild,
        tokio::process::ChildStdin,
        Lines<BufReader<ChildStdout>>,
        tokio::task::JoinHandle<()>,
    ),
    ProviderError,
> {
    let mut cmd = create_supervised_command(binary);
    cmd.arg("app-server");
    cmd.current_dir(cwd);
    apply_profile_environment(&mut cmd, ProviderKind::Codex, profile_dir, env_overrides);

    let mut child = SupervisedChild::spawn(cmd)?;
    let stdin = child
        .take_stdin()
        .ok_or_else(|| ProviderError::SupervisionError {
            message: "Failed to open Codex child stdin".to_string(),
        })?;
    let stdout = child
        .take_stdout()
        .ok_or_else(|| ProviderError::SupervisionError {
            message: "Failed to open Codex child stdout".to_string(),
        })?;
    let stderr = child
        .take_stderr()
        .ok_or_else(|| ProviderError::SupervisionError {
            message: "Failed to open Codex child stderr".to_string(),
        })?;

    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while matches!(lines.next_line().await, Ok(Some(_))) {}
    });

    Ok((child, stdin, BufReader::new(stdout).lines(), stderr_task))
}

async fn initialize_codex_app_server(
    stdin: &mut tokio::process::ChildStdin,
    lines: &mut Lines<BufReader<ChildStdout>>,
) -> Result<(), ProviderError> {
    write_codex_message(stdin, &initialize_request(1)).await?;
    let _ = read_codex_response(lines, 1).await?;
    write_codex_message(stdin, &initialized_notification()).await
}

async fn probe_codex_auth(
    binary: &std::path::Path,
    profile_dir: &std::path::Path,
    env_overrides: &std::collections::BTreeMap<String, String>,
) -> Result<(bool, Option<String>, Option<String>), ProviderError> {
    let mut auth = tokio::process::Command::new(binary);
    auth.arg("login").arg("status");
    auth.kill_on_drop(true);
    apply_profile_environment(&mut auth, ProviderKind::Codex, profile_dir, env_overrides);
    #[cfg(windows)]
    auth.creation_flags(crate::supervision::CREATE_NO_WINDOW);
    let auth_output = auth
        .output()
        .await
        .map_err(|error| ProviderError::ProcessFailed {
            program: binary.display().to_string(),
            message: error.to_string(),
        })?;
    let status_text = format!(
        "{}{}",
        String::from_utf8_lossy(&auth_output.stdout),
        String::from_utf8_lossy(&auth_output.stderr)
    );
    let logged_in = auth_output.status.success();
    if !logged_in && !status_text.to_ascii_lowercase().contains("not logged in") {
        return Err(codex_protocol_error(format!(
            "Codex login status failed with exit code {}",
            auth_output.status.code().unwrap_or(-1)
        )));
    }
    let method = logged_in.then(|| {
        if status_text.to_ascii_lowercase().contains("chatgpt") {
            "ChatGPT".to_string()
        } else if status_text.to_ascii_lowercase().contains("api key") {
            "API key".to_string()
        } else {
            "Codex account".to_string()
        }
    });

    let version = probe_codex_version(binary, profile_dir, env_overrides).await;

    Ok((logged_in, method, version))
}

async fn probe_codex_version(
    binary: &std::path::Path,
    profile_dir: &std::path::Path,
    env_overrides: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    let mut version = tokio::process::Command::new(binary);
    version.arg("--version");
    version.kill_on_drop(true);
    apply_profile_environment(
        &mut version,
        ProviderKind::Codex,
        profile_dir,
        env_overrides,
    );
    #[cfg(windows)]
    version.creation_flags(crate::supervision::CREATE_NO_WINDOW);
    version.output().await.ok().and_then(|output| {
        output.status.success().then(|| {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .trim_start_matches("codex-cli ")
                .to_string()
        })
    })
}

async fn list_codex_models(
    binary: &Path,
    profile_dir: &std::path::Path,
    env_overrides: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let cwd = std::env::current_dir().map_err(ProviderError::Io)?;
    let (mut child, mut stdin, mut lines, stderr_task) =
        spawn_codex_app_server(binary, profile_dir, &cwd, env_overrides).await?;
    let result: Result<Vec<ModelInfo>, ProviderError> = async {
        initialize_codex_app_server(&mut stdin, &mut lines).await?;
        let request = JsonRpcRequest {
            id: 2,
            method: "model/list".to_string(),
            params: serde_json::json!({ "limit": 100 }),
        };
        write_codex_message(&mut stdin, &request).await?;
        let response = read_codex_response(&mut lines, 2).await?;
        let models = response
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| codex_protocol_error("Codex model list omitted data"))?;
        Ok(models
            .iter()
            .filter(|model| {
                !model
                    .get("hidden")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .filter_map(|model| {
                let id = model
                    .get("model")
                    .or_else(|| model.get("id"))?
                    .as_str()?
                    .to_string();
                let display_name = model
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string();
                Some(ModelInfo {
                    id,
                    display_name,
                    context_window: None,
                    max_output_tokens: None,
                    supports_reasoning: model
                        .get("supportedReasoningEfforts")
                        .and_then(Value::as_array)
                        .is_some_and(|efforts| !efforts.is_empty()),
                })
            })
            .collect())
    }
    .await;
    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), stderr_task).await;
    result
}

async fn read_codex_usage_limits(
    binary: &Path,
    profile_dir: &std::path::Path,
    env_overrides: &std::collections::BTreeMap<String, String>,
) -> Result<UsageLimits, ProviderError> {
    let cwd = std::env::current_dir().map_err(ProviderError::Io)?;
    let (mut child, mut stdin, mut lines, stderr_task) =
        spawn_codex_app_server(binary, profile_dir, &cwd, env_overrides).await?;
    let result: Result<Vec<RateLimitEntry>, ProviderError> = async {
        initialize_codex_app_server(&mut stdin, &mut lines).await?;
        let request = JsonRpcRequest {
            id: 2,
            method: "account/rateLimits/read".to_string(),
            params: serde_json::json!({}),
        };
        write_codex_message(&mut stdin, &request).await?;
        let response = JsonRpcResponse {
            jsonrpc: None,
            id: 2,
            result: Some(read_codex_response(&mut lines, 2).await?),
            error: None,
        };
        parse_rate_limits_response(&response)
    }
    .await;
    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), stderr_task).await;
    let limits = result?;
    Ok(UsageLimits {
        source: "codex_account_rate_limits".to_string(),
        confidence: CapacityConfidence::Reported,
        provider_version: probe_codex_version(binary, profile_dir, env_overrides).await,
        as_of_ms: now_ms(),
        limits,
    })
}

type PendingResponse = oneshot::Sender<Result<Value, String>>;

#[derive(Debug)]
struct PendingApproval {
    wire_id: Value,
}

fn turn_usage_from_notification(params: &Value) -> Option<TurnUsage> {
    let last = params.get("tokenUsage")?.get("last")?;
    let input_tokens = last.get("inputTokens").and_then(Value::as_u64);
    let output_tokens = last.get("outputTokens").and_then(Value::as_u64);
    let mut unknown_fields = BTreeSet::new();
    if input_tokens.is_none() {
        unknown_fields.insert(UsageUnknownField::InputTokens);
    }
    if output_tokens.is_none() {
        unknown_fields.insert(UsageUnknownField::OutputTokens);
    }
    Some(TurnUsage {
        input_tokens: input_tokens.unwrap_or(0),
        output_tokens: output_tokens.unwrap_or(0),
        cache_read_tokens: last.get("cachedInputTokens").and_then(Value::as_u64),
        cache_write_tokens: last.get("cacheWriteInputTokens").and_then(Value::as_u64),
        cost_usd: None,
        unknown_fields,
        ..Default::default()
    })
}

fn turn_outcome(params: &Value) -> TurnOutcome {
    match params
        .get("turn")
        .and_then(|turn| turn.get("status"))
        .and_then(Value::as_str)
    {
        Some("completed") => TurnOutcome::Success,
        Some("interrupted") => TurnOutcome::Interrupted,
        _ => TurnOutcome::Failed,
    }
}

struct CodexTurnUsageState {
    current_turn_id: Arc<Mutex<Option<String>>>,
    latest_usage: Arc<Mutex<Option<TurnUsage>>>,
    provider_session_id: String,
    model: String,
}

async fn run_codex_reader(
    mut lines: Lines<BufReader<ChildStdout>>,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    events: mpsc::Sender<ProviderEvent>,
    pending_responses: Arc<Mutex<HashMap<u64, PendingResponse>>>,
    pending_approvals: Arc<Mutex<HashMap<String, PendingApproval>>>,
    usage_state: CodexTurnUsageState,
) {
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            let _ = events
                .send(ProviderEvent::Error {
                    message: "Codex App Server emitted invalid JSON".to_string(),
                    recoverable: false,
                })
                .await;
            continue;
        };

        let method = message.get("method").and_then(Value::as_str);
        if method.is_none()
            && let Some(id) = message.get("id").and_then(Value::as_u64)
        {
            if let Some(waiter) = pending_responses.lock().await.remove(&id) {
                let result = if let Some(error) = message.get("error") {
                    Err(error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex App Server request failed")
                        .to_string())
                } else {
                    message
                        .get("result")
                        .cloned()
                        .ok_or_else(|| "Codex response omitted its result".to_string())
                };
                let _ = waiter.send(result);
            }
            continue;
        }

        let Some(method) = method else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        if message.get("id").is_some() {
            if matches!(
                method,
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
            ) {
                let request_id = uuid::Uuid::new_v4().to_string();
                let kind = if method == "item/fileChange/requestApproval" {
                    ApprovalKind::FileEdit
                } else if params
                    .get("networkApprovalContext")
                    .is_some_and(|value| !value.is_null())
                {
                    ApprovalKind::NetworkAccess
                } else {
                    ApprovalKind::CommandExecution
                };
                let command = params
                    .get("command")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                pending_approvals.lock().await.insert(
                    request_id.clone(),
                    PendingApproval {
                        wire_id: message.get("id").cloned().unwrap_or(Value::Null),
                    },
                );
                if crate::forward_provider_event(
                    &events,
                    ProviderEvent::ApprovalRequested {
                        request_id,
                        kind,
                        detail: params,
                        command,
                    },
                )
                .await
                .is_err()
                {
                    break;
                }
            } else {
                let response = serde_json::json!({
                    "id": message.get("id").cloned().unwrap_or(Value::Null),
                    "error": {
                        "code": -32601,
                        "message": format!("PandaMUX does not support Codex server request '{method}'")
                    }
                });
                let mut writer = stdin.lock().await;
                let _ = write_codex_message(&mut writer, &response).await;
                let _ = events
                    .send(ProviderEvent::Error {
                        message: format!("Unsupported Codex server request: {method}"),
                        recoverable: false,
                    })
                    .await;
            }
            continue;
        }

        let provider_event = match method {
            "item/agentMessage/delta" => {
                params
                    .get("delta")
                    .and_then(Value::as_str)
                    .map(|delta| ProviderEvent::TextDelta {
                        delta: delta.to_string(),
                    })
            }
            "item/reasoning/summaryTextDelta" => {
                params.get("delta").and_then(Value::as_str).map(|delta| {
                    ProviderEvent::ReasoningDelta {
                        delta: delta.to_string(),
                    }
                })
            }
            "item/started" => params.get("item").and_then(|item| {
                let item_type = item.get("type")?.as_str()?;
                if !matches!(item_type, "commandExecution" | "fileChange" | "mcpToolCall") {
                    return None;
                }
                Some(ProviderEvent::ToolCallStarted {
                    id: item.get("id")?.as_str()?.to_string(),
                    name: item_type.to_string(),
                    input: item.clone(),
                })
            }),
            "item/completed" => params.get("item").and_then(|item| {
                let item_type = item.get("type")?.as_str()?;
                if !matches!(item_type, "commandExecution" | "fileChange" | "mcpToolCall") {
                    return None;
                }
                let error = item
                    .get("error")
                    .filter(|value| !value.is_null())
                    .map(Value::to_string);
                let output = item
                    .get("aggregatedOutput")
                    .or_else(|| item.get("result"))
                    .filter(|value| !value.is_null())
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string())
                    });
                Some(ProviderEvent::ToolCallFinished {
                    id: item.get("id")?.as_str()?.to_string(),
                    output,
                    error,
                })
            }),
            "thread/tokenUsage/updated" => {
                *usage_state.latest_usage.lock().await = turn_usage_from_notification(&params);
                None
            }
            "account/rateLimits/updated" => Some(ProviderEvent::RateLimitObserved {
                details: params.get("rateLimits").cloned().unwrap_or(params),
            }),
            "turn/started" => {
                if let Some(turn_id) = params
                    .get("turn")
                    .and_then(|turn| turn.get("id"))
                    .and_then(Value::as_str)
                {
                    *usage_state.current_turn_id.lock().await = Some(turn_id.to_string());
                }
                None
            }
            "configWarning" | "warning" => Some(ProviderEvent::Notice {
                level: "warning".to_string(),
                message: params
                    .get("summary")
                    .or_else(|| params.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Codex reported a warning")
                    .to_string(),
            }),
            "error" => Some(ProviderEvent::Error {
                message: params
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Codex turn failed")
                    .to_string(),
                recoverable: false,
            }),
            "turn/completed" => {
                let completed_turn_id = usage_state.current_turn_id.lock().await.take();
                pending_approvals.lock().await.clear();
                let mut usage = usage_state.latest_usage.lock().await.take();
                if let Some(usage) = usage.as_mut() {
                    usage.provenance = Some(TurnUsageProvenance {
                        provider_native_session_id: Some(usage_state.provider_session_id.clone()),
                        provider_native_turn_id: completed_turn_id,
                        occurred_at_ms: Some(now_ms()),
                        model: Some(usage_state.model.clone()),
                    });
                }
                Some(ProviderEvent::TurnCompleted {
                    outcome: turn_outcome(&params),
                    usage,
                })
            }
            _ => None,
        };

        if let Some(event) = provider_event
            && crate::forward_provider_event(&events, event).await.is_err()
        {
            break;
        }
    }

    let mut pending = pending_responses.lock().await;
    for (_, waiter) in pending.drain() {
        let _ = waiter.send(Err("Codex App Server closed before replying".to_string()));
    }
    drop(pending);
    let _ = events
        .send(ProviderEvent::Error {
            message: "Codex App Server closed its output stream".to_string(),
            recoverable: false,
        })
        .await;
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
                supports_images: true,
                max_access_mode: AccessMode::FullAccess,
            },
            supported_models: vec![ModelInfo {
                id: "default".to_string(),
                display_name: "Codex default".to_string(),
                context_window: None,
                max_output_tokens: None,
                supports_reasoning: true,
            }],
        }
    }

    fn probe<'a>(&'a self, cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, ProviderSnapshot> {
        Box::pin(async move {
            let Some(binary) = self.resolve_codex_binary(cfg) else {
                let checked_at_ms = now_ms();
                return ProviderSnapshot {
                    installed: false,
                    resolved_executable: None,
                    version: None,
                    auth: ProviderAuthStatus::Unknown,
                    auth_confidence: ProviderAuthConfidence::Unknown,
                    health: ProviderHealth::Unavailable {
                        reason: "Codex binary not found on PATH".to_string(),
                    },
                    runtime_code: Some("binary_missing".to_string()),
                    checked_at_ms,
                    expires_at_ms: provider_snapshot_expiry(checked_at_ms),
                };
            };
            let profile_dir = resolve_instance_profile_dir(&self.base_data_dir, cfg);
            match probe_codex_auth(&binary, &profile_dir, &cfg.env_overrides).await {
                Ok((logged_in, method, version)) => {
                    let checked_at_ms = now_ms();
                    ProviderSnapshot {
                        installed: true,
                        resolved_executable: Some(binary.to_string_lossy().to_string()),
                        version,
                        auth: if logged_in {
                            ProviderAuthStatus::Authenticated {
                                account_label: method,
                                email: None,
                                subscription: None,
                            }
                        } else {
                            ProviderAuthStatus::Unauthenticated
                        },
                        auth_confidence: ProviderAuthConfidence::Reported,
                        health: ProviderHealth::Healthy,
                        runtime_code: Some("runtime_ready".to_string()),
                        checked_at_ms,
                        expires_at_ms: provider_snapshot_expiry(checked_at_ms),
                    }
                }
                Err(error) => {
                    let checked_at_ms = now_ms();
                    ProviderSnapshot {
                        installed: true,
                        resolved_executable: Some(binary.to_string_lossy().to_string()),
                        version: None,
                        auth: ProviderAuthStatus::Unknown,
                        auth_confidence: ProviderAuthConfidence::Unknown,
                        health: ProviderHealth::Degraded {
                            reason: error.to_string(),
                        },
                        runtime_code: Some("auth_probe_failed".to_string()),
                        checked_at_ms,
                        expires_at_ms: provider_snapshot_expiry(checked_at_ms),
                    }
                }
            }
        })
    }

    fn list_models<'a>(
        &'a self,
        cfg: &'a ProviderInstanceConfig,
    ) -> BoxFuture<'a, Result<Vec<ModelInfo>, ProviderError>> {
        Box::pin(async move {
            let binary =
                self.resolve_codex_binary(cfg)
                    .ok_or_else(|| ProviderError::ProcessFailed {
                        program: "codex".to_string(),
                        message: "Executable not found in PATH".to_string(),
                    })?;
            let profile_dir = resolve_instance_profile_dir(&self.base_data_dir, cfg);
            list_codex_models(&binary, &profile_dir, &cfg.env_overrides).await
        })
    }

    fn usage_limits<'a>(
        &'a self,
        cfg: &'a ProviderInstanceConfig,
    ) -> BoxFuture<'a, Option<UsageLimits>> {
        Box::pin(async move {
            let binary = self.resolve_codex_binary(cfg)?;
            let profile_dir = resolve_instance_profile_dir(&self.base_data_dir, cfg);
            read_codex_usage_limits(&binary, &profile_dir, &cfg.env_overrides)
                .await
                .ok()
        })
    }

    fn start_session<'a>(
        &'a self,
        cfg: &'a ProviderInstanceConfig,
        spec: SessionSpec,
    ) -> BoxFuture<'a, Result<Box<dyn ProviderSession>, ProviderError>> {
        Box::pin(async move {
            let binary =
                self.resolve_codex_binary(cfg)
                    .ok_or_else(|| ProviderError::ProcessFailed {
                        program: "codex".to_string(),
                        message: "Executable not found in PATH".to_string(),
                    })?;

            let profile_dir = resolve_instance_profile_dir(&self.base_data_dir, cfg);
            ensure_profile_dir(&profile_dir)?;

            let (child, mut stdin, mut lines, stderr_task) =
                spawn_codex_app_server(&binary, &profile_dir, &spec.cwd, &cfg.env_overrides)
                    .await?;
            initialize_codex_app_server(&mut stdin, &mut lines).await?;

            let (sandbox, approval_policy) = codex_session_policies(spec.access);

            let preamble = spec.instructions.as_ref().map(|i| i.format_preamble());
            let start_req = if let Some(resume) = spec.resume.as_deref() {
                build_thread_resume_request(
                    2,
                    resume,
                    &spec.cwd.to_string_lossy(),
                    &spec.model,
                    preamble.as_deref(),
                    sandbox,
                    approval_policy,
                )
            } else {
                build_thread_start_request(
                    2,
                    &spec.cwd.to_string_lossy(),
                    &spec.model,
                    preamble.as_deref(),
                    sandbox,
                    approval_policy,
                )
            };
            write_codex_message(&mut stdin, &start_req).await?;
            let started = read_codex_response(&mut lines, 2).await?;
            let thread_id = started
                .get("thread")
                .and_then(|thread| thread.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| codex_protocol_error("Codex thread response omitted thread.id"))?
                .to_string();

            let (tx, rx) = mpsc::channel(128);
            let stdin = Arc::new(Mutex::new(stdin));
            let pending_responses = Arc::new(Mutex::new(HashMap::new()));
            let pending_approvals = Arc::new(Mutex::new(HashMap::new()));
            let current_turn_id = Arc::new(Mutex::new(None));
            let latest_usage = Arc::new(Mutex::new(None));
            let reader_task = tokio::spawn(run_codex_reader(
                lines,
                stdin.clone(),
                tx,
                pending_responses.clone(),
                pending_approvals.clone(),
                CodexTurnUsageState {
                    current_turn_id: current_turn_id.clone(),
                    latest_usage,
                    provider_session_id: thread_id.clone(),
                    model: spec.model.clone(),
                },
            ));

            Ok(Box::new(CodexSession {
                child,
                stdin,
                events_rx: Some(rx),
                next_id: Arc::new(AtomicU64::new(3)),
                thread_id,
                current_turn_id,
                pending_responses,
                pending_approvals,
                reader_task: Some(reader_task),
                stderr_task: Some(stderr_task),
            }) as Box<dyn ProviderSession>)
        })
    }
}

/// An active Codex app-server session.
pub struct CodexSession {
    child: SupervisedChild,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    events_rx: Option<mpsc::Receiver<ProviderEvent>>,
    next_id: Arc<AtomicU64>,
    thread_id: String,
    current_turn_id: Arc<Mutex<Option<String>>>,
    pending_responses: Arc<Mutex<HashMap<u64, PendingResponse>>>,
    pending_approvals: Arc<Mutex<HashMap<String, PendingApproval>>>,
    reader_task: Option<tokio::task::JoinHandle<()>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for CodexSession {
    fn drop(&mut self) {
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
    }
}

impl CodexSession {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ProviderError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        self.pending_responses.lock().await.insert(id, sender);
        let request = JsonRpcRequest {
            id,
            method: method.to_string(),
            params,
        };
        let write_result = {
            let mut stdin = self.stdin.lock().await;
            write_codex_message(&mut stdin, &request).await
        };
        if let Err(error) = write_result {
            self.pending_responses.lock().await.remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(Duration::from_secs(30), receiver).await {
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err(message))) => Err(codex_protocol_error(message)),
            Ok(Err(_)) => Err(codex_protocol_error(
                "Codex response waiter closed unexpectedly",
            )),
            Err(_) => {
                self.pending_responses.lock().await.remove(&id);
                Err(ProviderError::Timeout { timeout_secs: 30 })
            }
        }
    }
}

impl ProviderSession for CodexSession {
    fn send_turn<'a>(&'a mut self, input: TurnInput) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let mut request = build_turn_start_request_with_attachments(
                0,
                &self.thread_id,
                &input.text,
                &input.attachments,
            );
            let params = request
                .params
                .as_object_mut()
                .ok_or_else(|| codex_protocol_error("Codex turn params are not an object"))?;
            if let Some(model) = input.model.filter(|model| model != "default") {
                params.insert("model".to_string(), Value::String(model));
            }
            if let Some(effort) = input.effort {
                params.insert("effort".to_string(), Value::String(effort));
            }
            let result = self.request("turn/start", request.params).await?;
            let turn_id = result
                .get("turn")
                .and_then(|turn| turn.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| codex_protocol_error("Codex turn response omitted turn.id"))?
                .to_string();
            *self.current_turn_id.lock().await = Some(turn_id);
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
            let pending = self
                .pending_approvals
                .lock()
                .await
                .remove(&request_id)
                .ok_or_else(|| codex_protocol_error("Codex approval is no longer pending"))?;
            let decision = match decision {
                ApprovalDecision::Approved => "accept",
                ApprovalDecision::Denied => "decline",
                ApprovalDecision::TimedOut => "cancel",
            };
            let response = serde_json::json!({
                "id": pending.wire_id,
                "result": { "decision": decision }
            });
            let mut stdin = self.stdin.lock().await;
            write_codex_message(&mut stdin, &response).await?;
            Ok(())
        })
    }

    fn interrupt<'a>(&'a mut self) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let turn_id = self
                .current_turn_id
                .lock()
                .await
                .clone()
                .ok_or_else(|| codex_protocol_error("Codex has no active turn to interrupt"))?;
            let _ = self
                .request(
                    "turn/interrupt",
                    serde_json::json!({
                        "threadId": self.thread_id,
                        "turnId": turn_id
                    }),
                )
                .await?;
            Ok(())
        })
    }

    fn resume_token(&self) -> Option<String> {
        Some(self.thread_id.clone())
    }

    fn shutdown(mut self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let _ = self.child.kill().await;
            if let Some(task) = self.reader_task.take() {
                let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
            }
            if let Some(task) = self.stderr_task.take() {
                let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
            }
        })
    }
}
