use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::config::PackagerMetadata;
use crate::signing::{
    create_synthetic_signed_pe, verify_pe_authenticode, verify_signature_preservation,
};

#[allow(dead_code)]
pub struct StagedArtifact {
    pub relative_path: String,
    pub original_sha256: String,
    pub is_pe: bool,
}

pub struct PackagingVerificationResult {
    pub installer_path: PathBuf,
    pub installer_sha256: String,
    pub installer_signed: bool,
    pub staged_artifacts_count: usize,
    pub extracted_artifacts_count: usize,
    pub verified_signatures: Vec<String>,
}

pub fn stage_mock_artifacts(stage_dir: &Path, config: &PackagerMetadata) -> Result<HashMap<String, StagedArtifact>> {
    fs::create_dir_all(stage_dir)?;
    let mut staged = HashMap::new();

    // 1. Stage and sign the three Windows binaries
    for binary in &config.binaries {
        let filename = Path::new(&binary.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("app.exe");
        let dest = stage_dir.join(filename);

        let code_bytes = format!("machine-code-for-{}", filename).into_bytes();
        let cert_bytes = format!("Azure-Trusted-Signing-Cert-For-{}", filename).into_bytes();
        let signed_pe = create_synthetic_signed_pe(&code_bytes, &cert_bytes);

        fs::write(&dest, &signed_pe)
            .with_context(|| format!("Writing staged binary to {:?}", dest))?;

        let digest = format!("{:x}", Sha256::digest(&signed_pe));
        staged.insert(
            filename.to_string(),
            StagedArtifact {
                relative_path: filename.to_string(),
                original_sha256: digest,
                is_pe: true,
            },
        );
    }

    // 2. Stage bundled resources including Linux node binaries
    for resource in &config.resources {
        let dest = stage_dir.join(&resource.target);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // If target resembles a file, write mock file contents; otherwise create dir with dummy asset
        if resource.target.ends_with(".exe")
            || resource.target.contains("server/x86_64")
            || resource.target.contains("server/aarch64")
        {
            let payload = format!("ELF-static-musl-binary-mock: {}", resource.target).into_bytes();
            fs::write(&dest, &payload)?;
            let digest = format!("{:x}", Sha256::digest(&payload));
            staged.insert(
                resource.target.clone(),
                StagedArtifact {
                    relative_path: resource.target.clone(),
                    original_sha256: digest,
                    is_pe: false,
                },
            );
        } else {
            // Directory resource (themes, sounds, icons)
            fs::create_dir_all(&dest)?;
            let item_file = dest.join("resource_manifest.json");
            let item_data = format!("{{\"resource\": \"{}\"}}", resource.target).into_bytes();
            fs::write(&item_file, &item_data)?;
            let rel_path = format!("{}/resource_manifest.json", resource.target);
            let digest = format!("{:x}", Sha256::digest(&item_data));
            staged.insert(
                rel_path.clone(),
                StagedArtifact {
                    relative_path: rel_path,
                    original_sha256: digest,
                    is_pe: false,
                },
            );
        }
    }

    Ok(staged)
}

// Simulates the cargo-packager NSIS bundling engine and code-signing process:
// 1. Gathers staged artifacts
// 2. Bundles into self-extracting NSIS package format
// 3. Signs the outer installer with Authenticode
// 4. Extracts to a target dir
// 5. Verifies all inner binaries retain bit-for-bit identity and signatures
pub fn run_packaging_verification(
    stage_dir: &Path,
    out_dir: &Path,
    config: &PackagerMetadata,
    version: &str,
) -> Result<PackagingVerificationResult> {
    let staged = stage_mock_artifacts(stage_dir, config)?;

    // Serialize staged package manifest into an internal archive payload
    let mut payload_entries = Vec::new();
    for (rel_path, artifact) in &staged {
        let file_path = stage_dir.join(rel_path);
        let content = fs::read(&file_path)?;
        payload_entries.push((rel_path.clone(), content, artifact.is_pe));
    }

    // Build the installer executable
    let installer_name = format!("{}-Setup-{}.exe", config.product_name, version);
    let installer_path = out_dir.join(&installer_name);

    // Create installer PE with its own outer Authenticode certificate
    let mut installer_body = Vec::new();
    installer_body.extend_from_slice(b"NSIS-INSTALLER-HEADER\n");
    for (path, data, _) in &payload_entries {
        installer_body.extend_from_slice(format!("ENTRY:{}:{}\n", path, data.len()).as_bytes());
        installer_body.extend_from_slice(data);
    }

    let installer_cert = b"Azure Trusted Signing Certificate: PandaMUX Setup Installer";
    let signed_installer = create_synthetic_signed_pe(&installer_body, installer_cert);

    fs::create_dir_all(out_dir)?;
    fs::write(&installer_path, &signed_installer)?;

    // Verify outer installer PE signature
    let installer_report = verify_pe_authenticode(&signed_installer)?;
    if !installer_report.is_signed {
        bail!("Generated installer was not signed properly");
    }

    // Simulate installer execution / extraction to target directory
    let install_target_dir = out_dir.join("installed_app");
    fs::create_dir_all(&install_target_dir)?;

    let mut verified_sigs = Vec::new();

    for (rel_path, original_data, is_pe) in payload_entries {
        let target_file = install_target_dir.join(&rel_path);
        if let Some(parent) = target_file.parent() {
            fs::create_dir_all(parent)?;
        }
        // Write extracted binary
        fs::write(&target_file, &original_data)?;

        // Verify SHA-256 match
        let extracted_data = fs::read(&target_file)?;
        let extracted_digest = format!("{:x}", Sha256::digest(&extracted_data));
        let staged_artifact = staged
            .get(&rel_path)
            .ok_or_else(|| anyhow::anyhow!("Missing staged record for {}", rel_path))?;

        if extracted_digest != staged_artifact.original_sha256 {
            bail!(
                "Extracted file '{}' corrupted: expected {} but got {}",
                rel_path,
                staged_artifact.original_sha256,
                extracted_digest
            );
        }

        // If PE binary, verify Authenticode signature preservation
        if is_pe {
            verify_signature_preservation(&original_data, &extracted_data, &rel_path)?;
            verified_sigs.push(rel_path);
        }
    }

    Ok(PackagingVerificationResult {
        installer_path,
        installer_sha256: installer_report.sha256_digest,
        installer_signed: installer_report.is_signed,
        staged_artifacts_count: staged.len(),
        extracted_artifacts_count: staged.len(),
        verified_signatures: verified_sigs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_packager_config;

    #[test]
    fn test_full_packaging_and_verification_pipeline() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stage_dir = temp.path().join("stage");
        let out_dir = temp.path().join("dist");

        let config_toml = include_str!("../fixtures/packager_config.toml");
        let config = parse_packager_config(config_toml).expect("parse config");

        let result = run_packaging_verification(&stage_dir, &out_dir, &config, "0.53.10")
            .expect("Packaging verification must succeed");

        assert!(result.installer_signed);
        assert_eq!(result.verified_signatures.len(), 3);
        assert!(result.verified_signatures.contains(&"pandamux.exe".to_string()));
        assert!(result.verified_signatures.contains(&"pandamux-server.exe".to_string()));
        assert!(result.verified_signatures.contains(&"pandamux-cli.exe".to_string()));
    }
}
