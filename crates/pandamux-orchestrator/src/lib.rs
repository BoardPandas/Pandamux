#![forbid(unsafe_code)]

//! Pure orchestration domain logic.
//!
//! This crate owns the future Phase 5 state machine. It deliberately has no
//! runtime, filesystem, network, storage, UI, provider-process, keyring, or
//! embedding dependency. The server will interpret serializable effects.

pub mod compatibility;
pub mod diagnostics;
pub mod schema;

pub use compatibility::{
    LegacyCompatibilityError, LegacyRecordKind, require_explicit_legacy_migration,
};
pub use diagnostics::{
    ConfigurationIssue, ConfigurationIssueCode, ConfigurationStage, DiagnosticSet,
    InvalidRelativeConfigPath, JsonPointer, RelativeConfigPath, SafeText,
};
pub use schema::{
    ParsedDocumentV1, SourceDocument, generated_authored_schema_json, parse_source_document,
    validate_source_set_bounds,
};
