use pandamux_core::{AgentDefinition, MemoryEntry, Run, ScheduleRecord};
use serde::de::DeserializeOwned;
use std::fmt;

/// Unversioned Phase 1 record families that must cross an explicit adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyRecordKind {
    Run,
    AgentDefinition,
    MemoryEntry,
    ScheduleRecord,
}

impl LegacyRecordKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::AgentDefinition => "agent definition",
            Self::MemoryEntry => "memory entry",
            Self::ScheduleRecord => "schedule record",
        }
    }

    const fn rejection(self) -> (&'static str, &'static str) {
        match self {
            Self::Run => (
                "legacy_run_requires_versioned_adapter",
                "Preserve the original JSON and migrate it through the versioned Phase 5 run adapter before dispatch.",
            ),
            Self::AgentDefinition => (
                "legacy_agent_requires_versioned_adapter",
                "Preserve the original definition and import it through the versioned agent compiler before activation.",
            ),
            Self::MemoryEntry => (
                "legacy_memory_requires_markdown_migration",
                "Preserve the original entry, discard its derived embedding, and migrate canonical text through the Phase 5 markdown-memory importer.",
            ),
            Self::ScheduleRecord => (
                "legacy_schedule_deferred_to_phase6",
                "Preserve the raw schedule for Phase 6 import; it must land disabled and Approve-first and may require target repair.",
            ),
        }
    }
}

/// Stable, presentation-safe reason an unversioned record was not accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyCompatibilityError {
    pub code: &'static str,
    pub kind: LegacyRecordKind,
    pub message: String,
    pub next_action: &'static str,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl fmt::Display for LegacyCompatibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)?;
        if let (Some(line), Some(column)) = (self.line, self.column) {
            write!(formatter, " (line {line}, column {column})")?;
        }
        write!(formatter, ". Next action: {}", self.next_action)
    }
}

impl std::error::Error for LegacyCompatibilityError {}

/// Validate the historical shape, then reject it until its explicit adapter exists.
///
/// A successful return is intentionally impossible in the dependency-gate
/// checkpoint. This function prevents callers from treating an unversioned
/// Phase 1 record as canonical Phase 5 state merely because Serde can decode it.
pub fn require_explicit_legacy_migration(
    kind: LegacyRecordKind,
    raw_json: &str,
) -> Result<(), LegacyCompatibilityError> {
    let parsed = match kind {
        LegacyRecordKind::Run => parse_legacy::<Run>(raw_json),
        LegacyRecordKind::AgentDefinition => parse_legacy::<AgentDefinition>(raw_json),
        LegacyRecordKind::MemoryEntry => parse_legacy::<MemoryEntry>(raw_json),
        LegacyRecordKind::ScheduleRecord => parse_legacy::<ScheduleRecord>(raw_json),
    };

    if let Err(error) = parsed {
        return Err(LegacyCompatibilityError {
            code: "legacy_record_malformed",
            kind,
            message: format!(
                "The unversioned {} does not match the retained Phase 1 shape",
                kind.label()
            ),
            next_action: "Keep the original payload and repair or export the required Phase 1 fields before retrying compatibility import",
            line: Some(error.line()),
            column: Some(error.column()),
        });
    }

    let (code, next_action) = kind.rejection();
    Err(LegacyCompatibilityError {
        code,
        kind,
        message: format!(
            "The unversioned {} is recognized compatibility input, not accepted Phase 5 state",
            kind.label()
        ),
        next_action,
        line: None,
        column: None,
    })
}

fn parse_legacy<T: DeserializeOwned>(raw_json: &str) -> Result<(), serde_json::Error> {
    serde_json::from_str::<T>(raw_json).map(|_| ())
}
