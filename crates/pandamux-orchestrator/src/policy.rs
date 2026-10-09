use crate::diagnostics::{
    ConfigurationIssue, ConfigurationIssueCode, ConfigurationStage, RelativeConfigPath, issue,
};
use pandamux_core::{
    AccessMode, AgentDefinition, OrganizationId, ProjectId, TierMapping, ValueLayer,
};
use serde::{Deserialize, Serialize};

/// Server-issued proof that one exact source layer may participate in compilation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerGrant {
    pub layer: ValueLayer,
    pub trusted: bool,
    pub content_sha256: String,
    pub organization_id: OrganizationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

/// Trusted settings are supplied by the server and never parsed from repository content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedBundleSettings {
    pub revision: String,
    pub tier_mapping: TierMapping,
}

pub(crate) fn validate_grant_scope(
    grant: &LayerGrant,
    project_id: &ProjectId,
    organization_id: &OrganizationId,
) -> Vec<ConfigurationIssue> {
    let path = RelativeConfigPath::invalid();
    let mut issues = Vec::new();
    if !grant.trusted {
        issues.push(issue(
            ConfigurationIssueCode::LayerNotTrusted,
            path.clone(),
            None,
            ConfigurationStage::Policy,
            "A source layer has no active server-issued trust grant",
            "Approve the exact layer content hash before compiling it",
            None,
        ));
    }
    if &grant.organization_id != organization_id {
        issues.push(issue(
            ConfigurationIssueCode::PolicyCrossOrganization,
            path.clone(),
            None,
            ConfigurationStage::Policy,
            "A layer grant belongs to a different organization",
            "Use a grant issued for the bundle organization",
            None,
        ));
    }
    match grant.layer {
        ValueLayer::Instructions if grant.project_id.is_some() => issues.push(issue(
            ConfigurationIssueCode::LayerScopeViolation,
            path,
            None,
            ConfigurationStage::Policy,
            "The shared instructions layer cannot claim a project scope",
            "Issue an organization-scoped instructions grant",
            None,
        )),
        ValueLayer::Project if grant.project_id.as_ref() != Some(project_id) => issues.push(issue(
            ConfigurationIssueCode::LayerScopeViolation,
            path,
            None,
            ConfigurationStage::Policy,
            "The project layer grant does not match the effective project",
            "Approve the exact source hash for the effective project",
            None,
        )),
        ValueLayer::Personal => {
            if grant
                .project_id
                .as_ref()
                .is_some_and(|value| value != project_id)
            {
                issues.push(issue(
                    ConfigurationIssueCode::LayerScopeViolation,
                    path,
                    None,
                    ConfigurationStage::Policy,
                    "The personal layer grant is scoped to a different project",
                    "Use an organization-wide personal grant or one scoped to the effective project",
                    None,
                ));
            }
        }
        ValueLayer::Settings => issues.push(issue(
            ConfigurationIssueCode::LayerScopeViolation,
            path,
            None,
            ConfigurationStage::Policy,
            "Settings cannot be supplied as an authored source layer",
            "Pass settings through the trusted settings snapshot",
            None,
        )),
        ValueLayer::Instructions | ValueLayer::Project => {}
    }
    issues
}

pub(crate) fn validate_agent_override(
    base: &AgentDefinition,
    candidate: &AgentDefinition,
    candidate_path: &RelativeConfigPath,
) -> Vec<ConfigurationIssue> {
    let mut issues = Vec::new();
    if candidate.access_mode.security_rank() > base.access_mode.security_rank() {
        issues.push(policy_widening(
            candidate_path,
            "/accessMode",
            "An agent override widens the inherited access ceiling",
        ));
    }
    if candidate.access_mode == AccessMode::FullAccess {
        issues.push(full_access_issue(candidate_path));
    }
    if !candidate.tools.allow.is_subset(&base.tools.allow) {
        issues.push(policy_widening(
            candidate_path,
            "/tools/allow",
            "An agent override adds tools outside the inherited allow set",
        ));
    }
    if !candidate.tools.deny.is_superset(&base.tools.deny) {
        issues.push(policy_widening(
            candidate_path,
            "/tools/deny",
            "An agent override removes an inherited tool denial",
        ));
    }
    if !environment_is_subset(&candidate.environments, &base.environments) {
        issues.push(policy_widening(
            candidate_path,
            "/environments",
            "An agent override widens the inherited environment set",
        ));
    }
    if !candidate
        .required_capabilities
        .is_superset(&base.required_capabilities)
    {
        issues.push(policy_widening(
            candidate_path,
            "/requiredCapabilities",
            "An agent override removes an inherited provider capability requirement",
        ));
    }
    issues
}

pub(crate) fn full_access_issue(path: &RelativeConfigPath) -> ConfigurationIssue {
    issue(
        ConfigurationIssueCode::PolicyFullAccessFromRepo,
        path.clone(),
        Some("/accessMode"),
        ConfigurationStage::Policy,
        "Repository-authored agents cannot request FullAccess",
        "Use a narrower authored access mode; FullAccess opt-in is local trusted state",
        None,
    )
}

fn policy_widening(
    path: &RelativeConfigPath,
    field: &str,
    explanation: &str,
) -> ConfigurationIssue {
    issue(
        ConfigurationIssueCode::PolicyAccessWidening,
        path.clone(),
        Some(field),
        ConfigurationStage::Policy,
        explanation,
        "Keep the override at or below the inherited authority",
        None,
    )
}

fn environment_is_subset(
    candidate: &std::collections::BTreeSet<String>,
    base: &std::collections::BTreeSet<String>,
) -> bool {
    if base.contains("any") {
        return true;
    }
    !candidate.contains("any") && candidate.is_subset(base)
}
