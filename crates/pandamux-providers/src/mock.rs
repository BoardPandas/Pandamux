use pandamux_core::{
    event::{ApprovalDecision, ApprovalKind, TurnOutcome},
    provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind},
    thread::{TurnInput, TurnUsage},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use crate::error::ProviderError;
use crate::models::{
    ModelInfo, ProviderAuthConfidence, ProviderAuthStatus, ProviderEvent, ProviderHealth,
    ProviderMetadata, ProviderSnapshot, SessionSpec, UsageLimits, provider_snapshot_expiry,
};
use crate::traits::{BoxFuture, ProviderDriver, ProviderSession};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// A scriptable mock provider driver for unit and integration testing.
pub struct MockProviderDriver {
    kind: ProviderKind,
    display_name: String,
    resume_token_to_issue: Arc<Mutex<Option<String>>>,
    received_resume_tokens: Arc<Mutex<Vec<String>>>,
    received_configs: Arc<Mutex<Vec<ProviderInstanceConfig>>>,
    active_probes: Arc<AtomicUsize>,
    max_active_probes: Arc<AtomicUsize>,
    session_starts: Arc<AtomicUsize>,
}

struct MockProbeGuard {
    active: Arc<AtomicUsize>,
}

impl Drop for MockProbeGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl MockProviderDriver {
    pub fn new(kind: ProviderKind, display_name: &str) -> Self {
        Self {
            kind,
            display_name: display_name.to_string(),
            resume_token_to_issue: Arc::new(Mutex::new(None)),
            received_resume_tokens: Arc::new(Mutex::new(Vec::new())),
            received_configs: Arc::new(Mutex::new(Vec::new())),
            active_probes: Arc::new(AtomicUsize::new(0)),
            max_active_probes: Arc::new(AtomicUsize::new(0)),
            session_starts: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_resume_token(self, token: &str) -> Self {
        *self.resume_token_to_issue.lock().unwrap() = Some(token.to_string());
        self
    }

    pub fn received_resume_tokens(&self) -> Vec<String> {
        self.received_resume_tokens.lock().unwrap().clone()
    }

    pub fn received_configs(&self) -> Vec<ProviderInstanceConfig> {
        self.received_configs.lock().unwrap().clone()
    }

    pub fn max_concurrent_probes(&self) -> usize {
        self.max_active_probes.load(Ordering::SeqCst)
    }

    pub fn session_start_count(&self) -> usize {
        self.session_starts.load(Ordering::SeqCst)
    }
}

impl ProviderDriver for MockProviderDriver {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            id: self.kind,
            display_name: self.display_name.clone(),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calls: true,
                supports_sub_agents: true,
                supports_system_prompt: true,
                supports_reasoning: true,
                supports_approvals: true,
                supports_rate_limits: true,
                supports_resume: true,
                supports_images: true,
                max_access_mode: pandamux_core::AccessMode::FullAccess,
            },
            supported_models: vec![
                ModelInfo {
                    id: "mock-fast".to_string(),
                    display_name: "Mock Fast".to_string(),
                    context_window: Some(128_000),
                    max_output_tokens: Some(4096),
                    supports_reasoning: false,
                },
                ModelInfo {
                    id: "mock-reasoning".to_string(),
                    display_name: "Mock Reasoning".to_string(),
                    context_window: Some(200_000),
                    max_output_tokens: Some(8192),
                    supports_reasoning: true,
                },
            ],
        }
    }

    fn probe<'a>(&'a self, cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, ProviderSnapshot> {
        Box::pin(async move {
            let active = self.active_probes.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active_probes.fetch_max(active, Ordering::SeqCst);
            let _guard = MockProbeGuard {
                active: self.active_probes.clone(),
            };
            if let Some(delay_ms) = cfg
                .settings
                .get("probeDelayMs")
                .and_then(|value| value.as_u64())
            {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }

            let checked_at_ms = now_ms();
            let installed = cfg
                .settings
                .get("installed")
                .and_then(|value| value.as_bool())
                .unwrap_or(true);
            let auth = match cfg
                .settings
                .get("authState")
                .and_then(|value| value.as_str())
                .unwrap_or("authenticated")
            {
                "authenticated" => ProviderAuthStatus::Authenticated {
                    account_label: cfg
                        .settings
                        .get("accountLabel")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                        .or_else(|| Some("Mock Account".to_string())),
                    email: cfg
                        .settings
                        .get("email")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    subscription: cfg
                        .settings
                        .get("subscription")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                        .or_else(|| Some("Mock Pro".to_string())),
                },
                "configured" => ProviderAuthStatus::Configured,
                "unauthenticated" => ProviderAuthStatus::Unauthenticated,
                _ => ProviderAuthStatus::Unknown,
            };
            let auth_confidence = match cfg
                .settings
                .get("authConfidence")
                .and_then(|value| value.as_str())
                .unwrap_or("reported")
            {
                "reported" => ProviderAuthConfidence::Reported,
                "detected" => ProviderAuthConfidence::Detected,
                _ => ProviderAuthConfidence::Unknown,
            };
            let reason = cfg
                .settings
                .get("healthReason")
                .and_then(|value| value.as_str())
                .unwrap_or("mock provider health fixture")
                .to_string();
            let health = match cfg
                .settings
                .get("runtimeHealth")
                .and_then(|value| value.as_str())
                .unwrap_or(if installed { "healthy" } else { "unavailable" })
            {
                "healthy" => ProviderHealth::Healthy,
                "degraded" => ProviderHealth::Degraded { reason },
                _ => ProviderHealth::Unavailable { reason },
            };

            ProviderSnapshot {
                installed,
                resolved_executable: installed.then(|| {
                    cfg.binary_path
                        .clone()
                        .unwrap_or_else(|| "mock-provider".to_string())
                }),
                version: cfg
                    .settings
                    .get("version")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .or_else(|| Some("mock-1.0.0".to_string())),
                auth,
                auth_confidence,
                health,
                runtime_code: cfg
                    .settings
                    .get("runtimeCode")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .or_else(|| Some("runtime_ready".to_string())),
                checked_at_ms,
                expires_at_ms: provider_snapshot_expiry(checked_at_ms),
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
            cfg.settings
                .get("capacityReport")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
        })
    }

    fn start_session<'a>(
        &'a self,
        cfg: &'a ProviderInstanceConfig,
        spec: SessionSpec,
    ) -> BoxFuture<'a, Result<Box<dyn ProviderSession>, ProviderError>> {
        self.session_starts.fetch_add(1, Ordering::SeqCst);
        self.received_configs.lock().unwrap().push(cfg.clone());
        let resume_to_issue = self.resume_token_to_issue.lock().unwrap().clone();
        if let Some(ref r) = spec.resume {
            self.received_resume_tokens.lock().unwrap().push(r.clone());
        }

        Box::pin(async move {
            let (tx, rx) = mpsc::channel(64);
            let capacity_failure_mode = cfg
                .settings
                .get("capacityFailure")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            let session = MockProviderSession {
                tx: Some(tx),
                rx: Some(rx),
                resume_token: resume_to_issue,
                last_turn_text: Arc::new(Mutex::new(None)),
                capacity_failure_mode,
            };
            Ok(Box::new(session) as Box<dyn ProviderSession>)
        })
    }
}

/// Active mock provider session emitting simulated events on turn input.
pub struct MockProviderSession {
    tx: Option<mpsc::Sender<ProviderEvent>>,
    rx: Option<mpsc::Receiver<ProviderEvent>>,
    resume_token: Option<String>,
    last_turn_text: Arc<Mutex<Option<String>>>,
    capacity_failure_mode: Option<String>,
}

impl ProviderSession for MockProviderSession {
    fn send_turn<'a>(&'a mut self, input: TurnInput) -> BoxFuture<'a, Result<(), ProviderError>> {
        *self.last_turn_text.lock().unwrap() = Some(input.text.clone());
        let prompt = input.text;

        if prompt == "close_stream" {
            let tx = self.tx.take().expect("mock event sender is available");
            return Box::pin(async move {
                let _ = tx
                    .send(ProviderEvent::Error {
                        message: "mock provider process exited".to_string(),
                        recoverable: false,
                    })
                    .await;
                Ok(())
            });
        }

        let tx = self
            .tx
            .as_ref()
            .expect("mock event sender is available")
            .clone();

        Box::pin(async move {
            if let Some(mode) = self.capacity_failure_mode.clone() {
                match mode.as_str() {
                    "after_text" => {
                        let _ = crate::forward_provider_event(
                            &tx,
                            ProviderEvent::TextDelta {
                                delta: "partial mock output".to_string(),
                            },
                        )
                        .await;
                    }
                    "after_file" | "after_command" | "after_network" | "after_tool" => {
                        let _ = crate::forward_provider_event(
                            &tx,
                            ProviderEvent::ToolCallStarted {
                                id: format!("capacity-{mode}"),
                                name: mode.trim_start_matches("after_").to_string(),
                                input: serde_json::json!({ "fixture": mode }),
                            },
                        )
                        .await;
                    }
                    "after_approval" => {
                        let _ = crate::forward_provider_event(
                            &tx,
                            ProviderEvent::ApprovalRequested {
                                request_id: "capacity-approval".to_string(),
                                kind: ApprovalKind::CommandExecution,
                                detail: serde_json::json!({ "command": "fixture" }),
                                command: Some("fixture".to_string()),
                            },
                        )
                        .await;
                    }
                    _ => {}
                }
                let _ = crate::forward_provider_event(
                    &tx,
                    ProviderEvent::Error {
                        message: "mock rate limit exhausted".to_string(),
                        recoverable: true,
                    },
                )
                .await;
                return Ok(());
            }

            if prompt.starts_with("error:") {
                let _ = tx
                    .send(ProviderEvent::Error {
                        message: prompt.trim_start_matches("error:").trim().to_string(),
                        recoverable: false,
                    })
                    .await;
                let _ = tx
                    .send(ProviderEvent::TurnCompleted {
                        outcome: TurnOutcome::Failed,
                        usage: None,
                    })
                    .await;
                return Ok(());
            }

            if prompt.starts_with("request_approval:") {
                let cmd = prompt
                    .trim_start_matches("request_approval:")
                    .trim()
                    .to_string();
                let _ = crate::forward_provider_event(
                    &tx,
                    ProviderEvent::ApprovalRequested {
                        request_id: "req-mock-1".to_string(),
                        kind: ApprovalKind::CommandExecution,
                        detail: serde_json::json!({ "command": cmd }),
                        command: Some(cmd),
                    },
                )
                .await;
                return Ok(());
            }

            if prompt.starts_with("tool:") {
                let tool_name = prompt.trim_start_matches("tool:").trim().to_string();
                let _ = crate::forward_provider_event(
                    &tx,
                    ProviderEvent::ToolCallStarted {
                        id: "call-1".to_string(),
                        name: tool_name.clone(),
                        input: serde_json::json!({ "arg": "val" }),
                    },
                )
                .await;
                let _ = crate::forward_provider_event(
                    &tx,
                    ProviderEvent::ToolCallFinished {
                        id: "call-1".to_string(),
                        output: Some(format!("Executed {tool_name}")),
                        error: None,
                    },
                )
                .await;
            }

            if prompt.starts_with("unresolved_tool_success:") {
                let tool_name = prompt
                    .trim_start_matches("unresolved_tool_success:")
                    .trim()
                    .to_string();
                let _ = crate::forward_provider_event(
                    &tx,
                    ProviderEvent::ToolCallStarted {
                        id: "unresolved-call-1".to_string(),
                        name: tool_name,
                        input: serde_json::json!({ "fixture": "unresolved success" }),
                    },
                )
                .await;
            }

            if prompt.starts_with("subagent:") {
                let agent_type = prompt.trim_start_matches("subagent:").trim().to_string();
                let _ = crate::forward_provider_event(
                    &tx,
                    ProviderEvent::SubAgentSpawned {
                        tool_use_id: "sub-1".to_string(),
                        parent_tool_use_id: None,
                        agent_type: agent_type.clone(),
                        description: format!("Subagent {agent_type}"),
                        model: Some("mock-fast".to_string()),
                    },
                )
                .await;
            }

            let _ = crate::forward_provider_event(
                &tx,
                ProviderEvent::TextDelta {
                    delta: format!("Echo: {prompt}"),
                },
            )
            .await;

            let _ = tx
                .send(ProviderEvent::TurnCompleted {
                    outcome: TurnOutcome::Success,
                    usage: Some(TurnUsage {
                        input_tokens: 15,
                        output_tokens: 25,
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        cost_usd: Some(0.001),
                        ..Default::default()
                    }),
                })
                .await;

            Ok(())
        })
    }

    fn events(&mut self) -> &mut mpsc::Receiver<ProviderEvent> {
        self.rx.as_mut().expect("events receiver already taken")
    }

    fn take_event_receiver(&mut self) -> Option<mpsc::Receiver<ProviderEvent>> {
        self.rx.take()
    }

    fn respond_approval<'a>(
        &'a mut self,
        request_id: String,
        decision: ApprovalDecision,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        let tx = self
            .tx
            .as_ref()
            .expect("mock event sender is available")
            .clone();
        Box::pin(async move {
            match decision {
                ApprovalDecision::Approved => {
                    let _ = crate::forward_provider_event(
                        &tx,
                        ProviderEvent::TextDelta {
                            delta: format!("Approval {request_id} accepted"),
                        },
                    )
                    .await;
                    let _ = tx
                        .send(ProviderEvent::TurnCompleted {
                            outcome: TurnOutcome::Success,
                            usage: Some(TurnUsage {
                                input_tokens: 20,
                                output_tokens: 30,
                                cache_read_tokens: None,
                                cache_write_tokens: None,
                                cost_usd: Some(0.0015),
                                ..Default::default()
                            }),
                        })
                        .await;
                }
                ApprovalDecision::Denied | ApprovalDecision::TimedOut => {
                    let _ = crate::forward_provider_event(
                        &tx,
                        ProviderEvent::TextDelta {
                            delta: format!("Approval {request_id} rejected"),
                        },
                    )
                    .await;
                    let _ = tx
                        .send(ProviderEvent::TurnCompleted {
                            outcome: TurnOutcome::Success,
                            usage: None,
                        })
                        .await;
                }
            }
            Ok(())
        })
    }

    fn interrupt<'a>(&'a mut self) -> BoxFuture<'a, Result<(), ProviderError>> {
        let tx = self
            .tx
            .as_ref()
            .expect("mock event sender is available")
            .clone();
        Box::pin(async move {
            let _ = tx
                .send(ProviderEvent::TurnCompleted {
                    outcome: TurnOutcome::Cancelled,
                    usage: None,
                })
                .await;
            Ok(())
        })
    }

    fn resume_token(&self) -> Option<String> {
        self.resume_token.clone()
    }

    fn shutdown(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {})
    }
}
