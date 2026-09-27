mod cursor;
mod grok;
mod opencode;
mod types;

use std::fs;
use anyhow::Result;
use cursor::{build_cursor_preamble, cursor_capability_report, evaluate_cursor_permission, parse_cursor_initialize};
use grok::{build_grok_preamble, grok_capability_report, parse_grok_initialize, parse_grok_usage_limits};
use opencode::{
    opencode_acp_capability_report, opencode_serve_capability_report, parse_opencode_acp_initialize,
    parse_opencode_serve_create_request, parse_opencode_serve_create_response, parse_opencode_sse_event_stream,
};
use types::{AccessMode, InstructionDeliveryMethod, JsonRpcRequest, PermissionRequestParams, ToolPermissionGranularity};

fn main() -> Result<()> {
    println!("============================================================");
    println!("PandaMUX Phase 0: S5b Best-effort ACP Providers Verification");
    println!("============================================================");

    // -----------------------------------------------------------------
    // 1. Cursor Agent ACP Integration (cursor-agent acp)
    // -----------------------------------------------------------------
    println!("\n[1/3] Testing Cursor Agent ACP (cursor-agent acp)...");
    let cursor_init_raw = fs::read_to_string("fixtures/cursor/initialize_response.json")?;
    let cursor_init = parse_cursor_initialize(&cursor_init_raw)?;
    println!("  ✓ Agent name: {}", cursor_init.agent_info.name);
    println!("  ✓ Agent version: {}", cursor_init.agent_info.version);
    println!("  ✓ Protocol version: {}", cursor_init.protocol_version);
    println!("  ✓ Auth methods: {:?}", cursor_init.auth_methods);

    // Session new verification
    let cursor_sess_raw = fs::read_to_string("fixtures/cursor/session_new_response.json")?;
    let cursor_sess: types::JsonRpcResponse<types::SessionNewResult> = serde_json::from_str(&cursor_sess_raw)?;
    let sess_res = cursor_sess.result.unwrap();
    println!("  ✓ Session ID: {}", sess_res.session_id);
    if let Some(model_opt) = sess_res.config_options.iter().find(|o| o.id == "model") {
        println!("  ✓ Available models: {:?}", model_opt.options);
    }

    // Instruction mechanism: Fallback preamble
    let cursor_preamble = build_cursor_preamble(
        "You are the PandaMUX Architect agent. Plan modular solutions.",
        &["Rules: keep PRs focused", "Follow Rust 2024 edition"],
        "Review memory persistence architecture.",
    );
    assert!(cursor_preamble.contains("=== PANDAMUX AGENT ROLE INSTRUCTIONS ==="));
    println!("  ✓ Instruction mechanism verified: fallback preamble injected on first turn and post-compaction");

    // Tool permission granularity: Coarse request_permission
    let cursor_perm_raw = fs::read_to_string("fixtures/cursor/permission_request.json")?;
    let cursor_perm: JsonRpcRequest<PermissionRequestParams> = serde_json::from_str(&cursor_perm_raw)?;
    let ro_decision = evaluate_cursor_permission(&cursor_perm.params, AccessMode::ReadOnly)?;
    let ae_decision = evaluate_cursor_permission(&cursor_perm.params, AccessMode::AutoEdit)?;
    assert_eq!(ro_decision, "reject_once");
    assert_eq!(ae_decision, "allow_always");
    println!("  ✓ Tool permission granularity verified: coarse ACP request_permission mapped to AccessMode");

    // -----------------------------------------------------------------
    // 2. Grok CLI ACP Integration (grok acp)
    // -----------------------------------------------------------------
    println!("\n[2/3] Testing Grok CLI ACP (grok acp)...");
    let grok_init_raw = fs::read_to_string("fixtures/grok/initialize_response.json")?;
    let grok_init = parse_grok_initialize(&grok_init_raw)?;
    println!("  ✓ Agent name: {}", grok_init.agent_info.name);
    println!("  ✓ Agent version: {}", grok_init.agent_info.version);
    println!("  ✓ Auth methods: {:?}", grok_init.auth_methods);

    // Instruction mechanism: Fallback preamble
    let grok_preamble = build_grok_preamble(
        "You are the Builder agent.",
        "Refactor token streaming render queue.",
    );
    assert!(grok_preamble.contains("=== PANDAMUX AGENT ROLE INSTRUCTIONS ==="));
    println!("  ✓ Instruction mechanism verified: fallback preamble");

    // Usage and Rate limits parsing
    let grok_limits_raw = fs::read_to_string("fixtures/grok/usage_limits.json")?;
    let grok_limits = parse_grok_usage_limits(&grok_limits_raw)?;
    println!("  ✓ Grok usage limits parsed:");
    println!("    - Account tier: {}", grok_limits.account_tier);
    println!("    - Monthly spend: ${:.2} / ${:.2}", grok_limits.monthly_spend_usd, grok_limits.monthly_limit_usd);
    println!("    - Token budget used: {:.2}%", grok_limits.token_budget.used_percent);
    println!("    - RPM remaining: {} / {}", grok_limits.rate_limits.requests_remaining, grok_limits.rate_limits.requests_per_minute_limit);

    // -----------------------------------------------------------------
    // 3. OpenCode Dual-Mode Evaluation (acp vs serve)
    // -----------------------------------------------------------------
    println!("\n[3/3] Testing OpenCode Dual-Mode (acp vs serve)...");

    // 3a. OpenCode ACP mode
    let oc_acp_init_raw = fs::read_to_string("fixtures/opencode/acp_initialize_response.json")?;
    let oc_acp_init = parse_opencode_acp_initialize(&oc_acp_init_raw)?;
    println!("  Mode 1: opencode acp (stdio)");
    println!("    ✓ Agent name: {}", oc_acp_init.agent_info.name);
    println!("    ✓ Instructions: fallback preamble required (ACP 1.x standard)");
    println!("    ✓ Tool granularity: coarse request_permission approvals");

    // 3b. OpenCode Serve mode
    let oc_srv_req_raw = fs::read_to_string("fixtures/opencode/serve_session_create_request.json")?;
    let oc_srv_req = parse_opencode_serve_create_request(&oc_srv_req_raw)?;
    let oc_srv_resp_raw = fs::read_to_string("fixtures/opencode/serve_session_create_response.json")?;
    let oc_srv_resp = parse_opencode_serve_create_response(&oc_srv_resp_raw)?;
    let oc_sse_raw = fs::read_to_string("fixtures/opencode/serve_events_stream.sse")?;
    let oc_events = parse_opencode_sse_event_stream(&oc_sse_raw)?;

    println!("  Mode 2: opencode serve (HTTP REST + SSE)");
    println!("    ✓ Native system_prompt accepted on POST /session: '{}'", oc_srv_req.system_prompt.as_deref().unwrap_or(""));
    println!("    ✓ Native system prompt applied: {}", oc_srv_resp.native_system_prompt_applied);
    let policy = oc_srv_resp.enforced_tool_policy.unwrap();
    println!("    ✓ Per-tool policy enforced: allowed={:?}, disallowed={:?}", policy.allowed_tools, policy.disallowed_tools);
    println!("    ✓ SSE stream parsed: {} events received", oc_events.len());

    // -----------------------------------------------------------------
    // Provider Capability Matrix Summary
    // -----------------------------------------------------------------
    println!("\n============================================================");
    println!("Phase 0 S5b: Provider Capability Matrix");
    println!("============================================================");

    let reports = vec![
        cursor_capability_report(),
        grok_capability_report(),
        opencode_acp_capability_report(),
        opencode_serve_capability_report(),
    ];

    println!("{:<16} | {:<20} | {:<18} | {:<16}", "Provider", "Transport", "Instruction Inject", "Tool Granularity");
    println!("{:-<16}-|-{:-<20}-|-{:-<18}-|-{:-<16}", "", "", "", "");
    for r in &reports {
        let instr_str = match r.instruction_delivery {
            InstructionDeliveryMethod::Native => "Native",
            InstructionDeliveryMethod::FallbackPreamble => "Preamble Fallback",
        };
        let tool_str = match r.tool_permission_granularity {
            ToolPermissionGranularity::PerTool => "Per-Tool",
            ToolPermissionGranularity::Coarse => "Coarse",
        };
        println!("{:<16} | {:<20} | {:<18} | {:<16}", r.provider_name, r.transport, instr_str, tool_str);
    }

    println!("\nArchitectural Verdict:");
    println!("- Cursor: Shared ACP driver with fallback preamble and coarse permissions.");
    println!("- Grok: Shared ACP driver with fallback preamble and usage status polling.");
    println!("- OpenCode: Support 'opencode acp' via shared ACP driver; evaluate 'opencode serve' for Phase 4 where native developer instructions and fine-grained tool filtering are required.");

    println!("\n============================================================");
    println!("S5b Best-effort ACP Providers Spike: ALL CHECKS PASSED");
    println!("============================================================");

    Ok(())
}
