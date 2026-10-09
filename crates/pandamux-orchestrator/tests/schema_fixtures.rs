use pandamux_orchestrator::diagnostics::ConfigurationIssueCode;
use pandamux_orchestrator::schema::{
    MAX_CONTAINER_ITEMS, MAX_FRONTMATTER_BYTES, MAX_MARKDOWN_FILE_BYTES, MAX_SOURCE_BYTES,
    MAX_SOURCE_FILES, MAX_YAML_DEPTH, MAX_YAML_FILE_BYTES, ParsedDocumentV1, SourceDocument,
    generated_authored_schema_json, parse_source_document, validate_source_set_bounds,
};
use std::collections::BTreeSet;

fn source(path: &str, contents: impl Into<Vec<u8>>) -> SourceDocument {
    SourceDocument::new(path, contents).expect("test logical path is valid")
}

fn fixture(path: &str, name: &str) -> SourceDocument {
    let contents = match name {
        "orchestrator" => include_bytes!("fixtures/schema/orchestrator.yaml").as_slice(),
        "project" => include_bytes!("fixtures/schema/project.yaml").as_slice(),
        "agent" => include_bytes!("fixtures/schema/agent.md").as_slice(),
        "knowledge" => include_bytes!("fixtures/schema/knowledge.md").as_slice(),
        "prompt" => include_bytes!("fixtures/schema/prompt.md").as_slice(),
        _ => panic!("unknown fixture"),
    };
    source(path, contents)
}

fn codes(
    result: Result<ParsedDocumentV1, pandamux_orchestrator::DiagnosticSet>,
) -> Vec<ConfigurationIssueCode> {
    result
        .expect_err("fixture must be rejected")
        .issues()
        .iter()
        .map(|issue| issue.code)
        .collect()
}

fn assert_code(
    result: Result<ParsedDocumentV1, pandamux_orchestrator::DiagnosticSet>,
    expected: ConfigurationIssueCode,
) {
    let actual = codes(result);
    assert!(
        actual.contains(&expected),
        "expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn parses_all_five_authored_document_kinds() {
    assert!(matches!(
        parse_source_document(&fixture("orchestrator.yaml", "orchestrator")),
        Ok(ParsedDocumentV1::Orchestrator { .. })
    ));
    assert!(matches!(
        parse_source_document(&fixture("project.yaml", "project")),
        Ok(ParsedDocumentV1::Project { .. })
    ));
    assert!(matches!(
        parse_source_document(&fixture("agents/reviewer.md", "agent")),
        Ok(ParsedDocumentV1::Agent { .. })
    ));
    assert!(matches!(
        parse_source_document(&fixture("knowledge/review-checklist.md", "knowledge")),
        Ok(ParsedDocumentV1::Knowledge { .. })
    ));
    assert!(matches!(
        parse_source_document(&fixture("prompts/intake.md", "prompt")),
        Ok(ParsedDocumentV1::Prompt { .. })
    ));
}

#[test]
fn normalizes_crlf_before_frontmatter_and_body_parsing() {
    let crlf = String::from_utf8(include_bytes!("fixtures/schema/prompt.md").to_vec())
        .expect("fixture is utf8")
        .replace('\n', "\r\n");
    let parsed = parse_source_document(&source("prompts/intake.md", crlf)).expect("CRLF parses");
    let ParsedDocumentV1::Prompt { body, .. } = parsed else {
        panic!("expected prompt")
    };
    assert!(!body.contains('\r'));
    assert!(body.starts_with("Classify the request"));
}

#[test]
fn rejects_source_set_and_per_file_bounds() {
    let documents = (0..=MAX_SOURCE_FILES)
        .map(|index| source(&format!("knowledge/k{index}.md"), Vec::new()))
        .collect::<Vec<_>>();
    let error = validate_source_set_bounds(&documents).expect_err("file count must fail");
    assert_eq!(
        error.issues()[0].code,
        ConfigurationIssueCode::ConfigFileCountLimit
    );

    let aggregate = vec![source(
        "knowledge/large.md",
        vec![b'a'; MAX_SOURCE_BYTES + 1],
    )];
    let error = validate_source_set_bounds(&aggregate).expect_err("aggregate must fail");
    assert_eq!(
        error.issues()[0].code,
        ConfigurationIssueCode::ConfigBundleSizeLimit
    );

    assert_code(
        parse_source_document(&source(
            "orchestrator.yaml",
            vec![b'a'; MAX_YAML_FILE_BYTES + 1],
        )),
        ConfigurationIssueCode::ConfigFileTooLarge,
    );
    assert_code(
        parse_source_document(&source(
            "agents/large.md",
            vec![b'a'; MAX_MARKDOWN_FILE_BYTES + 1],
        )),
        ConfigurationIssueCode::ConfigFileTooLarge,
    );

    let large_frontmatter = format!(
        "---\nvalue: {}\n---\nbody\n",
        "x".repeat(MAX_FRONTMATTER_BYTES)
    );
    assert_code(
        parse_source_document(&source("knowledge/large.md", large_frontmatter)),
        ConfigurationIssueCode::ConfigFileTooLarge,
    );
}

#[test]
fn rejects_invalid_text_encodings_and_bytes() {
    assert_code(
        parse_source_document(&source("project.yaml", vec![0xff, 0xfe])),
        ConfigurationIssueCode::ConfigNonUtf8,
    );
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            b"\xef\xbb\xbfschemaVersion: 1\nkind: pandamux.project\n".to_vec(),
        )),
        ConfigurationIssueCode::ConfigBomUnsupported,
    );
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            b"schemaVersion: 1\0\nkind: pandamux.project\n".to_vec(),
        )),
        ConfigurationIssueCode::ConfigNulUnsupported,
    );
}

#[test]
fn rejects_frontmatter_failures() {
    assert_code(
        parse_source_document(&source(
            "prompts/intake.md",
            "schemaVersion: 1\nkind: pandamux.prompt\n",
        )),
        ConfigurationIssueCode::FrontmatterMissing,
    );
    assert_code(
        parse_source_document(&source(
            "prompts/intake.md",
            "---\nschemaVersion: 1\nkind: pandamux.prompt\n",
        )),
        ConfigurationIssueCode::FrontmatterUnterminated,
    );
    assert_code(
        parse_source_document(&source(
            "prompts/intake.md",
            "---\nschemaVersion: 1\nkind: pandamux.prompt\nid: intake\npurpose: intake\n---\n \n",
        )),
        ConfigurationIssueCode::FrontmatterEmptyBody,
    );
}

#[test]
fn rejects_unsafe_yaml_constructs_and_depth() {
    let base = "schemaVersion: 1\nkind: pandamux.project\n";
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            format!("{base}agents: []\nagents: []\n"),
        )),
        ConfigurationIssueCode::YamlDuplicateKey,
    );
    assert_code(
        parse_source_document(&source("project.yaml", format!("{base}---\n{base}"))),
        ConfigurationIssueCode::YamlMultipleDocuments,
    );
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            format!("{base}verification: !unsupported value\n"),
        )),
        ConfigurationIssueCode::YamlUnsupportedTag,
    );
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            format!("{base}agents: &shared []\nknowledge: *shared\n"),
        )),
        ConfigurationIssueCode::YamlUnsupportedTag,
    );

    let mut nested = base.to_string();
    for level in 0..=MAX_YAML_DEPTH {
        nested.push_str(&format!("{}level{level}:\n", "  ".repeat(level)));
    }
    nested.push_str(&format!("{}value\n", "  ".repeat(MAX_YAML_DEPTH + 1)));
    assert_code(
        parse_source_document(&source("project.yaml", nested)),
        ConfigurationIssueCode::YamlDepthLimit,
    );
}

#[test]
fn rejects_schema_envelope_and_strict_decode_failures() {
    assert_code(
        parse_source_document(&source("project.yaml", "kind: pandamux.project\n")),
        ConfigurationIssueCode::SchemaVersionMissing,
    );
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            "schemaVersion: \"1\"\nkind: pandamux.project\n",
        )),
        ConfigurationIssueCode::SchemaVersionInvalid,
    );
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            "schemaVersion: 2\nkind: pandamux.project\n",
        )),
        ConfigurationIssueCode::SchemaVersionUnsupported,
    );
    assert_code(
        parse_source_document(&source("project.yaml", "schemaVersion: 1\n")),
        ConfigurationIssueCode::SchemaKindMissing,
    );
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            "schemaVersion: 1\nkind: pandamux.agent\n",
        )),
        ConfigurationIssueCode::SchemaKindMismatch,
    );
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            "schemaVersion: 1\nkind: pandamux.project\nunknown: true\n",
        )),
        ConfigurationIssueCode::SchemaUnknownField,
    );
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            "schemaVersion: 1\nkind: pandamux.project\nagents: wrong\n",
        )),
        ConfigurationIssueCode::SchemaInvalidValue,
    );
}

#[test]
fn rejects_container_and_scalar_bounds() {
    let items = (0..=MAX_CONTAINER_ITEMS)
        .map(|index| format!("  - item-{index}\n"))
        .collect::<String>();
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            format!(
                "schemaVersion: 1\nkind: pandamux.project\nverification:\n{items}agents: []\nknowledge: []\n"
            ),
        )),
        ConfigurationIssueCode::SchemaInvalidValue,
    );

    let scalar = "x".repeat(32 * 1024 + 1);
    assert_code(
        parse_source_document(&source(
            "project.yaml",
            format!(
                "schemaVersion: 1\nkind: pandamux.project\nverification:\n  - {scalar}\nagents: []\nknowledge: []\n"
            ),
        )),
        ConfigurationIssueCode::SchemaInvalidValue,
    );
}

#[test]
fn rejects_identity_reference_and_minimum_version_failures() {
    let invalid_id = include_str!("fixtures/schema/agent.md").replace("id: reviewer", "id: Bad ID");
    assert_code(
        parse_source_document(&source("agents/reviewer.md", invalid_id)),
        ConfigurationIssueCode::DocumentIdInvalid,
    );

    let mismatch = include_str!("fixtures/schema/agent.md").replace("id: reviewer", "id: builder");
    assert_code(
        parse_source_document(&source("agents/reviewer.md", mismatch)),
        ConfigurationIssueCode::DocumentIdMismatch,
    );

    let escape = include_str!("fixtures/schema/agent.md")
        .replace("knowledge/review-checklist.md", "../secret.md");
    assert_code(
        parse_source_document(&source("agents/reviewer.md", escape)),
        ConfigurationIssueCode::ReferencePathEscape,
    );

    let future = include_str!("fixtures/schema/orchestrator.yaml").replace("0.60.67", "999.0.0");
    assert_code(
        parse_source_document(&source("orchestrator.yaml", future)),
        ConfigurationIssueCode::MinimumVersionUnsupported,
    );
}

#[test]
fn generated_schema_round_trips_and_publishes_every_bound() {
    let schema = generated_authored_schema_json();
    let encoded = serde_json::to_vec(&schema).expect("schema serializes");
    let decoded: serde_json::Value = serde_json::from_slice(&encoded).expect("schema parses");
    assert_eq!(decoded, schema);

    let limits = schema
        .get("x-pandamux-limits")
        .and_then(serde_json::Value::as_object)
        .expect("limits extension");
    let expected = [
        ("files", 1_024_u64),
        ("aggregateSourceBytes", 16 * 1024 * 1024),
        ("yamlFileBytes", 256 * 1024),
        ("markdownFileBytes", 512 * 1024),
        ("frontmatterBytes", 64 * 1024),
        ("yamlDepth", 32),
        ("mappingOrSequenceItems", 256),
        ("scalarBytes", 32 * 1024),
        ("logicalPathBytes", 240),
        ("diagnostics", 128),
        ("safeTextBytes", 512),
    ];
    for (name, value) in expected {
        assert_eq!(
            limits.get(name).and_then(serde_json::Value::as_u64),
            Some(value)
        );
    }
}

#[test]
fn diagnostic_codes_are_unique_and_safe_fields_do_not_echo_source() {
    let names = ConfigurationIssueCode::ALL
        .iter()
        .map(|code| code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), ConfigurationIssueCode::ALL.len());

    let secret = "do-not-echo-secret";
    let error = parse_source_document(&source(
        "project.yaml",
        format!("schemaVersion: 1\nkind: pandamux.project\nunknown: {secret}\n"),
    ))
    .expect_err("unknown field fails");
    let encoded = serde_json::to_string(error.issues()).expect("diagnostics serialize");
    assert!(!encoded.contains(secret));
    assert!(!encoded.contains("/home/"));
}
