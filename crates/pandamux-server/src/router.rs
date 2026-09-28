use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use pandamux_core::{ThreadEventKind, UserSettings};
use pandamux_protocol::antigravity_rpc::{
    AntigravityInstallParams, AntigravityOAuthRelayParams, AntigravityOAuthRelayResult,
    AntigravityOAuthStartParams, AntigravityOAuthStartResult, AntigravityStatusParams,
    AntigravityStatusResult,
};
use pandamux_protocol::{
    AttachmentGetParams, AttachmentImportPathParams, AttachmentListParams,
    AttachmentPutChunkParams, CheckpointDiffParams, CheckpointListParams, CheckpointRollbackParams,
    EnvironmentImportSshConfigParams, EnvironmentImportSshConfigResult, EnvironmentListResult,
    GitCommitParams, GitCreatePrParams, GitPushParams, GitStatusParams, HarkDictatePromptParams,
    HarkSpellbookSyncParams, HelloParams, HelloResult, IdentifyResult, McpCallToolParams,
    PROTOCOL_VERSION, PingResult, ProviderHealthParams, ProviderHealthReport, ProviderHealthResult,
    RpcError, RpcRequest, RpcResponse, ServerCapabilities, ServerRole, SettingsGetParams,
    SettingsGetResult, SettingsSetParams, SubAgentTreeItem, SubAgentTreeParams, SubAgentTreeResult,
    TerminalAttachParams, TerminalCloseParams, TerminalInputParams, TerminalOpenParams,
    TerminalResizeParams, ThreadCancelTurnParams, ThreadCreateParams, ThreadGetParams,
    ThreadListParams, ThreadRespondApprovalParams, ThreadResumeParams, ThreadSendTurnParams,
    WorktreeSyncDeltaParams, WorktreeSyncDeltaResult,
};

use crate::attachment::AttachmentManager;
use crate::driver_registry::DriverRegistry;
use crate::mcp_server::McpServer;
use crate::store::Store;
use crate::terminal::TerminalServerManager;
use crate::thread_manager::ThreadManager;

pub struct Router {
    store: Store,
    mcp: McpServer,
    thread_manager: ThreadManager,
    attachments: AttachmentManager,
    notifications: Arc<tokio::sync::Mutex<Vec<pandamux_core::notification::NotificationInfo>>>,
    drivers: Arc<DriverRegistry>,
    terminal: TerminalServerManager,
    environment_router: Arc<crate::environment_router::EnvironmentRouter>,
    remote_terminals: Arc<std::sync::Mutex<HashMap<String, pandamux_core::ids::EnvironmentId>>>,
    hark: Arc<crate::hark::HarkBridge>,
    data_dir: PathBuf,
    role: ServerRole,
    environment_id: String,
    server_version: String,
}

fn default_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PANDAMUX_DATA_DIR") {
        PathBuf::from(dir)
    } else if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_app_data).join("pandamux")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".pandamux")
    } else {
        std::env::temp_dir().join("pandamux")
    }
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
        let terminal = TerminalServerManager::new();
        let environment_router = Arc::new(crate::environment_router::EnvironmentRouter::new(
            pandamux_core::ids::EnvironmentId::new(environment_id.clone()),
        ));
        Self {
            store,
            mcp,
            thread_manager,
            attachments,
            notifications: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            drivers,
            terminal,
            environment_router,
            remote_terminals: Arc::new(std::sync::Mutex::new(HashMap::new())),
            hark: Arc::new(crate::hark::HarkBridge::default()),
            data_dir: default_data_dir(),
            role,
            environment_id,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn with_hark(mut self, hark: Arc<crate::hark::HarkBridge>) -> Self {
        self.hark = hark;
        self
    }

    pub fn hark(&self) -> &Arc<crate::hark::HarkBridge> {
        &self.hark
    }

    pub fn with_data_dir(mut self, dir: PathBuf) -> Self {
        self.data_dir = dir;
        self
    }

    pub fn with_environment_router(
        mut self,
        environment_router: Arc<crate::environment_router::EnvironmentRouter>,
    ) -> Self {
        self.environment_router = environment_router;
        self
    }

    pub fn environment_router(&self) -> &Arc<crate::environment_router::EnvironmentRouter> {
        &self.environment_router
    }

    pub fn remote_terminals(&self) -> HashMap<String, pandamux_core::ids::EnvironmentId> {
        self.remote_terminals
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    pub fn register_remote_terminal(
        &self,
        terminal_id: String,
        environment_id: pandamux_core::ids::EnvironmentId,
    ) {
        if let Ok(mut map) = self.remote_terminals.lock() {
            map.insert(terminal_id, environment_id);
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

    /// Syncs an attachment stored locally on the Hub across the tunnel to a remote environment.
    pub async fn sync_attachment_to_environment(
        &self,
        env_id: &pandamux_core::ids::EnvironmentId,
        thread_id: &pandamux_core::ids::ThreadId,
        attachment_id: &str,
    ) -> Result<(), RpcError> {
        let record = self
            .store
            .get_attachment(thread_id, attachment_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| {
                RpcError::invalid_params(format!(
                    "Attachment '{attachment_id}' not found for thread '{}'",
                    thread_id.as_str()
                ))
            })?;

        let bytes = self
            .attachments
            .get_file_bytes(thread_id, attachment_id)
            .map_err(|e| {
                RpcError::internal_error(format!("Failed to read attachment file: {e}"))
            })?;

        let chunks = AttachmentManager::create_chunk_params(
            thread_id,
            attachment_id,
            &record.file_name,
            &record.mime_type,
            &bytes,
        );

        for chunk in chunks {
            let req = RpcRequest::new(
                Self::now_ms() as i64,
                "attachment.put",
                Some(
                    serde_json::to_value(&chunk)
                        .map_err(|e| RpcError::internal_error(e.to_string()))?,
                ),
            );
            self.environment_router
                .forward_request(env_id, &req)
                .await?;
        }

        Ok(())
    }

    fn resolve_target_environment(
        &self,
        _method: &str,
        params: &Value,
    ) -> Option<pandamux_core::ids::EnvironmentId> {
        // 1. Direct environmentId in params
        if let Some(env_id_val) = params
            .get("environmentId")
            .or_else(|| params.get("environment_id"))
            .and_then(|v| v.as_str())
        {
            return Some(pandamux_core::ids::EnvironmentId::new(env_id_val));
        }

        // 2. Thread-scoped methods: look up thread's environment_id in store
        let thread_id_opt = params
            .get("threadId")
            .or_else(|| params.get("thread_id"))
            .and_then(|v| v.as_str());
        if let Some(tid_str) = thread_id_opt {
            let tid = pandamux_core::ids::ThreadId::new(tid_str);
            if let Ok(Some(thread)) = self.store.get_thread(&tid) {
                return Some(thread.environment_id);
            }
        }

        // 3. Terminal-scoped methods: look up terminal_id in remote_terminals map
        let terminal_id_opt = params
            .get("terminalId")
            .or_else(|| params.get("terminal_id"))
            .and_then(|v| v.as_str());
        if let Some(tid_str) = terminal_id_opt
            && let Ok(map) = self.remote_terminals.lock()
            && let Some(env_id) = map.get(tid_str)
        {
            return Some(env_id.clone());
        }

        None
    }

    /// Handle a single parsed JSON-RPC request and return an optional response.
    pub async fn handle_request(&self, req: RpcRequest) -> Option<RpcResponse> {
        let is_notification = req.is_notification();
        let id = req.id.clone();
        let method = req.method.clone();
        let params = req.params.clone().unwrap_or(Value::Null);

        // Hub environment routing: if targeted to a remote environment, forward across transport
        if self.role == ServerRole::Hub
            && let Some(target_env) = self.resolve_target_environment(&method, &params)
            && !self.environment_router.is_local(&Some(target_env.clone()))
        {
            // Sync referenced attachments across tunnel before dispatching remote turn
            if (method == "thread.send_turn" || method == "thread.sendTurn")
                && let Some(tid_str) = params.get("threadId").and_then(|v| v.as_str())
            {
                let tid = pandamux_core::ids::ThreadId::new(tid_str);
                if let Some(att_array) = params.get("attachments").and_then(|v| v.as_array()) {
                    for att_val in att_array {
                        let att_id_opt = att_val
                            .as_str()
                            .or_else(|| att_val.get("id").and_then(|v| v.as_str()));
                        if let Some(att_id) = att_id_opt
                            && self
                                .store
                                .get_attachment(&tid, att_id)
                                .ok()
                                .flatten()
                                .is_some()
                        {
                            let _ = self
                                .sync_attachment_to_environment(&target_env, &tid, att_id)
                                .await;
                        }
                    }
                }
            }

            let forwarded = self
                .environment_router
                .forward_request(&target_env, &req)
                .await;

            if is_notification {
                return None;
            }

            match forwarded {
                Ok(resp) => {
                    if method == "thread.create"
                        && let Some(val) = &resp.result
                        && let Ok(thread) =
                            serde_json::from_value::<pandamux_core::Thread>(val.clone())
                    {
                        let _ = self.store.save_thread(&thread);
                    }
                    if method == "terminal.open"
                        && let Some(val) = &resp.result
                        && let Ok(term_res) = serde_json::from_value::<
                            pandamux_protocol::terminal_rpc::TerminalOpenResult,
                        >(val.clone())
                        && let Ok(mut map) = self.remote_terminals.lock()
                    {
                        map.insert(term_res.terminal_id, target_env.clone());
                    }
                    if method == "terminal.close"
                        && let Some(tid_str) = params
                            .get("terminalId")
                            .or_else(|| params.get("terminal_id"))
                            .and_then(|v| v.as_str())
                        && let Ok(mut map) = self.remote_terminals.lock()
                    {
                        map.remove(tid_str);
                    }
                    return Some(resp);
                }
                Err(err) => return Some(RpcResponse::error(id, err)),
            }
        }

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
                self.handle_attachment_import_path(params).await
            }
            "attachment.put" => self.handle_attachment_put(params).await,
            "attachment.list" => self.handle_attachment_list(params),
            "attachment.get" => self.handle_attachment_get(params),
            "subagent.tree" | "subagent.list" => self.handle_subagent_tree(params),
            "fs.read" => self.handle_fs_read(params),
            "fs.list" => self.handle_fs_list(params),
            "terminal.open" => self.handle_terminal_open(params),
            "terminal.list" => self.handle_terminal_list(),
            "terminal.attach" => self.handle_terminal_attach(params),
            "terminal.input" => self.handle_terminal_input(params),
            "terminal.resize" => self.handle_terminal_resize(params),
            "terminal.close" => self.handle_terminal_close(params),
            "antigravity.status" => self.handle_antigravity_status(params),
            "antigravity.install" => self.handle_antigravity_install(params).await,
            "antigravity.oauth_start" | "antigravity.oauthStart" => {
                self.handle_antigravity_oauth_start(params)
            }
            "antigravity.oauth_relay" | "antigravity.oauthRelay" => {
                self.handle_antigravity_oauth_relay(params).await
            }
            "environment.import_ssh_config" | "environment.importSshConfig" => {
                self.handle_environment_import_ssh_config(params)
            }
            "environment.list" => self.handle_environment_list(),
            "worktree.sync_delta" | "worktree.syncDelta" => {
                self.handle_worktree_sync_delta(params).await
            }
            "hark.status" => self.handle_hark_status(),
            "hark.sync_spellbook" | "hark.syncSpellbook" => self.handle_hark_sync_spellbook(params),
            "hark.dictate_prompt" | "hark.dictatePrompt" => self.handle_hark_dictate_prompt(params),
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

    async fn handle_attachment_import_path(&self, params: Value) -> Result<Value, RpcError> {
        let p: AttachmentImportPathParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = self
            .attachments
            .import_path(&self.store, &p.thread_id, &p.path)?;

        if self.role == ServerRole::Hub
            && let Ok(Some(thread)) = self.store.get_thread(&p.thread_id)
            && !self
                .environment_router
                .is_local(&Some(thread.environment_id.clone()))
        {
            let _ = self
                .sync_attachment_to_environment(
                    &thread.environment_id,
                    &p.thread_id,
                    &result.attachment.id,
                )
                .await;
        }

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

    fn handle_terminal_open(&self, params: Value) -> Result<Value, RpcError> {
        let open_params: TerminalOpenParams = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(format!("Invalid terminal.open params: {e}")))?;
        let result = self
            .terminal
            .open(open_params)
            .map_err(RpcError::internal_error)?;
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_terminal_list(&self) -> Result<Value, RpcError> {
        let result = self.terminal.list();
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_terminal_attach(&self, params: Value) -> Result<Value, RpcError> {
        let attach_params: TerminalAttachParams = serde_json::from_value(params).map_err(|e| {
            RpcError::invalid_params(format!("Invalid terminal.attach params: {e}"))
        })?;
        let result = self
            .terminal
            .attach(attach_params)
            .map_err(RpcError::internal_error)?;
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_terminal_input(&self, params: Value) -> Result<Value, RpcError> {
        let input_params: TerminalInputParams = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(format!("Invalid terminal.input params: {e}")))?;
        self.terminal
            .input(input_params)
            .map_err(RpcError::internal_error)?;
        Ok(json!({ "success": true }))
    }

    fn handle_terminal_resize(&self, params: Value) -> Result<Value, RpcError> {
        let resize_params: TerminalResizeParams = serde_json::from_value(params).map_err(|e| {
            RpcError::invalid_params(format!("Invalid terminal.resize params: {e}"))
        })?;
        self.terminal
            .resize(resize_params)
            .map_err(RpcError::internal_error)?;
        Ok(json!({ "success": true }))
    }

    fn handle_terminal_close(&self, params: Value) -> Result<Value, RpcError> {
        let close_params: TerminalCloseParams = serde_json::from_value(params)
            .map_err(|e| RpcError::invalid_params(format!("Invalid terminal.close params: {e}")))?;
        if let Ok(mut map) = self.remote_terminals.lock() {
            map.remove(&close_params.terminal_id);
        }
        self.terminal
            .close(close_params)
            .map_err(RpcError::internal_error)?;
        Ok(json!({ "success": true }))
    }

    fn handle_antigravity_status(&self, params: Value) -> Result<Value, RpcError> {
        let _p: AntigravityStatusParams = serde_json::from_value(params).unwrap_or_default();

        #[cfg(target_os = "windows")]
        let (platform, arch) = ("windows", "x64");
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let (platform, arch) = ("linux", "x64");
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        let (platform, arch) = ("linux", "arm64");
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let (platform, arch) = ("unsupported", "unknown");

        let platform_arch = format!("{platform}-{arch}");
        let bin_path = pandamux_providers::antigravity::resolve_antigravity_binary(
            None,
            &self.data_dir,
            &platform_arch,
        );

        let gemini_home = self.data_dir.join("profiles/antigravity/default");
        let health = if let Some(ref bp) = bin_path {
            pandamux_providers::antigravity::probe_antigravity_health_offline(
                bp,
                &gemini_home,
                None,
            )
        } else {
            pandamux_providers::models::ProviderHealth::Unavailable {
                reason: "Binary not installed".to_string(),
            }
        };

        let authenticated = matches!(health, pandamux_providers::models::ProviderHealth::Healthy);
        let free_disk = pandamux_providers::antigravity::get_available_disk_bytes(&self.data_dir);

        let result = AntigravityStatusResult {
            installed: bin_path.is_some(),
            version: if bin_path.is_some() {
                Some("1.1.1".to_string())
            } else {
                None
            },
            binary_path: bin_path.map(|p| p.to_string_lossy().to_string()),
            authenticated,
            active_processes: 0,
            max_processes: 2,
            free_disk_bytes: free_disk,
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_antigravity_install(&self, params: Value) -> Result<Value, RpcError> {
        let p: AntigravityInstallParams = serde_json::from_value(params).map_err(|e| {
            RpcError::invalid_params(format!("Invalid antigravity.install params: {e}"))
        })?;

        let fallback_bytes = if let Some(ref b64) = p.fallback_bytes_base64 {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).ok()
        } else {
            None
        };

        let result = pandamux_providers::antigravity::install_managed_bundle(
            &self.data_dir,
            &p.manifest,
            fallback_bytes.as_deref(),
        )
        .await
        .map_err(|e| RpcError::internal_error(format!("Antigravity bundle install failed: {e}")))?;

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_antigravity_oauth_start(&self, params: Value) -> Result<Value, RpcError> {
        let _p: AntigravityOAuthStartParams = serde_json::from_value(params).unwrap_or_default();

        let state = format!("agy-state-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let redirect_port = 51123;
        let auth_url = format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id=pandamux-acp.apps.googleusercontent.com&redirect_uri=http%3A%2F%2F127.0.0.1%3A{redirect_port}%2F&response_type=code&scope=openid%20profile%20email&state={state}"
        );

        let result = AntigravityOAuthStartResult {
            auth_url,
            redirect_port,
            state,
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_antigravity_oauth_relay(&self, params: Value) -> Result<Value, RpcError> {
        let p: AntigravityOAuthRelayParams = serde_json::from_value(params).map_err(|e| {
            RpcError::invalid_params(format!("Invalid antigravity.oauth_relay params: {e}"))
        })?;

        let message = pandamux_providers::antigravity::execute_oauth_relay_callback(
            &p.callback_url,
            &p.state,
            p.redirect_port,
        )
        .await
        .map_err(|e| RpcError::internal_error(format!("OAuth relay execution failed: {e}")))?;

        let profile_dir = self.data_dir.join("profiles/antigravity/default");
        let _ = std::fs::create_dir_all(&profile_dir);
        let settings_path = profile_dir.join("settings.json");
        let _ = std::fs::write(
            &settings_path,
            serde_json::json!({ "auth": { "type": "oauth-personal" } }).to_string(),
        );

        let result = AntigravityOAuthRelayResult {
            success: true,
            message,
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_environment_import_ssh_config(&self, params: Value) -> Result<Value, RpcError> {
        let p: EnvironmentImportSshConfigParams = if params.is_null() {
            Default::default()
        } else {
            serde_json::from_value(params).map_err(|e| {
                RpcError::invalid_params(format!(
                    "Invalid environment.import_ssh_config params: {e}"
                ))
            })?
        };

        let config_text =
            if let Some(custom) = p.custom_config.as_deref().filter(|s| !s.trim().is_empty()) {
                custom.to_string()
            } else {
                pandamux_core::read_default_ssh_config().unwrap_or_default()
            };

        let mut settings = self
            .store
            .get_settings()
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .unwrap_or_default();

        let (new_envs, profiles, skipped_count) =
            pandamux_core::import_ssh_config_into_environments(
                &config_text,
                &settings.environments,
            );

        let imported_count = new_envs.len();

        if !p.dry_run && !new_envs.is_empty() {
            settings.environments.extend(new_envs.clone());
            settings.normalize();
            self.store
                .save_settings(&settings)
                .map_err(|e| RpcError::internal_error(e.to_string()))?;
        }

        let result = EnvironmentImportSshConfigResult {
            imported_count,
            skipped_existing_count: skipped_count,
            environments: new_envs,
            profiles,
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_environment_list(&self) -> Result<Value, RpcError> {
        let settings = self
            .store
            .get_settings()
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .unwrap_or_default();

        let result = EnvironmentListResult {
            environments: settings.environments,
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_worktree_sync_delta(&self, params: Value) -> Result<Value, RpcError> {
        let p: WorktreeSyncDeltaParams = serde_json::from_value(params).map_err(|e| {
            RpcError::invalid_params(format!("Invalid worktree.sync_delta params: {e}"))
        })?;

        // If target environment is a remote node and this is the Hub:
        if let Some(ref env_id) = p.environment_id
            && env_id.as_str() != self.environment_id.as_str()
            && self.environment_router.is_connected(env_id).await
        {
            let mut forwarded_params = p.clone();
            if forwarded_params.bundle_bytes_base64.is_none() {
                let local_path = PathBuf::from(&p.local_path);
                let bundle_bytes = crate::worktree::create_git_bundle(&local_path, &p.rev_range)
                    .map_err(|e| {
                        RpcError::internal_error(format!("Failed to create local bundle: {e}"))
                    })?;
                forwarded_params.bundle_bytes_base64 = Some(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &bundle_bytes,
                ));
            }

            let req = RpcRequest::new(
                1,
                "worktree.sync_delta",
                Some(
                    serde_json::to_value(&forwarded_params)
                        .map_err(|e| RpcError::internal_error(e.to_string()))?,
                ),
            );
            let resp = self
                .environment_router
                .forward_request(env_id, &req)
                .await?;
            if resp.is_success() {
                return Ok(resp.result.unwrap_or(Value::Null));
            } else {
                let err = resp
                    .error
                    .unwrap_or_else(|| RpcError::internal_error("Remote worktree sync failed"));
                return Err(err);
            }
        }

        // Apply on node or locally
        let target_repo = PathBuf::from(if !p.remote_path.is_empty() {
            &p.remote_path
        } else {
            &p.local_path
        });

        let bundle_bytes = if let Some(ref b64) = p.bundle_bytes_base64 {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                .map_err(|e| RpcError::invalid_params(format!("Invalid base64 bundle: {e}")))?
        } else {
            let local_path = PathBuf::from(&p.local_path);
            crate::worktree::create_git_bundle(&local_path, &p.rev_range)
                .map_err(|e| RpcError::internal_error(format!("Failed to create bundle: {e}")))?
        };

        let bundle_size = bundle_bytes.len();
        let commits_applied =
            crate::worktree::apply_git_bundle(&target_repo, &bundle_bytes, &p.target_branch)
                .map_err(|e| {
                    RpcError::internal_error(format!(
                        "Failed to apply bundle to {}: {e}",
                        target_repo.display()
                    ))
                })?;

        let result = WorktreeSyncDeltaResult {
            synced: true,
            target_branch: p.target_branch,
            bundle_size_bytes: bundle_size,
            message: format!("Applied {commits_applied} commit(s) successfully"),
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_hark_status(&self) -> Result<Value, RpcError> {
        let status = self.hark.get_status();
        serde_json::to_value(status).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_hark_sync_spellbook(&self, params: Value) -> Result<Value, RpcError> {
        let p: HarkSpellbookSyncParams = serde_json::from_value(params).map_err(|e| {
            RpcError::invalid_params(format!("Invalid hark.sync_spellbook params: {e}"))
        })?;

        let result = self.hark.sync_spellbook(p.symbols);
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_hark_dictate_prompt(&self, params: Value) -> Result<Value, RpcError> {
        let p: HarkDictatePromptParams = serde_json::from_value(params).map_err(|e| {
            RpcError::invalid_params(format!("Invalid hark.dictate_prompt params: {e}"))
        })?;

        let result = self.hark.handle_dictate_prompt(p);
        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
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

    #[tokio::test]
    async fn router_handles_terminal_lifecycle() {
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env-test".to_string());

        // 1. Open terminal
        let open_req = RpcRequest::new(
            1,
            "terminal.open",
            Some(json!({
                "terminalId": "test-router-term",
                "cwd": ".",
                "rows": 24,
                "cols": 80
            })),
        );
        let open_res = router.handle_request(open_req).await.unwrap();
        assert!(open_res.is_success());
        let open_val = open_res.result.unwrap();
        assert_eq!(open_val["terminalId"], "test-router-term");
        assert_eq!(open_val["rows"], 24);
        assert_eq!(open_val["cols"], 80);

        // 2. List terminals
        let list_req = RpcRequest::new(2, "terminal.list", None);
        let list_res = router.handle_request(list_req).await.unwrap();
        assert!(list_res.is_success());
        let list_val = list_res.result.unwrap();
        assert_eq!(list_val["terminals"].as_array().unwrap().len(), 1);

        // 3. Attach
        let attach_req = RpcRequest::new(
            3,
            "terminal.attach",
            Some(json!({
                "terminalId": "test-router-term",
                "sinceOffset": 0
            })),
        );
        let attach_res = router.handle_request(attach_req).await.unwrap();
        assert!(attach_res.is_success());
        let attach_val = attach_res.result.unwrap();
        assert_eq!(attach_val["terminalId"], "test-router-term");

        // 4. Input
        let input_req = RpcRequest::new(
            4,
            "terminal.input",
            Some(json!({
                "terminalId": "test-router-term",
                "data": "echo 1\n"
            })),
        );
        let input_res = router.handle_request(input_req).await.unwrap();
        assert!(input_res.is_success());

        // 5. Resize
        let resize_req = RpcRequest::new(
            5,
            "terminal.resize",
            Some(json!({
                "terminalId": "test-router-term",
                "rows": 40,
                "cols": 100
            })),
        );
        let resize_res = router.handle_request(resize_req).await.unwrap();
        assert!(resize_res.is_success());

        // 6. Close
        let close_req = RpcRequest::new(
            6,
            "terminal.close",
            Some(json!({
                "terminalId": "test-router-term"
            })),
        );
        let close_res = router.handle_request(close_req).await.unwrap();
        assert!(close_res.is_success());

        let list_res2 = router
            .handle_request(RpcRequest::new(7, "terminal.list", None))
            .await
            .unwrap();
        assert_eq!(
            list_res2.result.unwrap()["terminals"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn test_hub_environment_routing_and_forwarding() {
        use crate::environment_router::MockEnvironmentTransport;
        use pandamux_core::ids::{EnvironmentId, ThreadId};
        use pandamux_core::{Thread, ThreadWorkspace};

        let store = Store::in_memory().unwrap();
        let hub_router = Router::new(store.clone(), ServerRole::Hub, "env-local".to_string());
        let remote_env = EnvironmentId::new("env-remote");

        // 1. Remote request fails when environment is not registered
        let req = RpcRequest::new(
            1,
            "thread.create",
            Some(json!({
                "environmentId": "env-remote",
                "title": "Remote Thread"
            })),
        );
        let resp = hub_router.handle_request(req).await.unwrap();
        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("unreachable"));

        // 2. Register mock remote transport
        let mock_thread = Thread {
            id: ThreadId::new("th-remote-1"),
            project_id: None,
            environment_id: remote_env.clone(),
            parent_thread_id: None,
            title: "Remote Thread".to_string(),
            provider_instance_id: pandamux_core::ids::ProviderInstanceId::new("claude-default"),
            model: "claude-3-7-sonnet-latest".to_string(),
            effort: None,
            access_mode: pandamux_core::thread::AccessMode::WorkspaceOnly,
            workspace: ThreadWorkspace {
                cwd: "/remote/home/project".to_string(),
                worktree: None,
            },
            status: pandamux_core::thread::ThreadStatus::Idle,
            agent: None,
            origin: pandamux_core::thread::ThreadOrigin::Manual,
            created_at_ms: 100,
            updated_at_ms: 100,
        };

        let mock_thread_clone = mock_thread.clone();
        let mock_transport = Arc::new(MockEnvironmentTransport::new(move |req| {
            let id = req.id.clone().unwrap_or(1.into());
            if req.method == "thread.create" {
                Ok(RpcResponse::success(
                    id,
                    serde_json::to_value(&mock_thread_clone).unwrap(),
                ))
            } else if req.method == "thread.get" {
                Ok(RpcResponse::success(
                    id,
                    json!({
                        "thread": mock_thread_clone,
                        "turns": []
                    }),
                ))
            } else {
                Ok(RpcResponse::success(id, json!({ "status": "ok" })))
            }
        }));

        hub_router
            .environment_router()
            .register_transport(
                remote_env.clone(),
                mock_transport.clone(),
                Some(hub_router.thread_manager().broadcaster().clone()),
            )
            .await;

        // 3. Create thread on remote environment through Hub router
        let create_req = RpcRequest::new(
            2,
            "thread.create",
            Some(json!({
                "environmentId": "env-remote",
                "title": "Remote Thread"
            })),
        );
        let create_res = hub_router.handle_request(create_req).await.unwrap();
        assert!(create_res.is_success());
        let res_thread: Thread = serde_json::from_value(create_res.result.unwrap()).unwrap();
        assert_eq!(res_thread.id.as_str(), "th-remote-1");
        assert_eq!(res_thread.environment_id.as_str(), "env-remote");

        // Hub store now has the cached thread with its environment_id
        let cached = store.get_thread(&ThreadId::new("th-remote-1")).unwrap();
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().environment_id.as_str(), "env-remote");

        // 4. Query thread by threadId only; router resolves environment and forwards
        let get_req = RpcRequest::new(
            3,
            "thread.get",
            Some(json!({
                "threadId": "th-remote-1"
            })),
        );
        let get_res = hub_router.handle_request(get_req).await.unwrap();
        assert!(get_res.is_success());
        let get_val = get_res.result.unwrap();
        assert_eq!(get_val["thread"]["id"], "th-remote-1");

        // 5. Test event relaying to Hub broadcaster
        let mut hub_events = hub_router.thread_manager().subscribe();
        mock_transport.emit_event(pandamux_protocol::EventEnvelope {
            subscription_id: "remote-sub".to_string(),
            environment_id: EnvironmentId::new("node-reported-id"),
            thread_id: Some(ThreadId::new("th-remote-1")),
            run_id: None,
            seq: 101,
            at_ms: 5000,
            kind: pandamux_core::ThreadEventKind::TurnCompleted {
                outcome: pandamux_core::event::TurnOutcome::Success,
            },
        });

        let relayed = hub_events.recv().await.unwrap();
        assert_eq!(relayed.environment_id.as_str(), "env-remote");
        assert_eq!(relayed.seq, 101);
    }

    #[tokio::test]
    async fn test_attachments_through_the_tunnel() {
        use crate::environment_router::MockEnvironmentTransport;
        use base64::Engine;
        use pandamux_core::ids::{EnvironmentId, ThreadId};
        use pandamux_core::{Thread, ThreadWorkspace};
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let att_dir = temp_dir.path().join("attachments");
        let attachments = AttachmentManager::new(att_dir);
        let store = Store::in_memory().unwrap();
        let hub_router = Router::new(store.clone(), ServerRole::Hub, "env-local".to_string())
            .with_attachments(attachments);

        let remote_env = EnvironmentId::new("env-remote");
        let thread_id = ThreadId::new("th-remote-att");

        // Save remote thread in Hub store
        let remote_thread = Thread {
            id: thread_id.clone(),
            project_id: None,
            environment_id: remote_env.clone(),
            parent_thread_id: None,
            title: "Remote Thread With Attachments".to_string(),
            provider_instance_id: pandamux_core::ids::ProviderInstanceId::new("claude-default"),
            model: "claude-3-7-sonnet-latest".to_string(),
            effort: None,
            access_mode: pandamux_core::thread::AccessMode::WorkspaceOnly,
            workspace: ThreadWorkspace {
                cwd: "/remote/project".to_string(),
                worktree: None,
            },
            status: pandamux_core::thread::ThreadStatus::Idle,
            agent: None,
            origin: pandamux_core::thread::ThreadOrigin::Manual,
            created_at_ms: 100,
            updated_at_ms: 100,
        };
        store.save_thread(&remote_thread).unwrap();

        // Put attachment into Hub local storage
        let put_req = RpcRequest::new(
            1,
            "attachment.put",
            Some(json!({
                "threadId": thread_id.as_str(),
                "id": "att-paste-1",
                "chunkIndex": 0,
                "totalChunks": 1,
                "dataBase64": base64::engine::general_purpose::STANDARD.encode(b"PNG_IMAGE_DATA_BYTES"),
                "mimeType": "image/png",
                "fileName": "screenshot.png",
                "environmentId": "env-local"
            })),
        );
        let put_res = hub_router.handle_request(put_req).await.unwrap();
        assert!(put_res.is_success());

        // Track forwarded attachment requests
        let put_chunks_received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let turns_received = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let put_chunks_clone = put_chunks_received.clone();
        let turns_clone = turns_received.clone();
        let mock_transport = Arc::new(MockEnvironmentTransport::new(move |req| {
            let id = req.id.clone().unwrap_or(1.into());
            if req.method == "attachment.put" {
                put_chunks_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(RpcResponse::success(id, json!({ "isComplete": true })))
            } else if req.method == "thread.send_turn" || req.method == "thread.sendTurn" {
                turns_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(RpcResponse::success(id, json!({ "accepted": true })))
            } else {
                Ok(RpcResponse::success(id, json!({ "status": "ok" })))
            }
        }));

        hub_router
            .environment_router()
            .register_transport(remote_env.clone(), mock_transport, None)
            .await;

        // Dispatch a turn to remote thread with attachment reference
        let send_req = RpcRequest::new(
            2,
            "thread.send_turn",
            Some(json!({
                "threadId": thread_id.as_str(),
                "text": "Analyze this screenshot",
                "attachments": ["att-paste-1"]
            })),
        );
        let send_res = hub_router.handle_request(send_req).await.unwrap();
        assert!(send_res.is_success());

        // Verify attachment chunks streamed through tunnel to remote node before turn
        assert_eq!(
            put_chunks_received.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(turns_received.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn router_routes_remote_terminal_lifecycle() {
        use pandamux_core::EnvironmentId;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let remote_env = EnvironmentId::new("env_remote_node");
        let env_router = Arc::new(crate::environment_router::EnvironmentRouter::new(
            EnvironmentId::new("env_local"),
        ));

        let open_count = Arc::new(AtomicUsize::new(0));
        let attach_count = Arc::new(AtomicUsize::new(0));
        let input_count = Arc::new(AtomicUsize::new(0));
        let resize_count = Arc::new(AtomicUsize::new(0));
        let close_count = Arc::new(AtomicUsize::new(0));

        let oc = Arc::clone(&open_count);
        let ac = Arc::clone(&attach_count);
        let ic = Arc::clone(&input_count);
        let rc = Arc::clone(&resize_count);
        let cc = Arc::clone(&close_count);

        let mock_transport = Arc::new(crate::environment_router::MockEnvironmentTransport::new(
            move |req| match req.method.as_str() {
                "terminal.open" => {
                    oc.fetch_add(1, Ordering::SeqCst);
                    Ok(RpcResponse::success(
                        req.id.clone().unwrap_or(1.into()),
                        json!({
                            "terminalId": "term-remote-1",
                            "cwd": "/remote/project",
                            "rows": 30,
                            "cols": 120
                        }),
                    ))
                }
                "terminal.attach" => {
                    ac.fetch_add(1, Ordering::SeqCst);
                    let since = req
                        .params
                        .as_ref()
                        .and_then(|p| p.get("sinceOffset"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    Ok(RpcResponse::success(
                        req.id.clone().unwrap_or(2.into()),
                        json!({
                            "terminalId": "term-remote-1",
                            "rows": 30,
                            "cols": 120,
                            "offset": since + 45,
                            "data": "remote prompt$ ",
                            "truncated": false
                        }),
                    ))
                }
                "terminal.input" => {
                    ic.fetch_add(1, Ordering::SeqCst);
                    Ok(RpcResponse::success(
                        req.id.clone().unwrap_or(3.into()),
                        json!({ "success": true }),
                    ))
                }
                "terminal.resize" => {
                    rc.fetch_add(1, Ordering::SeqCst);
                    Ok(RpcResponse::success(
                        req.id.clone().unwrap_or(4.into()),
                        json!({ "success": true }),
                    ))
                }
                "terminal.close" => {
                    cc.fetch_add(1, Ordering::SeqCst);
                    Ok(RpcResponse::success(
                        req.id.clone().unwrap_or(5.into()),
                        json!({ "success": true }),
                    ))
                }
                _ => Err(RpcError::method_not_found(&req.method)),
            },
        ));

        env_router
            .register_transport(remote_env.clone(), mock_transport, None)
            .await;

        let store = Store::in_memory().unwrap();
        let hub_router = Router::new(store, ServerRole::Hub, "env_local".to_string())
            .with_environment_router(env_router);

        // 1. Open terminal on remote environment
        let open_req = RpcRequest::new(
            1,
            "terminal.open",
            Some(json!({
                "environmentId": "env_remote_node",
                "cwd": "/remote/project",
                "rows": 30,
                "cols": 120
            })),
        );
        let open_res = hub_router.handle_request(open_req).await.unwrap();
        assert!(open_res.is_success());
        let term_id = open_res.result.unwrap()["terminalId"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(term_id, "term-remote-1");
        assert_eq!(open_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            hub_router.remote_terminals().get("term-remote-1"),
            Some(&remote_env)
        );

        // 2. Attach using terminalId (automatically routed to env_remote_node)
        let attach_req = RpcRequest::new(
            2,
            "terminal.attach",
            Some(json!({
                "terminalId": "term-remote-1",
                "sinceOffset": 0
            })),
        );
        let attach_res = hub_router.handle_request(attach_req).await.unwrap();
        assert!(attach_res.is_success());
        assert_eq!(attach_count.load(Ordering::SeqCst), 1);
        assert_eq!(attach_res.result.unwrap()["offset"], 45);

        // 3. Input sent using terminalId
        let input_req = RpcRequest::new(
            3,
            "terminal.input",
            Some(json!({
                "terminalId": "term-remote-1",
                "data": "htop\r\n"
            })),
        );
        let input_res = hub_router.handle_request(input_req).await.unwrap();
        assert!(input_res.is_success());
        assert_eq!(input_count.load(Ordering::SeqCst), 1);

        // 4. Resize sent using terminalId
        let resize_req = RpcRequest::new(
            4,
            "terminal.resize",
            Some(json!({
                "terminalId": "term-remote-1",
                "rows": 24,
                "cols": 80
            })),
        );
        let resize_res = hub_router.handle_request(resize_req).await.unwrap();
        assert!(resize_res.is_success());
        assert_eq!(resize_count.load(Ordering::SeqCst), 1);

        // 5. Close terminal and verify remote registration cleaned up
        let close_req = RpcRequest::new(
            5,
            "terminal.close",
            Some(json!({
                "terminalId": "term-remote-1"
            })),
        );
        let close_res = hub_router.handle_request(close_req).await.unwrap();
        assert!(close_res.is_success());
        assert_eq!(close_count.load(Ordering::SeqCst), 1);
        assert!(!hub_router.remote_terminals().contains_key("term-remote-1"));
    }

    #[tokio::test]
    async fn router_handles_antigravity_node_install_and_oauth_relay() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Node, "env_node".to_string())
            .with_data_dir(temp_dir.path().to_path_buf());

        // 1. Initial status: uninstalled
        let status_req = RpcRequest::new(1, "antigravity.status", None);
        let status_res = router.handle_request(status_req).await.unwrap();
        assert!(status_res.is_success());
        let status: AntigravityStatusResult =
            serde_json::from_value(status_res.result.unwrap()).unwrap();
        assert!(!status.installed);
        assert!(!status.authenticated);
        assert_eq!(status.max_processes, 2);

        let payload = b"sample-binary-payload-for-testing";
        let sha256_hash =
            "92207cde24114feb016121530c70b5c3de6d8e021bfbc9412c328a778147f78a".to_string();

        #[cfg(windows)]
        let (platform, arch, bin_entry) = ("windows", "x64", "agy_acp_server.exe");
        #[cfg(not(windows))]
        let (platform, arch, bin_entry) = ("linux", "x64", "agy_acp_server.par");

        use pandamux_protocol::antigravity_rpc::{AntigravityInstallResult, ManagedBundleManifest};
        let manifest = ManagedBundleManifest {
            version: "1.1.1".to_string(),
            platform: platform.to_string(),
            arch: arch.to_string(),
            url: format!(
                "https://dl.google.com/antigravity/releases/{platform}-{arch}/agy_acp_server_1.1.1.zip"
            ),
            size: payload.len() as u64,
            sha256: sha256_hash,
            entries: vec![bin_entry.to_string(), "localharness_external".to_string()],
        };

        let b64_payload =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, payload);
        let install_req = RpcRequest::new(
            2,
            "antigravity.install",
            Some(json!({
                "manifest": manifest,
                "fallbackBytesBase64": b64_payload
            })),
        );
        let install_res = router.handle_request(install_req).await.unwrap();
        assert!(install_res.is_success());
        let install_out: AntigravityInstallResult =
            serde_json::from_value(install_res.result.unwrap()).unwrap();
        assert!(install_out.installed);
        assert_eq!(install_out.version, "1.1.1");

        // 3. Status after installation: installed, but unauthenticated
        let status_req_2 = RpcRequest::new(3, "antigravity.status", None);
        let status_res_2 = router.handle_request(status_req_2).await.unwrap();
        assert!(status_res_2.is_success());
        let status_2: AntigravityStatusResult =
            serde_json::from_value(status_res_2.result.unwrap()).unwrap();
        assert!(status_2.installed);
        assert!(!status_2.authenticated);

        // 4. OAuth start generates valid auth URL
        let oauth_start_req = RpcRequest::new(4, "antigravity.oauth_start", None);
        let oauth_start_res = router.handle_request(oauth_start_req).await.unwrap();
        assert!(oauth_start_res.is_success());
        let oauth_start: AntigravityOAuthStartResult =
            serde_json::from_value(oauth_start_res.result.unwrap()).unwrap();
        assert!(
            oauth_start
                .auth_url
                .starts_with("https://accounts.google.com/")
        );
        assert_eq!(oauth_start.redirect_port, 51123);

        // 5. OAuth relay callback completes authentication
        let callback_url = format!(
            "http://127.0.0.1:51123/?code=4/sample_code&state={}",
            oauth_start.state
        );
        let relay_req = RpcRequest::new(
            5,
            "antigravity.oauth_relay",
            Some(json!({
                "callbackUrl": callback_url,
                "state": oauth_start.state,
                "redirectPort": oauth_start.redirect_port
            })),
        );
        let relay_res = router.handle_request(relay_req).await.unwrap();
        assert!(relay_res.is_success());

        // 6. Final status: installed AND authenticated!
        let status_req_3 = RpcRequest::new(6, "antigravity.status", None);
        let status_res_3 = router.handle_request(status_req_3).await.unwrap();
        let status_3: AntigravityStatusResult =
            serde_json::from_value(status_res_3.result.unwrap()).unwrap();
        assert!(status_3.installed);
        assert!(status_3.authenticated);
    }

    #[tokio::test]
    async fn router_routes_antigravity_to_remote_node() {
        use pandamux_core::EnvironmentId;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let remote_env = EnvironmentId::new("env_galahad");
        let env_router = Arc::new(crate::environment_router::EnvironmentRouter::new(
            EnvironmentId::new("env_local"),
        ));

        let install_count = Arc::new(AtomicUsize::new(0));
        let relay_count = Arc::new(AtomicUsize::new(0));
        let ic = Arc::clone(&install_count);
        let rc = Arc::clone(&relay_count);

        let mock_transport = Arc::new(crate::environment_router::MockEnvironmentTransport::new(
            move |req| match req.method.as_str() {
                "antigravity.install" => {
                    ic.fetch_add(1, Ordering::SeqCst);
                    Ok(RpcResponse::success(
                        req.id.clone().unwrap_or(1.into()),
                        json!({
                            "installed": true,
                            "version": "1.1.1",
                            "binaryPath": "/remote/tools/agy_acp_server.par",
                            "source": "hub_push",
                            "freeDiskBytes": 10737418240u64
                        }),
                    ))
                }
                "antigravity.oauth_relay" => {
                    rc.fetch_add(1, Ordering::SeqCst);
                    Ok(RpcResponse::success(
                        req.id.clone().unwrap_or(2.into()),
                        json!({
                            "success": true,
                            "message": "Relayed to remote node loopback"
                        }),
                    ))
                }
                _ => Err(RpcError::method_not_found(&req.method)),
            },
        ));

        env_router
            .register_transport(remote_env.clone(), mock_transport, None)
            .await;

        let store = Store::in_memory().unwrap();
        let hub_router = Router::new(store, ServerRole::Hub, "env_local".to_string())
            .with_environment_router(env_router);

        // Forward install to remote node
        let install_req = RpcRequest::new(
            1,
            "antigravity.install",
            Some(json!({
                "environmentId": "env_galahad",
                "manifest": {
                    "version": "1.1.1",
                    "platform": "linux",
                    "arch": "x64",
                    "url": "https://dl.google.com/test.zip",
                    "size": 100,
                    "sha256": "0123456789abcdef",
                    "entries": ["agy_acp_server.par", "localharness_external"]
                }
            })),
        );
        let install_res = hub_router.handle_request(install_req).await.unwrap();
        assert!(install_res.is_success());
        assert_eq!(install_count.load(Ordering::SeqCst), 1);

        // Forward oauth_relay to remote node
        let relay_req = RpcRequest::new(
            2,
            "antigravity.oauth_relay",
            Some(json!({
                "environmentId": "env_galahad",
                "callbackUrl": "http://127.0.0.1:51123/?code=123&state=state-1",
                "state": "state-1",
                "redirectPort": 51123
            })),
        );
        let relay_res = hub_router.handle_request(relay_req).await.unwrap();
        assert!(relay_res.is_success());
        assert_eq!(relay_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn router_handles_environment_import_ssh_config_and_list() {
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env_local".to_string());

        // 1. Initial environments list has local machine
        let list_req = RpcRequest::new(1, "environment.list", None);
        let list_res = router.handle_request(list_req).await.unwrap();
        assert!(list_res.is_success());
        let list_out: EnvironmentListResult =
            serde_json::from_value(list_res.result.unwrap()).unwrap();
        assert_eq!(list_out.environments.len(), 1);
        assert_eq!(list_out.environments[0].id.as_str(), "env_local");

        // 2. Dry run import preview
        let config_text = "\
Host galahad
    HostName 10.55.88.48
    User chaz
    Port 2222

Host worker-node
    HostName worker.example.com
";
        let dry_run_req = RpcRequest::new(
            2,
            "environment.import_ssh_config",
            Some(json!({
                "customConfig": config_text,
                "dryRun": true
            })),
        );
        let dry_res = router.handle_request(dry_run_req).await.unwrap();
        assert!(dry_res.is_success());
        let dry_out: EnvironmentImportSshConfigResult =
            serde_json::from_value(dry_res.result.unwrap()).unwrap();
        assert_eq!(dry_out.imported_count, 2);
        assert_eq!(dry_out.skipped_existing_count, 0);

        // List should still have only 1 environment since it was dry run
        let list_req_2 = RpcRequest::new(3, "environment.list", None);
        let list_res_2 = router.handle_request(list_req_2).await.unwrap();
        let list_out_2: EnvironmentListResult =
            serde_json::from_value(list_res_2.result.unwrap()).unwrap();
        assert_eq!(list_out_2.environments.len(), 1);

        // 3. Real import
        let real_import_req = RpcRequest::new(
            4,
            "environment.import_ssh_config",
            Some(json!({
                "customConfig": config_text,
                "dryRun": false
            })),
        );
        let real_res = router.handle_request(real_import_req).await.unwrap();
        assert!(real_res.is_success());
        let real_out: EnvironmentImportSshConfigResult =
            serde_json::from_value(real_res.result.unwrap()).unwrap();
        assert_eq!(real_out.imported_count, 2);

        // List now has 3 environments (local + galahad + worker-node)
        let list_req_3 = RpcRequest::new(5, "environment.list", None);
        let list_res_3 = router.handle_request(list_req_3).await.unwrap();
        let list_out_3: EnvironmentListResult =
            serde_json::from_value(list_res_3.result.unwrap()).unwrap();
        assert_eq!(list_out_3.environments.len(), 3);
        assert!(
            list_out_3
                .environments
                .iter()
                .any(|e| e.display_name == "galahad")
        );
        assert!(
            list_out_3
                .environments
                .iter()
                .any(|e| e.display_name == "worker-node")
        );

        // 4. Re-importing detects existing and skips them
        let reimport_req = RpcRequest::new(
            6,
            "environment.import_ssh_config",
            Some(json!({
                "customConfig": config_text
            })),
        );
        let reimport_res = router.handle_request(reimport_req).await.unwrap();
        assert!(reimport_res.is_success());
        let reimport_out: EnvironmentImportSshConfigResult =
            serde_json::from_value(reimport_res.result.unwrap()).unwrap();
        assert_eq!(reimport_out.imported_count, 0);
        assert_eq!(reimport_out.skipped_existing_count, 2);
    }

    #[tokio::test]
    async fn router_handles_hark_voice_bridge_and_spellbook_sync() {
        use pandamux_protocol::{
            HarkDictatePromptResult, HarkSpellbookSyncResult, HarkStatusResult,
        };
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env_local".to_string());

        // 1. Initial Hark status
        let status_req = RpcRequest::new(1, "hark.status", None);
        let status_res = router.handle_request(status_req).await.unwrap();
        assert!(status_res.is_success());
        let status: HarkStatusResult = serde_json::from_value(status_res.result.unwrap()).unwrap();
        assert_eq!(status.spellbook_count, 0);

        // 2. Sync spellbook symbols
        let sync_req = RpcRequest::new(
            2,
            "hark.sync_spellbook",
            Some(json!({
                "symbols": [
                    { "symbol": "Galahad", "category": "environment", "weight": 2.0 },
                    { "symbol": "PandaMUX", "category": "project", "weight": 2.5 }
                ]
            })),
        );
        let sync_res = router.handle_request(sync_req).await.unwrap();
        assert!(sync_res.is_success());
        let sync_out: HarkSpellbookSyncResult =
            serde_json::from_value(sync_res.result.unwrap()).unwrap();
        assert_eq!(sync_out.synchronized_count, 2);

        // 3. Status now reflects synced count
        let status_res_2 = router
            .handle_request(RpcRequest::new(3, "hark.status", None))
            .await
            .unwrap();
        let status_2: HarkStatusResult =
            serde_json::from_value(status_res_2.result.unwrap()).unwrap();
        assert_eq!(status_2.spellbook_count, 2);

        // 4. Dictate prompt
        let prompt_req = RpcRequest::new(
            4,
            "hark.dictate_prompt",
            Some(json!({
                "text": "run cargo check across all workspaces",
                "isFinal": true,
                "confidence": 0.95
            })),
        );
        let prompt_res = router.handle_request(prompt_req).await.unwrap();
        assert!(prompt_res.is_success());
        let prompt_out: HarkDictatePromptResult =
            serde_json::from_value(prompt_res.result.unwrap()).unwrap();
        assert!(prompt_out.accepted);
        assert_eq!(prompt_out.text, "run cargo check across all workspaces");
    }

    #[tokio::test]
    async fn router_handles_worktree_sync_delta() {
        let origin = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();

        let run = |dir: &std::path::Path, args: &[&str]| {
            let res = std::process::Command::new("git")
                .current_dir(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(res.status.success());
        };

        run(origin.path(), &["init"]);
        run(origin.path(), &["config", "user.name", "Tester"]);
        run(origin.path(), &["config", "user.email", "test@example.com"]);
        std::fs::write(origin.path().join("code.rs"), "fn main() {}").unwrap();
        run(origin.path(), &["add", "code.rs"]);
        run(origin.path(), &["commit", "-m", "add code"]);

        run(target.path(), &["init"]);
        run(target.path(), &["config", "user.name", "Tester"]);
        run(target.path(), &["config", "user.email", "test@example.com"]);

        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env_local".to_string());

        let sync_req = RpcRequest::new(
            1,
            "worktree.sync_delta",
            Some(json!({
                "localPath": origin.path().to_string_lossy().to_string(),
                "remotePath": target.path().to_string_lossy().to_string(),
                "revRange": "HEAD",
                "targetBranch": "synced-main"
            })),
        );

        let sync_res = router.handle_request(sync_req).await.unwrap();
        assert!(sync_res.is_success());
        let sync_out: WorktreeSyncDeltaResult =
            serde_json::from_value(sync_res.result.unwrap()).unwrap();
        assert!(sync_out.synced);
        assert_eq!(sync_out.target_branch, "synced-main");
        assert!(sync_out.bundle_size_bytes > 0);
    }
}
