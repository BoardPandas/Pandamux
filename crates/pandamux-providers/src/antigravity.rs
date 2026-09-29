//! Google Antigravity provider driver and ACP profile layer.
//!
//! The managed runtime and authentication support are Rust ports of the T3
//! Code implementation listed in `THIRD_PARTY_NOTICES.md`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use pandamux_core::provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind};
use serde_json::Value;

pub use crate::antigravity_auth::{
    ANTIGRAVITY_AUTH_BROWSER_MARKER, ANTIGRAVITY_AUTH_STDOUT_PREFIX, AntigravityAuthManager,
    AntigravityAuthStart, AntigravityProcessEnv, MIN_FREE_DISK_BYTES, ValidatedOAuthUrl,
    browser_helper_command, build_antigravity_env, execute_oauth_relay_callback,
    extract_auth_url_from_output, list_antigravity_models, preflight_browser_helper,
    validate_callback_query, validate_google_oauth_url, validate_sanitized_environment,
};
pub use crate::antigravity_release::{
    ResolvedAntigravityExecutable, cache_bundle_archive, current_bundle_manifest,
    discard_bundle_upload, get_available_disk_bytes, install_managed_bundle,
    pinned_bundle_manifest, platform_arch, preflight_disk_space, prepare_bundle_upload,
    required_install_bytes, resolve_antigravity_binary, resolve_antigravity_executable,
    validate_pinned_manifest, verify_bundle_archive_file,
};
pub use pandamux_protocol::antigravity_rpc::{
    AntigravityInstallResult, ManagedBundleFile, ManagedBundleManifest,
};

use crate::antigravity_auth::start_antigravity_session;
use crate::error::ProviderError;
use crate::models::{
    ModelInfo, ProviderAuthConfidence, ProviderAuthStatus, ProviderHealth, ProviderMetadata,
    ProviderSnapshot, SessionSpec, UsageLimits, provider_snapshot_expiry, provider_snapshot_now_ms,
};
use crate::profiles::resolve_instance_profile_dir;
use crate::traits::{BoxFuture, ProviderDriver, ProviderSession};

pub const MAX_TOOL_PAYLOAD_BYTES: usize = 64 * 1024;
const MODEL_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
type AntigravityModelCache = Arc<Mutex<HashMap<String, (Instant, Vec<ModelInfo>)>>>;
static ANTIGRAVITY_PROCESS_LIMITER: OnceLock<Arc<AntigravityConcurrencyLimiter>> = OnceLock::new();

pub(crate) fn shared_antigravity_process_limiter() -> Arc<AntigravityConcurrencyLimiter> {
    ANTIGRAVITY_PROCESS_LIMITER
        .get_or_init(|| Arc::new(AntigravityConcurrencyLimiter::new(2)))
        .clone()
}

/// Returns the live environment-wide Antigravity process count and hard ceiling.
pub fn antigravity_process_counts() -> (usize, usize) {
    let limiter = shared_antigravity_process_limiter();
    (limiter.active_count(), limiter.max_processes())
}

/// Zero-spawn offline health check. The one-file runtime is never launched for polling.
pub fn probe_antigravity_health_offline(
    binary_path: &Path,
    gemini_home: &Path,
    cached_version: Option<&str>,
) -> ProviderHealth {
    if !binary_path.is_file() {
        return ProviderHealth::Unavailable {
            reason: "Antigravity runtime is not installed".to_string(),
        };
    }

    let acp_directory = gemini_home.join("antigravity-acp");
    let settings_path = acp_directory.join("settings.json");
    let token_path = acp_directory.join("acp_token.json");
    let selected_personal_auth = fs::read_to_string(&settings_path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
        .and_then(|settings| {
            settings
                .get("auth")
                .and_then(|auth| auth.get("type"))
                .and_then(Value::as_str)
                .map(|auth_type| auth_type == "oauth-personal")
        })
        .unwrap_or(false);
    let token_present = fs::symlink_metadata(&token_path).is_ok_and(|metadata| {
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() > 0
    });

    if selected_personal_auth && token_present {
        ProviderHealth::Healthy
    } else {
        ProviderHealth::Degraded {
            reason: format!(
                "Antigravity {} is installed but Google sign-in is incomplete",
                cached_version.unwrap_or("runtime")
            ),
        }
    }
}

/// Sweeps only known PyInstaller orphan directory patterns from app-owned scratch space.
pub fn sweep_orphan_temp_dirs(scratch_dir: &Path) -> Result<usize, ProviderError> {
    if !scratch_dir.exists() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in fs::read_dir(scratch_dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && (name.starts_with("_MEI") || name.starts_with("agy_tmp_"))
        {
            fs::remove_dir_all(path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Local process limiter retained for direct driver use. The server resolver also
/// holds the configured provider-instance permit for the complete process lifetime.
pub struct AntigravityConcurrencyLimiter {
    active_count: AtomicUsize,
    max_processes: usize,
}

impl AntigravityConcurrencyLimiter {
    pub fn new(max_processes: usize) -> Self {
        Self {
            active_count: AtomicUsize::new(0),
            max_processes,
        }
    }

    pub fn try_acquire(self: &Arc<Self>) -> Result<ConcurrencyGuard, ProviderError> {
        let current = self.active_count.fetch_add(1, Ordering::SeqCst);
        if current >= self.max_processes {
            self.active_count.fetch_sub(1, Ordering::SeqCst);
            return Err(ProviderError::SupervisionError {
                message: format!(
                    "Antigravity concurrency limit reached ({current} of {} active)",
                    self.max_processes
                ),
            });
        }
        Ok(ConcurrencyGuard {
            limiter: Arc::clone(self),
        })
    }

    pub fn active_count(&self) -> usize {
        self.active_count.load(Ordering::SeqCst)
    }

    pub fn max_processes(&self) -> usize {
        self.max_processes
    }
}

pub struct ConcurrencyGuard {
    limiter: Arc<AntigravityConcurrencyLimiter>,
}

impl Drop for ConcurrencyGuard {
    fn drop(&mut self) {
        self.limiter.active_count.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct AntigravityDriver {
    pub base_data_dir: PathBuf,
    pub limiter: Arc<AntigravityConcurrencyLimiter>,
    model_cache: AntigravityModelCache,
}

impl AntigravityDriver {
    pub fn new(base_data_dir: PathBuf) -> Self {
        Self {
            base_data_dir,
            limiter: shared_antigravity_process_limiter(),
            model_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn resolve_executable(
        &self,
        config: &ProviderInstanceConfig,
    ) -> Option<ResolvedAntigravityExecutable> {
        let manifest = current_bundle_manifest()?;
        let explicit = config.binary_path.as_deref().map(Path::new).or_else(|| {
            config
                .settings
                .get("binaryPath")
                .and_then(Value::as_str)
                .map(Path::new)
        });
        resolve_antigravity_executable(explicit, &self.base_data_dir, &platform_arch(&manifest))
    }
}

impl ProviderDriver for AntigravityDriver {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            id: ProviderKind::Antigravity,
            display_name: "Google Antigravity".to_string(),
            capabilities: ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calls: true,
                supports_sub_agents: true,
                supports_system_prompt: true,
                supports_reasoning: true,
                supports_approvals: true,
                supports_rate_limits: false,
                supports_resume: true,
                supports_images: true,
            },
            supported_models: Vec::new(),
        }
    }

    fn probe<'a>(&'a self, config: &'a ProviderInstanceConfig) -> BoxFuture<'a, ProviderSnapshot> {
        Box::pin(async move {
            let resolved = self.resolve_executable(config);
            let gemini_home = resolve_instance_profile_dir(&self.base_data_dir, config);
            let checked_at_ms = provider_snapshot_now_ms();
            let expires_at_ms = provider_snapshot_expiry(checked_at_ms);
            match resolved {
                Some(executable) => {
                    let health = probe_antigravity_health_offline(
                        &executable.executable_path,
                        &gemini_home,
                        executable.version.as_deref(),
                    );
                    let auth = if matches!(health, ProviderHealth::Healthy) {
                        ProviderAuthStatus::Configured
                    } else {
                        ProviderAuthStatus::Unauthenticated
                    };
                    ProviderSnapshot {
                        installed: true,
                        resolved_executable: Some(
                            executable.executable_path.to_string_lossy().to_string(),
                        ),
                        version: executable.version,
                        auth,
                        auth_confidence: ProviderAuthConfidence::Detected,
                        health,
                        runtime_code: Some("offline_manifest_probe".to_string()),
                        checked_at_ms,
                        expires_at_ms,
                    }
                }
                None => ProviderSnapshot {
                    installed: false,
                    resolved_executable: None,
                    version: None,
                    auth: ProviderAuthStatus::Unknown,
                    auth_confidence: ProviderAuthConfidence::Unknown,
                    health: ProviderHealth::Unavailable {
                        reason: "Antigravity ACP executable and sibling harness were not found"
                            .to_string(),
                    },
                    runtime_code: Some("binary_missing".to_string()),
                    checked_at_ms,
                    expires_at_ms,
                },
            }
        })
    }

    fn list_models<'a>(
        &'a self,
        config: &'a ProviderInstanceConfig,
    ) -> BoxFuture<'a, Result<Vec<ModelInfo>, ProviderError>> {
        Box::pin(async move {
            let cache_key = config.id.as_str().to_string();
            if let Ok(cache) = self.model_cache.lock()
                && let Some((cached_at, models)) = cache.get(&cache_key)
                && cached_at.elapsed() <= MODEL_CACHE_TTL
            {
                return Ok(models.clone());
            }
            let models = list_antigravity_models(config, &self.base_data_dir).await?;
            if let Ok(mut cache) = self.model_cache.lock() {
                cache.insert(cache_key, (Instant::now(), models.clone()));
            }
            Ok(models)
        })
    }

    fn usage_limits<'a>(
        &'a self,
        _config: &'a ProviderInstanceConfig,
    ) -> BoxFuture<'a, Option<UsageLimits>> {
        Box::pin(async { None })
    }

    fn start_session<'a>(
        &'a self,
        config: &'a ProviderInstanceConfig,
        spec: SessionSpec,
    ) -> BoxFuture<'a, Result<Box<dyn ProviderSession>, ProviderError>> {
        Box::pin(async move {
            let scratch_root = self.base_data_dir.join("scratch/antigravity");
            let _ = sweep_orphan_temp_dirs(&scratch_root)?;
            let preamble = spec
                .instructions
                .as_ref()
                .map(|instructions| instructions.format_preamble());
            let session = start_antigravity_session(config, &self.base_data_dir, &spec)
                .await?
                .with_preamble(preamble);
            Ok(Box::new(session) as Box<dyn ProviderSession>)
        })
    }
}
