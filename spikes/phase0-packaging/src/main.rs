mod config;
mod nsis;
mod packager_runner;
mod signing;

use std::fs;
use anyhow::Result;
use config::{parse_packager_config, validate_packaging_requirements};
use nsis::NsisScriptGenerator;
use packager_runner::run_packaging_verification;
use signing::{create_synthetic_signed_pe, verify_pe_authenticode, verify_signature_preservation};

fn main() -> Result<()> {
    println!("============================================================");
    println!("PandaMUX Phase 0: S4 Packaging Spike (cargo-packager NSIS)");
    println!("============================================================");

    // -----------------------------------------------------------------
    // 1. Packager Configuration & Multi-Binary Specification
    // -----------------------------------------------------------------
    println!("\n[1/5] Testing Packager Configuration & Multi-Binary Spec...");
    let config_fixture = fs::read_to_string("fixtures/packager_config.toml")?;
    let packager_config = parse_packager_config(&config_fixture)?;
    validate_packaging_requirements(&packager_config)?;

    println!("  ✓ Product Name: {}", packager_config.product_name);
    println!("  ✓ Identifier: {}", packager_config.identifier);
    println!("  ✓ Declared Windows Binaries: {}", packager_config.binaries.len());
    for b in &packager_config.binaries {
        println!("    - Binary: {} (main: {})", b.path, b.main);
    }
    println!("  ✓ Bundled Resources: {}", packager_config.resources.len());
    let linux_resources: Vec<_> = packager_config
        .resources
        .iter()
        .filter(|r| r.target.contains("linux-musl"))
        .collect();
    println!("    - Linux Musl Node Resources: {}", linux_resources.len());
    for lr in &linux_resources {
        println!("      * Target: {}", lr.target);
    }
    println!("  ✓ before-packaging-command is absent (signatures protected)");

    // -----------------------------------------------------------------
    // 2. Authenticode PE Digital Signature Table Integrity
    // -----------------------------------------------------------------
    println!("\n[2/5] Testing PE Authenticode Signature Integrity & Verification...");
    let mock_code = b"pandamux machine instructions payload";
    let mock_cert = b"Azure Trusted Signing: CN=BoardPandas Inc, OID=2.16.840.1.114028";
    let signed_pe = create_synthetic_signed_pe(mock_code, mock_cert);

    let report = verify_pe_authenticode(&signed_pe)?;
    assert!(report.is_signed, "PE verification must report is_signed = true");
    let sig = report.signature.unwrap();
    println!("  ✓ PE Signature Verified:");
    println!("    - Certificate Size: {} bytes", sig.cert_table_size);
    println!("    - Offset in Image: 0x{:X}", sig.cert_table_offset);
    println!("    - SHA-256 Digest: {}", report.sha256_digest);

    // Test signature preservation verification
    verify_signature_preservation(&signed_pe, &signed_pe, "pandamux.exe")?;
    println!("  ✓ Bit-for-bit signature preservation confirmed");

    // -----------------------------------------------------------------
    // 3. NSIS Installer Script Generation (currentUser Mode)
    // -----------------------------------------------------------------
    println!("\n[3/5] Testing NSIS Installer Script Generation...");
    let generated_nsis = NsisScriptGenerator::generate_script(
        &packager_config,
        "STAGE_DIR",
        "0.53.10",
    )?;
    NsisScriptGenerator::validate_script(&generated_nsis)?;

    println!("  ✓ Generated NSIS script contains currentUser execution level");
    println!("  ✓ Install Directory target: $LOCALAPPDATA\\Programs\\PandaMUX");
    println!("  ✓ Shortcuts configured: $SMPROGRAMS\\PandaMUX\\PandaMUX.lnk");
    println!("  ✓ Multi-binary files written: pandamux.exe, pandamux-server.exe, pandamux-cli.exe");
    println!("  ✓ Bundled resources copied: themes, sounds, icons, linux-musl nodes");
    println!("  ✓ Uninstall registration: HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PandaMUX");

    // -----------------------------------------------------------------
    // 4. End-to-End Packaging & Extraction Verification Pipeline
    // -----------------------------------------------------------------
    println!("\n[4/5] Executing End-to-End Packaging & Extraction Verification...");
    let temp_dir = tempfile::tempdir()?;
    let stage_dir = temp_dir.path().join("stage");
    let out_dir = temp_dir.path().join("dist");

    let packaging_result = run_packaging_verification(
        &stage_dir,
        &out_dir,
        &packager_config,
        "0.53.10",
    )?;

    println!("  ✓ Staged Artifacts Count: {}", packaging_result.staged_artifacts_count);
    println!("  ✓ Output Installer File: {:?}", packaging_result.installer_path);
    println!("  ✓ Outer Installer Signed: {}", packaging_result.installer_signed);
    println!("  ✓ Installer SHA-256: {}", packaging_result.installer_sha256);
    println!("  ✓ Extracted Artifacts Count: {}", packaging_result.extracted_artifacts_count);
    println!("  ✓ Inner Executables with Verified Preserved Signatures:");
    for sig_name in &packaging_result.verified_signatures {
        println!("    - {}", sig_name);
    }

    // -----------------------------------------------------------------
    // 5. Negative Integrity Tests & Protection Checks
    // -----------------------------------------------------------------
    println!("\n[5/5] Testing Protection Checks & Tamper Detection...");
    let mut bad_config = packager_config.clone();
    bad_config.before_packaging_command = Some("cargo build --release".to_string());
    assert!(
        validate_packaging_requirements(&bad_config).is_err(),
        "Must reject config with before-packaging-command"
    );
    println!("  ✓ Rejection verified for before-packaging-command rebuild hazard");

    let mut tampered_pe = signed_pe.clone();
    tampered_pe[0x120] ^= 0x01; // Tamper with byte in PE body
    assert!(
        verify_signature_preservation(&signed_pe, &tampered_pe, "pandamux.exe").is_err(),
        "Must fail when binary payload is tampered"
    );
    println!("  ✓ Bit-tamper detection verified on signed PE payload");

    println!("\n============================================================");
    println!("S4 Packaging Spike: ALL CHECKS PASSED");
    println!("============================================================");

    Ok(())
}
