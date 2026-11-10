pub mod v1;

use crate::diagnostics::{
    ConfigurationIssue, ConfigurationIssueCode, ConfigurationStage, DiagnosticSet, JsonPointer,
    RelativeConfigPath, issue,
};
use schemars::schema_for;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use serde_saphyr::{Budget, Error as YamlError, MergeKeyPolicy, Options};
use v1::{
    AgentDocumentV1, AuthoredSchemaCatalog, DocumentKind, KnowledgeDocumentV1,
    OrchestratorDocumentV1, ProjectDocumentV1, PromptDocumentV1, SCHEMA_VERSION_V1,
};

pub const MAX_SOURCE_FILES: usize = 1_024;
pub const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_YAML_FILE_BYTES: usize = 256 * 1024;
pub const MAX_MARKDOWN_FILE_BYTES: usize = 512 * 1024;
pub const MAX_FRONTMATTER_BYTES: usize = 64 * 1024;
pub const MAX_YAML_DEPTH: usize = 32;
pub const MAX_CONTAINER_ITEMS: usize = 256;
pub const MAX_SCALAR_BYTES: usize = 32 * 1024;
pub const MAX_LOGICAL_PATH_BYTES: usize = 240;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDocument {
    logical_path: RelativeConfigPath,
    bytes: Vec<u8>,
}

impl SourceDocument {
    pub fn new(
        logical_path: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<Self, DiagnosticSet> {
        let logical_path = RelativeConfigPath::new(logical_path).map_err(|_| {
            DiagnosticSet::one(issue(
                ConfigurationIssueCode::ReferencePathEscape,
                RelativeConfigPath::invalid(),
                None,
                ConfigurationStage::Reference,
                "The source document path is not a confined bundle-relative path",
                "Use a relative path without drive prefixes, backslashes, empty segments, dot segments, or parent traversal",
                None,
            ))
        })?;
        Ok(Self {
            logical_path,
            bytes: bytes.into(),
        })
    }

    pub fn logical_path(&self) -> &RelativeConfigPath {
        &self.logical_path
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedDocumentV1 {
    Orchestrator {
        source_path: RelativeConfigPath,
        document: Box<OrchestratorDocumentV1>,
    },
    Project {
        source_path: RelativeConfigPath,
        document: Box<ProjectDocumentV1>,
    },
    Agent {
        source_path: RelativeConfigPath,
        document: Box<AgentDocumentV1>,
        body: String,
    },
    Knowledge {
        source_path: RelativeConfigPath,
        document: Box<KnowledgeDocumentV1>,
        body: String,
    },
    Prompt {
        source_path: RelativeConfigPath,
        document: Box<PromptDocumentV1>,
        body: String,
    },
}

impl ParsedDocumentV1 {
    pub const fn kind(&self) -> DocumentKind {
        match self {
            Self::Orchestrator { .. } => DocumentKind::Orchestrator,
            Self::Project { .. } => DocumentKind::Project,
            Self::Agent { .. } => DocumentKind::Agent,
            Self::Knowledge { .. } => DocumentKind::Knowledge,
            Self::Prompt { .. } => DocumentKind::Prompt,
        }
    }

    pub fn source_path(&self) -> &RelativeConfigPath {
        match self {
            Self::Orchestrator { source_path, .. }
            | Self::Project { source_path, .. }
            | Self::Agent { source_path, .. }
            | Self::Knowledge { source_path, .. }
            | Self::Prompt { source_path, .. } => source_path,
        }
    }

    pub fn document_id(&self) -> Option<&str> {
        match self {
            Self::Agent { document, .. } => Some(&document.id),
            Self::Knowledge { document, .. } => Some(&document.id),
            Self::Prompt { document, .. } => Some(&document.id),
            Self::Orchestrator { .. } | Self::Project { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceKind {
    Orchestrator,
    Project,
    Agent,
    Knowledge,
    Prompt,
}

impl SourceKind {
    const fn document_kind(self) -> DocumentKind {
        match self {
            Self::Orchestrator => DocumentKind::Orchestrator,
            Self::Project => DocumentKind::Project,
            Self::Agent => DocumentKind::Agent,
            Self::Knowledge => DocumentKind::Knowledge,
            Self::Prompt => DocumentKind::Prompt,
        }
    }

    const fn max_file_bytes(self) -> usize {
        match self {
            Self::Orchestrator | Self::Project => MAX_YAML_FILE_BYTES,
            Self::Agent | Self::Knowledge | Self::Prompt => MAX_MARKDOWN_FILE_BYTES,
        }
    }

    const fn is_markdown(self) -> bool {
        matches!(self, Self::Agent | Self::Knowledge | Self::Prompt)
    }
}

pub fn validate_source_set_bounds(documents: &[SourceDocument]) -> Result<(), DiagnosticSet> {
    let mut issues = Vec::new();
    if documents.len() > MAX_SOURCE_FILES {
        issues.push(issue(
            ConfigurationIssueCode::ConfigFileCountLimit,
            RelativeConfigPath::invalid(),
            None,
            ConfigurationStage::Bounds,
            "The source set contains more than 1024 files",
            "Reduce the source set to 1024 files or fewer",
            None,
        ));
    }

    let aggregate_bytes = documents
        .iter()
        .try_fold(0usize, |total, document| {
            total.checked_add(document.bytes.len())
        })
        .unwrap_or(usize::MAX);
    if aggregate_bytes > MAX_SOURCE_BYTES {
        issues.push(issue(
            ConfigurationIssueCode::ConfigBundleSizeLimit,
            RelativeConfigPath::invalid(),
            None,
            ConfigurationStage::Bounds,
            "The source set is larger than 16 MiB",
            "Reduce the aggregate source size to 16 MiB or less",
            None,
        ));
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(DiagnosticSet::from_issues(issues))
    }
}

pub fn parse_source_document(source: &SourceDocument) -> Result<ParsedDocumentV1, DiagnosticSet> {
    let source_kind = classify_source_path(source.logical_path.as_str()).ok_or_else(|| {
        DiagnosticSet::one(issue(
            ConfigurationIssueCode::SchemaKindMismatch,
            source.logical_path.clone(),
            Some("/kind"),
            ConfigurationStage::Schema,
            "The logical path does not identify a supported authored document kind",
            "Use orchestrator.yaml, project.yaml, or a markdown file under agents, knowledge, or prompts",
            None,
        ))
    })?;

    if source.bytes.len() > source_kind.max_file_bytes() {
        return Err(DiagnosticSet::one(issue(
            ConfigurationIssueCode::ConfigFileTooLarge,
            source.logical_path.clone(),
            None,
            ConfigurationStage::Bounds,
            "The source document exceeds its file-size limit",
            "Reduce YAML files to 256 KiB or markdown files to 512 KiB",
            None,
        )));
    }
    if source.bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(DiagnosticSet::one(issue(
            ConfigurationIssueCode::ConfigBomUnsupported,
            source.logical_path.clone(),
            None,
            ConfigurationStage::Text,
            "UTF-8 byte order marks are not supported",
            "Save the source as UTF-8 without a byte order mark",
            Some((1, 1)),
        )));
    }
    if source.bytes.contains(&0) {
        return Err(DiagnosticSet::one(issue(
            ConfigurationIssueCode::ConfigNulUnsupported,
            source.logical_path.clone(),
            None,
            ConfigurationStage::Text,
            "NUL bytes are not supported in configuration sources",
            "Remove the NUL byte and validate the source again",
            None,
        )));
    }

    let text = std::str::from_utf8(&source.bytes).map_err(|_| {
        DiagnosticSet::one(issue(
            ConfigurationIssueCode::ConfigNonUtf8,
            source.logical_path.clone(),
            None,
            ConfigurationStage::Text,
            "The source document is not valid UTF-8",
            "Encode the source as UTF-8 without a byte order mark",
            None,
        ))
    })?;
    let normalized = text.replace("\r\n", "\n");

    let (yaml, body) = if source_kind.is_markdown() {
        let (frontmatter, body) = split_frontmatter(&normalized, &source.logical_path)?;
        if frontmatter.len() > MAX_FRONTMATTER_BYTES {
            return Err(DiagnosticSet::one(issue(
                ConfigurationIssueCode::ConfigFileTooLarge,
                source.logical_path.clone(),
                None,
                ConfigurationStage::Bounds,
                "The markdown frontmatter exceeds 64 KiB",
                "Reduce the frontmatter to 64 KiB or less",
                None,
            )));
        }
        (frontmatter, Some(body))
    } else {
        (normalized.as_str(), None)
    };

    let value = parse_yaml_value(yaml, &source.logical_path)?;
    validate_value_bounds(&value, &source.logical_path)?;
    probe_envelope(&value, source_kind.document_kind(), &source.logical_path)?;

    let parsed = match source_kind {
        SourceKind::Orchestrator => {
            let document = decode_v1::<OrchestratorDocumentV1>(yaml, &source.logical_path)?;
            let issues = validate_orchestrator(&document, &source.logical_path);
            finish_validation(issues)?;
            ParsedDocumentV1::Orchestrator {
                source_path: source.logical_path.clone(),
                document: Box::new(document),
            }
        }
        SourceKind::Project => {
            let document = decode_v1::<ProjectDocumentV1>(yaml, &source.logical_path)?;
            let issues = validate_project(&document, &source.logical_path);
            finish_validation(issues)?;
            ParsedDocumentV1::Project {
                source_path: source.logical_path.clone(),
                document: Box::new(document),
            }
        }
        SourceKind::Agent => {
            let document = decode_v1::<AgentDocumentV1>(yaml, &source.logical_path)?;
            let issues = validate_agent(&document, &source.logical_path);
            finish_validation(issues)?;
            ParsedDocumentV1::Agent {
                source_path: source.logical_path.clone(),
                document: Box::new(document),
                body: body.expect("markdown body checked").to_string(),
            }
        }
        SourceKind::Knowledge => {
            let document = decode_v1::<KnowledgeDocumentV1>(yaml, &source.logical_path)?;
            let issues = validate_knowledge(&document, &source.logical_path);
            finish_validation(issues)?;
            ParsedDocumentV1::Knowledge {
                source_path: source.logical_path.clone(),
                document: Box::new(document),
                body: body.expect("markdown body checked").to_string(),
            }
        }
        SourceKind::Prompt => {
            let document = decode_v1::<PromptDocumentV1>(yaml, &source.logical_path)?;
            let issues = validate_prompt(&document, &source.logical_path);
            finish_validation(issues)?;
            ParsedDocumentV1::Prompt {
                source_path: source.logical_path.clone(),
                document: Box::new(document),
                body: body.expect("markdown body checked").to_string(),
            }
        }
    };
    Ok(parsed)
}

pub fn generated_authored_schema_json() -> Value {
    let schema = schema_for!(AuthoredSchemaCatalog);
    let mut value = serde_json::to_value(schema).expect("Schemars output is serializable");
    let limits = json!({
        "aggregateSourceBytes": MAX_SOURCE_BYTES,
        "diagnostics": crate::diagnostics::MAX_DIAGNOSTICS,
        "files": MAX_SOURCE_FILES,
        "frontmatterBytes": MAX_FRONTMATTER_BYTES,
        "logicalPathBytes": MAX_LOGICAL_PATH_BYTES,
        "mappingOrSequenceItems": MAX_CONTAINER_ITEMS,
        "markdownFileBytes": MAX_MARKDOWN_FILE_BYTES,
        "safeTextBytes": crate::diagnostics::MAX_SAFE_TEXT_BYTES,
        "scalarBytes": MAX_SCALAR_BYTES,
        "yamlDepth": MAX_YAML_DEPTH,
        "yamlFileBytes": MAX_YAML_FILE_BYTES,
    });
    value
        .as_object_mut()
        .expect("Schemars root is an object")
        .insert("x-pandamux-limits".to_string(), limits);
    value
}

fn classify_source_path(path: &str) -> Option<SourceKind> {
    match path {
        "orchestrator.yaml" => Some(SourceKind::Orchestrator),
        "project.yaml" => Some(SourceKind::Project),
        _ if nested_markdown(path, "agents/") => Some(SourceKind::Agent),
        _ if nested_markdown(path, "knowledge/") => Some(SourceKind::Knowledge),
        _ if nested_markdown(path, "prompts/") => Some(SourceKind::Prompt),
        _ => None,
    }
}

fn nested_markdown(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|remainder| !remainder.contains('/') && remainder.ends_with(".md"))
}

fn split_frontmatter<'a>(
    text: &'a str,
    path: &RelativeConfigPath,
) -> Result<(&'a str, &'a str), DiagnosticSet> {
    let Some(rest) = text.strip_prefix("---\n") else {
        return Err(DiagnosticSet::one(issue(
            ConfigurationIssueCode::FrontmatterMissing,
            path.clone(),
            None,
            ConfigurationStage::Frontmatter,
            "Markdown frontmatter must begin at byte zero",
            "Add an opening line containing exactly three hyphens at byte zero",
            Some((1, 1)),
        )));
    };

    let mut frontmatter_end = None;
    let mut body_start = None;
    let mut offset = 4;
    for line in rest.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if content == "---" {
            frontmatter_end = Some(offset);
            body_start = Some(offset + line.len());
            break;
        }
        offset += line.len();
    }
    if frontmatter_end.is_none() && rest.ends_with("---") {
        let start = text.len() - 3;
        if start == 4 || text.as_bytes().get(start.wrapping_sub(1)) == Some(&b'\n') {
            frontmatter_end = Some(start);
            body_start = Some(text.len());
        }
    }

    let (Some(frontmatter_end), Some(body_start)) = (frontmatter_end, body_start) else {
        return Err(DiagnosticSet::one(issue(
            ConfigurationIssueCode::FrontmatterUnterminated,
            path.clone(),
            None,
            ConfigurationStage::Frontmatter,
            "Markdown frontmatter has no exact closing delimiter",
            "Add a closing line containing exactly three hyphens",
            None,
        )));
    };
    let body = &text[body_start..];
    if body.trim().is_empty() {
        return Err(DiagnosticSet::one(issue(
            ConfigurationIssueCode::FrontmatterEmptyBody,
            path.clone(),
            None,
            ConfigurationStage::Frontmatter,
            "Markdown authored documents require a nonempty body",
            "Add non-whitespace markdown content after the closing delimiter",
            None,
        )));
    }
    Ok((&text[4..frontmatter_end], body))
}

fn yaml_options() -> Options {
    let mut budget = Budget::default();
    budget.max_depth = MAX_YAML_DEPTH;
    budget.max_documents = 1;
    budget.max_total_scalar_bytes = MAX_YAML_FILE_BYTES;
    budget.max_aliases = 0;
    budget.max_anchors = 0;
    budget.max_merge_keys = 0;

    let mut options = Options::default();
    options.budget = Some(budget);
    options.emit_comments = false;
    options.merge_keys = MergeKeyPolicy::Error;
    options.strict_booleans = true;
    options.reject_unsupported_tags = true;
    options.with_snippet = false;
    options
}

fn parse_yaml_value(yaml: &str, path: &RelativeConfigPath) -> Result<Value, DiagnosticSet> {
    serde_saphyr::from_str_with_options(yaml, yaml_options())
        .map_err(|error| map_yaml_error(&error, path))
}

fn decode_v1<T: DeserializeOwned>(
    yaml: &str,
    path: &RelativeConfigPath,
) -> Result<T, DiagnosticSet> {
    serde_saphyr::from_str_with_options(yaml, yaml_options())
        .map_err(|error| map_yaml_error(&error, path))
}

fn map_yaml_error(error: &YamlError, path: &RelativeConfigPath) -> DiagnosticSet {
    use serde_saphyr::budget::BudgetBreach;

    let error = error.without_snippet();
    let (code, stage, explanation, action) = match error {
        YamlError::MultipleDocuments { .. }
        | YamlError::Budget {
            breach: BudgetBreach::Documents { .. },
            ..
        } => (
            ConfigurationIssueCode::YamlMultipleDocuments,
            ConfigurationStage::Yaml,
            "Only one YAML document is allowed",
            "Remove additional YAML document separators and documents",
        ),
        YamlError::DuplicateMappingKey { .. } => (
            ConfigurationIssueCode::YamlDuplicateKey,
            ConfigurationStage::Yaml,
            "Duplicate YAML mapping keys are not allowed",
            "Keep one value for each mapping key",
        ),
        YamlError::UnsupportedTag { .. }
        | YamlError::MergeKeyNotAllowed { .. }
        | YamlError::AliasReplayCounterOverflow { .. }
        | YamlError::AliasReplayLimitExceeded { .. }
        | YamlError::AliasExpansionLimitExceeded { .. }
        | YamlError::AliasReplayStackDepthExceeded { .. }
        | YamlError::UnknownAnchor { .. }
        | YamlError::Budget {
            breach: BudgetBreach::Aliases { .. } | BudgetBreach::Anchors { .. },
            ..
        } => (
            ConfigurationIssueCode::YamlUnsupportedTag,
            ConfigurationStage::Yaml,
            "YAML tags, merges, anchors, and aliases are not supported",
            "Replace the construct with explicit mappings, sequences, and scalar values",
        ),
        YamlError::Budget {
            breach: BudgetBreach::Depth { .. },
            ..
        } => (
            ConfigurationIssueCode::YamlDepthLimit,
            ConfigurationStage::Yaml,
            "YAML nesting exceeds 32 levels",
            "Flatten the document to 32 nested mappings or sequences or fewer",
        ),
        YamlError::SerdeUnknownField { .. } => (
            ConfigurationIssueCode::SchemaUnknownField,
            ConfigurationStage::Schema,
            "The document contains a field that is not part of schema version 1",
            "Remove the unknown field or use a supported schema version",
        ),
        _ => (
            ConfigurationIssueCode::SchemaInvalidValue,
            ConfigurationStage::Schema,
            "The document is not valid bounded YAML for its schema",
            "Correct the YAML value and validate the document again",
        ),
    };
    let location = error.location().map(|location| {
        (
            u32::try_from(location.line()).unwrap_or(u32::MAX),
            u32::try_from(location.column()).unwrap_or(u32::MAX),
        )
    });
    DiagnosticSet::one(issue(
        code,
        path.clone(),
        None,
        stage,
        explanation,
        action,
        location,
    ))
}

fn probe_envelope(
    value: &Value,
    expected_kind: DocumentKind,
    path: &RelativeConfigPath,
) -> Result<(), DiagnosticSet> {
    let Some(object) = value.as_object() else {
        return Err(schema_issue(
            ConfigurationIssueCode::SchemaInvalidValue,
            path,
            None,
            "The document root must be a mapping",
            "Use a YAML mapping with schemaVersion and kind fields",
        ));
    };

    match object.get("schemaVersion") {
        None => {
            return Err(schema_issue(
                ConfigurationIssueCode::SchemaVersionMissing,
                path,
                Some("/schemaVersion"),
                "The required schemaVersion field is missing",
                "Set schemaVersion to integer 1",
            ));
        }
        Some(Value::Number(number)) if number.as_u64() == Some(SCHEMA_VERSION_V1.into()) => {}
        Some(Value::Number(number)) if number.as_u64().is_some_and(|value| value > 1) => {
            return Err(schema_issue(
                ConfigurationIssueCode::SchemaVersionUnsupported,
                path,
                Some("/schemaVersion"),
                "The schema version is newer than this PandaMUX parser supports",
                "Use schemaVersion 1 or upgrade PandaMUX",
            ));
        }
        Some(_) => {
            return Err(schema_issue(
                ConfigurationIssueCode::SchemaVersionInvalid,
                path,
                Some("/schemaVersion"),
                "schemaVersion must be integer 1",
                "Set schemaVersion to integer 1 without quotes",
            ));
        }
    }

    match object.get("kind") {
        None => Err(schema_issue(
            ConfigurationIssueCode::SchemaKindMissing,
            path,
            Some("/kind"),
            "The required kind field is missing",
            "Set kind to the exact kind required by the logical path",
        )),
        Some(Value::String(kind)) if kind == expected_kind.as_str() => Ok(()),
        Some(_) => Err(schema_issue(
            ConfigurationIssueCode::SchemaKindMismatch,
            path,
            Some("/kind"),
            "The document kind does not match its logical path",
            "Use the exact authored kind required by the logical path",
        )),
    }
}

fn schema_issue(
    code: ConfigurationIssueCode,
    path: &RelativeConfigPath,
    field: Option<&str>,
    explanation: &str,
    action: &str,
) -> DiagnosticSet {
    DiagnosticSet::one(issue(
        code,
        path.clone(),
        field,
        ConfigurationStage::Schema,
        explanation,
        action,
        None,
    ))
}

fn validate_value_bounds(value: &Value, path: &RelativeConfigPath) -> Result<(), DiagnosticSet> {
    let mut issues = Vec::new();
    walk_value_bounds(value, "", path, &mut issues);
    finish_validation(issues)
}

fn walk_value_bounds(
    value: &Value,
    pointer: &str,
    path: &RelativeConfigPath,
    issues: &mut Vec<ConfigurationIssue>,
) {
    match value {
        Value::Object(object) => {
            if object.len() > MAX_CONTAINER_ITEMS {
                issues.push(bound_issue(path, pointer, "mapping"));
            }
            for (key, nested) in object {
                if key.len() > MAX_SCALAR_BYTES {
                    issues.push(scalar_issue(path, pointer));
                }
                let nested_pointer = join_pointer(pointer, key);
                walk_value_bounds(nested, &nested_pointer, path, issues);
            }
        }
        Value::Array(values) => {
            if values.len() > MAX_CONTAINER_ITEMS {
                issues.push(bound_issue(path, pointer, "sequence"));
            }
            for (index, nested) in values.iter().enumerate() {
                let nested_pointer = join_pointer(pointer, &index.to_string());
                walk_value_bounds(nested, &nested_pointer, path, issues);
            }
        }
        Value::String(value) if value.len() > MAX_SCALAR_BYTES => {
            issues.push(scalar_issue(path, pointer));
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn bound_issue(path: &RelativeConfigPath, pointer: &str, container: &str) -> ConfigurationIssue {
    issue(
        ConfigurationIssueCode::SchemaInvalidValue,
        path.clone(),
        JsonPointer::new(pointer)
            .map(|value| value.as_str().to_string())
            .as_deref(),
        ConfigurationStage::Schema,
        &format!("A YAML {container} contains more than 256 items"),
        "Reduce every mapping and sequence to 256 items or fewer",
        None,
    )
}

fn scalar_issue(path: &RelativeConfigPath, pointer: &str) -> ConfigurationIssue {
    issue(
        ConfigurationIssueCode::SchemaInvalidValue,
        path.clone(),
        JsonPointer::new(pointer)
            .map(|value| value.as_str().to_string())
            .as_deref(),
        ConfigurationStage::Schema,
        "A YAML scalar exceeds 32 KiB",
        "Reduce every scalar to 32 KiB or less",
        None,
    )
}

fn join_pointer(parent: &str, segment: &str) -> String {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    format!("{parent}/{escaped}")
}

fn validate_orchestrator(
    document: &OrchestratorDocumentV1,
    path: &RelativeConfigPath,
) -> Vec<ConfigurationIssue> {
    let mut issues = Vec::new();
    validate_exact_envelope(
        document.schema_version,
        document.kind,
        DocumentKind::Orchestrator,
        path,
        &mut issues,
    );
    match parse_numeric_version(&document.min_pandamux_version) {
        None => issues.push(issue(
            ConfigurationIssueCode::SchemaInvalidValue,
            path.clone(),
            Some("/minPandamuxVersion"),
            ConfigurationStage::Schema,
            "minPandamuxVersion must contain three numeric components",
            "Use a stable MAJOR.MINOR.PATCH version",
            None,
        )),
        Some(required) => {
            let current = parse_numeric_version(env!("CARGO_PKG_VERSION"))
                .expect("crate version is stable numeric MAJOR.MINOR.PATCH");
            if required > current {
                issues.push(issue(
                    ConfigurationIssueCode::MinimumVersionUnsupported,
                    path.clone(),
                    Some("/minPandamuxVersion"),
                    ConfigurationStage::Compilation,
                    "The document requires a newer PandaMUX version",
                    "Upgrade PandaMUX or lower the requirement only when the authored features remain compatible",
                    None,
                ));
            }
        }
    }
    validate_identifier(
        &document.defaults.default_agent,
        path,
        "/defaults/defaultAgent",
        &mut issues,
    );
    if document.defaults.max_parallel_tasks == 0 || document.defaults.max_parallel_tasks > 64 {
        issues.push(invalid_value_issue(
            path,
            "/defaults/maxParallelTasks",
            "maxParallelTasks must be between 1 and 64",
        ));
    }
    if document.defaults.max_tasks_per_run == 0 || document.defaults.max_tasks_per_run > 64 {
        issues.push(invalid_value_issue(
            path,
            "/defaults/maxTasksPerRun",
            "maxTasksPerRun must be between 1 and 64",
        ));
    }
    if document.defaults.max_attempts_per_task == 0 || document.defaults.max_attempts_per_task > 8 {
        issues.push(invalid_value_issue(
            path,
            "/defaults/maxAttemptsPerTask",
            "maxAttemptsPerTask must be between 1 and 8",
        ));
    }
    if document.provider_policy.min_remaining_millionths > 1_000_000 {
        issues.push(invalid_value_issue(
            path,
            "/providerPolicy/minRemainingMillionths",
            "minRemainingMillionths must not exceed 1000000",
        ));
    }
    for (provider, concurrency) in &document.provider_policy.concurrency {
        if !valid_identifier(provider) || *concurrency == 0 {
            issues.push(invalid_value_issue(
                path,
                "/providerPolicy/concurrency",
                "Provider concurrency keys must be identifiers and values must be positive",
            ));
        }
    }
    for (field, reference) in [
        ("/prompts/intake", document.prompts.intake.as_str()),
        ("/prompts/planner", document.prompts.planner.as_str()),
        ("/prompts/handoff", document.prompts.handoff.as_str()),
    ] {
        validate_reference(reference, path, field, &mut issues);
    }
    for (index, class) in document.classes.iter().enumerate() {
        validate_identifier(
            &class.id,
            path,
            &format!("/classes/{index}/id"),
            &mut issues,
        );
        for (agent_index, agent) in class.agents.iter().enumerate() {
            validate_identifier(
                agent,
                path,
                &format!("/classes/{index}/agents/{agent_index}"),
                &mut issues,
            );
        }
    }
    issues
}

fn validate_project(
    document: &ProjectDocumentV1,
    path: &RelativeConfigPath,
) -> Vec<ConfigurationIssue> {
    let mut issues = Vec::new();
    validate_exact_envelope(
        document.schema_version,
        document.kind,
        DocumentKind::Project,
        path,
        &mut issues,
    );
    for (index, reference) in document.agents.iter().enumerate() {
        validate_reference(reference, path, &format!("/agents/{index}"), &mut issues);
    }
    for (index, reference) in document.knowledge.iter().enumerate() {
        validate_reference(reference, path, &format!("/knowledge/{index}"), &mut issues);
    }
    issues
}

fn validate_agent(
    document: &AgentDocumentV1,
    path: &RelativeConfigPath,
) -> Vec<ConfigurationIssue> {
    let mut issues = Vec::new();
    validate_exact_envelope(
        document.schema_version,
        document.kind,
        DocumentKind::Agent,
        path,
        &mut issues,
    );
    validate_identifier(&document.id, path, "/id", &mut issues);
    validate_filename_id(&document.id, path, &mut issues);
    validate_identifier(&document.tier, path, "/tier", &mut issues);
    if document.name.trim().is_empty()
        || document.description.trim().is_empty()
        || document.output.kind.trim().is_empty()
        || document.concurrency == 0
    {
        issues.push(invalid_value_issue(
            path,
            "/",
            "Agent name, description, output kind, and positive concurrency are required",
        ));
    }
    for (index, reference) in document.knowledge.iter().enumerate() {
        validate_reference(reference, path, &format!("/knowledge/{index}"), &mut issues);
    }
    issues
}

fn validate_knowledge(
    document: &KnowledgeDocumentV1,
    path: &RelativeConfigPath,
) -> Vec<ConfigurationIssue> {
    let mut issues = Vec::new();
    validate_exact_envelope(
        document.schema_version,
        document.kind,
        DocumentKind::Knowledge,
        path,
        &mut issues,
    );
    validate_identifier(&document.id, path, "/id", &mut issues);
    validate_filename_id(&document.id, path, &mut issues);
    if document.title.trim().is_empty() {
        issues.push(invalid_value_issue(
            path,
            "/title",
            "Knowledge title must be nonempty",
        ));
    }
    issues
}

fn validate_prompt(
    document: &PromptDocumentV1,
    path: &RelativeConfigPath,
) -> Vec<ConfigurationIssue> {
    let mut issues = Vec::new();
    validate_exact_envelope(
        document.schema_version,
        document.kind,
        DocumentKind::Prompt,
        path,
        &mut issues,
    );
    validate_identifier(&document.id, path, "/id", &mut issues);
    validate_filename_id(&document.id, path, &mut issues);
    issues
}

fn validate_exact_envelope(
    version: u32,
    actual: DocumentKind,
    expected: DocumentKind,
    path: &RelativeConfigPath,
    issues: &mut Vec<ConfigurationIssue>,
) {
    if version != SCHEMA_VERSION_V1 {
        issues.push(invalid_value_issue(
            path,
            "/schemaVersion",
            "The decoded schemaVersion must remain 1",
        ));
    }
    if actual != expected {
        issues.push(issue(
            ConfigurationIssueCode::SchemaKindMismatch,
            path.clone(),
            Some("/kind"),
            ConfigurationStage::Schema,
            "The decoded document kind does not match its logical path",
            "Use the exact authored kind required by the logical path",
            None,
        ));
    }
}

fn validate_identifier(
    value: &str,
    path: &RelativeConfigPath,
    field: &str,
    issues: &mut Vec<ConfigurationIssue>,
) {
    if !valid_identifier(value) {
        issues.push(issue(
            ConfigurationIssueCode::DocumentIdInvalid,
            path.clone(),
            Some(field),
            ConfigurationStage::Identity,
            "The identifier is not in normalized lowercase form",
            "Use 1 to 64 lowercase ASCII letters, digits, hyphens, or underscores, beginning with a letter",
            None,
        ));
    }
}

fn valid_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    value.len() <= 64
        && first.is_ascii_lowercase()
        && bytes
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
}

fn validate_filename_id(id: &str, path: &RelativeConfigPath, issues: &mut Vec<ConfigurationIssue>) {
    let filename = path.as_str().rsplit('/').next().unwrap_or_default();
    let stem = filename.strip_suffix(".md").unwrap_or(filename);
    if stem != id {
        issues.push(issue(
            ConfigurationIssueCode::DocumentIdMismatch,
            path.clone(),
            Some("/id"),
            ConfigurationStage::Identity,
            "The document id does not match the logical filename stem",
            "Rename the file or change id so the normalized values match exactly",
            None,
        ));
    }
}

fn validate_reference(
    reference: &str,
    path: &RelativeConfigPath,
    field: &str,
    issues: &mut Vec<ConfigurationIssue>,
) {
    if RelativeConfigPath::new(reference).is_err() {
        issues.push(issue(
            ConfigurationIssueCode::ReferencePathEscape,
            path.clone(),
            Some(field),
            ConfigurationStage::Reference,
            "A reference is not a confined bundle-relative path",
            "Use a relative logical path without drive prefixes, backslashes, empty segments, dot segments, or parent traversal",
            None,
        ));
    }
}

fn invalid_value_issue(
    path: &RelativeConfigPath,
    field: &str,
    explanation: &str,
) -> ConfigurationIssue {
    issue(
        ConfigurationIssueCode::SchemaInvalidValue,
        path.clone(),
        Some(field),
        ConfigurationStage::Schema,
        explanation,
        "Correct the value to satisfy the published schema bounds",
        None,
    )
}

fn finish_validation(issues: Vec<ConfigurationIssue>) -> Result<(), DiagnosticSet> {
    if issues.is_empty() {
        Ok(())
    } else {
        Err(DiagnosticSet::from_issues(issues))
    }
}

fn parse_numeric_version(value: &str) -> Option<(u64, u64, u64)> {
    let mut components = value.split('.');
    let major = parse_version_component(components.next()?)?;
    let minor = parse_version_component(components.next()?)?;
    let patch = parse_version_component(components.next()?)?;
    components.next().is_none().then_some((major, minor, patch))
}

fn parse_version_component(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}
