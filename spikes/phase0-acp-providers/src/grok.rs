use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use crate::types::{
    InitializeResult, InstructionDeliveryMethod, JsonRpcResponse,
    ProviderCapabilityReport, ToolPermissionGranularity,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokTokenBudget {
    pub used_tokens: u64,
    pub limit_tokens: u64,
    pub used_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokRateLimits {
    pub requests_per_minute_limit: u32,
    pub requests_remaining: u32,
    pub window_duration_seconds: u32,
    pub resets_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokUsageLimits {
    pub account_tier: String,
    pub monthly_spend_usd: f64,
    pub monthly_limit_usd: f64,
    pub token_budget: GrokTokenBudget,
    pub rate_limits: GrokRateLimits,
}

pub fn parse_grok_initialize(json_str: &str) -> Result<InitializeResult> {
    let resp: JsonRpcResponse<InitializeResult> = serde_json::from_str(json_str)?;
    let result = resp
        .result
        .ok_or_else(|| anyhow::anyhow!("Grok initialize response has no result payload"))?;

    if result.protocol_version != 1 {
        bail!(
            "Unsupported Grok ACP protocol version: {}",
            result.protocol_version
        );
    }
    if !result.agent_info.name.contains("grok") {
        bail!(
            "Unexpected agentInfo.name for Grok: {}",
            result.agent_info.name
        );
    }

    Ok(result)
}

pub fn build_grok_preamble(agent_role: &str, user_task: &str) -> String {
    let mut prompt = String::new();
    prompt.push_str("=== PANDAMUX AGENT ROLE INSTRUCTIONS ===\n");
    prompt.push_str(agent_role.trim());
    prompt.push_str("\n\n=== USER TASK ===\n");
    prompt.push_str(user_task.trim());
    prompt
}

pub fn parse_grok_usage_limits(json_str: &str) -> Result<GrokUsageLimits> {
    let limits: GrokUsageLimits = serde_json::from_str(json_str)?;
    if limits.monthly_limit_usd <= 0.0 {
        bail!("Invalid monthly limit in Grok usage report");
    }
    Ok(limits)
}

pub fn grok_capability_report() -> ProviderCapabilityReport {
    ProviderCapabilityReport {
        provider_name: "grok-acp".to_string(),
        transport: "stdio (ACP JSON-RPC 2.0 NDJSON)".to_string(),
        auth_probe_method: "account status / initialize authMethods".to_string(),
        instruction_delivery: InstructionDeliveryMethod::FallbackPreamble,
        tool_permission_granularity: ToolPermissionGranularity::Coarse,
        notes: "Grok CLI entry point via ACP stdio; fallback preamble for instructions; account rate limits and budget exposed in usage status".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_grok_initialize_fixture() {
        let fixture = include_str!("../fixtures/grok/initialize_response.json");
        let init = parse_grok_initialize(fixture).expect("Must parse grok initialize");
        assert_eq!(init.protocol_version, 1);
        assert_eq!(init.agent_info.name, "grok-acp");
        assert!(init.auth_methods.contains(&"api_key".to_string()));
    }

    #[test]
    fn test_parse_grok_usage_limits_fixture() {
        let fixture = include_str!("../fixtures/grok/usage_limits.json");
        let limits = parse_grok_usage_limits(fixture).expect("Must parse grok usage limits");
        assert_eq!(limits.account_tier, "grok_developer_tier_2");
        assert_eq!(limits.rate_limits.requests_remaining, 58);
        assert_eq!(limits.token_budget.used_percent, 14.82);
    }
}
