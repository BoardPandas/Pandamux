use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use pandamux_core::{
    event::{ApprovalDecision, ApprovalKind, TurnOutcome},
    provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind},
    thread::{TurnInput, TurnUsage},
};
use tokio::sync::mpsc;

use crate::error::ProviderError;
use crate::models::{
    ModelInfo, ProviderAuthStatus, ProviderEvent, ProviderHealth, ProviderMetadata,
    ProviderSnapshot, SessionSpec, UsageLimits,
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
}

impl MockProviderDriver {
    pub fn new(kind: ProviderKind, display_name: &str) -> Self {
        Self {
            kind,
            display_name: display_name.to_string(),
            resume_token_to_issue: Arc::new(Mutex::new(None)),
            received_resume_tokens: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn with_resume_token(self, token: &str) -> Self {
        *self.resume_token_to_issue.lock().unwrap() = Some(token.to_string());
        self
    }

    pub fn received_resume_tokens(&self) -> Vec<String> {
        self.received_resume_tokens.lock().unwrap().clone()
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

    fn probe<'a>(&'a self, _cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, ProviderSnapshot> {
        Box::pin(async move {
            ProviderSnapshot {
                installed: true,
                version: Some("mock-1.0.0".to_string()),
                auth: ProviderAuthStatus::Authenticated {
                    account_label: Some("Mock Account".to_string()),
                    email: Some("mock@example.com".to_string()),
                    subscription: Some("Mock Pro".to_string()),
                },
                health: ProviderHealth::Healthy,
                checked_at_ms: now_ms(),
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
        _cfg: &'a ProviderInstanceConfig,
        spec: SessionSpec,
    ) -> BoxFuture<'a, Result<Box<dyn ProviderSession>, ProviderError>> {
        let resume_to_issue = self.resume_token_to_issue.lock().unwrap().clone();
        if let Some(ref r) = spec.resume {
            self.received_resume_tokens.lock().unwrap().push(r.clone());
        }

        Box::pin(async move {
            let (tx, rx) = mpsc::channel(64);
            let session = MockProviderSession {
                tx,
                rx: Some(rx),
                resume_token: resume_to_issue,
                last_turn_text: Arc::new(Mutex::new(None)),
            };
            Ok(Box::new(session) as Box<dyn ProviderSession>)
        })
    }
}

/// Active mock provider session emitting simulated events on turn input.
pub struct MockProviderSession {
    tx: mpsc::Sender<ProviderEvent>,
    rx: Option<mpsc::Receiver<ProviderEvent>>,
    resume_token: Option<String>,
    last_turn_text: Arc<Mutex<Option<String>>>,
}

impl ProviderSession for MockProviderSession {
    fn send_turn<'a>(&'a mut self, input: TurnInput) -> BoxFuture<'a, Result<(), ProviderError>> {
        *self.last_turn_text.lock().unwrap() = Some(input.text.clone());
        let tx = self.tx.clone();
        let prompt = input.text;

        Box::pin(async move {
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
                let cmd = prompt.trim_start_matches("request_approval:").trim().to_string();
                let _ = tx
                    .send(ProviderEvent::ApprovalRequested {
                        request_id: "req-mock-1".to_string(),
                        kind: ApprovalKind::CommandExecution,
                        detail: serde_json::json!({ "command": cmd }),
                        command: Some(cmd),
                    })
                    .await;
                return Ok(());
            }

            if prompt.starts_with("tool:") {
                let tool_name = prompt.trim_start_matches("tool:").trim().to_string();
                let _ = tx
                    .send(ProviderEvent::ToolCallStarted {
                        id: "call-1".to_string(),
                        name: tool_name.clone(),
                        input: serde_json::json!({ "arg": "val" }),
                    })
                    .await;
                let _ = tx
                    .send(ProviderEvent::ToolCallFinished {
                        id: "call-1".to_string(),
                        output: Some(format!("Executed {tool_name}")),
                        error: None,
                    })
                    .await;
            }

            if prompt.starts_with("subagent:") {
                let agent_type = prompt.trim_start_matches("subagent:").trim().to_string();
                let _ = tx
                    .send(ProviderEvent::SubAgentSpawned {
                        tool_use_id: "sub-1".to_string(),
                        parent_tool_use_id: None,
                        agent_type: agent_type.clone(),
                        description: format!("Subagent {agent_type}"),
                        model: Some("mock-fast".to_string()),
                    })
                    .await;
            }

            let _ = tx
                .send(ProviderEvent::TextDelta {
                    delta: format!("Echo: {prompt}"),
                })
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
        let tx = self.tx.clone();
        Box::pin(async move {
            match decision {
                ApprovalDecision::Approved => {
                    let _ = tx
                        .send(ProviderEvent::TextDelta {
                            delta: format!("Approval {request_id} accepted"),
                        })
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
                            }),
                        })
                        .await;
                }
                ApprovalDecision::Denied | ApprovalDecision::TimedOut => {
                    let _ = tx
                        .send(ProviderEvent::TextDelta {
                            delta: format!("Approval {request_id} rejected"),
                        })
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
        let tx = self.tx.clone();
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
