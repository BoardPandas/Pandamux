use pandamux_core::{ScheduleId, ScheduleRecord, ThreadId, TurnId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleValidateParams {
    pub schedule: ScheduleRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleValidationResult {
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_name: Option<String>,
    #[serde(default)]
    pub blocking_reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSaveParams {
    pub schedule: ScheduleRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRunParams {
    pub schedule_id: ScheduleId,
    #[serde(default)]
    pub approved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRunResult {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
}
