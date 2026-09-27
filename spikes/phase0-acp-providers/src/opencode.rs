use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use crate::types::{
    InitializeResult, InstructionDeliveryMethod, JsonRpcResponse,
    ProviderCapabilityReport, ToolPermissionGranularity,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeToolPolicy {
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeServeCreateRequest {
    pub cwd: String,
    pub model: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub tool_policy: Option<OpenCodeToolPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeServeCreateResponse {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub status: String,
    pub model: String,
    pub native_system_prompt_applied: bool,
    pub enforced_tool_policy: Option<OpenCodeToolPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
}

pub fn parse_opencode_acp_initialize(json_str: &str) -> Result<InitializeResult> {
    let resp: JsonRpcResponse<InitializeResult> = serde_json::from_str(json_str)?;
    let result = resp
        .result
        .ok_or_else(|| anyhow::anyhow!("OpenCode ACP initialize response has no result payload"))?;

    if result.protocol_version != 1 {
        bail!(
            "Unsupported OpenCode ACP protocol version: {}",
            result.protocol_version
        );
    }
    if !result.agent_info.name.contains("opencode") {
        bail!(
            "Unexpected agentInfo.name for OpenCode ACP: {}",
            result.agent_info.name
        );
    }
    Ok(result)
}

pub fn parse_opencode_serve_create_request(json_str: &str) -> Result<OpenCodeServeCreateRequest> {
    let req: OpenCodeServeCreateRequest = serde_json::from_str(json_str)?;
    if req.model.is_empty() {
        bail!("Model name cannot be empty in OpenCode serve request");
    }
    Ok(req)
}

pub fn parse_opencode_serve_create_response(json_str: &str) -> Result<OpenCodeServeCreateResponse> {
    let resp: OpenCodeServeCreateResponse = serde_json::from_str(json_str)?;
    if resp.session_id.is_empty() {
        bail!("Session ID cannot be empty in OpenCode serve response");
    }
    Ok(resp)
}

// Parses an SSE event stream from opencode serve /event
pub fn parse_opencode_sse_event_stream(sse_str: &str) -> Result<Vec<SseEvent>> {
    let mut events = Vec::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    for line in sse_str.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !current_event.is_empty() || !current_data.is_empty() {
                events.push(SseEvent {
                    event_type: if current_event.is_empty() { "message".to_string() } else { current_event.clone() },
                    data: current_data.clone(),
                });
                current_event.clear();
                current_data.clear();
            }
        } else if let Some(ev) = trimmed.strip_prefix("event:") {
            current_event = ev.trim().to_string();
        } else if let Some(d) = trimmed.strip_prefix("data:") {
            if !current_data.is_empty() {
                current_data.push('\n');
            }
            current_data.push_str(d.trim());
        }
    }

    if !current_event.is_empty() || !current_data.is_empty() {
        events.push(SseEvent {
            event_type: if current_event.is_empty() { "message".to_string() } else { current_event },
            data: current_data,
        });
    }

    Ok(events)
}

pub fn opencode_acp_capability_report() -> ProviderCapabilityReport {
    ProviderCapabilityReport {
        provider_name: "opencode-acp".to_string(),
        transport: "stdio (ACP JSON-RPC 2.0 NDJSON)".to_string(),
        auth_probe_method: "initialize authMethods (local_config)".to_string(),
        instruction_delivery: InstructionDeliveryMethod::FallbackPreamble,
        tool_permission_granularity: ToolPermissionGranularity::Coarse,
        notes: "OpenCode ACP CLI entry point; stdio NDJSON; fallback preamble for instructions; coarse request_permission approvals".to_string(),
    }
}

pub fn opencode_serve_capability_report() -> ProviderCapabilityReport {
    ProviderCapabilityReport {
        provider_name: "opencode-serve".to_string(),
        transport: "HTTP REST + SSE (/event) on 127.0.0.1".to_string(),
        auth_probe_method: "OPENCODE_SERVER_PASSWORD bearer authentication".to_string(),
        instruction_delivery: InstructionDeliveryMethod::Native,
        tool_permission_granularity: ToolPermissionGranularity::PerTool,
        notes: "OpenCode HTTP serve mode; native system_prompt in POST /session; per-tool allow/deny filters; SSE streaming".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_opencode_acp_initialize() {
        let fixture = include_str!("../fixtures/opencode/acp_initialize_response.json");
        let init = parse_opencode_acp_initialize(fixture).expect("Parse opencode ACP initialize");
        assert_eq!(init.protocol_version, 1);
        assert_eq!(init.agent_info.name, "opencode-acp");
    }

    #[test]
    fn test_parse_opencode_serve_request_and_response() {
        let req_fixture = include_str!("../fixtures/opencode/serve_session_create_request.json");
        let req = parse_opencode_serve_create_request(req_fixture).expect("Parse serve request");
        assert!(req.system_prompt.is_some());
        let policy = req.tool_policy.unwrap();
        assert!(policy.allowed_tools.contains(&"read_file".to_string()));
        assert!(policy.disallowed_tools.contains(&"delete_repository".to_string()));

        let resp_fixture = include_str!("../fixtures/opencode/serve_session_create_response.json");
        let resp = parse_opencode_serve_create_response(resp_fixture).expect("Parse serve response");
        assert!(resp.native_system_prompt_applied);
        assert_eq!(resp.status, "active");
    }

    #[test]
    fn test_parse_opencode_sse_stream() {
        let sse_fixture = include_str!("../fixtures/opencode/serve_events_stream.sse");
        let events = parse_opencode_sse_event_stream(sse_fixture).expect("Parse SSE stream");
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].event_type, "turn_started");
        assert_eq!(events[1].event_type, "content_delta");
        assert_eq!(events[2].event_type, "tool_invoked");
        assert_eq!(events[3].event_type, "turn_completed");
    }
}
