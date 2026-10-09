use crate::bundle::{
    ParsedLayerDocument, source_layer_sha256, validate_document_inventory, validate_references,
};
use crate::diagnostics::{ConfigurationIssue, DiagnosticSet};
use crate::policy::{canonical_json_sha256, full_access_issue};
use crate::schema::v1::{AuthoredAccessMode, ClassRuleV1};
use crate::schema::{
    ParsedDocumentV1, SourceDocument, parse_source_document, validate_source_set_bounds,
};
use pandamux_core::{
    ConfigurationRevision, ConfigurationUpdateSafety, ContentSha256, SourceCommitSha, TrackingRef,
    ValueLayer, ValueOrigin,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationDependencySnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions_source_commit_sha: Option<SourceCommitSha>,
    pub canonical_remote_identity: String,
    pub tracking: TrackingRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_content_sha256: Option<ContentSha256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_source_commit_sha: Option<SourceCommitSha>,
    pub settings_revision: ConfigurationRevision,
    pub trust_digest: ContentSha256,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationCandidateInspectionInput {
    pub baseline_documents: Vec<SourceDocument>,
    pub candidate_documents: Vec<SourceDocument>,
    pub baseline_dependencies: ConfigurationDependencySnapshot,
    pub candidate_dependencies: ConfigurationDependencySnapshot,
    pub scheduled_agent_ids: BTreeSet<String>,
    pub history_rewritten: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationFileChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationFileChange {
    pub path: String,
    pub kind: ConfigurationFileChangeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_sha256: Option<ContentSha256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_sha256: Option<ContentSha256>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BehavioralChangeSet {
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
    #[serde(default)]
    pub modified: Vec<String>,
}

impl BehavioralChangeSet {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationBehavioralDiff {
    pub agents: BehavioralChangeSet,
    pub access: BehavioralChangeSet,
    pub tools: BehavioralChangeSet,
    pub environments: BehavioralChangeSet,
    pub capabilities: BehavioralChangeSet,
    pub full_access: BehavioralChangeSet,
    pub prompts: BehavioralChangeSet,
    pub knowledge: BehavioralChangeSet,
    pub classes: BehavioralChangeSet,
    pub routing: BehavioralChangeSet,
    pub verification: BehavioralChangeSet,
    pub scheduled_agents: BehavioralChangeSet,
    pub source_identities: BehavioralChangeSet,
    pub trust: BehavioralChangeSet,
    pub settings: BehavioralChangeSet,
    pub origins: BehavioralChangeSet,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedConfigurationIssue {
    pub layer: ValueLayer,
    pub origin: ValueOrigin,
    #[serde(flatten)]
    pub issue: ConfigurationIssue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationCandidateInspection {
    pub baseline_content_sha256: ContentSha256,
    pub candidate_content_sha256: ContentSha256,
    pub file_diff: Vec<ConfigurationFileChange>,
    pub behavioral_diff: ConfigurationBehavioralDiff,
    pub issues: Vec<InspectedConfigurationIssue>,
    pub safety: ConfigurationUpdateSafety,
    pub local_approval_required: bool,
    pub activatable: bool,
    pub diff_digest: ContentSha256,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct BehaviorSnapshot {
    agents: BTreeMap<String, AgentBehavior>,
    prompts: BTreeMap<String, ContentSha256>,
    knowledge: BTreeMap<String, ContentSha256>,
    classes: BTreeMap<String, String>,
    routing: BTreeMap<String, String>,
    verification: BTreeMap<String, String>,
    origins: BTreeMap<String, ValueOrigin>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentBehavior {
    canonical: String,
    access: AuthoredAccessMode,
    tools: String,
    tool_allow: BTreeSet<String>,
    tool_deny: BTreeSet<String>,
    environments: BTreeSet<String>,
    capabilities: BTreeSet<String>,
    full_access: bool,
    verification: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectionDigestInput<'a> {
    baseline_content_sha256: &'a ContentSha256,
    candidate_content_sha256: &'a ContentSha256,
    baseline_dependencies: &'a ConfigurationDependencySnapshot,
    candidate_dependencies: &'a ConfigurationDependencySnapshot,
    scheduled_agent_ids: &'a BTreeSet<String>,
    history_rewritten: bool,
    file_diff: &'a [ConfigurationFileChange],
    behavioral_diff: &'a ConfigurationBehavioralDiff,
    issues: &'a [InspectedConfigurationIssue],
    safety: ConfigurationUpdateSafety,
    activatable: bool,
}

pub fn inspect_configuration_candidate(
    input: &ConfigurationCandidateInspectionInput,
) -> ConfigurationCandidateInspection {
    let baseline_content_sha256 = source_layer_sha256(&input.baseline_documents);
    let candidate_content_sha256 = source_layer_sha256(&input.candidate_documents);
    let file_diff = file_diff(&input.baseline_documents, &input.candidate_documents);
    let baseline = inspect_documents(&input.baseline_documents, false);
    let candidate = inspect_documents(&input.candidate_documents, true);
    let mut behavioral_diff = behavior_diff(&baseline.snapshot, &candidate.snapshot);
    dependency_diff(
        &input.baseline_dependencies,
        &input.candidate_dependencies,
        &mut behavioral_diff,
    );
    let changed_agents = changed_keys(&behavioral_diff.agents);
    let scheduled = changed_agents
        .intersection(&input.scheduled_agent_ids)
        .cloned()
        .collect::<Vec<_>>();
    behavioral_diff.scheduled_agents.modified = scheduled;

    let access_widened = access_widened(&baseline.snapshot, &candidate.snapshot);
    let safety = if input.history_rewritten {
        ConfigurationUpdateSafety::HistoryRewrite
    } else if input.baseline_dependencies.canonical_remote_identity
        != input.candidate_dependencies.canonical_remote_identity
    {
        ConfigurationUpdateSafety::RemoteIdentityChange
    } else if input.baseline_dependencies.tracking != input.candidate_dependencies.tracking {
        ConfigurationUpdateSafety::TrackingChange
    } else if !behavioral_diff.scheduled_agents.is_empty() {
        ConfigurationUpdateSafety::ScheduledAgentChange
    } else if access_widened {
        ConfigurationUpdateSafety::AccessWidening
    } else {
        ConfigurationUpdateSafety::Ordinary
    };
    let issues = candidate.issues;
    let activatable = issues.is_empty();
    let local_approval_required = safety.requires_local_approval();
    let digest = InspectionDigestInput {
        baseline_content_sha256: &baseline_content_sha256,
        candidate_content_sha256: &candidate_content_sha256,
        baseline_dependencies: &input.baseline_dependencies,
        candidate_dependencies: &input.candidate_dependencies,
        scheduled_agent_ids: &input.scheduled_agent_ids,
        history_rewritten: input.history_rewritten,
        file_diff: &file_diff,
        behavioral_diff: &behavioral_diff,
        issues: &issues,
        safety,
        activatable,
    };
    let diff_digest = ContentSha256::parse(canonical_json_sha256(&digest))
        .expect("canonical inspection digest is SHA-256");
    ConfigurationCandidateInspection {
        baseline_content_sha256,
        candidate_content_sha256,
        file_diff,
        behavioral_diff,
        issues,
        safety,
        local_approval_required,
        activatable,
        diff_digest,
    }
}

struct DocumentInspection {
    snapshot: BehaviorSnapshot,
    issues: Vec<InspectedConfigurationIssue>,
}

fn inspect_documents(documents: &[SourceDocument], validate_set: bool) -> DocumentInspection {
    let mut ordered = documents.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.logical_path()
            .cmp(right.logical_path())
            .then_with(|| left.bytes().cmp(right.bytes()))
    });
    let mut parsed = Vec::new();
    let mut issues = Vec::new();
    if validate_set && let Err(error) = validate_source_set_bounds(documents) {
        issues.extend(error.into_issues());
    }
    for source in ordered {
        match parse_source_document(source) {
            Ok(document) => parsed.push(ParsedLayerDocument {
                layer: ValueLayer::Instructions,
                parsed: document,
            }),
            Err(error) if validate_set => issues.extend(error.into_issues()),
            Err(_) => {}
        }
    }
    if validate_set {
        collect_result_issues(validate_document_inventory(&parsed), &mut issues);
        collect_result_issues(validate_references(&parsed).map(|_| ()), &mut issues);
        for document in &parsed {
            if let ParsedDocumentV1::Agent {
                document: agent, ..
            } = &document.parsed
                && agent.access_mode == AuthoredAccessMode::FullAccess
            {
                issues.push(full_access_issue(document.parsed.source_path()));
            }
        }
    }
    let issues = DiagnosticSet::from_issues(issues)
        .into_issues()
        .into_iter()
        .map(|issue| InspectedConfigurationIssue {
            layer: ValueLayer::Instructions,
            origin: ValueOrigin {
                layer: ValueLayer::Instructions,
                logical_path: issue.file.as_str().to_string(),
                field: issue
                    .field
                    .as_ref()
                    .map(|field| field.as_str().to_string())
                    .unwrap_or_else(|| "/".to_string()),
            },
            issue,
        })
        .collect();
    DocumentInspection {
        snapshot: behavior_snapshot(&parsed),
        issues,
    }
}

fn collect_result_issues(result: Result<(), DiagnosticSet>, issues: &mut Vec<ConfigurationIssue>) {
    if let Err(error) = result {
        issues.extend(error.into_issues());
    }
}

fn behavior_snapshot(parsed: &[ParsedLayerDocument]) -> BehaviorSnapshot {
    let mut snapshot = BehaviorSnapshot::default();
    for source in parsed {
        let path = source.parsed.source_path().as_str().to_string();
        match &source.parsed {
            ParsedDocumentV1::Orchestrator { document, .. } => {
                snapshot.routing.insert(
                    "orchestrator".to_string(),
                    canonical_json_sha256(&serde_json::json!({
                        "defaults": document.defaults,
                        "prompts": document.prompts,
                        "providerPolicy": document.provider_policy,
                        "fallbacks": document.fallbacks,
                    })),
                );
                snapshot.verification.insert(
                    "orchestrator".to_string(),
                    canonical_json_sha256(&document.verification),
                );
                for class in &document.classes {
                    insert_class(&mut snapshot.classes, class);
                }
                snapshot
                    .origins
                    .insert("routing".to_string(), origin(&path, "/"));
            }
            ParsedDocumentV1::Project { document, .. } => {
                snapshot.verification.insert(
                    "project".to_string(),
                    canonical_json_sha256(&document.verification),
                );
                for class in &document.class_overrides {
                    insert_class(&mut snapshot.classes, class);
                }
            }
            ParsedDocumentV1::Agent { document, body, .. } => {
                let id = document.id.clone();
                let canonical = canonical_json_sha256(&serde_json::json!({
                    "document": document,
                    "bodySha256": bytes_sha256(body.as_bytes()),
                }));
                snapshot.agents.insert(
                    id.clone(),
                    AgentBehavior {
                        canonical,
                        access: document.access_mode,
                        tools: canonical_json_sha256(&document.tools),
                        tool_allow: document.tools.allow.clone(),
                        tool_deny: document.tools.deny.clone(),
                        environments: document.environments.clone(),
                        capabilities: document.required_capabilities.clone(),
                        full_access: document.access_mode == AuthoredAccessMode::FullAccess,
                        verification: canonical_json_sha256(&document.verification),
                    },
                );
                snapshot.verification.insert(
                    format!("agent:{id}"),
                    canonical_json_sha256(&document.verification),
                );
                snapshot
                    .origins
                    .insert(format!("agent:{id}"), origin(&path, "/"));
            }
            ParsedDocumentV1::Knowledge { document, body, .. } => {
                snapshot
                    .knowledge
                    .insert(document.id.clone(), value_digest(document.as_ref(), body));
                snapshot
                    .origins
                    .insert(format!("knowledge:{}", document.id), origin(&path, "/"));
            }
            ParsedDocumentV1::Prompt { document, body, .. } => {
                snapshot
                    .prompts
                    .insert(document.id.clone(), value_digest(document.as_ref(), body));
                snapshot
                    .origins
                    .insert(format!("prompt:{}", document.id), origin(&path, "/"));
            }
        }
    }
    snapshot
}

fn insert_class(classes: &mut BTreeMap<String, String>, class: &ClassRuleV1) {
    classes.insert(class.id.clone(), canonical_json_sha256(class));
}

fn value_digest(value: &impl Serialize, body: &str) -> ContentSha256 {
    ContentSha256::parse(canonical_json_sha256(&serde_json::json!({
        "value": value,
        "bodySha256": bytes_sha256(body.as_bytes()),
    })))
    .expect("canonical content digest is SHA-256")
}

fn origin(path: &str, field: &str) -> ValueOrigin {
    ValueOrigin {
        layer: ValueLayer::Instructions,
        logical_path: path.to_string(),
        field: field.to_string(),
    }
}

fn behavior_diff(
    baseline: &BehaviorSnapshot,
    candidate: &BehaviorSnapshot,
) -> ConfigurationBehavioralDiff {
    ConfigurationBehavioralDiff {
        agents: map_diff(&baseline.agents, &candidate.agents),
        access: projected_agent_diff(baseline, candidate, |agent| &agent.access),
        tools: projected_agent_diff(baseline, candidate, |agent| &agent.tools),
        environments: projected_agent_diff(baseline, candidate, |agent| &agent.environments),
        capabilities: projected_agent_diff(baseline, candidate, |agent| &agent.capabilities),
        full_access: projected_agent_diff(baseline, candidate, |agent| &agent.full_access),
        prompts: map_diff(&baseline.prompts, &candidate.prompts),
        knowledge: map_diff(&baseline.knowledge, &candidate.knowledge),
        classes: map_diff(&baseline.classes, &candidate.classes),
        routing: map_diff(&baseline.routing, &candidate.routing),
        verification: map_diff(&baseline.verification, &candidate.verification),
        origins: map_diff(&baseline.origins, &candidate.origins),
        ..ConfigurationBehavioralDiff::default()
    }
}

fn projected_agent_diff<T: PartialEq>(
    baseline: &BehaviorSnapshot,
    candidate: &BehaviorSnapshot,
    projection: impl Fn(&AgentBehavior) -> &T,
) -> BehavioralChangeSet {
    let baseline = baseline
        .agents
        .iter()
        .map(|(id, agent)| (id.clone(), projection(agent)))
        .collect::<BTreeMap<_, _>>();
    let candidate = candidate
        .agents
        .iter()
        .map(|(id, agent)| (id.clone(), projection(agent)))
        .collect::<BTreeMap<_, _>>();
    map_diff(&baseline, &candidate)
}

fn map_diff<T: PartialEq>(
    baseline: &BTreeMap<String, T>,
    candidate: &BTreeMap<String, T>,
) -> BehavioralChangeSet {
    let mut diff = BehavioralChangeSet::default();
    for (key, value) in candidate {
        match baseline.get(key) {
            None => diff.added.push(key.clone()),
            Some(previous) if previous != value => diff.modified.push(key.clone()),
            Some(_) => {}
        }
    }
    for key in baseline.keys() {
        if !candidate.contains_key(key) {
            diff.removed.push(key.clone());
        }
    }
    diff
}

fn dependency_diff(
    baseline: &ConfigurationDependencySnapshot,
    candidate: &ConfigurationDependencySnapshot,
    diff: &mut ConfigurationBehavioralDiff,
) {
    if baseline.instructions_source_commit_sha != candidate.instructions_source_commit_sha
        || baseline.canonical_remote_identity != candidate.canonical_remote_identity
        || baseline.tracking != candidate.tracking
        || baseline.project_content_sha256 != candidate.project_content_sha256
        || baseline.personal_source_commit_sha != candidate.personal_source_commit_sha
    {
        diff.source_identities.modified.push("sources".to_string());
    }
    if baseline.trust_digest != candidate.trust_digest {
        diff.trust.modified.push("trust".to_string());
    }
    if baseline.settings_revision != candidate.settings_revision {
        diff.settings.modified.push("settings".to_string());
    }
}

fn changed_keys(diff: &BehavioralChangeSet) -> BTreeSet<String> {
    diff.added
        .iter()
        .chain(&diff.removed)
        .chain(&diff.modified)
        .cloned()
        .collect()
}

fn access_widened(baseline: &BehaviorSnapshot, candidate: &BehaviorSnapshot) -> bool {
    candidate.agents.iter().any(|(id, current)| {
        current.full_access
            || match baseline.agents.get(id) {
                None => true,
                Some(previous) => {
                    access_rank(&current.access) > access_rank(&previous.access)
                        || !current.tool_allow.is_subset(&previous.tool_allow)
                        || !current.tool_deny.is_superset(&previous.tool_deny)
                        || environment_set_widened(&previous.environments, &current.environments)
                        || !current.capabilities.is_superset(&previous.capabilities)
                }
            }
    })
}

const fn access_rank(access: &AuthoredAccessMode) -> usize {
    match access {
        AuthoredAccessMode::ReadOnly => 0,
        AuthoredAccessMode::WorkspaceOnly => 1,
        AuthoredAccessMode::Ask => 2,
        AuthoredAccessMode::AutoEdit => 3,
        AuthoredAccessMode::FullAccess => 4,
    }
}

fn environment_set_widened(previous: &BTreeSet<String>, current: &BTreeSet<String>) -> bool {
    if previous.contains("any") {
        return false;
    }
    current.contains("any") || !current.is_subset(previous)
}

fn file_diff(
    baseline: &[SourceDocument],
    candidate: &[SourceDocument],
) -> Vec<ConfigurationFileChange> {
    let baseline = document_map(baseline);
    let candidate = document_map(candidate);
    let mut changes = Vec::new();
    for (path, new_sha256) in &candidate {
        match baseline.get(path) {
            None => changes.push(ConfigurationFileChange {
                path: path.clone(),
                kind: ConfigurationFileChangeKind::Added,
                old_sha256: None,
                new_sha256: Some(new_sha256.clone()),
            }),
            Some(old_sha256) if old_sha256 != new_sha256 => {
                changes.push(ConfigurationFileChange {
                    path: path.clone(),
                    kind: ConfigurationFileChangeKind::Modified,
                    old_sha256: Some(old_sha256.clone()),
                    new_sha256: Some(new_sha256.clone()),
                });
            }
            Some(_) => {}
        }
    }
    for (path, old_sha256) in &baseline {
        if !candidate.contains_key(path) {
            changes.push(ConfigurationFileChange {
                path: path.clone(),
                kind: ConfigurationFileChangeKind::Deleted,
                old_sha256: Some(old_sha256.clone()),
                new_sha256: None,
            });
        }
    }
    changes.sort_by(|left, right| left.path.cmp(&right.path));
    changes
}

fn document_map(documents: &[SourceDocument]) -> BTreeMap<String, ContentSha256> {
    documents
        .iter()
        .map(|document| {
            (
                document.logical_path().as_str().to_string(),
                bytes_sha256(document.bytes()),
            )
        })
        .collect()
}

fn bytes_sha256(bytes: &[u8]) -> ContentSha256 {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    ContentSha256::parse(value).expect("SHA-256 output is valid")
}
