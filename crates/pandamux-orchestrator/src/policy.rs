use crate::diagnostics::{
    ConfigurationIssue, ConfigurationIssueCode, ConfigurationStage, RelativeConfigPath, SafeText,
    issue,
};
use pandamux_core::{
    AccessMode, AgentDefinition, ApprovalId, ApprovalRequirement, OrchestrationMode,
    OrganizationId, ProjectId, RunApprovalKind, RunApprovalStatus, RunId, RunTaskId, TierMapping,
    ValueLayer,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalSafetyClass {
    Ordinary,
    AccessWidening,
    CrossOrganization,
    HumanAgentEdit,
    FullAccess,
    McpMutation,
    ProjectLayerTrust,
    ProjectIdentityChange,
}

impl ApprovalSafetyClass {
    const fn allows_automatic_authorization(self) -> bool {
        matches!(self, Self::Ordinary)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalAction {
    pub approval_id: ApprovalId,
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub kind: RunApprovalKind,
    pub policy_revision: String,
    pub canonical_action: Value,
    pub safe_summary: SafeText,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    pub expires_at_ms: u64,
    pub safety_class: ApprovalSafetyClass,
    pub trusted_auto_permitted: bool,
    #[serde(default)]
    pub hard_denial_codes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionIdentity {
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub kind: RunApprovalKind,
    pub policy_revision: String,
    pub action_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApprovalReconciliation {
    Authorized {
        identity: ApprovalDecisionIdentity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval_id: Option<ApprovalId>,
    },
    Pending {
        approval: ApprovalRequirement,
    },
    Denied {
        identity: ApprovalDecisionIdentity,
        reason_codes: Vec<String>,
    },
}

pub fn reconcile_approval(
    mode: OrchestrationMode,
    action: &ApprovalAction,
    existing: &BTreeMap<ApprovalId, ApprovalRequirement>,
    now_ms: u64,
) -> ApprovalReconciliation {
    let identity = approval_identity(action);
    let mut hard_denial_codes = action.hard_denial_codes.clone();
    hard_denial_codes.sort();
    hard_denial_codes.dedup();
    if !hard_denial_codes.is_empty() {
        return ApprovalReconciliation::Denied {
            identity,
            reason_codes: hard_denial_codes,
        };
    }

    if existing
        .get(&action.approval_id)
        .is_some_and(|approval| !approval_matches_identity(approval, &identity))
    {
        return ApprovalReconciliation::Denied {
            identity,
            reason_codes: vec!["approval_identity_conflict".to_string()],
        };
    }

    let matching = existing
        .values()
        .filter(|approval| approval_matches_identity(approval, &identity))
        .collect::<Vec<_>>();
    if matching.len() > 1 {
        return ApprovalReconciliation::Denied {
            identity,
            reason_codes: vec!["approval_identity_conflict".to_string()],
        };
    }
    if let Some(approval) = matching.first() {
        if approval.expires_at_ms <= now_ms || approval.status == RunApprovalStatus::Expired {
            return ApprovalReconciliation::Denied {
                identity,
                reason_codes: vec!["approval_expired".to_string()],
            };
        }
        return match approval.status {
            RunApprovalStatus::Approved => ApprovalReconciliation::Authorized {
                identity,
                approval_id: Some(approval.id.clone()),
            },
            RunApprovalStatus::Pending => ApprovalReconciliation::Pending {
                approval: (*approval).clone(),
            },
            RunApprovalStatus::Rejected => ApprovalReconciliation::Denied {
                identity,
                reason_codes: vec!["approval_rejected".to_string()],
            },
            RunApprovalStatus::Expired => unreachable!("handled above"),
        };
    }

    let mut reason_codes = action.reason_codes.clone();
    reason_codes.sort();
    reason_codes.dedup();
    if mode == OrchestrationMode::Auto
        && action.trusted_auto_permitted
        && action.safety_class.allows_automatic_authorization()
        && reason_codes.is_empty()
    {
        return ApprovalReconciliation::Authorized {
            identity,
            approval_id: None,
        };
    }

    ApprovalReconciliation::Pending {
        approval: ApprovalRequirement {
            id: action.approval_id.clone(),
            run_id: action.run_id.clone(),
            task_id: action.task_id.clone(),
            kind: action.kind,
            policy_revision: action.policy_revision.clone(),
            action_digest: identity.action_digest,
            safe_summary: action.safe_summary.as_str().to_string(),
            reason_codes,
            expires_at_ms: action.expires_at_ms,
            status: RunApprovalStatus::Pending,
            decision: None,
        },
    }
}

pub fn approval_identity(action: &ApprovalAction) -> ApprovalDecisionIdentity {
    ApprovalDecisionIdentity {
        run_id: action.run_id.clone(),
        task_id: action.task_id.clone(),
        kind: action.kind,
        policy_revision: action.policy_revision.clone(),
        action_digest: canonical_action_digest(&action.canonical_action),
    }
}

pub fn canonical_action_digest(action: &Value) -> String {
    let canonical = canonicalize_json(action);
    let bytes = serde_json::to_vec(&canonical).expect("JSON values are serializable");
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn approval_matches_identity(
    approval: &ApprovalRequirement,
    identity: &ApprovalDecisionIdentity,
) -> bool {
    approval.run_id == identity.run_id
        && approval.task_id == identity.task_id
        && approval.kind == identity.kind
        && approval.policy_revision == identity.policy_revision
        && approval.action_digest == identity.action_digest
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize_json(value)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
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
