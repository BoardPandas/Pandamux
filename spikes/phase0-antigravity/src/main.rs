mod auth;
mod env;
mod manifest;
mod protocol;
mod reliability;

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use auth::{extract_auth_url_from_output, validate_callback_query, validate_google_oauth_url};
use env::{build_antigravity_env, validate_sanitized_environment};
use manifest::{
    resolve_antigravity_binary, verify_and_validate_bundle_zip, ActiveReleasePointer,
    ManagedBundleManifest,
};
use protocol::{
    build_session_new_request, is_interaction_prompt, map_access_mode_to_antigravity,
    map_acp_error_code, validate_antigravity_initialize, AcpErrorOutcome, InitializeResult,
    PermissionRequestParams,
};
use reliability::{
    extract_normalized_command, probe_antigravity_health_offline, sanitize_tool_payload,
    sweep_orphan_temp_dirs, AntigravityConcurrencyLimiter, AntigravityHealth,
};
use sha2::{Digest, Sha256};

fn create_synthetic_zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut zip_writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        for (name, data) in entries {
            zip_writer.start_file(*name, options)?;
            zip_writer.write_all(data)?;
        }
        zip_writer.finish()?;
    }
    Ok(buf)
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("============================================================");
    println!("PandaMUX Phase 0: S5a Antigravity Integration Spike");
    println!("============================================================");

    // -----------------------------------------------------------------
    // 1. Managed Bundle Manifest & Hardened Extraction Verification
    // -----------------------------------------------------------------
    println!("\n[1/5] Testing Managed Bundle Manifest & Hardened Extraction...");
    let manifest_raw = fs::read_to_string("fixtures/managed_bundle_manifest.json")?;
    let mut manifest: ManagedBundleManifest = serde_json::from_str(&manifest_raw)?;

    // Create a synthetic valid bundle matching the two required entries
    let valid_entries = [
        ("agy_acp_server.exe", b"fake_acp_server_binary_data" as &[u8]),
        ("localharness_external.exe", b"fake_harness_binary_data" as &[u8]),
    ];
    let zip_bytes = create_synthetic_zip(&valid_entries)?;
    manifest.size = zip_bytes.len() as u64;

    let mut hasher = Sha256::new();
    hasher.update(&zip_bytes);
    manifest.sha256 = hex::encode(hasher.finalize());

    // Verify valid bundle
    assert!(verify_and_validate_bundle_zip(&zip_bytes, &manifest).is_ok());
    println!("  (ok) Pinned bundle verified: 2 entries, correct SHA-256 and size");

    // Negative tests: Hardened checks
    // a. Hash mismatch
    let mut bad_hash_manifest = manifest.clone();
    bad_hash_manifest.sha256 = "0000000000000000000000000000000000000000000000000000000000000000".into();
    assert!(verify_and_validate_bundle_zip(&zip_bytes, &bad_hash_manifest).is_err());
    println!("  (ok) Hardened rejection: hash mismatch rejected");

    // b. Extra entries (zip-bomb or trojan payload)
    let extra_entries = [
        ("agy_acp_server.exe", b"fake_acp" as &[u8]),
        ("localharness_external.exe", b"fake_harness" as &[u8]),
        ("trojan.exe", b"evil_payload" as &[u8]),
    ];
    let bad_zip_extra = create_synthetic_zip(&extra_entries)?;
    let mut extra_manifest = manifest.clone();
    extra_manifest.size = bad_zip_extra.len() as u64;
    let mut h2 = Sha256::new();
    h2.update(&bad_zip_extra);
    extra_manifest.sha256 = hex::encode(h2.finalize());
    assert!(verify_and_validate_bundle_zip(&bad_zip_extra, &extra_manifest).is_err());
    println!("  (ok) Hardened rejection: unexpected third entry rejected");

    // c. Path traversal attempt (../escape.exe)
    let traversal_entries = [
        ("agy_acp_server.exe", b"fake_acp" as &[u8]),
        ("../escape.exe", b"traversal_payload" as &[u8]),
    ];
    let bad_zip_trav = create_synthetic_zip(&traversal_entries)?;
    let mut trav_manifest = manifest.clone();
    trav_manifest.size = bad_zip_trav.len() as u64;
    let mut h3 = Sha256::new();
    h3.update(&bad_zip_trav);
    trav_manifest.sha256 = hex::encode(h3.finalize());
    assert!(verify_and_validate_bundle_zip(&bad_zip_trav, &trav_manifest).is_err());
    println!("  (ok) Hardened rejection: path traversal entry rejected");

    // Binary resolution order test
    let temp_tools_dir = std::env::temp_dir().join("pandamux_test_tools_res");
    let managed_platform_dir = temp_tools_dir
        .join("tools/antigravity-acp")
        .join("windows-x86_64");
    let version_dir = managed_platform_dir
        .join("versions")
        .join("test_sha_123");
    fs::create_dir_all(&version_dir)?;

    let fake_binary = version_dir.join(if cfg!(windows) {
        "agy_acp_server.exe"
    } else {
        "agy_acp_server.par"
    });
    fs::write(&fake_binary, b"binary")?;

    let active_pointer = ActiveReleasePointer {
        active_sha256: "test_sha_123".into(),
        version: "1.1.1".into(),
        updated_at: "2026-09-26T00:00:00Z".into(),
    };
    fs::write(
        managed_platform_dir.join("active.json"),
        serde_json::to_string(&active_pointer)?,
    )?;

    let resolved_managed = resolve_antigravity_binary(None, &temp_tools_dir, "windows-x86_64");
    assert_eq!(resolved_managed, Some(fake_binary.clone()));
    println!("  (ok) Resolved managed active binary: {:?}", resolved_managed.unwrap());

    // Explicit path override test
    let explicit_dummy = temp_tools_dir.join("explicit_server.exe");
    fs::write(&explicit_dummy, b"explicit")?;
    let resolved_explicit =
        resolve_antigravity_binary(Some(&explicit_dummy), &temp_tools_dir, "windows-x86_64");
    assert_eq!(resolved_explicit, Some(explicit_dummy));
    println!("  (ok) Explicit path correctly overrides managed release");

    // Clean up temporary tools test directory
    let _ = fs::remove_dir_all(&temp_tools_dir);

    // -----------------------------------------------------------------
    // 2. Replaced Environment & Sibling Temp Isolation
    // -----------------------------------------------------------------
    println!("\n[2/5] Testing Isolated Environment & Sibling Temp Directories...");
    let test_base = Path::new("C:\\Users\\MockUser\\AppData\\Local\\pandamux");
    let test_harness = test_base.join("tools/localharness_external.exe");
    let env_res = build_antigravity_env(
        "workforce_sub_1",
        test_base,
        &test_harness,
        "pandamux-browser-helper",
    );

    assert_eq!(
        env_res.gemini_home,
        test_base.join("profiles/antigravity/workforce_sub_1")
    );
    assert_eq!(
        env_res.scratch_dir,
        test_base.join("scratch/antigravity/workforce_sub_1")
    );

    // Sibling verification: scratch temp dir is a sibling of profile, not a child
    assert!(!env_res.scratch_dir.starts_with(&env_res.gemini_home));
    assert!(!env_res.gemini_home.starts_with(&env_res.scratch_dir));
    println!("  (ok) Verified sibling temp dir relationship to protect MAX_PATH");

    // Env map assertions
    assert_eq!(env_res.vars.get("AGY_ACP_FORCE_FILE_STORAGE").unwrap(), "1");
    assert_eq!(env_res.vars.get("PYTHONUNBUFFERED").unwrap(), "1");
    assert_eq!(
        env_res.vars.get("BROWSER").unwrap(),
        "pandamux-browser-helper"
    );
    assert_eq!(
        env_res.vars.get("TEMP").unwrap(),
        &env_res.scratch_dir.to_string_lossy().to_string()
    );

    // Verify sanitization
    assert!(validate_sanitized_environment(&env_res.vars));
    let mut dirty_vars = env_res.vars.clone();
    dirty_vars.insert("GOOGLE_API_KEY".into(), "secret_token_123".into());
    assert!(!validate_sanitized_environment(&dirty_vars));
    println!("  (ok) Sanitized environment rejects leaked Google/Gemini API tokens");

    // -----------------------------------------------------------------
    // 3. ACP Protocol Handshake, Sessions & Security Warnings
    // -----------------------------------------------------------------
    println!("\n[3/5] Testing ACP Protocol, Session Flow & Security Warnings...");
    let init_fixture = fs::read_to_string("fixtures/initialize_response.json")?;
    let init_val: serde_json::Value = serde_json::from_str(&init_fixture)?;
    let init_result: InitializeResult = serde_json::from_value(init_val["result"].clone())?;

    validate_antigravity_initialize(&init_result)?;
    println!(
        "  (ok) ACP initialize handshake validated: agent={}, version={}",
        init_result.agent_info.name, init_result.agent_info.version
    );

    // Mode mapping and session/new request
    let sess_req = build_session_new_request(2, map_access_mode_to_antigravity("FullAccess"));
    assert_eq!(sess_req["params"]["mode"], "yolo");
    assert_eq!(sess_req["params"]["clientCapabilities"]["terminal"], false);
    assert_eq!(sess_req["params"]["clientCapabilities"]["fs"]["readTextFile"], true);
    assert_eq!(sess_req["params"]["clientCapabilities"]["fs"]["writeTextFile"], true);
    println!("  (ok) Generated session/new request with clientCapabilities (terminal=false, fs=true)");

    // session/new response and model selection via configOptions
    let sess_new_fixture = fs::read_to_string("fixtures/session_new_response.json")?;
    let sess_new_val: serde_json::Value = serde_json::from_str(&sess_new_fixture)?;
    let session_id = sess_new_val["result"]["sessionId"].as_str().unwrap();
    assert_eq!(session_id, "sess_agy_4242");

    let model_config = &sess_new_val["result"]["configOptions"][0];
    assert_eq!(model_config["configId"], "model");
    let available_options = model_config["options"].as_array().unwrap();
    assert_eq!(available_options.len(), 2);
    println!(
        "  (ok) Extracted models from session configOptions: {} available (default: {})",
        available_options.len(),
        model_config["currentValue"]
    );

    // Verify session turn stream ndjson
    let stream_raw = fs::read_to_string("fixtures/session_turn_stream.ndjson")?;
    let mut warning_found = false;
    let mut interaction_found = false;
    let mut completion_found = false;

    for line in stream_raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let event_val: serde_json::Value = serde_json::from_str(line)?;
        let method = event_val.get("method").and_then(|m| m.as_str()).unwrap_or_default();

        if method == "session/requestPermission" {
            let perm: PermissionRequestParams = serde_json::from_value(event_val["params"].clone())?;
            if let Some(warning) = perm.security_warning() {
                assert!(warning.contains("Prompt injection"));
                warning_found = true;
                println!("  (ok) Captured prompt injection security warning from _meta: {}", warning);
            }
        } else if method == "session/event" {
            if let Some(tool_name) = event_val["params"]["event"]["toolName"].as_str() {
                if is_interaction_prompt(tool_name) {
                    interaction_found = true;
                    println!("  (ok) Detected interactive prompt tool call: {}", tool_name);
                }
            }
        } else if method == "session/completed" {
            assert_eq!(event_val["params"]["status"], "success");
            completion_found = true;
            println!("  (ok) Turn completion detected from session event (not process exit code)");
        }
    }
    assert!(warning_found, "Prompt injection warning must be extracted");
    assert!(interaction_found, "Interactive prompt question must be detected");
    assert!(completion_found, "Session completion event must be processed");

    // Error code mapping
    assert_eq!(map_acp_error_code(-32000), AcpErrorOutcome::AuthRequired);
    assert_eq!(map_acp_error_code(-32603), AcpErrorOutcome::InternalSessionFailure);
    println!("  (ok) Verified ACP error code mappings: -32000 (AuthRequired), -32603 (InternalSessionFailure)");

    // -----------------------------------------------------------------
    // 4. OAuth Sign-in & Remote Redirect Relay
    // -----------------------------------------------------------------
    println!("\n[4/5] Testing OAuth Sign-in URL Extraction & Remote Relay...");
    let oauth_fixture_raw = fs::read_to_string("fixtures/oauth_auth_flow.json")?;
    let oauth_fixture: serde_json::Value = serde_json::from_str(&oauth_fixture_raw)?;

    let marker = oauth_fixture["stdoutMarker"].as_str().unwrap();
    let auth_url = extract_auth_url_from_output(marker).expect("Must extract URL from stdout marker");
    assert_eq!(auth_url, oauth_fixture["expectedAuthUrl"].as_str().unwrap());
    println!("  (ok) Extracted OAuth sign-in URL from process stdout marker");

    // Test stderr BROWSER marker extraction
    let browser_marker = format!("[BROWSER] {}", auth_url);
    assert_eq!(extract_auth_url_from_output(&browser_marker), Some(auth_url.clone()));
    println!("  (ok) Extracted OAuth URL from BROWSER helper stderr marker");

    // Validate URL
    let validated_oauth = validate_google_oauth_url(&auth_url)?;
    assert_eq!(validated_oauth.redirect_port, oauth_fixture["expectedPort"].as_u64().unwrap() as u16);
    assert_eq!(validated_oauth.state, oauth_fixture["expectedState"].as_str().unwrap());
    println!(
        "  (ok) Validated Google OAuth URL: origin accounts.google.com, redirect port={}, state={}",
        validated_oauth.redirect_port, validated_oauth.state
    );

    // Negative URL test: reject fraudulent domain
    let phish_url = "https://evil.google.com/o/oauth2/auth?client_id=1&redirect_uri=http%3A%2F%2F127.0.0.1%3A8085%2F&state=123";
    assert!(validate_google_oauth_url(phish_url).is_err());
    println!("  (ok) Strict origin validation: rejected fraudulent domain");

    // Validate callback query and state check
    let callback_query = oauth_fixture["simulatedCallbackQuery"].as_str().unwrap();
    let auth_code = validate_callback_query(callback_query, &validated_oauth.state)?;
    assert_eq!(auth_code, "4/0AQ_TEST_TOKEN");
    println!("  (ok) Callback query validated with matching state token: code={}", auth_code);

    // Remote sign-in relay simulation:
    // On a remote host (e.g. Galahad), loopback redirect listener receives code,
    // relays it across tunnel to node loopback server.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let assigned_port = listener.local_addr()?.port();

    let relay_task = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\nAuthenticated";
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
            req
        } else {
            String::new()
        }
    });

    // Make local client request to simulated loopback listener
    let client = reqwest_like_get(assigned_port, callback_query).await?;
    let received_http_req = relay_task.await?;
    assert!(received_http_req.contains("4/0AQ_TEST_TOKEN"));
    assert!(client.contains("Authenticated"));
    println!(
        "  (ok) Simulated remote OAuth loopback relay on port {}: code successfully forwarded",
        assigned_port
    );

    // -----------------------------------------------------------------
    // 5. Reliability Rules & Resource Constraints
    // -----------------------------------------------------------------
    println!("\n[5/5] Testing Reliability Rules & Resource Constraints...");

    // Rule 1: Zero-spawn offline health check
    let fake_gemini_home = temp_tools_dir.join("profiles/personal");
    fs::create_dir_all(&fake_gemini_home)?;
    fs::write(
        fake_gemini_home.join("settings.json"),
        r#"{"auth":{"type":"oauth-personal"}}"#,
    )?;
    let dummy_exe = temp_tools_dir.join("test_server.exe");
    fs::write(&dummy_exe, b"exe")?;

    let health = probe_antigravity_health_offline(&dummy_exe, &fake_gemini_home, Some("1.1.1"));
    assert_eq!(
        health,
        AntigravityHealth::Healthy {
            version: "1.1.1".into(),
            auth_type: Some("oauth-personal".into()),
        }
    );
    println!("  (ok) Zero-spawn health check passed without launching PyInstaller process");

    // Rule 2: Orphan temp directory sweeper
    let scratch_test = temp_tools_dir.join("scratch_sweep_test");
    fs::create_dir_all(scratch_test.join("_MEI123456"))?;
    fs::create_dir_all(scratch_test.join("agy_tmp_98765"))?;
    fs::create_dir_all(scratch_test.join("active_keep_dir"))?;
    fs::write(scratch_test.join("active_keep_dir/data.txt"), b"keep")?;

    let swept_count = sweep_orphan_temp_dirs(&scratch_test)?;
    assert_eq!(swept_count, 2);
    assert!(!scratch_test.join("_MEI123456").exists());
    assert!(!scratch_test.join("agy_tmp_98765").exists());
    assert!(scratch_test.join("active_keep_dir").exists());
    println!("  (ok) Swept {} orphaned PyInstaller temp directories (_MEI*, agy_tmp_*)", swept_count);

    // Rule 3: Concurrency limiter (max 2 processes)
    let limiter = AntigravityConcurrencyLimiter::new(2);
    let guard1 = limiter.try_acquire()?;
    let guard2 = limiter.try_acquire()?;
    assert_eq!(limiter.current_active(), 2);
    assert!(limiter.try_acquire().is_err(), "Third process must exceed concurrency cap");
    drop(guard1);
    assert_eq!(limiter.current_active(), 1);
    let _guard3 = limiter.try_acquire()?;
    assert_eq!(limiter.current_active(), 2);
    drop(guard2);
    drop(_guard3);
    println!("  (ok) Concurrency limiter enforced: maximum 2 processes per environment");

    // Rule 4: Payload sanitization and command casing normalization
    let huge_text = "Echo ".repeat(20_000); // ~100 KB
    let sanitized = sanitize_tool_payload(&huge_text);
    assert!(sanitized.contains("[Truncated: exceeded 64 KB limit]"));
    assert!(sanitized.len() < huge_text.len());
    println!("  (ok) Sanitized tool payload: 100 KB output capped at 64 KB threshold");

    let val_cmd1 = serde_json::json!({ "CommandLine": "cargo test --workspace" });
    let val_cmd2 = serde_json::json!({ "command_line": "cargo clippy" });
    let val_cmd3 = serde_json::json!({ "cmd": "cargo check" });
    assert_eq!(extract_normalized_command(&val_cmd1), Some("cargo test --workspace".into()));
    assert_eq!(extract_normalized_command(&val_cmd2), Some("cargo clippy".into()));
    assert_eq!(extract_normalized_command(&val_cmd3), Some("cargo check".into()));
    println!("  (ok) Normalized alternate command line casing (CommandLine, command_line, cmd)");

    // Clean up temp test directory
    let _ = fs::remove_dir_all(&temp_tools_dir);

    println!("\n============================================================");
    println!("S5a ANTIGRAVITY VERIFICATION: ALL CONTRACT CHECKS PASSED.");
    println!("============================================================");

    Ok(())
}

async fn reqwest_like_get(port: u16, query: &str) -> Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).await?;
    let req = format!("GET /{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", query, port);
    stream.write_all(req.as_bytes()).await?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}
