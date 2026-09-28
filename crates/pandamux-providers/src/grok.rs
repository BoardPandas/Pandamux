//! xAI Grok Build CLI ACP provider driver.
//!
//! Portions of this driver are derived from T3 Code (Layers/GrokAdapter.ts, Layers/grokUsageLimits.ts).
//! See THIRD_PARTY_NOTICES.md for license details.

use pandamux_core::provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind};
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
    InjectedInstructions, ModelInfo, ProviderAuthStatus, ProviderHealth, ProviderMetadata,
    ProviderSnapshot, RateLimitEntry, SessionSpec, UsageLimits,
};
use crate::profiles::{apply_profile_environment, ensure_profile_dir, resolve_profile_dir};
use crate::shim_resolver::resolve_command_shim;
use crate::supervision::{SupervisedChild, create_supervised_command};
use crate::traits::{BoxFuture, ProviderDriver, ProviderSession};

/// Grok token usage and allocation budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokTokenBudget {
    #[serde(rename = "used_tokens")]
    pub used_tokens: u64,
    #[serde(rename = "limit_tokens")]
    pub limit_tokens: u64,
    #[serde(rename = "used_percent")]
    pub used_percent: f64,
}

/// Grok rate limits and reset timing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokRateLimits {
    #[serde(rename = "requests_per_minute_limit")]
    pub requests_per_minute_limit: u32,
    #[serde(rename = "requests_remaining")]
    pub requests_remaining: u32,
    #[serde(rename = "window_duration_seconds")]
    pub window_duration_seconds: u32,
    #[serde(rename = "resets_at")]
    pub resets_at: String,
}

/// Grok account usage and rate limits report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokUsageLimits {
    #[serde(rename = "account_tier")]
    pub account_tier: String,
    #[serde(rename = "monthly_spend_usd")]
    pub monthly_spend_usd: f64,
    #[serde(rename = "monthly_limit_usd")]
    pub monthly_limit_usd: f64,
    #[serde(rename = "token_budget")]
    pub token_budget: GrokTokenBudget,
    #[serde(rename = "rate_limits")]
    pub rate_limits: GrokRateLimits,
}

impl From<GrokUsageLimits> for UsageLimits {
    fn from(g: GrokUsageLimits) -> Self {
        let req_used = g
            .rate_limits
            .requests_per_minute_limit
            .saturating_sub(g.rate_limits.requests_remaining) as f64;
        let req_limit = g.rate_limits.requests_per_minute_limit.max(1) as f64;
        let req_percent = (req_used / req_limit) * 100.0;

        Self {
            limits: vec![
                RateLimitEntry {
                    limit_name: "requests_per_minute".to_string(),
                    used_percent: req_percent,
                    window_duration_mins: (g.rate_limits.window_duration_seconds / 60).max(1),
                    resets_at: g.rate_limits.resets_at,
                },
                RateLimitEntry {
                    limit_name: "token_budget".to_string(),
                    used_percent: g.token_budget.used_percent,
                    window_duration_mins: 1440,
                    resets_at: "".to_string(),
                },
            ],
        }
    }
}

/// Parses Grok usage limits from a JSON string payload.
pub fn parse_grok_usage_limits(json_str: &str) -> Result<GrokUsageLimits, ProviderError> {
    serde_json::from_str(json_str).map_err(|e| ProviderError::ProtocolError {
        provider: "grok".into(),
        message: format!("Failed to parse Grok usage limits JSON: {e}"),
    })
}

/// Resolves the Grok CLI binary following the resolution order:
/// 1. Explicit path in configuration/settings
/// 2. `grok-acp` in PATH
/// 3. `grok` in PATH
pub fn resolve_grok_binary(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit
        && p.is_file()
    {
        return Some(p.to_path_buf());
    }

    if let Some(resolved) = resolve_command_shim("grok-acp") {
        return Some(resolved.program);
    }

    if let Some(resolved) = resolve_command_shim("grok") {
        return Some(resolved.program);
    }

    None
}

/// Validates that an ACP initialize response satisfies Grok requirements.
pub fn validate_grok_initialize(res: &AcpInitializeResult) -> Result<(), ProviderError> {
    if res.protocol_version != 1 {
        return Err(ProviderError::ProtocolError {
            provider: "grok".into(),
            message: format!("Expected protocolVersion 1, got {}", res.protocol_version),
        });
    }

    if !res.agent_info.name.contains("grok") {
        return Err(ProviderError::ProtocolError {
            provider: "grok".into(),
            message: format!(
                "Expected agentInfo.name containing 'grok', got '{}'",
                res.agent_info.name
            ),
        });
    }

    Ok(())
}

/// Constructs a preamble block containing role instructions and knowledge context for Grok.
pub fn build_grok_preamble(
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

/// Grok ACP provider driver.
pub struct GrokDriver {
    pub base_data_dir: PathBuf,
}

impl GrokDriver {
    pub fn new(base_data_dir: PathBuf) -> Self {
        Self { base_data_dir }
    }

    fn resolve_binary(&self, cfg: &ProviderInstanceConfig) -> Option<PathBuf> {
        let explicit = cfg
            .settings
            .get("binaryPath")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        resolve_grok_binary(explicit.as_deref())
    }
}

impl ProviderDriver for GrokDriver {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            id: ProviderKind::Grok,
            display_name: "xAI Grok".to_string(),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calls: true,
                supports_sub_agents: false,
                supports_system_prompt: false,
                supports_reasoning: true,
                supports_approvals: true,
                supports_rate_limits: true,
                supports_resume: true,
                supports_images: true,
            },
            supported_models: vec![
                ModelInfo {
                    id: "grok-3".to_string(),
                    display_name: "Grok 3".to_string(),
                    context_window: Some(131_072),
                    max_output_tokens: Some(8_192),
                    supports_reasoning: true,
                },
                ModelInfo {
                    id: "grok-3-mini".to_string(),
                    display_name: "Grok 3 Mini".to_string(),
                    context_window: Some(131_072),
                    max_output_tokens: Some(8_192),
                    supports_reasoning: true,
                },
                ModelInfo {
                    id: "grok-2".to_string(),
                    display_name: "Grok 2".to_string(),
                    context_window: Some(131_072),
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
                    resolve_profile_dir(&self.base_data_dir, ProviderKind::Grok, &cfg.id);
                let has_api_key = profile_dir.join("api_key.txt").exists()
                    || cfg.settings.get("apiKey").is_some()
                    || std::env::var("GROK_API_KEY").is_ok()
                    || std::env::var("XAI_API_KEY").is_ok();

                let auth = if has_api_key {
                    ProviderAuthStatus::Authenticated {
                        account_label: Some("xAI API Key".to_string()),
                        email: None,
                        subscription: Some("Grok Developer Tier".to_string()),
                    }
                } else {
                    ProviderAuthStatus::Unauthenticated
                };

                let health = if has_api_key {
                    ProviderHealth::Healthy
                } else {
                    ProviderHealth::Degraded {
                        reason: "Grok CLI binary found but GROK_API_KEY unconfigured".to_string(),
                    }
                };

                ProviderSnapshot {
                    installed: true,
                    version: Some("1.0.4".to_string()),
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
                        reason: "Grok CLI binary not found in PATH or settings".to_string(),
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
        cfg: &'a ProviderInstanceConfig,
    ) -> BoxFuture<'a, Option<UsageLimits>> {
        Box::pin(async move {
            let profile_dir = resolve_profile_dir(&self.base_data_dir, ProviderKind::Grok, &cfg.id);
            let limits_file = profile_dir.join("usage_limits.json");
            if let Ok(content) = std::fs::read_to_string(&limits_file)
                && let Ok(limits) = parse_grok_usage_limits(&content)
            {
                return Some(limits.into());
            }

            None
        })
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
                        message: "Grok binary could not be resolved".into(),
                    })?;

            let profile_dir = resolve_profile_dir(&self.base_data_dir, ProviderKind::Grok, &cfg.id);
            ensure_profile_dir(&profile_dir)?;

            let mut cmd = create_supervised_command(&binary_path);
            cmd.arg("acp");
            cmd.current_dir(&spec.cwd);

            apply_profile_environment(
                &mut cmd,
                ProviderKind::Grok,
                &profile_dir,
                &cfg.env_overrides,
            );

            let mut child = SupervisedChild::spawn(cmd)?;
            let mut stdin = child
                .take_stdin()
                .ok_or_else(|| ProviderError::SupervisionError {
                    message: "Failed to open Grok child stdin".to_string(),
                })?;
            let stdout = child
                .take_stdout()
                .ok_or_else(|| ProviderError::SupervisionError {
                    message: "Failed to open Grok child stdout".to_string(),
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
                Some(build_grok_preamble(&spec.instructions, ""))
            } else {
                None
            };

            let session = AcpSession::new(child, stdin, rx, "session-grok".to_string())
                .with_preamble(preamble);

            Ok(Box::new(session) as Box<dyn ProviderSession>)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grok_preamble_generation() {
        let inst = Some(InjectedInstructions {
            role: "You are the Architect agent.".to_string(),
            knowledge_files: vec![(
                "System Architecture".to_string(),
                "Event sourced".to_string(),
            )],
            task_prompt: "".to_string(),
            output_contract: None,
        });

        let preamble = build_grok_preamble(&inst, "Plan migration strategy");
        assert!(preamble.contains("=== PANDAMUX AGENT ROLE INSTRUCTIONS ==="));
        assert!(preamble.contains("You are the Architect agent."));
        assert!(preamble.contains("=== KNOWLEDGE CONTEXT ==="));
        assert!(preamble.contains("System Architecture: Event sourced"));
        assert!(preamble.contains("=== USER TASK ==="));
        assert!(preamble.contains("Plan migration strategy"));
    }

    #[test]
    fn test_parse_grok_usage_limits() {
        let fixture = r#"{
            "account_tier": "grok_developer_tier_2",
            "monthly_spend_usd": 14.85,
            "monthly_limit_usd": 100.0,
            "token_budget": {
                "used_tokens": 148200,
                "limit_tokens": 1000000,
                "used_percent": 14.82
            },
            "rate_limits": {
                "requests_per_minute_limit": 60,
                "requests_remaining": 58,
                "window_duration_seconds": 60,
                "resets_at": "2026-09-26T21:05:00Z"
            }
        }"#;

        let limits = parse_grok_usage_limits(fixture).expect("Parse Grok usage limits");
        assert_eq!(limits.account_tier, "grok_developer_tier_2");
        assert_eq!(limits.rate_limits.requests_remaining, 58);
        assert_eq!(limits.token_budget.used_percent, 14.82);

        let usage: UsageLimits = limits.into();
        assert_eq!(usage.limits.len(), 2);
        assert_eq!(usage.limits[0].limit_name, "requests_per_minute");
        assert_eq!(usage.limits[0].window_duration_mins, 1);
    }

    #[test]
    fn test_validate_grok_initialize() {
        let fixture = r#"{
            "protocolVersion": 1,
            "agentInfo": {
                "name": "grok-acp",
                "version": "1.0.4"
            },
            "capabilities": {
                "loadSession": false,
                "resumeSession": true
            },
            "authMethods": ["api_key"]
        }"#;

        let init: AcpInitializeResult = serde_json::from_str(fixture).expect("Parse initialize");
        assert!(validate_grok_initialize(&init).is_ok());
    }
}
