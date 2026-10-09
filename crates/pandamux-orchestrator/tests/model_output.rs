use pandamux_orchestrator::{
    ModelOutputAttempt, ModelParseErrorCode, ModelParseOutcome, ModelParseState,
    parse_model_attempt,
};
use serde_json::{Value, json};

fn valid_value() -> Value {
    json!({
        "schemaVersion": 1,
        "tasks": [{
            "id": "task-one",
            "summary": "Build the change",
            "class": "build",
            "projectId": "project-one",
            "requiredCapabilities": ["longContext"],
            "accessCeiling": "read_only",
            "inputs": [],
            "outputs": [{"kind": "patch"}],
            "maxAttempts": 2,
            "budgetLimit": {"tokens": 100, "costMicroUsd": 200, "runtimeMs": 300}
        }],
        "edges": []
    })
}

fn accepted(raw: &str) -> pandamux_orchestrator::ProposedPlanV1 {
    match parse_model_attempt(ModelParseState::new(), ModelOutputAttempt::Content(raw)) {
        ModelParseOutcome::Accepted(plan) => plan,
        other => panic!("expected accepted model output, got {other:?}"),
    }
}

fn first_failure(raw: &str) -> ModelParseErrorCode {
    match parse_model_attempt(ModelParseState::new(), ModelOutputAttempt::Content(raw)) {
        ModelParseOutcome::RetryAllowed { failure, .. } => failure.code,
        other => panic!("expected parse-only retry, got {other:?}"),
    }
}

#[test]
fn accepts_direct_or_one_string_aware_balanced_object() {
    let raw = serde_json::to_string(&valid_value()).expect("serialize fixture");
    assert_eq!(accepted(&raw).tasks.len(), 1);

    let surrounded = format!(
        "Proposal follows; braces inside JSON strings stay inert:\n```json\n{}\n```",
        raw.replace("Build the change", "Build a literal { brace } safely")
    );
    let parsed = accepted(&surrounded);
    assert_eq!(parsed.tasks[0].summary, "Build a literal { brace } safely");
}

#[test]
fn rejects_arrays_scalars_second_objects_and_invalid_json() {
    assert_eq!(first_failure("[]"), ModelParseErrorCode::ModelNotObject);
    assert_eq!(first_failure("42"), ModelParseErrorCode::ModelNotObject);
    assert_eq!(
        first_failure("{\"schemaVersion\":1} {\"schemaVersion\":1}"),
        ModelParseErrorCode::ModelMultipleObjects
    );
    assert_eq!(
        first_failure("not JSON at all"),
        ModelParseErrorCode::ModelInvalidJson
    );
}

#[test]
fn probes_version_before_strict_decode_and_rejects_unknown_fields() {
    assert_eq!(
        first_failure("{\"tasks\":[],\"edges\":[]}"),
        ModelParseErrorCode::ModelSchemaVersionMissing
    );
    assert_eq!(
        first_failure("{\"schemaVersion\":\"1\",\"tasks\":[],\"edges\":[]}"),
        ModelParseErrorCode::ModelSchemaVersionInvalid
    );
    assert_eq!(
        first_failure("{\"schemaVersion\":2,\"tasks\":[],\"edges\":[]}"),
        ModelParseErrorCode::ModelSchemaVersionUnsupported
    );
    let mut unknown = valid_value();
    unknown
        .as_object_mut()
        .expect("object")
        .insert("permissionGrant".to_string(), json!(true));
    assert_eq!(
        first_failure(&serde_json::to_string(&unknown).expect("serialize")),
        ModelParseErrorCode::ModelUnknownField
    );
    assert_eq!(
        first_failure(r#"{"schemaVersion":1,"schemaVersion":1,"tasks":[],"edges":[]}"#),
        ModelParseErrorCode::ModelInvalidJson
    );
}

#[test]
fn hostile_model_fields_cannot_grant_authority_or_mutation() {
    for (field, value) in [
        ("organizationId", json!("org-other")),
        ("tools", json!({"allow": ["shell"]})),
        ("schedules", json!(["nightly"])),
        ("agentEdits", json!([{"id": "builder"}])),
        ("mcpMutation", json!(true)),
    ] {
        let mut hostile = valid_value();
        hostile
            .as_object_mut()
            .expect("object")
            .insert(field.to_string(), value);
        assert_eq!(
            first_failure(&serde_json::to_string(&hostile).expect("serialize")),
            ModelParseErrorCode::ModelUnknownField,
            "{field}"
        );
    }
}

#[test]
fn enforces_raw_container_scalar_depth_and_plan_bounds() {
    let oversized = "x".repeat(1024 * 1024 + 1);
    assert_eq!(
        first_failure(&oversized),
        ModelParseErrorCode::ModelOutputTooLarge
    );

    let mut scalar = valid_value();
    scalar["tasks"][0]["summary"] = json!("x".repeat(32 * 1024 + 1));
    assert_eq!(
        first_failure(&serde_json::to_string(&scalar).expect("serialize")),
        ModelParseErrorCode::ModelBoundsExceeded
    );

    let mut too_many_tasks = valid_value();
    let task = too_many_tasks["tasks"][0].clone();
    too_many_tasks["tasks"] = Value::Array(vec![task; 65]);
    assert_eq!(
        first_failure(&serde_json::to_string(&too_many_tasks).expect("serialize")),
        ModelParseErrorCode::ModelBoundsExceeded
    );

    let mut nested = json!(null);
    for _ in 0..34 {
        nested = json!({"nested": nested});
    }
    let raw = serde_json::to_string(&json!({
        "schemaVersion": 1,
        "tasks": [],
        "edges": [],
        "unknown": nested
    }))
    .expect("serialize");
    assert_eq!(
        first_failure(&raw),
        ModelParseErrorCode::ModelBoundsExceeded
    );
}

#[test]
fn allows_exactly_one_parse_retry_but_no_transport_or_timeout_retry() {
    let first = parse_model_attempt(
        ModelParseState::new(),
        ModelOutputAttempt::Content("invalid"),
    );
    let ModelParseOutcome::RetryAllowed { state, .. } = first else {
        panic!("first parse failure should allow retry")
    };
    assert_eq!(state.parse_failures(), 1);

    let valid = serde_json::to_string(&valid_value()).expect("serialize");
    assert!(matches!(
        parse_model_attempt(state, ModelOutputAttempt::Content(&valid)),
        ModelParseOutcome::Accepted(_)
    ));
    let exhausted = parse_model_attempt(state, ModelOutputAttempt::Content("still invalid"));
    assert!(matches!(
        exhausted,
        ModelParseOutcome::Rejected(ref error)
            if error.code == ModelParseErrorCode::ModelParseBudgetExhausted
    ));

    assert!(matches!(
        parse_model_attempt(ModelParseState::new(), ModelOutputAttempt::Timeout),
        ModelParseOutcome::Rejected(ref error) if error.code == ModelParseErrorCode::ModelTimeout
    ));
    assert!(matches!(
        parse_model_attempt(
            ModelParseState::new(),
            ModelOutputAttempt::TransportFailure
        ),
        ModelParseOutcome::Rejected(ref error)
            if error.code == ModelParseErrorCode::ModelTransportFailure
    ));
}
