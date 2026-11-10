use crate::configuration::{ConfigurationRevision, ContentSha256};
use crate::ids::{
    AgentId, EnvironmentId, MutationAuditId, MutationProposalId, OrganizationId, ProjectId,
    ScheduleId,
};
use crate::settings::TrustedConfigurationSettings;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MutationCaller {
    pub client_id: String,
    pub peer_role: String,
    pub display_name: String,
}

/// The only settings payload allowed in a durable mutation proposal. This
/// deliberately excludes provider paths, environment overrides, and arbitrary
/// provider settings that may contain secrets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustedSettingsMutationState {
    pub trusted_configuration: TrustedConfigurationSettings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationTarget {
    Settings,
    InstructionsSource { source_id: String },
    ProjectLayerTrust { project_id: ProjectId },
    PersonalConfiguration,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MutationImpact {
    #[serde(default)]
    pub organization_ids: BTreeSet<OrganizationId>,
    #[serde(default)]
    pub project_ids: BTreeSet<ProjectId>,
    #[serde(default)]
    pub agent_ids: BTreeSet<AgentId>,
    #[serde(default)]
    pub schedule_ids: BTreeSet<ScheduleId>,
    #[serde(default)]
    pub environment_ids: BTreeSet<EnvironmentId>,
    #[serde(default)]
    pub access_summaries: BTreeSet<String>,
    #[serde(default)]
    pub full_access_summaries: BTreeSet<String>,
    #[serde(default)]
    pub budget_summaries: BTreeSet<String>,
    #[serde(default)]
    pub tracking_summaries: BTreeSet<String>,
    #[serde(default)]
    pub trust_summaries: BTreeSet<String>,
    #[serde(default)]
    pub validation_summaries: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationResolution {
    Pending,
    ApprovedApplied,
    Rejected,
    Expired,
    CancelledDisconnected,
    SupersededRevision,
}

impl MutationResolution {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MutationProposal {
    pub id: MutationProposalId,
    pub request_id: String,
    pub caller: MutationCaller,
    pub tool_name: String,
    pub environment_id: EnvironmentId,
    pub target: MutationTarget,
    pub expected_revision: ConfigurationRevision,
    pub old_json: Value,
    pub new_json: Value,
    pub old_digest: ContentSha256,
    pub new_digest: ContentSha256,
    pub impact: MutationImpact,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub resolution: MutationResolution,
    pub audit_id: Option<MutationAuditId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MutationAuditRecord {
    pub id: MutationAuditId,
    pub proposal_id: MutationProposalId,
    pub target: MutationTarget,
    pub old_digest: ContentSha256,
    pub new_digest: ContentSha256,
    pub resolver: MutationCaller,
    pub applied_revision: ConfigurationRevision,
    pub applied_at_ms: u64,
}
