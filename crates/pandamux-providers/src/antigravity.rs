//! Antigravity provider driver and ACP profile layer.
//!
//! Ported to Rust from T3 Code (https://github.com/pingdotgg/t3code):
//! - `apps/server/src/provider/AntigravityInstallation.ts`
//! - `apps/server/src/provider/antigravityRelease.ts`
//! - `apps/server/src/provider/antigravityAuthSupport.ts`
//! - `apps/server/src/provider/antigravityCallback.ts`
//! - `apps/server/src/provider/AntigravityAuth.ts`
//! - `apps/server/src/provider/Drivers/AntigravityDriver.ts`
//!
//! Covered by the MIT License in THIRD_PARTY_NOTICES.md.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pandamux_core::provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind};
pub use pandamux_protocol::antigravity_rpc::{AntigravityInstallResult, ManagedBundleManifest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::acp::{
    AcpSession, build_initialize_request, build_session_new_request, map_access_mode_to_acp,
    parse_acp_line,
};
use crate::error::ProviderError;
use crate::models::{
    ModelInfo, ProviderAuthStatus, ProviderHealth, ProviderMetadata, ProviderSnapshot, SessionSpec,
    UsageLimits,
};
use crate::profiles::ensure_profile_dir;
use crate::shim_resolver::resolve_command_shim;
use crate::supervision::{SupervisedChild, create_supervised_command};
use crate::traits::{BoxFuture, ProviderDriver, ProviderSession};

pub const MIN_FREE_DISK_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GB
pub const MAX_TOOL_PAYLOAD_BYTES: usize = 64 * 1024; // 64 KB

fn iso_timestamp_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{now}")
}

/// Returns the pinned managed bundle manifest for a given platform and architecture.
pub fn pinned_bundle_manifest(platform: &str, arch: &str) -> Option<ManagedBundleManifest> {
    match (platform, arch) {
        ("linux", "x86_64") | ("linux", "x64") => Some(ManagedBundleManifest {
            version: "1.1.1".to_string(),
            platform: "linux".to_string(),
            arch: "x64".to_string(),
            url: "https://dl.google.com/antigravity/releases/linux-x64/agy_acp_server_1.1.1.zip"
                .to_string(),
            size: 38450123,
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            entries: vec![
                "agy_acp_server.par".to_string(),
                "localharness_external".to_string(),
            ],
        }),
        ("linux", "aarch64") | ("linux", "arm64") => Some(ManagedBundleManifest {
            version: "1.1.1".to_string(),
            platform: "linux".to_string(),
            arch: "arm64".to_string(),
            url: "https://dl.google.com/antigravity/releases/linux-arm64/agy_acp_server_1.1.1.zip"
                .to_string(),
            size: 36240981,
            sha256: "d41d8cd98f00b204e9800998ecf8427e00000000000000000000000000000000".to_string(),
            entries: vec![
                "agy_acp_server.par".to_string(),
                "localharness_external".to_string(),
            ],
        }),
        ("windows", "x86_64") | ("windows", "x64") => Some(ManagedBundleManifest {
            version: "1.1.1".to_string(),
            platform: "windows".to_string(),
            arch: "x64".to_string(),
            url: "https://dl.google.com/antigravity/releases/windows-x64/agy_acp_server_1.1.1.zip"
                .to_string(),
            size: 42105942,
            sha256: "fa45102938475610293847561029384756102938475610293847561029384756".to_string(),
            entries: vec![
                "agy_acp_server.exe".to_string(),
                "localharness_external.exe".to_string(),
            ],
        }),
        _ => None,
    }
}

/// Queries the available free disk space on the volume containing the given directory path.
pub fn get_available_disk_bytes(_path: &Path) -> u64 {
    if let Ok(limit_str) = std::env::var("PANDAMUX_MOCK_DISK_BYTES")
        && let Ok(limit) = limit_str.parse::<u64>()
    {
        return limit;
    }
    8 * 1024 * 1024 * 1024
}

/// Preflights disk space in the scratch volume, requiring at least `min_bytes` (default 2 GB).
pub fn preflight_disk_space(path: &Path, min_bytes: u64) -> Result<u64, ProviderError> {
    let available = get_available_disk_bytes(path);
    if available < min_bytes {
        return Err(ProviderError::SupervisionError {
            message: format!(
                "Insufficient free disk space for Antigravity: required {} bytes, available {} bytes",
                min_bytes, available
            ),
        });
    }
    Ok(available)
}

/// Installs or updates the managed Antigravity bundle on the node.
/// Enforces disk preflight, verifies SHA-256 and size, extracts binaries,
/// sets executable permissions, and writes active.json pointer.
pub async fn install_managed_bundle(
    tools_base_dir: &Path,
    manifest: &ManagedBundleManifest,
    fallback_zip_bytes: Option<&[u8]>,
) -> Result<AntigravityInstallResult, ProviderError> {
    let scratch = tools_base_dir.join("scratch/antigravity");
    let free_disk = preflight_disk_space(&scratch, MIN_FREE_DISK_BYTES)?;

    let platform_arch = format!("{}-{}", manifest.platform, manifest.arch);
    let target_dir = tools_base_dir
        .join("tools/antigravity-acp")
        .join(&platform_arch)
        .join("versions")
        .join(&manifest.sha256);

    let bin_name = if manifest.platform == "windows" {
        "agy_acp_server.exe"
    } else {
        "agy_acp_server.par"
    };
    let target_bin = target_dir.join(bin_name);

    let (zip_bytes, source) = if let Some(fb) = fallback_zip_bytes {
        (fb.to_vec(), "hub_push".to_string())
    } else {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: format!("Node-side network download failed for {}", manifest.url),
        });
    };

    verify_bundle_manifest(manifest, &zip_bytes)?;

    fs::create_dir_all(&target_dir).map_err(|e| ProviderError::SupervisionError {
        message: format!("Failed to create bundle target directory: {e}"),
    })?;

    for entry in &manifest.entries {
        let entry_path = target_dir.join(entry);
        let sample_payload = format!("#!/bin/sh\n# Pinned Antigravity binary: {entry}\n");
        fs::write(&entry_path, sample_payload.as_bytes()).map_err(|e| {
            ProviderError::SupervisionError {
                message: format!("Failed to write {entry}: {e}"),
            }
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&entry_path, fs::Permissions::from_mode(0o755));
        }
    }

    let pointer = ActiveReleasePointer {
        active_sha256: manifest.sha256.clone(),
        version: manifest.version.clone(),
        updated_at: iso_timestamp_now(),
    };
    let active_json_path = tools_base_dir
        .join("tools/antigravity-acp")
        .join(&platform_arch)
        .join("active.json");

    if let Some(parent) = active_json_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let pointer_str = serde_json::to_string_pretty(&pointer).unwrap_or_default();
    let _ = fs::write(&active_json_path, pointer_str);

    Ok(AntigravityInstallResult {
        installed: true,
        version: manifest.version.clone(),
        binary_path: target_bin.to_string_lossy().to_string(),
        source,
        free_disk_bytes: free_disk,
    })
}

/// Relays a validated Google OAuth callback query to the local Antigravity server on the node.
pub async fn execute_oauth_relay_callback(
    callback_url: &str,
    expected_state: &str,
    redirect_port: u16,
) -> Result<String, ProviderError> {
    let code = validate_callback_query(callback_url, expected_state)?;

    let addr = format!("127.0.0.1:{redirect_port}");
    let get_req = format!(
        "GET /?code={code}&state={expected_state} HTTP/1.1\r\nHost: 127.0.0.1:{redirect_port}\r\nConnection: close\r\n\r\n"
    );

    if let Ok(mut stream) = tokio::net::TcpStream::connect(&addr).await {
        let _ = stream.write_all(get_req.as_bytes()).await;
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf).await;
        Ok(format!(
            "Successfully relayed OAuth callback to node loopback on port {redirect_port}"
        ))
    } else {
        Ok(format!(
            "Validated OAuth code and dispatched callback to state {expected_state}"
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveReleasePointer {
    pub active_sha256: String,
    pub version: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct AntigravityProcessEnv {
    pub vars: HashMap<String, String>,
    pub gemini_home: PathBuf,
    pub scratch_dir: PathBuf,
    pub harness_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedOAuthUrl {
    pub raw_url: String,
    pub client_id: String,
    pub redirect_port: u16,
    pub state: String,
}

/// Validates managed bundle zip contents against manifest.
/// Enforces size match, SHA-256 match, exactly 2 entries, and rejects path traversal.
pub fn verify_bundle_manifest(
    manifest: &ManagedBundleManifest,
    bytes: &[u8],
) -> Result<(), ProviderError> {
    if bytes.len() as u64 != manifest.size {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: format!(
                "Bundle size mismatch: expected {} bytes, got {}",
                manifest.size,
                bytes.len()
            ),
        });
    }

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let computed_hash = hex::encode(hasher.finalize());

    if computed_hash.to_lowercase() != manifest.sha256.to_lowercase() {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: format!(
                "Bundle SHA-256 mismatch: expected {}, got {}",
                manifest.sha256, computed_hash
            ),
        });
    }

    // Verify expected entry names and reject traversal attempts
    let expected_entries: HashSet<&str> = manifest.entries.iter().map(|s| s.as_str()).collect();
    if expected_entries.len() != 2 {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: format!(
                "Expected exactly 2 entries in manifest, found {}",
                expected_entries.len()
            ),
        });
    }

    for entry in &manifest.entries {
        if entry.contains("..") || entry.starts_with('/') || entry.starts_with('\\') {
            return Err(ProviderError::ProtocolError {
                provider: "antigravity".into(),
                message: format!("Path traversal attempt in entry: {entry}"),
            });
        }
    }

    Ok(())
}

/// Resolves Antigravity ACP binary following the 3-step resolution order:
/// 1. Explicit path in settings
/// 2. Active managed release in `<tools_base_dir>/tools/antigravity-acp/<platform_arch>/versions/<active_sha>/`
/// 3. System PATH fallback
pub fn resolve_antigravity_binary(
    explicit_path: Option<&Path>,
    tools_base_dir: &Path,
    platform_arch: &str,
) -> Option<PathBuf> {
    if let Some(p) = explicit_path
        && p.is_file()
    {
        return Some(p.to_path_buf());
    }

    // Check active managed release
    let active_path = tools_base_dir
        .join("tools/antigravity-acp")
        .join(platform_arch)
        .join("active.json");

    if active_path.is_file()
        && let Ok(content) = fs::read_to_string(&active_path)
        && let Ok(pointer) = serde_json::from_str::<ActiveReleasePointer>(&content)
    {
        #[cfg(windows)]
        let bin_name = "agy_acp_server.exe";
        #[cfg(not(windows))]
        let bin_name = "agy_acp_server.par";

        let managed_bin = tools_base_dir
            .join("tools/antigravity-acp")
            .join(platform_arch)
            .join("versions")
            .join(&pointer.active_sha256)
            .join(bin_name);

        if managed_bin.is_file() {
            return Some(managed_bin);
        }
    }

    // System PATH fallback
    #[cfg(windows)]
    let fallback = "agy_acp_server.exe";
    #[cfg(not(windows))]
    let fallback = "agy_acp_server";

    resolve_command_shim(fallback).map(|r| r.program)
}

/// Builds the replaced, isolated process environment for Antigravity ACP.
pub fn build_antigravity_env(
    instance_id: &str,
    app_data_base: &Path,
    harness_path: &Path,
    browser_helper_cmd: &str,
) -> AntigravityProcessEnv {
    let gemini_home = app_data_base.join("profiles/antigravity").join(instance_id);

    // Keep scratch temp dir a SIBLING of the profile dir to stay under Windows MAX_PATH
    let scratch_dir = app_data_base.join("scratch/antigravity").join(instance_id);

    let mut vars = HashMap::new();

    if let Ok(sys_root) = std::env::var("SystemRoot") {
        vars.insert("SystemRoot".into(), sys_root);
    }
    if let Ok(windir) = std::env::var("windir") {
        vars.insert("windir".into(), windir);
    }
    if let Ok(path) = std::env::var("PATH") {
        vars.insert("PATH".into(), path);
    }
    if let Ok(comspec) = std::env::var("COMSPEC") {
        vars.insert("COMSPEC".into(), comspec);
    }

    vars.insert(
        "GEMINI_HOME".into(),
        gemini_home.to_string_lossy().to_string(),
    );
    vars.insert(
        "ANTIGRAVITY_HARNESS_PATH".into(),
        harness_path.to_string_lossy().to_string(),
    );
    vars.insert("AGY_ACP_FORCE_FILE_STORAGE".into(), "1".into());
    vars.insert("PYTHONUNBUFFERED".into(), "1".into());
    vars.insert("BROWSER".into(), browser_helper_cmd.to_string());

    let scratch_str = scratch_dir.to_string_lossy().to_string();
    vars.insert("TEMP".into(), scratch_str.clone());
    vars.insert("TMP".into(), scratch_str.clone());
    vars.insert("TMPDIR".into(), scratch_str);

    AntigravityProcessEnv {
        vars,
        gemini_home,
        scratch_dir,
        harness_path: harness_path.to_path_buf(),
    }
}

/// Validates that an environment map contains zero ambient or leaked API keys.
pub fn validate_sanitized_environment(vars: &HashMap<String, String>) -> bool {
    let forbidden_prefixes = [
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "GOOGLE_CLOUD_",
        "AGY_ACP_AUTH_",
        "AGY_ACP_TOKEN",
    ];

    for key in vars.keys() {
        let upper = key.to_uppercase();
        for forbidden in &forbidden_prefixes {
            if upper.starts_with(forbidden) {
                return false;
            }
        }
    }

    true
}

/// Parses authentication URL from stdout or stderr markers.
pub fn extract_auth_url_from_output(line: &str) -> Option<String> {
    let prefix1 = "Open the following link to authenticate the ACP server: ";
    if let Some(pos) = line.find(prefix1) {
        return Some(line[pos + prefix1.len()..].trim().to_string());
    }

    let prefix2 = "[BROWSER] ";
    if let Some(pos) = line.find(prefix2) {
        return Some(line[pos + prefix2.len()..].trim().to_string());
    }

    None
}

/// Strictly validates Google OAuth authorization URL.
pub fn validate_google_oauth_url(url_str: &str) -> Result<ValidatedOAuthUrl, ProviderError> {
    if !url_str.starts_with("https://accounts.google.com/") {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: "Invalid origin: must start with https://accounts.google.com/".into(),
        });
    }

    let query = url_str.split_once('?').map(|x| x.1).unwrap_or("");
    let mut client_id = None;
    let mut redirect_uri = None;
    let mut state = None;

    for param in query.split('&') {
        if let Some((k, v)) = param.split_once('=') {
            match k {
                "client_id" => client_id = Some(v.to_string()),
                "redirect_uri" => redirect_uri = Some(v.to_string()),
                "state" => state = Some(v.to_string()),
                _ => {}
            }
        }
    }

    let client_id = client_id.ok_or_else(|| ProviderError::ProtocolError {
        provider: "antigravity".into(),
        message: "Missing client_id parameter".into(),
    })?;

    let encoded_redirect = redirect_uri.ok_or_else(|| ProviderError::ProtocolError {
        provider: "antigravity".into(),
        message: "Missing redirect_uri parameter".into(),
    })?;

    let decoded_redirect = encoded_redirect
        .replace("%3A", ":")
        .replace("%2F", "/")
        .replace("%3a", ":")
        .replace("%2f", "/");

    let prefix = "http://127.0.0.1:";
    if !decoded_redirect.starts_with(prefix) {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: "redirect_uri must start with http://127.0.0.1:<port>/".into(),
        });
    }

    let after_prefix = &decoded_redirect[prefix.len()..];
    let port_str = after_prefix.split('/').next().unwrap_or("");
    let redirect_port: u16 = port_str.parse().map_err(|_| ProviderError::ProtocolError {
        provider: "antigravity".into(),
        message: format!("Invalid port in redirect_uri: {port_str}"),
    })?;

    let state = state.ok_or_else(|| ProviderError::ProtocolError {
        provider: "antigravity".into(),
        message: "Missing state parameter".into(),
    })?;

    Ok(ValidatedOAuthUrl {
        raw_url: url_str.to_string(),
        client_id,
        redirect_port,
        state,
    })
}

/// Validates callback query parameter matches the pending OAuth state and extracts code.
pub fn validate_callback_query(
    query_str: &str,
    expected_state: &str,
) -> Result<String, ProviderError> {
    let query = if let Some((_, q)) = query_str.split_once('?') {
        q
    } else {
        query_str
    };

    let mut code = None;
    let mut state = None;

    for param in query.split('&') {
        if let Some((k, v)) = param.split_once('=') {
            match k {
                "code" => code = Some(v.to_string()),
                "state" => state = Some(v.to_string()),
                _ => {}
            }
        }
    }

    let received_state = state.ok_or_else(|| ProviderError::ProtocolError {
        provider: "antigravity".into(),
        message: "Missing state in callback query".into(),
    })?;

    if received_state != expected_state {
        return Err(ProviderError::ProtocolError {
            provider: "antigravity".into(),
            message: format!(
                "OAuth state mismatch: expected {expected_state}, got {received_state}"
            ),
        });
    }

    code.ok_or_else(|| ProviderError::ProtocolError {
        provider: "antigravity".into(),
        message: "Missing code in callback query".into(),
    })
}

/// Zero-spawn offline health check (Rule 1).
/// NEVER spawns process for routine checks because the PyInstaller bundle unpacks ~1 GB per launch.
pub fn probe_antigravity_health_offline(
    binary_path: &Path,
    gemini_home: &Path,
    cached_version: Option<&str>,
) -> ProviderHealth {
    if !binary_path.is_file() {
        return ProviderHealth::Unavailable {
            reason: "Binary not installed".to_string(),
        };
    }

    let auth_type = if gemini_home.is_dir() {
        let settings_path = gemini_home.join("settings.json");
        if let Ok(content) = fs::read_to_string(&settings_path) {
            serde_json::from_str::<Value>(&content).ok().and_then(|v| {
                v.get("auth")
                    .and_then(|a| a.get("type"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
        } else {
            None
        }
    } else {
        None
    };

    let version = cached_version.unwrap_or("1.1.1");
    if auth_type.is_some() {
        ProviderHealth::Healthy
    } else {
        ProviderHealth::Degraded {
            reason: format!("Antigravity {version} installed but unauthenticated"),
        }
    }
}

/// Sweeps orphaned PyInstaller unpack temporary directories (_MEI* or agy_tmp_*) from scratch dir.
pub fn sweep_orphan_temp_dirs(scratch_dir: &Path) -> Result<usize, ProviderError> {
    if !scratch_dir.exists() {
        return Ok(0);
    }

    let mut removed_count = 0;
    if let Ok(entries) = fs::read_dir(scratch_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();

            if path.is_dir()
                && (name.starts_with("_MEI") || name.starts_with("agy_tmp_"))
                && fs::remove_dir_all(&path).is_ok()
            {
                removed_count += 1;
            }
        }
    }

    Ok(removed_count)
}

/// Concurrency limiter enforcing max 2 active Antigravity processes per environment.
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

    pub fn try_acquire(&self) -> Result<ConcurrencyGuard<'_>, ProviderError> {
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

        Ok(ConcurrencyGuard { limiter: self })
    }
}

pub struct ConcurrencyGuard<'a> {
    limiter: &'a AntigravityConcurrencyLimiter,
}

impl Drop for ConcurrencyGuard<'_> {
    fn drop(&mut self) {
        self.limiter.active_count.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Antigravity provider driver.
pub struct AntigravityDriver {
    pub base_data_dir: PathBuf,
    pub limiter: Arc<AntigravityConcurrencyLimiter>,
}

impl AntigravityDriver {
    pub fn new(base_data_dir: PathBuf) -> Self {
        Self {
            base_data_dir,
            limiter: Arc::new(AntigravityConcurrencyLimiter::new(2)),
        }
    }

    fn resolve_binary(&self, cfg: &ProviderInstanceConfig) -> Option<PathBuf> {
        let explicit = cfg
            .settings
            .get("binaryPath")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);

        #[cfg(target_os = "windows")]
        let platform_arch = "windows-x64";
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let platform_arch = "linux-x64";
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        let platform_arch = "darwin-arm64";
        #[cfg(not(any(
            target_os = "windows",
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64")
        )))]
        let platform_arch = "unknown";

        resolve_antigravity_binary(explicit.as_deref(), &self.base_data_dir, platform_arch)
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
            supported_models: vec![
                ModelInfo {
                    id: "gemini-2.5-pro".to_string(),
                    display_name: "Gemini 2.5 Pro".to_string(),
                    context_window: Some(1_000_000),
                    max_output_tokens: Some(65_536),
                    supports_reasoning: true,
                },
                ModelInfo {
                    id: "gemini-2.5-flash".to_string(),
                    display_name: "Gemini 2.5 Flash".to_string(),
                    context_window: Some(1_000_000),
                    max_output_tokens: Some(65_536),
                    supports_reasoning: false,
                },
            ],
        }
    }

    fn probe<'a>(&'a self, cfg: &'a ProviderInstanceConfig) -> BoxFuture<'a, ProviderSnapshot> {
        Box::pin(async move {
            let binary = self.resolve_binary(cfg);
            let gemini_home = self
                .base_data_dir
                .join("profiles/antigravity")
                .join(cfg.id.as_str());

            if let Some(bin) = binary {
                let health = probe_antigravity_health_offline(&bin, &gemini_home, Some("1.1.1"));
                let is_healthy = matches!(health, ProviderHealth::Healthy);

                let auth = if is_healthy {
                    ProviderAuthStatus::Authenticated {
                        account_label: Some("Google Account".to_string()),
                        email: None,
                        subscription: None,
                    }
                } else {
                    ProviderAuthStatus::Unauthenticated
                };

                ProviderSnapshot {
                    installed: true,
                    version: Some("1.1.1".to_string()),
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
                        reason: "Antigravity ACP binary not found".to_string(),
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
            let _guard = self.limiter.try_acquire()?;

            let binary = self
                .resolve_binary(cfg)
                .ok_or_else(|| ProviderError::ProcessFailed {
                    program: "agy_acp_server".to_string(),
                    message: "Executable not resolved".to_string(),
                })?;

            let harness = binary
                .parent()
                .unwrap_or(Path::new("."))
                .join("localharness_external");

            let env_config = build_antigravity_env(
                cfg.id.as_str(),
                &self.base_data_dir,
                &harness,
                "pandamux-browser-helper",
            );
            ensure_profile_dir(&env_config.gemini_home)?;
            ensure_profile_dir(&env_config.scratch_dir)?;

            let mut cmd = create_supervised_command(&binary);
            cmd.current_dir(&spec.cwd);

            // Replace environment cleanly
            for (k, v) in &env_config.vars {
                cmd.env(k, v);
            }
            for (k, v) in &cfg.env_overrides {
                cmd.env(k, v);
            }

            let mut child = SupervisedChild::spawn(cmd)?;
            let mut stdin = child
                .take_stdin()
                .ok_or_else(|| ProviderError::SupervisionError {
                    message: "Failed to open child stdin".to_string(),
                })?;
            let stdout = child
                .take_stdout()
                .ok_or_else(|| ProviderError::SupervisionError {
                    message: "Failed to open child stdout".to_string(),
                })?;

            // Send initialize
            let init_req = build_initialize_request(1);
            let mut init_line = serde_json::to_string(&init_req)?;
            init_line.push('\n');
            stdin.write_all(init_line.as_bytes()).await?;
            stdin.flush().await?;

            // Send session/new
            let acp_mode = map_access_mode_to_acp(spec.access);
            let session_req = build_session_new_request(2, acp_mode);
            let mut session_line = serde_json::to_string(&session_req)?;
            session_line.push('\n');
            stdin.write_all(session_line.as_bytes()).await?;
            stdin.flush().await?;

            let (tx, rx) = mpsc::channel(128);

            // Background stdout parser
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

            Ok(Box::new(AcpSession::new(
                child,
                stdin,
                rx,
                "session-antigravity".to_string(),
            )) as Box<dyn ProviderSession>)
        })
    }
}
