use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, Mutex};

use pandamux_core::{
    event::{ThreadEvent, ThreadEventKind, ToolCallStatus, TurnOutcome},
    ids::{ThreadId, TurnId},
    provider_config::{ProviderInstanceConfig, ProviderKind},
    thread::{Thread, ThreadStatus, ThreadWorkspace, Turn, TurnInput, TurnStatus},
    EnvironmentId, ProviderInstanceId,
};
use pandamux_protocol::{
    EventEnvelope, RpcError, ThreadCancelTurnParams, ThreadCreateParams, ThreadGetParams,
    ThreadGetResult, ThreadListParams, ThreadRespondApprovalParams, ThreadResumeParams,
    ThreadResumeResult, ThreadSendTurnParams, ThreadSendTurnResult,
};
use pandamux_providers::traits::ProviderSession;
use pandamux_providers::{ProviderEvent, SessionSpec};

use crate::driver_registry::DriverRegistry;
use crate::store::Store;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn resolve_provider_kind(instance_id: &ProviderInstanceId) -> ProviderKind {
    let s = instance_id.as_str().to_lowercase();
    if s.starts_with("claude") {
        ProviderKind::Claude
    } else if s.starts_with("codex") {
        ProviderKind::Codex
    } else if s.starts_with("antigravity") || s.starts_with("gemini") {
        ProviderKind::Antigravity
    } else {
        ProviderKind::Custom
    }
}

/// An active supervised session paired with its event channel receiver.
#[derive(Clone)]
pub struct ActiveThreadSession {
    pub session: Arc<Mutex<Box<dyn ProviderSession>>>,
    pub events_rx: Arc<Mutex<Option<mpsc::Receiver<ProviderEvent>>>>,
}

/// Central manager orchestrating thread execution, streaming, and state persistence.
pub struct ThreadManager {
    store: Store,
    drivers: Arc<DriverRegistry>,
    environment_id: String,
    active_sessions: Arc<Mutex<HashMap<ThreadId, ActiveThreadSession>>>,
    broadcaster: broadcast::Sender<EventEnvelope>,
}

impl ThreadManager {
    pub fn new(store: Store, drivers: Arc<DriverRegistry>, environment_id: String) -> Self {
        // Reconcile any in-flight turns left unfinalized from a prior server run
        let _ = store.reconcile_active_turns();

        let (broadcaster, _) = broadcast::channel(512);

        Self {
            store,
            drivers,
            environment_id,
            active_sessions: Arc::new(Mutex::new(HashMap::new())),
            broadcaster,
        }
    }

    /// Subscribes to the broadcast stream of thread event envelopes.
    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.broadcaster.subscribe()
    }

    /// Creates and persists a new thread with optional git worktree.
    pub fn create_thread(&self, params: ThreadCreateParams) -> Result<Thread, RpcError> {
        let now = now_ms();
        let default_cwd = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let cwd = params.cwd.unwrap_or(default_cwd);
        let workspace = ThreadWorkspace {
            cwd,
            worktree: params.worktree,
        };

        let thread = Thread {
            id: ThreadId::generate(),
            project_id: params.project_id,
            environment_id: params
                .environment_id
                .unwrap_or_else(|| EnvironmentId::from(self.environment_id.as_str())),
            parent_thread_id: None,
            title: params.title.unwrap_or_else(|| "New Thread".to_string()),
            provider_instance_id: params
                .provider_instance_id
                .unwrap_or_else(|| ProviderInstanceId::from("prov-default")),
            model: params.model.unwrap_or_else(|| "default".to_string()),
            effort: params.effort,
            access_mode: params.access_mode.unwrap_or_default(),
            workspace,
            status: ThreadStatus::Idle,
            agent: None,
            origin: Default::default(),
            created_at_ms: now,
            updated_at_ms: now,
        };

        self.store
            .save_thread(&thread)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        Ok(thread)
    }

    /// Retrieves a thread and its turn history.
    pub fn get_thread(&self, params: ThreadGetParams) -> Result<ThreadGetResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread not found: {}", params.thread_id)))?;

        let turns = self
            .store
            .list_turns(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        Ok(ThreadGetResult { thread, turns })
    }

    /// Lists threads optionally filtered by project or status.
    pub fn list_threads(&self, params: ThreadListParams) -> Result<Vec<Thread>, RpcError> {
        let threads = self
            .store
            .list_threads()
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let filtered = threads
            .into_iter()
            .filter(|t| {
                if let Some(ref p) = params.project_id {
                    if t.project_id.as_ref() != Some(p) {
                        return false;
                    }
                }
                if let Some(st) = params.status {
                    if t.status != st {
                        return false;
                    }
                }
                if let Some(ref parent) = params.parent_thread_id {
                    if t.parent_thread_id.as_ref() != Some(parent) {
                        return false;
                    }
                }
                true
            })
            .collect();

        Ok(filtered)
    }

    /// Dispatches a prompt turn to an agent provider and streams events into SQLite.
    pub async fn send_turn(&self, params: ThreadSendTurnParams) -> Result<ThreadSendTurnResult, RpcError> {
        let mut thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread not found: {}", params.thread_id)))?;

        if thread.status == ThreadStatus::Working {
            return Err(RpcError::invalid_params("Thread is currently executing another turn"));
        }
        if thread.status == ThreadStatus::AwaitingApproval {
            return Err(RpcError::invalid_params("Thread is awaiting approval response"));
        }

        let turn_id = TurnId::generate();
        let base_seq = self
            .store
            .get_latest_seq(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let turn_seq = base_seq + 1;
        let started_ms = now_ms();

        let turn_input = TurnInput {
            text: params.text.clone(),
            attachment_ids: params.attachment_ids.clone(),
            model: params.model.clone().or_else(|| Some(thread.model.clone())),
            effort: params.effort.clone().or_else(|| thread.effort.clone()),
        };

        let turn = Turn {
            id: turn_id.clone(),
            thread_id: params.thread_id.clone(),
            seq: turn_seq,
            input: turn_input.clone(),
            status: TurnStatus::Running,
            started_at_ms: started_ms,
            ended_at_ms: None,
            usage: None,
            checkpoint_before: None,
            checkpoint_after: None,
        };

        self.store
            .save_turn(&turn)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        thread.status = ThreadStatus::Working;
        thread.updated_at_ms = started_ms;
        self.store
            .save_thread(&thread)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        // Emit TurnRequested event
        let req_event = ThreadEvent {
            thread_id: params.thread_id.clone(),
            seq: turn_seq,
            at_ms: started_ms,
            kind: ThreadEventKind::TurnRequested {
                turn_id: turn_id.clone(),
                input: turn_input.clone(),
            },
        };
        self.store
            .append_event(&req_event)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let _ = self.broadcaster.send(EventEnvelope {
            subscription_id: String::new(),
            environment_id: self.environment_id.clone().into(),
            thread_id: Some(params.thread_id.clone()),
            run_id: None,
            seq: turn_seq,
            at_ms: started_ms,
            kind: req_event.kind,
        });

        // Emit TurnStarted event
        let started_seq = turn_seq + 1;
        let start_event = ThreadEvent {
            thread_id: params.thread_id.clone(),
            seq: started_seq,
            at_ms: started_ms,
            kind: ThreadEventKind::TurnStarted {
                turn_id: turn_id.clone(),
            },
        };
        self.store
            .append_event(&start_event)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let _ = self.broadcaster.send(EventEnvelope {
            subscription_id: String::new(),
            environment_id: self.environment_id.clone().into(),
            thread_id: Some(params.thread_id.clone()),
            run_id: None,
            seq: started_seq,
            at_ms: started_ms,
            kind: start_event.kind,
        });

        // Resolve or spawn provider session
        let active_sess = {
            let mut active = self.active_sessions.lock().await;
            if let Some(existing) = active.get(&params.thread_id) {
                existing.clone()
            } else {
                let kind = resolve_provider_kind(&thread.provider_instance_id);
                let driver = self
                    .drivers
                    .get(kind)
                    .or_else(|| self.drivers.get(ProviderKind::Custom))
                    .ok_or_else(|| {
                        RpcError::internal_error(format!("No driver registered for provider {:?}", kind))
                    })?;

                let resume_token = self
                    .store
                    .get_resume_token(&params.thread_id)
                    .map_err(|e| RpcError::internal_error(e.to_string()))?;

                let spec = SessionSpec {
                    cwd: PathBuf::from(thread.workspace.effective_path()),
                    model: thread.model.clone(),
                    effort: thread.effort.clone(),
                    access: thread.access_mode,
                    instructions: None,
                    tool_policy: None,
                    resume: resume_token,
                };

                let cfg = ProviderInstanceConfig {
                    id: thread.provider_instance_id.clone(),
                    provider: kind,
                    display_name: thread.provider_instance_id.to_string(),
                    profile_dir: format!(
                        "profiles/{}/{}",
                        kind.as_str(),
                        thread.provider_instance_id.as_str()
                    ),
                    concurrency_limit: 2,
                    env_overrides: Default::default(),
                    settings: serde_json::Value::Null,
                    enabled: true,
                };

                let mut session = driver
                    .start_session(&cfg, spec)
                    .await
                    .map_err(|e| RpcError::internal_error(e.to_string()))?;

                let rx = session.take_event_receiver();
                let entry = ActiveThreadSession {
                    session: Arc::new(Mutex::new(session)),
                    events_rx: Arc::new(Mutex::new(rx)),
                };
                active.insert(params.thread_id.clone(), entry.clone());
                entry
            }
        };

        // Send turn prompt into provider session
        {
            let mut sess = active_sess.session.lock().await;
            sess.send_turn(turn_input)
                .await
                .map_err(|e| RpcError::internal_error(e.to_string()))?;
        }

        // Take receiver for lock-free streaming in background task
        let rx_opt = {
            let mut rx_guard = active_sess.events_rx.lock().await;
            rx_guard.take()
        };

        // Spawn background task to stream events into SQLite and broadcast
        let store = self.store.clone();
        let broadcaster = self.broadcaster.clone();
        let thread_id = params.thread_id.clone();
        let env_id = self.environment_id.clone();
        let session_clone = active_sess.session.clone();
        let events_rx_holder = active_sess.events_rx.clone();
        let turn_id_for_task = turn_id.clone();
        let mut stream_seq = started_seq;

        tokio::spawn(async move {
            let turn_id = turn_id_for_task;
            if let Some(mut rx) = rx_opt {
                while let Some(prov_event) = rx.recv().await {
                    stream_seq += 1;
                    let at = now_ms();

                match prov_event {
                    ProviderEvent::TextDelta { delta } => {
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::AssistantText {
                                item_id: format!("{}-text", turn_id.as_str()),
                                text: delta,
                            },
                        };
                        let _ = store.append_event(&ev);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::ReasoningDelta { delta } => {
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::Reasoning {
                                item_id: format!("{}-reasoning", turn_id.as_str()),
                                text: delta,
                            },
                        };
                        let _ = store.append_event(&ev);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::ToolCallStarted { id, name, input } => {
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::ToolCall {
                                item_id: id,
                                name,
                                input,
                                status: ToolCallStatus::Executing,
                                output: None,
                            },
                        };
                        let _ = store.append_event(&ev);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::ToolCallFinished { id, output, error } => {
                        let status = if error.is_some() {
                            ToolCallStatus::Failed
                        } else {
                            ToolCallStatus::Success
                        };
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::ToolCall {
                                item_id: id,
                                name: String::new(),
                                input: serde_json::Value::Null,
                                status,
                                output,
                            },
                        };
                        let _ = store.append_event(&ev);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::ApprovalRequested {
                        request_id,
                        kind,
                        detail,
                        ..
                    } => {
                        if let Ok(Some(mut th)) = store.get_thread(&thread_id) {
                            th.status = ThreadStatus::AwaitingApproval;
                            th.updated_at_ms = at;
                            let _ = store.save_thread(&th);
                        }
                        if let Ok(Some(mut tr)) = store.get_turn(&turn_id) {
                            tr.status = TurnStatus::AwaitingApproval;
                            let _ = store.save_turn(&tr);
                        }

                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::ApprovalRequested {
                                request_id,
                                kind,
                                detail,
                            },
                        };
                        let _ = store.append_event(&ev);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::SubAgentSpawned {
                        tool_use_id,
                        parent_tool_use_id,
                        agent_type,
                        description,
                        model,
                    } => {
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::SubAgentSpawned {
                                sub_agent_id: tool_use_id,
                                parent_sub_agent_id: parent_tool_use_id,
                                parent_item_id: None,
                                title: description,
                                agent_type,
                                model: model.unwrap_or_else(|| "default".to_string()),
                                effort: None,
                            },
                        };
                        let _ = store.append_event(&ev);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::TurnCompleted { outcome, usage } => {
                        let res_tok = session_clone.lock().await.resume_token().map(|s| s.to_string());
                        if let Some(res_tok) = res_tok {
                            let _ = store.save_resume_token(&thread_id, &res_tok);
                        }

                        if let Ok(Some(mut tr)) = store.get_turn(&turn_id) {
                            tr.status = match outcome {
                                TurnOutcome::Success => TurnStatus::Completed,
                                TurnOutcome::Cancelled => TurnStatus::Interrupted,
                                TurnOutcome::Failed => TurnStatus::Failed,
                                TurnOutcome::Interrupted => TurnStatus::Interrupted,
                            };
                            tr.ended_at_ms = Some(at);
                            tr.usage = usage;
                            let _ = store.save_turn(&tr);
                        }

                        if let Ok(Some(mut th)) = store.get_thread(&thread_id) {
                            th.status = ThreadStatus::Idle;
                            th.updated_at_ms = at;
                            let _ = store.save_thread(&th);
                        }

                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::TurnCompleted { outcome },
                        };
                        let _ = store.append_event(&ev);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });

                        stream_seq += 1;
                        let settle = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::TurnSettled {
                                changed_files: vec![],
                            },
                        };
                        let _ = store.append_event(&settle);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: settle.kind,
                        });

                        break;
                    }
                    ProviderEvent::Error {
                        message,
                        recoverable,
                    } => {
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::Error {
                                class: "provider".to_string(),
                                message,
                                recoverable,
                            },
                        };
                        let _ = store.append_event(&ev);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::RateLimitObserved { details } => {
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::RateLimitObserved {
                                provider: "agent".to_string(),
                                details,
                            },
                        };
                        let _ = store.append_event(&ev);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::Notice { level, message } => {
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::Notice { level, message },
                        };
                        let _ = store.append_event(&ev);
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                }
            }

            let mut rx_guard = events_rx_holder.lock().await;
            *rx_guard = Some(rx);
        }
    });

        Ok(ThreadSendTurnResult {
            turn_id,
            seq: started_seq,
        })
    }

    /// Responds to a pending approval request and resumes the turn.
    pub async fn respond_approval(&self, params: ThreadRespondApprovalParams) -> Result<(), RpcError> {
        let session_opt = {
            let active = self.active_sessions.lock().await;
            active.get(&params.thread_id).cloned()
        };

        let active_sess = session_opt.ok_or_else(|| {
            RpcError::invalid_params(format!("No active session found for thread {}", params.thread_id))
        })?;

        {
            let mut sess = active_sess.session.lock().await;
            sess.respond_approval(params.request_id.clone(), params.decision)
                .await
                .map_err(|e| RpcError::internal_error(e.to_string()))?;
        }

        let now = now_ms();
        let next_seq = self
            .store
            .get_latest_seq(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            + 1;

        let ev = ThreadEvent {
            thread_id: params.thread_id.clone(),
            seq: next_seq,
            at_ms: now,
            kind: ThreadEventKind::ApprovalResolved {
                request_id: params.request_id,
                decision: params.decision,
                by: "user".to_string(),
            },
        };
        self.store
            .append_event(&ev)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let _ = self.broadcaster.send(EventEnvelope {
            subscription_id: String::new(),
            environment_id: self.environment_id.clone().into(),
            thread_id: Some(params.thread_id.clone()),
            run_id: None,
            seq: next_seq,
            at_ms: now,
            kind: ev.kind,
        });

        if let Ok(Some(mut th)) = self.store.get_thread(&params.thread_id) {
            th.status = ThreadStatus::Working;
            th.updated_at_ms = now;
            let _ = self.store.save_thread(&th);
        }

        Ok(())
    }

    /// Interrupts the active turn on a thread.
    pub async fn interrupt_turn(&self, params: ThreadCancelTurnParams) -> Result<(), RpcError> {
        let session_opt = {
            let active = self.active_sessions.lock().await;
            active.get(&params.thread_id).cloned()
        };

        if let Some(active_sess) = session_opt {
            let mut sess = active_sess.session.lock().await;
            let _ = sess.interrupt().await;
        }

        let now = now_ms();
        if let Ok(Some(mut th)) = self.store.get_thread(&params.thread_id) {
            th.status = ThreadStatus::Idle;
            th.updated_at_ms = now;
            let _ = self.store.save_thread(&th);
        }

        if let Ok(turns) = self.store.list_turns(&params.thread_id) {
            for mut tr in turns {
                if matches!(
                    tr.status,
                    TurnStatus::Running | TurnStatus::AwaitingApproval | TurnStatus::Requested
                ) {
                    tr.status = TurnStatus::Interrupted;
                    tr.ended_at_ms = Some(now);
                    let _ = self.store.save_turn(&tr);
                }
            }
        }

        Ok(())
    }

    /// Queries the saved resumption token for continuing a thread across restarts.
    pub fn resume_thread(&self, params: ThreadResumeParams) -> Result<ThreadResumeResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread not found: {}", params.thread_id)))?;

        let resume_token = self
            .store
            .get_resume_token(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        Ok(ThreadResumeResult {
            thread_id: thread.id,
            resume_token,
        })
    }
}
