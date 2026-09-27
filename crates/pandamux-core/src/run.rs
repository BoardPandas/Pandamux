use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::ids::{AgentId, ProjectId, RunId, ScheduleId, ThreadId};
use crate::thread::TurnUsage;

/// Multi-repo orchestrator execution run or scheduled task invocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: RunId,
    pub kind: RunKind,
    #[serde(default)]
    pub status: RunStatus,
    pub trigger: String,
    pub started_at_ms: u64,
    #[serde(default)]
    pub ended_at_ms: Option<u64>,
    #[serde(default)]
    pub tasks: Vec<RunTask>,
    #[serde(default)]
    pub blackboard: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub usage: Option<TurnUsage>,
    #[serde(default)]
    pub outcome: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunKind {
    Orchestrator,
    Schedule {
        schedule_id: ScheduleId,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    Planned,
    Running,
    Paused,
    Succeeded,
    Failed,
    Cancelled,
}

/// A node in a multi-repo orchestration DAG.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTask {
    pub task_id: String,
    #[serde(default)]
    pub target_project_id: Option<ProjectId>,
    #[serde(default)]
    pub agent_id: Option<AgentId>,
    #[serde(default)]
    pub thread_id: Option<ThreadId>,
    #[serde(default)]
    pub status: RunTaskStatus,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTaskStatus {
    #[default]
    Pending,
    Dispatched,
    Running,
    Finished,
    Failed,
    Skipped,
}

/// Lifecycle events emitted during orchestrator run progression.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    Planned {
        run_id: RunId,
        tasks: Vec<RunTask>,
    },
    ApprovalRequested {
        run_id: RunId,
        request_id: String,
        reason: String,
    },
    TaskDispatched {
        run_id: RunId,
        task_id: String,
        #[serde(default)]
        target_project_id: Option<ProjectId>,
        #[serde(default)]
        agent_id: Option<AgentId>,
        thread_id: ThreadId,
    },
    TaskRerouted {
        run_id: RunId,
        task_id: String,
        new_route: String,
    },
    TaskFinished {
        run_id: RunId,
        task_id: String,
        outcome: String,
    },
    Paused {
        run_id: RunId,
        reason: String,
    },
    Resumed {
        run_id: RunId,
    },
    Finished {
        run_id: RunId,
        status: RunStatus,
    },
}
