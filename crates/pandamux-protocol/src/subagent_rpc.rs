use pandamux_core::ThreadId;
use serde::{Deserialize, Serialize};

/// Parameters for `subagent.tree`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTreeParams {
    pub thread_id: ThreadId,
}

/// A node in the provider sub-agent tree representing a nested or top-level sub-agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTreeItem {
    pub id: String,
    #[serde(default)]
    pub parent_sub_agent_id: Option<String>,
    #[serde(default)]
    pub parent_item_id: Option<String>,
    pub title: String,
    pub agent_type: String,
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub latest_activity: Option<String>,
    pub tokens: u64,
    pub tool_calls: u32,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub summary: Option<String>,
    pub finished: bool,
    #[serde(default)]
    pub children: Vec<SubAgentTreeItem>,
}

/// Result returned by `subagent.tree`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTreeResult {
    pub thread_id: ThreadId,
    pub root_agents: Vec<SubAgentTreeItem>,
    pub total_count: usize,
}
