use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest<T> {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse<T> {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<T>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcNotification<T> {
    pub jsonrpc: String,
    pub method: String,
    pub params: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilities {
    #[serde(rename = "loadSession", default)]
    pub load_session: bool,
    #[serde(rename = "sessionResume", default)]
    pub session_resume: bool,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub tools: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub name: String,
    pub version: String,
    pub capabilities: Option<AgentCapabilities>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(rename = "agentInfo")]
    pub agent_info: AgentInfo,
    #[serde(rename = "authMethods", default)]
    pub auth_methods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    #[serde(rename = "currentValue")]
    pub current_value: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionNewResult {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "configOptions", default)]
    pub config_options: Vec<ConfigOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallDetail {
    pub id: String,
    pub tool: String,
    pub parameters: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequestParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "permissionId")]
    pub permission_id: String,
    #[serde(rename = "toolCall")]
    pub tool_call: ToolCallDetail,
    pub reason: Option<String>,
    pub options: Vec<PermissionOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AccessMode {
    ReadOnly,
    Ask,
    AutoEdit,
    FullAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstructionDeliveryMethod {
    Native,
    FallbackPreamble,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolPermissionGranularity {
    Coarse,
    PerTool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilityReport {
    pub provider_name: String,
    pub transport: String,
    pub auth_probe_method: String,
    pub instruction_delivery: InstructionDeliveryMethod,
    pub tool_permission_granularity: ToolPermissionGranularity,
    pub notes: String,
}
