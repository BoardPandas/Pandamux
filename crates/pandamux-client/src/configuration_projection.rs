use pandamux_core::{
    ConfigurationAuditRecord, ConfigurationRevision, ContentSha256, InstructionCandidateRecord,
    InstructionsSourceRecord, MutationImpact, MutationProposal, MutationResolution, MutationTarget,
    ProjectLayerTrustRecord, ProjectRepositoryRecord,
};
use pandamux_protocol::{
    InstructionsDiffResult, PersonalConfigHistoryEntry, ProjectLayerDiffResult,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeMutationDiff {
    pub old_digest: String,
    pub new_digest: String,
    pub changed_sections: Vec<String>,
    pub values_redacted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationConfirmationProjection {
    pub proposal_id: String,
    pub caller_name: String,
    pub caller_role: String,
    pub tool_name: String,
    pub environment_id: String,
    pub target_label: String,
    pub expected_revision: u64,
    pub diff: SafeMutationDiff,
    pub affected_scopes: Vec<String>,
    pub access_changes: Vec<String>,
    pub full_access_changes: Vec<String>,
    pub budget_changes: Vec<String>,
    pub tracking_changes: Vec<String>,
    pub trust_changes: Vec<String>,
    pub issues: Vec<String>,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub status_label: String,
    pub can_resolve: bool,
}

impl MutationConfirmationProjection {
    pub fn from_proposal(proposal: &MutationProposal, now_ms: u64) -> Self {
        let resolution = if proposal.resolution == MutationResolution::Pending
            && now_ms >= proposal.expires_at_ms
        {
            MutationResolution::Expired
        } else {
            proposal.resolution
        };
        Self {
            proposal_id: proposal.id.as_str().to_string(),
            caller_name: proposal.caller.display_name.clone(),
            caller_role: readable_role(&proposal.caller.peer_role).to_string(),
            tool_name: proposal.tool_name.clone(),
            environment_id: proposal.environment_id.as_str().to_string(),
            target_label: target_label(&proposal.target),
            expected_revision: proposal.expected_revision.get(),
            diff: SafeMutationDiff {
                old_digest: proposal.old_digest.as_str().to_string(),
                new_digest: proposal.new_digest.as_str().to_string(),
                changed_sections: changed_sections(&proposal.old_json, &proposal.new_json),
                values_redacted: true,
            },
            affected_scopes: affected_scopes(&proposal.impact),
            access_changes: proposal.impact.access_summaries.iter().cloned().collect(),
            full_access_changes: proposal
                .impact
                .full_access_summaries
                .iter()
                .cloned()
                .collect(),
            budget_changes: proposal.impact.budget_summaries.iter().cloned().collect(),
            tracking_changes: proposal.impact.tracking_summaries.iter().cloned().collect(),
            trust_changes: proposal.impact.trust_summaries.iter().cloned().collect(),
            issues: proposal
                .impact
                .validation_summaries
                .iter()
                .cloned()
                .collect(),
            created_at_ms: proposal.created_at_ms,
            expires_at_ms: proposal.expires_at_ms,
            status_label: resolution_label(resolution).to_string(),
            can_resolve: resolution == MutationResolution::Pending,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrustedConfigurationProjection {
    pub source: Option<InstructionsSourceRecord>,
    pub candidate: Option<InstructionCandidateRecord>,
    pub repository: Option<ProjectRepositoryRecord>,
    pub project_trust: Option<ProjectLayerTrustRecord>,
    pub project_content_sha256: Option<ContentSha256>,
    pub project_visible_but_inactive: bool,
    pub candidate_diff: Option<InstructionsDiffResult>,
    pub project_diff: Option<ProjectLayerDiffResult>,
    pub configuration_history: Vec<ConfigurationAuditRecord>,
    pub personal_history: Vec<PersonalConfigHistoryEntry>,
    pub personal_head_source_commit_sha: Option<pandamux_core::SourceCommitSha>,
    pub personal_revision: Option<ConfigurationRevision>,
    pub confirmations: Vec<MutationConfirmationProjection>,
    pub status_message: Option<String>,
    pub is_loading: bool,
}

fn affected_scopes(impact: &MutationImpact) -> Vec<String> {
    let mut scopes = Vec::new();
    scopes.extend(
        impact
            .organization_ids
            .iter()
            .map(|id| format!("Organization {}", id.as_str())),
    );
    scopes.extend(
        impact
            .project_ids
            .iter()
            .map(|id| format!("Project {}", id.as_str())),
    );
    scopes.extend(
        impact
            .agent_ids
            .iter()
            .map(|id| format!("Agent {}", id.as_str())),
    );
    scopes.extend(
        impact
            .schedule_ids
            .iter()
            .map(|id| format!("Schedule {}", id.as_str())),
    );
    scopes.extend(
        impact
            .environment_ids
            .iter()
            .map(|id| format!("Environment {}", id.as_str())),
    );
    scopes
}

fn changed_sections(old: &Value, new: &Value) -> Vec<String> {
    match (old.as_object(), new.as_object()) {
        (Some(old), Some(new)) => {
            let mut keys = old.keys().chain(new.keys()).cloned().collect::<Vec<_>>();
            keys.sort();
            keys.dedup();
            keys.into_iter()
                .filter(|key| old.get(key) != new.get(key))
                .map(|key| safe_section_name(&key))
                .collect()
        }
        _ if old != new => vec!["Configuration value".to_string()],
        _ => Vec::new(),
    }
}

fn safe_section_name(key: &str) -> String {
    match key {
        "trustedConfiguration" => "Trusted configuration".to_string(),
        "tracking" => "Tracking reference".to_string(),
        "policy" => "Update policy".to_string(),
        "activeBundleSha" => "Active bundle identity".to_string(),
        "activeSourceCommitSha" => "Active source identity".to_string(),
        "trusted" => "Trust state".to_string(),
        "targetSourceCommitSha" => "Revert target identity".to_string(),
        "headSourceCommitSha" => "Personal source identity".to_string(),
        _ => "Configuration section".to_string(),
    }
}

fn target_label(target: &MutationTarget) -> String {
    match target {
        MutationTarget::Settings => "Trusted settings".to_string(),
        MutationTarget::InstructionsSource { source_id } => {
            format!("Instructions source {source_id}")
        }
        MutationTarget::ProjectLayerTrust { project_id } => {
            format!("Project trust {}", project_id.as_str())
        }
        MutationTarget::PersonalConfiguration => "Personal configuration".to_string(),
    }
}

fn readable_role(role: &str) -> &str {
    match role {
        "local_control" => "Authenticated local control",
        "mcp_adapter" => "Authenticated MCP adapter",
        "provider_child" => "Provider child",
        "node_proxy" => "Authenticated node proxy",
        "internal" => "PandaMUX server",
        _ => "Authenticated caller",
    }
}

fn resolution_label(resolution: MutationResolution) -> &'static str {
    match resolution {
        MutationResolution::Pending => "Pending local confirmation",
        MutationResolution::ApprovedApplied => "Approved and applied",
        MutationResolution::Rejected => "Rejected without changes",
        MutationResolution::Expired => "Expired without changes",
        MutationResolution::CancelledDisconnected => "Cancelled after caller disconnected",
        MutationResolution::SupersededRevision => "Not applied because configuration changed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{
        ConfigurationRevision, ContentSha256, EnvironmentId, MutationCaller, MutationProposalId,
    };
    use serde_json::json;

    fn proposal() -> MutationProposal {
        MutationProposal {
            id: MutationProposalId::new("proposal-safe"),
            request_id: "request-safe".to_string(),
            caller: MutationCaller {
                client_id: "caller-digest".to_string(),
                peer_role: "mcp_adapter".to_string(),
                display_name: "Local assistant".to_string(),
            },
            tool_name: "mcp.propose_agent_access".to_string(),
            environment_id: EnvironmentId::new("environment-local"),
            target: MutationTarget::Settings,
            expected_revision: ConfigurationRevision::new(7).expect("revision"),
            old_json: json!({
                "trustedConfiguration": { "token": "secret-old-sentinel" },
            }),
            new_json: json!({
                "trustedConfiguration": { "token": "secret-new-sentinel" },
            }),
            old_digest: ContentSha256::parse("a".repeat(64)).expect("digest"),
            new_digest: ContentSha256::parse("b".repeat(64)).expect("digest"),
            impact: MutationImpact {
                access_summaries: ["Enable agent".to_string()].into_iter().collect(),
                full_access_summaries: ["Grant FullAccess".to_string()].into_iter().collect(),
                budget_summaries: ["No budget change".to_string()].into_iter().collect(),
                validation_summaries: ["Review exact scope".to_string()].into_iter().collect(),
                ..MutationImpact::default()
            },
            created_at_ms: 100,
            expires_at_ms: 200,
            resolution: MutationResolution::Pending,
            audit_id: None,
        }
    }

    #[test]
    fn confirmation_redacts_values_and_retains_review_fields() {
        let projection = MutationConfirmationProjection::from_proposal(&proposal(), 150);
        let encoded = serde_json::to_string(&projection).expect("projection JSON");
        assert!(!encoded.contains("secret-old-sentinel"));
        assert!(!encoded.contains("secret-new-sentinel"));
        assert_eq!(projection.caller_name, "Local assistant");
        assert_eq!(projection.caller_role, "Authenticated MCP adapter");
        assert_eq!(projection.environment_id, "environment-local");
        assert_eq!(
            projection.diff.changed_sections,
            vec!["Trusted configuration"]
        );
        assert_eq!(projection.access_changes, vec!["Enable agent"]);
        assert_eq!(projection.full_access_changes, vec!["Grant FullAccess"]);
        assert_eq!(projection.budget_changes, vec!["No budget change"]);
        assert_eq!(projection.issues, vec!["Review exact scope"]);
        assert!(projection.can_resolve);
    }

    #[test]
    fn expired_confirmation_has_text_status_and_disables_actions() {
        let projection = MutationConfirmationProjection::from_proposal(&proposal(), 200);
        assert_eq!(projection.status_label, "Expired without changes");
        assert!(!projection.can_resolve);
    }
}
