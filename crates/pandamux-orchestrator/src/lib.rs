#![forbid(unsafe_code)]

//! Pure orchestration domain logic.
//!
//! This crate owns the future Phase 5 state machine. It deliberately has no
//! runtime, filesystem, network, storage, UI, provider-process, keyring, or
//! embedding dependency. The server will interpret serializable effects.

pub mod bundle;
pub mod compatibility;
pub mod diagnostics;
pub mod policy;
pub mod schema;

pub use bundle::{
    CompileBundleInput, LayeredSourceDocument, canonical_bundle_bytes, compile_instruction_bundle,
    source_layer_sha256, verify_compiled_bundle,
};
pub use compatibility::{
    LegacyCompatibilityError, LegacyRecordKind, require_explicit_legacy_migration,
};
pub use diagnostics::{
    ConfigurationIssue, ConfigurationIssueCode, ConfigurationStage, DiagnosticSet,
    InvalidRelativeConfigPath, JsonPointer, RelativeConfigPath, SafeText,
};
pub use policy::{LayerGrant, TrustedBundleSettings};
pub use schema::{
    ParsedDocumentV1, SourceDocument, generated_authored_schema_json, parse_source_document,
    validate_source_set_bounds,
};
