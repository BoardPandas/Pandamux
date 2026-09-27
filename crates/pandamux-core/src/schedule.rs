use serde::{Deserialize, Serialize};
use crate::ids::{EnvironmentId, ScheduleId};

/// Machine-local schedule definition stored on the node that executes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRecord {
    pub id: ScheduleId,
    pub name: String,
    pub environment: ScheduleEnvironmentRef,
    pub trigger: ScheduleTrigger,
    pub target: ScheduleTarget,
    #[serde(default)]
    pub approval: ScheduleApprovalPolicy,
    #[serde(default)]
    pub overlap: ScheduleOverlapPolicy,
    #[serde(default)]
    pub catch_up: ScheduleCatchUpPolicy,
    #[serde(default)]
    pub budget: ScheduleBudget,
    #[serde(default)]
    pub notify: Vec<String>,
    #[serde(default)]
    pub full_access_opt_in: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleEnvironmentRef {
    pub id: EnvironmentId,
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleTrigger {
    pub cron: String,
    pub timezone: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleTarget {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleThreadMode {
    #[default]
    NewThreadPerRun,
    ContinueExistingThread,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleApprovalPolicy {
    #[default]
    Auto,
    ApproveFirst,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScheduleOverlapPolicy {
    #[default]
    Skip,
    Queue,
    Parallel {
        max: u32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleCatchUpPolicy {
    #[default]
    Skip,
    RunOnce,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleBudget {
    #[serde(default)]
    pub per_run: Option<PerRunBudget>,
    #[serde(default)]
    pub per_day: Option<PerDayBudget>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerRunBudget {
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_runtime_minutes: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerDayBudget {
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
}
