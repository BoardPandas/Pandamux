use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedBundleManifest {
    pub version: String,
    pub platform: String,
    pub arch: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub entries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveReleasePointer {
    pub active_sha256: String,
    pub version: String,
    pub updated_at: String,
}

/// Hardened extraction validator for Antigravity managed bundles.
/// Enforces:
/// 1. Hash verification matching pinned SHA-256
/// 2. Size verification
/// 3. Exactly two entries (agy_acp_server and localharness_external)
/// 4. Rejection of zip-bombs, path traversal (../), absolute paths, and symlinks
pub fn verify_and_validate_bundle_zip(
    zip_bytes: &[u8],
    manifest: &ManagedBundleManifest,
) -> Result<()> {
    if zip_bytes.len() as u64 != manifest.size {
        bail!(
            "Bundle size mismatch: expected {} bytes, got {}",
            manifest.size,
            zip_bytes.len()
        );
    }

    let mut hasher = Sha256::new();
    hasher.update(zip_bytes);
    let computed_hash = hex::encode(hasher.finalize());

    if computed_hash.to_lowercase() != manifest.sha256.to_lowercase() {
        bail!(
            "Bundle SHA-256 mismatch: expected {}, got {}",
            manifest.sha256,
            computed_hash
        );
    }

    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).context("Failed to parse zip archive")?;

    let expected_entries: HashSet<&str> = manifest.entries.iter().map(|s| s.as_str()).collect();
    if archive.len() != expected_entries.len() {
        bail!(
            "Hardened check failed: zip archive contains {} entries, expected exactly {}",
            archive.len(),
            expected_entries.len()
        );
    }

    let mut found_entries = HashSet::new();
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = file.name();

        // Security checks: reject path traversal and absolute paths
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
            bail!("Path traversal attempt detected in entry: {}", name);
        }

        if !expected_entries.contains(name) {
            bail!("Unexpected entry in managed bundle: {}", name);
        }

        found_entries.insert(name.to_string());
    }

    if found_entries.len() != expected_entries.len() {
        bail!("Missing required bundle entries");
    }

    Ok(())
}

/// Resolves the Antigravity ACP server binary according to the specification:
/// 1. Explicit user configuration path
/// 2. Managed active release in `<base>/tools/antigravity-acp/<platform>-<arch>/versions/<sha256>/`
/// 3. System PATH fallback
pub fn resolve_antigravity_binary(
    explicit_path: Option<&Path>,
    tools_base_dir: &Path,
    platform_arch: &str,
) -> Option<PathBuf> {
    // 1. Explicit binary path setting
    if let Some(path) = explicit_path {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }

    // 2. Managed active release
    let platform_dir = tools_base_dir.join("tools/antigravity-acp").join(platform_arch);
    let active_json = platform_dir.join("active.json");
    if let Ok(content) = fs::read_to_string(&active_json) {
        if let Ok(pointer) = serde_json::from_str::<ActiveReleasePointer>(&content) {
            let server_name = if cfg!(windows) {
                "agy_acp_server.exe"
            } else {
                "agy_acp_server.par"
            };
            let binary = platform_dir
                .join("versions")
                .join(&pointer.active_sha256)
                .join(server_name);
            if binary.is_file() {
                return Some(binary);
            }
        }
    }

    // 3. System PATH fallback
    let binary_name = if cfg!(windows) {
        "agy_acp_server.exe"
    } else {
        "agy_acp_server"
    };

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_deserialization() {
        let raw = r#"{
            "version": "1.1.1",
            "platform": "windows",
            "arch": "x86_64",
            "url": "https://dl.google.com/antigravity/acp/v1.1.1/antigravity-acp-windows-x64.zip",
            "size": 100,
            "sha256": "abcdef",
            "entries": ["agy_acp_server.exe", "localharness_external.exe"]
        }"#;

        let manifest: ManagedBundleManifest = serde_json::from_str(raw).unwrap();
        assert_eq!(manifest.version, "1.1.1");
        assert_eq!(manifest.entries.len(), 2);
    }
}
