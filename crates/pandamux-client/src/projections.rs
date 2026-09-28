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
    #[serde(default)]
    pub item_timestamps_ms: HashMap<String, u64>,
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
            item_timestamps_ms: HashMap::new(),
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
                self.item_timestamps_ms
                    .insert(turn_id.as_str().to_string(), event.at_ms);
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
                    attachments: input.attachments.clone(),
                });
            }
            ThreadEventKind::TurnStarted { turn_id } => {
                if let Some(turn) = self.turns.iter_mut().find(|t| &t.id == turn_id) {
                    turn.status = TurnStatus::Running;
                }
                self.thread.status = ThreadStatus::Working;
            }
            ThreadEventKind::AssistantText { item_id, text } => {
                self.item_timestamps_ms.insert(item_id.clone(), event.at_ms);
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
                self.item_timestamps_ms.insert(item_id.clone(), event.at_ms);
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
                self.item_timestamps_ms.insert(item_id.clone(), event.at_ms);
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
                self.item_timestamps_ms.insert(item_id.clone(), event.at_ms);
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
                self.item_timestamps_ms.insert(item_id.clone(), event.at_ms);
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
                self.item_timestamps_ms
                    .insert(request_id.clone(), event.at_ms);
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
                if !changed_files.is_empty() {
                    let summaries: Vec<ChangedFileSummary> = changed_files
                        .iter()
                        .map(|s| ChangedFileSummary::parse(s))
                        .collect();
                    let total_adds = summaries.iter().map(|f| f.additions).sum();
                    let total_dels = summaries.iter().map(|f| f.deletions).sum();
                    self.items.push(TimelineItem::ChangedFiles {
                        files: summaries,
                        total_additions: total_adds,
                        total_deletions: total_dels,
                    });
                }
            }
            ThreadEventKind::SubAgentSpawned {
                sub_agent_id,
                parent_sub_agent_id,
                parent_item_id,
                title,
                agent_type,
                model,
                effort,
            } => {
                let proj = SubAgentProjection {
                    id: sub_agent_id.clone(),
                    title: title.clone(),
                    agent_type: agent_type.clone(),
                    model: model.clone(),
                    parent_sub_agent_id: parent_sub_agent_id.clone(),
                    parent_item_id: parent_item_id.clone(),
                    effort: effort.clone(),
                    latest_activity: None,
                    tokens: 0,
                    tool_calls: 0,
                    outcome: None,
                    elapsed_ms: None,
                    summary: None,
                    started_at_ms: event.at_ms,
                    finished: false,
                };
                self.sub_agents.insert(sub_agent_id.clone(), proj);
                self.items.push(TimelineItem::SubAgent {
                    id: sub_agent_id.clone(),
                    parent_sub_agent_id: parent_sub_agent_id.clone(),
                    parent_item_id: parent_item_id.clone(),
                    title: title.clone(),
                    agent_type: agent_type.clone(),
                    model: model.clone(),
                    effort: effort.clone(),
                    latest_activity: None,
                    tokens: 0,
                    tool_calls: 0,
                    outcome: None,
                    elapsed_ms: None,
                    summary: None,
                    finished: false,
                });
            }
            ThreadEventKind::SubAgentActivity {
                sub_agent_id,
                preview,
            } => {
                if let Some(agent) = self.sub_agents.get_mut(sub_agent_id) {
                    agent.latest_activity = Some(preview.clone());
                }
                for item in self.items.iter_mut().rev() {
                    if let TimelineItem::SubAgent {
                        id,
                        latest_activity,
                        ..
                    } = item
                        && id == sub_agent_id
                    {
                        *latest_activity = Some(preview.clone());
                        break;
                    }
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
                for item in self.items.iter_mut().rev() {
                    if let TimelineItem::SubAgent {
                        id,
                        tokens: t,
                        tool_calls: tc,
                        ..
                    } = item
                        && id == sub_agent_id
                    {
                        *t = *tokens;
                        *tc = *tool_calls;
                        break;
                    }
                }
            }
            ThreadEventKind::SubAgentFinished {
                sub_agent_id,
                outcome,
                elapsed_ms,
                summary,
            } => {
                if let Some(agent) = self.sub_agents.get_mut(sub_agent_id) {
                    agent.outcome = Some(outcome.clone());
                    agent.elapsed_ms = Some(*elapsed_ms);
                    agent.summary = summary.clone();
                    agent.finished = true;
                }
                for item in self.items.iter_mut().rev() {
                    if let TimelineItem::SubAgent {
                        id,
                        outcome: out,
                        elapsed_ms: el,
                        summary: sum,
                        finished,
                        ..
                    } = item
                        && id == sub_agent_id
                    {
                        *out = Some(outcome.clone());
                        *el = Some(*elapsed_ms);
                        *sum = summary.clone();
                        *finished = true;
                        break;
                    }
                }
            }
        }
    }

    /// Assembles the nested tree of sub-agents rooted at top-level agents (parent_sub_agent_id is None).
    pub fn subagent_tree(&self) -> Vec<SubAgentTreeNode> {
        let mut by_parent: HashMap<Option<String>, Vec<SubAgentProjection>> = HashMap::new();
        for agent in self.sub_agents.values() {
            by_parent
                .entry(agent.parent_sub_agent_id.clone())
                .or_default()
                .push(agent.clone());
        }

        for list in by_parent.values_mut() {
            list.sort_by(|a, b| a.id.cmp(&b.id));
        }

        fn build_subtree(
            parent_id: Option<&str>,
            by_parent: &HashMap<Option<String>, Vec<SubAgentProjection>>,
        ) -> Vec<SubAgentTreeNode> {
            let key = parent_id.map(|s| s.to_string());
            let Some(children) = by_parent.get(&key) else {
                return Vec::new();
            };
            children
                .iter()
                .map(|child| SubAgentTreeNode {
                    agent: child.clone(),
                    children: build_subtree(Some(&child.id), by_parent),
                })
                .collect()
        }

        build_subtree(None, &by_parent)
    }

    /// Returns the timeline items with consecutive intermediate reasoning, tool calls,
    /// command runs, and file changes aggregated into unified collapsible WorkLog items.
    pub fn grouped_items(&self) -> Vec<TimelineItem> {
        let mut result = Vec::new();
        let mut current_group: Vec<WorkLogEntry> = Vec::new();
        let mut group_item_ids: Vec<String> = Vec::new();

        let flush_group =
            |group: &mut Vec<WorkLogEntry>, ids: &mut Vec<String>, res: &mut Vec<TimelineItem>| {
                if group.is_empty() {
                    return;
                }
                let entries = std::mem::take(group);
                let item_ids = std::mem::take(ids);
                let first_id = match entries.first() {
                    Some(WorkLogEntry::Reasoning { id, .. }) => id.clone(),
                    Some(WorkLogEntry::ToolCall { id, .. }) => id.clone(),
                    Some(WorkLogEntry::CommandRun { id, .. }) => id.clone(),
                    Some(WorkLogEntry::FileChange { id, .. }) => id.clone(),
                    None => "work-log".to_string(),
                };

                // Calculate timing based on item timestamps or turn elapsed
                let mut min_ts: Option<u64> = None;
                let mut max_ts: Option<u64> = None;
                for id in &item_ids {
                    if let Some(&ts) = self.item_timestamps_ms.get(id) {
                        min_ts = Some(min_ts.map_or(ts, |m| m.min(ts)));
                        max_ts = Some(max_ts.map_or(ts, |m| m.max(ts)));
                    }
                }

                // Determine status
                let is_any_failed = entries.iter().any(|e| match e {
                    WorkLogEntry::ToolCall { status, .. } => *status == ToolCallStatus::Failed,
                    WorkLogEntry::CommandRun { exit_code, .. } => *exit_code != Some(0),
                    _ => false,
                });
                let is_any_running = entries.iter().any(|e| match e {
                    WorkLogEntry::ToolCall { status, .. } => {
                        *status == ToolCallStatus::Executing || *status == ToolCallStatus::Pending
                    }
                    _ => false,
                });

                let status = if is_any_failed {
                    WorkLogStatus::Failed
                } else if is_any_running || self.thread.status == ThreadStatus::Working {
                    WorkLogStatus::Running
                } else {
                    WorkLogStatus::Completed
                };

                let duration_ms = match (min_ts, max_ts) {
                    (Some(s), Some(e)) if e >= s => Some(e - s),
                    (Some(s), None) => {
                        let now_ms = self.turns.last().and_then(|t| t.ended_at_ms).unwrap_or(s);
                        Some(now_ms.saturating_sub(s))
                    }
                    _ => {
                        if let Some(turn) = self.turns.last() {
                            turn.ended_at_ms
                                .map(|e| e.saturating_sub(turn.started_at_ms))
                        } else {
                            None
                        }
                    }
                };

                let summary = match status {
                    WorkLogStatus::Running => {
                        if let Some(dur) = duration_ms {
                            format!("Thinking and executing tools ({})", format_duration(dur))
                        } else {
                            "Thinking and executing tools...".to_string()
                        }
                    }
                    WorkLogStatus::Completed => {
                        let count = entries.len();
                        let items_label = if count == 1 {
                            "1 item"
                        } else {
                            &format!("{count} items")
                        };
                        let only_reasoning = entries
                            .iter()
                            .all(|e| matches!(e, WorkLogEntry::Reasoning { .. }));
                        if only_reasoning {
                            if let Some(dur) = duration_ms {
                                format!("Thought for {}", format_duration(dur))
                            } else {
                                "Thought process completed".to_string()
                            }
                        } else if let Some(dur) = duration_ms {
                            format!("Worked for {} ({items_label})", format_duration(dur))
                        } else {
                            format!("Worked on {items_label}")
                        }
                    }
                    WorkLogStatus::Failed => {
                        let count = entries.len();
                        if let Some(dur) = duration_ms {
                            format!("Failed after {} ({count} items)", format_duration(dur))
                        } else {
                            format!("Failed during work ({count} items)")
                        }
                    }
                };

                res.push(TimelineItem::WorkLog {
                    id: format!("wl-{first_id}"),
                    summary,
                    entries,
                    duration_ms,
                    status,
                });
            };

        for item in &self.items {
            if let Some(entry) = item.as_work_log_entry() {
                let id = match item {
                    TimelineItem::Reasoning { id, .. } => id.clone(),
                    TimelineItem::ToolCall { id, .. } => id.clone(),
                    TimelineItem::CommandRun { id, .. } => id.clone(),
                    TimelineItem::FileChange { id, .. } => id.clone(),
                    _ => String::new(),
                };
                group_item_ids.push(id);
                current_group.push(entry);
            } else {
                flush_group(&mut current_group, &mut group_item_ids, &mut result);
                result.push(item.clone());
            }
        }

        flush_group(&mut current_group, &mut group_item_ids, &mut result);
        result
    }
}

/// Status of an aggregated work log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkLogStatus {
    Running,
    Completed,
    Failed,
}

impl WorkLogStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

/// An entry within a grouped work log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "entry_type", rename_all = "snake_case")]
pub enum WorkLogEntry {
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
}

/// Formats duration in milliseconds to a concise human-readable string.
/// E.g. 680_000 -> "11m 20s", 12_000 -> "12s", 500 -> "<1s".
pub fn format_duration(duration_ms: u64) -> String {
    let total_secs = duration_ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    if mins > 0 {
        format!("{mins}m {secs}s")
    } else if secs > 0 {
        format!("{secs}s")
    } else {
        "<1s".to_string()
    }
}

/// Summary of a file modified, created, or deleted during a turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFileSummary {
    pub path: String,
    pub kind: FileChangeKind,
    pub additions: usize,
    pub deletions: usize,
}

impl ChangedFileSummary {
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        if let Some(idx) = raw.find(" (+") {
            let path = raw[..idx].trim().to_string();
            let rest = &raw[idx + 3..];
            let adds = rest
                .split(' ')
                .next()
                .unwrap_or("0")
                .parse::<usize>()
                .unwrap_or(0);
            let dels = if let Some(d_idx) = rest.find('-') {
                rest[d_idx + 1..]
                    .trim_end_matches(')')
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(0)
            } else {
                0
            };
            Self {
                path,
                kind: FileChangeKind::Modified,
                additions: adds,
                deletions: dels,
            }
        } else if let Some(stripped) = raw.strip_prefix("A\t").or_else(|| raw.strip_prefix("A ")) {
            Self {
                path: stripped.trim().to_string(),
                kind: FileChangeKind::Created,
                additions: 0,
                deletions: 0,
            }
        } else if let Some(stripped) = raw.strip_prefix("D\t").or_else(|| raw.strip_prefix("D ")) {
            Self {
                path: stripped.trim().to_string(),
                kind: FileChangeKind::Deleted,
                additions: 0,
                deletions: 0,
            }
        } else {
            Self {
                path: raw.to_string(),
                kind: FileChangeKind::Modified,
                additions: 0,
                deletions: 0,
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
        #[serde(default)]
        attachments: Vec<pandamux_core::AttachmentRecord>,
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
    WorkLog {
        id: String,
        summary: String,
        entries: Vec<WorkLogEntry>,
        duration_ms: Option<u64>,
        status: WorkLogStatus,
    },
    ChangedFiles {
        files: Vec<ChangedFileSummary>,
        total_additions: usize,
        total_deletions: usize,
    },
    SubAgent {
        id: String,
        #[serde(default)]
        parent_sub_agent_id: Option<String>,
        #[serde(default)]
        parent_item_id: Option<String>,
        title: String,
        agent_type: String,
        model: String,
        #[serde(default)]
        effort: Option<String>,
        #[serde(default)]
        latest_activity: Option<String>,
        tokens: u64,
        tool_calls: u32,
        #[serde(default)]
        outcome: Option<String>,
        #[serde(default)]
        elapsed_ms: Option<u64>,
        #[serde(default)]
        summary: Option<String>,
        finished: bool,
    },
}

impl TimelineItem {
    pub fn as_work_log_entry(&self) -> Option<WorkLogEntry> {
        match self {
            TimelineItem::Reasoning { id, text } => Some(WorkLogEntry::Reasoning {
                id: id.clone(),
                text: text.clone(),
            }),
            TimelineItem::ToolCall {
                id,
                name,
                input,
                status,
                output,
            } => Some(WorkLogEntry::ToolCall {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
                status: *status,
                output: output.clone(),
            }),
            TimelineItem::CommandRun {
                id,
                command,
                cwd,
                exit_code,
                output_tail,
            } => Some(WorkLogEntry::CommandRun {
                id: id.clone(),
                command: command.clone(),
                cwd: cwd.clone(),
                exit_code: *exit_code,
                output_tail: output_tail.clone(),
            }),
            TimelineItem::FileChange { id, path, kind } => Some(WorkLogEntry::FileChange {
                id: id.clone(),
                path: path.clone(),
                kind: *kind,
            }),
            _ => None,
        }
    }
}

/// Status and activity projection of a provider sub-agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentProjection {
    pub id: String,
    pub title: String,
    pub agent_type: String,
    pub model: String,
    #[serde(default)]
    pub parent_sub_agent_id: Option<String>,
    #[serde(default)]
    pub parent_item_id: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub latest_activity: Option<String>,
    pub tokens: u64,
    pub tool_calls: u32,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub started_at_ms: u64,
    pub finished: bool,
}

/// A node in the provider sub-agent tree representing a nested or top-level sub-agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentTreeNode {
    pub agent: SubAgentProjection,
    #[serde(default)]
    pub children: Vec<SubAgentTreeNode>,
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
