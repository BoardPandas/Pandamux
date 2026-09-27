use crate::ids::{ThreadId, TurnId};
use crate::thread::{TurnInput, TurnUsage};
use serde::{Deserialize, Serialize};

/// Monotonic, append-only event in a thread stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadEvent {
    pub thread_id: ThreadId,
    pub seq: u64,
    pub at_ms: u64,
    pub kind: ThreadEventKind,
}

/// The payload variety of a thread event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThreadEventKind {
    TurnRequested {
        turn_id: TurnId,
        input: TurnInput,
    },
    TurnStarted {
        turn_id: TurnId,
    },
    AssistantText {
        item_id: String,
        text: String,
    },
    Reasoning {
        item_id: String,
        text: String,
    },
    ToolCall {
        item_id: String,
        name: String,
        input: serde_json::Value,
        status: ToolCallStatus,
        #[serde(default)]
        output: Option<String>,
    },
    CommandRun {
        item_id: String,
        command: String,
        cwd: String,
        #[serde(default)]
        exit_code: Option<i32>,
        #[serde(default)]
        output_tail: Option<String>,
    },
    FileChange {
        item_id: String,
        path: String,
        kind: FileChangeKind,
    },
    ApprovalRequested {
        request_id: String,
        kind: ApprovalKind,
        detail: serde_json::Value,
    },
    ApprovalResolved {
        request_id: String,
        decision: ApprovalDecision,
        by: String,
    },
    PlanUpdated {
        steps: Vec<PlanStep>,
    },
    TokenUsage {
        usage: TurnUsage,
    },
    RateLimitObserved {
        provider: String,
        details: serde_json::Value,
    },
    BudgetExceeded {
        limit: String,
    },
    Notice {
        level: String,
        message: String,
    },
    Error {
        class: String,
        message: String,
        recoverable: bool,
    },
    TurnCompleted {
        outcome: TurnOutcome,
    },
    TurnSettled {
        #[serde(default)]
        changed_files: Vec<String>,
    },
    SubAgentSpawned {
        sub_agent_id: String,
        #[serde(default)]
        parent_sub_agent_id: Option<String>,
        #[serde(default)]
        parent_item_id: Option<String>,
        title: String,
        agent_type: String,
        model: String,
        #[serde(default)]
        effort: Option<String>,
    },
    SubAgentActivity {
        sub_agent_id: String,
        preview: String,
    },
    SubAgentUsage {
        sub_agent_id: String,
        tokens: u64,
        tool_calls: u32,
    },
    SubAgentFinished {
        sub_agent_id: String,
        outcome: String,
        elapsed_ms: u64,
        #[serde(default)]
        summary: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    #[default]
    Pending,
    Executing,
    Success,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    CommandExecution,
    FileEdit,
    NetworkAccess,
    ToolCall,
    FullAccessEscalation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Denied,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    pub status: PlanStepStatus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed,
    Skipped,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    #[default]
    Success,
    Cancelled,
    Failed,
    Interrupted,
}
