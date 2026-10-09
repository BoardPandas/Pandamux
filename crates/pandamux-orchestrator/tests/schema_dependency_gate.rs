use schemars::{JsonSchema, schema_for};
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DependencyGateDocument {
    schema_version: u32,
    name: String,
}

#[test]
fn selected_schema_and_yaml_dependencies_compile_and_parse_on_the_pure_boundary() {
    let parsed: DependencyGateDocument =
        serde_saphyr::from_str("schemaVersion: 1\nname: dependency-gate\n")
            .expect("the pinned YAML parser should decode a typed document");
    assert_eq!(
        parsed,
        DependencyGateDocument {
            schema_version: 1,
            name: "dependency-gate".to_string(),
        }
    );

    let schema = schema_for!(DependencyGateDocument);
    let serialized = serde_json::to_value(schema).expect("JSON Schema should serialize");
    assert_eq!(serialized["type"], "object");
}
