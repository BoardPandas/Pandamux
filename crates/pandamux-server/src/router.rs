use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use pandamux_core::{ThreadEventKind, UserSettings};
use pandamux_protocol::{
    AttachmentGetParams, AttachmentImportPathParams, AttachmentListParams,
    AttachmentPutChunkParams, CheckpointDiffParams, CheckpointListParams, CheckpointRollbackParams,
    GitCommitParams, GitCreatePrParams, GitPushParams, GitStatusParams, HelloParams, HelloResult,
    IdentifyResult, McpCallToolParams, PROTOCOL_VERSION, PingResult, ProviderHealthParams,
    ProviderHealthReport, ProviderHealthResult, RpcError, RpcRequest, RpcResponse,
    ServerCapabilities, ServerRole, SettingsGetParams, SettingsGetResult, SettingsSetParams,
    SubAgentTreeItem, SubAgentTreeParams, SubAgentTreeResult, ThreadCancelTurnParams,
    ThreadCreateParams, ThreadGetParams, ThreadListParams, ThreadRespondApprovalParams,
    ThreadResumeParams, ThreadSendTurnParams,
};

use crate::attachment::AttachmentManager;
use crate::driver_registry::DriverRegistry;
use crate::mcp_server::McpServer;
use crate::store::Store;
use crate::thread_manager::ThreadManager;

pub struct Router {
    store: Store,
    mcp: McpServer,
    thread_manager: ThreadManager,
    attachments: AttachmentManager,
    notifications: Arc<tokio::sync::Mutex<Vec<pandamux_core::notification::NotificationInfo>>>,
    drivers: Arc<DriverRegistry>,
    role: ServerRole,
    environment_id: String,
    server_version: String,
}

fn default_attachments_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PANDAMUX_ATTACHMENTS_DIR") {
        PathBuf::from(dir)
    } else if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_app_data)
            .join("pandamux")
            .join("attachments")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".pandamux")
            .join("data")
            .join("attachments")
    } else {
        std::env::temp_dir().join("pandamux").join("attachments")
    }
}

fn account_badge_label(label: Option<&str>, sub: Option<&str>) -> Option<String> {
    match (label, sub) {
        (Some(l), Some(s)) if !l.is_empty() && !s.is_empty() && l != s => {
            Some(format!("{l} ({s})"))
        }
        (Some(l), _) if !l.is_empty() => Some(l.to_string()),
        (_, Some(s)) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    }
}

impl Router {
    pub fn new(store: Store, role: ServerRole, environment_id: String) -> Self {
        let drivers = Arc::new(DriverRegistry::default_local());
        Self::with_drivers(store, role, environment_id, drivers)
    }

    pub fn with_drivers(
        store: Store,
        role: ServerRole,
        environment_id: String,
        drivers: Arc<DriverRegistry>,
    ) -> Self {
        let mcp = McpServer::new(store.clone());
        let thread_manager =
            ThreadManager::new(store.clone(), drivers.clone(), environment_id.clone());
        let attachments = AttachmentManager::new(default_attachments_dir());
        Self {
            store,
            mcp,
            thread_manager,
            attachments,
            notifications: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            drivers,
            role,
            environment_id,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn with_attachments(mut self, attachments: AttachmentManager) -> Self {
        self.attachments = attachments;
        self
    }

    pub fn attachments(&self) -> &AttachmentManager {
        &self.attachments
    }

    pub fn thread_manager(&self) -> &ThreadManager {
        &self.thread_manager
    }

    pub fn drivers(&self) -> &Arc<DriverRegistry> {
        &self.drivers
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Handle a single parsed JSON-RPC request and return an optional response.
    pub async fn handle_request(&self, req: RpcRequest) -> Option<RpcResponse> {
        let is_notification = req.is_notification();
        let id = req.id;
        let method = req.method;
        let params = req.params.unwrap_or(Value::Null);

        let result = match method.as_str() {
            "system.ping" => self.handle_ping(),
            "system.hello" => self.handle_hello(params),
            "system.identify" => self.handle_identify(),
            "system.capabilities" => self.handle_capabilities(),
            "thread.create" => self.handle_thread_create(params),
            "thread.get" => self.handle_thread_get(params),
            "thread.list" => self.handle_thread_list(params),
            "thread.send_turn" | "thread.sendTurn" => self.handle_thread_send_turn(params).await,
            "thread.cancel_turn" | "thread.interrupt" => {
                self.handle_thread_cancel_turn(params).await
            }
            "thread.respond_approval" | "thread.respondApproval" => {
                self.handle_thread_respond_approval(params).await
            }
            "thread.resume" => self.handle_thread_resume(params),
            "settings.get" | "config.get" => self.handle_settings_get(params),
            "settings.set" | "config.set" => self.handle_settings_set(params),
            "provider.health" | "provider.probe" => self.handle_provider_health(params).await,
            "mcp.list_tools" => self.handle_mcp_list_tools(),
            "mcp.call_tool" => self.handle_mcp_call_tool(params),
            "notification.post" => self.handle_notification_post(params).await,
            "notification.list" => self.handle_notification_list().await,
            "notification.clear" => self.handle_notification_clear(params).await,
            "checkpoint.list" => self.handle_checkpoint_list(params),
            "checkpoint.diff" => self.handle_checkpoint_diff(params),
            "checkpoint.rollback" => self.handle_checkpoint_rollback(params),
            "git.status" => self.handle_git_status(params),
            "git.commit" => self.handle_git_commit(params),
            "git.push" => self.handle_git_push(params),
            "git.create_pr" | "git.createPr" => self.handle_git_create_pr(params),
            "attachment.import_path" | "attachment.importPath" => {
                self.handle_attachment_import_path(params)
            }
            "attachment.put" => self.handle_attachment_put(params).await,
            "attachment.list" => self.handle_attachment_list(params),
            "attachment.get" => self.handle_attachment_get(params),
            "subagent.tree" | "subagent.list" => self.handle_subagent_tree(params),
            "fs.read" => self.handle_fs_read(params),
            "fs.list" => self.handle_fs_list(params),
            unknown => Err(RpcError::method_not_found(unknown)),
        };

        if is_notification {
            return None;
        }

        Some(match result {
            Ok(val) => RpcResponse::success(id.unwrap_or(1.into()), val),
            Err(err) => RpcResponse::error(id, err),
        })
    }

    fn handle_ping(&self) -> Result<Value, RpcError> {
        let ping = PingResult {
            pong: true,
            timestamp_ms: Self::now_ms(),
        };
        serde_json::to_value(ping).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_hello(&self, params: Value) -> Result<Value, RpcError> {
        let hello: HelloParams = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(format!("Invalid hello params: {e}")))?;

        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(RpcError::invalid_params(format!(
                "Protocol version mismatch: client requested {}, server supports {}",
                hello.protocol_version, PROTOCOL_VERSION
            )));
        }

        let result = HelloResult {
            server_version: self.server_version.clone(),
            protocol_version: PROTOCOL_VERSION,
            role: self.role,
            capabilities: ServerCapabilities {
                streaming: true,
                attachments: true,
                worktrees: true,
                mcp: true,
                schedules: true,
            },
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_identify(&self) -> Result<Value, RpcError> {
        let result = IdentifyResult {
            server_version: self.server_version.clone(),
            protocol_version: PROTOCOL_VERSION,
            role: self.role,
            platform: std::env::consts::OS.to_string(),
            environment_id: self.environment_id.clone(),
        };
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_capabilities(&self) -> Result<Value, RpcError> {
        let caps = ServerCapabilities {
            streaming: true,
            attachments: true,
            worktrees: true,
            mcp: true,
            schedules: true,
        };
        serde_json::to_value(caps).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_thread_create(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadCreateParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let thread = self.thread_manager.create_thread(p)?;
        serde_json::to_value(&thread).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_thread_get(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadGetParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.thread_manager.get_thread(p)?;
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_thread_list(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadListParams = if params.is_null() {
            Default::default()
        } else {
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?
        };
        let threads = self.thread_manager.list_threads(p)?;
        serde_json::to_value(threads).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_thread_send_turn(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadSendTurnParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.thread_manager.send_turn(p).await?;
        serde_json::to_value(&result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_thread_cancel_turn(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadCancelTurnParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        self.thread_manager.interrupt_turn(p).await?;
        Ok(json!({ "interrupted": true }))
    }

    async fn handle_thread_respond_approval(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadRespondApprovalParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        self.thread_manager.respond_approval(p).await?;
        Ok(json!({ "acknowledged": true }))
    }

    fn handle_thread_resume(&self, params: Value) -> Result<Value, RpcError> {
        let p: ThreadResumeParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.thread_manager.resume_thread(p)?;
        serde_json::to_value(&result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_git_status(&self, params: Value) -> Result<Value, RpcError> {
        let p: GitStatusParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let res = self.thread_manager.git_status(p)?;
        serde_json::to_value(&res).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_git_commit(&self, params: Value) -> Result<Value, RpcError> {
        let p: GitCommitParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let res = self.thread_manager.git_commit(p)?;
        serde_json::to_value(&res).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_git_push(&self, params: Value) -> Result<Value, RpcError> {
        let p: GitPushParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let res = self.thread_manager.git_push(p)?;
        serde_json::to_value(&res).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_git_create_pr(&self, params: Value) -> Result<Value, RpcError> {
        let p: GitCreatePrParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let res = self.thread_manager.git_create_pr(p)?;
        serde_json::to_value(&res).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_settings_get(&self, params: Value) -> Result<Value, RpcError> {
        let req_params: SettingsGetParams = serde_json::from_value(params).unwrap_or_default();
        let settings = self
            .store
            .get_settings()
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .unwrap_or_default();

        if let Some(key) = req_params.key {
            let val =
                pandamux_core::settings_get(&settings, &key).map_err(RpcError::invalid_params)?;
            let res = SettingsGetResult {
                settings,
                value: Some(val),
            };
            serde_json::to_value(res).map_err(|e| RpcError::internal_error(e.to_string()))
        } else {
            let res = SettingsGetResult {
                settings,
                value: None,
            };
            serde_json::to_value(res).map_err(|e| RpcError::internal_error(e.to_string()))
        }
    }

    fn handle_settings_set(&self, params: Value) -> Result<Value, RpcError> {
        let mut current = self
            .store
            .get_settings()
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .unwrap_or_default();

        // 1. Try parsing as SettingsSetParams { settings, key, value }
        if let Ok(p) = serde_json::from_value::<SettingsSetParams>(params.clone()) {
            if let Some(mut updated) = p.settings {
                updated.normalize();
                self.store
                    .save_settings(&updated)
                    .map_err(|e| RpcError::internal_error(e.to_string()))?;
                return Ok(json!({ "success": true }));
            } else if let (Some(key), Some(value)) = (p.key, p.value) {
                pandamux_core::settings_set(&mut current, &key, value)
                    .map_err(RpcError::invalid_params)?;
                current.normalize();
                self.store
                    .save_settings(&current)
                    .map_err(|e| RpcError::internal_error(e.to_string()))?;
                return Ok(json!({ "success": true }));
            }
        }

        // 2. Try parsing directly as UserSettings
        if let Ok(mut settings) = serde_json::from_value::<UserSettings>(params) {
            settings.normalize();
            self.store
                .save_settings(&settings)
                .map_err(|e| RpcError::internal_error(e.to_string()))?;
            return Ok(json!({ "success": true }));
        }

        Err(RpcError::invalid_params("Invalid settings payload"))
    }

    async fn handle_provider_health(&self, params: Value) -> Result<Value, RpcError> {
        let filter: ProviderHealthParams = serde_json::from_value(params).unwrap_or_default();
        let settings = self
            .store
            .get_settings()
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .unwrap_or_default();

        let mut reports = Vec::new();

        for inst in &settings.providers {
            if let Some(ref target_id) = filter.instance_id
                && &inst.id != target_id
            {
                continue;
            }

            if let Some(driver) = self.drivers.get(inst.provider) {
                let snapshot = driver.probe(inst).await;
                let (status, reason) = match &snapshot.health {
                    pandamux_providers::models::ProviderHealth::Healthy => match &snapshot.auth {
                        pandamux_providers::models::ProviderAuthStatus::Authenticated {
                            ..
                        } => ("healthy".to_string(), None),
                        pandamux_providers::models::ProviderAuthStatus::Unauthenticated => (
                            "unauthenticated".to_string(),
                            Some("Not logged in".to_string()),
                        ),
                        pandamux_providers::models::ProviderAuthStatus::Unknown => {
                            ("healthy".to_string(), None)
                        }
                    },
                    pandamux_providers::models::ProviderHealth::Degraded { reason } => {
                        ("degraded".to_string(), Some(reason.clone()))
                    }
                    pandamux_providers::models::ProviderHealth::Unavailable { reason } => {
                        ("unavailable".to_string(), Some(reason.clone()))
                    }
                };

                let (account_badge, subscription) = match &snapshot.auth {
                    pandamux_providers::models::ProviderAuthStatus::Authenticated {
                        account_label,
                        email: _,
                        subscription,
                    } => (
                        account_badge_label(account_label.as_deref(), subscription.as_deref()),
                        subscription.clone(),
                    ),
                    _ => (None, None),
                };

                reports.push(ProviderHealthReport {
                    instance_id: inst.id.clone(),
                    provider: inst.provider,
                    display_name: inst.display_name.clone(),
                    enabled: inst.enabled,
                    installed: snapshot.installed,
                    version: snapshot.version,
                    status,
                    account_badge,
                    subscription,
                    reason,
                    checked_at_ms: snapshot.checked_at_ms,
                });
            } else {
                reports.push(ProviderHealthReport {
                    instance_id: inst.id.clone(),
                    provider: inst.provider,
                    display_name: inst.display_name.clone(),
                    enabled: inst.enabled,
                    installed: false,
                    version: None,
                    status: "unavailable".to_string(),
                    account_badge: None,
                    subscription: None,
                    reason: Some("No registered driver for provider".to_string()),
                    checked_at_ms: Self::now_ms(),
                });
            }
        }

        let res = ProviderHealthResult { reports };
        serde_json::to_value(&res).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_mcp_list_tools(&self) -> Result<Value, RpcError> {
        let tools = self.mcp.list_tools();
        serde_json::to_value(tools).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_mcp_call_tool(&self, params: Value) -> Result<Value, RpcError> {
        let call_params: McpCallToolParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.mcp.call_tool(&call_params);
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_notification_post(&self, params: Value) -> Result<Value, RpcError> {
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if title.is_empty() {
            return Err(RpcError::invalid_params("Notification title is required"));
        }
        let body = params
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let source_str = params
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("generic");
        let source = match source_str {
            "build" => pandamux_core::notification::NotificationSource::Build,
            "agent" => pandamux_core::notification::NotificationSource::Agent,
            "deploy" => pandamux_core::notification::NotificationSource::Deploy,
            "port" => pandamux_core::notification::NotificationSource::Port,
            _ => pandamux_core::notification::NotificationSource::Generic,
        };

        let id = format!("notif-{}", uuid::Uuid::new_v4().simple());
        let info = pandamux_core::notification::NotificationInfo {
            id: id.clone(),
            workspace_id: None,
            surface_id: None,
            title,
            body,
            source,
            timestamp_ms: Self::now_ms(),
            read: false,
        };

        let mut lock = self.notifications.lock().await;
        lock.push(info);

        Ok(json!({ "id": id, "posted": true }))
    }

    async fn handle_notification_list(&self) -> Result<Value, RpcError> {
        let lock = self.notifications.lock().await;
        serde_json::to_value(&*lock).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_notification_clear(&self, params: Value) -> Result<Value, RpcError> {
        let mut lock = self.notifications.lock().await;
        if let Some(id) = params.get("id").and_then(|v| v.as_str()) {
            lock.retain(|n| n.id != id);
        } else {
            lock.clear();
        }
        Ok(json!({ "cleared": true }))
    }

    fn handle_checkpoint_list(&self, params: Value) -> Result<Value, RpcError> {
        let params: CheckpointListParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let checkpoints = self.thread_manager.list_checkpoints(&params.thread_id)?;
        Ok(json!({ "checkpoints": checkpoints }))
    }

    fn handle_checkpoint_diff(&self, params: Value) -> Result<Value, RpcError> {
        let params: CheckpointDiffParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let stats = self.thread_manager.diff_checkpoints(
            &params.thread_id,
            &params.before_ref,
            &params.after_ref,
        )?;

        let patch = self
            .thread_manager
            .diff_checkpoints_patch(
                &params.thread_id,
                &params.before_ref,
                &params.after_ref,
                params.file_path.as_deref(),
            )
            .ok();

        let formatted: Vec<Value> = stats
            .into_iter()
            .map(|s| {
                json!({
                    "path": s.path,
                    "kind": format!("{:?}", s.kind).to_lowercase(),
                    "additions": s.additions,
                    "deletions": s.deletions,
                })
            })
            .collect();

        Ok(json!({
            "changedFiles": formatted,
            "patch": patch
        }))
    }

    fn handle_checkpoint_rollback(&self, params: Value) -> Result<Value, RpcError> {
        let params: CheckpointRollbackParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        self.thread_manager
            .rollback_checkpoint(&params.thread_id, &params.checkpoint_ref)?;
        Ok(json!({ "ok": true }))
    }

    fn handle_subagent_tree(&self, params: Value) -> Result<Value, RpcError> {
        let params: SubAgentTreeParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let events = self
            .store
            .get_events(&params.thread_id, None)
            .map_err(|e| RpcError::internal_error(format!("Failed to query events: {e}")))?;

        let mut subagents: HashMap<String, SubAgentTreeItem> = HashMap::new();
        for ev in &events {
            match &ev.kind {
                ThreadEventKind::SubAgentSpawned {
                    sub_agent_id,
                    parent_sub_agent_id,
                    parent_item_id,
                    title,
                    agent_type,
                    model,
                    effort,
                } => {
                    subagents.insert(
                        sub_agent_id.clone(),
                        SubAgentTreeItem {
                            id: sub_agent_id.clone(),
                            parent_sub_agent_id: parent_sub_agent_id.clone(),
                            parent_item_id: parent_item_id.clone(),
                            title: title.clone(),
                            agent_type: agent_type.clone(),
                            model: model.clone(),
                            effort: effort.clone(),
                            latest_activity: None,
                            tokens: 0,
                            tool_calls: 0,
                            outcome: None,
                            elapsed_ms: None,
                            summary: None,
                            finished: false,
                            children: Vec::new(),
                        },
                    );
                }
                ThreadEventKind::SubAgentActivity {
                    sub_agent_id,
                    preview,
                } => {
                    if let Some(agent) = subagents.get_mut(sub_agent_id) {
                        agent.latest_activity = Some(preview.clone());
                    }
                }
                ThreadEventKind::SubAgentUsage {
                    sub_agent_id,
                    tokens,
                    tool_calls,
                } => {
                    if let Some(agent) = subagents.get_mut(sub_agent_id) {
                        agent.tokens = *tokens;
                        agent.tool_calls = *tool_calls;
                    }
                }
                ThreadEventKind::SubAgentFinished {
                    sub_agent_id,
                    outcome,
                    elapsed_ms,
                    summary,
                } => {
                    if let Some(agent) = subagents.get_mut(sub_agent_id) {
                        agent.outcome = Some(outcome.clone());
                        agent.elapsed_ms = Some(*elapsed_ms);
                        agent.summary = summary.clone();
                        agent.finished = true;
                    }
                }
                _ => {}
            }
        }

        let total_count = subagents.len();

        let mut by_parent: HashMap<Option<String>, Vec<SubAgentTreeItem>> = HashMap::new();
        for agent in subagents.values() {
            by_parent
                .entry(agent.parent_sub_agent_id.clone())
                .or_default()
                .push(agent.clone());
        }
        for list in by_parent.values_mut() {
            list.sort_by(|a, b| a.id.cmp(&b.id));
        }

        fn build_tree(
            parent_id: Option<&str>,
            by_parent: &HashMap<Option<String>, Vec<SubAgentTreeItem>>,
        ) -> Vec<SubAgentTreeItem> {
            let key = parent_id.map(|s| s.to_string());
            let Some(children) = by_parent.get(&key) else {
                return Vec::new();
            };
            children
                .iter()
                .map(|child| {
                    let mut node = child.clone();
                    node.children = build_tree(Some(&child.id), by_parent);
                    node
                })
                .collect()
        }

        let root_agents = build_tree(None, &by_parent);

        let result = SubAgentTreeResult {
            thread_id: params.thread_id,
            root_agents,
            total_count,
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_fs_read(&self, params: Value) -> Result<Value, RpcError> {
        let p: pandamux_protocol::FsReadParams = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(format!("Invalid fs.read params: {e}")))?;

        let res = crate::fs::read_confined_file(&p.root_path, &p.path)?;
        serde_json::to_value(res).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_fs_list(&self, params: Value) -> Result<Value, RpcError> {
        let p: pandamux_protocol::FsListParams = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(format!("Invalid fs.list params: {e}")))?;

        let res = crate::fs::list_confined_dir(&p.root_path, p.path.as_deref())?;
        serde_json::to_value(res).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_attachment_import_path(&self, params: Value) -> Result<Value, RpcError> {
        let p: AttachmentImportPathParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self
            .attachments
            .import_path(&self.store, &p.thread_id, &p.path)?;
        serde_json::to_value(&result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_attachment_put(&self, params: Value) -> Result<Value, RpcError> {
        let p: AttachmentPutChunkParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.attachments.put_chunk(&self.store, p).await?;
        serde_json::to_value(&result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_attachment_list(&self, params: Value) -> Result<Value, RpcError> {
        let p: AttachmentListParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.attachments.list(&self.store, &p.thread_id)?;
        serde_json::to_value(&result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_attachment_get(&self, params: Value) -> Result<Value, RpcError> {
        let p: AttachmentGetParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self.attachments.get(&self.store, &p.thread_id, &p.id)?;
        serde_json::to_value(&result).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn router_handles_ping_and_capabilities() {
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env-test".to_string());

        let ping_req = RpcRequest::new(1, "system.ping", None);
        let res = router.handle_request(ping_req).await.expect("response");
        assert!(res.is_success());
        assert_eq!(res.result.unwrap()["pong"], true);

        let caps_req = RpcRequest::new(2, "system.capabilities", None);
        let res = router.handle_request(caps_req).await.expect("response");
        assert!(res.is_success());
        assert_eq!(res.result.unwrap()["streaming"], true);
    }

    #[tokio::test]
    async fn router_handles_mcp_discovery_and_call() {
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env-test".to_string());

        let list_req = RpcRequest::new(1, "mcp.list_tools", None);
        let res = router.handle_request(list_req).await.expect("response");
        assert!(res.is_success());
        let tools = res.result.unwrap();
        assert!(tools.as_array().unwrap().len() >= 2);

        let call_req = RpcRequest::new(
            2,
            "mcp.call_tool",
            Some(json!({ "name": "read_settings", "arguments": {} })),
        );
        let res = router.handle_request(call_req).await.expect("response");
        assert!(res.is_success());
    }

    #[tokio::test]
    async fn router_handles_thread_lifecycle_and_turn_streaming() {
        let store = Store::in_memory().unwrap();
        let drivers = Arc::new(DriverRegistry::default_local());
        let router = Router::with_drivers(store, ServerRole::Hub, "env-test".to_string(), drivers);

        // 1. Create thread
        let create_req = RpcRequest::new(
            1,
            "thread.create",
            Some(json!({
                "title": "Test Chat",
                "model": "mock-fast"
            })),
        );
        let res = router
            .handle_request(create_req)
            .await
            .expect("create response");
        assert!(res.is_success());
        let thread_val = res.result.unwrap();
        let thread_id = thread_val["id"].as_str().unwrap().to_string();

        // 2. Subscribe to events
        let mut event_rx = router.thread_manager().subscribe();

        // 3. Send turn
        let send_req = RpcRequest::new(
            2,
            "thread.send_turn",
            Some(json!({
                "threadId": thread_id,
                "text": "Hello PandaMUX"
            })),
        );
        let send_res = router
            .handle_request(send_req)
            .await
            .expect("send response");
        assert!(send_res.is_success());

        // 4. Receive streamed events
        let mut got_turn_requested = false;
        let mut got_assistant_text = false;
        let mut got_turn_settled = false;

        for _ in 0..10 {
            if let Ok(Ok(env)) =
                tokio::time::timeout(std::time::Duration::from_millis(500), event_rx.recv()).await
            {
                match env.kind {
                    pandamux_core::event::ThreadEventKind::TurnRequested { .. } => {
                        got_turn_requested = true;
                    }
                    pandamux_core::event::ThreadEventKind::AssistantText { ref text, .. } => {
                        if text.contains("Hello PandaMUX") {
                            got_assistant_text = true;
                        }
                    }
                    pandamux_core::event::ThreadEventKind::TurnSettled { .. } => {
                        got_turn_settled = true;
                        break;
                    }
                    _ => {}
                }
            }
        }

        assert!(got_turn_requested, "Should receive TurnRequested event");
        assert!(got_assistant_text, "Should receive AssistantText event");
        assert!(got_turn_settled, "Should receive TurnSettled event");

        // 5. Get thread and verify turn history
        let get_req = RpcRequest::new(3, "thread.get", Some(json!({ "threadId": thread_id })));
        let get_res = router.handle_request(get_req).await.expect("get response");
        assert!(get_res.is_success());
        let get_val = get_res.result.unwrap();
        assert_eq!(get_val["thread"]["id"], thread_id);
        assert_eq!(get_val["turns"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn router_handles_attachment_methods() {
        let store = Store::in_memory().unwrap();
        let temp_dir = tempfile::tempdir().unwrap();
        let att_dir = temp_dir.path().join("attachments");
        let attachment_manager = AttachmentManager::new(att_dir);
        let router = Router::new(store, ServerRole::Hub, "env-test".to_string())
            .with_attachments(attachment_manager);

        // Create thread first
        let create_req = RpcRequest::new(
            1,
            "thread.create",
            Some(json!({
                "title": "Attachment Test",
                "cwd": "/tmp"
            })),
        );
        let create_res = router.handle_request(create_req).await.unwrap();
        assert!(create_res.is_success());
        let thread_val = create_res.result.unwrap();
        let thread_id = thread_val["id"].as_str().unwrap().to_string();

        // Create a local sample file to import
        let sample_file = temp_dir.path().join("code.rs");
        std::fs::write(&sample_file, b"fn main() { println!(\"Hello!\"); }").unwrap();

        // 1. Call attachment.import_path
        let import_req = RpcRequest::new(
            2,
            "attachment.import_path",
            Some(json!({
                "threadId": thread_id,
                "path": sample_file.to_str().unwrap()
            })),
        );
        let import_res = router.handle_request(import_req).await.unwrap();
        assert!(import_res.is_success());
        let att_val = import_res.result.unwrap();
        let att_id = att_val["attachment"]["id"].as_str().unwrap().to_string();
        assert_eq!(att_val["attachment"]["fileName"], "code.rs");
        assert_eq!(att_val["attachment"]["mimeType"], "text/x-rust");

        // 2. Call attachment.list
        let list_req =
            RpcRequest::new(3, "attachment.list", Some(json!({ "threadId": thread_id })));
        let list_res = router.handle_request(list_req).await.unwrap();
        assert!(list_res.is_success());
        let list_val = list_res.result.unwrap();
        assert_eq!(list_val["attachments"].as_array().unwrap().len(), 1);

        // 3. Call attachment.get
        let get_req = RpcRequest::new(
            4,
            "attachment.get",
            Some(json!({ "threadId": thread_id, "id": att_id })),
        );
        let get_res = router.handle_request(get_req).await.unwrap();
        assert!(get_res.is_success());
        let get_val = get_res.result.unwrap();
        assert_eq!(get_val["attachment"]["id"], att_id);

        // 4. Send turn referencing attachment
        let send_req = RpcRequest::new(
            5,
            "thread.send_turn",
            Some(json!({
                "threadId": thread_id,
                "text": "Review this code",
                "attachmentIds": [att_id]
            })),
        );
        let send_res = router.handle_request(send_req).await.unwrap();
        assert!(send_res.is_success());
    }

    #[tokio::test]
    async fn router_handles_settings_and_provider_health() {
        let store = Store::in_memory().unwrap();
        let drivers = Arc::new(DriverRegistry::default_local());
        let router = Router::with_drivers(store, ServerRole::Hub, "env-test".to_string(), drivers);

        // 1. Get initial settings
        let get_req = RpcRequest::new(1, "settings.get", None);
        let get_res = router.handle_request(get_req).await.unwrap();
        assert!(get_res.is_success());
        let get_val = get_res.result.unwrap();
        assert_eq!(get_val["settings"]["version"], 2);
        assert_eq!(get_val["settings"]["terminal"]["scrollbackLines"], 10_000);

        // 2. Update a specific key using config.set dotted path
        let set_req = RpcRequest::new(
            2,
            "config.set",
            Some(json!({
                "key": "terminal.scrollbackLines",
                "value": 25_000
            })),
        );
        let set_res = router.handle_request(set_req).await.unwrap();
        assert!(set_res.is_success());

        // 3. Query the updated key via config.get
        let get_key_req = RpcRequest::new(
            3,
            "config.get",
            Some(json!({ "key": "terminal.scrollbackLines" })),
        );
        let get_key_res = router.handle_request(get_key_req).await.unwrap();
        assert!(get_key_res.is_success());
        let key_val = get_key_res.result.unwrap();
        assert_eq!(key_val["value"], 25_000);

        // 4. Run provider health checks (offline, never triggers auth)
        let health_req = RpcRequest::new(4, "provider.health", None);
        let health_res = router.handle_request(health_req).await.unwrap();
        assert!(health_res.is_success());
        let health_val = health_res.result.unwrap();
        let reports = health_val["reports"].as_array().unwrap();
        assert!(!reports.is_empty());
        // Verify report structure
        for report in reports {
            assert!(report.get("instanceId").is_some());
            assert!(report.get("provider").is_some());
            assert!(report.get("status").is_some());
        }
    }

    #[tokio::test]
    async fn router_handles_git_actions() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = temp.path();

        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("init")
            .output();
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("config")
            .arg("user.name")
            .arg("Test")
            .output();
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("config")
            .arg("user.email")
            .arg("test@local")
            .output();
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("config")
            .arg("remote.origin.url")
            .arg("https://github.com/BoardPandas/Pandamux.git")
            .output();

        std::fs::write(repo_path.join("README.md"), "# Initial\n").unwrap();
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("add")
            .arg(".")
            .output();
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("commit")
            .arg("-m")
            .arg("initial commit")
            .output();

        let store = Store::in_memory().unwrap();
        let drivers = Arc::new(DriverRegistry::default_local());
        let router = Router::with_drivers(store, ServerRole::Hub, "env-test".to_string(), drivers);

        // 1. Create a thread pointing to this repo
        let create_req = RpcRequest::new(
            1,
            "thread.create",
            Some(json!({
                "agentId": "claude",
                "cwd": repo_path.to_str().unwrap()
            })),
        );
        let create_res = router.handle_request(create_req).await.unwrap();
        let thread_val = create_res.result.unwrap();
        let thread_id = thread_val["id"].as_str().unwrap().to_string();

        // 2. Query git status on clean repo
        let status_req = RpcRequest::new(
            2,
            "git.status",
            Some(json!({
                "threadId": thread_id
            })),
        );
        let status_res = router.handle_request(status_req).await.unwrap();
        assert!(status_res.is_success());
        let status_val = status_res.result.unwrap();
        assert_eq!(status_val["isRepo"], true);
        assert_eq!(status_val["files"].as_array().unwrap().len(), 0);

        // 3. Create a new file to modify repository state
        std::fs::write(repo_path.join("test_file.rs"), "fn hello() {}\n").unwrap();

        // 4. Query status again and verify untracked file + drafted commit message
        let status_req2 = RpcRequest::new(
            3,
            "git.status",
            Some(json!({
                "threadId": thread_id
            })),
        );
        let status_res2 = router.handle_request(status_req2).await.unwrap();
        assert!(status_res2.is_success());
        let status_val2 = status_res2.result.unwrap();
        let files = status_val2["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["status"], "untracked");
        assert_eq!(
            status_val2["draftedCommitMessage"],
            "feat: add test_file.rs"
        );

        // 5. Commit using git.commit
        let commit_req = RpcRequest::new(
            4,
            "git.commit",
            Some(json!({
                "threadId": thread_id
            })),
        );
        let commit_res = router.handle_request(commit_req).await.unwrap();
        assert!(commit_res.is_success());
        let commit_val = commit_res.result.unwrap();
        assert_eq!(commit_val["success"], true);
        assert_eq!(commit_val["filesCommitted"], 1);
        assert!(!commit_val["commitHash"].as_str().unwrap().is_empty());

        // 6. Create PR via git.create_pr (fallback compare URL)
        let pr_req = RpcRequest::new(
            5,
            "git.create_pr",
            Some(json!({
                "threadId": thread_id
            })),
        );
        let pr_res = router.handle_request(pr_req).await.unwrap();
        assert!(pr_res.is_success());
        let pr_val = pr_res.result.unwrap();
        assert_eq!(pr_val["success"], true);
        assert!(
            pr_val["url"]
                .as_str()
                .unwrap()
                .starts_with("https://github.com/BoardPandas/Pandamux/compare/")
        );
    }

    #[tokio::test]
    async fn router_handles_subagent_tree() {
        let store = Store::in_memory().unwrap();
        let router = Router::new(store.clone(), ServerRole::Node, "env-local".to_string());

        // Create thread
        let req = RpcRequest::new(
            1,
            "thread.create",
            Some(json!({ "title": "Subagent Thread" })),
        );
        let res = router.handle_request(req).await.unwrap();
        assert!(res.is_success());
        let tid: pandamux_core::ThreadId =
            serde_json::from_value(res.result.unwrap()["id"].clone()).unwrap();

        // Append sub-agent events
        let ev1 = pandamux_core::ThreadEvent {
            thread_id: tid.clone(),
            seq: 1,
            at_ms: 1000,
            kind: ThreadEventKind::SubAgentSpawned {
                sub_agent_id: "agent-root".to_string(),
                parent_sub_agent_id: None,
                parent_item_id: None,
                title: "Root Coordinator".to_string(),
                agent_type: "task".to_string(),
                model: "claude-3-7-sonnet".to_string(),
                effort: Some("high".to_string()),
            },
        };
        store.append_event(&ev1).unwrap();

        let ev2 = pandamux_core::ThreadEvent {
            thread_id: tid.clone(),
            seq: 2,
            at_ms: 1010,
            kind: ThreadEventKind::SubAgentSpawned {
                sub_agent_id: "agent-child".to_string(),
                parent_sub_agent_id: Some("agent-root".to_string()),
                parent_item_id: None,
                title: "Code Explorer".to_string(),
                agent_type: "explore".to_string(),
                model: "claude-3-5-haiku".to_string(),
                effort: Some("low".to_string()),
            },
        };
        store.append_event(&ev2).unwrap();

        let ev3 = pandamux_core::ThreadEvent {
            thread_id: tid.clone(),
            seq: 3,
            at_ms: 1020,
            kind: ThreadEventKind::SubAgentActivity {
                sub_agent_id: "agent-child".to_string(),
                preview: "Scanning repository dependencies...".to_string(),
            },
        };
        store.append_event(&ev3).unwrap();

        let ev4 = pandamux_core::ThreadEvent {
            thread_id: tid.clone(),
            seq: 4,
            at_ms: 1030,
            kind: ThreadEventKind::SubAgentUsage {
                sub_agent_id: "agent-child".to_string(),
                tokens: 450,
                tool_calls: 3,
            },
        };
        store.append_event(&ev4).unwrap();

        let ev5 = pandamux_core::ThreadEvent {
            thread_id: tid.clone(),
            seq: 5,
            at_ms: 1040,
            kind: ThreadEventKind::SubAgentFinished {
                sub_agent_id: "agent-child".to_string(),
                outcome: "Completed scan".to_string(),
                elapsed_ms: 1200,
                summary: Some("Discovered 5 crates".to_string()),
            },
        };
        store.append_event(&ev5).unwrap();

        // Query subagent.tree
        let tree_req = RpcRequest::new(
            2,
            "subagent.tree",
            Some(json!({
                "threadId": tid
            })),
        );
        let tree_res = router.handle_request(tree_req).await.unwrap();
        assert!(tree_res.is_success());
        let tree_val: SubAgentTreeResult =
            serde_json::from_value(tree_res.result.unwrap()).unwrap();

        assert_eq!(tree_val.total_count, 2);
        assert_eq!(tree_val.root_agents.len(), 1);
        assert_eq!(tree_val.root_agents[0].id, "agent-root");
        assert_eq!(tree_val.root_agents[0].children.len(), 1);

        let child = &tree_val.root_agents[0].children[0];
        assert_eq!(child.id, "agent-child");
        assert_eq!(
            child.latest_activity.as_deref(),
            Some("Scanning repository dependencies...")
        );
        assert_eq!(child.tokens, 450);
        assert_eq!(child.tool_calls, 3);
        assert_eq!(child.outcome.as_deref(), Some("Completed scan"));
        assert_eq!(child.elapsed_ms, Some(1200));
        assert!(child.finished);
    }

    #[tokio::test]
    async fn router_handles_fs_read_and_list() {
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Node, "env-local".to_string());

        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("lib.rs"), "pub fn demo() {}").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]").unwrap();

        // 1. fs.list root
        let list_req = RpcRequest::new(
            1,
            "fs.list",
            Some(json!({
                "root_path": root.to_str().unwrap()
            })),
        );
        let list_res = router.handle_request(list_req).await.unwrap();
        assert!(list_res.is_success());
        let list_val: pandamux_protocol::FsListResult =
            serde_json::from_value(list_res.result.unwrap()).unwrap();
        assert_eq!(list_val.entries.len(), 2);
        assert_eq!(list_val.entries[0].name, "src");
        assert!(list_val.entries[0].is_dir);

        // 2. fs.read valid file
        let read_req = RpcRequest::new(
            2,
            "fs.read",
            Some(json!({
                "root_path": root.to_str().unwrap(),
                "path": "src/lib.rs"
            })),
        );
        let read_res = router.handle_request(read_req).await.unwrap();
        assert!(read_res.is_success());
        let read_val: pandamux_protocol::FsReadResult =
            serde_json::from_value(read_res.result.unwrap()).unwrap();
        assert_eq!(read_val.content, "pub fn demo() {}");
        assert!(!read_val.is_binary);

        // 3. fs.read traversal attempt (must fail confinement)
        let evil_req = RpcRequest::new(
            3,
            "fs.read",
            Some(json!({
                "root_path": root.to_str().unwrap(),
                "path": "../../outside_secret.txt"
            })),
        );
        let evil_res = router.handle_request(evil_req).await.unwrap();
        assert!(evil_res.error.is_some());
    }
}
