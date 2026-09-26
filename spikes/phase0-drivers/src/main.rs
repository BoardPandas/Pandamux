mod claude;
mod codex;
mod shim_resolver;

use std::fs;

use anyhow::Result;
use claude::{
    build_claude_launch_args, parse_stream_json_lines, probe_claude_auth, ClaudeAuthStatus,
    ControlResponse,
};
use codex::{
    build_approval_response, build_thread_start_request, build_turn_start_request,
    parse_codex_notification, parse_rate_limits_response, CodexApprovalPolicy,
    CodexNotificationEvent, CodexSandboxPolicy, JsonRpcResponse,
};
use shim_resolver::resolve_command_shim;

fn main() -> Result<()> {
    println!("============================================================");
    println!("PandaMUX Phase 0: S2 Driver Verification (Claude + Codex)");
    println!("============================================================");

    // -----------------------------------------------------------------
    // 1. Windows Command Shim Resolution
    // -----------------------------------------------------------------
    println!("\n[1/5] Testing Windows Command & Shim Resolution...");
    let cmd_resolved = resolve_command_shim("cmd")
        .expect("Should resolve 'cmd' on Windows");
    println!("  ✓ Resolved 'cmd': {:?}", cmd_resolved.program);

    if let Some(claude_resolved) = resolve_command_shim("claude") {
        println!("  ✓ Resolved 'claude': {:?} (shim: {:?})", claude_resolved.program, claude_resolved.shim_type);
    } else {
        println!("  - 'claude' not found in PATH; skipping live shim check");
    }

    // -----------------------------------------------------------------
    // 2. Claude Zero-Session Auth Probe
    // -----------------------------------------------------------------
    println!("\n[2/5] Testing Claude Auth Probe...");
    let unauth_fixture = fs::read_to_string("fixtures/claude/auth_status_unauth.json")?;
    let unauth_status: ClaudeAuthStatus = serde_json::from_str(&unauth_fixture)?;
    assert!(!unauth_status.logged_in, "Fixture unauth must report loggedIn = false");
    println!("  ✓ Unauthenticated fixture parsed: loggedIn=false, method={:?}", unauth_status.auth_method);

    let auth_fixture = fs::read_to_string("fixtures/claude/auth_status_auth.json")?;
    let auth_status: ClaudeAuthStatus = serde_json::from_str(&auth_fixture)?;
    assert!(auth_status.logged_in, "Fixture auth must report loggedIn = true");
    println!(
        "  ✓ Authenticated fixture parsed: email={:?}, tier={:?}",
        auth_status.email, auth_status.subscription_type
    );

    // Live probe check against installed claude executable if present
    if let Some(claude_cmd) = resolve_command_shim("claude") {
        println!("  -> Running live probe: claude auth status --json...");
        match probe_claude_auth(&claude_cmd.program, None) {
            Ok(live_status) => {
                println!(
                    "  ✓ Live Claude probe succeeded without session creation: loggedIn={}, provider={:?}",
                    live_status.logged_in, live_status.api_provider
                );
            }
            Err(e) => {
                println!("  ! Live probe warning (non-fatal): {}", e);
            }
        }
    }

    // -----------------------------------------------------------------
    // 3. Claude Stream-JSON & Sub-Agent Parsing
    // -----------------------------------------------------------------
    println!("\n[3/5] Testing Claude Stream-JSON Protocol & Sub-Agents...");
    let stream_fixture = fs::read_to_string("fixtures/claude/stream_json_turn.ndjson")?;
    let parsed_stream = parse_stream_json_lines(&stream_fixture)?;

    assert_eq!(parsed_stream.session_id.as_deref(), Some("sess_claude_01"));
    assert_eq!(parsed_stream.control_requests.len(), 1);
    let perm_req = &parsed_stream.control_requests[0];
    assert_eq!(perm_req.tool_name, "Bash");
    println!("  ✓ Captured control_request: id={}, tool={}, cmd={:?}", perm_req.request_id, perm_req.tool_name, perm_req.command);

    let allow_resp = ControlResponse::allow(&perm_req.request_id);
    let allow_json = serde_json::to_string(&allow_resp)?;
    assert!(allow_json.contains("\"decision\":\"allow\""));
    println!("  ✓ Generated control_response (allow): {}", allow_json);

    assert_eq!(parsed_stream.subagents.len(), 1);
    let sub = &parsed_stream.subagents[0];
    assert_eq!(sub.subagent_type, "reviewer");
    assert_eq!(sub.parent_tool_use_id.as_deref(), Some("toolu_01"));
    println!(
        "  ✓ Extracted sub-agent Task: type={}, parent_id={:?}, model={:?}",
        sub.subagent_type, sub.parent_tool_use_id, sub.model
    );

    let usage = parsed_stream.usage.expect("Usage must be parsed");
    assert_eq!(usage.input_tokens, 1250);
    assert_eq!(usage.output_tokens, 380);
    println!("  ✓ Extracted turn usage: {} in, {} out, cost=${:.4}", usage.input_tokens, usage.output_tokens, usage.cost_usd.unwrap_or(0.0));

    // Denial fixture check
    let denial_fixture = fs::read_to_string("fixtures/claude/control_denial_turn.ndjson")?;
    let parsed_denial = parse_stream_json_lines(&denial_fixture)?;
    assert_eq!(parsed_denial.control_requests.len(), 1);
    let deny_resp = ControlResponse::deny(&parsed_denial.control_requests[0].request_id, "Outbound POST blocked");
    println!("  ✓ Generated control_response (deny with message): {}", serde_json::to_string(&deny_resp)?);

    // Argument builder check
    let args = build_claude_launch_args(
        Some("Injected instructions"),
        &["Read".into(), "Edit".into()],
        &["Bash".into()],
        Some("auto"),
        Some("sess-uuid-42"),
    );
    assert!(args.contains(&"--append-system-prompt".into()));
    assert!(args.contains(&"--allowedTools".into()));
    assert!(args.contains(&"--disallowedTools".into()));
    println!("  ✓ Verified CLI launch arguments: {:?}", args);

    // -----------------------------------------------------------------
    // 4. Codex Schema Dump & Knobs Verification
    // -----------------------------------------------------------------
    println!("\n[4/5] Testing Codex Schema & Knobs...");
    let schema_fixture = fs::read_to_string("fixtures/codex/schema_dump.json")?;
    let schema_val: serde_json::Value = serde_json::from_str(&schema_fixture)?;
    let methods = schema_val.get("methods").expect("Schema must have methods");

    assert!(methods.get("thread/start").is_some());
    assert!(methods.get("account/rateLimits/read").is_some());

    let thread_start = &methods["thread/start"]["params"]["properties"];
    assert!(thread_start.get("developerInstructions").is_some(), "Must support developerInstructions");
    assert!(thread_start.get("sandboxPolicy").is_some(), "Must support sandboxPolicy");
    assert!(thread_start.get("approvalPolicy").is_some(), "Must support approvalPolicy");
    println!("  ✓ Codex schema confirms developerInstructions, sandboxPolicy, and approvalPolicy knobs");

    let thread_req = build_thread_start_request(
        10,
        "C:\\Workspace",
        "o3-mini",
        Some("Developer role instructions for architect"),
        CodexSandboxPolicy::WorkspaceWrite,
        CodexApprovalPolicy::OnWrite,
    );
    let thread_json = serde_json::to_string(&thread_req)?;
    assert!(thread_json.contains("developerInstructions"));
    println!("  ✓ Generated thread/start request with developerInstructions");

    let turn_req = build_turn_start_request(11, "th_codex_01", "Inspect crate boundaries");
    let turn_json = serde_json::to_string(&turn_req)?;
    assert!(turn_json.contains("th_codex_01"));
    println!("  ✓ Generated turn/start request for thread");

    // -----------------------------------------------------------------
    // 5. Codex Turn Lifecycle & Rate Limits
    // -----------------------------------------------------------------
    println!("\n[5/5] Testing Codex Turn Lifecycle & Rate Limits...");
    let turn_jsonl = fs::read_to_string("fixtures/codex/turn_lifecycle.jsonl")?;
    let mut turn_tokens = 0;
    let mut approval_seen = false;

    for line in turn_jsonl.lines() {
        if line.contains("\"turn/approvalRequest\"") {
            if let Ok(CodexNotificationEvent::ApprovalRequest { approval_id, command, .. }) = parse_codex_notification(line) {
                approval_seen = true;
                let appr_resp = build_approval_response(20, &approval_id, true);
                assert_eq!(appr_resp.method, "turn/approvalResponse");
                println!("  ✓ Handled Codex approval request for {:?}: generated approvalResponse", command);
            }
        } else if line.contains("\"turn/completed\"") {
            if let Ok(CodexNotificationEvent::TurnCompleted { total_tokens }) = parse_codex_notification(line) {
                turn_tokens = total_tokens;
            }
        }
    }
    assert!(approval_seen, "Approval flow must be verified");
    assert_eq!(turn_tokens, 1420);
    println!("  ✓ Verified Codex turn lifecycle: approval roundtrip + {} total tokens", turn_tokens);

    let limits_raw = fs::read_to_string("fixtures/codex/account_ratelimits.json")?;
    let limits_resp = JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id: 5,
        result: Some(serde_json::from_str(&limits_raw)?),
        error: None,
    };
    let parsed_limits = parse_rate_limits_response(&limits_resp)?;
    assert_eq!(parsed_limits.len(), 2);
    println!(
        "  ✓ Parsed Codex rateLimits: {} limit={} used={}% resets={}",
        parsed_limits.len(),
        parsed_limits[0].limit_name,
        parsed_limits[0].used_percent,
        parsed_limits[0].resets_at
    );

    println!("\n============================================================");
    println!("S2 DRIVER VERIFICATION: ALL CONTRACT CHECKS PASSED.");
    println!("============================================================");

    Ok(())
}
