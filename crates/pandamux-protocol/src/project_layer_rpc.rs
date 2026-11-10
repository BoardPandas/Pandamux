use pandamux_core::{
    ConfigurationRevision, ContentSha256, MutationProposalId, ProjectId, ProjectLayerTrustRecord,
    ProjectRepositoryRecord,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectLayerStatusParams {
    pub project_id: ProjectId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectLayerStatusResult {
    pub repository: ProjectRepositoryRecord,
    pub project_content_sha256: Option<ContentSha256>,
    pub trust: Option<ProjectLayerTrustRecord>,
    pub visible_but_inactive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectLayerDiffParams {
    pub project_id: ProjectId,
    pub project_content_sha256: ContentSha256,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectLayerDiffResult {
    pub project_content_sha256: ContentSha256,
    pub behavioral_diff: serde_json::Value,
    pub issues: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectLayerTrustParams {
    pub request_id: String,
    pub project_id: ProjectId,
    pub project_content_sha256: ContentSha256,
    pub expected_revision: ConfigurationRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectLayerRevokeParams {
    pub request_id: String,
    pub project_id: ProjectId,
    pub expected_revision: ConfigurationRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectLayerProposalResult {
    pub proposal_id: MutationProposalId,
}
