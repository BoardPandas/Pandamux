use pandamux_core::{
    ConfigurationRevision, ConfigurationUpdateSafety, ContentSha256, SourceCommitSha, TrackingRef,
};
use pandamux_orchestrator::{
    ConfigurationCandidateInspectionInput, ConfigurationDependencySnapshot, SourceDocument,
    inspect_configuration_candidate,
};
use std::collections::BTreeSet;

fn source(path: &str, contents: impl Into<Vec<u8>>) -> SourceDocument {
    SourceDocument::new(path, contents).expect("valid test source path")
}

fn prompt(id: &str, purpose: &str) -> SourceDocument {
    source(
        &format!("prompts/{id}.md"),
        format!(
            "---\nschemaVersion: 1\nkind: pandamux.prompt\nid: {id}\npurpose: {purpose}\n---\nPrompt {id}.\n"
        ),
    )
}

fn agent(
    access: &str,
    allow: &str,
    deny: &str,
    environments: &str,
    capabilities: &str,
) -> SourceDocument {
    source(
        "agents/builder.md",
        format!(
            "---\n\
             schemaVersion: 1\n\
             kind: pandamux.agent\n\
             id: builder\n\
             name: Builder\n\
             description: Builds deterministic changes.\n\
             specialties: [implementation]\n\
             tier: reasoning\n\
             effort: high\n\
             accessMode: {access}\n\
             requiredCapabilities: [{capabilities}]\n\
             providers: {{ prefer: [codex], avoid: [] }}\n\
             environments: [{environments}]\n\
             knowledge: [knowledge/guide.md]\n\
             tools: {{ allow: [{allow}], deny: [{deny}] }}\n\
             output: {{ kind: change_set }}\n\
             verification: [cargo test]\n\
             concurrency: 1\n\
             budget: {{ maxTokens: 1000, maxCostMicroUsd: 1000, maxRuntimeMs: 1000 }}\n\
             ---\n\
             Build safely.\n"
        ),
    )
}

fn valid_documents(access: &str) -> Vec<SourceDocument> {
    vec![
        source(
            "orchestrator.yaml",
            "schemaVersion: 1\nkind: pandamux.orchestrator\nminPandamuxVersion: \"0.60.79\"\ndefaults:\n  mode: approve_first\n  defaultAgent: builder\n  maxParallelTasks: 4\n  maxTasksPerRun: 6\n  maxAttemptsPerTask: 2\nprompts:\n  intake: prompts/intake.md\n  planner: prompts/planner.md\n  handoff: prompts/handoff.md\nclasses: []\nproviderPolicy:\n  minRemainingMillionths: 100000\n  concurrency: {}\nfallbacks:\n  onRateLimit: next_provider\n  onNoProvider: queue\n  onFailure: ask\nverification:\n  useProjectCommands: false\n",
        ),
        agent(access, "read", "shell", "local", "longContext"),
        source(
            "knowledge/guide.md",
            "---\nschemaVersion: 1\nkind: pandamux.knowledge\nid: guide\ntitle: Guide\ntags: []\n---\nGuide.\n",
        ),
        prompt("intake", "intake"),
        prompt("planner", "planner"),
        prompt("handoff", "handoff"),
    ]
}

fn dependencies(commit: char, revision: u64) -> ConfigurationDependencySnapshot {
    ConfigurationDependencySnapshot {
        instructions_source_commit_sha: Some(
            SourceCommitSha::parse(commit.to_string().repeat(40)).expect("commit"),
        ),
        canonical_remote_identity: "https://example.invalid/org/instructions".to_string(),
        tracking: TrackingRef::Branch("refs/heads/main".to_string()),
        project_content_sha256: None,
        personal_source_commit_sha: None,
        settings_revision: ConfigurationRevision::new(revision).expect("revision"),
        trust_digest: ContentSha256::parse("f".repeat(64)).expect("trust digest"),
    }
}

fn input(
    baseline: Vec<SourceDocument>,
    candidate: Vec<SourceDocument>,
) -> ConfigurationCandidateInspectionInput {
    ConfigurationCandidateInspectionInput {
        baseline_documents: baseline,
        candidate_documents: candidate,
        baseline_dependencies: dependencies('a', 1),
        candidate_dependencies: dependencies('b', 1),
        scheduled_agent_ids: BTreeSet::new(),
        history_rewritten: false,
    }
}

#[test]
fn valid_candidate_produces_complete_file_and_behavioral_diff() {
    let report = inspect_configuration_candidate(&input(Vec::new(), valid_documents("read_only")));
    assert!(report.activatable, "issues: {:?}", report.issues);
    assert_eq!(report.safety, ConfigurationUpdateSafety::AccessWidening);
    assert_eq!(report.file_diff.len(), 6);
    assert_eq!(report.behavioral_diff.agents.added, ["builder"]);
    assert_eq!(report.behavioral_diff.prompts.added.len(), 3);
    assert_eq!(report.behavioral_diff.knowledge.added, ["guide"]);
    assert_eq!(
        report.behavioral_diff.source_identities.modified,
        ["sources"]
    );
}

#[test]
fn inspection_collects_independent_file_issues_in_stable_order() {
    let candidate = vec![
        source("agents/bad.md", b"not frontmatter".to_vec()),
        source("prompts/bad.md", b"also not frontmatter".to_vec()),
    ];
    let mut reversed = candidate.clone();
    reversed.reverse();
    let first = inspect_configuration_candidate(&input(Vec::new(), candidate));
    let second = inspect_configuration_candidate(&input(Vec::new(), reversed));
    assert!(!first.activatable);
    assert!(first.issues.len() >= 3);
    assert_eq!(first.issues, second.issues);
    assert_eq!(first.diff_digest, second.diff_digest);
    let files = first
        .issues
        .iter()
        .map(|issue| issue.issue.file.as_str())
        .collect::<Vec<_>>();
    assert!(files.contains(&"agents/bad.md"));
    assert!(files.contains(&"prompts/bad.md"));
    assert!(files.contains(&"orchestrator.yaml"));
}

#[test]
fn safety_detects_access_widening_and_exact_scheduled_agent_impact() {
    let baseline = valid_documents("read_only");
    let mut candidate = valid_documents("read_only");
    candidate.retain(|document| document.logical_path().as_str() != "agents/builder.md");
    candidate.push(agent("auto_edit", "read, shell", "", "any", ""));
    let mut inspection = input(baseline, candidate);
    let access_only = inspect_configuration_candidate(&inspection);
    assert_eq!(
        access_only.safety,
        ConfigurationUpdateSafety::AccessWidening
    );
    assert_eq!(access_only.behavioral_diff.access.modified, ["builder"]);
    assert_eq!(access_only.behavioral_diff.tools.modified, ["builder"]);
    assert_eq!(
        access_only.behavioral_diff.environments.modified,
        ["builder"]
    );
    assert_eq!(
        access_only.behavioral_diff.capabilities.modified,
        ["builder"]
    );

    inspection
        .scheduled_agent_ids
        .insert("unrelated".to_string());
    assert_eq!(
        inspect_configuration_candidate(&inspection).safety,
        ConfigurationUpdateSafety::AccessWidening
    );
    inspection.scheduled_agent_ids.insert("builder".to_string());
    let scheduled = inspect_configuration_candidate(&inspection);
    assert_eq!(
        scheduled.safety,
        ConfigurationUpdateSafety::ScheduledAgentChange
    );
    assert_eq!(
        scheduled.behavioral_diff.scheduled_agents.modified,
        ["builder"]
    );
}

#[test]
fn every_dependency_input_participates_in_inspection_currency() {
    let documents = valid_documents("read_only");
    let first = input(documents.clone(), documents.clone());
    let mut second = first.clone();
    second.candidate_dependencies.settings_revision =
        ConfigurationRevision::new(2).expect("revision");
    let first = inspect_configuration_candidate(&first);
    let second = inspect_configuration_candidate(&second);
    assert_ne!(first.diff_digest, second.diff_digest);
    assert_eq!(second.behavioral_diff.settings.modified, ["settings"]);
}

#[test]
fn remote_and_tracking_changes_require_their_exact_safety_class() {
    let documents = valid_documents("read_only");
    let mut remote_change = input(documents.clone(), documents.clone());
    remote_change
        .candidate_dependencies
        .canonical_remote_identity = "https://example.invalid/other/instructions".to_string();
    assert_eq!(
        inspect_configuration_candidate(&remote_change).safety,
        ConfigurationUpdateSafety::RemoteIdentityChange
    );

    let mut tracking_change = input(documents.clone(), documents);
    tracking_change.candidate_dependencies.tracking =
        TrackingRef::Tag("refs/tags/stable".to_string());
    assert_eq!(
        inspect_configuration_candidate(&tracking_change).safety,
        ConfigurationUpdateSafety::TrackingChange
    );
}

#[test]
fn repository_full_access_is_visible_and_blocks_activation() {
    let report =
        inspect_configuration_candidate(&input(Vec::new(), valid_documents("full_access")));
    assert!(!report.activatable);
    assert_eq!(report.safety, ConfigurationUpdateSafety::AccessWidening);
    assert!(report.issues.iter().any(|issue| {
        issue.issue.code.as_str() == "policy_full_access_from_repo"
            && issue.origin.layer == pandamux_core::ValueLayer::Instructions
    }));
}
