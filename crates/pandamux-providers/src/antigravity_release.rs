//! Managed Google Antigravity ACP release installation.
//!
//! Ported to Rust from T3 Code's `AntigravityInstallation.ts` and
//! `antigravityRelease.ts`. The source material is covered by the MIT notice
//! in the repository root `THIRD_PARTY_NOTICES.md`.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pandamux_protocol::antigravity_rpc::{
    AntigravityInstallResult, ManagedBundleFile, ManagedBundleManifest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zip::CompressionMethod;

use crate::antigravity::shared_antigravity_process_limiter;
use crate::antigravity_auth::validate_antigravity_runtime;
use crate::error::ProviderError;
use crate::shim_resolver::resolve_command_shim;

const RELEASE_RECORD: &str = ".install-complete.json";
const RECORD_MAX_BYTES: u64 = 8 * 1024;
const FREE_SPACE_MARGIN: u64 = 256 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const HASH_BUFFER_BYTES: usize = 1024 * 1024;
const STALE_CACHE_TEMP_AGE: Duration = Duration::from_secs(2 * 60 * 60);

static INSTALL_GATE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn install_error(operation: &str, code: &str, message: impl Into<String>) -> ProviderError {
    ProviderError::InstallationError {
        operation: operation.to_string(),
        code: code.to_string(),
        message: message.into(),
    }
}

fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Returns the official pinned Antigravity ACP release for a host target.
pub fn pinned_bundle_manifest(platform: &str, arch: &str) -> Option<ManagedBundleManifest> {
    let key = match (platform, arch) {
        ("linux", "x64" | "x86_64") => "linux-x64",
        ("linux", "arm64" | "aarch64") => "linux-arm64",
        ("windows" | "win32", "x64" | "x86_64") => "windows-x64",
        ("windows" | "win32", "arm64" | "aarch64") => "windows-arm64",
        ("darwin" | "macos", "arm64" | "aarch64") => "darwin-arm64",
        _ => return None,
    };

    let (platform, arch, url, sha256, size, executable, executable_size, harness, harness_size) =
        match key {
            "linux-x64" => (
                "linux",
                "x64",
                "https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-agy_acp_server_1.1.1-linux-x86_64.zip",
                "38f62d01b32deb0907b3d39a71ec301fd36369f6ffd1cf262d4af385177f79df",
                681_969_407,
                "agy_acp_server.par",
                1_880_360_328,
                "localharness_external",
                128_966_920,
            ),
            "linux-arm64" => (
                "linux",
                "arm64",
                "https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-agy_acp_server_1.1.1-linux-arm64.zip",
                "ed69e64b308fcb123ab54bf3277bf9cb0d651064f885ea5aab0ff520c7175398",
                656_572_786,
                "agy_acp_server.par",
                1_862_073_131,
                "localharness_external",
                122_158_704,
            ),
            "windows-x64" => (
                "windows",
                "x64",
                "https://dl.google.com/agy-extensions/releases/windows/agy-acp-server-agy_acp_server_1.1.1-windows-x86_64.zip",
                "47cb50eef14f0a4655d78cfcfda869bcea7aaee5f9787e936bc2935ea612c3b8",
                468_238_392,
                "agy_acp_server.exe",
                430_801_616,
                "localharness_external.exe",
                130_971_800,
            ),
            "windows-arm64" => (
                "windows",
                "arm64",
                "https://dl.google.com/agy-extensions/releases/windows/agy-acp-server-agy_acp_server_1.1.1-windows-arm64.zip",
                "35f4b1f47ba6a3fea7b0a3e30010df5ea73a64b4f0e7cf991cddc673ddfbcafc",
                468_521_191,
                "agy_acp_server.exe",
                435_075_816,
                "localharness_external.exe",
                122_455_704,
            ),
            "darwin-arm64" => (
                "darwin",
                "arm64",
                "https://dl.google.com/agy-extensions/releases/macos/agy-acp-server-agy_acp_server_1.1.1-darwin-arm64.zip",
                "fdfa915652cdb7ba8085cc8fffed072cbe009251aa2c951aabdda07a8c28a189",
                316_014_828,
                "agy_acp_server.par",
                802_163_856,
                "localharness_external",
                116_766_704,
            ),
            _ => return None,
        };

    Some(ManagedBundleManifest {
        version: "agy_acp_server_1.1.1".to_string(),
        platform: platform.to_string(),
        arch: arch.to_string(),
        url: url.to_string(),
        size,
        sha256: sha256.to_string(),
        executable: ManagedBundleFile {
            name: executable.to_string(),
            size: executable_size,
        },
        harness: ManagedBundleFile {
            name: harness.to_string(),
            size: harness_size,
        },
    })
}

/// Returns the pinned release for this compile target.
pub fn current_bundle_manifest() -> Option<ManagedBundleManifest> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return pinned_bundle_manifest("linux", "x64");
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return pinned_bundle_manifest("linux", "arm64");
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return pinned_bundle_manifest("windows", "x64");
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return pinned_bundle_manifest("windows", "arm64");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return pinned_bundle_manifest("darwin", "arm64");
    #[allow(unreachable_code)]
    None
}

pub fn platform_arch(manifest: &ManagedBundleManifest) -> String {
    format!("{}-{}", manifest.platform, manifest.arch)
}

pub fn required_install_bytes(manifest: &ManagedBundleManifest) -> u64 {
    manifest
        .size
        .saturating_add(manifest.executable.size)
        .saturating_add(manifest.harness.size)
        .saturating_add(FREE_SPACE_MARGIN)
}

/// Rejects caller-controlled release metadata. Only the compile-time pin may install.
pub fn validate_pinned_manifest(manifest: &ManagedBundleManifest) -> Result<(), ProviderError> {
    let expected = pinned_bundle_manifest(&manifest.platform, &manifest.arch).ok_or_else(|| {
        install_error(
            "manifest",
            "antigravity_target_unsupported",
            format!(
                "Google does not publish the pinned Antigravity runtime for {}-{}",
                manifest.platform, manifest.arch
            ),
        )
    })?;
    if expected != *manifest {
        return Err(install_error(
            "manifest",
            "antigravity_manifest_not_pinned",
            "The requested Antigravity manifest does not exactly match PandaMUX's pinned release",
        ));
    }
    Ok(())
}

/// Queries caller-available free space on the filesystem containing `path`.
pub fn get_available_disk_bytes(path: &Path) -> Result<u64, ProviderError> {
    let mut candidate = path;
    while !candidate.exists() {
        candidate = candidate.parent().ok_or_else(|| {
            install_error(
                "preflight",
                "antigravity_disk_path_missing",
                format!("No existing parent for {}", path.display()),
            )
        })?;
    }
    fs4::available_space(candidate).map_err(|error| {
        install_error(
            "preflight",
            "antigravity_disk_query_failed",
            format!(
                "Could not query free space at {}: {error}",
                candidate.display()
            ),
        )
    })
}

pub fn preflight_disk_space(path: &Path, min_bytes: u64) -> Result<u64, ProviderError> {
    let available = get_available_disk_bytes(path)?;
    if available < min_bytes {
        return Err(install_error(
            "preflight",
            "antigravity_disk_space_insufficient",
            format!(
                "Antigravity requires {min_bytes} bytes of free space, but {available} bytes are available"
            ),
        ));
    }
    Ok(available)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ActiveReleasePointer {
    #[serde(alias = "activeSha256")]
    release_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct InstalledRelease {
    release_id: String,
    version: String,
    executable: ManagedBundleFile,
    harness: ManagedBundleFile,
    installed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAntigravityExecutable {
    pub executable_path: PathBuf,
    pub harness_path: PathBuf,
    pub source: &'static str,
    pub version: Option<String>,
    pub managed_version_directory: Option<PathBuf>,
}

fn managed_directory(tools_base_dir: &Path, manifest: &ManagedBundleManifest) -> PathBuf {
    tools_base_dir
        .join("tools/antigravity-acp")
        .join(platform_arch(manifest))
}

fn cache_directory(tools_base_dir: &Path) -> PathBuf {
    tools_base_dir.join("cache/antigravity")
}

fn sweep_stale_cache_temp_files(cache_dir: &Path) -> Result<(), ProviderError> {
    let now = SystemTime::now();
    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let known_temporary = (name.starts_with(".upload-") && name.ends_with(".zip"))
            || (name.starts_with('.') && name.ends_with(".tmp"));
        if !known_temporary {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_CACHE_TEMP_AGE);
        if stale {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn private_directory(path: &Path) -> Result<(), ProviderError> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(install_error(
            "filesystem",
            "antigravity_unsafe_directory",
            format!("Refusing unsafe Antigravity directory {}", path.display()),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn validate_regular_file(path: &Path, expected_size: u64) -> Result<(), ProviderError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        install_error(
            "resolve",
            "antigravity_runtime_incomplete",
            format!("Missing {}: {error}", path.display()),
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != expected_size {
        return Err(install_error(
            "resolve",
            "antigravity_runtime_incomplete",
            format!("Managed runtime file is invalid: {}", path.display()),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(install_error(
                "resolve",
                "antigravity_runtime_not_executable",
                format!("Managed runtime file is not executable: {}", path.display()),
            ));
        }
    }
    Ok(())
}

fn read_json_record<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ProviderError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        install_error(
            "resolve",
            "antigravity_install_record_missing",
            format!("Could not read {}: {error}", path.display()),
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > RECORD_MAX_BYTES
    {
        return Err(install_error(
            "resolve",
            "antigravity_install_record_invalid",
            format!("Managed runtime record is invalid: {}", path.display()),
        ));
    }
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(ProviderError::from)
}

fn completed_release(
    managed_dir: &Path,
    manifest: &ManagedBundleManifest,
) -> Result<ResolvedAntigravityExecutable, ProviderError> {
    let directory = managed_dir.join("versions").join(&manifest.sha256);
    let record: InstalledRelease = read_json_record(&directory.join(RELEASE_RECORD))?;
    if record.release_id != manifest.sha256
        || record.version != manifest.version
        || record.executable != manifest.executable
        || record.harness != manifest.harness
    {
        return Err(install_error(
            "resolve",
            "antigravity_install_record_mismatch",
            "Managed Antigravity runtime record does not match the pinned release",
        ));
    }

    let executable_path = directory.join(&manifest.executable.name);
    let harness_path = directory.join(&manifest.harness.name);
    validate_regular_file(&executable_path, manifest.executable.size)?;
    validate_regular_file(&harness_path, manifest.harness.size)?;
    Ok(ResolvedAntigravityExecutable {
        executable_path,
        harness_path,
        source: "managed",
        version: Some(record.version),
        managed_version_directory: Some(directory),
    })
}

fn external_release(
    executable_path: &Path,
    manifest: &ManagedBundleManifest,
    source: &'static str,
) -> Option<ResolvedAntigravityExecutable> {
    let metadata = fs::symlink_metadata(executable_path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return None;
    }
    let executable_path = fs::canonicalize(executable_path).ok()?;
    let harness_path = executable_path.parent()?.join(&manifest.harness.name);
    let harness_metadata = fs::symlink_metadata(&harness_path).ok()?;
    if !harness_metadata.is_file() || harness_metadata.file_type().is_symlink() {
        return None;
    }
    Some(ResolvedAntigravityExecutable {
        executable_path,
        harness_path,
        source,
        version: None,
        managed_version_directory: None,
    })
}

/// Resolves the executable and its required sibling harness.
pub fn resolve_antigravity_executable(
    explicit_path: Option<&Path>,
    tools_base_dir: &Path,
    platform_arch: &str,
) -> Option<ResolvedAntigravityExecutable> {
    let (platform, arch) = platform_arch.split_once('-')?;
    let manifest = pinned_bundle_manifest(platform, arch)?;

    if let Some(path) = explicit_path {
        return external_release(path, &manifest, "override");
    }

    let managed_dir = managed_directory(tools_base_dir, &manifest);
    let active_path = managed_dir.join("active.json");
    if let Ok(pointer) = read_json_record::<ActiveReleasePointer>(&active_path)
        && pointer.release_id == manifest.sha256
        && let Ok(resolved) = completed_release(&managed_dir, &manifest)
    {
        return Some(resolved);
    }

    if let Some(resolved) = resolve_command_shim(&manifest.executable.name)
        && let Some(external) = external_release(&resolved.program, &manifest, "path")
    {
        return Some(external);
    }
    None
}

pub fn resolve_antigravity_binary(
    explicit_path: Option<&Path>,
    tools_base_dir: &Path,
    platform_arch: &str,
) -> Option<PathBuf> {
    resolve_antigravity_executable(explicit_path, tools_base_dir, platform_arch)
        .map(|resolved| resolved.executable_path)
}

fn open_private_file(path: &Path) -> Result<tokio::fs::File, ProviderError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    Ok(tokio::fs::File::from_std(file))
}

async fn download_archive_to(
    destination: &Path,
    manifest: &ManagedBundleManifest,
) -> Result<(), ProviderError> {
    validate_pinned_manifest(manifest)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("PandaMUX/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| {
            install_error(
                "download",
                "antigravity_node_download_failed",
                format!("Could not construct the download client: {error}"),
            )
        })?;

    let download = async {
        let mut response = client
            .get(&manifest.url)
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|error| {
                install_error(
                    "download",
                    "antigravity_node_download_failed",
                    format!("Could not download the pinned Antigravity runtime: {error}"),
                )
            })?;
        if !response.status().is_success() {
            return Err(install_error(
                "download",
                "antigravity_node_download_failed",
                format!(
                    "Pinned Antigravity download returned HTTP {}",
                    response.status()
                ),
            ));
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .is_some_and(|encoding| encoding.as_bytes() != b"identity")
        {
            return Err(install_error(
                "download",
                "antigravity_download_encoding_unexpected",
                "Pinned Antigravity download used an unexpected content encoding",
            ));
        }
        if let Some(length) = response.content_length()
            && length != manifest.size
        {
            return Err(install_error(
                "download",
                "antigravity_download_size_mismatch",
                format!(
                    "Pinned Antigravity archive is {} bytes, expected {} bytes",
                    length, manifest.size
                ),
            ));
        }

        let mut file = open_private_file(destination)?;
        let mut bytes_written = 0_u64;
        let mut hasher = Sha256::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            install_error(
                "download",
                "antigravity_node_download_failed",
                format!("Pinned Antigravity download failed: {error}"),
            )
        })? {
            bytes_written = bytes_written.saturating_add(chunk.len() as u64);
            if bytes_written > manifest.size {
                return Err(install_error(
                    "download",
                    "antigravity_download_size_mismatch",
                    "Pinned Antigravity archive exceeded its expected size",
                ));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        file.sync_all().await?;
        let digest = hex::encode(hasher.finalize());
        if bytes_written != manifest.size || digest != manifest.sha256 {
            return Err(install_error(
                "download",
                "antigravity_download_integrity_failed",
                format!(
                    "Pinned Antigravity archive failed size or SHA-256 verification: got {bytes_written} bytes and {digest}"
                ),
            ));
        }
        Ok(())
    };

    match tokio::time::timeout(DOWNLOAD_TIMEOUT, download).await {
        Ok(result) => result,
        Err(_) => Err(install_error(
            "download",
            "antigravity_node_download_failed",
            "Pinned Antigravity download timed out after 45 minutes",
        )),
    }
}

/// Downloads and verifies the pin into a reusable Hub-side cache.
pub async fn cache_bundle_archive(
    tools_base_dir: &Path,
    manifest: &ManagedBundleManifest,
) -> Result<PathBuf, ProviderError> {
    validate_pinned_manifest(manifest)?;
    let cache_dir = cache_directory(tools_base_dir);
    private_directory(&cache_dir)?;
    sweep_stale_cache_temp_files(&cache_dir)?;
    let destination = cache_dir.join(format!("{}.zip", manifest.sha256));
    if destination.is_file() {
        verify_bundle_archive_file(manifest, &destination).await?;
        return Ok(destination);
    }
    preflight_disk_space(&cache_dir, manifest.size.saturating_add(FREE_SPACE_MARGIN))?;

    let temporary = cache_dir.join(format!(".{}.{}.tmp", manifest.sha256, uuid::Uuid::new_v4()));
    let result = download_archive_to(&temporary, manifest).await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error);
    }
    match tokio::fs::rename(&temporary, &destination).await {
        Ok(()) => {}
        Err(error) if destination.is_file() => {
            let _ = tokio::fs::remove_file(&temporary).await;
            verify_bundle_archive_file(manifest, &destination).await?;
            let _ = error;
        }
        Err(error) => return Err(ProviderError::Io(error)),
    }
    Ok(destination)
}

/// Creates a private, non-existent node-side path for a subsequent SFTP upload.
pub fn prepare_bundle_upload(
    tools_base_dir: &Path,
    manifest: &ManagedBundleManifest,
) -> Result<PathBuf, ProviderError> {
    validate_pinned_manifest(manifest)?;
    let cache_dir = cache_directory(tools_base_dir);
    private_directory(&cache_dir)?;
    sweep_stale_cache_temp_files(&cache_dir)?;
    Ok(cache_dir.join(format!(
        ".upload-{}-{}.zip",
        manifest.sha256,
        uuid::Uuid::new_v4()
    )))
}

fn validate_staged_path(
    tools_base_dir: &Path,
    staged_path: &Path,
) -> Result<PathBuf, ProviderError> {
    let cache_dir = cache_directory(tools_base_dir);
    let canonical_cache = fs::canonicalize(&cache_dir)?;
    let original_metadata = fs::symlink_metadata(staged_path)?;
    if original_metadata.file_type().is_symlink() || !original_metadata.is_file() {
        return Err(install_error(
            "upload",
            "antigravity_staged_archive_unsafe",
            "The staged Antigravity archive is not a regular file",
        ));
    }
    let canonical_file = fs::canonicalize(staged_path)?;
    let metadata = fs::symlink_metadata(&canonical_file)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || canonical_file.parent() != Some(canonical_cache.as_path())
    {
        return Err(install_error(
            "upload",
            "antigravity_staged_archive_unsafe",
            "The staged Antigravity archive is outside the private upload cache",
        ));
    }
    Ok(canonical_file)
}

/// Removes one failed upload after proving it is an ordinary file in the private cache.
pub fn discard_bundle_upload(
    tools_base_dir: &Path,
    staged_path: &Path,
) -> Result<(), ProviderError> {
    if !staged_path.exists() {
        return Ok(());
    }
    let canonical = validate_staged_path(tools_base_dir, staged_path)?;
    let name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !name.starts_with(".upload-") || !name.ends_with(".zip") {
        return Err(install_error(
            "upload",
            "antigravity_staged_archive_unsafe",
            "Refusing to remove an unrecognized Antigravity upload name",
        ));
    }
    fs::remove_file(canonical)?;
    Ok(())
}

/// Verifies a local archive without loading it into memory.
pub async fn verify_bundle_archive_file(
    manifest: &ManagedBundleManifest,
    path: &Path,
) -> Result<(), ProviderError> {
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != manifest.size {
        return Err(install_error(
            "verify",
            "antigravity_archive_size_mismatch",
            format!(
                "Antigravity archive size mismatch: expected {}, got {}",
                manifest.size,
                metadata.len()
            ),
        ));
    }
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > manifest.size {
            return Err(install_error(
                "verify",
                "antigravity_archive_size_mismatch",
                "Antigravity archive exceeded its pinned size",
            ));
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hex::encode(hasher.finalize());
    if total != manifest.size || digest != manifest.sha256 {
        return Err(install_error(
            "verify",
            "antigravity_archive_integrity_failed",
            format!(
                "Antigravity archive SHA-256 mismatch: expected {}, got {digest}",
                manifest.sha256
            ),
        ));
    }
    Ok(())
}

fn create_extracted_file(path: &Path) -> Result<File, ProviderError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700);
    }
    options.open(path).map_err(ProviderError::from)
}

fn extract_verified_archive_sync(
    manifest: &ManagedBundleManifest,
    archive_path: &Path,
    runtime_dir: &Path,
) -> Result<(), ProviderError> {
    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        install_error(
            "extract",
            "antigravity_archive_invalid",
            format!("Could not open the verified Antigravity archive: {error}"),
        )
    })?;
    if archive.len() != 2 {
        return Err(install_error(
            "extract",
            "antigravity_archive_entry_count",
            format!(
                "Antigravity archive contains {} entries, expected 2",
                archive.len()
            ),
        ));
    }

    let expected = [&manifest.executable, &manifest.harness];
    let mut seen = HashSet::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            install_error(
                "extract",
                "antigravity_archive_member_unreadable",
                format!("Could not read Antigravity archive member: {error}"),
            )
        })?;
        let expected_file = expected
            .iter()
            .copied()
            .find(|candidate| candidate.name == entry.name())
            .ok_or_else(|| {
                install_error(
                    "extract",
                    "antigravity_archive_member_unexpected",
                    format!("Unexpected Antigravity archive member: {}", entry.name()),
                )
            })?;
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            install_error(
                "extract",
                "antigravity_archive_member_unsafe",
                "Antigravity archive member has an unsafe path",
            )
        })?;
        let unsafe_mode = entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000);
        if !seen.insert(entry.name().to_string())
            || enclosed.components().count() != 1
            || entry.is_dir()
            || entry.encrypted()
            || unsafe_mode
            || !matches!(
                entry.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            )
            || entry.size() != expected_file.size
        {
            return Err(install_error(
                "extract",
                "antigravity_archive_member_unsafe",
                "Antigravity archive contains a duplicate, unsafe, encrypted, unsupported, or incorrectly sized member",
            ));
        }

        let output_path = runtime_dir.join(&expected_file.name);
        let mut output = create_extracted_file(&output_path)?;
        let mut buffer = [0_u8; 64 * 1024];
        let mut written = 0_u64;
        loop {
            let read = entry.read(&mut buffer).map_err(|error| {
                install_error(
                    "extract",
                    "antigravity_archive_member_unreadable",
                    format!("Could not extract {}: {error}", expected_file.name),
                )
            })?;
            if read == 0 {
                break;
            }
            written = written.saturating_add(read as u64);
            if written > expected_file.size {
                return Err(install_error(
                    "extract",
                    "antigravity_archive_member_oversized",
                    format!(
                        "Archive member {} exceeded its pinned size",
                        expected_file.name
                    ),
                ));
            }
            output.write_all(&buffer[..read])?;
        }
        if written != expected_file.size {
            return Err(install_error(
                "extract",
                "antigravity_archive_member_truncated",
                format!("Archive member {} was truncated", expected_file.name),
            ));
        }
        output.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&output_path, fs::Permissions::from_mode(0o755))?;
        }
    }

    if !seen.contains(&manifest.executable.name) || !seen.contains(&manifest.harness.name) {
        return Err(install_error(
            "extract",
            "antigravity_archive_member_missing",
            "Antigravity archive is missing its executable or harness",
        ));
    }
    Ok(())
}

pub(crate) async fn extract_verified_archive(
    manifest: &ManagedBundleManifest,
    archive_path: &Path,
    runtime_dir: &Path,
) -> Result<(), ProviderError> {
    let manifest = manifest.clone();
    let archive_path = archive_path.to_path_buf();
    let runtime_dir = runtime_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        extract_verified_archive_sync(&manifest, &archive_path, &runtime_dir)
    })
    .await
    .map_err(|error| {
        install_error(
            "extract",
            "antigravity_extract_task_failed",
            format!("Antigravity extraction task failed: {error}"),
        )
    })?
}

fn write_private_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ProviderError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_file_atomically(source: &Path, destination: &Path) -> Result<(), ProviderError> {
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file_atomically(source: &Path, destination: &Path) -> Result<(), ProviderError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(ProviderError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

fn activate_release(managed_dir: &Path, release_id: &str) -> Result<(), ProviderError> {
    private_directory(managed_dir)?;
    let temporary = managed_dir.join(format!(".active-{}.tmp", uuid::Uuid::new_v4()));
    write_private_json(
        &temporary,
        &ActiveReleasePointer {
            release_id: release_id.to_string(),
        },
    )?;
    let result = replace_file_atomically(&temporary, &managed_dir.join("active.json"));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

async fn install_inner(
    tools_base_dir: &Path,
    manifest: &ManagedBundleManifest,
    staged_archive_path: Option<&Path>,
    force: bool,
) -> Result<AntigravityInstallResult, ProviderError> {
    validate_pinned_manifest(manifest)?;
    let managed_dir = managed_directory(tools_base_dir, manifest);
    let versions_dir = managed_dir.join("versions");
    private_directory(&versions_dir)?;
    private_directory(&cache_directory(tools_base_dir))?;
    let free_disk = preflight_disk_space(&versions_dir, required_install_bytes(manifest))?;
    let destination = versions_dir.join(&manifest.sha256);

    if destination.exists() {
        let existing = completed_release(&managed_dir, manifest)?;
        if force {
            validate_antigravity_runtime(
                &existing.executable_path,
                &existing.harness_path,
                &manifest.version,
                tools_base_dir,
            )
            .await?;
        }
        activate_release(&managed_dir, &manifest.sha256)?;
        return Ok(AntigravityInstallResult {
            installed: true,
            version: manifest.version.clone(),
            binary_path: existing.executable_path.to_string_lossy().to_string(),
            source: "existing".to_string(),
            free_disk_bytes: free_disk,
        });
    }

    let staging = versions_dir.join(format!(".install-{}", uuid::Uuid::new_v4()));
    let runtime_dir = staging.join("runtime");
    private_directory(&staging)?;
    private_directory(&runtime_dir)?;

    let install_result = async {
        let (archive_path, source) = if let Some(staged) = staged_archive_path {
            (validate_staged_path(tools_base_dir, staged)?, "hub_push")
        } else {
            let path = staging.join("download.zip");
            download_archive_to(&path, manifest).await?;
            (path, "node_download")
        };

        verify_bundle_archive_file(manifest, &archive_path).await?;
        extract_verified_archive(manifest, &archive_path, &runtime_dir).await?;
        validate_antigravity_runtime(
            &runtime_dir.join(&manifest.executable.name),
            &runtime_dir.join(&manifest.harness.name),
            &manifest.version,
            tools_base_dir,
        )
        .await?;

        let record = InstalledRelease {
            release_id: manifest.sha256.clone(),
            version: manifest.version.clone(),
            executable: manifest.executable.clone(),
            harness: manifest.harness.clone(),
            installed_at: unix_timestamp_now(),
        };
        write_private_json(&runtime_dir.join(RELEASE_RECORD), &record)?;

        match fs::rename(&runtime_dir, &destination) {
            Ok(()) => {}
            Err(error) if destination.exists() => {
                completed_release(&managed_dir, manifest).map_err(|_| {
                    install_error(
                        "activate",
                        "antigravity_publish_race_failed",
                        format!("Another installer published an invalid release: {error}"),
                    )
                })?;
            }
            Err(error) => return Err(ProviderError::Io(error)),
        }
        activate_release(&managed_dir, &manifest.sha256)?;
        let installed = completed_release(&managed_dir, manifest)?;
        Ok(AntigravityInstallResult {
            installed: true,
            version: manifest.version.clone(),
            binary_path: installed.executable_path.to_string_lossy().to_string(),
            source: source.to_string(),
            free_disk_bytes: free_disk,
        })
    }
    .await;

    let _ = tokio::fs::remove_dir_all(&staging).await;
    install_result
}

/// Installs or activates the pinned managed Antigravity runtime.
pub async fn install_managed_bundle(
    tools_base_dir: &Path,
    manifest: &ManagedBundleManifest,
    staged_archive_path: Option<&Path>,
    force: bool,
) -> Result<AntigravityInstallResult, ProviderError> {
    let _process_guard = shared_antigravity_process_limiter().try_acquire()?;
    let gate = INSTALL_GATE.get_or_init(|| tokio::sync::Mutex::new(()));
    let _guard = gate.lock().await;
    let result = install_inner(tools_base_dir, manifest, staged_archive_path, force).await;
    if let Some(staged_path) = staged_archive_path {
        let _ = discard_bundle_upload(tools_base_dir, staged_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;

    fn tiny_manifest(archive: &[u8]) -> ManagedBundleManifest {
        let mut hasher = Sha256::new();
        hasher.update(archive);
        ManagedBundleManifest {
            version: "test".to_string(),
            platform: "linux".to_string(),
            arch: "x64".to_string(),
            url: "https://example.invalid/runtime.zip".to_string(),
            size: archive.len() as u64,
            sha256: hex::encode(hasher.finalize()),
            executable: ManagedBundleFile {
                name: "agy_acp_server.par".to_string(),
                size: 4,
            },
            harness: ManagedBundleFile {
                name: "localharness_external".to_string(),
                size: 7,
            },
        }
    }

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut output);
            for (name, bytes) in entries {
                writer
                    .start_file(*name, SimpleFileOptions::default())
                    .expect("start ZIP member");
                writer.write_all(bytes).expect("write ZIP member");
            }
            writer.finish().expect("finish ZIP");
        }
        output.into_inner()
    }

    #[tokio::test]
    async fn hardened_extraction_accepts_only_the_two_pinned_members() {
        let bytes = archive(&[
            ("agy_acp_server.par", b"exec"),
            ("localharness_external", b"harness"),
        ]);
        let manifest = tiny_manifest(&bytes);
        let temp = tempfile::tempdir().expect("tempdir");
        let archive_path = temp.path().join("runtime.zip");
        fs::write(&archive_path, bytes).expect("write archive");
        let runtime = temp.path().join("runtime");
        fs::create_dir(&runtime).expect("runtime dir");

        extract_verified_archive(&manifest, &archive_path, &runtime)
            .await
            .expect("extract safe archive");
        assert_eq!(
            fs::read(runtime.join("agy_acp_server.par")).unwrap(),
            b"exec"
        );
        assert_eq!(
            fs::read(runtime.join("localharness_external")).unwrap(),
            b"harness"
        );
    }

    #[tokio::test]
    async fn hardened_extraction_rejects_extra_or_traversing_members() {
        for entries in [
            vec![
                ("agy_acp_server.par", b"exec".as_slice()),
                ("localharness_external", b"harness".as_slice()),
                ("extra", b"bad".as_slice()),
            ],
            vec![
                ("../agy_acp_server.par", b"exec".as_slice()),
                ("localharness_external", b"harness".as_slice()),
            ],
        ] {
            let bytes = archive(&entries);
            let manifest = tiny_manifest(&bytes);
            let temp = tempfile::tempdir().expect("tempdir");
            let archive_path = temp.path().join("runtime.zip");
            fs::write(&archive_path, bytes).expect("write archive");
            let runtime = temp.path().join("runtime");
            fs::create_dir(&runtime).expect("runtime dir");
            assert!(
                extract_verified_archive(&manifest, &archive_path, &runtime)
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn pinned_release_matches_the_official_linux_asset() {
        let manifest = pinned_bundle_manifest("linux", "x86_64").expect("Linux x64 pin");
        assert_eq!(manifest.size, 681_969_407);
        assert_eq!(
            manifest.sha256,
            "38f62d01b32deb0907b3d39a71ec301fd36369f6ffd1cf262d4af385177f79df"
        );
        assert_eq!(manifest.executable.size, 1_880_360_328);
        assert_eq!(manifest.harness.size, 128_966_920);
    }
}
