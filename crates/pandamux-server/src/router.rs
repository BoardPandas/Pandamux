use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pandamux_core::{
    AccessMode, AgentId, CAPACITY_SNAPSHOT_SCHEMA_VERSION, CapacityConfidence,
    CapacityEvidenceScope, CapacitySnapshot, DEFAULT_CAPACITY_MAX_AGE_MS, EnvironmentId,
    OrganizationPolicy, ProviderInstanceId, ProviderKind, RunId, ScheduleApprovalPolicy,
    ScheduleRecord, ScheduleTarget, SubscriptionKey, ThreadEventKind, ThreadOrigin,
    UsageSubscription, UserSettings,
};
use pandamux_protocol::antigravity_rpc::{
    AntigravityDiscardUploadParams, AntigravityInstallParams, AntigravityOAuthRelayParams,
    AntigravityOAuthRelayResult, AntigravityOAuthStartParams, AntigravityOAuthStartResult,
    AntigravityPrepareUploadParams, AntigravityPrepareUploadResult, AntigravityStatusParams,
    AntigravityStatusResult,
};
use pandamux_protocol::{
    AttachmentGetParams, AttachmentImportPathParams, AttachmentListParams,
    AttachmentPutChunkParams, CapacityPushParams, CapacityPushResult, CapacityQueryParams,
    CapacityRefreshParams, CapacityRefreshResult, CheckpointDiffParams, CheckpointListParams,
    CheckpointRollbackParams, EnvironmentImportSshConfigParams, EnvironmentImportSshConfigResult,
    EnvironmentListResult, GitCommitParams, GitCreatePrParams, GitPushParams, GitStatusParams,
    HarkDictatePromptParams, HarkSpellbookSyncParams, HelloParams, HelloResult, HubRoutingContext,
    IdentifyResult, McpCallToolParams, PROTOCOL_VERSION, PingResult, ProviderHealthParams,
    ProviderHealthReport, ProviderHealthResult, ProviderLoginFlow, ProviderLoginGuidance,
    ProviderLoginLaunchParams, ProviderLoginLaunchResult, ProviderLoginPreview,
    ProviderLoginPreviewParams, RoutingProviderCatalogEntry, RpcError, RpcRequest, RpcResponse,
    ScheduleRunParams, ScheduleRunResult, ScheduleSaveParams, ScheduleValidateParams,
    ScheduleValidationResult, ServerCapabilities, ServerRole, SettingsGetParams, SettingsGetResult,
    SettingsSetParams, SubAgentTreeItem, SubAgentTreeParams, SubAgentTreeResult,
    TerminalAttachParams, TerminalCloseParams, TerminalInputParams, TerminalOpenParams,
    TerminalResizeParams, ThreadCancelTurnParams, ThreadContinueParams, ThreadCreateParams,
    ThreadGetParams, ThreadListParams, ThreadRespondApprovalParams, ThreadResumeParams,
    ThreadSendTurnParams, UsageAckParams, UsageAckResult, UsageBackfillParams, UsageCatalogResult,
    UsageExportParams, UsageQueryParams, UsageSubscriptionSetParams, WorktreeSyncDeltaParams,
    WorktreeSyncDeltaResult,
};

use crate::attachment::AttachmentManager;
use crate::driver_registry::DriverRegistry;
use crate::mcp_server::McpServer;
use crate::provider_resolver::ProviderResolver;
use crate::request_context::{CapabilityAuthority, PeerRole, RequestContext};
use crate::store::Store;
use crate::terminal::TerminalServerManager;
use crate::thread_manager::ThreadManager;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

pub struct Router {
    store: Store,
    mcp: McpServer,
    thread_manager: ThreadManager,
    attachments: AttachmentManager,
    notifications: Arc<tokio::sync::Mutex<Vec<pandamux_core::notification::NotificationInfo>>>,
    drivers: Arc<DriverRegistry>,
    provider_resolver: Arc<ProviderResolver>,
    antigravity_auth: Arc<pandamux_providers::antigravity::AntigravityAuthManager>,
    capability_authority: Arc<CapabilityAuthority>,
    terminal: TerminalServerManager,
    environment_router: Arc<crate::environment_router::EnvironmentRouter>,
    remote_terminals: Arc<std::sync::Mutex<HashMap<String, pandamux_core::ids::EnvironmentId>>>,
    login_previews: Arc<std::sync::Mutex<HashMap<String, LoginPreviewRecord>>>,
    hark: Arc<crate::hark::HarkBridge>,
    data_dir: PathBuf,
    role: ServerRole,
    environment_id: String,
    server_version: String,
    default_drivers: bool,
}

#[derive(Clone)]
struct LoginPreviewRecord {
    preview: pandamux_protocol::ProviderLoginPreview,
    environment: HashMap<String, String>,
    shell: Option<String>,
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
    let label = label.and_then(mask_account_label);
    let sub = sub.and_then(mask_account_label);
    match (label, sub) {
        (Some(l), Some(s)) if !l.is_empty() && !s.is_empty() && l != s => {
            Some(format!("{l} ({s})"))
        }
        (Some(l), _) if !l.is_empty() => Some(l),
        (_, Some(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

fn mask_account_label(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some((local, domain)) = value.split_once('@') {
        let prefix = local
            .chars()
            .next()
            .map_or("*".to_string(), |first| format!("{first}***"));
        return Some(format!("{prefix}@{domain}"));
    }
    let prefix = value.chars().take(2).collect::<String>();
    Some(format!("{prefix}***"))
}

fn provider_login_guidance(provider: ProviderKind) -> ProviderLoginGuidance {
    match provider {
        ProviderKind::Claude | ProviderKind::Codex => ProviderLoginGuidance {
            flow: ProviderLoginFlow::Terminal,
            available: true,
            summary: "Preview the isolated profile and exact CLI login command before launch"
                .to_string(),
            unavailable_reason: None,
        },
        ProviderKind::Antigravity => ProviderLoginGuidance {
            flow: ProviderLoginFlow::Browser,
            available: true,
            summary: "Use the explicit Antigravity OAuth action for this isolated profile"
                .to_string(),
            unavailable_reason: None,
        },
        _ => ProviderLoginGuidance {
            flow: ProviderLoginFlow::Manual,
            available: false,
            summary: "No verified non-destructive login launcher is available".to_string(),
            unavailable_reason: Some("provider_login_flow_unverified".to_string()),
        },
    }
}

#[cfg(windows)]
fn render_login_command(program: &str, args: &[String]) -> String {
    let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
    std::iter::once("&".to_string())
        .chain(std::iter::once(quote(program)))
        .chain(args.iter().map(|arg| quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(not(windows))]
fn render_login_command(program: &str, args: &[String]) -> String {
    let quote = |value: &str| format!("'{}'", value.replace('\'', "'\"'\"'"));
    std::iter::once(quote(program))
        .chain(args.iter().map(|arg| quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn login_shell() -> Result<Option<String>, RpcError> {
    #[cfg(windows)]
    {
        return pandamux_term::shell::resolve_powershell()
            .map(Some)
            .ok_or_else(|| {
                RpcError::with_data(
                    -32020,
                    "PowerShell is required for a safe provider login terminal",
                    json!({
                        "reason": "provider_login_shell_missing",
                        "retryable": true,
                    }),
                )
            });
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

fn login_terminal_input(display_command: &str, windows: bool) -> String {
    let submit = if windows { '\r' } else { '\n' };
    format!("{display_command}{submit}")
}

fn provider_health_report_from_snapshot(
    environment_id: &EnvironmentId,
    config: pandamux_core::provider_config::ProviderInstanceConfig,
    snapshot: pandamux_providers::ProviderSnapshot,
) -> ProviderHealthReport {
    use pandamux_providers::{ProviderAuthConfidence, ProviderAuthStatus, ProviderHealth};

    let (authentication_state, account_badge, subscription) = match &snapshot.auth {
        ProviderAuthStatus::Authenticated {
            account_label,
            email,
            subscription,
        } => (
            "authenticated",
            account_badge_label(
                account_label.as_deref().or(email.as_deref()),
                subscription.as_deref(),
            ),
            subscription.as_deref().and_then(mask_account_label),
        ),
        ProviderAuthStatus::Configured => ("configured", None, None),
        ProviderAuthStatus::Unauthenticated => ("unauthenticated", None, None),
        ProviderAuthStatus::Unknown => ("unknown", None, None),
    };
    let authentication_confidence = match snapshot.auth_confidence {
        ProviderAuthConfidence::Reported => "reported",
        ProviderAuthConfidence::Detected => "detected",
        ProviderAuthConfidence::Unknown => "unknown",
    };
    let (runtime_status, runtime_reason) = match &snapshot.health {
        ProviderHealth::Healthy => ("healthy", None),
        ProviderHealth::Degraded { reason } => ("degraded", Some(reason.clone())),
        ProviderHealth::Unavailable { reason } => ("unavailable", Some(reason.clone())),
    };
    let (status, reason) = match &snapshot.health {
        ProviderHealth::Unavailable { .. } => ("unavailable", runtime_reason),
        ProviderHealth::Degraded { .. } => ("degraded", runtime_reason),
        ProviderHealth::Healthy => match &snapshot.auth {
            ProviderAuthStatus::Authenticated { .. } => ("healthy", None),
            ProviderAuthStatus::Configured => (
                "configured",
                Some("Credentials were detected but authentication was not verified".to_string()),
            ),
            ProviderAuthStatus::Unauthenticated => {
                ("unauthenticated", Some("Not logged in".to_string()))
            }
            ProviderAuthStatus::Unknown => (
                "unknown",
                Some("Authentication state is unknown".to_string()),
            ),
        },
    };

    ProviderHealthReport {
        environment_id: environment_id.clone(),
        instance_id: config.id,
        provider: config.provider,
        display_name: config.display_name,
        enabled: config.enabled,
        installed: snapshot.installed,
        resolved_executable: snapshot.resolved_executable,
        version: snapshot.version,
        status: status.to_string(),
        authentication_state: authentication_state.to_string(),
        authentication_confidence: authentication_confidence.to_string(),
        runtime_status: runtime_status.to_string(),
        runtime_code: snapshot.runtime_code,
        account_badge,
        subscription,
        reason,
        disabled_reason: None,
        checked_at_ms: snapshot.checked_at_ms,
        expires_at_ms: snapshot.expires_at_ms,
        last_success_at_ms: None,
        login_guidance: Some(provider_login_guidance(config.provider)),
        retryable: true,
    }
}

fn capacity_snapshots_from_provider_report(
    environment_id: &EnvironmentId,
    config: &pandamux_core::provider_config::ProviderInstanceConfig,
    subscription_key: pandamux_core::SubscriptionKey,
    report: Option<pandamux_providers::UsageLimits>,
    observed_at_ms: u64,
) -> Vec<CapacitySnapshot> {
    let Some(report) = report else {
        return vec![CapacitySnapshot {
            schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION,
            subscription_key,
            environment_id: Some(environment_id.clone()),
            provider_instance_id: Some(config.id.clone()),
            window_name: None,
            window_start_ms: None,
            window_end_ms: None,
            used_millionths: None,
            remaining_millionths: None,
            reset_at_ms: None,
            source: if config.enabled {
                format!("{}_capacity_unreported", config.provider.as_str())
            } else {
                "provider_disabled".to_string()
            },
            confidence: CapacityConfidence::Unknown,
            evidence_scope: CapacityEvidenceScope::Local,
            hard_limit: false,
            provider_version: None,
            as_of_ms: observed_at_ms,
            max_age_ms: DEFAULT_CAPACITY_MAX_AGE_MS,
        }];
    };

    let as_of_ms = if report.as_of_ms == 0 {
        observed_at_ms
    } else {
        report.as_of_ms
    };
    if report.limits.is_empty() {
        return vec![CapacitySnapshot {
            schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION,
            subscription_key,
            environment_id: Some(environment_id.clone()),
            provider_instance_id: Some(config.id.clone()),
            window_name: None,
            window_start_ms: None,
            window_end_ms: None,
            used_millionths: None,
            remaining_millionths: None,
            reset_at_ms: None,
            source: report.source,
            confidence: CapacityConfidence::Unknown,
            evidence_scope: CapacityEvidenceScope::Local,
            hard_limit: false,
            provider_version: report.provider_version,
            as_of_ms,
            max_age_ms: DEFAULT_CAPACITY_MAX_AGE_MS,
        }];
    }

    report
        .limits
        .into_iter()
        .map(|limit| {
            let duration_ms = u64::from(limit.window_duration_mins).saturating_mul(60 * 1000);
            let window_start_ms = limit
                .resets_at_ms
                .map(|reset| reset.saturating_sub(duration_ms));
            CapacitySnapshot {
                schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION,
                subscription_key: subscription_key.clone(),
                environment_id: Some(environment_id.clone()),
                provider_instance_id: Some(config.id.clone()),
                window_name: Some(limit.limit_name),
                window_start_ms,
                window_end_ms: limit.resets_at_ms,
                used_millionths: limit.used_millionths,
                remaining_millionths: limit.remaining_millionths,
                reset_at_ms: limit.resets_at_ms,
                source: report.source.clone(),
                confidence: report.confidence,
                evidence_scope: CapacityEvidenceScope::Local,
                hard_limit: limit.hard_limit
                    && limit.remaining_millionths == Some(0)
                    && limit.resets_at_ms.is_some(),
                provider_version: report.provider_version.clone(),
                as_of_ms,
                max_age_ms: DEFAULT_CAPACITY_MAX_AGE_MS,
            }
        })
        .collect()
}

fn parse_usage_params<T>(params: Value, method: &str) -> Result<T, RpcError>
where
    T: serde::de::DeserializeOwned + Default,
{
    if params.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(params).map_err(|error| {
        usage_params_error(
            "usage_params_invalid",
            format!("Invalid {method} params: {error}"),
        )
    })
}

fn usage_params_error(reason: &str, message: impl Into<String>) -> RpcError {
    RpcError::with_data(
        -32602,
        message,
        json!({
            "reason": reason,
        }),
    )
}

fn organization_policy_rpc_error(
    project_id: &pandamux_core::ProjectId,
    message: impl Into<String>,
) -> RpcError {
    RpcError::with_data(
        -32030,
        message,
        json!({
            "reason": "organization_policy_denied",
            "projectId": project_id,
        }),
    )
}

fn require_local_query_peer(context: &RequestContext) -> Result<(), RpcError> {
    if matches!(
        context.peer_role,
        PeerRole::Internal | PeerRole::LocalControl
    ) {
        Ok(())
    } else {
        Err(RpcError::with_data(
            -32001,
            "Usage queries require authenticated local control",
            json!({ "reason": "usage_local_control_required" }),
        ))
    }
}

fn provider_rpc_error(error: pandamux_providers::ProviderError) -> RpcError {
    match error {
        pandamux_providers::ProviderError::InstallationError {
            operation,
            code,
            message,
        } => RpcError::with_data(
            -32020,
            message,
            serde_json::json!({
                "provider": "antigravity",
                "providerCode": code,
                "operation": operation,
            }),
        ),
        other => RpcError::with_data(
            -32021,
            other.to_string(),
            serde_json::json!({
                "provider": "antigravity",
                "providerCode": "antigravity_operation_failed",
            }),
        ),
    }
}

fn provider_rpc_error_for_target(
    error: pandamux_providers::ProviderError,
    instance_id: &pandamux_core::ProviderInstanceId,
    environment_id: &EnvironmentId,
) -> RpcError {
    let mut error = provider_rpc_error(error);
    let mut data = error
        .data
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    data.insert(
        "providerInstanceId".to_string(),
        Value::String(instance_id.as_str().to_string()),
    );
    data.insert(
        "environmentId".to_string(),
        Value::String(environment_id.as_str().to_string()),
    );
    error.data = Some(Value::Object(data));
    error
}

impl Router {
    pub fn new(store: Store, role: ServerRole, environment_id: String) -> Self {
        let data_dir = default_data_dir();
        let drivers = Arc::new(DriverRegistry::default_local_at(data_dir.clone()));
        let mut router = Self::with_drivers(store, role, environment_id, drivers);
        router.default_drivers = true;
        router.with_data_dir(data_dir)
    }

    pub fn with_drivers(
        store: Store,
        role: ServerRole,
        environment_id: String,
        drivers: Arc<DriverRegistry>,
    ) -> Self {
        let data_dir = default_data_dir();
        let capability_authority = Arc::new(CapabilityAuthority::new(EnvironmentId::new(
            environment_id.clone(),
        )));
        let mcp = McpServer::new(store.clone());
        let provider_resolver = Arc::new(ProviderResolver::new(
            store.clone(),
            drivers.clone(),
            EnvironmentId::new(environment_id.clone()),
            data_dir.clone(),
        ));
        let thread_manager = ThreadManager::new(
            store.clone(),
            provider_resolver.clone(),
            environment_id.clone(),
            capability_authority.clone(),
        );
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
            provider_resolver,
            antigravity_auth: Arc::new(
                pandamux_providers::antigravity::AntigravityAuthManager::new(),
            ),
            capability_authority,
            terminal,
            environment_router,
            remote_terminals: Arc::new(std::sync::Mutex::new(HashMap::new())),
            login_previews: Arc::new(std::sync::Mutex::new(HashMap::new())),
            hark: Arc::new(crate::hark::HarkBridge::default()),
            data_dir,
            role,
            environment_id,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            default_drivers: false,
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
        if self.default_drivers {
            let drivers = Arc::new(DriverRegistry::default_local_at(dir.clone()));
            let provider_resolver = Arc::new(ProviderResolver::new(
                self.store.clone(),
                drivers.clone(),
                EnvironmentId::new(self.environment_id.clone()),
                dir.clone(),
            ));
            self.thread_manager = ThreadManager::new(
                self.store.clone(),
                provider_resolver.clone(),
                self.environment_id.clone(),
                self.capability_authority.clone(),
            );
            self.drivers = drivers;
            self.provider_resolver = provider_resolver;
        } else {
            self.provider_resolver.set_data_dir(dir.clone());
        }
        self.data_dir = dir;
        self
    }

    pub fn provider_resolver(&self) -> &Arc<ProviderResolver> {
        &self.provider_resolver
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    pub async fn shutdown_antigravity_auth(&self) {
        self.antigravity_auth.shutdown().await;
    }

    pub fn capability_authority(&self) -> &Arc<CapabilityAuthority> {
        &self.capability_authority
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

    async fn forward_remote_antigravity_install(
        &self,
        environment_id: &EnvironmentId,
        request: &RpcRequest,
    ) -> Result<RpcResponse, RpcError> {
        let params: AntigravityInstallParams = serde_json::from_value(
            request.params.clone().unwrap_or(Value::Null),
        )
        .map_err(|error| {
            RpcError::invalid_params(format!("Invalid antigravity.install params: {error}"))
        })?;
        if params.staged_archive_path.is_some() {
            return Err(RpcError::unauthorized(
                "A control client cannot choose a remote staged archive path",
            ));
        }

        let node_download = self
            .environment_router
            .forward_request(environment_id, request)
            .await?;
        let fallback_allowed = node_download
            .error
            .as_ref()
            .and_then(|error| error.data.as_ref())
            .and_then(|data| data.get("providerCode"))
            .and_then(Value::as_str)
            == Some("antigravity_node_download_failed");
        if !fallback_allowed {
            return Ok(node_download);
        }

        let archive =
            pandamux_providers::antigravity::cache_bundle_archive(&self.data_dir, &params.manifest)
                .await
                .map_err(provider_rpc_error)?;
        let prepare_request = RpcRequest::new(
            Self::now_ms() as i64,
            "antigravity.prepare_upload",
            Some(
                serde_json::to_value(AntigravityPrepareUploadParams {
                    environment_id: Some(environment_id.clone()),
                    manifest: params.manifest.clone(),
                })
                .map_err(|error| RpcError::internal_error(error.to_string()))?,
            ),
        );
        let prepare_response = self
            .environment_router
            .forward_request(environment_id, &prepare_request)
            .await?;
        if let Some(error) = prepare_response.error {
            return Err(error);
        }
        let prepared: AntigravityPrepareUploadResult = serde_json::from_value(
            prepare_response.result.unwrap_or(Value::Null),
        )
        .map_err(|error| {
            RpcError::internal_error(format!(
                "Remote Antigravity upload preparation returned invalid data: {error}"
            ))
        })?;
        if let Err(upload_error) = self
            .environment_router
            .upload_file(environment_id, &archive, &prepared.remote_path)
            .await
        {
            let discard_request = RpcRequest::new(
                Self::now_ms() as i64,
                "antigravity.discard_upload",
                Some(
                    serde_json::to_value(AntigravityDiscardUploadParams {
                        environment_id: Some(environment_id.clone()),
                        staged_archive_path: prepared.remote_path.clone(),
                    })
                    .map_err(|error| RpcError::internal_error(error.to_string()))?,
                ),
            );
            let _ = self
                .environment_router
                .forward_request(environment_id, &discard_request)
                .await;
            return Err(upload_error);
        }

        let staged_path = prepared.remote_path.clone();
        let staged_request = RpcRequest {
            jsonrpc: request.jsonrpc.clone(),
            id: request.id.clone(),
            method: request.method.clone(),
            params: Some(
                serde_json::to_value(AntigravityInstallParams {
                    environment_id: Some(environment_id.clone()),
                    manifest: params.manifest,
                    staged_archive_path: Some(staged_path.clone()),
                    force: params.force,
                })
                .map_err(|error| RpcError::internal_error(error.to_string()))?,
            ),
        };
        let response = self
            .environment_router
            .forward_request(environment_id, &staged_request)
            .await;
        let discard_request = RpcRequest::new(
            Self::now_ms() as i64,
            "antigravity.discard_upload",
            Some(
                serde_json::to_value(AntigravityDiscardUploadParams {
                    environment_id: Some(environment_id.clone()),
                    staged_archive_path: staged_path,
                })
                .map_err(|error| RpcError::internal_error(error.to_string()))?,
            ),
        );
        let _ = self
            .environment_router
            .forward_request(environment_id, &discard_request)
            .await;
        response
    }

    fn resolve_authorized_project_provider(
        &self,
        project_id: &pandamux_core::ProjectId,
        environment_id: &EnvironmentId,
        requested: Option<&ProviderInstanceId>,
    ) -> Result<(OrganizationPolicy, ProviderInstanceId), RpcError> {
        let policy = self
            .store
            .organization_policy_for_project(project_id)
            .map_err(|error| organization_policy_rpc_error(project_id, error.to_string()))?;
        if let Some(requested) = requested {
            self.store
                .authorize_provider_for_project(project_id, environment_id, requested)
                .map_err(|error| organization_policy_rpc_error(project_id, error.to_string()))?;
            self.provider_resolver
                .resolve(requested, environment_id)
                .map_err(|_| {
                    organization_policy_rpc_error(
                        project_id,
                        "allowed named provider is unavailable in the target environment",
                    )
                })?;
            return Ok((policy, requested.clone()));
        }

        let selected = policy
            .allowed_provider_instances
            .iter()
            .find(|candidate| {
                self.store
                    .authorize_provider_for_project(project_id, environment_id, candidate)
                    .is_ok()
                    && self
                        .provider_resolver
                        .resolve(candidate, environment_id)
                        .is_ok()
            })
            .cloned()
            .ok_or_else(|| {
                organization_policy_rpc_error(
                    project_id,
                    "organization has no enabled provider with an allowed named subscription",
                )
            })?;
        Ok((policy, selected))
    }

    fn prepare_forwarded_thread_create(
        &self,
        params: &Value,
        target_environment: &EnvironmentId,
    ) -> Result<Value, RpcError> {
        let mut params: ThreadCreateParams = serde_json::from_value(params.clone())
            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
        if let Some(project_id) = params.project_id.as_ref() {
            let (_, provider_instance_id) = self.resolve_authorized_project_provider(
                project_id,
                target_environment,
                params.provider_instance_id.as_ref(),
            )?;
            params.provider_instance_id = Some(provider_instance_id);
        }
        params.environment_id = Some(target_environment.clone());
        serde_json::to_value(params).map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn routing_context_for_forwarded_thread(
        &self,
        thread: &pandamux_core::Thread,
        target_environment: &EnvironmentId,
    ) -> Result<Option<HubRoutingContext>, RpcError> {
        if &thread.environment_id != target_environment {
            return Err(RpcError::with_data(
                -32030,
                "Thread environment disagrees with the routed environment",
                json!({ "reason": "organization_policy_environment_mismatch" }),
            ));
        }
        if let Some(project_id) = thread.project_id.as_ref() {
            let (organization, _) = self.resolve_authorized_project_provider(
                project_id,
                target_environment,
                Some(&thread.provider_instance_id),
            )?;
            let failover_pools = organization
                .failover_pool_ids
                .iter()
                .map(|pool_id| {
                    self.store
                        .get_failover_pool(pool_id)
                        .map_err(|error| RpcError::internal_error(error.to_string()))?
                        .ok_or_else(|| {
                            organization_policy_rpc_error(
                                project_id,
                                "organization references a missing failover pool",
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Some(HubRoutingContext {
                organization,
                failover_pools,
            }));
        }
        Ok(None)
    }

    fn prepare_forwarded_thread_turn(
        &self,
        params: &Value,
        target_environment: &EnvironmentId,
    ) -> Result<Value, RpcError> {
        let mut params: ThreadSendTurnParams = serde_json::from_value(params.clone())
            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .ok_or_else(|| RpcError::not_found("Thread not found"))?;
        params.hub_routing_context =
            self.routing_context_for_forwarded_thread(&thread, target_environment)?;
        serde_json::to_value(params).map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn prepare_forwarded_continuation(
        &self,
        params: &Value,
        target_environment: &EnvironmentId,
    ) -> Result<Value, RpcError> {
        let mut params: ThreadContinueParams = serde_json::from_value(params.clone())
            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
        let thread_id = params.thread_id.as_ref().ok_or_else(|| {
            RpcError::invalid_params("Remote capacity continuation requires threadId")
        })?;
        let thread = self
            .store
            .get_thread(thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .ok_or_else(|| RpcError::not_found("Thread not found"))?;
        params.hub_routing_context =
            self.routing_context_for_forwarded_thread(&thread, target_environment)?;
        serde_json::to_value(params).map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn resolve_target_environment(
        &self,
        method: &str,
        params: &Value,
    ) -> Option<pandamux_core::ids::EnvironmentId> {
        if matches!(
            method,
            "usage.export"
                | "usage.backfill"
                | "usage.ack"
                | "usage.query"
                | "usage.catalog"
                | "usage.subscription.set"
                | "capacity.query"
        ) {
            return None;
        }

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

        // 3. Capacity continuation methods: resolve through the durable record.
        let continuation_id_opt = params
            .get("continuationId")
            .or_else(|| params.get("continuation_id"))
            .and_then(|v| v.as_str());
        if let Some(continuation_id) = continuation_id_opt {
            let continuation_id = pandamux_core::ContinuationId::new(continuation_id);
            if let Ok(Some(continuation)) = self.store.get_continuation(&continuation_id)
                && let Ok(Some(thread)) = self.store.get_thread(&continuation.thread_id)
            {
                return Some(thread.environment_id);
            }
        }

        // 4. Terminal-scoped methods: look up terminal_id in remote_terminals map
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
        let context = RequestContext::internal(EnvironmentId::new(self.environment_id.clone()));
        self.handle_request_with_context(&context, req).await
    }

    pub async fn handle_request_with_context(
        &self,
        context: &RequestContext,
        req: RpcRequest,
    ) -> Option<RpcResponse> {
        let is_notification = req.is_notification();
        let id = req.id.clone();
        let method = req.method.clone();
        let params = req.params.clone().unwrap_or(Value::Null);

        if let Err(error) = context.authorize(&method, &params) {
            if is_notification {
                return None;
            }
            return Some(RpcResponse::error(id, error));
        }

        // Hub environment routing: if targeted to a remote environment, forward across transport
        let requires_hub_preprocessing = matches!(
            method.as_str(),
            "attachment.import_path"
                | "attachment.importPath"
                | "worktree.sync_delta"
                | "worktree.syncDelta"
        );
        if self.role == ServerRole::Hub
            && !requires_hub_preprocessing
            && let Some(target_env) = self.resolve_target_environment(&method, &params)
            && !self.environment_router.is_local(&Some(target_env.clone()))
        {
            let request_to_forward = if method == "thread.create" {
                match self.prepare_forwarded_thread_create(&params, &target_env) {
                    Ok(params) => RpcRequest {
                        params: Some(params),
                        ..req.clone()
                    },
                    Err(error) => {
                        if is_notification {
                            return None;
                        }
                        return Some(RpcResponse::error(id, error));
                    }
                }
            } else if method == "thread.send_turn" || method == "thread.sendTurn" {
                match self.prepare_forwarded_thread_turn(&params, &target_env) {
                    Ok(params) => RpcRequest {
                        params: Some(params),
                        ..req.clone()
                    },
                    Err(error) => {
                        if is_notification {
                            return None;
                        }
                        return Some(RpcResponse::error(id, error));
                    }
                }
            } else if method == "thread.continue_capacity" || method == "thread.continueCapacity" {
                match self.prepare_forwarded_continuation(&params, &target_env) {
                    Ok(params) => RpcRequest {
                        params: Some(params),
                        ..req.clone()
                    },
                    Err(error) => {
                        if is_notification {
                            return None;
                        }
                        return Some(RpcResponse::error(id, error));
                    }
                }
            } else {
                req.clone()
            };

            // Sync referenced attachments across tunnel before dispatching remote turn
            if (method == "thread.send_turn" || method == "thread.sendTurn")
                && let Some(tid_str) = params.get("threadId").and_then(|v| v.as_str())
            {
                let tid = pandamux_core::ids::ThreadId::new(tid_str);
                if let Some(att_array) = params
                    .get("attachmentIds")
                    .or_else(|| params.get("attachment_ids"))
                    .or_else(|| params.get("attachments"))
                    .and_then(|v| v.as_array())
                {
                    for att_val in att_array {
                        let att_id_opt = att_val
                            .as_str()
                            .or_else(|| att_val.get("id").and_then(|v| v.as_str()));
                        let Some(att_id) = att_id_opt else {
                            continue;
                        };
                        let stored_on_hub = self
                            .store
                            .get_attachment(&tid, att_id)
                            .ok()
                            .flatten()
                            .is_some();
                        if !stored_on_hub {
                            continue;
                        }
                        if let Err(error) = self
                            .sync_attachment_to_environment(&target_env, &tid, att_id)
                            .await
                        {
                            if is_notification {
                                return None;
                            }
                            return Some(RpcResponse::error(id, error));
                        }
                    }
                }
            }

            let forwarded = if method == "antigravity.install" {
                self.forward_remote_antigravity_install(&target_env, &request_to_forward)
                    .await
            } else {
                self.environment_router
                    .forward_request(&target_env, &request_to_forward)
                    .await
            };

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
                    if method == "provider.login.launch"
                        && let Some(val) = &resp.result
                        && let Ok(login_res) =
                            serde_json::from_value::<ProviderLoginLaunchResult>(val.clone())
                        && let Ok(mut map) = self.remote_terminals.lock()
                    {
                        map.insert(login_res.terminal_id, target_env.clone());
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
            "thread.create" => self.handle_thread_create(context, params),
            "thread.get" => self.handle_thread_get(params),
            "thread.list" => self.handle_thread_list(params),
            "thread.send_turn" | "thread.sendTurn" => {
                self.handle_thread_send_turn(context, params).await
            }
            "thread.cancel_turn" | "thread.interrupt" => {
                self.handle_thread_cancel_turn(params).await
            }
            "thread.respond_approval" | "thread.respondApproval" => {
                self.handle_thread_respond_approval(params).await
            }
            "thread.continue_capacity" | "thread.continueCapacity" => {
                self.handle_thread_continue_capacity(context, params).await
            }
            "thread.resume" => self.handle_thread_resume(params),
            "settings.get" | "config.get" => self.handle_settings_get(params),
            "settings.set" | "config.set" => self.handle_settings_set(params),
            "provider.health" | "provider.probe" => self.handle_provider_health(params).await,
            "provider.login.preview" => self.handle_provider_login_preview(params).await,
            "provider.login.launch" => self.handle_provider_login_launch(params),
            "usage.export" => self.handle_usage_export(context, params),
            "usage.backfill" => self.handle_usage_backfill(context, params),
            "usage.ack" => self.handle_usage_ack(context, params),
            "usage.query" => self.handle_usage_query(context, params),
            "usage.catalog" => self.handle_usage_catalog(context),
            "usage.subscription.set" => self.handle_usage_subscription_set(context, params),
            "capacity.query" => self.handle_capacity_query(context, params),
            "capacity.refresh" => self.handle_capacity_refresh(context, params).await,
            "capacity.push" => self.handle_capacity_push(context, params),
            "schedule.validate" => self.handle_schedule_validate(context, params),
            "schedule.save" => self.handle_schedule_save(context, params),
            "schedule.run" => self.handle_schedule_run(context, params).await,
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
            "antigravity.status" => self.handle_antigravity_status(params).await,
            "antigravity.install" => self.handle_antigravity_install(context, params).await,
            "antigravity.prepare_upload" | "antigravity.prepareUpload" => {
                self.handle_antigravity_prepare_upload(context, params)
            }
            "antigravity.discard_upload" | "antigravity.discardUpload" => {
                self.handle_antigravity_discard_upload(context, params)
            }
            "antigravity.oauth_start" | "antigravity.oauthStart" => {
                self.handle_antigravity_oauth_start(params).await
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

    fn handle_thread_create(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        let p: ThreadCreateParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let thread = if matches!(context.peer_role, PeerRole::NodeProxy) {
            if p.project_id.is_some() && p.provider_instance_id.is_none() {
                return Err(RpcError::with_data(
                    -32030,
                    "Hub-authorized project routes must pin a named provider",
                    json!({ "reason": "organization_policy_preauthorization_missing" }),
                ));
            }
            self.thread_manager.create_thread_from_hub(p)?
        } else {
            self.thread_manager.create_thread(p)?
        };
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

    async fn handle_thread_send_turn(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        let p: ThreadSendTurnParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = if matches!(context.peer_role, PeerRole::NodeProxy) {
            self.thread_manager.send_turn_from_hub(p).await?
        } else {
            if p.hub_routing_context.is_some() {
                return Err(RpcError::unauthorized(
                    "Hub routing context is accepted only from an authenticated node proxy",
                ));
            }
            self.thread_manager.send_turn(p).await?
        };
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

    async fn handle_thread_continue_capacity(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        let p: ThreadContinueParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let result = if matches!(context.peer_role, PeerRole::NodeProxy) {
            self.thread_manager
                .continue_after_capacity_from_hub(p)
                .await?
        } else {
            if p.hub_routing_context.is_some() {
                return Err(RpcError::unauthorized(
                    "Hub routing context is accepted only from an authenticated node proxy",
                ));
            }
            self.thread_manager.continue_after_capacity(p).await?
        };
        serde_json::to_value(&result).map_err(|e| RpcError::internal_error(e.to_string()))
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
                self.canonicalize_failover_pools(&mut updated)?;
                self.store
                    .save_settings_with_organization_policies(&updated, Self::now_ms())
                    .map_err(|e| RpcError::internal_error(e.to_string()))?;
                return Ok(json!({ "success": true }));
            } else if let (Some(key), Some(value)) = (p.key, p.value) {
                pandamux_core::settings_set(&mut current, &key, value)
                    .map_err(RpcError::invalid_params)?;
                current.normalize();
                self.canonicalize_failover_pools(&mut current)?;
                self.store
                    .save_settings_with_organization_policies(&current, Self::now_ms())
                    .map_err(|e| RpcError::internal_error(e.to_string()))?;
                return Ok(json!({ "success": true }));
            }
        }

        // 2. Try parsing directly as UserSettings
        if let Ok(mut settings) = serde_json::from_value::<UserSettings>(params) {
            settings.normalize();
            self.canonicalize_failover_pools(&mut settings)?;
            self.store
                .save_settings_with_organization_policies(&settings, Self::now_ms())
                .map_err(|e| RpcError::internal_error(e.to_string()))?;
            return Ok(json!({ "success": true }));
        }

        Err(RpcError::invalid_params("Invalid settings payload"))
    }

    fn canonicalize_failover_pools(&self, settings: &mut UserSettings) -> Result<(), RpcError> {
        for pool in &mut settings.failover_pools {
            let mut capabilities_changed = false;
            for member in &mut pool.members {
                let config = settings
                    .providers
                    .iter()
                    .find(|provider| provider.id == member.provider_instance_id)
                    .ok_or_else(|| {
                        RpcError::invalid_params(format!(
                            "Failover pool '{}' references a missing provider instance",
                            pool.display_name
                        ))
                    })?;
                if !config.enabled {
                    return Err(RpcError::invalid_params(format!(
                        "Failover pool '{}' references a disabled provider instance",
                        pool.display_name
                    )));
                }
                let driver = self.drivers.get(config.provider).ok_or_else(|| {
                    RpcError::invalid_params(format!(
                        "Failover pool '{}' references an unavailable provider driver",
                        pool.display_name
                    ))
                })?;
                let metadata = driver.metadata();
                let mut declared_models = metadata
                    .supported_models
                    .iter()
                    .map(|model| model.id.clone())
                    .collect::<Vec<_>>();
                if let Some(default_model) = config
                    .settings
                    .get("defaultModel")
                    .and_then(Value::as_str)
                    .filter(|model| !model.trim().is_empty())
                {
                    declared_models.push(default_model.to_string());
                }
                declared_models.sort();
                declared_models.dedup();
                if !declared_models.iter().any(|model| model == &member.model) {
                    return Err(RpcError::invalid_params(format!(
                        "Failover pool '{}' uses a model not declared by its provider driver",
                        pool.display_name
                    )));
                }
                if member.declared_capabilities != metadata.capabilities {
                    member.declared_capabilities = metadata.capabilities;
                    capabilities_changed = true;
                }
                pool.requirements
                    .validate_capabilities(metadata.capabilities)
                    .map_err(|reason| {
                        RpcError::invalid_params(format!(
                            "Failover pool '{}' has an incompatible member: {reason}",
                            pool.display_name
                        ))
                    })?;
            }
            if capabilities_changed {
                pool.revision = pool.revision.saturating_add(1);
            }
            pool.validate()
                .map_err(|error| RpcError::invalid_params(error.to_string()))?;
        }
        Ok(())
    }

    async fn handle_provider_health(&self, params: Value) -> Result<Value, RpcError> {
        let filter = if params.is_null() {
            ProviderHealthParams::default()
        } else {
            serde_json::from_value::<ProviderHealthParams>(params).map_err(|error| {
                RpcError::with_data(
                    -32602,
                    format!("Invalid provider.health params: {error}"),
                    json!({ "reason": "provider_health_params_invalid" }),
                )
            })?
        };
        filter.validate().map_err(|error| {
            RpcError::with_data(
                -32602,
                error,
                json!({ "reason": "provider_health_page_limit_invalid" }),
            )
        })?;
        let environment_id = filter
            .environment_id
            .clone()
            .unwrap_or_else(|| EnvironmentId::new(self.environment_id.clone()));
        let settings = self
            .store
            .get_settings()
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .unwrap_or_default();

        if let Some(instance_id) = filter.instance_id.as_ref()
            && !settings
                .providers
                .iter()
                .any(|config| &config.id == instance_id)
        {
            self.provider_resolver
                .resolve_allow_disabled(instance_id, &environment_id)?;
        }

        let mut instances = settings
            .providers
            .iter()
            .filter(|instance| {
                filter
                    .instance_id
                    .as_ref()
                    .is_none_or(|target_id| &instance.id == target_id)
            })
            .filter(|instance| {
                filter
                    .after_instance_id
                    .as_ref()
                    .is_none_or(|after| instance.id.as_str() > after.as_str())
            })
            .collect::<Vec<_>>();
        instances.sort_by(|left, right| left.id.cmp(&right.id));
        let has_more = instances.len() > filter.limit as usize;
        instances.truncate(filter.limit as usize);

        enum ProbeOutcome {
            Snapshot(pandamux_providers::ProviderSnapshot),
            TimedOut,
        }

        let semaphore = Arc::new(Semaphore::new(filter.max_concurrency as usize));
        let mut tasks = JoinSet::new();
        let mut reports = Vec::with_capacity(instances.len());
        let mut expected = HashMap::new();

        for inst in instances {
            expected.insert(
                inst.id.clone(),
                (inst.provider, inst.display_name.clone(), inst.enabled),
            );
            match self
                .provider_resolver
                .resolve_allow_disabled(&inst.id, &environment_id)
            {
                Ok(resolved) if !resolved.config.enabled => {
                    let disabled_reason = resolved.config.disabled_reason.clone();
                    let reason = disabled_reason
                        .as_ref()
                        .map(|reason| reason.message.clone())
                        .unwrap_or_else(|| "Provider instance is disabled".to_string());
                    reports.push(ProviderHealthReport {
                        environment_id: environment_id.clone(),
                        instance_id: resolved.config.id,
                        provider: resolved.config.provider,
                        display_name: resolved.config.display_name,
                        enabled: false,
                        installed: false,
                        resolved_executable: None,
                        version: None,
                        status: "disabled".to_string(),
                        authentication_state: "unknown".to_string(),
                        authentication_confidence: "unknown".to_string(),
                        runtime_status: "disabled".to_string(),
                        runtime_code: Some("provider_disabled".to_string()),
                        account_badge: None,
                        subscription: None,
                        reason: Some(reason),
                        disabled_reason,
                        checked_at_ms: Self::now_ms(),
                        expires_at_ms: Self::now_ms()
                            .saturating_add(pandamux_providers::PROVIDER_SNAPSHOT_TTL_MS),
                        last_success_at_ms: None,
                        login_guidance: Some(provider_login_guidance(resolved.config.provider)),
                        retryable: false,
                    });
                }
                Ok(resolved) => {
                    let semaphore = semaphore.clone();
                    let timeout = Duration::from_millis(filter.timeout_ms);
                    tasks.spawn(async move {
                        let permit = semaphore.acquire_owned().await;
                        let outcome = match permit {
                            Ok(_permit) => {
                                match tokio::time::timeout(
                                    timeout,
                                    resolved.driver.probe(&resolved.config),
                                )
                                .await
                                {
                                    Ok(snapshot) => ProbeOutcome::Snapshot(snapshot),
                                    Err(_) => ProbeOutcome::TimedOut,
                                }
                            }
                            Err(_) => ProbeOutcome::TimedOut,
                        };
                        (resolved.config, outcome)
                    });
                }
                Err(error) => {
                    let reason = error
                        .data
                        .as_ref()
                        .and_then(|data| data.get("reason"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| error.message.clone());
                    reports.push(ProviderHealthReport {
                        environment_id: environment_id.clone(),
                        instance_id: inst.id.clone(),
                        provider: inst.provider,
                        display_name: inst.display_name.clone(),
                        enabled: inst.enabled,
                        installed: false,
                        resolved_executable: None,
                        version: None,
                        status: "unavailable".to_string(),
                        authentication_state: "unknown".to_string(),
                        authentication_confidence: "unknown".to_string(),
                        runtime_status: "unavailable".to_string(),
                        runtime_code: Some("provider_resolution_failed".to_string()),
                        account_badge: None,
                        subscription: None,
                        reason: Some(reason),
                        disabled_reason: None,
                        checked_at_ms: Self::now_ms(),
                        expires_at_ms: Self::now_ms()
                            .saturating_add(pandamux_providers::PROVIDER_SNAPSHOT_TTL_MS),
                        last_success_at_ms: None,
                        login_guidance: Some(provider_login_guidance(inst.provider)),
                        retryable: true,
                    });
                }
            }
        }

        while let Some(result) = tasks.join_next().await {
            let Ok((config, outcome)) = result else {
                continue;
            };
            let report = match outcome {
                ProbeOutcome::Snapshot(snapshot) => {
                    provider_health_report_from_snapshot(&environment_id, config, snapshot)
                }
                ProbeOutcome::TimedOut => {
                    let checked_at_ms = Self::now_ms();
                    ProviderHealthReport {
                        environment_id: environment_id.clone(),
                        instance_id: config.id,
                        provider: config.provider,
                        display_name: config.display_name,
                        enabled: config.enabled,
                        installed: config.binary_path.is_some(),
                        resolved_executable: config.binary_path,
                        version: None,
                        status: "unavailable".to_string(),
                        authentication_state: "unknown".to_string(),
                        authentication_confidence: "unknown".to_string(),
                        runtime_status: "timeout".to_string(),
                        runtime_code: Some("probe_timeout".to_string()),
                        account_badge: None,
                        subscription: None,
                        reason: Some(format!(
                            "Provider health probe exceeded {} ms",
                            filter.timeout_ms
                        )),
                        disabled_reason: None,
                        checked_at_ms,
                        expires_at_ms: checked_at_ms
                            .saturating_add(pandamux_providers::PROVIDER_SNAPSHOT_TTL_MS),
                        last_success_at_ms: None,
                        login_guidance: Some(provider_login_guidance(config.provider)),
                        retryable: true,
                    }
                }
            };
            reports.push(report);
        }

        for (instance_id, (provider, display_name, enabled)) in expected {
            if reports
                .iter()
                .any(|report| report.instance_id == instance_id)
            {
                continue;
            }
            let checked_at_ms = Self::now_ms();
            reports.push(ProviderHealthReport {
                environment_id: environment_id.clone(),
                instance_id,
                provider,
                display_name,
                enabled,
                installed: false,
                resolved_executable: None,
                version: None,
                status: "unavailable".to_string(),
                authentication_state: "unknown".to_string(),
                authentication_confidence: "unknown".to_string(),
                runtime_status: "unavailable".to_string(),
                runtime_code: Some("probe_task_failed".to_string()),
                account_badge: None,
                subscription: None,
                reason: Some("Provider health probe task failed".to_string()),
                disabled_reason: None,
                checked_at_ms,
                expires_at_ms: checked_at_ms
                    .saturating_add(pandamux_providers::PROVIDER_SNAPSHOT_TTL_MS),
                last_success_at_ms: None,
                login_guidance: Some(provider_login_guidance(provider)),
                retryable: true,
            });
        }

        reports.sort_by(|left, right| left.instance_id.cmp(&right.instance_id));
        for report in &mut reports {
            let previous = self
                .store
                .latest_provider_health_report(&report.environment_id, &report.instance_id)
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
            report.last_success_at_ms = if report.status == "healthy" {
                Some(report.checked_at_ms)
            } else {
                previous.and_then(|previous| previous.last_success_at_ms)
            };
            self.store
                .save_provider_health_report(report)
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
        }

        let next_instance_id = reports.last().map(|report| report.instance_id.clone());
        let res = ProviderHealthResult {
            reports,
            next_instance_id,
            has_more,
        };
        serde_json::to_value(&res).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_provider_login_preview(&self, params: Value) -> Result<Value, RpcError> {
        let params: ProviderLoginPreviewParams =
            serde_json::from_value(params).map_err(|error| {
                RpcError::invalid_params(format!("Invalid provider.login.preview params: {error}"))
            })?;
        let environment_id = params
            .environment_id
            .unwrap_or_else(|| EnvironmentId::new(self.environment_id.clone()));
        let resolved = self
            .provider_resolver
            .resolve_allow_disabled(&params.provider_instance_id, &environment_id)?;
        let args = match resolved.config.provider {
            ProviderKind::Claude => vec!["auth".to_string(), "login".to_string()],
            ProviderKind::Codex => vec!["login".to_string()],
            provider => {
                let guidance = provider_login_guidance(provider);
                return Err(RpcError::with_data(
                    -32020,
                    guidance.summary,
                    json!({
                        "reason": guidance.unavailable_reason
                            .unwrap_or_else(|| "provider_login_requires_dedicated_flow".to_string()),
                        "providerInstanceId": params.provider_instance_id,
                        "environmentId": environment_id,
                        "flow": guidance.flow,
                        "retryable": true,
                    }),
                ));
            }
        };
        let snapshot = tokio::time::timeout(
            Duration::from_millis(pandamux_protocol::DEFAULT_PROVIDER_PROBE_TIMEOUT_MS),
            resolved.driver.probe(&resolved.config),
        )
        .await
        .map_err(|_| {
            RpcError::with_data(
                -32020,
                "Provider login preview could not resolve the executable before its deadline",
                json!({
                    "reason": "provider_login_preview_timeout",
                    "providerInstanceId": params.provider_instance_id,
                    "environmentId": environment_id,
                    "retryable": true,
                }),
            )
        })?;
        let program = snapshot.resolved_executable.ok_or_else(|| {
            RpcError::with_data(
                -32020,
                "Provider executable is not installed or could not be resolved",
                json!({
                    "reason": "provider_login_executable_missing",
                    "providerInstanceId": params.provider_instance_id,
                    "environmentId": environment_id,
                    "retryable": true,
                }),
            )
        })?;
        if program.chars().any(char::is_control) {
            return Err(RpcError::with_data(
                -32020,
                "Resolved provider executable contains unsafe control characters",
                json!({
                    "reason": "provider_login_executable_invalid",
                    "retryable": false,
                }),
            ));
        }
        let display_command = render_login_command(&program, &args);
        let mut environment = resolved
            .config
            .env_overrides
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<HashMap<_, _>>();
        let profile = resolved.config.profile_dir.clone();
        let profile_key = match resolved.config.provider {
            ProviderKind::Claude => pandamux_providers::ENV_CLAUDE_CONFIG_DIR,
            ProviderKind::Codex => pandamux_providers::ENV_CODEX_HOME,
            _ => unreachable!("unsupported login provider was rejected"),
        };
        environment.insert(profile_key.to_string(), profile.clone());
        let now_ms = Self::now_ms();
        let preview = ProviderLoginPreview {
            preview_id: format!("login-preview-{}", uuid::Uuid::new_v4()),
            environment_id,
            provider_instance_id: resolved.config.id,
            provider: resolved.config.provider,
            profile,
            flow: ProviderLoginFlow::Terminal,
            display_command,
            expires_at_ms: now_ms.saturating_add(5 * 60 * 1000),
        };
        let record = LoginPreviewRecord {
            preview: preview.clone(),
            environment,
            shell: login_shell()?,
        };
        let mut previews = self
            .login_previews
            .lock()
            .map_err(|_| RpcError::internal_error("Provider login preview registry lock failed"))?;
        previews.retain(|_, candidate| candidate.preview.expires_at_ms > now_ms);
        previews.insert(preview.preview_id.clone(), record);
        serde_json::to_value(preview).map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn handle_provider_login_launch(&self, params: Value) -> Result<Value, RpcError> {
        let params: ProviderLoginLaunchParams =
            serde_json::from_value(params).map_err(|error| {
                RpcError::invalid_params(format!("Invalid provider.login.launch params: {error}"))
            })?;
        let environment_id = params
            .environment_id
            .unwrap_or_else(|| EnvironmentId::new(self.environment_id.clone()));
        let now_ms = Self::now_ms();
        let record = {
            let mut previews = self.login_previews.lock().map_err(|_| {
                RpcError::internal_error("Provider login preview registry lock failed")
            })?;
            previews.retain(|_, candidate| candidate.preview.expires_at_ms > now_ms);
            previews.get(&params.preview_id).cloned()
        }
        .ok_or_else(|| {
            RpcError::with_data(
                -32020,
                "Provider login preview is missing or expired",
                json!({
                    "reason": "provider_login_preview_expired",
                    "retryable": true,
                }),
            )
        })?;
        if record.preview.environment_id != environment_id
            || record.preview.provider_instance_id != params.provider_instance_id
        {
            return Err(RpcError::with_data(
                -32020,
                "Provider login launch does not match its preview",
                json!({
                    "reason": "provider_login_preview_mismatch",
                    "retryable": true,
                }),
            ));
        }
        let opened = self
            .terminal
            .open(TerminalOpenParams {
                terminal_id: None,
                environment_id: Some(environment_id.clone()),
                cwd: std::env::current_dir()
                    .ok()
                    .map(|path| path.to_string_lossy().to_string()),
                shell: record.shell,
                rows: Some(30),
                cols: Some(120),
                env: Some(record.environment),
            })
            .map_err(|error| {
                RpcError::with_data(
                    -32020,
                    format!("Failed to open provider login terminal: {error}"),
                    json!({
                        "reason": "provider_login_terminal_open_failed",
                        "retryable": true,
                    }),
                )
            })?;
        if let Err(error) = self.terminal.input(TerminalInputParams {
            terminal_id: opened.terminal_id.clone(),
            data: login_terminal_input(&record.preview.display_command, cfg!(windows)),
        }) {
            let _ = self.terminal.close(TerminalCloseParams {
                terminal_id: opened.terminal_id.clone(),
            });
            return Err(RpcError::with_data(
                -32020,
                format!("Failed to launch provider login command: {error}"),
                json!({
                    "reason": "provider_login_terminal_input_failed",
                    "terminalId": opened.terminal_id,
                    "retryable": true,
                }),
            ));
        }
        if let Ok(mut previews) = self.login_previews.lock() {
            previews.remove(&params.preview_id);
        }
        serde_json::to_value(ProviderLoginLaunchResult {
            environment_id,
            provider_instance_id: params.provider_instance_id,
            terminal_id: opened.terminal_id,
            display_command: record.preview.display_command,
            retryable: true,
        })
        .map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn handle_usage_export(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        self.require_hub_usage_peer(context)?;
        let params: UsageExportParams = parse_usage_params(params, "usage.export")?;
        params
            .validate()
            .map_err(|error| usage_params_error("usage_page_limit_invalid", error))?;
        let result = self
            .store
            .export_usage(&params)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        serde_json::to_value(result).map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn handle_usage_backfill(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        self.require_hub_usage_peer(context)?;
        let params: UsageBackfillParams = parse_usage_params(params, "usage.backfill")?;
        params
            .validate()
            .map_err(|error| usage_params_error("usage_backfill_invalid", error))?;
        let result = self
            .store
            .export_usage_backfill(&params)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        serde_json::to_value(result).map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn handle_usage_ack(&self, context: &RequestContext, params: Value) -> Result<Value, RpcError> {
        self.require_hub_usage_peer(context)?;
        let params: UsageAckParams = serde_json::from_value(params).map_err(|error| {
            usage_params_error(
                "usage_ack_params_invalid",
                format!("Invalid usage.ack params: {error}"),
            )
        })?;
        params
            .validate()
            .map_err(|error| usage_params_error("usage_ack_invalid", error))?;
        let outcome = self
            .store
            .acknowledge_and_compact_usage(
                params.through_seq,
                crate::store::UsageOutboxRetention::default(),
            )
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        serde_json::to_value(UsageAckResult {
            acknowledged: outcome.acknowledged,
            through_seq: params.through_seq,
            compacted: outcome.compacted,
            retained_from_seq: outcome.retained_from_seq,
        })
        .map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn handle_usage_query(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        require_local_query_peer(context)?;
        let params: UsageQueryParams = parse_usage_params(params, "usage.query")?;
        params
            .validate()
            .map_err(|error| usage_params_error("usage_query_invalid", error))?;
        let result = self
            .store
            .query_usage(&params)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        serde_json::to_value(result).map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn handle_usage_catalog(&self, context: &RequestContext) -> Result<Value, RpcError> {
        require_local_query_peer(context)?;
        let settings = self
            .store
            .get_settings()
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .unwrap_or_default();
        let mut routing_providers = Vec::new();
        for config in settings
            .providers
            .iter()
            .filter(|provider| provider.enabled)
        {
            let Some(driver) = self.drivers.get(config.provider) else {
                continue;
            };
            let metadata = driver.metadata();
            let mut models = metadata
                .supported_models
                .into_iter()
                .map(|model| model.id)
                .collect::<Vec<_>>();
            if let Some(default_model) = config
                .settings
                .get("defaultModel")
                .and_then(Value::as_str)
                .filter(|model| !model.trim().is_empty())
            {
                models.push(default_model.to_string());
            }
            models.sort();
            models.dedup();
            if models.is_empty() {
                continue;
            }
            routing_providers.push(RoutingProviderCatalogEntry {
                environment_id: config
                    .environment_id
                    .clone()
                    .unwrap_or_else(|| EnvironmentId::new(self.environment_id.clone())),
                provider_instance_id: config.id.clone(),
                provider: config.provider,
                display_name: config.display_name.clone(),
                capabilities: metadata.capabilities,
                models,
            });
        }
        routing_providers.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.provider_instance_id.cmp(&right.provider_instance_id))
        });
        let result = UsageCatalogResult {
            subscriptions: self
                .store
                .list_usage_subscriptions()
                .map_err(|error| RpcError::internal_error(error.to_string()))?,
            mappings: self
                .store
                .list_provider_subscription_mappings()
                .map_err(|error| RpcError::internal_error(error.to_string()))?,
            routing_providers,
        };
        serde_json::to_value(result).map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn handle_usage_subscription_set(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        require_local_query_peer(context)?;
        let params: UsageSubscriptionSetParams =
            serde_json::from_value(params).map_err(|error| {
                usage_params_error(
                    "usage_subscription_params_invalid",
                    format!("Invalid usage.subscription.set params: {error}"),
                )
            })?;
        params
            .validate()
            .map_err(|error| usage_params_error("usage_subscription_invalid", error))?;

        let resolved = self
            .provider_resolver
            .resolve_allow_disabled(&params.provider_instance_id, &params.environment_id)?;
        if pandamux_core::UsageProvider::from(resolved.config.provider) != params.provider {
            return Err(usage_params_error(
                "usage_subscription_provider_mismatch",
                "subscription provider does not match the configured provider instance",
            ));
        }
        if let Some(subscription_key) = params.subscription_key.as_ref()
            && let Some(existing) = self
                .store
                .usage_subscription_by_key(subscription_key)
                .map_err(|error| RpcError::internal_error(error.to_string()))?
            && existing.provider != params.provider
        {
            return Err(usage_params_error(
                "usage_subscription_provider_change_denied",
                "an existing named subscription cannot be reassigned to another provider",
            ));
        }

        let subscription = UsageSubscription {
            key: params.subscription_key.unwrap_or_else(|| {
                SubscriptionKey::new(format!("subscription-{}", uuid::Uuid::new_v4().simple()))
            }),
            provider: params.provider,
            display_name: params.display_name.trim().to_string(),
            masked_account_label: params
                .masked_account_label
                .map(|label| label.trim().to_string()),
        };
        self.store
            .save_usage_subscription_mapping(
                &subscription,
                &params.environment_id,
                &params.provider_instance_id,
                Self::now_ms(),
            )
            .map_err(|error| {
                usage_params_error("usage_subscription_save_failed", error.to_string())
            })?;
        self.handle_usage_catalog(context)
    }

    fn handle_capacity_query(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        require_local_query_peer(context)?;
        let params: CapacityQueryParams = parse_usage_params(params, "capacity.query")?;
        params
            .validate()
            .map_err(|error| usage_params_error("capacity_query_invalid", error))?;
        let result = self
            .store
            .query_capacity(&params)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        serde_json::to_value(result).map_err(|error| RpcError::internal_error(error.to_string()))
    }

    async fn handle_capacity_refresh(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        self.require_capacity_refresh_peer(context)?;
        let params: CapacityRefreshParams = parse_usage_params(params, "capacity.refresh")?;
        params
            .validate()
            .map_err(|error| usage_params_error("capacity_refresh_invalid", error))?;
        let environment_id = params
            .environment_id
            .clone()
            .unwrap_or_else(|| EnvironmentId::new(self.environment_id.clone()));
        if environment_id != context.authenticated_environment_id
            && !matches!(
                context.peer_role,
                PeerRole::Internal | PeerRole::LocalControl
            )
        {
            return Err(RpcError::with_data(
                -32001,
                "Capacity refresh environment does not match the authenticated peer",
                json!({ "reason": "capacity_environment_mismatch" }),
            ));
        }

        let settings = self
            .store
            .get_settings()
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .unwrap_or_default();
        let mut snapshots = Vec::new();
        let mut subscriptions = BTreeMap::new();
        let mut matched = false;
        for config in settings.providers.iter().filter(|config| {
            params
                .provider_instance_id
                .as_ref()
                .is_none_or(|requested| requested == &config.id)
        }) {
            matched = true;
            let Some((subscription_key, _)) = self
                .store
                .usage_subscription_for_provider(&environment_id, &config.id)
                .map_err(|error| RpcError::internal_error(error.to_string()))?
            else {
                if params.provider_instance_id.is_some() {
                    return Err(RpcError::with_data(
                        -32020,
                        "Provider instance has no explicit subscription mapping",
                        json!({
                            "reason": "capacity_subscription_mapping_missing",
                            "providerInstanceId": config.id,
                            "environmentId": environment_id,
                        }),
                    ));
                }
                continue;
            };
            let subscription = self
                .store
                .usage_subscription_by_key(&subscription_key)
                .map_err(|error| RpcError::internal_error(error.to_string()))?
                .ok_or_else(|| {
                    RpcError::internal_error(
                        "Provider subscription mapping references missing metadata",
                    )
                })?;
            subscriptions.insert(subscription_key.clone(), subscription);
            let resolved = self
                .provider_resolver
                .resolve_allow_disabled(&config.id, &environment_id)?;
            let report = if resolved.config.enabled {
                tokio::time::timeout(
                    Duration::from_millis(params.timeout_ms),
                    resolved.driver.usage_limits(&resolved.config),
                )
                .await
                .ok()
                .flatten()
            } else {
                None
            };
            let mut instance_snapshots = capacity_snapshots_from_provider_report(
                &environment_id,
                &resolved.config,
                subscription_key,
                report,
                Self::now_ms(),
            );
            self.store
                .save_capacity_snapshots(&instance_snapshots)
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
            snapshots.append(&mut instance_snapshots);
        }
        if params.provider_instance_id.is_some() && !matched {
            return Err(RpcError::with_data(
                -32020,
                "Provider instance does not exist",
                json!({ "reason": "provider_instance_missing" }),
            ));
        }
        let subscriptions: Vec<UsageSubscription> = subscriptions.into_values().collect();
        serde_json::to_value(CapacityRefreshResult {
            subscriptions,
            snapshots,
        })
        .map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn handle_capacity_push(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        self.require_capacity_push_peer(context)?;
        let params: CapacityPushParams = serde_json::from_value(params).map_err(|error| {
            usage_params_error(
                "capacity_push_params_invalid",
                format!("Invalid capacity.push params: {error}"),
            )
        })?;
        params
            .validate()
            .map_err(|error| usage_params_error("capacity_push_invalid", error))?;
        let authenticated_environment_id = if matches!(context.peer_role, PeerRole::Internal) {
            EnvironmentId::new(self.environment_id.clone())
        } else {
            context.authenticated_environment_id.clone()
        };
        if params.environment_id != authenticated_environment_id {
            return Err(RpcError::with_data(
                -32001,
                "Capacity push environment does not match the authenticated context",
                json!({ "reason": "capacity_environment_mismatch" }),
            ));
        }
        let snapshots = params
            .snapshots
            .into_iter()
            .map(|mut snapshot| {
                snapshot.schema_version = params.schema_version;
                snapshot.environment_id = Some(authenticated_environment_id.clone());
                snapshot.evidence_scope = CapacityEvidenceScope::Hub;
                snapshot.max_age_ms = params.max_age_ms;
                snapshot
            })
            .collect::<Vec<_>>();
        self.store
            .save_capacity_snapshots(&snapshots)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        serde_json::to_value(CapacityPushResult {
            accepted: u32::try_from(snapshots.len()).unwrap_or(u32::MAX),
        })
        .map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn validate_schedule_policy(
        &self,
        schedule: &ScheduleRecord,
    ) -> Result<(Option<OrganizationPolicy>, Option<ProviderInstanceId>), RpcError> {
        schedule.validate().map_err(|error| {
            RpcError::with_data(
                -32602,
                error,
                json!({ "reason": "schedule_definition_invalid" }),
            )
        })?;

        if matches!(&schedule.target, ScheduleTarget::Orchestrator { .. }) {
            return Err(RpcError::with_data(
                -32031,
                "Orchestrator schedule validation is not available in this checkpoint",
                json!({ "reason": "schedule_orchestrator_unavailable" }),
            ));
        }
        if matches!(
            &schedule.target,
            ScheduleTarget::Session {
                thread_mode: pandamux_core::ScheduleThreadMode::ContinueExistingThread,
                ..
            }
        ) {
            return Err(RpcError::with_data(
                -32031,
                "Continue-existing-thread schedules are not available in this checkpoint",
                json!({ "reason": "schedule_continue_unavailable" }),
            ));
        }
        let project_id = schedule.project_id.as_ref().ok_or_else(|| {
            RpcError::with_data(
                -32030,
                "policy-sensitive schedule has no project organization",
                json!({ "reason": "organization_policy_missing" }),
            )
        })?;
        let requested = match &schedule.target {
            ScheduleTarget::Session { provider, .. } => {
                provider.as_deref().map(ProviderInstanceId::new)
            }
            ScheduleTarget::Agent { .. } | ScheduleTarget::Orchestrator { .. } => None,
        };
        let (policy, provider_instance_id) = self.resolve_authorized_project_provider(
            project_id,
            &schedule.environment.id,
            requested.as_ref(),
        )?;
        Ok((Some(policy), Some(provider_instance_id)))
    }

    fn handle_schedule_validate(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        require_local_query_peer(context)?;
        let params: ScheduleValidateParams = serde_json::from_value(params)
            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
        let result = match self.validate_schedule_policy(&params.schedule) {
            Ok((policy, _)) => ScheduleValidationResult {
                valid: true,
                organization_name: policy.map(|policy| policy.display_name),
                blocking_reasons: Vec::new(),
            },
            Err(error) => ScheduleValidationResult {
                valid: false,
                organization_name: None,
                blocking_reasons: vec![error.message],
            },
        };
        serde_json::to_value(result).map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn handle_schedule_save(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        require_local_query_peer(context)?;
        let params: ScheduleSaveParams = serde_json::from_value(params)
            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
        self.validate_schedule_policy(&params.schedule)?;
        if !self
            .environment_router
            .is_local(&Some(params.schedule.environment.id.clone()))
        {
            return Err(RpcError::with_data(
                -32031,
                "Remote schedule storage is not available in this checkpoint",
                json!({ "reason": "schedule_remote_storage_unavailable" }),
            ));
        }
        self.store
            .save_schedule(&params.schedule)
            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
        Ok(json!({ "success": true }))
    }

    async fn handle_schedule_run(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        require_local_query_peer(context)?;
        let params: ScheduleRunParams = serde_json::from_value(params)
            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
        let schedule = self
            .store
            .get_schedule(&params.schedule_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .ok_or_else(|| RpcError::not_found("Schedule not found"))?;
        if !schedule.enabled {
            return Err(RpcError::with_data(
                -32031,
                "Schedule is disabled",
                json!({ "reason": "schedule_disabled" }),
            ));
        }
        if !self
            .environment_router
            .is_local(&Some(schedule.environment.id.clone()))
        {
            return Err(RpcError::with_data(
                -32031,
                "Remote schedule execution is not available in this checkpoint",
                json!({ "reason": "schedule_remote_execution_unavailable" }),
            ));
        }
        if schedule.approval == ScheduleApprovalPolicy::ApproveFirst && !params.approved {
            return Err(RpcError::with_data(
                -32031,
                "Schedule requires explicit approval before its first run",
                json!({ "reason": "schedule_approval_required" }),
            ));
        }
        let (_, provider_instance_id) = self.validate_schedule_policy(&schedule)?;
        let provider_instance_id = provider_instance_id.ok_or_else(|| {
            RpcError::internal_error("validated schedule did not resolve a provider")
        })?;

        let (cwd, prompt, model, agent_id) = match &schedule.target {
            ScheduleTarget::Agent {
                agent,
                project_path,
                prompt,
            } => (
                project_path.clone(),
                prompt.clone(),
                None,
                Some(AgentId::new(agent)),
            ),
            ScheduleTarget::Session {
                project_path,
                prompt,
                model,
                ..
            } => (project_path.clone(), prompt.clone(), model.clone(), None),
            ScheduleTarget::Orchestrator { .. } => unreachable!("handled above"),
        };
        let mut thread = self.thread_manager.create_thread(ThreadCreateParams {
            project_id: schedule.project_id.clone(),
            environment_id: Some(schedule.environment.id.clone()),
            title: Some(format!("Scheduled: {}", schedule.name)),
            provider_instance_id: Some(provider_instance_id),
            model,
            effort: None,
            access_mode: Some(if schedule.full_access_opt_in {
                AccessMode::FullAccess
            } else {
                AccessMode::WorkspaceOnly
            }),
            agent_id,
            cwd: Some(cwd),
            worktree: None,
        })?;
        thread.origin = ThreadOrigin::Schedule {
            schedule_id: schedule.id,
            run_id: RunId::generate(),
        };
        self.store
            .save_thread(&thread)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        let turn = self
            .thread_manager
            .send_turn(ThreadSendTurnParams {
                thread_id: thread.id.clone(),
                text: prompt,
                attachment_ids: Vec::new(),
                model: None,
                effort: None,
                hub_routing_context: None,
            })
            .await?;
        serde_json::to_value(ScheduleRunResult {
            thread_id: thread.id,
            turn_id: turn.turn_id,
        })
        .map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn require_capacity_refresh_peer(&self, context: &RequestContext) -> Result<(), RpcError> {
        if matches!(
            context.peer_role,
            PeerRole::Internal | PeerRole::LocalControl | PeerRole::NodeProxy
        ) {
            return Ok(());
        }
        Err(RpcError::with_data(
            -32001,
            "Capacity refresh requires authenticated control or Hub context",
            json!({ "reason": "capacity_refresh_peer_required" }),
        ))
    }

    fn require_capacity_push_peer(&self, context: &RequestContext) -> Result<(), RpcError> {
        if matches!(context.peer_role, PeerRole::Internal | PeerRole::NodeProxy) {
            return Ok(());
        }
        Err(RpcError::with_data(
            -32001,
            "Capacity push is restricted to the authenticated Hub context",
            json!({ "reason": "capacity_hub_peer_required" }),
        ))
    }

    fn require_hub_usage_peer(&self, context: &RequestContext) -> Result<(), RpcError> {
        if matches!(context.peer_role, PeerRole::Internal) {
            return Ok(());
        }
        if matches!(context.peer_role, PeerRole::NodeProxy)
            && context.authenticated_environment_id.as_str() == self.environment_id
        {
            return Ok(());
        }
        Err(RpcError::with_data(
            -32001,
            "Usage export is restricted to the authenticated Hub peer",
            json!({ "reason": "usage_hub_peer_required" }),
        ))
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
            self.sync_attachment_to_environment(
                &thread.environment_id,
                &p.thread_id,
                &result.attachment.id,
            )
            .await?;
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

    async fn handle_antigravity_status(&self, params: Value) -> Result<Value, RpcError> {
        let p: AntigravityStatusParams = if params.is_null() {
            AntigravityStatusParams::default()
        } else {
            serde_json::from_value(params).map_err(|error| {
                RpcError::invalid_params(format!("Invalid antigravity.status params: {error}"))
            })?
        };
        let environment_id = p
            .environment_id
            .unwrap_or_else(|| EnvironmentId::new(self.environment_id.clone()));
        let resolved = self.provider_resolver.resolve_kind(
            p.provider_instance_id.as_ref(),
            &environment_id,
            ProviderKind::Antigravity,
        )?;
        let snapshot = resolved.driver.probe(&resolved.config).await;

        #[cfg(target_os = "windows")]
        let (platform, arch) = ("windows", "x64");
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let (platform, arch) = ("linux", "x64");
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        let (platform, arch) = ("linux", "arm64");
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let (platform, arch) = ("unsupported", "unknown");

        let platform_arch = format!("{platform}-{arch}");
        let bin_path = if let Some(binary_path) = resolved.config.binary_path.as_ref() {
            Some(PathBuf::from(binary_path))
        } else {
            pandamux_providers::antigravity::resolve_antigravity_binary(
                None,
                &self.data_dir,
                &platform_arch,
            )
        };

        let authenticated = matches!(
            snapshot.auth,
            pandamux_providers::models::ProviderAuthStatus::Authenticated { .. }
        );
        let free_disk = pandamux_providers::antigravity::get_available_disk_bytes(&self.data_dir)
            .map_err(provider_rpc_error)?;

        let (active_processes, max_processes) =
            pandamux_providers::antigravity::antigravity_process_counts();
        let result = AntigravityStatusResult {
            provider_instance_id: resolved.config.id.clone(),
            installed: snapshot.installed,
            version: snapshot.version,
            binary_path: bin_path.map(|p| p.to_string_lossy().to_string()),
            authenticated,
            active_processes,
            max_processes,
            free_disk_bytes: free_disk,
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_antigravity_install(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        let p: AntigravityInstallParams = serde_json::from_value(params).map_err(|e| {
            RpcError::invalid_params(format!("Invalid antigravity.install params: {e}"))
        })?;
        if p.staged_archive_path.is_some()
            && !matches!(context.peer_role, PeerRole::Internal | PeerRole::NodeProxy)
        {
            return Err(RpcError::unauthorized(
                "Only the authenticated Hub proxy may install a staged Antigravity archive",
            ));
        }
        let staged_archive = p.staged_archive_path.as_deref().map(PathBuf::from);

        let result = pandamux_providers::antigravity::install_managed_bundle(
            &self.data_dir,
            &p.manifest,
            staged_archive.as_deref(),
            p.force,
        )
        .await
        .map_err(provider_rpc_error)?;

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    fn handle_antigravity_prepare_upload(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        if !matches!(context.peer_role, PeerRole::Internal | PeerRole::NodeProxy) {
            return Err(RpcError::unauthorized(
                "Only the authenticated Hub proxy may prepare an Antigravity upload",
            ));
        }
        let params: AntigravityPrepareUploadParams =
            serde_json::from_value(params).map_err(|e| {
                RpcError::invalid_params(format!("Invalid antigravity.prepare_upload params: {e}"))
            })?;
        let path = pandamux_providers::antigravity::prepare_bundle_upload(
            &self.data_dir,
            &params.manifest,
        )
        .map_err(provider_rpc_error)?;
        serde_json::to_value(AntigravityPrepareUploadResult {
            remote_path: path.to_string_lossy().to_string(),
        })
        .map_err(|error| RpcError::internal_error(error.to_string()))
    }

    fn handle_antigravity_discard_upload(
        &self,
        context: &RequestContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        if !matches!(context.peer_role, PeerRole::Internal | PeerRole::NodeProxy) {
            return Err(RpcError::unauthorized(
                "Only the authenticated Hub proxy may discard an Antigravity upload",
            ));
        }
        let params: AntigravityDiscardUploadParams =
            serde_json::from_value(params).map_err(|e| {
                RpcError::invalid_params(format!("Invalid antigravity.discard_upload params: {e}"))
            })?;
        pandamux_providers::antigravity::discard_bundle_upload(
            &self.data_dir,
            std::path::Path::new(&params.staged_archive_path),
        )
        .map_err(provider_rpc_error)?;
        Ok(json!({ "discarded": true }))
    }

    async fn handle_antigravity_oauth_start(&self, params: Value) -> Result<Value, RpcError> {
        let p: AntigravityOAuthStartParams = if params.is_null() {
            AntigravityOAuthStartParams::default()
        } else {
            serde_json::from_value(params).map_err(|error| {
                RpcError::invalid_params(format!("Invalid antigravity.oauth_start params: {error}"))
            })?
        };
        let environment_id = p
            .environment_id
            .unwrap_or_else(|| EnvironmentId::new(self.environment_id.clone()));
        let resolved = self.provider_resolver.resolve_kind(
            p.provider_instance_id.as_ref(),
            &environment_id,
            ProviderKind::Antigravity,
        )?;
        let instance_id = resolved.config.id.clone();
        let started = self
            .antigravity_auth
            .start(&resolved.config, &self.data_dir)
            .await
            .map_err(|error| provider_rpc_error_for_target(error, &instance_id, &environment_id))?;

        let result = AntigravityOAuthStartResult {
            provider_instance_id: started.provider_instance_id,
            flow_id: started.flow_id,
            auth_url: started.auth_url,
            redirect_port: started.redirect_port,
            state: started.state,
        };

        serde_json::to_value(result).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    async fn handle_antigravity_oauth_relay(&self, params: Value) -> Result<Value, RpcError> {
        let p: AntigravityOAuthRelayParams = serde_json::from_value(params).map_err(|e| {
            RpcError::invalid_params(format!("Invalid antigravity.oauth_relay params: {e}"))
        })?;
        let environment_id = p
            .environment_id
            .clone()
            .unwrap_or_else(|| EnvironmentId::new(self.environment_id.clone()));
        let resolved = self.provider_resolver.resolve_kind(
            p.provider_instance_id.as_ref(),
            &environment_id,
            ProviderKind::Antigravity,
        )?;
        let instance_id = resolved.config.id.clone();

        let message = self
            .antigravity_auth
            .complete(&resolved.config.id, &p.flow_id, &p.callback_url)
            .await
            .map_err(|error| provider_rpc_error_for_target(error, &instance_id, &environment_id))?;

        let result = AntigravityOAuthRelayResult {
            provider_instance_id: resolved.config.id,
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

    #[test]
    fn provider_login_command_uses_the_native_terminal_submit_character() {
        assert_eq!(
            login_terminal_input("claude auth login", true),
            "claude auth login\r"
        );
        assert_eq!(
            login_terminal_input("claude auth login", false),
            "claude auth login\n"
        );
    }

    fn mock_router(store: Store) -> Router {
        let settings = UserSettings {
            providers: vec![pandamux_core::ProviderInstanceConfig {
                id: pandamux_core::ProviderInstanceId::new("mock-default"),
                provider: ProviderKind::Custom,
                display_name: "Mock Provider".to_string(),
                profile_dir: "profiles/custom/mock-default".to_string(),
                environment_id: None,
                binary_path: Some("/mock/bin/provider".to_string()),
                concurrency_limit: 2,
                env_overrides: Default::default(),
                settings: json!({ "fixture": true }),
                enabled: true,
                disabled_reason: None,
            }],
            ..UserSettings::default()
        };
        store.save_settings(&settings).expect("save mock settings");
        let drivers = Arc::new(DriverRegistry::new());
        drivers.register(
            ProviderKind::Custom,
            Arc::new(pandamux_providers::MockProviderDriver::new(
                ProviderKind::Custom,
                "Mock Provider",
            )),
        );
        Router::with_drivers(store, ServerRole::Hub, "env-test".to_string(), drivers)
    }

    fn add_usage_fixture(store: &Store) {
        store
            .save_usage_subscription(
                &pandamux_core::UsageSubscription {
                    key: pandamux_core::SubscriptionKey::new("subkey-work"),
                    provider: pandamux_core::UsageProvider::Codex,
                    display_name: "Work Codex".to_string(),
                    masked_account_label: None,
                },
                1000,
            )
            .expect("save usage subscription");
        store
            .map_provider_subscription(
                &EnvironmentId::new("env-test"),
                &pandamux_core::ProviderInstanceId::new("codex-work"),
                &pandamux_core::SubscriptionKey::new("subkey-work"),
                1000,
            )
            .expect("map usage subscription");
        store
            .record_usage_batch(
                &EnvironmentId::new("env-test"),
                &[pandamux_core::UsageObservation {
                    id: pandamux_core::UsageObservationId::new("usage-router"),
                    source_kind: pandamux_core::UsageSourceKind::ManagedTurn,
                    parser_version: 1,
                    provider_native_session_id: None,
                    provider_native_turn_id: Some("native-turn".to_string()),
                    occurred_at_ms: Some(1000),
                    provider: pandamux_core::UsageProvider::Codex,
                    provider_instance_id: pandamux_core::ProviderInstanceId::new("codex-work"),
                    subscription_key: pandamux_core::SubscriptionKey::new("subkey-work"),
                    model: Some("gpt-fixture".to_string()),
                    tokens: pandamux_core::TokenCounts {
                        input: 5,
                        output: 2,
                        ..pandamux_core::TokenCounts::default()
                    },
                    unknown_fields: std::collections::BTreeSet::new(),
                    provider_reported_micro_usd: None,
                    source_record_count: 1,
                    thread_id: None,
                    turn_id: None,
                    agent_id: None,
                    schedule_id: None,
                    fact_kind: Default::default(),
                }],
                None,
            )
            .expect("record usage observation");
    }

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

    fn add_mock_organization_policy(store: &Store) {
        store
            .save_usage_subscription_mapping(
                &pandamux_core::UsageSubscription {
                    key: pandamux_core::SubscriptionKey::new("subkey-work"),
                    provider: pandamux_core::UsageProvider::Custom,
                    display_name: "Work Custom".to_string(),
                    masked_account_label: Some("te***@example.com".to_string()),
                },
                &EnvironmentId::new("env-test"),
                &pandamux_core::ProviderInstanceId::new("mock-default"),
                1000,
            )
            .expect("save subscription mapping");
        let mut settings = store
            .get_settings()
            .expect("load settings")
            .expect("mock settings");
        settings.organizations = vec![pandamux_core::OrganizationPolicy {
            id: pandamux_core::OrganizationId::new("org-work"),
            display_name: "Work".to_string(),
            project_ids: vec![pandamux_core::ProjectId::new("project-work")],
            allowed_provider_instances: vec![pandamux_core::ProviderInstanceId::new(
                "mock-default",
            )],
            allowed_subscriptions: vec![pandamux_core::SubscriptionKey::new("subkey-work")],
            failover_pool_ids: Vec::new(),
            default_tier_mapping: None,
            max_concurrency: Some(2),
            daily_budget_micro_usd: None,
            broadside: Default::default(),
            revision: 1,
        }];
        store
            .save_settings_with_organization_policies(&settings, 1001)
            .expect("save canonical settings");
    }

    #[tokio::test]
    async fn usage_catalog_edit_and_project_routing_are_fail_closed() {
        let store = Store::in_memory().expect("open store");
        let router = mock_router(store.clone());
        add_mock_organization_policy(&store);

        let catalog = router
            .handle_request(RpcRequest::new(1, "usage.catalog", None))
            .await
            .expect("catalog response");
        assert_eq!(
            catalog.result.as_ref().unwrap()["subscriptions"][0]["displayName"],
            "Work Custom"
        );

        let edited = router
            .handle_request(RpcRequest::new(
                2,
                "usage.subscription.set",
                Some(json!({
                    "subscriptionKey": "subkey-work",
                    "environmentId": "env-test",
                    "providerInstanceId": "mock-default",
                    "provider": "custom",
                    "displayName": "Primary Work Custom",
                    "maskedAccountLabel": "wo***@example.com"
                })),
            ))
            .await
            .expect("edit response");
        assert!(edited.is_success());
        assert_eq!(
            edited.result.as_ref().unwrap()["subscriptions"][0]["displayName"],
            "Primary Work Custom"
        );

        let allowed = router
            .handle_request(RpcRequest::new(
                3,
                "thread.create",
                Some(json!({
                    "projectId": "project-work",
                    "environmentId": "env-test",
                    "providerInstanceId": "mock-default",
                    "title": "Allowed"
                })),
            ))
            .await
            .expect("allowed route response");
        assert!(allowed.is_success());

        let missing = router
            .handle_request(RpcRequest::new(
                4,
                "thread.create",
                Some(json!({
                    "projectId": "project-personal",
                    "environmentId": "env-test",
                    "providerInstanceId": "mock-default"
                })),
            ))
            .await
            .expect("missing policy response");
        assert_eq!(
            missing.error.expect("missing policy denied").data.unwrap()["reason"],
            "organization_policy_denied"
        );

        let disallowed = router
            .handle_request(RpcRequest::new(
                5,
                "thread.create",
                Some(json!({
                    "projectId": "project-work",
                    "environmentId": "env-test",
                    "providerInstanceId": "personal-instance"
                })),
            ))
            .await
            .expect("disallowed route response");
        assert_eq!(
            disallowed.error.expect("route denied").data.unwrap()["reason"],
            "organization_policy_denied"
        );
    }

    #[tokio::test]
    async fn schedule_validation_applies_organization_policy_before_save() {
        let store = Store::in_memory().expect("open store");
        let router = mock_router(store.clone());
        add_mock_organization_policy(&store);
        let mut schedule = ScheduleRecord {
            id: pandamux_core::ScheduleId::new("schedule-work"),
            name: "Work review".to_string(),
            project_id: Some(pandamux_core::ProjectId::new("project-work")),
            environment: pandamux_core::ScheduleEnvironmentRef {
                id: EnvironmentId::new("env-test"),
                display_name: "Local test".to_string(),
            },
            trigger: pandamux_core::ScheduleTrigger {
                cron: "0 0 * * *".to_string(),
                timezone: "UTC".to_string(),
            },
            target: ScheduleTarget::Session {
                project_path: "/repo".to_string(),
                prompt: "Review the repository".to_string(),
                provider: Some("mock-default".to_string()),
                model: Some("mock-model".to_string()),
                thread_mode: Default::default(),
            },
            approval: ScheduleApprovalPolicy::ApproveFirst,
            overlap: Default::default(),
            catch_up: Default::default(),
            budget: Default::default(),
            notify: Vec::new(),
            full_access_opt_in: false,
            enabled: true,
        };
        let valid = router
            .handle_request(RpcRequest::new(
                1,
                "schedule.validate",
                Some(json!({ "schedule": schedule.clone() })),
            ))
            .await
            .expect("validation response");
        assert_eq!(valid.result.as_ref().unwrap()["valid"], true);

        let saved = router
            .handle_request(RpcRequest::new(
                2,
                "schedule.save",
                Some(json!({ "schedule": schedule.clone() })),
            ))
            .await
            .expect("save response");
        assert!(saved.is_success());

        let approval_required = router
            .handle_request(RpcRequest::new(
                20,
                "schedule.run",
                Some(json!({
                    "scheduleId": "schedule-work",
                    "approved": false
                })),
            ))
            .await
            .expect("approval response");
        assert_eq!(
            approval_required
                .error
                .expect("approval required")
                .data
                .unwrap()["reason"],
            "schedule_approval_required"
        );

        if let ScheduleTarget::Session { provider, .. } = &mut schedule.target {
            *provider = Some("personal-instance".to_string());
        }
        let invalid = router
            .handle_request(RpcRequest::new(
                3,
                "schedule.validate",
                Some(json!({ "schedule": schedule.clone() })),
            ))
            .await
            .expect("invalid validation response");
        assert_eq!(invalid.result.as_ref().unwrap()["valid"], false);
        assert!(
            invalid.result.as_ref().unwrap()["blockingReasons"][0]
                .as_str()
                .unwrap()
                .contains("not allowed")
        );

        if let ScheduleTarget::Session {
            provider,
            thread_mode,
            ..
        } = &mut schedule.target
        {
            *provider = Some("mock-default".to_string());
            *thread_mode = pandamux_core::ScheduleThreadMode::ContinueExistingThread;
        }
        let continue_unavailable = router
            .handle_request(RpcRequest::new(
                4,
                "schedule.validate",
                Some(json!({ "schedule": schedule.clone() })),
            ))
            .await
            .expect("continue validation response");
        assert_eq!(
            continue_unavailable.result.as_ref().unwrap()["valid"],
            false
        );
        assert!(
            continue_unavailable.result.as_ref().unwrap()["blockingReasons"][0]
                .as_str()
                .unwrap()
                .contains("not available")
        );

        schedule.target = ScheduleTarget::Orchestrator {
            prompt: "Coordinate the review".to_string(),
        };
        let orchestrator_unavailable = router
            .handle_request(RpcRequest::new(
                5,
                "schedule.validate",
                Some(json!({ "schedule": schedule })),
            ))
            .await
            .expect("orchestrator validation response");
        assert_eq!(
            orchestrator_unavailable.result.as_ref().unwrap()["valid"],
            false
        );
        assert!(
            orchestrator_unavailable.result.as_ref().unwrap()["blockingReasons"][0]
                .as_str()
                .unwrap()
                .contains("not available")
        );
    }

    #[tokio::test]
    async fn usage_export_is_hub_only_and_queries_are_local_control_only() {
        let store = Store::in_memory().expect("open store");
        add_usage_fixture(&store);
        let router = mock_router(store);
        router.capability_authority().configure_local_capabilities(
            "endpoint".to_string(),
            "control-token".to_string(),
            "mcp-token".to_string(),
        );
        let control = router
            .capability_authority()
            .authenticate(
                "control-token",
                crate::request_context::ClientIdentity::default(),
            )
            .expect("control context");
        let node = RequestContext::node_proxy(
            EnvironmentId::new("env-test"),
            crate::request_context::ClientIdentity::default(),
        );

        let denied_export = router
            .handle_request_with_context(
                &control,
                RpcRequest::new(1, "usage.export", Some(json!({ "limit": 10 }))),
            )
            .await
            .expect("export response");
        assert_eq!(denied_export.error.expect("denied export").code, -32001);

        let denied_backfill = router
            .handle_request_with_context(
                &control,
                RpcRequest::new(10, "usage.backfill", Some(json!({ "limit": 10 }))),
            )
            .await
            .expect("backfill response");
        assert_eq!(denied_backfill.error.expect("denied backfill").code, -32001);

        let denied_routed_export = router
            .handle_request_with_context(
                &control,
                RpcRequest::new(
                    11,
                    "usage.export",
                    Some(json!({ "environmentId": "env-other", "limit": 10 })),
                ),
            )
            .await
            .expect("routed export response");
        assert_eq!(
            denied_routed_export
                .error
                .expect("routed export denied")
                .data
                .unwrap()["reason"],
            "capability_scope_denied"
        );

        let export = router
            .handle_request_with_context(
                &node,
                RpcRequest::new(2, "usage.export", Some(json!({ "limit": 10 }))),
            )
            .await
            .expect("export response");
        assert!(export.is_success());
        assert_eq!(
            export.result.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let backfill = router
            .handle_request_with_context(
                &node,
                RpcRequest::new(13, "usage.backfill", Some(json!({ "limit": 10 }))),
            )
            .await
            .expect("backfill response");
        assert!(backfill.is_success());
        assert_eq!(
            backfill.result.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let forged_node = RequestContext::node_proxy(
            EnvironmentId::new("env-other"),
            crate::request_context::ClientIdentity::default(),
        );
        let forged = router
            .handle_request_with_context(
                &forged_node,
                RpcRequest::new(3, "usage.export", Some(json!({ "limit": 10 }))),
            )
            .await
            .expect("forged response");
        assert_eq!(
            forged.error.expect("forged export denied").data.unwrap()["reason"],
            "usage_hub_peer_required"
        );

        let query = router
            .handle_request_with_context(
                &control,
                RpcRequest::new(4, "usage.query", Some(json!({ "limit": 10 }))),
            )
            .await
            .expect("query response");
        assert!(query.is_success());
        assert_eq!(
            query.result.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let filtered_remote_environment = router
            .handle_request_with_context(
                &control,
                RpcRequest::new(
                    12,
                    "usage.query",
                    Some(json!({ "environmentId": "env-other", "limit": 10 })),
                ),
            )
            .await
            .expect("filtered query response");
        assert!(filtered_remote_environment.is_success());
        assert!(
            filtered_remote_environment.result.unwrap()["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let denied_query = router
            .handle_request_with_context(
                &node,
                RpcRequest::new(5, "usage.query", Some(json!({ "limit": 10 }))),
            )
            .await
            .expect("query response");
        assert_eq!(denied_query.error.expect("node query denied").code, -32001);

        let removed_write = router
            .handle_request_with_context(
                &control,
                RpcRequest::new(6, "usage.record_buckets", Some(json!({ "buckets": [] }))),
            )
            .await
            .expect("removed write response");
        assert_eq!(removed_write.error.expect("method missing").code, -32601);
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
        let router = mock_router(store);

        // 1. Create thread
        let create_req = RpcRequest::new(
            1,
            "thread.create",
            Some(json!({
                "title": "Test Chat",
                "providerInstanceId": "mock-default",
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
        let router = mock_router(store).with_attachments(attachment_manager);

        // Create thread first
        let create_req = RpcRequest::new(
            1,
            "thread.create",
            Some(json!({
                "title": "Attachment Test",
                "providerInstanceId": "mock-default",
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
        assert_eq!(
            get_val["settings"]["version"],
            pandamux_core::SETTINGS_SCHEMA_VERSION
        );
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
        assert_eq!(health_val["hasMore"], false);
        assert!(health_val["nextInstanceId"].is_string());
        // Verify report structure
        for report in reports {
            assert!(report.get("instanceId").is_some());
            assert!(report.get("provider").is_some());
            assert!(report.get("status").is_some());
        }
        let disabled = reports
            .iter()
            .filter(|report| report["status"] == "disabled")
            .collect::<Vec<_>>();
        assert_eq!(disabled.len(), 3);
        for report in disabled {
            assert_eq!(
                report["disabledReason"]["code"],
                "live_acceptance_incomplete"
            );
            assert!(report["disabledReason"]["message"].is_string());
            assert!(report["disabledReason"]["remediation"].is_string());
        }

        let first_page = router
            .handle_request(RpcRequest::new(
                5,
                "provider.health",
                Some(json!({ "limit": 1 })),
            ))
            .await
            .unwrap();
        assert!(first_page.is_success());
        let first_page = first_page.result.unwrap();
        assert_eq!(first_page["reports"].as_array().unwrap().len(), 1);
        assert_eq!(first_page["hasMore"], true);
        let first_cursor = first_page["nextInstanceId"].as_str().unwrap();

        let second_page = router
            .handle_request(RpcRequest::new(
                6,
                "provider.health",
                Some(json!({ "afterInstanceId": first_cursor, "limit": 1 })),
            ))
            .await
            .unwrap();
        assert!(second_page.is_success());
        assert_ne!(
            second_page.result.unwrap()["reports"][0]["instanceId"],
            first_cursor
        );

        let invalid_page = router
            .handle_request(RpcRequest::new(
                7,
                "provider.health",
                Some(json!({ "limit": 0 })),
            ))
            .await
            .unwrap();
        assert_eq!(
            invalid_page.error.unwrap().data.unwrap()["reason"],
            "provider_health_page_limit_invalid"
        );
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
        let forwarded_methods = Arc::new(std::sync::Mutex::new(Vec::new()));
        let forwarded_methods_for_transport = forwarded_methods.clone();
        let mock_transport = Arc::new(MockEnvironmentTransport::new(move |req| {
            forwarded_methods_for_transport
                .lock()
                .expect("forwarded methods")
                .push(req.method.clone());
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

        // 5. Continuations include thread identity so the Hub can route them.
        let continuation_res = hub_router
            .handle_request(RpcRequest::new(
                4,
                "thread.continue_capacity",
                Some(json!({
                    "continuationId": "continue-remote-1",
                    "threadId": "th-remote-1"
                })),
            ))
            .await
            .expect("remote continuation response");
        assert!(continuation_res.is_success());
        assert_eq!(
            forwarded_methods
                .lock()
                .expect("forwarded methods")
                .last()
                .map(String::as_str),
            Some("thread.continue_capacity")
        );

        // 6. Test event relaying to Hub broadcaster
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
    async fn remote_project_routes_are_filtered_and_pinned_before_forwarding() {
        use crate::environment_router::MockEnvironmentTransport;
        use pandamux_core::{
            OrganizationId, OrganizationPolicy, ProjectId, ProviderInstanceId, SubscriptionKey,
            Thread, ThreadId, ThreadOrigin, ThreadStatus, ThreadWorkspace, UsageProvider,
            UsageSubscription,
        };
        use std::sync::Mutex as StdMutex;

        let store = Store::in_memory().expect("store");
        let remote_env = EnvironmentId::new("env-remote-policy");
        for (key, provider, instance, name) in [
            (
                "subkey-remote-claude",
                UsageProvider::Claude,
                "claude-default",
                "Remote Claude",
            ),
            (
                "subkey-remote-codex",
                UsageProvider::Codex,
                "codex-default",
                "Remote Codex",
            ),
        ] {
            store
                .save_usage_subscription_mapping(
                    &UsageSubscription {
                        key: SubscriptionKey::new(key),
                        provider,
                        display_name: name.to_string(),
                        masked_account_label: None,
                    },
                    &remote_env,
                    &ProviderInstanceId::new(instance),
                    1000,
                )
                .expect("save remote mapping");
        }
        let settings = UserSettings {
            organizations: vec![OrganizationPolicy {
                id: OrganizationId::new("org-remote"),
                display_name: "Remote work".to_string(),
                project_ids: vec![ProjectId::new("project-remote")],
                allowed_provider_instances: vec![ProviderInstanceId::new("claude-default")],
                allowed_subscriptions: vec![SubscriptionKey::new("subkey-remote-claude")],
                failover_pool_ids: Vec::new(),
                default_tier_mapping: None,
                max_concurrency: None,
                daily_budget_micro_usd: None,
                broadside: Default::default(),
                revision: 1,
            }],
            ..UserSettings::default()
        };
        store
            .save_settings_with_organization_policies(&settings, 1001)
            .expect("save remote policy");

        let hub_router = Router::new(store.clone(), ServerRole::Hub, "env-local".to_string());
        let forwarded = Arc::new(StdMutex::new(Vec::new()));
        let forwarded_for_transport = forwarded.clone();
        let remote_env_for_transport = remote_env.clone();
        let transport = Arc::new(MockEnvironmentTransport::new(move |request| {
            forwarded_for_transport
                .lock()
                .expect("forwarded requests")
                .push(request.clone());
            let params: ThreadCreateParams =
                serde_json::from_value(request.params.clone().expect("thread create params"))
                    .expect("decode forwarded create");
            let thread = Thread {
                id: ThreadId::new("thread-remote-policy"),
                project_id: params.project_id,
                environment_id: remote_env_for_transport.clone(),
                parent_thread_id: None,
                title: params.title.unwrap_or_else(|| "Remote policy".to_string()),
                provider_instance_id: params
                    .provider_instance_id
                    .expect("Hub pins provider before forwarding"),
                model: params.model.unwrap_or_else(|| "default".to_string()),
                effort: params.effort,
                access_mode: params.access_mode.unwrap_or_default(),
                workspace: ThreadWorkspace {
                    cwd: params.cwd.unwrap_or_else(|| "/remote/project".to_string()),
                    worktree: params.worktree,
                },
                status: ThreadStatus::Idle,
                agent: None,
                origin: ThreadOrigin::Manual,
                created_at_ms: 100,
                updated_at_ms: 100,
            };
            Ok(RpcResponse::success(
                request.id.clone().unwrap_or(1.into()),
                serde_json::to_value(thread).expect("encode thread"),
            ))
        }));
        hub_router
            .environment_router()
            .register_transport(remote_env.clone(), transport, None)
            .await;

        let denied = hub_router
            .handle_request(RpcRequest::new(
                1,
                "thread.create",
                Some(json!({
                    "projectId": "project-remote",
                    "environmentId": remote_env.as_str(),
                    "providerInstanceId": "codex-default"
                })),
            ))
            .await
            .expect("denied response");
        assert_eq!(
            denied.error.expect("route denied").data.unwrap()["reason"],
            "organization_policy_denied"
        );
        assert!(forwarded.lock().expect("forwarded requests").is_empty());

        let allowed = hub_router
            .handle_request(RpcRequest::new(
                2,
                "thread.create",
                Some(json!({
                    "projectId": "project-remote",
                    "environmentId": remote_env.as_str(),
                    "title": "Remote policy"
                })),
            ))
            .await
            .expect("allowed response");
        assert!(allowed.is_success());
        let forwarded = forwarded.lock().expect("forwarded requests");
        assert_eq!(forwarded.len(), 1);
        assert_eq!(
            forwarded[0].params.as_ref().unwrap()["providerInstanceId"],
            "claude-default"
        );
        drop(forwarded);

        let prepared_turn = hub_router
            .prepare_forwarded_thread_turn(
                &json!({
                    "threadId": "thread-remote-policy",
                    "text": "use the canonical remote policy"
                }),
                &remote_env,
            )
            .expect("prepare remote project turn");
        assert_eq!(
            prepared_turn["hubRoutingContext"]["organization"]["id"],
            "org-remote"
        );
        assert_eq!(
            prepared_turn["hubRoutingContext"]["failoverPools"],
            json!([])
        );
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
                "attachmentIds": ["att-paste-1"]
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
    async fn remote_attachment_path_is_imported_on_the_hub_before_forwarding() {
        use crate::environment_router::MockEnvironmentTransport;
        use pandamux_core::ids::{EnvironmentId, ThreadId};
        use pandamux_core::{Thread, ThreadWorkspace};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let temp_dir = tempfile::tempdir().unwrap();
        let source_path = temp_dir.path().join("source.png");
        std::fs::write(&source_path, b"PNG_BYTES").unwrap();
        let store = Store::in_memory().unwrap();
        let hub_router = Router::new(store.clone(), ServerRole::Hub, "env-local".to_string())
            .with_attachments(AttachmentManager::new(temp_dir.path().join("attachments")));
        let remote_env = EnvironmentId::new("env-remote");
        let thread_id = ThreadId::new("th-remote-import");
        store
            .save_thread(&Thread {
                id: thread_id.clone(),
                project_id: None,
                environment_id: remote_env.clone(),
                parent_thread_id: None,
                title: "Remote import".to_string(),
                provider_instance_id: pandamux_core::ProviderInstanceId::new("claude-default"),
                model: "claude-test".to_string(),
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
            })
            .unwrap();

        let import_calls = Arc::new(AtomicUsize::new(0));
        let put_calls = Arc::new(AtomicUsize::new(0));
        let import_calls_for_transport = Arc::clone(&import_calls);
        let put_calls_for_transport = Arc::clone(&put_calls);
        let transport = Arc::new(MockEnvironmentTransport::new(move |request| {
            let id = request.id.clone().unwrap_or(1.into());
            match request.method.as_str() {
                "attachment.import_path" | "attachment.importPath" => {
                    import_calls_for_transport.fetch_add(1, Ordering::SeqCst);
                }
                "attachment.put" => {
                    put_calls_for_transport.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
            Ok(RpcResponse::success(id, json!({"isComplete": true})))
        }));
        hub_router
            .environment_router()
            .register_transport(remote_env, transport, None)
            .await;

        let response = hub_router
            .handle_request(RpcRequest::new(
                1,
                "attachment.import_path",
                Some(json!({
                    "threadId": thread_id,
                    "path": source_path
                })),
            ))
            .await
            .unwrap();
        assert!(response.is_success());
        assert!(response.result.unwrap().get("attachment").is_some());
        assert_eq!(import_calls.load(Ordering::SeqCst), 0);
        assert_eq!(put_calls.load(Ordering::SeqCst), 1);
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
        let login_launch_count = Arc::new(AtomicUsize::new(0));

        let oc = Arc::clone(&open_count);
        let ac = Arc::clone(&attach_count);
        let ic = Arc::clone(&input_count);
        let rc = Arc::clone(&resize_count);
        let cc = Arc::clone(&close_count);
        let lc = Arc::clone(&login_launch_count);

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
                    let terminal_id = req
                        .params
                        .as_ref()
                        .and_then(|p| p.get("terminalId"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("term-remote-1");
                    let since = req
                        .params
                        .as_ref()
                        .and_then(|p| p.get("sinceOffset"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    Ok(RpcResponse::success(
                        req.id.clone().unwrap_or(2.into()),
                        json!({
                            "terminalId": terminal_id,
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
                "provider.login.launch" => {
                    lc.fetch_add(1, Ordering::SeqCst);
                    Ok(RpcResponse::success(
                        req.id.clone().unwrap_or(6.into()),
                        json!({
                            "environmentId": "env_remote_node",
                            "providerInstanceId": "claude-remote",
                            "terminalId": "term-remote-login",
                            "displayCommand": "claude auth login",
                            "retryable": true
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

        // 6. A provider login creates a terminal too; register it for later
        // attach/input/close routing just like terminal.open.
        let login_req = RpcRequest::new(
            6,
            "provider.login.launch",
            Some(json!({
                "previewId": "login-preview-remote",
                "providerInstanceId": "claude-remote",
                "environmentId": "env_remote_node"
            })),
        );
        let login_res = hub_router.handle_request(login_req).await.unwrap();
        assert!(login_res.is_success());
        assert_eq!(login_launch_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            hub_router.remote_terminals().get("term-remote-login"),
            Some(&remote_env)
        );

        let attach_login_req = RpcRequest::new(
            7,
            "terminal.attach",
            Some(json!({
                "terminalId": "term-remote-login",
                "sinceOffset": 0
            })),
        );
        let attach_login_res = hub_router.handle_request(attach_login_req).await.unwrap();
        assert!(attach_login_res.is_success());
        assert_eq!(
            attach_login_res.result.unwrap()["terminalId"],
            "term-remote-login"
        );
        assert_eq!(attach_count.load(Ordering::SeqCst), 2);

        let close_login_req = RpcRequest::new(
            8,
            "terminal.close",
            Some(json!({ "terminalId": "term-remote-login" })),
        );
        let close_login_res = hub_router.handle_request(close_login_req).await.unwrap();
        assert!(close_login_res.is_success());
        assert_eq!(close_count.load(Ordering::SeqCst), 2);
        assert!(
            !hub_router
                .remote_terminals()
                .contains_key("term-remote-login")
        );
    }

    #[tokio::test]
    async fn router_rejects_unpinned_antigravity_installs_without_fabricating_auth() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Node, "env_node".to_string())
            .with_data_dir(temp_dir.path().to_path_buf());

        let status_req = RpcRequest::new(1, "antigravity.status", None);
        let status_res = router.handle_request(status_req).await.unwrap();
        assert!(status_res.is_success());
        let status: AntigravityStatusResult =
            serde_json::from_value(status_res.result.unwrap()).unwrap();
        assert!(!status.installed);
        assert!(!status.authenticated);
        assert_eq!(status.max_processes, 2);

        let manifest = pandamux_providers::antigravity::current_bundle_manifest()
            .expect("test target has an Antigravity pin");
        let prepare_req = RpcRequest::new(
            2,
            "antigravity.prepare_upload",
            Some(json!({ "manifest": manifest.clone() })),
        );
        let prepare_res = router.handle_request(prepare_req).await.unwrap();
        assert!(prepare_res.is_success());
        let prepared: AntigravityPrepareUploadResult =
            serde_json::from_value(prepare_res.result.unwrap()).unwrap();
        assert!(prepared.remote_path.contains("cache/antigravity/.upload-"));

        let mut unpinned = manifest;
        unpinned.url = "https://example.invalid/not-the-pinned-runtime.zip".to_string();
        let install_req = RpcRequest::new(
            3,
            "antigravity.install",
            Some(json!({ "manifest": unpinned })),
        );
        let install_res = router.handle_request(install_req).await.unwrap();
        let install_error = install_res.error.expect("unpinned install error");
        assert_eq!(
            install_error.data.unwrap()["providerCode"],
            "antigravity_manifest_not_pinned"
        );

        let oauth_start_req = RpcRequest::new(4, "antigravity.oauth_start", None);
        let oauth_start_res = router.handle_request(oauth_start_req).await.unwrap();
        assert!(oauth_start_res.error.is_some());
        assert!(oauth_start_res.result.is_none());
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
                            "version": "agy_acp_server_1.1.1",
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
                    "version": "agy_acp_server_1.1.1",
                    "platform": "linux",
                    "arch": "x64",
                    "url": "https://dl.google.com/test.zip",
                    "size": 100,
                    "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    "executable": { "name": "agy_acp_server.par", "size": 80 },
                    "harness": { "name": "localharness_external", "size": 20 }
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
                "flowId": "flow-1",
                "callbackUrl": "http://127.0.0.1:51123/?code=123&state=state-1",
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

    #[tokio::test]
    async fn remote_worktree_sync_builds_the_bundle_on_the_hub() {
        use crate::environment_router::MockEnvironmentTransport;
        use pandamux_core::EnvironmentId;
        use std::sync::atomic::{AtomicBool, Ordering};

        let origin = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let result = std::process::Command::new("git")
                .current_dir(origin.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        };
        run(&["init"]);
        run(&["config", "user.name", "Tester"]);
        run(&["config", "user.email", "test@example.com"]);
        std::fs::write(origin.path().join("delta.txt"), "unpushed delta").unwrap();
        run(&["add", "delta.txt"]);
        run(&["commit", "-m", "add unpushed delta"]);

        let store = Store::in_memory().unwrap();
        let router = Router::new(store, ServerRole::Hub, "env-local".to_string());
        let remote_env = EnvironmentId::new("env-remote-sync");
        let received_bundle = Arc::new(AtomicBool::new(false));
        let received_bundle_for_transport = Arc::clone(&received_bundle);
        let transport = Arc::new(MockEnvironmentTransport::new(move |request| {
            let id = request.id.clone().unwrap_or(1.into());
            let params = request.params.as_ref().unwrap();
            received_bundle_for_transport.store(
                params
                    .get("bundleBytesBase64")
                    .and_then(Value::as_str)
                    .is_some_and(|bundle| !bundle.is_empty()),
                Ordering::SeqCst,
            );
            Ok(RpcResponse::success(
                id,
                json!({
                    "synced": true,
                    "targetBranch": "r0-live-delta",
                    "bundleSizeBytes": 1,
                    "message": "Applied 1 commit(s) successfully"
                }),
            ))
        }));
        router
            .environment_router()
            .register_transport(remote_env.clone(), transport, None)
            .await;

        let response = router
            .handle_request(RpcRequest::new(
                1,
                "worktree.sync_delta",
                Some(json!({
                    "environmentId": remote_env,
                    "localPath": origin.path(),
                    "remotePath": "/remote/repository",
                    "revRange": "HEAD",
                    "targetBranch": "r0-live-delta"
                })),
            ))
            .await
            .unwrap();
        assert!(response.is_success());
        assert!(received_bundle.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn health_fixtures_are_bounded_persisted_and_never_start_sessions() {
        let store = Store::in_memory().expect("open health store");
        let scenarios = [
            (
                "missing",
                json!({
                    "installed": false,
                    "authState": "unknown",
                    "authConfidence": "unknown",
                    "runtimeHealth": "unavailable",
                    "runtimeCode": "binary_missing",
                    "probeDelayMs": 30
                }),
            ),
            (
                "unsupported",
                json!({
                    "version": "0.0.1",
                    "authState": "authenticated",
                    "runtimeHealth": "unavailable",
                    "runtimeCode": "unsupported_version",
                    "probeDelayMs": 30
                }),
            ),
            (
                "unknown-auth",
                json!({
                    "authState": "unknown",
                    "authConfidence": "unknown",
                    "runtimeHealth": "healthy",
                    "probeDelayMs": 30
                }),
            ),
            (
                "logged-out",
                json!({
                    "authState": "unauthenticated",
                    "authConfidence": "reported",
                    "runtimeHealth": "healthy",
                    "probeDelayMs": 30
                }),
            ),
            (
                "logged-in",
                json!({
                    "authState": "authenticated",
                    "authConfidence": "reported",
                    "runtimeHealth": "healthy",
                    "accountLabel": "person@example.com",
                    "subscription": "Team",
                    "probeDelayMs": 30
                }),
            ),
            (
                "malformed",
                json!({
                    "authState": "unknown",
                    "authConfidence": "unknown",
                    "runtimeHealth": "degraded",
                    "runtimeCode": "malformed_output",
                    "healthReason": "Provider returned malformed probe output",
                    "probeDelayMs": 30
                }),
            ),
            (
                "timeout",
                json!({
                    "authState": "authenticated",
                    "runtimeHealth": "healthy",
                    "probeDelayMs": 150
                }),
            ),
        ];
        let providers = scenarios
            .into_iter()
            .map(|(id, settings)| pandamux_core::ProviderInstanceConfig {
                id: pandamux_core::ProviderInstanceId::new(id),
                provider: ProviderKind::Custom,
                display_name: id.to_string(),
                profile_dir: format!("profiles/custom/{id}"),
                environment_id: None,
                binary_path: Some(format!("/mock/bin/{id}")),
                concurrency_limit: 1,
                env_overrides: Default::default(),
                settings,
                enabled: true,
                disabled_reason: None,
            })
            .collect::<Vec<_>>();
        store
            .save_settings(&UserSettings {
                providers,
                ..UserSettings::default()
            })
            .expect("save health fixtures");
        let driver = Arc::new(pandamux_providers::MockProviderDriver::new(
            ProviderKind::Custom,
            "Health fixture",
        ));
        let drivers = Arc::new(DriverRegistry::new());
        drivers.register(ProviderKind::Custom, driver.clone());
        let router = Router::with_drivers(
            store.clone(),
            ServerRole::Hub,
            "env-health".to_string(),
            drivers,
        );

        let response = router
            .handle_request(RpcRequest::new(
                1,
                "provider.health",
                Some(json!({
                    "timeoutMs": 80,
                    "maxConcurrency": 2
                })),
            ))
            .await
            .expect("health response");
        let result: ProviderHealthResult =
            serde_json::from_value(response.result.expect("health result"))
                .expect("typed health result");
        assert_eq!(result.reports.len(), 7);
        let report = |id: &str| {
            result
                .reports
                .iter()
                .find(|report| report.instance_id.as_str() == id)
                .expect("health fixture report")
        };
        assert_eq!(
            report("missing").runtime_code.as_deref(),
            Some("binary_missing")
        );
        assert_eq!(
            report("unsupported").runtime_code.as_deref(),
            Some("unsupported_version")
        );
        assert_eq!(report("unknown-auth").status, "unknown");
        assert_eq!(report("logged-out").status, "unauthenticated");
        assert_eq!(report("logged-in").status, "healthy");
        assert_eq!(
            report("logged-in").account_badge.as_deref(),
            Some("p***@example.com (Te***)")
        );
        assert_eq!(report("logged-in").subscription.as_deref(), Some("Te***"));
        assert!(
            !serde_json::to_string(report("logged-in"))
                .expect("serialize masked health report")
                .contains("Team")
        );
        assert_eq!(report("malformed").status, "degraded");
        assert_eq!(report("timeout").runtime_status, "timeout");
        assert!(report("timeout").retryable);
        assert!(driver.max_concurrent_probes() >= 2);
        assert!(driver.max_concurrent_probes() <= 2);
        assert_eq!(driver.session_start_count(), 0);
        assert_eq!(
            store
                .provider_health_snapshot_count()
                .expect("health snapshot count"),
            7
        );

        let first_success = report("logged-in")
            .last_success_at_ms
            .expect("healthy last success");
        let mut settings = store
            .get_settings()
            .expect("load health settings")
            .expect("health settings");
        settings
            .providers
            .iter_mut()
            .find(|config| config.id.as_str() == "logged-in")
            .expect("logged-in config")
            .settings = json!({
            "authState": "unknown",
            "authConfidence": "unknown",
            "runtimeHealth": "unavailable",
            "runtimeCode": "remote_probe_failed"
        });
        store
            .save_settings(&settings)
            .expect("save failed health state");
        let failed = router
            .handle_request(RpcRequest::new(
                2,
                "provider.health",
                Some(json!({
                    "instanceId": "logged-in",
                    "timeoutMs": 80,
                    "maxConcurrency": 1
                })),
            ))
            .await
            .expect("failed health response");
        let failed: ProviderHealthResult =
            serde_json::from_value(failed.result.expect("failed health result"))
                .expect("typed failed health result");
        assert_eq!(failed.reports[0].status, "unavailable");
        assert_eq!(failed.reports[0].last_success_at_ms, Some(first_success));
        assert_eq!(driver.session_start_count(), 0);
    }

    #[tokio::test]
    async fn remote_health_and_login_launch_failures_remain_visible_and_retryable() {
        let store = Store::in_memory().expect("open provider action store");
        let provider = pandamux_core::ProviderInstanceConfig {
            id: pandamux_core::ProviderInstanceId::new("claude-login"),
            provider: ProviderKind::Claude,
            display_name: "Claude Login".to_string(),
            profile_dir: "profiles/claude/login".to_string(),
            environment_id: None,
            binary_path: Some("/mock/bin/claude".to_string()),
            concurrency_limit: 1,
            env_overrides: Default::default(),
            settings: json!({ "authState": "unauthenticated" }),
            enabled: true,
            disabled_reason: None,
        };
        store
            .save_settings(&UserSettings {
                providers: vec![provider],
                ..UserSettings::default()
            })
            .expect("save login provider");
        let driver = Arc::new(pandamux_providers::MockProviderDriver::new(
            ProviderKind::Claude,
            "Login fixture",
        ));
        let drivers = Arc::new(DriverRegistry::new());
        drivers.register(ProviderKind::Claude, driver.clone());
        let router = Router::with_drivers(store, ServerRole::Hub, "env-login".to_string(), drivers);

        let unreachable = router
            .handle_request(RpcRequest::new(
                1,
                "provider.health",
                Some(json!({ "environmentId": "env-offline" })),
            ))
            .await
            .expect("unreachable response");
        let unreachable = unreachable.error.expect("unreachable error");
        let unreachable_data = unreachable.data.expect("unreachable data");
        assert_eq!(unreachable_data["state"], "unreachable");
        assert_eq!(unreachable_data["retryable"], true);

        let preview = router
            .handle_request(RpcRequest::new(
                2,
                "provider.login.preview",
                Some(json!({
                    "providerInstanceId": "claude-login",
                    "environmentId": "env-login"
                })),
            ))
            .await
            .expect("preview response");
        let preview: ProviderLoginPreview =
            serde_json::from_value(preview.result.expect("preview result")).expect("typed preview");
        assert_eq!(preview.flow, ProviderLoginFlow::Terminal);
        assert!(preview.display_command.contains("auth"));
        assert!(preview.display_command.contains("login"));
        assert!(router.terminal.list().terminals.is_empty());

        let failed_launch = router
            .handle_request(RpcRequest::new(
                3,
                "provider.login.launch",
                Some(json!({
                    "previewId": preview.preview_id.clone(),
                    "providerInstanceId": "different-instance",
                    "environmentId": "env-login"
                })),
            ))
            .await
            .expect("failed launch response");
        let failed_launch = failed_launch.error.expect("failed launch error");
        let data = failed_launch.data.expect("failed launch data");
        assert_eq!(data["reason"], "provider_login_preview_mismatch");
        assert_eq!(data["retryable"], true);
        assert!(router.terminal.list().terminals.is_empty());

        let invalid_shell = tempfile::tempdir().expect("invalid shell directory");
        router
            .login_previews
            .lock()
            .expect("login preview registry")
            .get_mut(&preview.preview_id)
            .expect("retained login preview")
            .shell = Some(invalid_shell.path().to_string_lossy().to_string());
        let terminal_failure = router
            .handle_request(RpcRequest::new(
                4,
                "provider.login.launch",
                Some(json!({
                    "previewId": preview.preview_id.clone(),
                    "providerInstanceId": "claude-login",
                    "environmentId": "env-login"
                })),
            ))
            .await
            .expect("terminal failure response")
            .error
            .expect("terminal failure error");
        let terminal_failure = terminal_failure.data.expect("terminal failure data");
        assert_eq!(
            terminal_failure["reason"],
            "provider_login_terminal_open_failed"
        );
        assert_eq!(terminal_failure["retryable"], true);
        assert!(router.terminal.list().terminals.is_empty());
        assert_eq!(driver.session_start_count(), 0);
    }

    #[tokio::test]
    async fn capacity_refresh_and_context_bound_push_preserve_safer_local_evidence() {
        let store = Store::in_memory().expect("open capacity store");
        let environment_id = EnvironmentId::new("env-capacity");
        let instance_id = pandamux_core::ProviderInstanceId::new("custom-capacity");
        let subscription_key = pandamux_core::SubscriptionKey::new("subkey-capacity");
        store
            .save_settings(&UserSettings {
                providers: vec![pandamux_core::ProviderInstanceConfig {
                    id: instance_id.clone(),
                    provider: ProviderKind::Custom,
                    display_name: "Capacity fixture".to_string(),
                    profile_dir: "profiles/custom/capacity".to_string(),
                    environment_id: None,
                    binary_path: Some("/mock/bin/capacity".to_string()),
                    concurrency_limit: 1,
                    env_overrides: Default::default(),
                    settings: json!({
                        "capacityReport": {
                            "source": "fixture_report",
                            "confidence": "reported",
                            "providerVersion": "fixture-2",
                            "asOfMs": 1_000_000,
                            "limits": [{
                                "limitName": "primary",
                                "usedMillionths": 400_000,
                                "remainingMillionths": 600_000,
                                "windowDurationMins": 60,
                                "resetsAtMs": 2_000_000,
                                "hardLimit": false
                            }]
                        }
                    }),
                    enabled: true,
                    disabled_reason: None,
                }],
                ..UserSettings::default()
            })
            .expect("save capacity settings");
        store
            .save_usage_subscription(
                &pandamux_core::UsageSubscription {
                    key: subscription_key.clone(),
                    provider: pandamux_core::UsageProvider::Custom,
                    display_name: "Capacity subscription".to_string(),
                    masked_account_label: None,
                },
                1,
            )
            .expect("save capacity subscription");
        store
            .map_provider_subscription(&environment_id, &instance_id, &subscription_key, 1)
            .expect("map capacity subscription");
        let drivers = Arc::new(DriverRegistry::new());
        drivers.register(
            ProviderKind::Custom,
            Arc::new(pandamux_providers::MockProviderDriver::new(
                ProviderKind::Custom,
                "Capacity fixture",
            )),
        );
        let router = Router::with_drivers(
            store.clone(),
            ServerRole::Hub,
            environment_id.as_str().to_string(),
            drivers,
        );

        let refreshed = router
            .handle_request(RpcRequest::new(
                1,
                "capacity.refresh",
                Some(json!({
                    "environmentId": environment_id,
                    "providerInstanceId": instance_id,
                    "timeoutMs": 1000
                })),
            ))
            .await
            .expect("capacity refresh response");
        let refreshed: CapacityRefreshResult =
            serde_json::from_value(refreshed.result.expect("capacity refresh result"))
                .expect("typed capacity refresh");
        assert_eq!(refreshed.snapshots.len(), 1);
        assert_eq!(refreshed.snapshots[0].remaining_millionths, Some(600_000));
        assert_eq!(
            refreshed.snapshots[0].confidence,
            CapacityConfidence::Reported
        );

        let local_hard_limit = CapacitySnapshot {
            schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION,
            subscription_key: subscription_key.clone(),
            environment_id: Some(environment_id.clone()),
            provider_instance_id: Some(instance_id.clone()),
            window_name: Some("primary".to_string()),
            window_start_ms: Some(900_000),
            window_end_ms: Some(2_000_000),
            used_millionths: Some(1_000_000),
            remaining_millionths: Some(0),
            reset_at_ms: Some(2_000_000),
            source: "local_rate_limit".to_string(),
            confidence: CapacityConfidence::Reported,
            evidence_scope: CapacityEvidenceScope::Local,
            hard_limit: true,
            provider_version: Some("fixture-2".to_string()),
            as_of_ms: 900_000,
            max_age_ms: 1,
        };
        store
            .save_capacity_snapshot(&local_hard_limit)
            .expect("save local hard limit");
        let hub_snapshot = CapacitySnapshot {
            schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION,
            subscription_key: subscription_key.clone(),
            environment_id: Some(EnvironmentId::new("spoofed-environment")),
            provider_instance_id: Some(instance_id.clone()),
            window_name: Some("primary".to_string()),
            window_start_ms: Some(900_000),
            window_end_ms: Some(2_000_000),
            used_millionths: Some(100_000),
            remaining_millionths: Some(900_000),
            reset_at_ms: Some(2_000_000),
            source: "hub_projection".to_string(),
            confidence: CapacityConfidence::Reported,
            evidence_scope: CapacityEvidenceScope::Local,
            hard_limit: false,
            provider_version: Some("fixture-2".to_string()),
            as_of_ms: 1_010_000,
            max_age_ms: DEFAULT_CAPACITY_MAX_AGE_MS,
        };
        let node_context = RequestContext::node_proxy(
            environment_id.clone(),
            crate::request_context::ClientIdentity::default(),
        );
        let pushed = router
            .handle_request_with_context(
                &node_context,
                RpcRequest::new(
                    2,
                    "capacity.push",
                    Some(
                        serde_json::to_value(CapacityPushParams {
                            environment_id: environment_id.clone(),
                            schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION,
                            issued_at_ms: 1_010_000,
                            max_age_ms: DEFAULT_CAPACITY_MAX_AGE_MS,
                            snapshots: vec![hub_snapshot],
                        })
                        .expect("serialize capacity push"),
                    ),
                ),
            )
            .await
            .expect("capacity push response");
        assert!(pushed.is_success());

        let queried = store
            .query_capacity(&CapacityQueryParams {
                subscription_key: Some(subscription_key),
                environment_id: Some(environment_id.clone()),
                at_ms: Some(1_010_000),
                after: None,
                limit: 100,
            })
            .expect("query effective capacity");
        let effective = queried
            .effective
            .iter()
            .find(|projection| projection.window_name.as_deref() == Some("primary"))
            .expect("primary capacity projection");
        assert_eq!(effective.remaining_millionths, Some(0));
        assert!(effective.hard_limited);
        assert!(effective.conflicting_evidence);
        assert!(queried.snapshots.iter().any(|snapshot| {
            snapshot.evidence_scope == CapacityEvidenceScope::Hub
                && snapshot.environment_id.as_ref() == Some(&environment_id)
        }));

        store
            .record_usage_sync_failure(&environment_id, true, "capacity_fixture_offline")
            .expect("record offline environment");
        let offline = store
            .query_capacity(&CapacityQueryParams {
                environment_id: Some(environment_id.clone()),
                at_ms: Some(1_010_000),
                ..CapacityQueryParams::default()
            })
            .expect("query offline coverage");
        assert_eq!(offline.offline_environments, vec![environment_id]);
    }
}
