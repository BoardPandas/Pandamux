use pandamux_core::ProjectId;
use serde::{Deserialize, Serialize};

/// An individual symbol entry for Hark speech-to-text vocabulary tuning.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarkSpellbookEntry {
    pub symbol: String,
    pub category: String,
    pub weight: f32,
}

/// Parameters for `hark.sync_spellbook` or `hark.syncSpellbook`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarkSpellbookSyncParams {
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    pub symbols: Vec<HarkSpellbookEntry>,
}

/// Result of `hark.sync_spellbook` or `hark.syncSpellbook`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarkSpellbookSyncResult {
    pub synchronized_count: usize,
    pub pipe_path: String,
    pub status: String,
}

/// Result of `hark.status`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarkStatusResult {
    pub connected: bool,
    pub pipe_path: String,
    pub spellbook_count: usize,
    pub daemon_available: bool,
}

/// Parameters for `hark.dictate_prompt` or `hark.dictatePrompt`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarkDictatePromptParams {
    pub text: String,
    #[serde(default)]
    pub is_final: bool,
    #[serde(default)]
    pub confidence: f32,
}

/// Result of `hark.dictate_prompt` or `hark.dictatePrompt`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarkDictatePromptResult {
    pub accepted: bool,
    pub text: String,
}
