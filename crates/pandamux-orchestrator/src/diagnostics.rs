use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_DIAGNOSTICS: usize = 128;
pub const MAX_SAFE_TEXT_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidRelativeConfigPath;

impl fmt::Display for InvalidRelativeConfigPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid confined relative configuration path")
    }
}

impl std::error::Error for InvalidRelativeConfigPath {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RelativeConfigPath(String);

impl RelativeConfigPath {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidRelativeConfigPath> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 240
            || value.starts_with('/')
            || value.starts_with('\\')
            || value.contains('\\')
            || value.contains('\0')
            || has_drive_prefix(&value)
            || value
                .split('/')
                .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
        {
            return Err(InvalidRelativeConfigPath);
        }
        Ok(Self(value))
    }

    pub fn invalid() -> Self {
        Self("<invalid>".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn has_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

impl fmt::Display for RelativeConfigPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct JsonPointer(String);

impl JsonPointer {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        (value.is_empty() || (value.starts_with('/') && value.len() <= MAX_SAFE_TEXT_BYTES))
            .then_some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SafeText(String);

impl SafeText {
    pub fn new(value: impl AsRef<str>) -> Self {
        let sanitized = value
            .as_ref()
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect::<String>();
        Self(truncate_utf8(&sanitized, MAX_SAFE_TEXT_BYTES))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationStage {
    Bounds,
    Text,
    Frontmatter,
    Yaml,
    Schema,
    Identity,
    Reference,
    Policy,
    Compilation,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationIssueCode {
    ConfigFileCountLimit,
    ConfigBundleSizeLimit,
    ConfigFileTooLarge,
    DiagnosticLimitReached,
    ConfigNonUtf8,
    ConfigBomUnsupported,
    ConfigNulUnsupported,
    FrontmatterMissing,
    FrontmatterUnterminated,
    FrontmatterEmptyBody,
    YamlMultipleDocuments,
    YamlDuplicateKey,
    YamlUnsupportedTag,
    YamlDepthLimit,
    SchemaVersionMissing,
    SchemaVersionInvalid,
    SchemaVersionUnsupported,
    SchemaKindMissing,
    SchemaKindMismatch,
    SchemaUnknownField,
    SchemaInvalidValue,
    DocumentIdInvalid,
    DocumentIdMismatch,
    DocumentIdDuplicate,
    ReferencePathEscape,
    ReferenceNotFound,
    ReferenceKindMismatch,
    ReferenceDuplicateId,
    LayerNotTrusted,
    LayerScopeViolation,
    PolicyAccessWidening,
    PolicyFullAccessFromRepo,
    PolicyCrossOrganization,
    TierUnmapped,
    BundleHashMismatch,
    MinimumVersionUnsupported,
}

impl ConfigurationIssueCode {
    pub const ALL: &'static [Self] = &[
        Self::ConfigFileCountLimit,
        Self::ConfigBundleSizeLimit,
        Self::ConfigFileTooLarge,
        Self::DiagnosticLimitReached,
        Self::ConfigNonUtf8,
        Self::ConfigBomUnsupported,
        Self::ConfigNulUnsupported,
        Self::FrontmatterMissing,
        Self::FrontmatterUnterminated,
        Self::FrontmatterEmptyBody,
        Self::YamlMultipleDocuments,
        Self::YamlDuplicateKey,
        Self::YamlUnsupportedTag,
        Self::YamlDepthLimit,
        Self::SchemaVersionMissing,
        Self::SchemaVersionInvalid,
        Self::SchemaVersionUnsupported,
        Self::SchemaKindMissing,
        Self::SchemaKindMismatch,
        Self::SchemaUnknownField,
        Self::SchemaInvalidValue,
        Self::DocumentIdInvalid,
        Self::DocumentIdMismatch,
        Self::DocumentIdDuplicate,
        Self::ReferencePathEscape,
        Self::ReferenceNotFound,
        Self::ReferenceKindMismatch,
        Self::ReferenceDuplicateId,
        Self::LayerNotTrusted,
        Self::LayerScopeViolation,
        Self::PolicyAccessWidening,
        Self::PolicyFullAccessFromRepo,
        Self::PolicyCrossOrganization,
        Self::TierUnmapped,
        Self::BundleHashMismatch,
        Self::MinimumVersionUnsupported,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfigFileCountLimit => "config_file_count_limit",
            Self::ConfigBundleSizeLimit => "config_bundle_size_limit",
            Self::ConfigFileTooLarge => "config_file_too_large",
            Self::DiagnosticLimitReached => "diagnostic_limit_reached",
            Self::ConfigNonUtf8 => "config_non_utf8",
            Self::ConfigBomUnsupported => "config_bom_unsupported",
            Self::ConfigNulUnsupported => "config_nul_unsupported",
            Self::FrontmatterMissing => "frontmatter_missing",
            Self::FrontmatterUnterminated => "frontmatter_unterminated",
            Self::FrontmatterEmptyBody => "frontmatter_empty_body",
            Self::YamlMultipleDocuments => "yaml_multiple_documents",
            Self::YamlDuplicateKey => "yaml_duplicate_key",
            Self::YamlUnsupportedTag => "yaml_unsupported_tag",
            Self::YamlDepthLimit => "yaml_depth_limit",
            Self::SchemaVersionMissing => "schema_version_missing",
            Self::SchemaVersionInvalid => "schema_version_invalid",
            Self::SchemaVersionUnsupported => "schema_version_unsupported",
            Self::SchemaKindMissing => "schema_kind_missing",
            Self::SchemaKindMismatch => "schema_kind_mismatch",
            Self::SchemaUnknownField => "schema_unknown_field",
            Self::SchemaInvalidValue => "schema_invalid_value",
            Self::DocumentIdInvalid => "document_id_invalid",
            Self::DocumentIdMismatch => "document_id_mismatch",
            Self::DocumentIdDuplicate => "document_id_duplicate",
            Self::ReferencePathEscape => "reference_path_escape",
            Self::ReferenceNotFound => "reference_not_found",
            Self::ReferenceKindMismatch => "reference_kind_mismatch",
            Self::ReferenceDuplicateId => "reference_duplicate_id",
            Self::LayerNotTrusted => "layer_not_trusted",
            Self::LayerScopeViolation => "layer_scope_violation",
            Self::PolicyAccessWidening => "policy_access_widening",
            Self::PolicyFullAccessFromRepo => "policy_full_access_from_repo",
            Self::PolicyCrossOrganization => "policy_cross_organization",
            Self::TierUnmapped => "tier_unmapped",
            Self::BundleHashMismatch => "bundle_hash_mismatch",
            Self::MinimumVersionUnsupported => "minimum_version_unsupported",
        }
    }
}

impl fmt::Display for ConfigurationIssueCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationIssue {
    pub code: ConfigurationIssueCode,
    pub file: RelativeConfigPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<JsonPointer>,
    pub stage: ConfigurationStage,
    pub explanation: SafeText,
    pub corrective_action: SafeText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticSet {
    issues: Vec<ConfigurationIssue>,
}

impl DiagnosticSet {
    pub fn from_issues(mut issues: Vec<ConfigurationIssue>) -> Self {
        let overflow = issues.len() > MAX_DIAGNOSTICS;
        if overflow {
            issues.truncate(MAX_DIAGNOSTICS - 1);
        }
        issues.sort_by(|left, right| {
            left.file
                .cmp(&right.file)
                .then_with(|| left.field.cmp(&right.field))
                .then_with(|| left.code.cmp(&right.code))
                .then_with(|| left.line.cmp(&right.line))
                .then_with(|| left.column.cmp(&right.column))
        });
        if overflow {
            issues.push(ConfigurationIssue {
                code: ConfigurationIssueCode::DiagnosticLimitReached,
                file: RelativeConfigPath::invalid(),
                field: None,
                stage: ConfigurationStage::Bounds,
                explanation: SafeText::new("Additional configuration issues were omitted"),
                corrective_action: SafeText::new(
                    "Resolve the reported issues and validate the source set again",
                ),
                line: None,
                column: None,
            });
        }
        Self { issues }
    }

    pub fn one(issue: ConfigurationIssue) -> Self {
        Self::from_issues(vec![issue])
    }

    pub fn issues(&self) -> &[ConfigurationIssue] {
        &self.issues
    }

    pub fn into_issues(self) -> Vec<ConfigurationIssue> {
        self.issues
    }
}

impl fmt::Display for DiagnosticSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} configuration issue(s)", self.issues.len())
    }
}

impl std::error::Error for DiagnosticSet {}

pub(crate) fn issue(
    code: ConfigurationIssueCode,
    file: RelativeConfigPath,
    field: Option<&str>,
    stage: ConfigurationStage,
    explanation: &str,
    corrective_action: &str,
    location: Option<(u32, u32)>,
) -> ConfigurationIssue {
    ConfigurationIssue {
        code,
        file,
        field: field.and_then(JsonPointer::new),
        stage,
        explanation: SafeText::new(explanation),
        corrective_action: SafeText::new(corrective_action),
        line: location.map(|value| value.0),
        column: location.map(|value| value.1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_paths_reject_platform_and_escape_forms() {
        for invalid in [
            "",
            "/absolute",
            "C:/drive",
            "agents\\bad.md",
            "agents/../bad.md",
            "./agent.md",
            "agents//bad.md",
        ] {
            assert!(RelativeConfigPath::new(invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            RelativeConfigPath::new("agents/reviewer.md")
                .expect("valid path")
                .as_str(),
            "agents/reviewer.md"
        );
    }

    #[test]
    fn diagnostics_are_bounded_and_safe_text_is_utf8_aware() {
        let long = "🙂".repeat(200);
        assert!(SafeText::new(long).as_str().len() <= MAX_SAFE_TEXT_BYTES);
        let base = issue(
            ConfigurationIssueCode::SchemaInvalidValue,
            RelativeConfigPath::new("project.yaml").expect("path"),
            None,
            ConfigurationStage::Schema,
            "invalid",
            "repair",
            None,
        );
        let set = DiagnosticSet::from_issues(vec![base; MAX_DIAGNOSTICS + 20]);
        assert_eq!(set.issues().len(), MAX_DIAGNOSTICS);
        assert_eq!(
            set.issues().last().expect("limit issue").code,
            ConfigurationIssueCode::DiagnosticLimitReached
        );
    }
}
