use crate::store::{
    BudgetConservationReport, BudgetExecutionFailpoint, BudgetGrantOutcome,
    BudgetReconciliationOutcome, OrchestratedAdmissionTransition, Store, StoreError,
    StoredBudgetReservation, execution_ledger_utc_date, supplemental_budget_action_digest,
};
use pandamux_core::{
    AgentId, Attempt, BudgetAmount, BudgetGrantId, BudgetReservation, BudgetReservationId,
    BudgetReservationState, BudgetUsage, OrganizationId, Run, RunId, RunStatus, RunTaskId,
    SupplementalBudgetGrant,
};
use std::collections::BTreeMap;

#[derive(Debug)]
pub enum ExecutionBudgetError {
    Store(StoreError),
    Worker(String),
}

impl std::fmt::Display for ExecutionBudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::Worker(error) => write!(f, "execution budget worker failed: {error}"),
        }
    }
}

impl std::error::Error for ExecutionBudgetError {}

impl From<StoreError> for ExecutionBudgetError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanExecutionBudgetRequest {
    pub run_id: RunId,
    pub attempt: Attempt,
    pub agent_id: AgentId,
    pub organization_id: OrganizationId,
    pub agent_budget_limit: BudgetAmount,
    pub trusted_estimate: Option<BudgetAmount>,
    pub daily_budget_limit_micro_usd: u64,
    pub claimed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantExecutionBudgetRequest {
    pub grant: SupplementalBudgetGrant,
    pub granted_at_ms: u64,
    pub local_control_authenticated: bool,
    pub failpoint: Option<BudgetExecutionFailpoint>,
}

#[derive(Clone)]
pub struct ExecutionBudgetService {
    store: Store,
}

impl ExecutionBudgetService {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Builds the exact attempt-bound reservation consumed by the shared
    /// execution-admission transaction. This check is deliberately read-only;
    /// the claim transaction repeats every authoritative ceiling check.
    pub async fn plan_admission(
        &self,
        request: PlanExecutionBudgetRequest,
    ) -> Result<OrchestratedAdmissionTransition, ExecutionBudgetError> {
        let store = self.store.clone();
        run_blocking(move || {
            let record = store
                .load_verified_orchestration_run(&request.run_id)?
                .ok_or_else(|| StoreError::Data("orchestration run was not found".to_string()))?;
            let runtime = store.load_orchestration_runtime_records(&request.run_id)?;
            plan_admission(&record.run, &runtime.budget_reservations, request)
        })
        .await
    }

    pub async fn grant(
        &self,
        request: GrantExecutionBudgetRequest,
    ) -> Result<BudgetGrantOutcome, ExecutionBudgetError> {
        let store = self.store.clone();
        run_blocking(move || {
            store.grant_orchestration_budget(
                &request.grant,
                request.granted_at_ms,
                request.local_control_authenticated,
                request.failpoint,
            )
        })
        .await
    }

    pub async fn reconcile(
        &self,
        run_id: RunId,
        reservation_id: BudgetReservationId,
        reconciled_at_ms: u64,
        failpoint: Option<BudgetExecutionFailpoint>,
    ) -> Result<BudgetReconciliationOutcome, ExecutionBudgetError> {
        let store = self.store.clone();
        run_blocking(move || {
            store.reconcile_orchestration_budget(
                &run_id,
                &reservation_id,
                reconciled_at_ms,
                failpoint,
            )
        })
        .await
    }

    pub async fn audit_conservation(
        &self,
        run_id: RunId,
    ) -> Result<BudgetConservationReport, ExecutionBudgetError> {
        let store = self.store.clone();
        run_blocking(move || store.audit_budget_conservation(&run_id)).await
    }
}

pub fn bind_supplemental_budget_grant_digest(
    mut grant: SupplementalBudgetGrant,
) -> SupplementalBudgetGrant {
    grant.action_digest = supplemental_budget_action_digest(&grant);
    grant
}

fn plan_admission(
    run: &Run,
    stored_reservations: &[StoredBudgetReservation],
    request: PlanExecutionBudgetRequest,
) -> Result<OrchestratedAdmissionTransition, StoreError> {
    if request.run_id != run.id
        || request.organization_id != run.organization_id
        || request.attempt.task_id.as_str().trim().is_empty()
        || request.attempt.id.as_str().trim().is_empty()
        || request.daily_budget_limit_micro_usd > i64::MAX as u64
        || amount_exceeds_sqlite(request.agent_budget_limit)
        || request.trusted_estimate.is_some_and(amount_exceeds_sqlite)
    {
        return Err(StoreError::Data(
            "execution budget plan identity or integer range is malformed".to_string(),
        ));
    }
    let task = run
        .tasks
        .get(&request.attempt.task_id)
        .ok_or_else(|| StoreError::Data("execution budget task was not found".to_string()))?;
    if task.route.as_ref() != Some(&request.attempt.route)
        || request.attempt.route.selected_agent_id.as_ref() != Some(&request.agent_id)
        || task.selected_agent_id.as_ref() != Some(&request.agent_id)
        || task.status != pandamux_core::RunTaskStatus::Ready
        || !matches!(run.status, RunStatus::Ready | RunStatus::Running)
        || run.budget.incomplete.tokens
        || run.budget.incomplete.cost_micro_usd
        || run.budget.incomplete.runtime_ms
        || amount_nonzero(run.budget.overage)
        || run.attempts.contains_key(&request.attempt.id)
    {
        return Err(StoreError::Data(
            "execution budget plan differs from the pinned task route".to_string(),
        ));
    }

    let task_committed = scope_committed(run, Some(&request.attempt.task_id))?;
    let run_committed = scope_committed(run, None)?;
    let task_remaining = checked_sub_amount(task.budget_limit, task_committed, "task")?;
    let run_remaining = checked_sub_amount(run.budget.limit, run_committed, "run")?;
    let active_grants = run
        .budget_grants
        .iter()
        .filter(|(_, grant)| {
            grant.attempt_id == request.attempt.id && grant.expires_at_ms > request.claimed_at_ms
        })
        .map(|(id, grant)| (id.clone(), grant.clone()))
        .collect::<BTreeMap<BudgetGrantId, SupplementalBudgetGrant>>();
    let grant_amount = grant_amount(&active_grants)?;
    let task_ceiling = checked_add_amount(task_remaining, grant_amount, "task grant")?;
    let run_ceiling = checked_add_amount(run_remaining, grant_amount, "run grant")?;
    let amount = request.trusted_estimate.unwrap_or(task_ceiling);
    if !amount_within(amount, task_ceiling) || !amount_within(amount, run_ceiling) {
        return Err(StoreError::Data(
            "execution budget estimate exceeds the remaining task or run ceiling".to_string(),
        ));
    }

    let mut agent_committed = BudgetAmount::default();
    for record in stored_reservations
        .iter()
        .filter(|record| record.agent_id == request.agent_id)
    {
        if record.agent_budget_limit != request.agent_budget_limit {
            return Err(StoreError::Data(
                "compiled agent budget changed during feasibility planning".to_string(),
            ));
        }
        agent_committed = checked_add_amount(
            agent_committed,
            stored_committed_after_grants(record)?,
            "agent committed",
        )?;
    }
    let incoming_agent = saturating_sub_amount(amount, grant_amount);
    agent_committed = checked_add_amount(agent_committed, incoming_agent, "agent incoming")?;
    if !amount_within(agent_committed, request.agent_budget_limit) {
        return Err(StoreError::Data(
            "execution budget estimate exceeds the compiled agent ceiling".to_string(),
        ));
    }

    let reservation = BudgetReservation {
        id: BudgetReservationId::new(format!("budget-{}", request.attempt.id.as_str())),
        task_id: request.attempt.task_id.clone(),
        attempt_id: request.attempt.id.clone(),
        amount,
        actual: BudgetUsage::default(),
        ledger_utc_date: execution_ledger_utc_date(request.claimed_at_ms)?,
        state: BudgetReservationState::Reserved,
    };
    Ok(OrchestratedAdmissionTransition {
        reservation: StoredBudgetReservation {
            run_id: request.run_id,
            agent_id: request.agent_id,
            organization_id: request.organization_id,
            agent_budget_limit: request.agent_budget_limit,
            reservation,
            grants: active_grants,
            reconciliation: None,
            created_at_ms: request.claimed_at_ms,
            updated_at_ms: request.claimed_at_ms,
        },
        attempt: request.attempt,
        daily_budget_limit_micro_usd: request.daily_budget_limit_micro_usd,
    })
}

fn scope_committed(run: &Run, task_id: Option<&RunTaskId>) -> Result<BudgetAmount, StoreError> {
    run.reservations
        .values()
        .filter(|reservation| task_id.is_none_or(|task_id| reservation.task_id == *task_id))
        .try_fold(BudgetAmount::default(), |total, reservation| {
            let retained = matches!(
                reservation.state,
                BudgetReservationState::Reserved | BudgetReservationState::PartiallyReconciled
            );
            let committed = BudgetAmount {
                tokens: reservation.actual.tokens.unwrap_or(if retained {
                    reservation.amount.tokens
                } else {
                    0
                }),
                cost_micro_usd: reservation.actual.cost_micro_usd.unwrap_or(if retained {
                    reservation.amount.cost_micro_usd
                } else {
                    0
                }),
                runtime_ms: reservation.actual.runtime_ms.unwrap_or(if retained {
                    reservation.amount.runtime_ms
                } else {
                    0
                }),
            };
            let grants = run
                .budget_grants
                .values()
                .filter(|grant| grant.attempt_id == reservation.attempt_id)
                .try_fold(BudgetAmount::default(), |total, grant| {
                    add_grant(total, grant)
                })?;
            checked_add_amount(
                total,
                saturating_sub_amount(committed, grants),
                "scope committed",
            )
        })
}

fn stored_committed_after_grants(
    record: &StoredBudgetReservation,
) -> Result<BudgetAmount, StoreError> {
    let retained = matches!(
        record.reservation.state,
        BudgetReservationState::Reserved | BudgetReservationState::PartiallyReconciled
    );
    let committed = BudgetAmount {
        tokens: record.reservation.actual.tokens.unwrap_or(if retained {
            record.reservation.amount.tokens
        } else {
            0
        }),
        cost_micro_usd: record
            .reservation
            .actual
            .cost_micro_usd
            .unwrap_or(if retained {
                record.reservation.amount.cost_micro_usd
            } else {
                0
            }),
        runtime_ms: record.reservation.actual.runtime_ms.unwrap_or(if retained {
            record.reservation.amount.runtime_ms
        } else {
            0
        }),
    };
    Ok(saturating_sub_amount(
        committed,
        grant_amount(&record.grants)?,
    ))
}

fn grant_amount(
    grants: &BTreeMap<BudgetGrantId, SupplementalBudgetGrant>,
) -> Result<BudgetAmount, StoreError> {
    grants.values().try_fold(BudgetAmount::default(), add_grant)
}

fn add_grant(
    mut amount: BudgetAmount,
    grant: &SupplementalBudgetGrant,
) -> Result<BudgetAmount, StoreError> {
    let target = match grant.dimension {
        pandamux_core::BudgetDimension::Tokens => &mut amount.tokens,
        pandamux_core::BudgetDimension::CostMicroUsd => &mut amount.cost_micro_usd,
        pandamux_core::BudgetDimension::RuntimeMs => &mut amount.runtime_ms,
    };
    *target = target
        .checked_add(grant.amount)
        .ok_or_else(|| StoreError::Data("supplemental budget grant overflow".to_string()))?;
    Ok(amount)
}

fn checked_add_amount(
    left: BudgetAmount,
    right: BudgetAmount,
    scope: &str,
) -> Result<BudgetAmount, StoreError> {
    Ok(BudgetAmount {
        tokens: left
            .tokens
            .checked_add(right.tokens)
            .ok_or_else(|| StoreError::Data(format!("{scope} budget token total overflow")))?,
        cost_micro_usd: left
            .cost_micro_usd
            .checked_add(right.cost_micro_usd)
            .ok_or_else(|| StoreError::Data(format!("{scope} budget cost total overflow")))?,
        runtime_ms: left
            .runtime_ms
            .checked_add(right.runtime_ms)
            .ok_or_else(|| StoreError::Data(format!("{scope} budget runtime total overflow")))?,
    })
}

fn checked_sub_amount(
    left: BudgetAmount,
    right: BudgetAmount,
    scope: &str,
) -> Result<BudgetAmount, StoreError> {
    Ok(BudgetAmount {
        tokens: left.tokens.checked_sub(right.tokens).ok_or_else(|| {
            StoreError::Data(format!("{scope} budget token total is already over limit"))
        })?,
        cost_micro_usd: left
            .cost_micro_usd
            .checked_sub(right.cost_micro_usd)
            .ok_or_else(|| {
                StoreError::Data(format!("{scope} budget cost total is already over limit"))
            })?,
        runtime_ms: left
            .runtime_ms
            .checked_sub(right.runtime_ms)
            .ok_or_else(|| {
                StoreError::Data(format!(
                    "{scope} budget runtime total is already over limit"
                ))
            })?,
    })
}

fn saturating_sub_amount(left: BudgetAmount, right: BudgetAmount) -> BudgetAmount {
    BudgetAmount {
        tokens: left.tokens.saturating_sub(right.tokens),
        cost_micro_usd: left.cost_micro_usd.saturating_sub(right.cost_micro_usd),
        runtime_ms: left.runtime_ms.saturating_sub(right.runtime_ms),
    }
}

fn amount_within(value: BudgetAmount, ceiling: BudgetAmount) -> bool {
    value.tokens <= ceiling.tokens
        && value.cost_micro_usd <= ceiling.cost_micro_usd
        && value.runtime_ms <= ceiling.runtime_ms
}

fn amount_nonzero(amount: BudgetAmount) -> bool {
    amount.tokens != 0 || amount.cost_micro_usd != 0 || amount.runtime_ms != 0
}

fn amount_exceeds_sqlite(amount: BudgetAmount) -> bool {
    amount.tokens > i64::MAX as u64
        || amount.cost_micro_usd > i64::MAX as u64
        || amount.runtime_ms > i64::MAX as u64
}

async fn run_blocking<T, F>(operation: F) -> Result<T, ExecutionBudgetError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, StoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| ExecutionBudgetError::Worker(error.to_string()))?
        .map_err(ExecutionBudgetError::Store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{
        AccessMode, AgentTask, AttemptBoundary, AttemptStatus, CleanupState, OrchestrationMode,
        OutputContract, ProjectId, ProviderInstanceId, RoutingDecision, RunCommand, RunEvent,
        RunEventId, RunSeed, RunTaskStatus, RunTrigger, SubscriptionKey, TaskRouteDecision,
        ThreadId, TurnAttemptId, ValidatedPlan,
    };
    use std::collections::BTreeSet;

    const AT_MS: u64 = 86_399_999;

    fn amount(tokens: u64, cost_micro_usd: u64, runtime_ms: u64) -> BudgetAmount {
        BudgetAmount {
            tokens,
            cost_micro_usd,
            runtime_ms,
        }
    }

    fn route() -> TaskRouteDecision {
        TaskRouteDecision {
            task_id: RunTaskId::new("task-budget"),
            selected_agent_id: Some(AgentId::new("agent-budget")),
            agent_evaluations: Vec::new(),
            provider: RoutingDecision {
                selected_provider_instance_id: Some(ProviderInstanceId::new("provider-budget")),
                selected_subscription_key: Some(SubscriptionKey::new("subscription-budget")),
                selected_model: Some("model-budget".to_string()),
                evaluations: Vec::new(),
            },
        }
    }

    fn apply(run: &mut Run, command: RunCommand, at_ms: u64) {
        let kinds = pandamux_orchestrator::decide(run, command).expect("legal budget fixture");
        for kind in kinds {
            let seq = run.last_seq + 1;
            let event = RunEvent {
                event_id: RunEventId::new(format!("budget-event-{seq}")),
                run_id: run.id.clone(),
                seq,
                at_ms,
                idempotency_key: format!("budget-event:{seq}"),
                kind,
            };
            *run = pandamux_orchestrator::apply(run, &event)
                .expect("apply budget fixture")
                .run;
        }
    }

    fn ready_run() -> Run {
        let task = AgentTask {
            id: RunTaskId::new("task-budget"),
            summary: "Budget fixture".to_string(),
            class: "build".to_string(),
            project_id: ProjectId::new("project-budget"),
            organization_id: OrganizationId::new("org-budget"),
            required_capabilities: BTreeSet::new(),
            access_ceiling: AccessMode::WorkspaceOnly,
            inputs: Vec::new(),
            outputs: vec![OutputContract {
                kind: "change_set".to_string(),
                schema: None,
            }],
            required_verification: BTreeSet::new(),
            selected_agent_id: None,
            route: None,
            status: RunTaskStatus::Ready,
            max_attempts: 2,
            approval_ids: BTreeSet::new(),
            budget_limit: amount(100, 200, 300),
            budget_reservation_id: None,
        };
        let seed = RunSeed {
            trigger: RunTrigger::Manual {
                client_id: "budget-client".to_string(),
            },
            policy_revision: "budget-policy".to_string(),
            bundle_sha: "a".repeat(64),
            organization_id: OrganizationId::new("org-budget"),
            mode: OrchestrationMode::Auto,
            budget_limit: amount(1_000, 2_000, 3_000),
            max_parallel_tasks: 1,
        };
        let plan = ValidatedPlan {
            schema_version: 1,
            source_digest: "b".repeat(64),
            topological_order: vec![task.id.clone()],
            tasks: BTreeMap::from([(task.id.clone(), task)]),
            edges: BTreeMap::new(),
            total_budget: amount(100, 200, 300),
        };
        let mut run = Run::draft(RunId::new("run-budget"), seed.clone(), AT_MS - 10);
        apply(&mut run, RunCommand::AcceptPlan { seed, plan }, AT_MS - 9);
        apply(
            &mut run,
            RunCommand::SelectRoute { decision: route() },
            AT_MS - 8,
        );
        run
    }

    fn attempt(id: &str, ordinal: u32) -> Attempt {
        Attempt {
            id: pandamux_core::RunAttemptId::new(id),
            task_id: RunTaskId::new("task-budget"),
            ordinal,
            route: route(),
            thread_id: None,
            turn_attempt_id: None,
            status: AttemptStatus::DispatchRequested,
            boundary: AttemptBoundary::SafeToRetry,
            started_at_ms: AT_MS,
            ended_at_ms: None,
            failure: None,
            usage_reconciled: false,
            ambiguous_effect: false,
            ambiguity_reason: None,
            worktree_id: None,
            checkpoint_ids: BTreeSet::new(),
            cleanup_state: CleanupState::NotStarted,
        }
    }

    fn request(
        attempt: Attempt,
        trusted_estimate: Option<BudgetAmount>,
    ) -> PlanExecutionBudgetRequest {
        PlanExecutionBudgetRequest {
            run_id: RunId::new("run-budget"),
            attempt,
            agent_id: AgentId::new("agent-budget"),
            organization_id: OrganizationId::new("org-budget"),
            agent_budget_limit: amount(1_000, 2_000, 3_000),
            trusted_estimate,
            daily_budget_limit_micro_usd: 10_000,
            claimed_at_ms: AT_MS,
        }
    }

    #[test]
    fn grant_digest_changes_with_every_bound_numeric_field() {
        let grant = SupplementalBudgetGrant {
            id: BudgetGrantId::new("grant-one"),
            run_id: RunId::new("run-one"),
            task_id: RunTaskId::new("task-one"),
            attempt_id: pandamux_core::RunAttemptId::new("attempt-one"),
            approval_id: pandamux_core::ApprovalId::new("approval-one"),
            dimension: pandamux_core::BudgetDimension::Tokens,
            amount: 10,
            policy_revision: "policy-one".to_string(),
            action_digest: String::new(),
            expires_at_ms: 100,
        };
        let bound = bind_supplemental_budget_grant_digest(grant.clone());
        assert_eq!(
            bound.action_digest,
            supplemental_budget_action_digest(&bound)
        );

        let mut changed = bound.clone();
        changed.amount += 1;
        assert_ne!(
            bound.action_digest,
            supplemental_budget_action_digest(&changed)
        );
        let mut changed = bound.clone();
        changed.expires_at_ms += 1;
        assert_ne!(
            bound.action_digest,
            supplemental_budget_action_digest(&changed)
        );
    }

    #[test]
    fn bud_01_and_02_plan_checked_task_and_run_reservations() {
        let run = ready_run();
        let default = plan_admission(&run, &[], request(attempt("attempt-one", 1), None))
            .expect("default task-ceiling reservation");
        assert_eq!(
            default.reservation.reservation.amount,
            amount(100, 200, 300)
        );
        assert_eq!(
            default.reservation.reservation.ledger_utc_date,
            "1970-01-01"
        );

        let trusted = amount(40, 50, 60);
        let smaller = plan_admission(&run, &[], request(attempt("attempt-one", 1), Some(trusted)))
            .expect("trusted smaller reservation");
        assert_eq!(smaller.reservation.reservation.amount, trusted);

        let error = plan_admission(
            &run,
            &[],
            request(attempt("attempt-one", 1), Some(amount(101, 200, 300))),
        )
        .expect_err("task ceiling must reject oversized estimate");
        assert!(error.to_string().contains("remaining task or run ceiling"));

        let error = plan_admission(
            &run,
            &[],
            request(
                attempt("attempt-one", 1),
                Some(amount(i64::MAX as u64 + 1, 1, 1)),
            ),
        )
        .expect_err("SQLite integer overflow must reject");
        assert!(error.to_string().contains("integer range"));
    }

    #[test]
    fn bud_06_retry_reserves_only_the_unreconciled_remainder() {
        let mut run = ready_run();
        let first = attempt("attempt-one", 1);
        apply(
            &mut run,
            RunCommand::ReserveBudget {
                reservation: BudgetReservation {
                    id: BudgetReservationId::new("budget-attempt-one"),
                    task_id: first.task_id.clone(),
                    attempt_id: first.id.clone(),
                    amount: amount(100, 200, 300),
                    actual: BudgetUsage::default(),
                    ledger_utc_date: "1970-01-01".to_string(),
                    state: BudgetReservationState::Reserved,
                },
            },
            AT_MS,
        );
        apply(
            &mut run,
            RunCommand::RequestDispatch {
                attempt: first.clone(),
            },
            AT_MS + 1,
        );
        apply(
            &mut run,
            RunCommand::StartAttempt {
                attempt_id: first.id.clone(),
                thread_id: ThreadId::new("thread-one"),
                turn_attempt_id: TurnAttemptId::new("turn-attempt-one"),
            },
            AT_MS + 2,
        );
        apply(
            &mut run,
            RunCommand::SettleAttempt {
                attempt_id: first.id,
                status: AttemptStatus::Failed,
                failure: Some(pandamux_core::AttemptFailure {
                    class: pandamux_core::FailureClass::Provider,
                    code: "fixture_failure".to_string(),
                    safe_message: "fixture failure".to_string(),
                }),
                usage_reconciled: false,
            },
            AT_MS + 3,
        );
        apply(
            &mut run,
            RunCommand::ReconcileBudget {
                reservation_id: BudgetReservationId::new("budget-attempt-one"),
                actual: BudgetUsage {
                    tokens: Some(40),
                    cost_micro_usd: Some(50),
                    runtime_ms: Some(60),
                },
            },
            AT_MS + 4,
        );
        let retry = plan_admission(&run, &[], request(attempt("attempt-two", 2), None))
            .expect("retry remainder plan");
        assert_eq!(retry.reservation.reservation.amount, amount(60, 150, 240));
    }

    #[test]
    fn bud_07_utc_date_conversion_has_exact_midnight_boundary() {
        assert_eq!(execution_ledger_utc_date(86_399_999).unwrap(), "1970-01-01");
        assert_eq!(execution_ledger_utc_date(86_400_000).unwrap(), "1970-01-02");
        assert_eq!(
            execution_ledger_utc_date(1_782_864_000_000).unwrap(),
            "2026-07-01"
        );
    }
}
