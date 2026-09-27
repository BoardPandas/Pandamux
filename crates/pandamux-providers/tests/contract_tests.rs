use pandamux_core::{ids::ProviderInstanceId, provider_config::ProviderKind};
use pandamux_providers::{
    ENV_CLAUDE_CONFIG_DIR, ENV_CODEX_HOME, ENV_GEMINI_HOME, apply_profile_environment,
    build_claude_launch_args,
    claude::{ClaudeAuthStatus, ControlResponse, parse_stream_json_lines},
    codex::{
        CodexApprovalPolicy, CodexNotificationEvent, CodexSandboxPolicy, JsonRpcResponse,
        build_approval_response, build_thread_start_request, build_turn_start_request,
        parse_codex_notification, parse_rate_limits_response,
    },
    resolve_command_shim, resolve_profile_dir,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CLAUDE_AUTH_JSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/claude/auth_status_auth.json");
const CLAUDE_UNAUTH_JSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/claude/auth_status_unauth.json");
const CLAUDE_STREAM_NDJSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/claude/stream_json_turn.ndjson");
const CLAUDE_DENIAL_NDJSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/claude/control_denial_turn.ndjson");

const CODEX_SCHEMA_JSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/codex/schema_dump.json");
const CODEX_LIFECYCLE_JSONL: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/codex/turn_lifecycle.jsonl");
const CODEX_RATELIMITS_JSON: &str =
    include_str!("../../../spikes/phase0-drivers/fixtures/codex/account_ratelimits.json");

#[test]
fn test_claude_auth_probe_contract() {
    let unauth: ClaudeAuthStatus =
        serde_json::from_str(CLAUDE_UNAUTH_JSON).expect("parse unauth fixture");
    assert!(!unauth.logged_in);
    assert_eq!(unauth.auth_method.as_deref(), Some("none"));

    let auth: ClaudeAuthStatus =
        serde_json::from_str(CLAUDE_AUTH_JSON).expect("parse auth fixture");
    assert!(auth.logged_in);
    assert_eq!(auth.email.as_deref(), Some("developer@boardpandas.com"));
    assert_eq!(auth.subscription_type.as_deref(), Some("max"));
}

#[test]
fn test_claude_stream_json_and_subagents() {
    let parsed = parse_stream_json_lines(CLAUDE_STREAM_NDJSON).expect("parse stream ndjson");
    assert_eq!(parsed.session_id.as_deref(), Some("sess_claude_01"));

    // Control requests
    assert_eq!(parsed.control_requests.len(), 1);
    let perm = &parsed.control_requests[0];
    assert_eq!(perm.tool_name, "Bash");
    assert_eq!(
        perm.command.as_deref(),
        Some("curl -I https://api.anthropic.com")
    );

    // Allow response
    let allow = ControlResponse::allow(&perm.request_id);
    let allow_json = serde_json::to_string(&allow).expect("serialize allow");
    assert!(allow_json.contains("\"decision\":\"allow\""));

    // Subagent extraction
    assert_eq!(parsed.subagents.len(), 1);
    let sub = &parsed.subagents[0];
    assert_eq!(sub.subagent_type, "reviewer");
    assert_eq!(sub.parent_tool_use_id.as_deref(), Some("toolu_01"));
    assert_eq!(sub.model.as_deref(), Some("claude-3-5-haiku-20241022"));

    // Usage extraction
    let usage = parsed.usage.expect("usage exists");
    assert_eq!(usage.input_tokens, 1250);
    assert_eq!(usage.output_tokens, 380);
    assert_eq!(usage.cost_usd, Some(0.0084));
}

#[test]
fn test_claude_control_denial_and_launch_args() {
    let parsed = parse_stream_json_lines(CLAUDE_DENIAL_NDJSON).expect("parse denial ndjson");
    assert_eq!(parsed.control_requests.len(), 1);

    let deny = ControlResponse::deny(
        &parsed.control_requests[0].request_id,
        "Outbound POST blocked",
    );
    let deny_json = serde_json::to_string(&deny).expect("serialize deny");
    assert!(deny_json.contains("\"decision\":\"deny\""));
    assert!(deny_json.contains("Outbound POST blocked"));

    let args = build_claude_launch_args(
        Some("Injected agent instructions"),
        &["Read".into(), "Edit".into()],
        &["Bash".into()],
        Some("auto"),
        Some("sess-uuid-42"),
    );
    assert!(args.contains(&"--append-system-prompt".into()));
    assert!(args.contains(&"--allowedTools".into()));
    assert!(args.contains(&"--disallowedTools".into()));
    assert!(args.contains(&"--permission-mode".into()));
    assert!(args.contains(&"--session-id".into()));
}

#[test]
fn test_codex_schema_and_knobs_contract() {
    let schema_val: serde_json::Value =
        serde_json::from_str(CODEX_SCHEMA_JSON).expect("parse schema");
    let methods = schema_val.get("methods").expect("methods in schema");

    assert!(methods.get("thread/start").is_some());
    assert!(methods.get("account/rateLimits/read").is_some());

    let thread_start = &methods["thread/start"]["params"]["properties"];
    assert!(thread_start.get("developerInstructions").is_some());
    assert!(thread_start.get("sandboxPolicy").is_some());
    assert!(thread_start.get("approvalPolicy").is_some());

    let thread_req = build_thread_start_request(
        10,
        "C:\\Workspace",
        "o3-mini",
        Some("Developer instructions for reviewer"),
        CodexSandboxPolicy::WorkspaceWrite,
        CodexApprovalPolicy::OnWrite,
    );
    let thread_json = serde_json::to_string(&thread_req).expect("serialize thread/start");
    assert!(thread_json.contains("developerInstructions"));
    assert!(thread_json.contains("workspace-write"));
    assert!(thread_json.contains("on-write"));

    let turn_req = build_turn_start_request(11, "th_codex_01", "Inspect crate boundaries");
    let turn_json = serde_json::to_string(&turn_req).expect("serialize turn/start");
    assert!(turn_json.contains("th_codex_01"));
}

#[test]
fn test_codex_lifecycle_and_ratelimits() {
    let mut turn_tokens = 0;
    let mut approval_seen = false;

    for line in CODEX_LIFECYCLE_JSONL.lines() {
        if line.contains("\"turn/approvalRequest\"") {
            if let Ok(CodexNotificationEvent::ApprovalRequest {
                approval_id,
                command,
                ..
            }) = parse_codex_notification(line)
            {
                approval_seen = true;
                let appr_resp = build_approval_response(20, &approval_id, true);
                assert_eq!(appr_resp.method, "turn/approvalResponse");
                assert_eq!(command, "cargo check --workspace");
            }
        } else if line.contains("\"turn/completed\"")
            && let Ok(CodexNotificationEvent::TurnCompleted { total_tokens }) =
                parse_codex_notification(line)
        {
            turn_tokens = total_tokens;
        }
    }

    assert!(approval_seen);
    assert_eq!(turn_tokens, 1420);

    let limits_raw: serde_json::Value =
        serde_json::from_str(CODEX_RATELIMITS_JSON).expect("parse limits fixture");
    let limits_resp = JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id: 5,
        result: Some(limits_raw),
        error: None,
    };
    let parsed_limits = parse_rate_limits_response(&limits_resp).expect("parse rate limits");
    assert_eq!(parsed_limits.len(), 2);
    assert_eq!(parsed_limits[0].limit_name, "tokens");
    assert_eq!(parsed_limits[0].used_percent, 34.2);
    assert_eq!(parsed_limits[1].limit_name, "requests");
    assert_eq!(parsed_limits[1].used_percent, 18.5);
}

#[test]
fn test_multi_instance_profile_isolation() {
    let base = Path::new("/var/pandamux");
    let instance_1 = ProviderInstanceId::from("inst-1");
    let instance_2 = ProviderInstanceId::from("inst-2");

    let p1 = resolve_profile_dir(base, ProviderKind::Claude, &instance_1);
    let p2 = resolve_profile_dir(base, ProviderKind::Claude, &instance_2);
    assert_ne!(p1, p2);
    assert_eq!(p1, PathBuf::from("/var/pandamux/profiles/claude/inst-1"));

    // Check env injection
    let mut cmd = tokio::process::Command::new("echo");
    let mut overrides = BTreeMap::new();
    overrides.insert("CUSTOM_VAR".to_string(), "custom_val".to_string());

    apply_profile_environment(&mut cmd, ProviderKind::Claude, &p1, &overrides);

    let codex_dir = resolve_profile_dir(base, ProviderKind::Codex, &instance_1);
    let mut codex_cmd = tokio::process::Command::new("echo");
    apply_profile_environment(
        &mut codex_cmd,
        ProviderKind::Codex,
        &codex_dir,
        &BTreeMap::new(),
    );

    let gemini_dir = resolve_profile_dir(base, ProviderKind::Antigravity, &instance_1);
    let mut gemini_cmd = tokio::process::Command::new("echo");
    apply_profile_environment(
        &mut gemini_cmd,
        ProviderKind::Antigravity,
        &gemini_dir,
        &BTreeMap::new(),
    );

    assert_eq!(ENV_CLAUDE_CONFIG_DIR, "CLAUDE_CONFIG_DIR");
    assert_eq!(ENV_CODEX_HOME, "CODEX_HOME");
    assert_eq!(ENV_GEMINI_HOME, "GEMINI_HOME");
}

#[test]
fn test_shim_resolution() {
    #[cfg(windows)]
    let target = "cmd";
    #[cfg(not(windows))]
    let target = "sh";

    let resolved = resolve_command_shim(target).expect("resolve system shell");
    assert!(resolved.program.exists());
    assert!(resolved.is_direct_executable);
}

const AGY_INIT_JSON: &str =
    include_str!("../../../spikes/phase0-antigravity/fixtures/initialize_response.json");
const AGY_MANIFEST_JSON: &str =
    include_str!("../../../spikes/phase0-antigravity/fixtures/managed_bundle_manifest.json");
const AGY_OAUTH_JSON: &str =
    include_str!("../../../spikes/phase0-antigravity/fixtures/oauth_auth_flow.json");
const AGY_STREAM_NDJSON: &str =
    include_str!("../../../spikes/phase0-antigravity/fixtures/session_turn_stream.ndjson");

#[test]
fn test_antigravity_initialize_contract() {
    let init_val: serde_json::Value =
        serde_json::from_str(AGY_INIT_JSON).expect("parse init fixture");
    let result_obj = init_val.get("result").expect("result in init fixture");
    let res: pandamux_providers::acp::AcpInitializeResult =
        serde_json::from_value(result_obj.clone()).expect("deserialize AcpInitializeResult");

    assert_eq!(res.protocol_version, 1);
    assert_eq!(res.agent_info.name, "antigravity-acp");
    assert!(res.capabilities.load_session);
    assert!(res.capabilities.resume_session);
    assert!(res.auth_methods.contains(&"oauth-personal".to_string()));

    pandamux_providers::acp::validate_antigravity_initialize(&res)
        .expect("validate initialize succeeds");
}

#[test]
fn test_antigravity_session_new_and_client_capabilities() {
    use pandamux_core::thread::AccessMode;
    use pandamux_providers::acp::{AcpMode, build_session_new_request, map_access_mode_to_acp};

    assert_eq!(
        map_access_mode_to_acp(AccessMode::FullAccess),
        AcpMode::Yolo
    );
    assert_eq!(
        map_access_mode_to_acp(AccessMode::AutoEdit),
        AcpMode::AutoEdit
    );
    assert_eq!(
        map_access_mode_to_acp(AccessMode::ReadOnly),
        AcpMode::Default
    );

    let session_req = build_session_new_request(101, AcpMode::Yolo);
    assert_eq!(session_req["method"], "session/new");
    assert_eq!(session_req["params"]["mode"], "yolo");

    let client_caps = &session_req["params"]["clientCapabilities"];
    assert_eq!(client_caps["terminal"], false);
    assert_eq!(client_caps["fs"]["readTextFile"], true);
    assert_eq!(client_caps["fs"]["writeTextFile"], true);
}

#[test]
fn test_antigravity_oauth_flow_contract() {
    use pandamux_providers::antigravity::{
        extract_auth_url_from_output, validate_callback_query, validate_google_oauth_url,
    };

    let oauth_fixture: serde_json::Value =
        serde_json::from_str(AGY_OAUTH_JSON).expect("parse oauth fixture");

    let stdout_marker = oauth_fixture["stdoutMarker"]
        .as_str()
        .expect("stdoutMarker");
    let extracted_url =
        extract_auth_url_from_output(stdout_marker).expect("extract auth url from marker");

    let validated = validate_google_oauth_url(&extracted_url).expect("validate google oauth url");
    assert_eq!(validated.redirect_port, 8085);
    assert_eq!(validated.state, "sec_state_9876");
    assert!(validated.client_id.contains("apps.googleusercontent.com"));

    let callback_query = oauth_fixture["simulatedCallbackQuery"]
        .as_str()
        .expect("simulatedCallbackQuery");
    let code =
        validate_callback_query(callback_query, "sec_state_9876").expect("validate callback query");
    assert_eq!(code, "4/0AQ_TEST_TOKEN");

    // Mismatched state must be rejected
    assert!(validate_callback_query(callback_query, "wrong_state").is_err());
}

#[test]
fn test_antigravity_managed_bundle_manifest_validation() {
    use pandamux_providers::antigravity::{ManagedBundleManifest, verify_bundle_manifest};

    let manifest: ManagedBundleManifest =
        serde_json::from_str(AGY_MANIFEST_JSON).expect("parse manifest fixture");
    assert_eq!(manifest.version, "1.1.1");
    assert_eq!(manifest.entries.len(), 2);
    assert!(manifest.entries.contains(&"agy_acp_server.exe".to_string()));
    assert!(
        manifest
            .entries
            .contains(&"localharness_external.exe".to_string())
    );

    // Path traversal in manifest must be rejected
    let mut bad_manifest = manifest.clone();
    bad_manifest.entries = vec![
        "../evil.exe".to_string(),
        "localharness_external.exe".to_string(),
    ];
    let dummy_bytes = vec![0u8; bad_manifest.size as usize];
    assert!(verify_bundle_manifest(&bad_manifest, &dummy_bytes).is_err());
}

#[test]
fn test_antigravity_spawn_environment_and_credential_sanitization() {
    use pandamux_providers::antigravity::{build_antigravity_env, validate_sanitized_environment};
    use std::collections::HashMap;

    let base = Path::new("/var/pandamux");
    let harness = Path::new("/var/pandamux/tools/localharness_external");
    let env_config = build_antigravity_env("inst-test", base, harness, "pandamux-browser-helper");

    assert_eq!(
        env_config.gemini_home,
        PathBuf::from("/var/pandamux/profiles/antigravity/inst-test")
    );
    assert_eq!(
        env_config.scratch_dir,
        PathBuf::from("/var/pandamux/scratch/antigravity/inst-test")
    );
    assert_eq!(
        env_config
            .vars
            .get("AGY_ACP_FORCE_FILE_STORAGE")
            .map(|s| s.as_str()),
        Some("1")
    );
    assert_eq!(
        env_config.vars.get("PYTHONUNBUFFERED").map(|s| s.as_str()),
        Some("1")
    );
    assert_eq!(
        env_config.vars.get("BROWSER").map(|s| s.as_str()),
        Some("pandamux-browser-helper")
    );

    // Sanitized environment check
    assert!(validate_sanitized_environment(&env_config.vars));

    let mut tainted_vars = HashMap::new();
    tainted_vars.insert("GEMINI_API_KEY".to_string(), "secret-123".to_string());
    assert!(!validate_sanitized_environment(&tainted_vars));

    let mut tainted_oauth = HashMap::new();
    tainted_oauth.insert("AGY_ACP_TOKEN".to_string(), "tok-456".to_string());
    assert!(!validate_sanitized_environment(&tainted_oauth));
}

#[test]
fn test_antigravity_seven_reliability_rules() {
    use pandamux_providers::acp::{AcpPermissionRequest, is_interaction_prompt};
    use pandamux_providers::antigravity::{
        AntigravityConcurrencyLimiter, probe_antigravity_health_offline, sweep_orphan_temp_dirs,
    };

    // Rule 1: Zero-spawn offline health check
    let non_existent = Path::new("/non/existent/agy_acp_server");
    let gemini_home = Path::new("/var/pandamux/profiles/antigravity/inst-1");
    let health = probe_antigravity_health_offline(non_existent, gemini_home, None);
    assert!(matches!(
        health,
        pandamux_providers::ProviderHealth::Unavailable { .. }
    ));

    // Rule 2: Temp directory sweeper
    let scratch = std::env::temp_dir().join("pandamux_test_scratch");
    let _ = std::fs::create_dir_all(&scratch);
    let orphan1 = scratch.join("_MEI12345");
    let orphan2 = scratch.join("agy_tmp_67890");
    let keep_file = scratch.join("valid_file.txt");
    let _ = std::fs::create_dir_all(&orphan1);
    let _ = std::fs::create_dir_all(&orphan2);
    let _ = std::fs::write(&keep_file, b"keep");

    let swept = sweep_orphan_temp_dirs(&scratch).expect("sweep orphan dirs");
    assert_eq!(swept, 2);
    assert!(!orphan1.exists());
    assert!(!orphan2.exists());
    assert!(keep_file.exists());
    let _ = std::fs::remove_dir_all(&scratch);

    // Rule 3: Concurrency limiter (max 2 active processes)
    let limiter = AntigravityConcurrencyLimiter::new(2);
    let guard1 = limiter.try_acquire().expect("acquire 1");
    let guard2 = limiter.try_acquire().expect("acquire 2");
    assert!(
        limiter.try_acquire().is_err(),
        "Third concurrent acquire must fail"
    );
    drop(guard1);
    let guard3 = limiter.try_acquire().expect("acquire 3 after drop");
    drop(guard2);
    drop(guard3);

    // Rule 6: Interaction prompt detection
    assert!(is_interaction_prompt("interaction_confirm_deployment"));
    assert!(!is_interaction_prompt("fs/writeTextFile"));

    // Rule 7: Permission request security warning extraction
    let perm_json = r#"{
        "sessionId": "sess_agy_4242",
        "permissionType": "file_write",
        "resource": "src/main.rs",
        "options": ["allow_once", "allow_always", "reject_once"],
        "_meta": {
            "agy.security.warning": "Prompt injection risk detected in source comment"
        }
    }"#;
    let perm_req: AcpPermissionRequest = serde_json::from_str(perm_json).expect("parse permission");
    assert_eq!(
        perm_req.security_warning().as_deref(),
        Some("Prompt injection risk detected in source comment")
    );
}

#[test]
fn test_antigravity_stream_parsing() {
    use pandamux_providers::acp::parse_acp_line;

    let mut event_count = 0;
    for line in AGY_STREAM_NDJSON.lines() {
        if let Some(event) = parse_acp_line(line) {
            event_count += 1;
            match event {
                pandamux_providers::ProviderEvent::TextDelta { delta } => {
                    assert!(delta.contains("Analyzing codebase"));
                }
                pandamux_providers::ProviderEvent::TurnCompleted { usage, .. } => {
                    assert_eq!(usage.unwrap().input_tokens, 2450);
                }
                _ => {}
            }
        }
    }
    assert!(event_count >= 1);
}
