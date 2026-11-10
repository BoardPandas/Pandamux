use crate::ids::{EnvironmentId, ProjectId, ScheduleId};
use serde::{Deserialize, Serialize};

/// Machine-local schedule definition stored on the node that executes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRecord {
    pub id: ScheduleId,
    pub name: String,
    /// Canonical project identity used for organization-policy enforcement.
    #[serde(default)]
    pub project_id: Option<ProjectId>,
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

impl ScheduleRecord {
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("schedule id", self.id.as_str()),
            ("schedule name", self.name.as_str()),
            ("environment id", self.environment.id.as_str()),
            (
                "environment display name",
                self.environment.display_name.as_str(),
            ),
            ("cron expression", self.trigger.cron.as_str()),
            ("timezone", self.trigger.timezone.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("{field} must not be empty"));
            }
        }
        if self
            .project_id
            .as_ref()
            .is_some_and(|project_id| project_id.as_str().trim().is_empty())
        {
            return Err("project id must be absent or non-empty".to_string());
        }
        if matches!(self.overlap, ScheduleOverlapPolicy::Parallel { max: 0 }) {
            return Err("parallel overlap max must be greater than zero".to_string());
        }
        if self
            .budget
            .per_run
            .is_some_and(|budget| budget.max_runtime_minutes == Some(0))
        {
            return Err("per-run runtime budget must be greater than zero".to_string());
        }
        if self
            .budget
            .per_day
            .and_then(|budget| budget.max_cost_usd)
            .is_some_and(|cost| !cost.is_finite() || cost.is_sign_negative())
        {
            return Err("per-day cost budget must be finite and non-negative".to_string());
        }
        match &self.target {
            ScheduleTarget::Orchestrator { prompt } => {
                if prompt.trim().is_empty() {
                    return Err("orchestrator prompt must not be empty".to_string());
                }
            }
            ScheduleTarget::Agent {
                agent,
                project_path,
                prompt,
            } => {
                if agent.trim().is_empty()
                    || project_path.trim().is_empty()
                    || prompt.trim().is_empty()
                {
                    return Err("agent, project path, and prompt must not be empty".to_string());
                }
                if self.project_id.is_none() {
                    return Err(
                        "agent schedules require a project id for organization policy".to_string(),
                    );
                }
            }
            ScheduleTarget::Session {
                project_path,
                prompt,
                provider,
                model,
                ..
            } => {
                if project_path.trim().is_empty() || prompt.trim().is_empty() {
                    return Err("session project path and prompt must not be empty".to_string());
                }
                if self.project_id.is_none() {
                    return Err(
                        "session schedules require a project id for organization policy"
                            .to_string(),
                    );
                }
                if provider
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                {
                    return Err("schedule provider must be absent or non-empty".to_string());
                }
                if model
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                {
                    return Err("schedule model must be absent or non-empty".to_string());
                }
            }
        }
        Ok(())
    }
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
