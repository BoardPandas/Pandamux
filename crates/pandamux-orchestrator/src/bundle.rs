use crate::diagnostics::{
    ConfigurationIssue, ConfigurationIssueCode, ConfigurationStage, DiagnosticSet,
    RelativeConfigPath, issue,
};
use crate::policy::{
    LayerGrant, TrustedBundleSettings, canonical_json_bytes, full_access_issue,
    validate_agent_override, validate_grant_scope,
};
use crate::schema::v1::{
    AgentDocumentV1, AuthoredAccessMode, AuthoredOrchestrationMode, ClassRuleV1, DocumentKind,
    FailureFallbackV1, NoProviderFallbackV1, PromptPurposeV1, RateLimitFallbackV1,
};
use crate::schema::{
    ParsedDocumentV1, SourceDocument, parse_source_document, validate_source_set_bounds,
};
use pandamux_core::{
    AccessMode, AgentAuthor, AgentBudget, AgentDefinition, AgentId, AgentProviderPolicy,
    AgentScope, AgentToolPolicy, BundleReference, BundleSourceManifest,
    COMPILED_BUNDLE_SCHEMA_VERSION, CompiledClassRule, CompiledInstructionBundleV1,
    CompiledKnowledge, CompiledOrchestrationPolicy, CompiledPrompt, ContentSha256, FailureFallback,
    NoProviderFallback, OrchestrationMode, OrganizationId, OutputContract, ProjectId,
    PromptPurpose, ProviderKind, RateLimitFallback, ValueLayer, ValueOrigin,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const COMPILED_BUNDLE_KIND: &str = "pandamux.compiled-bundle";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayeredSourceDocument {
    pub layer: ValueLayer,
    pub source: SourceDocument,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileBundleInput {
    pub documents: Vec<LayeredSourceDocument>,
    pub grants: Vec<LayerGrant>,
    pub source_manifest: BundleSourceManifest,
    pub project_id: ProjectId,
    pub organization_id: OrganizationId,
    pub settings: TrustedBundleSettings,
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedLayerDocument {
    pub(crate) layer: ValueLayer,
    pub(crate) parsed: ParsedDocumentV1,
}

struct CompiledAgents {
    agents: BTreeMap<AgentId, AgentDefinition>,
    origins: BTreeMap<String, ValueOrigin>,
}

pub fn compile_instruction_bundle(
    input: &CompileBundleInput,
) -> Result<CompiledInstructionBundleV1, DiagnosticSet> {
    let sources = input
        .documents
        .iter()
        .map(|document| document.source.clone())
        .collect::<Vec<_>>();
    validate_source_set_bounds(&sources)?;

    let mut ordered_sources = input.documents.iter().collect::<Vec<_>>();
    ordered_sources.sort_by(|left, right| {
        left.layer
            .cmp(&right.layer)
            .then_with(|| left.source.logical_path().cmp(right.source.logical_path()))
            .then_with(|| left.source.bytes().cmp(right.source.bytes()))
    });

    let mut parsed = Vec::with_capacity(ordered_sources.len());
    let mut parse_issues = Vec::new();
    for layered in ordered_sources {
        match parse_source_document(&layered.source) {
            Ok(document) => parsed.push(ParsedLayerDocument {
                layer: layered.layer,
                parsed: document,
            }),
            Err(error) => parse_issues.extend(error.into_issues()),
        }
    }
    finish(parse_issues)?;

    let grants = validate_grants(input)?;
    validate_document_inventory(&parsed, true)?;
    let references = validate_references(&parsed)?;

    let orchestrator = parsed
        .iter()
        .find(|document| {
            document.layer == ValueLayer::Instructions
                && document.parsed.kind() == DocumentKind::Orchestrator
        })
        .expect("inventory validation requires an orchestrator");
    let ParsedDocumentV1::Orchestrator {
        document: orchestrator_document,
        ..
    } = &orchestrator.parsed
    else {
        unreachable!("kind matched")
    };
    let project_document = parsed.iter().find_map(|document| {
        if document.layer != ValueLayer::Project {
            return None;
        }
        match &document.parsed {
            ParsedDocumentV1::Project { document, .. } => Some(document.as_ref()),
            _ => None,
        }
    });

    let compiled_agents = compile_agents(&parsed, input)?;
    let mut agents = compiled_agents.agents;
    let mut value_origins = compiled_agents.origins;
    apply_trusted_settings(&mut agents, &mut value_origins, &input.settings);
    let prompts = compile_prompts(&parsed, &mut value_origins);
    let knowledge = compile_knowledge(&parsed, &mut value_origins);
    let classes = compile_classes(
        &orchestrator_document.classes,
        project_document.map(|document| document.class_overrides.as_slice()),
        &agents,
        orchestrator.parsed.source_path(),
    )?;

    let default_agent_id = AgentId::new(orchestrator_document.defaults.default_agent.clone());
    if !agents.contains_key(&default_agent_id) {
        return Err(DiagnosticSet::one(issue(
            ConfigurationIssueCode::ReferenceNotFound,
            orchestrator.parsed.source_path().clone(),
            Some("/defaults/defaultAgent"),
            ConfigurationStage::Reference,
            "The default agent does not resolve to a compiled agent",
            "Reference an agent visible to the instructions layer",
            None,
        )));
    }

    value_origins.insert("/policy".to_string(), origin(orchestrator, "/"));
    if let Some(project) = parsed.iter().find(|document| {
        document.layer == ValueLayer::Project && document.parsed.kind() == DocumentKind::Project
    }) {
        value_origins.insert(
            "/policy/projectVerification".to_string(),
            origin(project, "/verification"),
        );
    }

    let policy = CompiledOrchestrationPolicy {
        mode: match orchestrator_document.defaults.mode {
            AuthoredOrchestrationMode::Auto => OrchestrationMode::Auto,
            AuthoredOrchestrationMode::ApproveFirst => OrchestrationMode::ApproveFirst,
        },
        default_agent_id,
        max_parallel_tasks: orchestrator_document.defaults.max_parallel_tasks,
        max_tasks_per_run: orchestrator_document.defaults.max_tasks_per_run,
        max_attempts_per_task: orchestrator_document.defaults.max_attempts_per_task,
        min_remaining_millionths: orchestrator_document
            .provider_policy
            .min_remaining_millionths,
        provider_concurrency: orchestrator_document.provider_policy.concurrency.clone(),
        on_rate_limit: match orchestrator_document.fallbacks.on_rate_limit {
            RateLimitFallbackV1::NextProvider => RateLimitFallback::NextProvider,
            RateLimitFallbackV1::Queue => RateLimitFallback::Queue,
            RateLimitFallbackV1::Ask => RateLimitFallback::Ask,
            RateLimitFallbackV1::Fail => RateLimitFallback::Fail,
        },
        on_no_provider: match orchestrator_document.fallbacks.on_no_provider {
            NoProviderFallbackV1::Queue => NoProviderFallback::Queue,
            NoProviderFallbackV1::Ask => NoProviderFallback::Ask,
            NoProviderFallbackV1::Fail => NoProviderFallback::Fail,
        },
        on_failure: match orchestrator_document.fallbacks.on_failure {
            FailureFallbackV1::Ask => FailureFallback::Ask,
            FailureFallbackV1::Fail => FailureFallback::Fail,
        },
        use_project_commands: orchestrator_document.verification.use_project_commands,
        project_verification: project_document
            .map(|document| document.verification.clone())
            .unwrap_or_default(),
        tier_mapping: input.settings.tier_mapping.clone(),
    };

    let mut bundle = CompiledInstructionBundleV1 {
        schema_version: COMPILED_BUNDLE_SCHEMA_VERSION,
        kind: COMPILED_BUNDLE_KIND.to_string(),
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        source_manifest: input.source_manifest.clone(),
        project_id: input.project_id.clone(),
        organization_id: input.organization_id.clone(),
        agents,
        prompts,
        knowledge,
        references,
        classes,
        policy,
        value_origins,
        payload_sha256: String::new(),
    };
    let digest = unsigned_bundle_digest(&bundle);
    for agent in bundle.agents.values_mut() {
        agent.bundle_sha.clone_from(&digest);
    }
    bundle.payload_sha256 = digest;
    verify_compiled_bundle(&bundle)?;

    // Ensure every grant participated in the trusted source set. This assertion
    // is after construction so it cannot accidentally select policy.
    debug_assert!(
        grants
            .keys()
            .all(|layer| parsed.iter().any(|document| document.layer == *layer))
    );
    Ok(bundle)
}

fn apply_trusted_settings(
    agents: &mut BTreeMap<AgentId, AgentDefinition>,
    origins: &mut BTreeMap<String, ValueOrigin>,
    settings: &TrustedBundleSettings,
) {
    for (id, agent) in agents {
        let pointer = escape_pointer(id.as_str());
        if settings.disabled_agent_ids.contains(id) {
            agent.enabled = false;
            origins.insert(
                format!("/agents/{pointer}/enabled"),
                settings_origin("/trustedConfiguration/disabledAgentIds"),
            );
        }
        if settings.full_access_agent_ids.contains(id) {
            agent.access_mode = AccessMode::FullAccess;
            agent.full_access_opt_in = true;
            origins.insert(
                format!("/agents/{pointer}/accessMode"),
                settings_origin("/trustedConfiguration/fullAccessAgentIds"),
            );
            origins.insert(
                format!("/agents/{pointer}/fullAccessOptIn"),
                settings_origin("/trustedConfiguration/fullAccessAgentIds"),
            );
        }
    }
}

fn settings_origin(field: &str) -> ValueOrigin {
    ValueOrigin {
        layer: ValueLayer::Settings,
        logical_path: "settings".to_string(),
        field: field.to_string(),
    }
}

pub fn canonical_bundle_bytes(bundle: &CompiledInstructionBundleV1) -> Vec<u8> {
    canonical_json_bytes(bundle)
}

pub fn verify_compiled_bundle(bundle: &CompiledInstructionBundleV1) -> Result<(), DiagnosticSet> {
    let expected = unsigned_bundle_digest(bundle);
    let agents_match = bundle
        .agents
        .values()
        .all(|agent| agent.bundle_sha == bundle.payload_sha256);
    if bundle.schema_version != COMPILED_BUNDLE_SCHEMA_VERSION
        || bundle.kind != COMPILED_BUNDLE_KIND
        || bundle.payload_sha256 != expected
        || !agents_match
    {
        return Err(DiagnosticSet::one(issue(
            ConfigurationIssueCode::BundleHashMismatch,
            RelativeConfigPath::invalid(),
            Some("/payloadSha256"),
            ConfigurationStage::Compilation,
            "The compiled bundle does not match its canonical payload digest",
            "Discard the candidate and compile again from the trusted source set",
            None,
        )));
    }
    Ok(())
}

pub fn source_layer_sha256(documents: &[SourceDocument]) -> ContentSha256 {
    let mut documents = documents.iter().collect::<Vec<_>>();
    documents.sort_by(|left, right| {
        left.logical_path()
            .cmp(right.logical_path())
            .then_with(|| left.bytes().cmp(right.bytes()))
    });
    let mut hasher = Sha256::new();
    for document in documents {
        hash_len_and_bytes(&mut hasher, document.logical_path().as_str().as_bytes());
        hash_len_and_bytes(&mut hasher, document.bytes());
    }
    ContentSha256::parse(hex_digest(hasher.finalize().as_slice()))
        .expect("SHA-256 output is a valid content identity")
}

fn validate_grants(
    input: &CompileBundleInput,
) -> Result<BTreeMap<ValueLayer, &LayerGrant>, DiagnosticSet> {
    let mut grants = BTreeMap::new();
    let mut issues = Vec::new();
    for grant in &input.grants {
        if grants.insert(grant.layer, grant).is_some() {
            issues.push(issue(
                ConfigurationIssueCode::LayerScopeViolation,
                RelativeConfigPath::invalid(),
                None,
                ConfigurationStage::Policy,
                "More than one grant was supplied for a source layer",
                "Supply exactly one server-issued grant for each source layer",
                None,
            ));
        }
        issues.extend(validate_grant_scope(
            grant,
            &input.project_id,
            &input.organization_id,
        ));
    }

    let layers = input
        .documents
        .iter()
        .map(|document| document.layer)
        .collect::<BTreeSet<_>>();
    for layer in &layers {
        let Some(grant) = grants.get(layer) else {
            issues.push(issue(
                ConfigurationIssueCode::LayerNotTrusted,
                RelativeConfigPath::invalid(),
                None,
                ConfigurationStage::Policy,
                "A source layer has no server-issued grant",
                "Supply a grant bound to the exact source content hash",
                None,
            ));
            continue;
        };
        let layer_sources = input
            .documents
            .iter()
            .filter(|document| document.layer == *layer)
            .map(|document| document.source.clone())
            .collect::<Vec<_>>();
        if source_layer_sha256(&layer_sources) != grant.content_sha256 {
            issues.push(issue(
                ConfigurationIssueCode::BundleHashMismatch,
                RelativeConfigPath::invalid(),
                None,
                ConfigurationStage::Compilation,
                "A source layer does not match the content hash in its grant",
                "Request a new grant for the exact bounded source bytes",
                None,
            ));
        }
    }
    for layer in grants.keys() {
        if !layers.contains(layer) {
            issues.push(issue(
                ConfigurationIssueCode::LayerScopeViolation,
                RelativeConfigPath::invalid(),
                None,
                ConfigurationStage::Policy,
                "A layer grant has no corresponding source documents",
                "Remove unused grants from the compilation input",
                None,
            ));
        }
    }

    if input.settings.revision != input.source_manifest.settings_revision {
        issues.push(issue(
            ConfigurationIssueCode::BundleHashMismatch,
            RelativeConfigPath::invalid(),
            Some("/sourceManifest/settingsRevision"),
            ConfigurationStage::Compilation,
            "The trusted settings snapshot does not match the source manifest revision",
            "Compile with the settings snapshot named by the source manifest",
            None,
        ));
    }
    let project_hash = grants
        .get(&ValueLayer::Project)
        .map(|grant| grant.content_sha256.clone());
    if input.source_manifest.project_layer_content_sha256.as_ref() != project_hash.as_ref() {
        issues.push(issue(
            ConfigurationIssueCode::BundleHashMismatch,
            RelativeConfigPath::invalid(),
            Some("/sourceManifest/projectLayerContentSha256"),
            ConfigurationStage::Compilation,
            "The project layer hash does not match the source manifest",
            "Bind the manifest to the exact granted project-layer content hash",
            None,
        ));
    }
    finish(issues)?;
    Ok(grants)
}

pub(crate) fn validate_document_inventory(
    parsed: &[ParsedLayerDocument],
    require_instructions: bool,
) -> Result<(), DiagnosticSet> {
    let mut issues = Vec::new();
    let mut paths = BTreeSet::new();
    let mut ids = BTreeMap::<(ValueLayer, String), DocumentKind>::new();
    let mut orchestrators = 0usize;
    let mut projects = 0usize;

    for document in parsed {
        let path = document.parsed.source_path();
        if !paths.insert((document.layer, path.as_str().to_string())) {
            issues.push(issue(
                ConfigurationIssueCode::DocumentIdDuplicate,
                path.clone(),
                None,
                ConfigurationStage::Identity,
                "A source layer contains the same logical path more than once",
                "Keep exactly one document at each logical path in a layer",
                None,
            ));
        }
        if !kind_allowed_in_layer(document.layer, document.parsed.kind()) {
            issues.push(issue(
                ConfigurationIssueCode::LayerScopeViolation,
                path.clone(),
                Some("/kind"),
                ConfigurationStage::Policy,
                "This authored document kind is not allowed in the source layer",
                "Move the document to an authorized layer or remove it",
                None,
            ));
        }
        if let Some(id) = document.parsed.document_id() {
            match ids.insert((document.layer, id.to_string()), document.parsed.kind()) {
                Some(previous) if previous == document.parsed.kind() => issues.push(issue(
                    ConfigurationIssueCode::DocumentIdDuplicate,
                    path.clone(),
                    Some("/id"),
                    ConfigurationStage::Identity,
                    "A source layer contains a duplicate document id",
                    "Use one unique id per authored document in a layer",
                    None,
                )),
                Some(_) => issues.push(issue(
                    ConfigurationIssueCode::ReferenceDuplicateId,
                    path.clone(),
                    Some("/id"),
                    ConfigurationStage::Reference,
                    "A source layer reuses one id for different document kinds",
                    "Use unique ids across agents, knowledge, and prompts",
                    None,
                )),
                None => {}
            }
        }
        if document.layer == ValueLayer::Instructions
            && document.parsed.kind() == DocumentKind::Orchestrator
        {
            orchestrators += 1;
        }
        if document.layer == ValueLayer::Project && document.parsed.kind() == DocumentKind::Project
        {
            projects += 1;
        }
    }
    if require_instructions && orchestrators != 1 {
        issues.push(issue(
            ConfigurationIssueCode::ReferenceNotFound,
            RelativeConfigPath::new("orchestrator.yaml").expect("static path"),
            None,
            ConfigurationStage::Reference,
            "The instructions layer must contain exactly one orchestrator document",
            "Add one orchestrator.yaml document to the instructions layer",
            None,
        ));
    }
    let has_project_layer = parsed
        .iter()
        .any(|document| document.layer == ValueLayer::Project);
    if (has_project_layer && projects != 1) || (!has_project_layer && projects != 0) {
        issues.push(issue(
            ConfigurationIssueCode::ReferenceNotFound,
            RelativeConfigPath::new("project.yaml").expect("static path"),
            None,
            ConfigurationStage::Reference,
            "A project source layer must contain exactly one project document",
            "Add one project.yaml document or remove the project layer",
            None,
        ));
    }
    finish(issues)
}

fn kind_allowed_in_layer(layer: ValueLayer, kind: DocumentKind) -> bool {
    match layer {
        ValueLayer::Instructions => {
            !matches!(kind, DocumentKind::Project | DocumentKind::CompiledBundle)
        }
        ValueLayer::Project => matches!(
            kind,
            DocumentKind::Project | DocumentKind::Agent | DocumentKind::Knowledge
        ),
        ValueLayer::Personal => matches!(kind, DocumentKind::Agent | DocumentKind::Knowledge),
        ValueLayer::Settings => false,
    }
}

pub(crate) fn validate_references(
    parsed: &[ParsedLayerDocument],
) -> Result<BTreeMap<String, Vec<BundleReference>>, DiagnosticSet> {
    let mut path_index = BTreeMap::new();
    let mut id_index = BTreeMap::new();
    for (index, document) in parsed.iter().enumerate() {
        path_index.insert(
            (
                document.layer,
                document.parsed.source_path().as_str().to_string(),
            ),
            index,
        );
        if let Some(id) = document.parsed.document_id() {
            id_index.insert(
                (document.layer, document.parsed.kind(), id.to_string()),
                index,
            );
        }
    }

    let mut issues = Vec::new();
    let mut references = BTreeMap::<String, Vec<BundleReference>>::new();
    for document in parsed {
        let key = format!(
            "{}:{}",
            layer_label(document.layer),
            document.parsed.source_path()
        );
        let mut entries = Vec::new();
        match &document.parsed {
            ParsedDocumentV1::Orchestrator {
                document: value, ..
            } => {
                for (path, purpose) in [
                    (value.prompts.intake.as_str(), PromptPurposeV1::Intake),
                    (value.prompts.planner.as_str(), PromptPurposeV1::Planner),
                    (value.prompts.handoff.as_str(), PromptPurposeV1::Handoff),
                ] {
                    resolve_path_reference(
                        parsed,
                        &path_index,
                        document,
                        path,
                        DocumentKind::Prompt,
                        Some(purpose),
                        &mut entries,
                        &mut issues,
                    );
                }
                resolve_id_reference(
                    parsed,
                    &id_index,
                    document,
                    &value.defaults.default_agent,
                    DocumentKind::Agent,
                    &mut entries,
                    &mut issues,
                );
                for class in &value.classes {
                    resolve_class_references(
                        parsed,
                        &id_index,
                        document,
                        class,
                        &mut entries,
                        &mut issues,
                    );
                }
            }
            ParsedDocumentV1::Project {
                document: value, ..
            } => {
                for path in &value.agents {
                    resolve_path_reference(
                        parsed,
                        &path_index,
                        document,
                        path,
                        DocumentKind::Agent,
                        None,
                        &mut entries,
                        &mut issues,
                    );
                }
                for path in &value.knowledge {
                    resolve_path_reference(
                        parsed,
                        &path_index,
                        document,
                        path,
                        DocumentKind::Knowledge,
                        None,
                        &mut entries,
                        &mut issues,
                    );
                }
                for class in &value.class_overrides {
                    resolve_class_references(
                        parsed,
                        &id_index,
                        document,
                        class,
                        &mut entries,
                        &mut issues,
                    );
                }
            }
            ParsedDocumentV1::Agent {
                document: value, ..
            } => {
                for path in &value.knowledge {
                    resolve_path_reference(
                        parsed,
                        &path_index,
                        document,
                        path,
                        DocumentKind::Knowledge,
                        None,
                        &mut entries,
                        &mut issues,
                    );
                }
            }
            ParsedDocumentV1::Knowledge { .. } | ParsedDocumentV1::Prompt { .. } => {}
        }
        entries.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.expected_kind.cmp(&right.expected_kind))
                .then_with(|| left.resolved_id.cmp(&right.resolved_id))
        });
        if entries.windows(2).any(|pair| pair[0] == pair[1]) {
            issues.push(issue(
                ConfigurationIssueCode::ReferenceDuplicateId,
                document.parsed.source_path().clone(),
                None,
                ConfigurationStage::Reference,
                "A document contains the same resolved reference more than once",
                "Keep one reference to each resolved document",
                None,
            ));
        }
        entries.dedup();
        if !entries.is_empty() {
            references.insert(key, entries);
        }
    }
    finish(issues)?;
    Ok(references)
}

#[allow(clippy::too_many_arguments)]
fn resolve_path_reference(
    parsed: &[ParsedLayerDocument],
    index: &BTreeMap<(ValueLayer, String), usize>,
    owner: &ParsedLayerDocument,
    target_path: &str,
    expected_kind: DocumentKind,
    expected_purpose: Option<PromptPurposeV1>,
    entries: &mut Vec<BundleReference>,
    issues: &mut Vec<ConfigurationIssue>,
) {
    let Some(target_index) = visible_layers(owner.layer)
        .iter()
        .find_map(|layer| index.get(&(*layer, target_path.to_string())).copied())
    else {
        issues.push(reference_issue(
            ConfigurationIssueCode::ReferenceNotFound,
            owner,
            "A referenced logical path does not exist in a visible layer",
            "Add the referenced document or correct the logical path",
        ));
        return;
    };
    let target = &parsed[target_index];
    if target.parsed.kind() != expected_kind || !purpose_matches(&target.parsed, expected_purpose) {
        issues.push(reference_issue(
            ConfigurationIssueCode::ReferenceKindMismatch,
            owner,
            "A referenced document has the wrong kind or prompt purpose",
            "Point the reference at a document with the required kind and purpose",
        ));
        return;
    }
    entries.push(BundleReference {
        path: target_path.to_string(),
        expected_kind: expected_kind.as_str().to_string(),
        resolved_id: target
            .parsed
            .document_id()
            .expect("path references target identified documents")
            .to_string(),
    });
}

fn resolve_id_reference(
    parsed: &[ParsedLayerDocument],
    index: &BTreeMap<(ValueLayer, DocumentKind, String), usize>,
    owner: &ParsedLayerDocument,
    target_id: &str,
    expected_kind: DocumentKind,
    entries: &mut Vec<BundleReference>,
    issues: &mut Vec<ConfigurationIssue>,
) {
    let Some(target_index) = visible_layers(owner.layer).iter().find_map(|layer| {
        index
            .get(&(*layer, expected_kind, target_id.to_string()))
            .copied()
    }) else {
        issues.push(reference_issue(
            ConfigurationIssueCode::ReferenceNotFound,
            owner,
            "A referenced document id does not exist in a visible layer",
            "Add the referenced document or correct the id",
        ));
        return;
    };
    let target = &parsed[target_index];
    entries.push(BundleReference {
        path: target.parsed.source_path().as_str().to_string(),
        expected_kind: expected_kind.as_str().to_string(),
        resolved_id: target_id.to_string(),
    });
}

fn resolve_class_references(
    parsed: &[ParsedLayerDocument],
    index: &BTreeMap<(ValueLayer, DocumentKind, String), usize>,
    owner: &ParsedLayerDocument,
    class: &ClassRuleV1,
    entries: &mut Vec<BundleReference>,
    issues: &mut Vec<ConfigurationIssue>,
) {
    for agent in &class.agents {
        resolve_id_reference(
            parsed,
            index,
            owner,
            agent,
            DocumentKind::Agent,
            entries,
            issues,
        );
    }
    if let Some(review) = &class.review {
        resolve_id_reference(
            parsed,
            index,
            owner,
            &review.agent,
            DocumentKind::Agent,
            entries,
            issues,
        );
    }
}

fn purpose_matches(parsed: &ParsedDocumentV1, expected: Option<PromptPurposeV1>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    matches!(
        parsed,
        ParsedDocumentV1::Prompt { document, .. } if document.purpose == expected
    )
}

fn visible_layers(layer: ValueLayer) -> &'static [ValueLayer] {
    match layer {
        ValueLayer::Instructions => &[ValueLayer::Instructions],
        ValueLayer::Project => &[ValueLayer::Project, ValueLayer::Instructions],
        ValueLayer::Personal => &[
            ValueLayer::Personal,
            ValueLayer::Project,
            ValueLayer::Instructions,
        ],
        ValueLayer::Settings => &[ValueLayer::Settings],
    }
}

fn reference_issue(
    code: ConfigurationIssueCode,
    owner: &ParsedLayerDocument,
    explanation: &str,
    action: &str,
) -> ConfigurationIssue {
    issue(
        code,
        owner.parsed.source_path().clone(),
        None,
        ConfigurationStage::Reference,
        explanation,
        action,
        None,
    )
}

fn compile_agents(
    parsed: &[ParsedLayerDocument],
    input: &CompileBundleInput,
) -> Result<CompiledAgents, DiagnosticSet> {
    let mut agents = BTreeMap::new();
    let mut origins = BTreeMap::new();
    let mut issues = Vec::new();
    for layer in [
        ValueLayer::Instructions,
        ValueLayer::Project,
        ValueLayer::Personal,
    ] {
        for source in parsed.iter().filter(|document| document.layer == layer) {
            let ParsedDocumentV1::Agent { document, body, .. } = &source.parsed else {
                continue;
            };
            let Some(candidate) = convert_agent(document, body, source, input, &mut issues) else {
                continue;
            };
            if let Some(base) = agents.get(&candidate.id) {
                issues.extend(validate_agent_override(
                    base,
                    &candidate,
                    source.parsed.source_path(),
                ));
            }
            let id = candidate.id.clone();
            agents.insert(id.clone(), candidate);
            for field in [
                "",
                "/accessMode",
                "/tools",
                "/environments",
                "/requiredCapabilities",
                "/instructions",
            ] {
                origins.insert(
                    format!("/agents/{}{}", escape_pointer(id.as_str()), field),
                    origin(source, if field.is_empty() { "/" } else { field }),
                );
            }
        }
    }
    finish(issues)?;
    Ok(CompiledAgents { agents, origins })
}

fn convert_agent(
    authored: &AgentDocumentV1,
    body: &str,
    source: &ParsedLayerDocument,
    input: &CompileBundleInput,
    issues: &mut Vec<ConfigurationIssue>,
) -> Option<AgentDefinition> {
    if authored.access_mode == AuthoredAccessMode::FullAccess {
        issues.push(full_access_issue(source.parsed.source_path()));
        return None;
    }
    if !input
        .settings
        .tier_mapping
        .tiers
        .contains_key(&authored.tier)
    {
        issues.push(issue(
            ConfigurationIssueCode::TierUnmapped,
            source.parsed.source_path().clone(),
            Some("/tier"),
            ConfigurationStage::Policy,
            "The agent tier has no trusted settings mapping",
            "Add the tier to trusted settings or choose a mapped tier",
            None,
        ));
        return None;
    }

    let mut prefer = Vec::new();
    let mut seen_prefer = BTreeSet::new();
    for value in &authored.providers.prefer {
        match parse_provider_kind(value) {
            Some(provider) if seen_prefer.insert(provider.as_str()) => prefer.push(provider),
            Some(_) => {}
            None => issues.push(invalid_provider_issue(source, value)),
        }
    }
    let mut avoid = BTreeMap::new();
    for value in &authored.providers.avoid {
        match parse_provider_kind(value) {
            Some(provider) => {
                avoid.insert(provider.as_str().to_string(), provider);
            }
            None => issues.push(invalid_provider_issue(source, value)),
        }
    }

    let (scope, author, project_id) = match source.layer {
        ValueLayer::Instructions => (AgentScope::Team, AgentAuthor::InstructionsRepository, None),
        ValueLayer::Project => (
            AgentScope::Project,
            AgentAuthor::ProjectRepository,
            Some(input.project_id.clone()),
        ),
        ValueLayer::Personal => (
            AgentScope::Personal,
            AgentAuthor::User,
            Some(input.project_id.clone()),
        ),
        ValueLayer::Settings => return None,
    };
    Some(AgentDefinition {
        id: AgentId::new(authored.id.clone()),
        name: authored.name.clone(),
        description: authored.description.clone(),
        instructions: body.trim().to_string(),
        scope,
        author,
        version: "1".to_string(),
        bundle_sha: String::new(),
        source_path: authored_source_key(source),
        organization_id: Some(input.organization_id.clone()),
        project_id,
        specialties: authored.specialties.clone(),
        tier: authored.tier.clone(),
        effort: authored.effort.clone(),
        access_mode: match authored.access_mode {
            AuthoredAccessMode::ReadOnly => AccessMode::ReadOnly,
            AuthoredAccessMode::WorkspaceOnly => AccessMode::WorkspaceOnly,
            AuthoredAccessMode::Ask => AccessMode::Ask,
            AuthoredAccessMode::AutoEdit => AccessMode::AutoEdit,
            AuthoredAccessMode::FullAccess => unreachable!("rejected above"),
        },
        full_access_opt_in: false,
        required_capabilities: authored.required_capabilities.clone(),
        providers: AgentProviderPolicy {
            prefer,
            avoid: avoid.into_values().collect(),
        },
        environments: authored.environments.clone(),
        knowledge: authored.knowledge.clone(),
        tools: AgentToolPolicy {
            allow: authored.tools.allow.clone(),
            deny: authored.tools.deny.clone(),
        },
        output: OutputContract {
            kind: authored.output.kind.clone(),
            schema: None,
        },
        verification: authored.verification.clone(),
        concurrency: authored.concurrency,
        budget: AgentBudget {
            max_tokens: authored.budget.max_tokens,
            max_cost_micro_usd: authored.budget.max_cost_micro_usd,
            max_runtime_ms: authored.budget.max_runtime_ms,
        },
        enabled: true,
    })
}

fn invalid_provider_issue(source: &ParsedLayerDocument, _provider: &str) -> ConfigurationIssue {
    issue(
        ConfigurationIssueCode::SchemaInvalidValue,
        source.parsed.source_path().clone(),
        Some("/providers"),
        ConfigurationStage::Schema,
        "The agent names an unsupported provider kind",
        "Use claude, codex, antigravity, cursor, grok, opencode, or custom",
        None,
    )
}

fn parse_provider_kind(value: &str) -> Option<ProviderKind> {
    match value {
        "claude" => Some(ProviderKind::Claude),
        "codex" => Some(ProviderKind::Codex),
        "antigravity" => Some(ProviderKind::Antigravity),
        "cursor" => Some(ProviderKind::Cursor),
        "grok" => Some(ProviderKind::Grok),
        "opencode" => Some(ProviderKind::OpenCode),
        "custom" => Some(ProviderKind::Custom),
        _ => None,
    }
}

fn compile_prompts(
    parsed: &[ParsedLayerDocument],
    origins: &mut BTreeMap<String, ValueOrigin>,
) -> BTreeMap<String, CompiledPrompt> {
    let mut prompts = BTreeMap::new();
    for source in parsed {
        let ParsedDocumentV1::Prompt { document, body, .. } = &source.parsed else {
            continue;
        };
        prompts.insert(
            document.id.clone(),
            CompiledPrompt {
                id: document.id.clone(),
                purpose: match document.purpose {
                    PromptPurposeV1::Intake => PromptPurpose::Intake,
                    PromptPurposeV1::Planner => PromptPurpose::Planner,
                    PromptPurposeV1::Handoff => PromptPurpose::Handoff,
                },
                content: body.trim().to_string(),
                source_path: authored_source_key(source),
            },
        );
        origins.insert(
            format!("/prompts/{}", escape_pointer(&document.id)),
            origin(source, "/"),
        );
    }
    prompts
}

fn compile_knowledge(
    parsed: &[ParsedLayerDocument],
    origins: &mut BTreeMap<String, ValueOrigin>,
) -> BTreeMap<String, CompiledKnowledge> {
    let mut knowledge = BTreeMap::new();
    for layer in [
        ValueLayer::Instructions,
        ValueLayer::Project,
        ValueLayer::Personal,
    ] {
        for source in parsed.iter().filter(|document| document.layer == layer) {
            let ParsedDocumentV1::Knowledge { document, body, .. } = &source.parsed else {
                continue;
            };
            knowledge.insert(
                document.id.clone(),
                CompiledKnowledge {
                    id: document.id.clone(),
                    title: document.title.clone(),
                    tags: document.tags.clone(),
                    content: body.trim().to_string(),
                    source_path: authored_source_key(source),
                },
            );
            origins.insert(
                format!("/knowledge/{}", escape_pointer(&document.id)),
                origin(source, "/"),
            );
        }
    }
    knowledge
}

fn compile_classes(
    base: &[ClassRuleV1],
    overrides: Option<&[ClassRuleV1]>,
    agents: &BTreeMap<AgentId, AgentDefinition>,
    source_path: &RelativeConfigPath,
) -> Result<Vec<CompiledClassRule>, DiagnosticSet> {
    let mut order = Vec::new();
    let mut classes = BTreeMap::new();
    for class in base {
        order.push(class.id.clone());
        classes.insert(class.id.clone(), class.clone());
    }
    for class in overrides.unwrap_or_default() {
        if !classes.contains_key(&class.id) {
            order.push(class.id.clone());
        }
        classes.insert(class.id.clone(), class.clone());
    }
    let mut compiled = Vec::new();
    let mut issues = Vec::new();
    for id in order {
        let class = classes.get(&id).expect("class order and map agree");
        let class_agents = class
            .agents
            .iter()
            .map(|id| AgentId::new(id.clone()))
            .collect::<Vec<_>>();
        if class_agents.iter().any(|id| !agents.contains_key(id)) {
            issues.push(issue(
                ConfigurationIssueCode::ReferenceNotFound,
                source_path.clone(),
                Some("/classes"),
                ConfigurationStage::Reference,
                "A class rule references an agent that is not in the effective bundle",
                "Reference a compiled agent id",
                None,
            ));
        }
        let review_agent = class
            .review
            .as_ref()
            .map(|review| AgentId::new(review.agent.clone()));
        if review_agent
            .as_ref()
            .is_some_and(|id| !agents.contains_key(id))
        {
            issues.push(issue(
                ConfigurationIssueCode::ReferenceNotFound,
                source_path.clone(),
                Some("/classes/review/agent"),
                ConfigurationStage::Reference,
                "A class review rule references an agent that is not in the effective bundle",
                "Reference a compiled agent id",
                None,
            ));
        }
        compiled.push(CompiledClassRule {
            id: class.id.clone(),
            agents: class_agents,
            match_keywords: class
                .match_rule
                .as_ref()
                .map(|value| value.keywords.iter().cloned().collect())
                .unwrap_or_default(),
            max_files_hint: class
                .match_rule
                .as_ref()
                .and_then(|value| value.max_files_hint),
            decompose: class.decompose,
            require_approval: class.require_approval,
            review_agent,
            review_when: class.review.as_ref().map(|review| review.when.clone()),
        });
    }
    finish(issues)?;
    Ok(compiled)
}

fn origin(source: &ParsedLayerDocument, field: &str) -> ValueOrigin {
    ValueOrigin {
        layer: source.layer,
        logical_path: source.parsed.source_path().as_str().to_string(),
        field: field.to_string(),
    }
}

fn authored_source_key(source: &ParsedLayerDocument) -> String {
    format!(
        "{}:{}",
        layer_label(source.layer),
        source.parsed.source_path()
    )
}

const fn layer_label(layer: ValueLayer) -> &'static str {
    match layer {
        ValueLayer::Instructions => "instructions",
        ValueLayer::Project => "project",
        ValueLayer::Personal => "personal",
        ValueLayer::Settings => "settings",
    }
}

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn unsigned_bundle_digest(bundle: &CompiledInstructionBundleV1) -> String {
    let mut unsigned = bundle.clone();
    unsigned.payload_sha256.clear();
    for agent in unsigned.agents.values_mut() {
        agent.bundle_sha.clear();
    }
    let bytes = canonical_json_bytes(&unsigned);
    hex_digest(Sha256::digest(bytes).as_slice())
}

fn hash_len_and_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(bytes);
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn finish(issues: Vec<ConfigurationIssue>) -> Result<(), DiagnosticSet> {
    if issues.is_empty() {
        Ok(())
    } else {
        Err(DiagnosticSet::from_issues(issues))
    }
}
