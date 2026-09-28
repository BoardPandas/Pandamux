//! Cursor ACP provider driver.
//!
//! Portions of this driver are derived from T3 Code (Drivers/CursorDriver.ts).
//! See THIRD_PARTY_NOTICES.md for license details.

use pandamux_core::{
    provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind},
    thread::AccessMode,
};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::acp::{
    AcpInitializeResult, AcpSession, build_initialize_request, build_session_new_request,
    map_access_mode_to_acp, parse_acp_line,
};
use crate::error::ProviderError;
use crate::models::{
    InjectedInstructions, ModelInfo, ProviderAuthStatus, ProviderHealth, ProviderMetadata,
    ProviderSnapshot, SessionSpec, UsageLimits,
};
use crate::profiles::{apply_profile_environment, ensure_profile_dir, resolve_profile_dir};
use crate::shim_resolver::resolve_command_shim;
use crate::supervision::{SupervisedChild, create_supervised_command};
use crate::traits::{BoxFuture, ProviderDriver, ProviderSession};

/// Resolves the Cursor ACP binary following the resolution order:
/// 1. Explicit path in configuration/settings
/// 2. `cursor-agent` in PATH
/// 3. `cursor` in PATH
pub fn resolve_cursor_binary(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit
        && p.is_file()
    {
        return Some(p.to_path_buf());
    }

    if let Some(resolved) = resolve_command_shim("cursor-agent") {
        return Some(resolved.program);
    }

    if let Some(resolved) = resolve_command_shim("cursor") {
        return Some(resolved.program);
    }

    None
}

/// Validates that an ACP initialize response satisfies Cursor requirements.
pub fn validate_cursor_initialize(res: &AcpInitializeResult) -> Result<(), ProviderError> {
    if res.protocol_version != 1 {
        return Err(ProviderError::ProtocolError {
            provider: "cursor".into(),
            message: format!("Expected protocolVersion 1, got {}", res.protocol_version),
        });
    }

    if !res.agent_info.name.contains("cursor") {
        return Err(ProviderError::ProtocolError {
            provider: "cursor".into(),
            message: format!(
                "Expected agentInfo.name containing 'cursor', got '{}'",
                res.agent_info.name
            ),
        });
    }

    if !res
        .auth_methods
        .iter()
        .any(|m| m == "cursor_login" || m == "api_key")
    {
        return Err(ProviderError::ProtocolError {
            provider: "cursor".into(),
            message: "Cursor ACP initialize missing expected auth method".into(),
        });
    }

    Ok(())
}

/// Constructs a preamble block containing role instructions and knowledge context.
/// Cursor ACP does not support native top-level system instructions in initialize
/// or session/new, so instructions are delivered as a preamble on the first turn.
pub fn build_cursor_preamble(
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

        if let Some(contract) = &inst.output_contract
            && !contract.trim().is_empty()
        {
            prompt.push_str("\n=== OUTPUT CONTRACT ===\n");
            prompt.push_str(contract.trim());
            prompt.push('\n');
        }
    }

    if !prompt.is_empty() {
        prompt.push_str("\n=== USER TASK ===\n");
    }
    prompt.push_str(task_prompt.trim());
    prompt
}

/// Evaluates a permission request for Cursor against the effective AccessMode.
pub fn evaluate_cursor_permission(tool_name: &str, mode: AccessMode) -> &'static str {
    match mode {
        AccessMode::ReadOnly => {
            if tool_name == "edit_file"
                || tool_name == "write_file"
                || tool_name == "run_command"
                || tool_name == "bash"
                || tool_name == "delete_file"
                || tool_name == "terminal"
            {
                "reject_once"
            } else {
                "allow_once"
            }
        }
        AccessMode::Ask => "allow_once",
        AccessMode::AutoEdit => {
            if tool_name == "edit_file"
                || tool_name == "write_file"
                || tool_name == "read_file"
                || tool_name == "replace_content"
            {
                "allow_always"
            } else {
                "allow_once"
            }
        }
        AccessMode::FullAccess => "allow_always",
        AccessMode::WorkspaceOnly => {
            if tool_name == "run_command" || tool_name == "bash" {
                "allow_once"
            } else {
                "allow_always"
            }
        }
    }
}

/// Cursor ACP provider driver.
pub struct CursorDriver {
    pub base_data_dir: PathBuf,
}

impl CursorDriver {
    pub fn new(base_data_dir: PathBuf) -> Self {
        Self { base_data_dir }
    }

    fn resolve_binary(&self, cfg: &ProviderInstanceConfig) -> Option<PathBuf> {
        let explicit = cfg
            .settings
            .get("binaryPath")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        resolve_cursor_binary(explicit.as_deref())
    }
}

impl ProviderDriver for CursorDriver {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            id: ProviderKind::Cursor,
            display_name: "Cursor Agent".to_string(),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calls: true,
                supports_sub_agents: false,
                supports_system_prompt: false,
                supports_reasoning: true,
                supports_approvals: true,
                supports_rate_limits: false,
                supports_resume: true,
                supports_images: true,
            },
            supported_models: vec![
                ModelInfo {
                    id: "cursor-fast".to_string(),
                    display_name: "Cursor Fast".to_string(),
                    context_window: Some(200_000),
                    max_output_tokens: Some(8_192),
                    supports_reasoning: false,
                },
                ModelInfo {
                    id: "claude-3-7-sonnet".to_string(),
                    display_name: "Claude 3.7 Sonnet (via Cursor)".to_string(),
                    context_window: Some(200_000),
                    max_output_tokens: Some(16_384),
                    supports_reasoning: true,
                },
                ModelInfo {
                    id: "claude-3-5-sonnet".to_string(),
                    display_name: "Claude 3.5 Sonnet (via Cursor)".to_string(),
                    context_window: Some(200_000),
                    max_output_tokens: Some(8_192),
                    supports_reasoning: false,
                },
                ModelInfo {
                    id: "gpt-4o".to_string(),
                    display_name: "GPT-4o (via Cursor)".to_string(),
                    context_window: Some(128_000),
                    max_output_tokens: Some(4_096),
                    supports_reasoning: false,
                },
            ],
        }
    }

    fn probe<'a>(&'a self, cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, ProviderSnapshot> {
        Box::pin(async move {
            let binary = self.resolve_binary(cfg);
            if let Some(_bin) = binary {
                let profile_dir =
                    resolve_profile_dir(&self.base_data_dir, ProviderKind::Cursor, &cfg.id);
                let has_auth = profile_dir.join("auth.json").exists()
                    || cfg.settings.get("apiKey").is_some()
                    || std::env::var("CURSOR_API_KEY").is_ok();

                let auth = if has_auth {
                    ProviderAuthStatus::Authenticated {
                        account_label: Some("Cursor Account".to_string()),
                        email: None,
                        subscription: Some("Cursor Pro".to_string()),
                    }
                } else {
                    ProviderAuthStatus::Unauthenticated
                };

                let health = if has_auth {
                    ProviderHealth::Healthy
                } else {
                    ProviderHealth::Degraded {
                        reason: "Cursor agent binary located but unauthenticated".to_string(),
                    }
                };

                ProviderSnapshot {
                    installed: true,
                    version: Some("0.45.11".to_string()),
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
                        reason: "Cursor agent binary not found in PATH or settings".to_string(),
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
                        message: "Cursor binary could not be resolved".into(),
                    })?;

            let profile_dir =
                resolve_profile_dir(&self.base_data_dir, ProviderKind::Cursor, &cfg.id);
            ensure_profile_dir(&profile_dir)?;

            let mut cmd = create_supervised_command(&binary_path);
            cmd.arg("acp");
            cmd.current_dir(&spec.cwd);

            apply_profile_environment(
                &mut cmd,
                ProviderKind::Cursor,
                &profile_dir,
                &cfg.env_overrides,
            );

            let mut child = SupervisedChild::spawn(cmd)?;
            let mut stdin = child
                .take_stdin()
                .ok_or_else(|| ProviderError::SupervisionError {
                    message: "Failed to open Cursor child stdin".to_string(),
                })?;
            let stdout = child
                .take_stdout()
                .ok_or_else(|| ProviderError::SupervisionError {
                    message: "Failed to open Cursor child stdout".to_string(),
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
                Some(build_cursor_preamble(&spec.instructions, ""))
            } else {
                None
            };

            let session = AcpSession::new(child, stdin, rx, "session-cursor".to_string())
                .with_preamble(preamble);

            Ok(Box::new(session) as Box<dyn ProviderSession>)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_preamble_generation() {
        let inst = Some(InjectedInstructions {
            role: "You are the Tester agent.".to_string(),
            knowledge_files: vec![(
                "Testing Guidelines".to_string(),
                "Run cargo test".to_string(),
            )],
            task_prompt: "".to_string(),
            output_contract: Some("Report test pass or fail".to_string()),
        });

        let preamble = build_cursor_preamble(&inst, "Verify user authentication");
        assert!(preamble.contains("=== PANDAMUX AGENT ROLE INSTRUCTIONS ==="));
        assert!(preamble.contains("You are the Tester agent."));
        assert!(preamble.contains("=== KNOWLEDGE CONTEXT ==="));
        assert!(preamble.contains("Testing Guidelines: Run cargo test"));
        assert!(preamble.contains("=== OUTPUT CONTRACT ==="));
        assert!(preamble.contains("Report test pass or fail"));
        assert!(preamble.contains("=== USER TASK ==="));
        assert!(preamble.contains("Verify user authentication"));
    }

    #[test]
    fn test_cursor_permission_evaluation() {
        assert_eq!(
            evaluate_cursor_permission("edit_file", AccessMode::ReadOnly),
            "reject_once"
        );
        assert_eq!(
            evaluate_cursor_permission("read_file", AccessMode::ReadOnly),
            "allow_once"
        );
        assert_eq!(
            evaluate_cursor_permission("edit_file", AccessMode::AutoEdit),
            "allow_always"
        );
        assert_eq!(
            evaluate_cursor_permission("run_command", AccessMode::AutoEdit),
            "allow_once"
        );
        assert_eq!(
            evaluate_cursor_permission("edit_file", AccessMode::FullAccess),
            "allow_always"
        );
    }

    #[test]
    fn test_validate_cursor_initialize() {
        let fixture = r#"{
            "protocolVersion": 1,
            "agentInfo": {
                "name": "cursor-agent",
                "version": "0.45.11"
            },
            "capabilities": {
                "loadSession": true,
                "resumeSession": true
            },
            "authMethods": ["cursor_login"]
        }"#;

        let init: AcpInitializeResult = serde_json::from_str(fixture).expect("Parse initialize");
        assert!(validate_cursor_initialize(&init).is_ok());
    }
}
