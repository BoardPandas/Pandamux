use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, broadcast, mpsc};

use pandamux_core::{
    CAPACITY_SNAPSHOT_SCHEMA_VERSION, CapacityConfidence, CapacityEvidenceScope, CapacitySnapshot,
    DEFAULT_CAPACITY_MAX_AGE_MS, EnvironmentId, ProjectId, ProviderKind,
    event::{ThreadEvent, ThreadEventKind, ToolCallStatus, TurnOutcome},
    ids::{ThreadId, TurnId},
    thread::{Thread, ThreadStatus, ThreadWorkspace, Turn, TurnInput, TurnStatus},
};
use pandamux_protocol::{
    EventEnvelope, GitCommitParams, GitCommitResult, GitCreatePrParams, GitCreatePrResult,
    GitPushParams, GitPushResult, GitStatusParams, GitStatusResult, RpcError,
    ThreadCancelTurnParams, ThreadCreateParams, ThreadGetParams, ThreadGetResult, ThreadListParams,
    ThreadRespondApprovalParams, ThreadResumeParams, ThreadResumeResult, ThreadSendTurnParams,
    ThreadSendTurnResult,
};
use pandamux_providers::traits::ProviderSession;
use pandamux_providers::{ProviderEvent, SessionSpec};

use crate::provider_resolver::{ProviderResolver, ResolvedProviderExecution};
use crate::request_context::CapabilityAuthority;
use crate::store::{ProviderExecutionAudit, Store};
use crate::usage_scanner::{UsageScanTrigger, UsageScannerHandle};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn organization_policy_error(project_id: &ProjectId, message: impl Into<String>) -> RpcError {
    RpcError::with_data(
        -32030,
        message,
        serde_json::json!({
            "reason": "organization_policy_denied",
            "projectId": project_id,
        }),
    )
}

fn record_local_codex_capacity(
    store: &Store,
    environment_id: &EnvironmentId,
    provider_instance_id: &pandamux_core::ProviderInstanceId,
    provider_version: Option<&str>,
    details: &serde_json::Value,
    observed_at_ms: u64,
) -> Result<(), crate::store::StoreError> {
    let Some((subscription_key, _)) =
        store.usage_subscription_for_provider(environment_id, provider_instance_id)?
    else {
        return Ok(());
    };
    let limits = pandamux_providers::parse_rate_limits_value(details).map_err(|error| {
        crate::store::StoreError::Data(format!("invalid Codex rate-limit event: {error}"))
    })?;
    let snapshots = limits
        .into_iter()
        .map(|limit| {
            let duration_ms = u64::from(limit.window_duration_mins).saturating_mul(60 * 1000);
            CapacitySnapshot {
                schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION,
                subscription_key: subscription_key.clone(),
                environment_id: Some(environment_id.clone()),
                provider_instance_id: Some(provider_instance_id.clone()),
                window_name: Some(limit.limit_name),
                window_start_ms: limit
                    .resets_at_ms
                    .map(|reset| reset.saturating_sub(duration_ms)),
                window_end_ms: limit.resets_at_ms,
                used_millionths: limit.used_millionths,
                remaining_millionths: limit.remaining_millionths,
                reset_at_ms: limit.resets_at_ms,
                source: "codex_rate_limit_event".to_string(),
                confidence: CapacityConfidence::Reported,
                evidence_scope: CapacityEvidenceScope::Local,
                hard_limit: limit.hard_limit
                    && limit.remaining_millionths == Some(0)
                    && limit.resets_at_ms.is_some(),
                provider_version: provider_version.map(str::to_string),
                as_of_ms: observed_at_ms,
                max_age_ms: DEFAULT_CAPACITY_MAX_AGE_MS,
            }
        })
        .collect::<Vec<_>>();
    store.save_capacity_snapshots(&snapshots)
}

/// An active supervised session paired with its event channel receiver.
#[derive(Clone)]
pub struct ActiveThreadSession {
    pub session: Arc<Mutex<Option<Box<dyn ProviderSession>>>>,
    pub events_rx: Arc<Mutex<Option<mpsc::Receiver<ProviderEvent>>>>,
    pub event_writer: Arc<Mutex<()>>,
    pub execution: Arc<ResolvedProviderExecution>,
    pub generation: uuid::Uuid,
    pub child_capability_token: String,
    pub capability_authority: Arc<CapabilityAuthority>,
}

async fn shutdown_active_session(active: ActiveThreadSession, interrupt: bool) {
    let session = {
        let mut session = active.session.lock().await;
        session.take()
    };
    if let Some(mut session) = session {
        if interrupt {
            let _ = session.interrupt().await;
        }
        session.shutdown().await;
    }
    active
        .capability_authority
        .revoke(&active.child_capability_token);
}

async fn remove_active_generation(
    active_sessions: &Arc<Mutex<HashMap<ThreadId, ActiveThreadSession>>>,
    thread_id: &ThreadId,
    generation: uuid::Uuid,
) -> Option<ActiveThreadSession> {
    let mut active = active_sessions.lock().await;
    if active
        .get(thread_id)
        .is_some_and(|session| session.generation == generation)
    {
        active.remove(thread_id)
    } else {
        None
    }
}

/// Central manager orchestrating thread execution, streaming, and state persistence.
pub struct ThreadManager {
    store: Store,
    provider_resolver: Arc<ProviderResolver>,
    capability_authority: Arc<CapabilityAuthority>,
    environment_id: String,
    active_sessions: Arc<Mutex<HashMap<ThreadId, ActiveThreadSession>>>,
    broadcaster: broadcast::Sender<EventEnvelope>,
    usage_scanner: Arc<std::sync::RwLock<Option<UsageScannerHandle>>>,
}

impl ThreadManager {
    pub fn new(
        store: Store,
        provider_resolver: Arc<ProviderResolver>,
        environment_id: String,
        capability_authority: Arc<CapabilityAuthority>,
    ) -> Self {
        // Reconcile any in-flight turns left unfinalized from a prior server run
        let _ = store.reconcile_active_turns();

        let (broadcaster, _) = broadcast::channel(512);

        Self {
            store,
            provider_resolver,
            capability_authority,
            environment_id,
            active_sessions: Arc::new(Mutex::new(HashMap::new())),
            broadcaster,
            usage_scanner: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    pub fn set_usage_scanner(&self, scanner: Option<UsageScannerHandle>) {
        *self
            .usage_scanner
            .write()
            .expect("usage scanner handle lock") = scanner;
    }

    /// Subscribes to the broadcast stream of thread event envelopes.
    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.broadcaster.subscribe()
    }

    /// Access the raw broadcast channel sender for event relaying.
    pub fn broadcaster(&self) -> &broadcast::Sender<EventEnvelope> {
        &self.broadcaster
    }

    pub async fn shutdown_all(&self) {
        let sessions = {
            let mut active = self.active_sessions.lock().await;
            active
                .drain()
                .map(|(_, session)| session)
                .collect::<Vec<_>>()
        };
        for session in sessions {
            shutdown_active_session(session, true).await;
        }
    }

    async fn get_or_start_session(&self, thread: &Thread) -> Result<ActiveThreadSession, RpcError> {
        let resolved = self
            .provider_resolver
            .resolve(&thread.provider_instance_id, &thread.environment_id)?;
        let existing = {
            let active = self.active_sessions.lock().await;
            active.get(&thread.id).cloned()
        };
        if let Some(existing) = existing {
            if existing.execution.instance.config == resolved.config {
                return Ok(existing);
            }
            if let Some(stale) =
                remove_active_generation(&self.active_sessions, &thread.id, existing.generation)
                    .await
            {
                shutdown_active_session(stale, false).await;
            }
        }

        let execution = Arc::new(
            self.provider_resolver
                .acquire_execution(&thread.provider_instance_id, &thread.environment_id)
                .await?,
        );
        let resume_token = self
            .store
            .get_resume_token(&thread.id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        let spec = SessionSpec {
            cwd: PathBuf::from(thread.workspace.effective_path()),
            model: thread.model.clone(),
            effort: thread.effort.clone(),
            access: thread.access_mode,
            instructions: None,
            tool_policy: None,
            resume: resume_token,
        };
        let child_capability_token = self.capability_authority.issue_provider_child(
            thread.id.clone(),
            [
                "notification.post".to_string(),
                "memory.list".to_string(),
                "memory.search".to_string(),
                "memory.add".to_string(),
                "memory.update".to_string(),
            ],
            24 * 60 * 60 * 1000,
        );
        let mut launch_config = execution.instance.config.clone();
        launch_config.env_overrides.insert(
            "PANDAMUX_CAPABILITY".to_string(),
            child_capability_token.clone(),
        );
        launch_config.env_overrides.insert(
            "PANDAMUX_THREAD_ID".to_string(),
            thread.id.as_str().to_string(),
        );
        if let Some(endpoint) = self.capability_authority.endpoint() {
            launch_config
                .env_overrides
                .insert("PANDAMUX_PIPE".to_string(), endpoint);
        }
        let start_result = execution
            .instance
            .driver
            .start_session(&launch_config, spec)
            .await;
        let mut session = match start_result {
            Ok(session) => session,
            Err(error) => {
                self.capability_authority.revoke(&child_capability_token);
                return Err(RpcError::internal_error(error.to_string()));
            }
        };
        let events_rx = session.take_event_receiver();
        let candidate = ActiveThreadSession {
            session: Arc::new(Mutex::new(Some(session))),
            events_rx: Arc::new(Mutex::new(events_rx)),
            event_writer: Arc::new(Mutex::new(())),
            execution,
            generation: uuid::Uuid::new_v4(),
            child_capability_token,
            capability_authority: self.capability_authority.clone(),
        };

        let raced = {
            let mut active = self.active_sessions.lock().await;
            if let Some(existing) = active.get(&thread.id) {
                Some(existing.clone())
            } else {
                active.insert(thread.id.clone(), candidate.clone());
                None
            }
        };
        if let Some(existing) = raced {
            shutdown_active_session(candidate, false).await;
            Ok(existing)
        } else {
            Ok(candidate)
        }
    }

    /// Creates and persists a new thread with optional git worktree.
    pub fn create_thread(&self, params: ThreadCreateParams) -> Result<Thread, RpcError> {
        self.create_thread_with_policy(params, true)
    }

    /// Creates a thread after an authenticated Hub has already applied the
    /// canonical organization policy and pinned the provider instance.
    pub fn create_thread_from_hub(&self, params: ThreadCreateParams) -> Result<Thread, RpcError> {
        self.create_thread_with_policy(params, false)
    }

    fn create_thread_with_policy(
        &self,
        params: ThreadCreateParams,
        enforce_organization_policy: bool,
    ) -> Result<Thread, RpcError> {
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

        let environment_id = params
            .environment_id
            .unwrap_or_else(|| EnvironmentId::from(self.environment_id.as_str()));
        let provider_instance_id = if enforce_organization_policy
            && let Some(project_id) = params.project_id.as_ref()
        {
            let policy = self
                .store
                .organization_policy_for_project(project_id)
                .map_err(|error| organization_policy_error(project_id, error.to_string()))?;
            match params.provider_instance_id {
                Some(instance_id) => {
                    self.store
                        .authorize_provider_for_project(project_id, &environment_id, &instance_id)
                        .map_err(|error| {
                            organization_policy_error(project_id, error.to_string())
                        })?;
                    instance_id
                }
                None => {
                    let mut selected = None;
                    for instance_id in &policy.allowed_provider_instances {
                        if self
                            .store
                            .authorize_provider_for_project(
                                project_id,
                                &environment_id,
                                instance_id,
                            )
                            .is_ok()
                            && self
                                .provider_resolver
                                .resolve(instance_id, &environment_id)
                                .is_ok()
                        {
                            selected = Some(instance_id.clone());
                            break;
                        }
                    }
                    selected.ok_or_else(|| {
                        organization_policy_error(
                            project_id,
                            "organization has no enabled provider with an allowed named subscription",
                        )
                    })?
                }
            }
        } else {
            match params.provider_instance_id {
                Some(instance_id) => instance_id,
                None => self
                    .provider_resolver
                    .default_instance_id(&environment_id)?,
            }
        };
        self.provider_resolver
            .resolve(&provider_instance_id, &environment_id)?;

        let thread = Thread {
            id: ThreadId::generate(),
            project_id: params.project_id,
            environment_id,
            parent_thread_id: None,
            title: params.title.unwrap_or_else(|| "New Thread".to_string()),
            provider_instance_id,
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
            .ok_or_else(|| {
                RpcError::not_found(format!("Thread not found: {}", params.thread_id))
            })?;

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
                if let Some(ref p) = params.project_id
                    && t.project_id.as_ref() != Some(p)
                {
                    return false;
                }
                if let Some(st) = params.status
                    && t.status != st
                {
                    return false;
                }
                if let Some(ref parent) = params.parent_thread_id
                    && t.parent_thread_id.as_ref() != Some(parent)
                {
                    return false;
                }
                true
            })
            .collect();

        Ok(filtered)
    }

    /// Dispatches a prompt turn to an agent provider and streams events into SQLite.
    pub async fn send_turn(
        &self,
        params: ThreadSendTurnParams,
    ) -> Result<ThreadSendTurnResult, RpcError> {
        self.send_turn_with_policy(params, true).await
    }

    /// Dispatches a turn after an authenticated Hub has revalidated the
    /// mirrored thread against its canonical organization policy.
    pub async fn send_turn_from_hub(
        &self,
        params: ThreadSendTurnParams,
    ) -> Result<ThreadSendTurnResult, RpcError> {
        self.send_turn_with_policy(params, false).await
    }

    async fn send_turn_with_policy(
        &self,
        params: ThreadSendTurnParams,
        enforce_organization_policy: bool,
    ) -> Result<ThreadSendTurnResult, RpcError> {
        let mut thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| {
                RpcError::not_found(format!("Thread not found: {}", params.thread_id))
            })?;

        if thread.status == ThreadStatus::Working {
            return Err(RpcError::invalid_params(
                "Thread is currently executing another turn",
            ));
        }
        if thread.status == ThreadStatus::AwaitingApproval {
            return Err(RpcError::invalid_params(
                "Thread is awaiting approval response",
            ));
        }
        if enforce_organization_policy && let Some(project_id) = thread.project_id.as_ref() {
            self.store
                .authorize_provider_for_project(
                    project_id,
                    &thread.environment_id,
                    &thread.provider_instance_id,
                )
                .map_err(|error| organization_policy_error(project_id, error.to_string()))?;
        }

        let turn_id = TurnId::generate();
        let base_seq = self
            .store
            .get_latest_seq(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let turn_seq = base_seq + 1;
        let started_ms = now_ms();

        // Resolve and validate attachments if provided
        let resolved_attachments = if !params.attachment_ids.is_empty() {
            let records = self
                .store
                .get_attachments_by_ids(&params.thread_id, &params.attachment_ids)
                .map_err(|e| RpcError::internal_error(e.to_string()))?;

            for id in &params.attachment_ids {
                if !records.iter().any(|r| &r.id == id) {
                    return Err(RpcError::invalid_params(format!(
                        "Attachment '{id}' not found for thread '{}'",
                        params.thread_id.as_str()
                    )));
                }
            }

            pandamux_core::validate_total_attachments_size(&records)
                .map_err(|e| RpcError::invalid_params(e.to_string()))?;

            records
        } else {
            Vec::new()
        };

        let turn_input = TurnInput {
            text: params.text.clone(),
            attachment_ids: params.attachment_ids.clone(),
            attachments: resolved_attachments,
            model: params.model.clone().or_else(|| Some(thread.model.clone())),
            effort: params.effort.clone().or_else(|| thread.effort.clone()),
        };

        let effective_path = PathBuf::from(thread.workspace.effective_path());
        let cp_before = crate::checkpoint::create_checkpoint(
            &effective_path,
            &params.thread_id,
            &turn_id,
            crate::checkpoint::CheckpointPhase::Before,
        )
        .unwrap_or(None);

        let active_sess = self.get_or_start_session(&thread).await?;

        let turn = Turn {
            id: turn_id.clone(),
            thread_id: params.thread_id.clone(),
            environment_id: thread.environment_id.clone(),
            provider_instance_id: thread.provider_instance_id.clone(),
            seq: turn_seq,
            input: turn_input.clone(),
            status: TurnStatus::Running,
            started_at_ms: started_ms,
            ended_at_ms: None,
            usage: None,
            checkpoint_before: cp_before,
            checkpoint_after: None,
        };

        self.store
            .save_turn(&turn)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;
        let execution_config = &active_sess.execution.instance.config;
        self.store
            .record_provider_execution(&ProviderExecutionAudit {
                turn_id: turn_id.clone(),
                thread_id: params.thread_id.clone(),
                environment_id: thread.environment_id.clone(),
                provider_instance_id: execution_config.id.clone(),
                provider_kind: execution_config.provider,
                profile_dir: execution_config.profile_dir.clone(),
                binary_path: execution_config.binary_path.clone(),
                capabilities: active_sess.execution.instance.capabilities,
                model: turn_input
                    .model
                    .clone()
                    .unwrap_or_else(|| thread.model.clone()),
                created_at_ms: started_ms,
            })
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

        // Send turn prompt into provider session
        let send_result = {
            let mut sess = active_sess.session.lock().await;
            sess.as_mut()
                .ok_or_else(|| RpcError::internal_error("Provider session is closed"))?
                .send_turn(turn_input)
                .await
                .map_err(|e| RpcError::internal_error(e.to_string()))
        };
        if let Err(error) = send_result {
            if let Some(active) = remove_active_generation(
                &self.active_sessions,
                &params.thread_id,
                active_sess.generation,
            )
            .await
            {
                shutdown_active_session(active, false).await;
            }
            let failed_at = now_ms();
            if let Ok(Some(mut failed_turn)) = self.store.get_turn(&turn_id) {
                failed_turn.status = TurnStatus::Failed;
                failed_turn.ended_at_ms = Some(failed_at);
                let _ = self.store.save_turn(&failed_turn);
            }
            thread.status = ThreadStatus::Errored;
            thread.updated_at_ms = failed_at;
            let _ = self.store.save_thread(&thread);
            return Err(error);
        }

        // Take receiver for lock-free streaming in background task
        let rx = {
            let mut rx_guard = active_sess.events_rx.lock().await;
            rx_guard.take()
        }
        .ok_or_else(|| RpcError::internal_error("Provider event stream is already active"))?;

        // Spawn background task to stream events into SQLite and broadcast
        let store = self.store.clone();
        let broadcaster = self.broadcaster.clone();
        let thread_id = params.thread_id.clone();
        let env_id = self.environment_id.clone();
        let session_clone = active_sess.session.clone();
        let events_rx_holder = active_sess.events_rx.clone();
        let active_sessions = self.active_sessions.clone();
        let generation = active_sess.generation;
        let turn_id_for_task = turn_id.clone();
        let event_writer = active_sess.event_writer.clone();
        let usage_scanner = self.usage_scanner.clone();
        let provider_kind = active_sess.execution.instance.config.provider;
        let provider_instance_id = active_sess.execution.instance.config.id.clone();
        let provider_version = active_sess.execution.snapshot.version.clone();

        tokio::spawn(async move {
            let turn_id = turn_id_for_task;
            let mut rx = rx;
            let mut terminal_outcome = None;
            while let Some(prov_event) = rx.recv().await {
                let _event_guard = event_writer.lock().await;
                let still_active = {
                    let active = active_sessions.lock().await;
                    active
                        .get(&thread_id)
                        .is_some_and(|session| session.generation == generation)
                };
                if !still_active {
                    break;
                }
                let mut stream_seq = match store
                    .get_latest_seq(&thread_id)
                    .ok()
                    .and_then(|seq| seq.checked_add(1))
                {
                    Some(seq) => seq,
                    None => break,
                };
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
                        if store.append_event(&ev).is_err() {
                            break;
                        }
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
                        if store.append_event(&ev).is_err() {
                            break;
                        }
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
                        if store.append_event(&ev).is_err() {
                            break;
                        }
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
                        if store.append_event(&ev).is_err() {
                            break;
                        }
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
                        if store.append_event(&ev).is_err() {
                            break;
                        }
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
                        if store.append_event(&ev).is_err() {
                            break;
                        }
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
                        let res_tok = session_clone
                            .lock()
                            .await
                            .as_ref()
                            .and_then(|session| session.resume_token());
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
                        if let Some(scanner) = usage_scanner
                            .read()
                            .expect("usage scanner handle lock")
                            .as_ref()
                        {
                            let _ = scanner.trigger(UsageScanTrigger::TurnCompleted);
                        }

                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::TurnCompleted { outcome },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });

                        let worktree_dir = store
                            .get_thread(&thread_id)
                            .ok()
                            .flatten()
                            .map(|th| PathBuf::from(th.workspace.effective_path()))
                            .unwrap_or_default();

                        let cp_after = crate::checkpoint::create_checkpoint(
                            &worktree_dir,
                            &thread_id,
                            &turn_id,
                            crate::checkpoint::CheckpointPhase::After,
                        )
                        .unwrap_or(None);

                        let mut changed_files = Vec::new();
                        if let Ok(Some(mut tr)) = store.get_turn(&turn_id) {
                            tr.checkpoint_after = cp_after.clone();
                            if let (Some(before), Some(after)) = (&tr.checkpoint_before, &cp_after)
                                && let Ok(stats) = crate::checkpoint::compute_changed_files(
                                    &worktree_dir,
                                    before,
                                    after,
                                )
                            {
                                changed_files = stats
                                    .iter()
                                    .map(|s| {
                                        if s.additions > 0 || s.deletions > 0 {
                                            format!(
                                                "{} (+{} -{})",
                                                s.path, s.additions, s.deletions
                                            )
                                        } else {
                                            s.path.clone()
                                        }
                                    })
                                    .collect();
                            }
                            let _ = store.save_turn(&tr);
                        }

                        let _ =
                            crate::checkpoint::prune_old_checkpoints(&worktree_dir, &thread_id, 50);

                        let Some(settle_seq) = stream_seq.checked_add(1) else {
                            break;
                        };
                        stream_seq = settle_seq;
                        let settle = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::TurnSettled { changed_files },
                        };
                        if store.append_event(&settle).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: settle.kind,
                        });

                        terminal_outcome = Some(outcome);
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
                        if store.append_event(&ev).is_err() {
                            break;
                        }
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
                        if provider_kind == ProviderKind::Codex {
                            let environment_id = EnvironmentId::new(env_id.clone());
                            let _ = record_local_codex_capacity(
                                &store,
                                &environment_id,
                                &provider_instance_id,
                                provider_version.as_deref(),
                                &details,
                                at,
                            );
                        }
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::RateLimitObserved {
                                provider: "agent".to_string(),
                                details,
                            },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
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
                        if store.append_event(&ev).is_err() {
                            break;
                        }
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

            let finished_at = now_ms();
            match terminal_outcome {
                Some(TurnOutcome::Success) => {
                    let still_active = {
                        let active = active_sessions.lock().await;
                        active
                            .get(&thread_id)
                            .is_some_and(|session| session.generation == generation)
                    };
                    if still_active {
                        let mut rx_guard = events_rx_holder.lock().await;
                        *rx_guard = Some(rx);
                    }
                    if let Ok(Some(mut thread)) = store.get_thread(&thread_id) {
                        thread.status = ThreadStatus::Idle;
                        thread.updated_at_ms = finished_at;
                        let _ = store.save_thread(&thread);
                    }
                }
                Some(TurnOutcome::Cancelled | TurnOutcome::Interrupted) => {
                    if let Some(active) =
                        remove_active_generation(&active_sessions, &thread_id, generation).await
                    {
                        shutdown_active_session(active, false).await;
                    }
                    if let Ok(Some(mut thread)) = store.get_thread(&thread_id) {
                        thread.status = ThreadStatus::Idle;
                        thread.updated_at_ms = finished_at;
                        let _ = store.save_thread(&thread);
                    }
                }
                Some(TurnOutcome::Failed) => {
                    if let Some(active) =
                        remove_active_generation(&active_sessions, &thread_id, generation).await
                    {
                        shutdown_active_session(active, false).await;
                    }
                    if let Ok(Some(mut thread)) = store.get_thread(&thread_id) {
                        thread.status = ThreadStatus::Errored;
                        thread.updated_at_ms = finished_at;
                        let _ = store.save_thread(&thread);
                    }
                }
                None => {
                    let still_active = {
                        let active = active_sessions.lock().await;
                        active
                            .get(&thread_id)
                            .is_some_and(|session| session.generation == generation)
                    };
                    if !still_active {
                        return;
                    }

                    let worktree_dir = store
                        .get_thread(&thread_id)
                        .ok()
                        .flatten()
                        .map(|thread| PathBuf::from(thread.workspace.effective_path()))
                        .unwrap_or_default();
                    let checkpoint_after = crate::checkpoint::create_checkpoint(
                        &worktree_dir,
                        &thread_id,
                        &turn_id,
                        crate::checkpoint::CheckpointPhase::After,
                    )
                    .unwrap_or(None);
                    let mut changed_files = Vec::new();
                    if let Ok(Some(mut turn)) = store.get_turn(&turn_id) {
                        turn.checkpoint_after = checkpoint_after.clone();
                        if let (Some(before), Some(after)) =
                            (&turn.checkpoint_before, &checkpoint_after)
                            && let Ok(stats) = crate::checkpoint::compute_changed_files(
                                &worktree_dir,
                                before,
                                after,
                            )
                        {
                            changed_files = stats
                                .iter()
                                .map(|stat| {
                                    if stat.additions > 0 || stat.deletions > 0 {
                                        format!(
                                            "{} (+{} -{})",
                                            stat.path, stat.additions, stat.deletions
                                        )
                                    } else {
                                        stat.path.clone()
                                    }
                                })
                                .collect();
                        }
                        turn.status = TurnStatus::Failed;
                        turn.ended_at_ms = Some(finished_at);
                        let _ = store.save_turn(&turn);
                    }
                    if let Ok(Some(mut thread)) = store.get_thread(&thread_id) {
                        thread.status = ThreadStatus::Errored;
                        thread.updated_at_ms = finished_at;
                        let _ = store.save_thread(&thread);
                    }
                    let _ = crate::checkpoint::prune_old_checkpoints(&worktree_dir, &thread_id, 50);

                    let _event_guard = event_writer.lock().await;
                    if let Some(error_seq) = store
                        .get_latest_seq(&thread_id)
                        .ok()
                        .and_then(|seq| seq.checked_add(1))
                    {
                        let event = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: error_seq,
                            at_ms: finished_at,
                            kind: ThreadEventKind::Error {
                                class: "provider_process".to_string(),
                                message: "Provider event stream closed before turn completion"
                                    .to_string(),
                                recoverable: false,
                            },
                        };
                        if store.append_event(&event).is_ok() {
                            let _ = broadcaster.send(EventEnvelope {
                                subscription_id: String::new(),
                                environment_id: env_id.clone().into(),
                                thread_id: Some(thread_id.clone()),
                                run_id: None,
                                seq: error_seq,
                                at_ms: finished_at,
                                kind: event.kind,
                            });

                            if let Some(completed_seq) = error_seq.checked_add(1) {
                                let completed = ThreadEvent {
                                    thread_id: thread_id.clone(),
                                    seq: completed_seq,
                                    at_ms: finished_at,
                                    kind: ThreadEventKind::TurnCompleted {
                                        outcome: TurnOutcome::Failed,
                                    },
                                };
                                if store.append_event(&completed).is_ok() {
                                    let _ = broadcaster.send(EventEnvelope {
                                        subscription_id: String::new(),
                                        environment_id: env_id.clone().into(),
                                        thread_id: Some(thread_id.clone()),
                                        run_id: None,
                                        seq: completed_seq,
                                        at_ms: finished_at,
                                        kind: completed.kind,
                                    });

                                    if let Some(settled_seq) = completed_seq.checked_add(1) {
                                        let settled = ThreadEvent {
                                            thread_id: thread_id.clone(),
                                            seq: settled_seq,
                                            at_ms: finished_at,
                                            kind: ThreadEventKind::TurnSettled { changed_files },
                                        };
                                        if store.append_event(&settled).is_ok() {
                                            let _ = broadcaster.send(EventEnvelope {
                                                subscription_id: String::new(),
                                                environment_id: env_id.into(),
                                                thread_id: Some(thread_id.clone()),
                                                run_id: None,
                                                seq: settled_seq,
                                                at_ms: finished_at,
                                                kind: settled.kind,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(active) =
                        remove_active_generation(&active_sessions, &thread_id, generation).await
                    {
                        shutdown_active_session(active, false).await;
                    }
                }
            }
        });

        Ok(ThreadSendTurnResult {
            turn_id,
            seq: started_seq,
        })
    }

    /// Responds to a pending approval request and resumes the turn.
    pub async fn respond_approval(
        &self,
        params: ThreadRespondApprovalParams,
    ) -> Result<(), RpcError> {
        let session_opt = {
            let active = self.active_sessions.lock().await;
            active.get(&params.thread_id).cloned()
        };

        let active_sess = session_opt.ok_or_else(|| {
            RpcError::invalid_params(format!(
                "No active session found for thread {}",
                params.thread_id
            ))
        })?;

        let _event_guard = active_sess.event_writer.lock().await;

        {
            let mut sess = active_sess.session.lock().await;
            sess.as_mut()
                .ok_or_else(|| RpcError::internal_error("Provider session is closed"))?
                .respond_approval(params.request_id.clone(), params.decision)
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
        let session = {
            let mut active = self.active_sessions.lock().await;
            active.remove(&params.thread_id)
        };

        let event_writer = session
            .as_ref()
            .map(|active_session| active_session.event_writer.clone());
        let _event_guard = if let Some(event_writer) = event_writer.as_ref() {
            Some(event_writer.lock().await)
        } else {
            None
        };

        if let Some(active_session) = session {
            shutdown_active_session(active_session, true).await;
        }

        let now = now_ms();
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        if let Some(mut th) = thread.clone() {
            th.status = ThreadStatus::Idle;
            th.updated_at_ms = now;
            self.store
                .save_thread(&th)
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
        }

        let mut interrupted = false;
        let mut changed_files = Vec::new();
        let worktree_dir = thread
            .as_ref()
            .map(|thread| PathBuf::from(thread.workspace.effective_path()));
        let turns = self
            .store
            .list_turns(&params.thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        for mut turn in turns {
            if matches!(
                turn.status,
                TurnStatus::Running | TurnStatus::AwaitingApproval | TurnStatus::Requested
            ) {
                if let Some(worktree_dir) = worktree_dir.as_ref() {
                    let checkpoint_after = crate::checkpoint::create_checkpoint(
                        worktree_dir,
                        &params.thread_id,
                        &turn.id,
                        crate::checkpoint::CheckpointPhase::After,
                    )
                    .unwrap_or(None);
                    if let (Some(before), Some(after)) =
                        (&turn.checkpoint_before, &checkpoint_after)
                        && let Ok(stats) =
                            crate::checkpoint::compute_changed_files(worktree_dir, before, after)
                    {
                        changed_files = stats
                            .iter()
                            .map(|stat| {
                                if stat.additions > 0 || stat.deletions > 0 {
                                    format!(
                                        "{} (+{} -{})",
                                        stat.path, stat.additions, stat.deletions
                                    )
                                } else {
                                    stat.path.clone()
                                }
                            })
                            .collect();
                    }
                    turn.checkpoint_after = checkpoint_after;
                }
                turn.status = TurnStatus::Interrupted;
                turn.ended_at_ms = Some(now);
                self.store
                    .save_turn(&turn)
                    .map_err(|error| RpcError::internal_error(error.to_string()))?;
                interrupted = true;
            }
        }

        if interrupted {
            let completed_seq = self
                .store
                .get_latest_seq(&params.thread_id)
                .map_err(|error| RpcError::internal_error(error.to_string()))?
                .checked_add(1)
                .ok_or_else(|| RpcError::internal_error("Thread event sequence overflow"))?;
            let completed = ThreadEvent {
                thread_id: params.thread_id.clone(),
                seq: completed_seq,
                at_ms: now,
                kind: ThreadEventKind::TurnCompleted {
                    outcome: TurnOutcome::Interrupted,
                },
            };
            self.store
                .append_event(&completed)
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
            let _ = self.broadcaster.send(EventEnvelope {
                subscription_id: String::new(),
                environment_id: self.environment_id.clone().into(),
                thread_id: Some(params.thread_id.clone()),
                run_id: None,
                seq: completed_seq,
                at_ms: now,
                kind: completed.kind,
            });

            let settled_seq = completed_seq
                .checked_add(1)
                .ok_or_else(|| RpcError::internal_error("Thread event sequence overflow"))?;
            let settled = ThreadEvent {
                thread_id: params.thread_id.clone(),
                seq: settled_seq,
                at_ms: now,
                kind: ThreadEventKind::TurnSettled { changed_files },
            };
            self.store
                .append_event(&settled)
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
            let _ = self.broadcaster.send(EventEnvelope {
                subscription_id: String::new(),
                environment_id: self.environment_id.clone().into(),
                thread_id: Some(params.thread_id.clone()),
                run_id: None,
                seq: settled_seq,
                at_ms: now,
                kind: settled.kind,
            });
        }

        Ok(())
    }

    /// Queries the saved resumption token for continuing a thread across restarts.
    pub fn resume_thread(
        &self,
        params: ThreadResumeParams,
    ) -> Result<ThreadResumeResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| {
                RpcError::not_found(format!("Thread not found: {}", params.thread_id))
            })?;

        let resume_token = self
            .store
            .get_resume_token(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        Ok(ThreadResumeResult {
            thread_id: thread.id,
            resume_token,
        })
    }

    /// Lists checkpoint refs for a thread.
    pub fn list_checkpoints(&self, thread_id: &ThreadId) -> Result<Vec<String>, RpcError> {
        let thread = self
            .store
            .get_thread(thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {thread_id} not found")))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        if !crate::checkpoint::is_git_repository(&worktree_dir) {
            return Ok(Vec::new());
        }

        let prefix = format!("refs/pandamux/checkpoints/{}/", thread_id.as_str());
        let list_out = std::process::Command::new("git")
            .arg("-C")
            .arg(&worktree_dir)
            .arg("for-each-ref")
            .arg("--format=%(refname)")
            .arg(&prefix)
            .output()
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        if !list_out.status.success() {
            return Ok(Vec::new());
        }

        let refs_str = String::from_utf8_lossy(&list_out.stdout);
        let refs: Vec<String> = refs_str
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(refs)
    }

    /// Computes diff stats between two checkpoint refs in a thread.
    pub fn diff_checkpoints(
        &self,
        thread_id: &ThreadId,
        before_ref: &str,
        after_ref: &str,
    ) -> Result<Vec<crate::checkpoint::ChangedFileStat>, RpcError> {
        let thread = self
            .store
            .get_thread(thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {thread_id} not found")))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::checkpoint::compute_changed_files(&worktree_dir, before_ref, after_ref)
            .map_err(RpcError::internal_error)
    }

    /// Computes unified diff patch between two checkpoint refs in a thread.
    pub fn diff_checkpoints_patch(
        &self,
        thread_id: &ThreadId,
        before_ref: &str,
        after_ref: &str,
        file_path: Option<&str>,
    ) -> Result<String, RpcError> {
        let thread = self
            .store
            .get_thread(thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {thread_id} not found")))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::checkpoint::compute_checkpoint_patch(&worktree_dir, before_ref, after_ref, file_path)
            .map_err(RpcError::internal_error)
    }

    /// Rolls back a thread's worktree to the specified checkpoint ref.
    pub fn rollback_checkpoint(
        &self,
        thread_id: &ThreadId,
        checkpoint_ref: &str,
    ) -> Result<(), RpcError> {
        let thread = self
            .store
            .get_thread(thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {thread_id} not found")))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::checkpoint::rollback_to_checkpoint(&worktree_dir, checkpoint_ref)
            .map_err(RpcError::internal_error)
    }

    /// Queries the Git status and file modifications for a thread's worktree.
    pub fn git_status(&self, params: GitStatusParams) -> Result<GitStatusResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {} not found", params.thread_id)))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::git::get_git_status(&worktree_dir).map_err(RpcError::internal_error)
    }

    /// Stages all changes and creates a Git commit in the thread's worktree.
    pub fn git_commit(&self, params: GitCommitParams) -> Result<GitCommitResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {} not found", params.thread_id)))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::git::git_commit(&worktree_dir, params.message).map_err(RpcError::internal_error)
    }

    /// Pushes committed changes in the thread's worktree to the remote repository.
    pub fn git_push(&self, params: GitPushParams) -> Result<GitPushResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {} not found", params.thread_id)))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::git::git_push(
            &worktree_dir,
            params.remote.as_deref(),
            params.branch.as_deref(),
        )
        .map_err(RpcError::internal_error)
    }

    /// Creates a Pull Request using gh CLI if installed, or generates a compare URL.
    pub fn git_create_pr(&self, params: GitCreatePrParams) -> Result<GitCreatePrResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {} not found", params.thread_id)))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::git::git_create_pr(
            &worktree_dir,
            params.title.as_deref(),
            params.body.as_deref(),
        )
        .map_err(RpcError::internal_error)
    }
}
