use pandamux_core::{
    ApprovalDecision, ApprovalKind, FileChangeKind, PlanStep, Run, RunEvent, RunStatus,
    RunTaskStatus, Thread, ThreadEvent, ThreadEventKind, ThreadStatus, ToolCallStatus, Turn,
    TurnOutcome, TurnStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Client-side projection of a thread accumulated from an event stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadProjection {
    pub thread: Thread,
    #[serde(default)]
    pub turns: Vec<Turn>,
    #[serde(default)]
    pub items: Vec<TimelineItem>,
    #[serde(default)]
    pub sub_agents: HashMap<String, SubAgentProjection>,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub last_seq: u64,
}

impl ThreadProjection {
    pub fn new(thread: Thread) -> Self {
        Self {
            thread,
            turns: Vec::new(),
            items: Vec::new(),
            sub_agents: HashMap::new(),
            changed_files: Vec::new(),
            last_seq: 0,
        }
    }

    /// Reducer applying a single ThreadEvent into the projection.
    pub fn apply_event(&mut self, event: &ThreadEvent) {
        if event.seq > self.last_seq {
            self.last_seq = event.seq;
        }

        match &event.kind {
            ThreadEventKind::TurnRequested { turn_id, input } => {
                self.thread.status = ThreadStatus::Working;
                let turn = Turn {
                    id: turn_id.clone(),
                    thread_id: self.thread.id.clone(),
                    seq: event.seq,
                    input: input.clone(),
                    status: TurnStatus::Requested,
                    started_at_ms: event.at_ms,
                    ended_at_ms: None,
                    usage: None,
                    checkpoint_before: None,
                    checkpoint_after: None,
                };
                self.turns.push(turn);
                self.items.push(TimelineItem::TurnUserPrompt {
                    turn_id: turn_id.as_str().to_string(),
                    text: input.text.clone(),
                });
            }
            ThreadEventKind::TurnStarted { turn_id } => {
                if let Some(turn) = self.turns.iter_mut().find(|t| &t.id == turn_id) {
                    turn.status = TurnStatus::Running;
                }
                self.thread.status = ThreadStatus::Working;
            }
            ThreadEventKind::AssistantText { item_id, text } => {
                if let Some(TimelineItem::AssistantText { id, text: prev }) = self.items.last_mut()
                    && id == item_id
                {
                    prev.push_str(text);
                    return;
                }
                self.items.push(TimelineItem::AssistantText {
                    id: item_id.clone(),
                    text: text.clone(),
                });
            }
            ThreadEventKind::Reasoning { item_id, text } => {
                if let Some(TimelineItem::Reasoning { id, text: prev }) = self.items.last_mut()
                    && id == item_id
                {
                    prev.push_str(text);
                    return;
                }
                self.items.push(TimelineItem::Reasoning {
                    id: item_id.clone(),
                    text: text.clone(),
                });
            }
            ThreadEventKind::ToolCall {
                item_id,
                name,
                input,
                status,
                output,
            } => {
                if let Some(existing) = self.items.iter_mut().find_map(|item| match item {
                    TimelineItem::ToolCall {
                        id,
                        status: s,
                        output: o,
                        ..
                    } if id == item_id => Some((s, o)),
                    _ => None,
                }) {
                    *existing.0 = *status;
                    *existing.1 = output.clone();
                } else {
                    self.items.push(TimelineItem::ToolCall {
                        id: item_id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                        status: *status,
                        output: output.clone(),
                    });
                }
            }
            ThreadEventKind::CommandRun {
                item_id,
                command,
                cwd,
                exit_code,
                output_tail,
            } => {
                self.items.push(TimelineItem::CommandRun {
                    id: item_id.clone(),
                    command: command.clone(),
                    cwd: cwd.clone(),
                    exit_code: *exit_code,
                    output_tail: output_tail.clone(),
                });
            }
            ThreadEventKind::FileChange {
                item_id,
                path,
                kind,
            } => {
                self.items.push(TimelineItem::FileChange {
                    id: item_id.clone(),
                    path: path.clone(),
                    kind: *kind,
                });
            }
            ThreadEventKind::ApprovalRequested {
                request_id,
                kind,
                detail,
            } => {
                self.thread.status = ThreadStatus::AwaitingApproval;
                self.items.push(TimelineItem::Approval {
                    request_id: request_id.clone(),
                    kind: *kind,
                    detail: detail.clone(),
                    decision: None,
                });
            }
            ThreadEventKind::ApprovalResolved {
                request_id,
                decision,
                ..
            } => {
                if let Some(TimelineItem::Approval { decision: d, .. }) =
                    self.items.iter_mut().find(|i| match i {
                        TimelineItem::Approval { request_id: id, .. } => id == request_id,
                        _ => false,
                    })
                {
                    *d = Some(*decision);
                }
                self.thread.status = ThreadStatus::Working;
            }
            ThreadEventKind::PlanUpdated { steps } => {
                self.items.push(TimelineItem::Plan {
                    steps: steps.clone(),
                });
            }
            ThreadEventKind::TokenUsage { usage } => {
                if let Some(turn) = self.turns.last_mut() {
                    turn.usage = Some(usage.clone());
                }
            }
            ThreadEventKind::RateLimitObserved { provider, details } => {
                self.items.push(TimelineItem::Notice {
                    level: "warning".to_string(),
                    message: format!("Rate limit observed on {provider}: {details}"),
                });
            }
            ThreadEventKind::BudgetExceeded { limit } => {
                self.items.push(TimelineItem::Notice {
                    level: "error".to_string(),
                    message: format!("Budget limit exceeded: {limit}"),
                });
            }
            ThreadEventKind::Notice { level, message } => {
                self.items.push(TimelineItem::Notice {
                    level: level.clone(),
                    message: message.clone(),
                });
            }
            ThreadEventKind::Error {
                class,
                message,
                recoverable,
            } => {
                if !*recoverable {
                    self.thread.status = ThreadStatus::Errored;
                }
                self.items.push(TimelineItem::Error {
                    class: class.clone(),
                    message: message.clone(),
                    recoverable: *recoverable,
                });
            }
            ThreadEventKind::TurnCompleted { outcome } => {
                if let Some(turn) = self.turns.last_mut() {
                    turn.status = match outcome {
                        TurnOutcome::Success => TurnStatus::Completed,
                        TurnOutcome::Cancelled => TurnStatus::Interrupted,
                        TurnOutcome::Failed => TurnStatus::Failed,
                        TurnOutcome::Interrupted => TurnStatus::Interrupted,
                    };
                    turn.ended_at_ms = Some(event.at_ms);
                }
            }
            ThreadEventKind::TurnSettled { changed_files } => {
                self.changed_files = changed_files.clone();
                self.thread.status = ThreadStatus::Idle;
            }
            ThreadEventKind::SubAgentSpawned {
                sub_agent_id,
                title,
                agent_type,
                model,
                ..
            } => {
                self.sub_agents.insert(
                    sub_agent_id.clone(),
                    SubAgentProjection {
                        id: sub_agent_id.clone(),
                        title: title.clone(),
                        agent_type: agent_type.clone(),
                        model: model.clone(),
                        latest_activity: None,
                        tokens: 0,
                        tool_calls: 0,
                        outcome: None,
                        finished: false,
                    },
                );
            }
            ThreadEventKind::SubAgentActivity {
                sub_agent_id,
                preview,
            } => {
                if let Some(agent) = self.sub_agents.get_mut(sub_agent_id) {
                    agent.latest_activity = Some(preview.clone());
                }
            }
            ThreadEventKind::SubAgentUsage {
                sub_agent_id,
                tokens,
                tool_calls,
            } => {
                if let Some(agent) = self.sub_agents.get_mut(sub_agent_id) {
                    agent.tokens = *tokens;
                    agent.tool_calls = *tool_calls;
                }
            }
            ThreadEventKind::SubAgentFinished {
                sub_agent_id,
                outcome,
                ..
            } => {
                if let Some(agent) = self.sub_agents.get_mut(sub_agent_id) {
                    agent.outcome = Some(outcome.clone());
                    agent.finished = true;
                }
            }
        }
    }
}

/// A rendered item inside a thread's timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineItem {
    TurnUserPrompt {
        turn_id: String,
        text: String,
    },
    AssistantText {
        id: String,
        text: String,
    },
    Reasoning {
        id: String,
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
        status: ToolCallStatus,
        output: Option<String>,
    },
    CommandRun {
        id: String,
        command: String,
        cwd: String,
        exit_code: Option<i32>,
        output_tail: Option<String>,
    },
    FileChange {
        id: String,
        path: String,
        kind: FileChangeKind,
    },
    Approval {
        request_id: String,
        kind: ApprovalKind,
        detail: serde_json::Value,
        decision: Option<ApprovalDecision>,
    },
    Plan {
        steps: Vec<PlanStep>,
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
}

/// Status and activity projection of a provider sub-agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentProjection {
    pub id: String,
    pub title: String,
    pub agent_type: String,
    pub model: String,
    pub latest_activity: Option<String>,
    pub tokens: u64,
    pub tool_calls: u32,
    pub outcome: Option<String>,
    pub finished: bool,
}

/// Client-side projection of an orchestrator or schedule run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunProjection {
    pub run: Run,
}

impl RunProjection {
    pub fn new(run: Run) -> Self {
        Self { run }
    }

    /// Reducer applying a RunEvent into the projection.
    pub fn apply_event(&mut self, event: &RunEvent) {
        match event {
            RunEvent::Planned { tasks, .. } => {
                self.run.tasks = tasks.clone();
                self.run.status = RunStatus::Planned;
            }
            RunEvent::ApprovalRequested { .. } => {
                self.run.status = RunStatus::Paused;
            }
            RunEvent::TaskDispatched {
                task_id, thread_id, ..
            } => {
                self.run.status = RunStatus::Running;
                if let Some(task) = self.run.tasks.iter_mut().find(|t| &t.task_id == task_id) {
                    task.thread_id = Some(thread_id.clone());
                    task.status = RunTaskStatus::Dispatched;
                }
            }
            RunEvent::TaskRerouted { .. } => {}
            RunEvent::TaskFinished {
                task_id, outcome, ..
            } => {
                if let Some(task) = self.run.tasks.iter_mut().find(|t| &t.task_id == task_id) {
                    task.status = RunTaskStatus::Finished;
                    task.summary = Some(outcome.clone());
                }
            }
            RunEvent::Paused { .. } => {
                self.run.status = RunStatus::Paused;
            }
            RunEvent::Resumed { .. } => {
                self.run.status = RunStatus::Running;
            }
            RunEvent::Finished { status, .. } => {
                self.run.status = *status;
            }
        }
    }
}
