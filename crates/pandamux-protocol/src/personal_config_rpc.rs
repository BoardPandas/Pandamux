use pandamux_core::{ConfigurationRevision, MutationProposalId, SourceCommitSha};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalConfigHistoryParams {
    #[serde(default)]
    pub before_source_commit_sha: Option<SourceCommitSha>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalConfigHistoryEntry {
    pub source_commit_sha: SourceCommitSha,
    pub parent_source_commit_sha: Option<SourceCommitSha>,
    pub request_id: String,
    pub caller: String,
    pub accepted_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalConfigHistoryResult {
    pub head_source_commit_sha: SourceCommitSha,
    pub revision: ConfigurationRevision,
    pub entries: Vec<PersonalConfigHistoryEntry>,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalConfigDiffParams {
    pub from_source_commit_sha: SourceCommitSha,
    pub to_source_commit_sha: SourceCommitSha,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalConfigDiffResult {
    pub file_diff: serde_json::Value,
    pub behavioral_diff: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalConfigRevertParams {
    pub request_id: String,
    pub target_source_commit_sha: SourceCommitSha,
    pub expected_source_commit_sha: SourceCommitSha,
    pub expected_revision: ConfigurationRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalConfigRevertResult {
    pub proposal_id: MutationProposalId,
}
