use pandamux_core::{
    BundleActivationRecord, BundleSha, ConfigurationAuditRecord, ConfigurationRevision,
    ConfigurationUpdateSafety, ContentSha256, InstructionCandidateId, InstructionCandidateRecord,
    InstructionSourceId, InstructionsSourceRecord, InstructionsUpdatePolicy, MutationProposalId,
    SourceCommitSha, TrackingRef,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsStatusParams {
    #[serde(default)]
    pub source_id: Option<InstructionSourceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsStatusResult {
    pub source: Option<InstructionsSourceRecord>,
    pub candidate: Option<InstructionCandidateRecord>,
    pub active_bundle_sha: Option<BundleSha>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsCheckParams {
    pub request_id: String,
    pub source_id: InstructionSourceId,
    pub expected_revision: ConfigurationRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsCheckResult {
    pub checked_at_ms: u64,
    pub unchanged: bool,
    pub candidate: Option<InstructionCandidateRecord>,
    pub safe_error_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsDiffParams {
    pub candidate_id: InstructionCandidateId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsDiffResult {
    pub candidate_id: InstructionCandidateId,
    pub observed_source_commit_sha: SourceCommitSha,
    pub diff_digest: ContentSha256,
    pub safety: ConfigurationUpdateSafety,
    pub file_diff: serde_json::Value,
    pub behavioral_diff: serde_json::Value,
    pub issues: Vec<serde_json::Value>,
    pub activatable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsApproveParams {
    pub request_id: String,
    pub project_id: pandamux_core::ProjectId,
    pub candidate_id: InstructionCandidateId,
    pub observed_source_commit_sha: SourceCommitSha,
    pub diff_digest: ContentSha256,
    pub expected_revision: ConfigurationRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsApproveResult {
    pub activation: BundleActivationRecord,
    pub source_revision: ConfigurationRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsSetTrackingParams {
    pub request_id: String,
    pub source_id: InstructionSourceId,
    pub tracking: TrackingRef,
    pub expected_revision: ConfigurationRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsSetPolicyParams {
    pub request_id: String,
    pub source_id: InstructionSourceId,
    pub policy: InstructionsUpdatePolicy,
    pub expected_revision: ConfigurationRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsProposalResult {
    pub proposal_id: MutationProposalId,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsHistoryParams {
    #[serde(default)]
    pub before_at_ms: Option<u64>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsHistoryResult {
    pub entries: Vec<ConfigurationAuditRecord>,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsRevertParams {
    pub request_id: String,
    pub source_id: InstructionSourceId,
    pub bundle_sha: BundleSha,
    pub expected_revision: ConfigurationRevision,
}
