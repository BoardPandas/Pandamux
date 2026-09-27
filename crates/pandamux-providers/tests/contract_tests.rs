use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use pandamux_core::{
    ids::ProviderInstanceId,
    provider_config::ProviderKind,
};
use pandamux_providers::{
    apply_profile_environment, build_claude_launch_args,
    claude::{
        parse_stream_json_lines, ClaudeAuthStatus, ControlResponse,
    },
    codex::{
        build_approval_response, build_thread_start_request, build_turn_start_request,
        parse_codex_notification, parse_rate_limits_response, CodexApprovalPolicy,
        CodexNotificationEvent, CodexSandboxPolicy, JsonRpcResponse,
    },
    resolve_command_shim, resolve_profile_dir,
    ENV_CLAUDE_CONFIG_DIR, ENV_CODEX_HOME, ENV_GEMINI_HOME,
};

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
    assert_eq!(perm.command.as_deref(), Some("curl -I https://api.anthropic.com"));

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
            if let Ok(CodexNotificationEvent::ApprovalRequest { approval_id, command, .. }) =
                parse_codex_notification(line)
            {
                approval_seen = true;
                let appr_resp = build_approval_response(20, &approval_id, true);
                assert_eq!(appr_resp.method, "turn/approvalResponse");
                assert_eq!(command, "cargo check --workspace");
            }
        } else if line.contains("\"turn/completed\"") {
            if let Ok(CodexNotificationEvent::TurnCompleted { total_tokens }) =
                parse_codex_notification(line)
            {
                turn_tokens = total_tokens;
            }
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
    apply_profile_environment(&mut codex_cmd, ProviderKind::Codex, &codex_dir, &BTreeMap::new());

    let gemini_dir = resolve_profile_dir(base, ProviderKind::Antigravity, &instance_1);
    let mut gemini_cmd = tokio::process::Command::new("echo");
    apply_profile_environment(&mut gemini_cmd, ProviderKind::Antigravity, &gemini_dir, &BTreeMap::new());

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
