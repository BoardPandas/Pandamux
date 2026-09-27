use anyhow::{bail, Result};
use crate::types::{
    AccessMode, InitializeResult, InstructionDeliveryMethod, JsonRpcResponse,
    PermissionRequestParams, ProviderCapabilityReport, ToolPermissionGranularity,
};

pub fn parse_cursor_initialize(json_str: &str) -> Result<InitializeResult> {
    let resp: JsonRpcResponse<InitializeResult> = serde_json::from_str(json_str)?;
    let result = resp
        .result
        .ok_or_else(|| anyhow::anyhow!("Cursor initialize response has no result payload"))?;

    if result.protocol_version != 1 {
        bail!(
            "Unsupported Cursor ACP protocol version: {}",
            result.protocol_version
        );
    }
    if !result.agent_info.name.contains("cursor") {
        bail!(
            "Unexpected agentInfo.name for Cursor: {}",
            result.agent_info.name
        );
    }
    if !result.auth_methods.iter().any(|m| m == "cursor_login") {
        bail!("Cursor initialize missing expected 'cursor_login' auth method");
    }

    Ok(result)
}

// Cursor ACP does not support top-level system or developer instruction fields
// in initialize or session/new. The driver injects the agent instructions as a
// structured preamble in the first user turn.
pub fn build_cursor_preamble(agent_role: &str, knowledge: &[&str], user_task: &str) -> String {
    let mut prompt = String::new();
    prompt.push_str("=== PANDAMUX AGENT ROLE INSTRUCTIONS ===\n");
    prompt.push_str(agent_role.trim());
    prompt.push('\n');

    if !knowledge.is_empty() {
        prompt.push_str("\n=== KNOWLEDGE CONTEXT ===\n");
        for k in knowledge {
            prompt.push_str("- ");
            prompt.push_str(k.trim());
            prompt.push('\n');
        }
    }

    prompt.push_str("\n=== USER TASK ===\n");
    prompt.push_str(user_task.trim());
    prompt
}

// Maps PandaMUX AccessMode to Cursor ACP permission responses (allow_once, allow_always, reject_once)
pub fn evaluate_cursor_permission(
    params: &PermissionRequestParams,
    mode: AccessMode,
) -> Result<&'static str> {
    let tool_name = &params.tool_call.tool;

    match mode {
        AccessMode::ReadOnly => {
            if tool_name == "edit_file"
                || tool_name == "write_file"
                || tool_name == "run_command"
                || tool_name == "bash"
            {
                // Mutation tools rejected in ReadOnly mode
                Ok("reject_once")
            } else {
                Ok("allow_once")
            }
        }
        AccessMode::Ask => {
            // In Ask mode, decisions route to user approval card. Default is allow_once if approved.
            Ok("allow_once")
        }
        AccessMode::AutoEdit => {
            if tool_name == "edit_file" || tool_name == "write_file" || tool_name == "read_file" {
                Ok("allow_always")
            } else if tool_name == "run_command" || tool_name == "bash" {
                // Command executions still require approval in AutoEdit
                Ok("allow_once")
            } else {
                Ok("allow_once")
            }
        }
        AccessMode::FullAccess => Ok("allow_always"),
    }
}

pub fn cursor_capability_report() -> ProviderCapabilityReport {
    ProviderCapabilityReport {
        provider_name: "cursor-agent".to_string(),
        transport: "stdio (ACP JSON-RPC 2.0 NDJSON)".to_string(),
        auth_probe_method: "initialize authMethods (cursor_login)".to_string(),
        instruction_delivery: InstructionDeliveryMethod::FallbackPreamble,
        tool_permission_granularity: ToolPermissionGranularity::Coarse,
        notes: "No native session instruction API; preamble injected on first turn and after compaction; coarse request_permission approvals".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cursor_initialize_fixture() {
        let fixture = include_str!("../fixtures/cursor/initialize_response.json");
        let init = parse_cursor_initialize(fixture).expect("Must parse cursor initialize");
        assert_eq!(init.protocol_version, 1);
        assert_eq!(init.agent_info.name, "cursor-agent");
        assert!(init.auth_methods.contains(&"cursor_login".to_string()));
    }

    #[test]
    fn test_cursor_preamble_formatting() {
        let preamble = build_cursor_preamble(
            "You are the Reviewer.",
            &["Never commit secrets", "Prefer const"],
            "Check the diff in crates/core",
        );
        assert!(preamble.contains("=== PANDAMUX AGENT ROLE INSTRUCTIONS ==="));
        assert!(preamble.contains("=== KNOWLEDGE CONTEXT ==="));
        assert!(preamble.contains("=== USER TASK ==="));
        assert!(preamble.contains("Check the diff in crates/core"));
    }

    #[test]
    fn test_cursor_permission_evaluation() {
        let req_json = include_str!("../fixtures/cursor/permission_request.json");
        let req: crate::types::JsonRpcRequest<PermissionRequestParams> =
            serde_json::from_str(req_json).expect("Parse permission request");

        // In ReadOnly mode, file edit must be rejected
        let decision_ro = evaluate_cursor_permission(&req.params, AccessMode::ReadOnly).unwrap();
        assert_eq!(decision_ro, "reject_once");

        // In AutoEdit mode, file edit is allowed always
        let decision_ae = evaluate_cursor_permission(&req.params, AccessMode::AutoEdit).unwrap();
        assert_eq!(decision_ae, "allow_always");
    }
}
