use pandamux_core::{
    AccessMode, AgentId, BundleSourceManifest, ConfigurationRevision, ModelTarget, OrganizationId,
    ProjectId, ProviderInstanceId, SourceCommitSha, TierMapping, ValueLayer,
};
use pandamux_orchestrator::{
    CompileBundleInput, ConfigurationIssueCode, LayerGrant, LayeredSourceDocument, SourceDocument,
    TrustedBundleSettings, canonical_bundle_bytes, compile_instruction_bundle, source_layer_sha256,
    verify_compiled_bundle,
};
use std::collections::{BTreeMap, BTreeSet};

fn source(path: &str, contents: impl Into<Vec<u8>>) -> SourceDocument {
    SourceDocument::new(path, contents).expect("test path is valid")
}

fn prompt(id: &str, purpose: &str) -> String {
    format!(
        "---\nschemaVersion: 1\nkind: pandamux.prompt\nid: {id}\npurpose: {purpose}\n---\nPrompt {id}.\n"
    )
}

fn agent(
    id: &str,
    access_mode: &str,
    allow: &str,
    deny: &str,
    environments: &str,
    tier: &str,
) -> String {
    format!(
        "---\n\
         schemaVersion: 1\n\
         kind: pandamux.agent\n\
         id: {id}\n\
         name: Builder\n\
         description: Builds deterministic changes.\n\
         specialties: [implementation]\n\
         tier: {tier}\n\
         effort: high\n\
         accessMode: {access_mode}\n\
         requiredCapabilities: [longContext]\n\
         providers: {{ prefer: [codex, claude], avoid: [] }}\n\
         environments: [{environments}]\n\
         knowledge: [knowledge/review-checklist.md]\n\
         tools: {{ allow: [{allow}], deny: [{deny}] }}\n\
         output: {{ kind: change_set }}\n\
         verification: [cargo test]\n\
         concurrency: 2\n\
         budget: {{ maxTokens: 400000, maxCostMicroUsd: 5000000, maxRuntimeMs: 1800000 }}\n\
         ---\n\
         Build the requested change without widening authority.\n"
    )
}

fn instructions_documents() -> Vec<SourceDocument> {
    vec![
        source(
            "orchestrator.yaml",
            include_bytes!("fixtures/schema/orchestrator.yaml").to_vec(),
        ),
        source(
            "agents/builder.md",
            agent(
                "builder",
                "read_only",
                "read, grep",
                "write, shell",
                "local",
                "reasoning",
            ),
        ),
        source(
            "knowledge/review-checklist.md",
            include_bytes!("fixtures/schema/knowledge.md").to_vec(),
        ),
        source("prompts/intake.md", prompt("intake", "intake")),
        source("prompts/planner.md", prompt("planner", "planner")),
        source("prompts/handoff.md", prompt("handoff", "handoff")),
    ]
}

fn project_documents(agent_override: Option<String>) -> Vec<SourceDocument> {
    let agents = if agent_override.is_some() {
        "[agents/builder.md]"
    } else {
        "[]"
    };
    let mut documents = vec![source(
        "project.yaml",
        format!(
            "schemaVersion: 1\nkind: pandamux.project\nclassOverrides: []\nverification: [cargo test]\nagents: {agents}\nknowledge: []\n"
        ),
    )];
    if let Some(agent_override) = agent_override {
        documents.push(source("agents/builder.md", agent_override));
    }
    documents
}

fn settings(with_tier: bool) -> TrustedBundleSettings {
    let tiers = if with_tier {
        BTreeMap::from([(
            "reasoning".to_string(),
            ModelTarget {
                provider_instance_id: ProviderInstanceId::new("provider-test"),
                model: "test-model".to_string(),
                effort: Some("high".to_string()),
            },
        )])
    } else {
        BTreeMap::new()
    };
    TrustedBundleSettings {
        revision: ConfigurationRevision::new(7).expect("revision"),
        tier_mapping: TierMapping { tiers },
        disabled_agent_ids: BTreeSet::new(),
        full_access_agent_ids: BTreeSet::new(),
    }
}

fn compile_input(
    instructions: Vec<SourceDocument>,
    project: Vec<SourceDocument>,
) -> CompileBundleInput {
    let instructions_hash = source_layer_sha256(&instructions);
    let project_hash = (!project.is_empty()).then(|| source_layer_sha256(&project));
    let mut documents = instructions
        .into_iter()
        .map(|source| LayeredSourceDocument {
            layer: ValueLayer::Instructions,
            source,
        })
        .collect::<Vec<_>>();
    documents.extend(project.into_iter().map(|source| LayeredSourceDocument {
        layer: ValueLayer::Project,
        source,
    }));
    let organization_id = OrganizationId::new("org-test");
    let project_id = ProjectId::new("project-test");
    let mut grants = vec![LayerGrant {
        layer: ValueLayer::Instructions,
        trusted: true,
        content_sha256: instructions_hash,
        organization_id: organization_id.clone(),
        project_id: None,
    }];
    if let Some(project_hash) = &project_hash {
        grants.push(LayerGrant {
            layer: ValueLayer::Project,
            trusted: true,
            content_sha256: project_hash.clone(),
            organization_id: organization_id.clone(),
            project_id: Some(project_id.clone()),
        });
    }
    CompileBundleInput {
        documents,
        grants,
        source_manifest: BundleSourceManifest {
            instructions_commit_sha: SourceCommitSha::parse("0".repeat(40)).expect("commit"),
            project_layer_content_sha256: project_hash,
            personal_commit_sha: None,
            settings_revision: ConfigurationRevision::new(7).expect("revision"),
        },
        project_id,
        organization_id,
        settings: settings(true),
    }
}

fn error_codes(input: &CompileBundleInput) -> Vec<ConfigurationIssueCode> {
    compile_instruction_bundle(input)
        .expect_err("compilation must fail")
        .issues()
        .iter()
        .map(|issue| issue.code)
        .collect()
}

fn assert_code(input: &CompileBundleInput, code: ConfigurationIssueCode) {
    let actual = error_codes(input);
    assert!(actual.contains(&code), "expected {code:?}, got {actual:?}");
}

#[test]
fn compiles_canonical_bundle_with_origins_references_and_verified_digest() {
    let input = compile_input(instructions_documents(), project_documents(None));
    let bundle = compile_instruction_bundle(&input).expect("valid bundle compiles");

    assert_eq!(bundle.kind, "pandamux.compiled-bundle");
    assert_eq!(bundle.payload_sha256.len(), 64);
    assert_eq!(bundle.agents.len(), 1);
    assert_eq!(
        bundle.agents.values().next().expect("builder").bundle_sha,
        bundle.payload_sha256
    );
    assert_eq!(bundle.prompts.len(), 3);
    assert_eq!(bundle.knowledge.len(), 1);
    assert_eq!(bundle.policy.project_verification, ["cargo test"]);
    assert!(
        bundle
            .references
            .contains_key("instructions:orchestrator.yaml")
    );
    assert!(
        bundle
            .value_origins
            .contains_key("/agents/builder/accessMode")
    );
    verify_compiled_bundle(&bundle).expect("digest verifies");
    let decoded: serde_json::Value =
        serde_json::from_slice(&canonical_bundle_bytes(&bundle)).expect("canonical JSON parses");
    assert_eq!(decoded["payloadSha256"], bundle.payload_sha256);
}

#[test]
fn canonical_bytes_and_digest_ignore_source_and_grant_input_order() {
    let mut first = compile_input(instructions_documents(), project_documents(None));
    let mut second = first.clone();
    first.documents.rotate_left(2);
    first.grants.reverse();
    second.documents.reverse();

    let first = compile_instruction_bundle(&first).expect("first compiles");
    let second = compile_instruction_bundle(&second).expect("second compiles");
    assert_eq!(first.payload_sha256, second.payload_sha256);
    assert_eq!(
        canonical_bundle_bytes(&first),
        canonical_bundle_bytes(&second)
    );
}

#[test]
fn rejects_broken_wrong_kind_duplicate_and_escaping_references() {
    let mut missing = instructions_documents();
    missing.retain(|document| document.logical_path().as_str() != "prompts/planner.md");
    assert_code(
        &compile_input(missing, project_documents(None)),
        ConfigurationIssueCode::ReferenceNotFound,
    );

    let mut wrong_kind = instructions_documents();
    let orchestrator = wrong_kind
        .iter_mut()
        .find(|document| document.logical_path().as_str() == "orchestrator.yaml")
        .expect("orchestrator");
    *orchestrator = source(
        "orchestrator.yaml",
        include_str!("fixtures/schema/orchestrator.yaml")
            .replace("prompts/intake.md", "knowledge/review-checklist.md"),
    );
    assert_code(
        &compile_input(wrong_kind, project_documents(None)),
        ConfigurationIssueCode::ReferenceKindMismatch,
    );

    let mut duplicate = instructions_documents();
    duplicate.push(duplicate[1].clone());
    assert_code(
        &compile_input(duplicate, project_documents(None)),
        ConfigurationIssueCode::DocumentIdDuplicate,
    );

    let duplicate_reference = vec![source(
        "project.yaml",
        "schemaVersion: 1\nkind: pandamux.project\nclassOverrides: []\nverification: []\nagents: [agents/builder.md, agents/builder.md]\nknowledge: []\n",
    )];
    assert_code(
        &compile_input(instructions_documents(), duplicate_reference),
        ConfigurationIssueCode::ReferenceDuplicateId,
    );

    let escaping = project_documents(Some(agent(
        "builder",
        "read_only",
        "read, grep",
        "write, shell",
        "local",
        "reasoning",
    )));
    let escaping = escaping
        .into_iter()
        .map(|document| {
            if document.logical_path().as_str() == "agents/builder.md" {
                source(
                    "agents/builder.md",
                    String::from_utf8(document.bytes().to_vec())
                        .expect("utf8")
                        .replace("knowledge/review-checklist.md", "../secret.md"),
                )
            } else {
                document
            }
        })
        .collect();
    assert_code(
        &compile_input(instructions_documents(), escaping),
        ConfigurationIssueCode::ReferencePathEscape,
    );
}

#[test]
fn rejects_project_authority_widening_and_full_access() {
    for (candidate, expected) in [
        (
            agent(
                "builder",
                "auto_edit",
                "read, grep",
                "write, shell",
                "local",
                "reasoning",
            ),
            ConfigurationIssueCode::PolicyAccessWidening,
        ),
        (
            agent(
                "builder",
                "read_only",
                "read, grep, shell",
                "write",
                "local",
                "reasoning",
            ),
            ConfigurationIssueCode::PolicyAccessWidening,
        ),
        (
            agent(
                "builder",
                "read_only",
                "read, grep",
                "write, shell",
                "any",
                "reasoning",
            ),
            ConfigurationIssueCode::PolicyAccessWidening,
        ),
        (
            agent(
                "builder",
                "full_access",
                "read, grep",
                "write, shell",
                "local",
                "reasoning",
            ),
            ConfigurationIssueCode::PolicyFullAccessFromRepo,
        ),
    ] {
        assert_code(
            &compile_input(instructions_documents(), project_documents(Some(candidate))),
            expected,
        );
    }
}

#[test]
fn trusted_settings_apply_only_after_repository_policy_validation() {
    let mut valid = compile_input(instructions_documents(), project_documents(None));
    valid
        .settings
        .disabled_agent_ids
        .insert(AgentId::new("builder"));
    valid
        .settings
        .full_access_agent_ids
        .insert(AgentId::new("builder"));
    let bundle = compile_instruction_bundle(&valid).expect("trusted settings compile");
    let builder = bundle
        .agents
        .get(&AgentId::new("builder"))
        .expect("builder");
    assert!(!builder.enabled);
    assert_eq!(builder.access_mode, AccessMode::FullAccess);
    assert!(builder.full_access_opt_in);
    assert_eq!(
        bundle
            .value_origins
            .get("/agents/builder/accessMode")
            .expect("access origin")
            .layer,
        ValueLayer::Settings
    );

    let mut widening = compile_input(
        instructions_documents(),
        project_documents(Some(agent(
            "builder",
            "auto_edit",
            "read, grep",
            "write, shell",
            "local",
            "reasoning",
        ))),
    );
    widening
        .settings
        .full_access_agent_ids
        .insert(AgentId::new("builder"));
    assert_code(&widening, ConfigurationIssueCode::PolicyAccessWidening);
}

#[test]
fn compiled_origins_cover_all_four_configuration_layers() {
    let mut input = compile_input(instructions_documents(), project_documents(None));
    let personal = source(
        "agents/personal.md",
        agent(
            "personal",
            "read_only",
            "read, grep",
            "write, shell",
            "local",
            "reasoning",
        ),
    );
    let personal_hash = source_layer_sha256(std::slice::from_ref(&personal));
    input.documents.push(LayeredSourceDocument {
        layer: ValueLayer::Personal,
        source: personal,
    });
    input.grants.push(LayerGrant {
        layer: ValueLayer::Personal,
        trusted: true,
        content_sha256: personal_hash,
        organization_id: input.organization_id.clone(),
        project_id: Some(input.project_id.clone()),
    });
    input.source_manifest.personal_commit_sha =
        Some(SourceCommitSha::parse("1".repeat(40)).expect("personal commit"));
    input
        .settings
        .disabled_agent_ids
        .insert(AgentId::new("personal"));

    let bundle = compile_instruction_bundle(&input).expect("four-layer bundle");
    let layers = bundle
        .value_origins
        .values()
        .map(|origin| origin.layer)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        layers,
        BTreeSet::from([
            ValueLayer::Instructions,
            ValueLayer::Project,
            ValueLayer::Personal,
            ValueLayer::Settings,
        ])
    );
}

#[test]
fn rejects_untrusted_wrong_scope_hash_and_unmapped_tier() {
    let mut untrusted = compile_input(instructions_documents(), project_documents(None));
    untrusted.grants[0].trusted = false;
    assert_code(&untrusted, ConfigurationIssueCode::LayerNotTrusted);

    let mut cross_org = compile_input(instructions_documents(), project_documents(None));
    cross_org.grants[0].organization_id = OrganizationId::new("org-other");
    assert_code(&cross_org, ConfigurationIssueCode::PolicyCrossOrganization);

    let mut wrong_scope = compile_input(instructions_documents(), project_documents(None));
    let project_grant = wrong_scope
        .grants
        .iter_mut()
        .find(|grant| grant.layer == ValueLayer::Project)
        .expect("project grant");
    project_grant.project_id = Some(ProjectId::new("project-other"));
    assert_code(&wrong_scope, ConfigurationIssueCode::LayerScopeViolation);

    let mut wrong_hash = compile_input(instructions_documents(), project_documents(None));
    wrong_hash.grants[0].content_sha256 =
        pandamux_core::ContentSha256::parse("0".repeat(64)).expect("content hash");
    assert_code(&wrong_hash, ConfigurationIssueCode::BundleHashMismatch);

    let mut unmapped = compile_input(instructions_documents(), project_documents(None));
    unmapped.settings = settings(false);
    assert_code(&unmapped, ConfigurationIssueCode::TierUnmapped);
}

#[test]
fn rejects_project_prompt_control_and_unknown_privileged_fields() {
    let mut project = project_documents(None);
    project.push(source("prompts/intake.md", prompt("intake", "intake")));
    assert_code(
        &compile_input(instructions_documents(), project),
        ConfigurationIssueCode::LayerScopeViolation,
    );

    for field in ["organizationId: org-other", "schedules: [nightly]"] {
        let project = vec![source(
            "project.yaml",
            format!(
                "schemaVersion: 1\nkind: pandamux.project\nclassOverrides: []\nverification: []\nagents: []\nknowledge: []\n{field}\n"
            ),
        )];
        assert_code(
            &compile_input(instructions_documents(), project),
            ConfigurationIssueCode::SchemaUnknownField,
        );
    }
}

#[test]
fn digest_verification_fails_closed_after_payload_or_nested_hash_tampering() {
    let input = compile_input(instructions_documents(), project_documents(None));
    let bundle = compile_instruction_bundle(&input).expect("valid bundle");

    let mut payload_tampered = bundle.clone();
    payload_tampered.policy.max_parallel_tasks += 1;
    let error = verify_compiled_bundle(&payload_tampered).expect_err("payload tamper fails");
    assert_eq!(
        error.issues()[0].code,
        ConfigurationIssueCode::BundleHashMismatch
    );

    let mut nested_hash_tampered = bundle;
    nested_hash_tampered
        .agents
        .values_mut()
        .next()
        .expect("agent")
        .bundle_sha = "0".repeat(64);
    let error = verify_compiled_bundle(&nested_hash_tampered).expect_err("nested hash fails");
    assert_eq!(
        error.issues()[0].code,
        ConfigurationIssueCode::BundleHashMismatch
    );
}
