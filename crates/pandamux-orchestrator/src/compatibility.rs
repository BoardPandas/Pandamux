use serde::de::DeserializeOwned;
use std::fmt;

/// Unversioned Phase 1 record families that must cross an explicit adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyRecordKind {
    Run,
    RunEvent,
    AgentDefinition,
    MemoryEntry,
    ScheduleRecord,
}

impl LegacyRecordKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::RunEvent => "run event",
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
            Self::RunEvent => (
                "legacy_run_event_requires_versioned_adapter",
                "Preserve the original JSON and migrate it together with its legacy run through the versioned Phase 5 run adapter before replay.",
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
        LegacyRecordKind::Run => parse_legacy::<v0::Run>(raw_json),
        LegacyRecordKind::RunEvent => parse_legacy::<v0::RunEvent>(raw_json),
        LegacyRecordKind::AgentDefinition => parse_legacy::<v0::AgentDefinition>(raw_json),
        LegacyRecordKind::MemoryEntry => parse_legacy::<v0::MemoryEntry>(raw_json),
        LegacyRecordKind::ScheduleRecord => parse_legacy::<v0::ScheduleRecord>(raw_json),
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

/// Exact private copies of the unversioned Phase 1 wire records.
///
/// These types intentionally do not use `pandamux-core`. They exist only to
/// recognize historical input before returning a fail-closed compatibility
/// error. `deny_unknown_fields` prevents a later canonical field from making a
/// legacy payload look executable after the live core model changes.
mod v0 {
    #![allow(dead_code)]

    use serde::Deserialize;
    use serde_json::Value;
    use std::collections::{BTreeSet, HashMap};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct Run {
        id: String,
        kind: RunKind,
        #[serde(default)]
        status: RunStatus,
        trigger: String,
        started_at_ms: u64,
        #[serde(default)]
        ended_at_ms: Option<u64>,
        #[serde(default)]
        tasks: Vec<RunTask>,
        #[serde(default)]
        blackboard: HashMap<String, Value>,
        #[serde(default)]
        usage: Option<TurnUsage>,
        #[serde(default)]
        outcome: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum RunKind {
        Orchestrator,
        Schedule { schedule_id: String },
    }

    #[derive(Default, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum RunStatus {
        #[default]
        Planned,
        Running,
        Paused,
        Succeeded,
        Failed,
        Cancelled,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct RunTask {
        task_id: String,
        #[serde(default)]
        target_project_id: Option<String>,
        #[serde(default)]
        agent_id: Option<String>,
        #[serde(default)]
        thread_id: Option<String>,
        #[serde(default)]
        status: RunTaskStatus,
        #[serde(default)]
        depends_on: Vec<String>,
        #[serde(default)]
        summary: Option<String>,
    }

    #[derive(Default, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum RunTaskStatus {
        #[default]
        Pending,
        Dispatched,
        Running,
        Finished,
        Failed,
        Skipped,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    pub(super) enum RunEvent {
        Planned {
            run_id: String,
            tasks: Vec<RunTask>,
        },
        ApprovalRequested {
            run_id: String,
            request_id: String,
            reason: String,
        },
        TaskDispatched {
            run_id: String,
            task_id: String,
            #[serde(default)]
            target_project_id: Option<String>,
            #[serde(default)]
            agent_id: Option<String>,
            thread_id: String,
        },
        TaskRerouted {
            run_id: String,
            task_id: String,
            new_route: String,
        },
        TaskFinished {
            run_id: String,
            task_id: String,
            outcome: String,
        },
        Paused {
            run_id: String,
            reason: String,
        },
        Resumed {
            run_id: String,
        },
        Finished {
            run_id: String,
            status: RunStatus,
        },
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct AgentDefinition {
        id: String,
        name: String,
        scope: AgentScope,
        author: AgentAuthor,
        #[serde(default)]
        version: String,
        description: String,
        system_prompt: String,
        #[serde(default)]
        allowed_tools: Vec<String>,
        #[serde(default)]
        disallowed_tools: Vec<String>,
        #[serde(default)]
        max_turns: Option<u32>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum AgentScope {
        Team,
        Project,
        Personal,
        BuiltIn,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum AgentAuthor {
        User,
        Orchestrator { run_id: String },
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct MemoryEntry {
        id: String,
        agent_id: String,
        scope: MemoryScope,
        text: String,
        #[serde(default)]
        embedding: Option<Vec<f32>>,
        created_at_ms: u64,
        updated_at_ms: u64,
        #[serde(default)]
        source_run_id: Option<String>,
        #[serde(default)]
        tombstone: bool,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum MemoryScope {
        Project { project_id: String },
        AgentWide,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct ScheduleRecord {
        id: String,
        name: String,
        #[serde(default)]
        project_id: Option<String>,
        environment: ScheduleEnvironmentRef,
        trigger: ScheduleTrigger,
        target: ScheduleTarget,
        #[serde(default)]
        approval: ScheduleApprovalPolicy,
        #[serde(default)]
        overlap: ScheduleOverlapPolicy,
        #[serde(default)]
        catch_up: ScheduleCatchUpPolicy,
        #[serde(default)]
        budget: ScheduleBudget,
        #[serde(default)]
        notify: Vec<String>,
        #[serde(default)]
        full_access_opt_in: bool,
        #[serde(default = "default_true")]
        enabled: bool,
    }

    fn default_true() -> bool {
        true
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct ScheduleEnvironmentRef {
        id: String,
        display_name: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct ScheduleTrigger {
        cron: String,
        timezone: String,
    }

    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum ScheduleTarget {
        Orchestrator {
            prompt: String,
        },
        Agent {
            agent: String,
            project_path: String,
            prompt: String,
        },
        Session {
            project_path: String,
            prompt: String,
            #[serde(default)]
            provider: Option<String>,
            #[serde(default)]
            model: Option<String>,
            #[serde(default)]
            thread_mode: ScheduleThreadMode,
        },
    }

    #[derive(Default, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum ScheduleThreadMode {
        #[default]
        NewThreadPerRun,
        ContinueExistingThread,
    }

    #[derive(Default, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum ScheduleApprovalPolicy {
        #[default]
        Auto,
        ApproveFirst,
    }

    #[derive(Default, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum ScheduleOverlapPolicy {
        #[default]
        Skip,
        Queue,
        Parallel {
            max: u32,
        },
    }

    #[derive(Default, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum ScheduleCatchUpPolicy {
        #[default]
        Skip,
        RunOnce,
    }

    #[derive(Default, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct ScheduleBudget {
        #[serde(default)]
        per_run: Option<PerRunBudget>,
        #[serde(default)]
        per_day: Option<PerDayBudget>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct PerRunBudget {
        #[serde(default)]
        max_tokens: Option<u64>,
        #[serde(default)]
        max_runtime_minutes: Option<u32>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct PerDayBudget {
        #[serde(default)]
        max_cost_usd: Option<f64>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct TurnUsage {
        #[serde(default)]
        input_tokens: u64,
        #[serde(default)]
        output_tokens: u64,
        #[serde(default)]
        cache_read_tokens: Option<u64>,
        #[serde(default)]
        cache_write_tokens: Option<u64>,
        #[serde(default)]
        cost_usd: Option<f64>,
        #[serde(default)]
        unknown_fields: BTreeSet<UsageUnknownField>,
        #[serde(default)]
        provider_reported_micro_usd: Option<u64>,
        #[serde(default)]
        provenance: Option<TurnUsageProvenance>,
        #[serde(default)]
        components: Vec<TurnUsageComponent>,
    }

    #[derive(Deserialize, PartialEq, Eq, PartialOrd, Ord)]
    #[serde(rename_all = "snake_case")]
    enum UsageUnknownField {
        OccurredAt,
        Model,
        Identity,
        InputTokens,
        OutputTokens,
        CacheReadTokens,
        CacheWriteTokens,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct TurnUsageComponent {
        provider_native_message_id: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        input_tokens: u64,
        #[serde(default)]
        output_tokens: u64,
        #[serde(default)]
        cache_read_tokens: Option<u64>,
        #[serde(default)]
        cache_write_tokens: Option<u64>,
        #[serde(default)]
        occurred_at_ms: Option<u64>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct TurnUsageProvenance {
        #[serde(default)]
        provider_native_session_id: Option<String>,
        #[serde(default)]
        provider_native_turn_id: Option<String>,
        #[serde(default)]
        occurred_at_ms: Option<u64>,
        #[serde(default)]
        model: Option<String>,
    }
}
