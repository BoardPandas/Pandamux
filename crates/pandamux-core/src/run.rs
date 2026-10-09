use crate::agent_def::OutputContract;
use crate::ids::{
    AgentId, ApprovalId, BlackboardEntryId, BudgetReservationId, DependencyEdgeId, EnvironmentId,
    OrchestrationEffectId, OrganizationId, ProjectId, RunAttemptId, RunEventId, RunId, RunTaskId,
    ThreadId, TurnAttemptId,
};
use crate::routing::{AttemptBoundary, RoutingDecision};
use crate::thread::AccessMode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationMode {
    Auto,
    #[default]
    ApproveFirst,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunTrigger {
    Manual { client_id: String },
    Orchestrator { parent_run_id: RunId },
    Schedule { schedule_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSeed {
    pub trigger: RunTrigger,
    pub policy_revision: String,
    pub bundle_sha: String,
    pub organization_id: OrganizationId,
    pub mode: OrchestrationMode,
    pub budget_limit: BudgetAmount,
    pub max_parallel_tasks: u32,
}

/// Canonical replayable orchestration state. All replay-affecting collections are ordered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: RunId,
    pub trigger: RunTrigger,
    pub policy_revision: String,
    pub bundle_sha: String,
    pub organization_id: OrganizationId,
    pub mode: OrchestrationMode,
    pub status: RunStatus,
    pub last_seq: u64,
    #[serde(default)]
    pub validated_plan: Option<ValidatedPlan>,
    #[serde(default)]
    pub tasks: BTreeMap<RunTaskId, AgentTask>,
    #[serde(default)]
    pub attempts: BTreeMap<RunAttemptId, Attempt>,
    #[serde(default)]
    pub edges: BTreeMap<DependencyEdgeId, DependencyEdge>,
    #[serde(default)]
    pub approvals: BTreeMap<ApprovalId, ApprovalRequirement>,
    #[serde(default)]
    pub blackboard: BTreeMap<BlackboardEntryId, BlackboardEntry>,
    #[serde(default)]
    pub reservations: BTreeMap<BudgetReservationId, BudgetReservation>,
    #[serde(default)]
    pub checkpoints: BTreeMap<RunAttemptId, CheckpointRef>,
    #[serde(default)]
    pub routes: BTreeMap<RunTaskId, TaskRouteDecision>,
    #[serde(default)]
    pub replay_receipts: BTreeMap<RunEventId, String>,
    #[serde(default)]
    pub idempotency_receipts: BTreeMap<String, RunEventId>,
    #[serde(default)]
    pub pending_effects: BTreeMap<OrchestrationEffectId, OrchestrationEffect>,
    #[serde(default)]
    pub settled_effects: BTreeMap<OrchestrationEffectId, EffectReceipt>,
    pub budget: RunBudget,
    pub max_parallel_tasks: u32,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub ended_at_ms: Option<u64>,
    #[serde(default)]
    pub outcome: Option<String>,
}

impl Run {
    pub fn draft(id: RunId, seed: RunSeed, created_at_ms: u64) -> Self {
        Self {
            id,
            trigger: seed.trigger,
            policy_revision: seed.policy_revision,
            bundle_sha: seed.bundle_sha,
            organization_id: seed.organization_id,
            mode: seed.mode,
            status: RunStatus::Draft,
            last_seq: 0,
            validated_plan: None,
            tasks: BTreeMap::new(),
            attempts: BTreeMap::new(),
            edges: BTreeMap::new(),
            approvals: BTreeMap::new(),
            blackboard: BTreeMap::new(),
            reservations: BTreeMap::new(),
            checkpoints: BTreeMap::new(),
            routes: BTreeMap::new(),
            replay_receipts: BTreeMap::new(),
            idempotency_receipts: BTreeMap::new(),
            pending_effects: BTreeMap::new(),
            settled_effects: BTreeMap::new(),
            budget: RunBudget {
                limit: seed.budget_limit,
                ..RunBudget::default()
            },
            max_parallel_tasks: seed.max_parallel_tasks,
            created_at_ms,
            updated_at_ms: created_at_ms,
            ended_at_ms: None,
            outcome: None,
        }
    }

    pub const fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    Draft,
    AwaitingApproval,
    Ready,
    Running,
    Paused,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedPlan {
    pub schema_version: u32,
    pub source_digest: String,
    #[serde(default)]
    pub tasks: BTreeMap<RunTaskId, AgentTask>,
    #[serde(default)]
    pub edges: BTreeMap<DependencyEdgeId, DependencyEdge>,
    #[serde(default)]
    pub topological_order: Vec<RunTaskId>,
    pub total_budget: BudgetAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTask {
    pub id: RunTaskId,
    pub summary: String,
    pub class: String,
    pub project_id: ProjectId,
    pub organization_id: OrganizationId,
    #[serde(default)]
    pub required_capabilities: BTreeSet<String>,
    pub access_ceiling: AccessMode,
    #[serde(default)]
    pub inputs: Vec<TaskInput>,
    #[serde(default)]
    pub outputs: Vec<OutputContract>,
    #[serde(default)]
    pub selected_agent_id: Option<AgentId>,
    #[serde(default)]
    pub route: Option<TaskRouteDecision>,
    pub status: RunTaskStatus,
    pub max_attempts: u32,
    #[serde(default)]
    pub approval_ids: BTreeSet<ApprovalId>,
    pub budget_limit: BudgetAmount,
    #[serde(default)]
    pub budget_reservation_id: Option<BudgetReservationId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInput {
    pub name: String,
    pub contract: OutputContract,
    #[serde(default)]
    pub producer_task_id: Option<RunTaskId>,
    #[serde(default)]
    pub blackboard_entry_id: Option<BlackboardEntryId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTaskStatus {
    #[default]
    Pending,
    Blocked,
    AwaitingApproval,
    Ready,
    DispatchRequested,
    Running,
    Succeeded,
    Failed,
    Skipped,
    Cancelled,
}

impl RunTaskStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Skipped | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub id: RunAttemptId,
    pub task_id: RunTaskId,
    pub ordinal: u32,
    pub route: TaskRouteDecision,
    #[serde(default)]
    pub thread_id: Option<ThreadId>,
    #[serde(default)]
    pub turn_attempt_id: Option<TurnAttemptId>,
    pub status: AttemptStatus,
    pub boundary: AttemptBoundary,
    pub started_at_ms: u64,
    #[serde(default)]
    pub ended_at_ms: Option<u64>,
    #[serde(default)]
    pub failure: Option<AttemptFailure>,
    #[serde(default)]
    pub usage_reconciled: bool,
    #[serde(default)]
    pub ambiguous_effect: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    #[default]
    DispatchRequested,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptFailure {
    pub class: FailureClass,
    pub code: String,
    pub safe_message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Capacity,
    Provider,
    Policy,
    Validation,
    Cancelled,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyEdge {
    pub id: DependencyEdgeId,
    pub source: RunTaskId,
    pub target: RunTaskId,
    pub requirement: DependencyRequirement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyRequirement {
    RequiresSuccess,
    RequiresSettlement,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlackboardEntry {
    pub id: BlackboardEntryId,
    pub version: u64,
    pub producer: RunTaskId,
    #[serde(default)]
    pub consumers: BTreeSet<RunTaskId>,
    pub output_contract: OutputContract,
    pub content: serde_json::Value,
    pub content_sha256: String,
    pub created_event_id: RunEventId,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetAmount {
    pub tokens: u64,
    pub cost_micro_usd: u64,
    pub runtime_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBudget {
    pub limit: BudgetAmount,
    pub reserved: BudgetAmount,
    pub reconciled: BudgetAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetReservation {
    pub id: BudgetReservationId,
    pub task_id: RunTaskId,
    #[serde(default)]
    pub attempt_id: Option<RunAttemptId>,
    pub amount: BudgetAmount,
    pub state: BudgetReservationState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetReservationState {
    Reserved,
    Reconciled,
    Released,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequirement {
    pub id: ApprovalId,
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub kind: RunApprovalKind,
    pub policy_revision: String,
    pub action_digest: String,
    pub safe_summary: String,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    pub expires_at_ms: u64,
    pub status: RunApprovalStatus,
    #[serde(default)]
    pub decision: Option<ApprovalDecisionProvenance>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunApprovalKind {
    Plan,
    Access,
    Budget,
    Retry,
    AgentEdit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionProvenance {
    pub actor: String,
    pub decided_at_ms: u64,
    pub decision_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointRef {
    pub value: String,
    pub task_id: RunTaskId,
    pub attempt_id: RunAttemptId,
    pub thread_id: ThreadId,
    pub kind: CheckpointKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    BeforeAttempt,
    AfterAttempt,
    Recovery,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSelectionScore {
    pub class_rule: i64,
    pub specialty: i64,
    pub provider_preference: i64,
    pub total: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvaluation {
    pub agent_id: AgentId,
    pub eligible: bool,
    #[serde(default)]
    pub rejection_codes: Vec<String>,
    #[serde(default)]
    pub explanation_codes: Vec<String>,
    #[serde(default)]
    pub score: Option<AgentSelectionScore>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRouteDecision {
    pub task_id: RunTaskId,
    #[serde(default)]
    pub selected_agent_id: Option<AgentId>,
    #[serde(default)]
    pub agent_evaluations: Vec<AgentEvaluation>,
    pub provider: RoutingDecision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub event_id: RunEventId,
    pub run_id: RunId,
    pub seq: u64,
    pub at_ms: u64,
    pub idempotency_key: String,
    pub kind: RunEventKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEventKind {
    PlanAccepted {
        seed: RunSeed,
        plan: ValidatedPlan,
    },
    ApprovalRequested {
        approval: ApprovalRequirement,
    },
    ApprovalResolved {
        approval_id: ApprovalId,
        action_digest: String,
        status: RunApprovalStatus,
        provenance: ApprovalDecisionProvenance,
    },
    BudgetReserved {
        reservation: BudgetReservation,
    },
    BudgetReconciled {
        reservation_id: BudgetReservationId,
        actual: BudgetAmount,
    },
    BudgetReleased {
        reservation_id: BudgetReservationId,
    },
    RouteSelected {
        decision: TaskRouteDecision,
    },
    DispatchRequested {
        attempt: Attempt,
    },
    AttemptStarted {
        attempt_id: RunAttemptId,
        thread_id: ThreadId,
        turn_attempt_id: TurnAttemptId,
    },
    AttemptBoundaryAdvanced {
        attempt_id: RunAttemptId,
        boundary: AttemptBoundary,
    },
    AttemptSettled {
        attempt_id: RunAttemptId,
        status: AttemptStatus,
        failure: Option<AttemptFailure>,
        usage_reconciled: bool,
    },
    BlackboardPublished {
        entry: BlackboardEntry,
    },
    TaskSettled {
        task_id: RunTaskId,
        status: RunTaskStatus,
        summary: Option<String>,
    },
    CancellationRequested {
        task_ids: Vec<RunTaskId>,
    },
    Paused {
        reason: String,
    },
    Resumed,
    EffectAcknowledged {
        effect_id: OrchestrationEffectId,
        receipt: EffectReceipt,
    },
    RunSettled {
        status: RunStatus,
        outcome: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunCommand {
    AcceptPlan {
        seed: RunSeed,
        plan: ValidatedPlan,
    },
    RequestApproval {
        approval: ApprovalRequirement,
    },
    ResolveApproval {
        approval_id: ApprovalId,
        action_digest: String,
        status: RunApprovalStatus,
        provenance: ApprovalDecisionProvenance,
    },
    ReserveBudget {
        reservation: BudgetReservation,
    },
    ReconcileBudget {
        reservation_id: BudgetReservationId,
        actual: BudgetAmount,
    },
    ReleaseBudget {
        reservation_id: BudgetReservationId,
    },
    SelectRoute {
        decision: TaskRouteDecision,
    },
    RequestDispatch {
        attempt: Attempt,
    },
    StartAttempt {
        attempt_id: RunAttemptId,
        thread_id: ThreadId,
        turn_attempt_id: TurnAttemptId,
    },
    AdvanceAttemptBoundary {
        attempt_id: RunAttemptId,
        boundary: AttemptBoundary,
    },
    SettleAttempt {
        attempt_id: RunAttemptId,
        status: AttemptStatus,
        failure: Option<AttemptFailure>,
        usage_reconciled: bool,
    },
    PublishBlackboard {
        entry: BlackboardEntry,
    },
    SettleTask {
        task_id: RunTaskId,
        status: RunTaskStatus,
        summary: Option<String>,
    },
    Cancel,
    Pause {
        reason: String,
    },
    Resume,
    AcknowledgeEffect {
        effect_id: OrchestrationEffectId,
        receipt: EffectReceipt,
    },
    SettleRun {
        status: RunStatus,
        outcome: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationEffect {
    pub id: OrchestrationEffectId,
    pub source_event_id: RunEventId,
    pub kind: OrchestrationEffectKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OrchestrationEffectKind {
    DispatchTask {
        run_id: RunId,
        task_id: RunTaskId,
        attempt_id: RunAttemptId,
        route: TaskRouteDecision,
    },
    CancelAttempt {
        run_id: RunId,
        task_id: RunTaskId,
        attempt_id: RunAttemptId,
    },
    RequestApproval {
        approval: ApprovalRequirement,
    },
    PublishBlackboardEntry {
        entry: BlackboardEntry,
    },
    NotifyRunState {
        run_id: RunId,
        status: RunStatus,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectReceipt {
    pub effect_id: OrchestrationEffectId,
    pub acknowledged_at_ms: u64,
    pub result_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingEnvironment {
    pub environment_id: EnvironmentId,
    pub organization_id: OrganizationId,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str) -> AgentTask {
        AgentTask {
            id: RunTaskId::from(id),
            summary: id.to_string(),
            class: "build".to_string(),
            project_id: ProjectId::from("proj-1"),
            organization_id: OrganizationId::from("org-1"),
            required_capabilities: BTreeSet::new(),
            access_ceiling: AccessMode::WorkspaceOnly,
            inputs: Vec::new(),
            outputs: vec![OutputContract {
                kind: "patch".to_string(),
                schema: None,
            }],
            selected_agent_id: None,
            route: None,
            status: RunTaskStatus::Pending,
            max_attempts: 2,
            approval_ids: BTreeSet::new(),
            budget_limit: BudgetAmount::default(),
            budget_reservation_id: None,
        }
    }

    #[test]
    fn ordered_run_maps_serialize_identically_across_insertion_order() {
        let seed = RunSeed {
            trigger: RunTrigger::Manual {
                client_id: "client".to_string(),
            },
            policy_revision: "policy-1".to_string(),
            bundle_sha: "bundle-1".to_string(),
            organization_id: OrganizationId::from("org-1"),
            mode: OrchestrationMode::ApproveFirst,
            budget_limit: BudgetAmount::default(),
            max_parallel_tasks: 2,
        };
        let mut left = Run::draft(RunId::from("run-1"), seed.clone(), 10);
        left.tasks.insert(RunTaskId::from("task-b"), task("task-b"));
        left.tasks.insert(RunTaskId::from("task-a"), task("task-a"));

        let mut right = Run::draft(RunId::from("run-1"), seed, 10);
        right
            .tasks
            .insert(RunTaskId::from("task-a"), task("task-a"));
        right
            .tasks
            .insert(RunTaskId::from("task-b"), task("task-b"));

        assert_eq!(
            serde_json::to_vec(&left).expect("serialize left"),
            serde_json::to_vec(&right).expect("serialize right")
        );
    }

    #[test]
    fn only_settled_run_states_are_terminal() {
        for state in [
            RunStatus::Draft,
            RunStatus::AwaitingApproval,
            RunStatus::Ready,
            RunStatus::Running,
            RunStatus::Paused,
            RunStatus::Cancelling,
        ] {
            assert!(!state.is_terminal());
        }
        for state in [
            RunStatus::Succeeded,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ] {
            assert!(state.is_terminal());
        }
    }
}
