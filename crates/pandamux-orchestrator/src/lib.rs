#![forbid(unsafe_code)]

//! Pure orchestration domain logic.
//!
//! This crate owns the future Phase 5 state machine. It deliberately has no
//! runtime, filesystem, network, storage, UI, provider-process, keyring, or
//! embedding dependency. The server will interpret serializable effects.

pub mod bundle;
pub mod compatibility;
pub mod configuration;
pub mod dag;
pub mod diagnostics;
pub mod model_output;
pub mod policy;
pub mod routing;
pub mod schema;
pub mod state;

pub use bundle::{
    CompileBundleInput, LayeredSourceDocument, canonical_bundle_bytes, compile_instruction_bundle,
    source_layer_sha256, verify_compiled_bundle,
};
pub use compatibility::{
    LegacyCompatibilityError, LegacyRecordKind, require_explicit_legacy_migration,
};
pub use configuration::{
    BehavioralChangeSet, ConfigurationBehavioralDiff, ConfigurationCandidateInspection,
    ConfigurationCandidateInspectionInput, ConfigurationDependencySnapshot,
    ConfigurationFileChange, ConfigurationFileChangeKind, InspectedConfigurationIssue,
    inspect_configuration_candidate,
};
pub use dag::{
    AgentFeasibilitySnapshot, PlanValidationContext, ProjectPolicySnapshot,
    ProposedDependencyEdgeV1, ProposedPlanV1, ProposedTaskInputV1, ProposedTaskV1, is_task_ready,
    validate_plan,
};
pub use diagnostics::{
    ConfigurationIssue, ConfigurationIssueCode, ConfigurationStage, DiagnosticSet,
    InvalidRelativeConfigPath, JsonPointer, RelativeConfigPath, SafeText,
};
pub use model_output::{
    ModelOutputAttempt, ModelParseError, ModelParseErrorCode, ModelParseOutcome, ModelParseState,
    parse_model_attempt,
};
pub use policy::{
    ApprovalAction, ApprovalDecisionIdentity, ApprovalReconciliation, ApprovalSafetyClass,
    LayerGrant, TrustedBundleSettings, approval_identity, canonical_action_digest,
    canonical_json_bytes, canonical_json_sha256, canonicalize_json, reconcile_approval,
};
pub use routing::{
    FallbackEligibility, ProviderCandidateSnapshot, RoutingContext, RoutingFailure,
    ValidatedAgentTask, fallback_eligibility, route_task,
};
pub use schema::{
    ParsedDocumentV1, SourceDocument, generated_authored_schema_json, parse_source_document,
    validate_source_set_bounds,
};
pub use state::{
    ApplyOutcome, OrchestrationError, OrchestrationErrorCode, apply, canonical_blackboard_sha256,
    canonical_run_bytes, decide, replay, replay_from,
};
