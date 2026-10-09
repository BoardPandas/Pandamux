#![forbid(unsafe_code)]

//! Pure orchestration domain logic.
//!
//! This crate owns the future Phase 5 state machine. It deliberately has no
//! runtime, filesystem, network, storage, UI, provider-process, keyring, or
//! embedding dependency. The server will interpret serializable effects.

pub mod compatibility;

pub use compatibility::{
    LegacyCompatibilityError, LegacyRecordKind, require_explicit_legacy_migration,
};
