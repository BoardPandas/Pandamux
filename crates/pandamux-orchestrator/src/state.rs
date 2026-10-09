use crate::dag::is_task_ready;
use crate::diagnostics::SafeText;
use crate::routing::{FallbackEligibility, fallback_eligibility};
use pandamux_core::{
    Attempt, AttemptBoundary, AttemptStatus, BlackboardEntry, BudgetAmount, BudgetReservationState,
    DependencyRequirement, OrchestrationEffect, OrchestrationEffectId, OrchestrationEffectKind,
    Run, RunApprovalKind, RunApprovalStatus, RunCommand, RunEvent, RunEventId, RunEventKind,
    RunStatus, RunTaskId, RunTaskStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationErrorCode {
    IllegalTransition,
    RunIdentityMismatch,
    EventGap,
    ReplayConflict,
    IdempotencyConflict,
    TerminalRun,
    MissingRecord,
    IdentityCollision,
    BudgetOverflow,
    BudgetExceeded,
    HashMismatch,
    InvalidEvent,
    EmptyReplay,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationError {
    pub code: OrchestrationErrorCode,
    pub explanation: SafeText,
}

impl std::fmt::Display for OrchestrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.explanation.as_str())
    }
}

impl std::error::Error for OrchestrationError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub run: Run,
    pub new_effects: Vec<OrchestrationEffect>,
    pub duplicate: bool,
}

pub fn decide(run: &Run, command: RunCommand) -> Result<Vec<RunEventKind>, OrchestrationError> {
    if run.is_terminal() {
        return Err(orchestration_error(
            OrchestrationErrorCode::TerminalRun,
            "A terminal run absorbs every new command",
        ));
    }
    let event = match command {
        RunCommand::AcceptPlan { seed, plan } => {
            if run.status != RunStatus::Draft || run.validated_plan.is_some() {
                return Err(illegal("A plan can be accepted only once for a draft run"));
            }
            RunEventKind::PlanAccepted { seed, plan }
        }
        RunCommand::RequestApproval { approval } => {
            if run.approvals.contains_key(&approval.id) {
                return Err(orchestration_error(
                    OrchestrationErrorCode::IdentityCollision,
                    "The approval id is already present in the run",
                ));
            }
            RunEventKind::ApprovalRequested { approval }
        }
        RunCommand::ResolveApproval {
            approval_id,
            action_digest,
            status,
            provenance,
        } => {
            if !run.approvals.contains_key(&approval_id) {
                return Ok(Vec::new());
            }
            RunEventKind::ApprovalResolved {
                approval_id,
                action_digest,
                status,
                provenance,
            }
        }
        RunCommand::ReserveBudget { reservation } => {
            if run.reservations.contains_key(&reservation.id) {
                return Err(orchestration_error(
                    OrchestrationErrorCode::IdentityCollision,
                    "The budget reservation id is already present in the run",
                ));
            }
            RunEventKind::BudgetReserved { reservation }
        }
        RunCommand::ReconcileBudget {
            reservation_id,
            actual,
        } => RunEventKind::BudgetReconciled {
            reservation_id,
            actual,
        },
        RunCommand::ReleaseBudget { reservation_id } => {
            RunEventKind::BudgetReleased { reservation_id }
        }
        RunCommand::SelectRoute { decision } => {
            if let Some(task) = run.tasks.get(&decision.task_id)
                && let Some(recorded) = &task.route
            {
                if recorded == &decision {
                    return Ok(Vec::new());
                }
                return Err(orchestration_error(
                    OrchestrationErrorCode::ReplayConflict,
                    "A recorded route is immutable",
                ));
            }
            RunEventKind::RouteSelected { decision }
        }
        RunCommand::RequestDispatch { attempt } => RunEventKind::DispatchRequested { attempt },
        RunCommand::StartAttempt {
            attempt_id,
            thread_id,
            turn_attempt_id,
        } => RunEventKind::AttemptStarted {
            attempt_id,
            thread_id,
            turn_attempt_id,
        },
        RunCommand::AdvanceAttemptBoundary {
            attempt_id,
            boundary,
        } => RunEventKind::AttemptBoundaryAdvanced {
            attempt_id,
            boundary,
        },
        RunCommand::SettleAttempt {
            attempt_id,
            status,
            failure,
            usage_reconciled,
        } => RunEventKind::AttemptSettled {
            attempt_id,
            status,
            failure,
            usage_reconciled,
        },
        RunCommand::PublishBlackboard { entry } => RunEventKind::BlackboardPublished { entry },
        RunCommand::SettleTask {
            task_id,
            status,
            summary,
        } => RunEventKind::TaskSettled {
            task_id,
            status,
            summary,
        },
        RunCommand::Cancel => {
            let task_ids = run
                .tasks
                .values()
                .filter(|task| !task.status.is_terminal())
                .map(|task| task.id.clone())
                .collect();
            RunEventKind::CancellationRequested { task_ids }
        }
        RunCommand::Pause { reason } => {
            if reason.trim().is_empty() {
                return Err(invalid("A pause requires a safe nonempty reason"));
            }
            RunEventKind::Paused { reason }
        }
        RunCommand::Resume => RunEventKind::Resumed,
        RunCommand::AcknowledgeEffect { effect_id, receipt } => {
            if let Some(existing) = run.settled_effects.get(&effect_id) {
                if existing == &receipt {
                    return Ok(Vec::new());
                }
                return Err(orchestration_error(
                    OrchestrationErrorCode::ReplayConflict,
                    "An acknowledged effect receipt cannot change",
                ));
            }
            RunEventKind::EffectAcknowledged { effect_id, receipt }
        }
        RunCommand::SettleRun { status, outcome } => {
            if !status.is_terminal() || outcome.trim().is_empty() {
                return Err(invalid(
                    "Run settlement requires a terminal status and nonempty outcome",
                ));
            }
            RunEventKind::RunSettled { status, outcome }
        }
    };
    validate_kind(run, &event, None)?;
    Ok(vec![event])
}

pub fn apply(run: &Run, event: &RunEvent) -> Result<ApplyOutcome, OrchestrationError> {
    if event.run_id != run.id {
        return Err(orchestration_error(
            OrchestrationErrorCode::RunIdentityMismatch,
            "The event run id does not match the target run",
        ));
    }
    if event.event_id.as_str().trim().is_empty() || event.idempotency_key.trim().is_empty() {
        return Err(invalid(
            "The event id and idempotency key must both be nonempty",
        ));
    }
    let digest = event_digest(event);
    if let Some(existing) = run.replay_receipts.get(&event.event_id) {
        if existing == &digest
            && run.idempotency_receipts.get(&event.idempotency_key) == Some(&event.event_id)
        {
            return Ok(ApplyOutcome {
                run: run.clone(),
                new_effects: Vec::new(),
                duplicate: true,
            });
        }
        return Err(orchestration_error(
            OrchestrationErrorCode::ReplayConflict,
            "An event id was reused with different canonical bytes",
        ));
    }
    if run
        .idempotency_receipts
        .contains_key(&event.idempotency_key)
    {
        return Err(orchestration_error(
            OrchestrationErrorCode::IdempotencyConflict,
            "An idempotency key was reused by a different event",
        ));
    }
    if run.is_terminal() {
        return Err(orchestration_error(
            OrchestrationErrorCode::TerminalRun,
            "A terminal run absorbs every non-identical event",
        ));
    }
    let expected_seq = run.last_seq.checked_add(1).ok_or_else(|| {
        orchestration_error(
            OrchestrationErrorCode::EventGap,
            "The event sequence cannot advance past u64 maximum",
        )
    })?;
    if event.seq != expected_seq {
        return Err(orchestration_error(
            if event.seq <= run.last_seq {
                OrchestrationErrorCode::ReplayConflict
            } else {
                OrchestrationErrorCode::EventGap
            },
            "The event sequence is not the exact next sequence",
        ));
    }
    if event.at_ms < run.updated_at_ms {
        return Err(invalid(
            "The event timestamp cannot precede the current run timestamp",
        ));
    }
    if let RunEventKind::BlackboardPublished { entry } = &event.kind
        && entry.created_event_id != event.event_id
    {
        return Err(invalid(
            "The blackboard entry must identify its durable creation event",
        ));
    }

    validate_kind(run, &event.kind, Some(event.at_ms))?;
    let mut next = run.clone();
    let effect_kinds = apply_kind(&mut next, event)?;
    next.last_seq = event.seq;
    next.updated_at_ms = event.at_ms;
    next.replay_receipts.insert(event.event_id.clone(), digest);
    next.idempotency_receipts
        .insert(event.idempotency_key.clone(), event.event_id.clone());

    let mut new_effects = Vec::new();
    for (index, kind) in effect_kinds.into_iter().enumerate() {
        let effect = OrchestrationEffect {
            id: derived_effect_id(&next.id, &event.event_id, index, &kind),
            source_event_id: event.event_id.clone(),
            kind,
        };
        if next.settled_effects.contains_key(&effect.id) {
            return Err(orchestration_error(
                OrchestrationErrorCode::IdentityCollision,
                "A derived effect id collides with a settled effect",
            ));
        }
        match next.pending_effects.get(&effect.id) {
            Some(existing) if existing == &effect => {}
            Some(_) => {
                return Err(orchestration_error(
                    OrchestrationErrorCode::IdentityCollision,
                    "A derived effect id collides with different pending work",
                ));
            }
            None => {
                next.pending_effects
                    .insert(effect.id.clone(), effect.clone());
                new_effects.push(effect);
            }
        }
    }
    Ok(ApplyOutcome {
        run: next,
        new_effects,
        duplicate: false,
    })
}

pub fn replay<'a>(
    events: impl IntoIterator<Item = &'a RunEvent>,
) -> Result<Run, OrchestrationError> {
    let mut events = events.into_iter();
    let first = events.next().ok_or_else(|| {
        orchestration_error(
            OrchestrationErrorCode::EmptyReplay,
            "Replay requires a first PlanAccepted event",
        )
    })?;
    let RunEventKind::PlanAccepted { seed, .. } = &first.kind else {
        return Err(invalid("The first replay event must accept the plan"));
    };
    let initial = Run::draft(first.run_id.clone(), seed.clone(), first.at_ms);
    replay_from(&initial, std::iter::once(first).chain(events))
}

pub fn replay_from<'a>(
    snapshot: &Run,
    events: impl IntoIterator<Item = &'a RunEvent>,
) -> Result<Run, OrchestrationError> {
    let mut run = snapshot.clone();
    for event in events {
        run = apply(&run, event)?.run;
    }
    Ok(run)
}

pub fn canonical_run_bytes(run: &Run) -> Vec<u8> {
    canonical_json_bytes(run)
}

pub fn canonical_blackboard_sha256(content: &Value) -> String {
    sha256_hex(&serde_json::to_vec(&canonicalize_json(content)).expect("JSON is serializable"))
}

fn validate_kind(
    run: &Run,
    kind: &RunEventKind,
    at_ms: Option<u64>,
) -> Result<(), OrchestrationError> {
    match kind {
        RunEventKind::PlanAccepted { seed, plan } => {
            if run.status != RunStatus::Draft || run.validated_plan.is_some() {
                return Err(illegal("A plan can be accepted only for a draft run"));
            }
            if run.trigger != seed.trigger
                || run.policy_revision != seed.policy_revision
                || run.bundle_sha != seed.bundle_sha
                || run.organization_id != seed.organization_id
                || run.mode != seed.mode
                || run.budget.limit != seed.budget_limit
                || run.max_parallel_tasks != seed.max_parallel_tasks
                || plan
                    .tasks
                    .values()
                    .any(|task| task.organization_id != run.organization_id)
            {
                return Err(invalid(
                    "The accepted plan or seed does not match the draft run identity and policy",
                ));
            }
        }
        RunEventKind::ApprovalRequested { approval } => {
            let task = run
                .tasks
                .get(&approval.task_id)
                .ok_or_else(|| missing("The approval task does not exist"))?;
            if approval.run_id != run.id
                || approval.policy_revision != run.policy_revision
                || run.approvals.contains_key(&approval.id)
                || approval.status != RunApprovalStatus::Pending
                || approval.decision.is_some()
                || !is_sha256(&approval.action_digest)
                || at_ms.is_some_and(|at| approval.expires_at_ms <= at)
                || matches!(
                    task.status,
                    RunTaskStatus::DispatchRequested
                        | RunTaskStatus::Running
                        | RunTaskStatus::Succeeded
                        | RunTaskStatus::Failed
                        | RunTaskStatus::Skipped
                        | RunTaskStatus::Cancelled
                )
                || run.approvals.values().any(|existing| {
                    existing.run_id == approval.run_id
                        && existing.task_id == approval.task_id
                        && existing.kind == approval.kind
                        && existing.policy_revision == approval.policy_revision
                        && existing.action_digest == approval.action_digest
                })
            {
                return Err(invalid(
                    "The approval request is stale, malformed, or colliding",
                ));
            }
        }
        RunEventKind::ApprovalResolved {
            status, provenance, ..
        } => {
            if *status == RunApprovalStatus::Pending
                || provenance.actor.trim().is_empty()
                || provenance.decision_id.trim().is_empty()
            {
                return Err(invalid("The approval resolution is malformed"));
            }
        }
        RunEventKind::BudgetReserved { reservation } => {
            let Some(task) = run.tasks.get(&reservation.task_id) else {
                return Err(missing("The budget reservation task does not exist"));
            };
            if run.reservations.contains_key(&reservation.id)
                || reservation.state != BudgetReservationState::Reserved
                || reservation.attempt_id.is_some()
                || task.budget_reservation_id.is_some()
                || !amount_within(reservation.amount, task.budget_limit)
            {
                return Err(invalid("The budget reservation is malformed or colliding"));
            }
            checked_budget_after_reserve(run, reservation.amount)?;
        }
        RunEventKind::BudgetReconciled {
            reservation_id,
            actual,
        } => {
            let reservation = run
                .reservations
                .get(reservation_id)
                .ok_or_else(|| missing("The budget reservation does not exist"))?;
            if reservation.state != BudgetReservationState::Reserved {
                return Err(illegal("Only a reserved budget can be reconciled"));
            }
            checked_budget_after_reconcile(run, reservation.amount, *actual)?;
        }
        RunEventKind::BudgetReleased { reservation_id } => {
            let reservation = run
                .reservations
                .get(reservation_id)
                .ok_or_else(|| missing("The budget reservation does not exist"))?;
            if reservation.state != BudgetReservationState::Reserved {
                return Err(illegal("Only a reserved budget can be released"));
            }
        }
        RunEventKind::RouteSelected { decision } => {
            let task = run
                .tasks
                .get(&decision.task_id)
                .ok_or_else(|| missing("The routed task does not exist"))?;
            if task.route.is_some()
                || run.routes.contains_key(&decision.task_id)
                || task.status != RunTaskStatus::Ready
                || decision.selected_agent_id.is_none()
                || decision.provider.selected_provider_instance_id.is_none()
            {
                return Err(illegal("The task cannot record this route"));
            }
        }
        RunEventKind::DispatchRequested { attempt } => validate_dispatch(run, attempt, at_ms)?,
        RunEventKind::AttemptStarted { attempt_id, .. } => {
            let attempt = run
                .attempts
                .get(attempt_id)
                .ok_or_else(|| missing("The attempt does not exist"))?;
            if attempt.status != AttemptStatus::DispatchRequested
                || attempt.thread_id.is_some()
                || attempt.turn_attempt_id.is_some()
            {
                return Err(illegal("Only a newly dispatched attempt can start"));
            }
        }
        RunEventKind::AttemptBoundaryAdvanced {
            attempt_id,
            boundary,
        } => {
            let attempt = run
                .attempts
                .get(attempt_id)
                .ok_or_else(|| missing("The attempt does not exist"))?;
            if attempt.status != AttemptStatus::Running || *boundary <= attempt.boundary {
                return Err(illegal("Attempt boundaries only advance while running"));
            }
        }
        RunEventKind::AttemptSettled {
            attempt_id,
            status,
            failure,
            ..
        } => {
            let attempt = run
                .attempts
                .get(attempt_id)
                .ok_or_else(|| missing("The attempt does not exist"))?;
            if !matches!(
                attempt.status,
                AttemptStatus::DispatchRequested | AttemptStatus::Running
            ) || !attempt_status_is_settled(*status)
                || (*status == AttemptStatus::Succeeded && failure.is_some())
                || (matches!(status, AttemptStatus::Failed | AttemptStatus::Interrupted)
                    && failure.is_none())
            {
                return Err(illegal("The attempt settlement is inconsistent"));
            }
        }
        RunEventKind::BlackboardPublished { entry } => validate_blackboard(run, entry)?,
        RunEventKind::TaskSettled {
            task_id, status, ..
        } => {
            let task = run
                .tasks
                .get(task_id)
                .ok_or_else(|| missing("The settled task does not exist"))?;
            if task.status.is_terminal() || !status.is_terminal() {
                return Err(illegal(
                    "Task settlement requires a nonterminal task and terminal status",
                ));
            }
            let latest_attempt = run
                .attempts
                .values()
                .filter(|attempt| attempt.task_id == *task_id)
                .max_by_key(|attempt| attempt.ordinal);
            match status {
                RunTaskStatus::Succeeded
                    if latest_attempt
                        .is_none_or(|attempt| attempt.status != AttemptStatus::Succeeded) =>
                {
                    return Err(illegal(
                        "Successful task settlement requires a successful latest attempt",
                    ));
                }
                RunTaskStatus::Failed
                    if latest_attempt.is_none_or(|attempt| {
                        !matches!(
                            attempt.status,
                            AttemptStatus::Failed | AttemptStatus::Interrupted
                        )
                    }) =>
                {
                    return Err(illegal(
                        "Failed task settlement requires a failed latest attempt",
                    ));
                }
                RunTaskStatus::Cancelled
                    if run.status != RunStatus::Cancelling
                        || latest_attempt
                            .is_none_or(|attempt| attempt.status != AttemptStatus::Cancelled) =>
                {
                    return Err(illegal(
                        "Active task cancellation requires a cancelled latest attempt",
                    ));
                }
                _ => {}
            }
        }
        RunEventKind::CancellationRequested { task_ids } => {
            let expected = run
                .tasks
                .values()
                .filter(|task| !task.status.is_terminal())
                .map(|task| task.id.clone())
                .collect::<BTreeSet<_>>();
            let actual = task_ids.iter().cloned().collect::<BTreeSet<_>>();
            if run.status == RunStatus::Cancelling
                || (run.validated_plan.is_some() && expected.is_empty())
                || expected != actual
                || actual.len() != task_ids.len()
            {
                return Err(invalid(
                    "Cancellation must name every nonterminal task exactly once",
                ));
            }
        }
        RunEventKind::Paused { reason } => {
            if !matches!(run.status, RunStatus::Ready | RunStatus::Running)
                || reason.trim().is_empty()
            {
                return Err(illegal("The run cannot enter the requested paused state"));
            }
        }
        RunEventKind::Resumed => {
            if run.status != RunStatus::Paused
                || run.approvals.values().any(|approval| {
                    approval.kind == RunApprovalKind::Plan
                        && (approval.status != RunApprovalStatus::Approved
                            || at_ms.is_some_and(|at| approval.expires_at_ms <= at))
                })
            {
                return Err(illegal("Only a paused run can resume"));
            }
        }
        RunEventKind::EffectAcknowledged { effect_id, receipt } => {
            if &receipt.effect_id != effect_id {
                return Err(invalid(
                    "The effect receipt id does not match the acknowledged effect",
                ));
            }
            if !run.pending_effects.contains_key(effect_id) {
                return Err(missing("The pending effect does not exist"));
            }
        }
        RunEventKind::RunSettled { status, outcome } => {
            if !status.is_terminal()
                || outcome.trim().is_empty()
                || (run.validated_plan.is_none()
                    && !(run.status == RunStatus::Cancelling && *status == RunStatus::Cancelled))
                || run.tasks.values().any(|task| !task.status.is_terminal())
                || !run.pending_effects.is_empty()
                || run
                    .reservations
                    .values()
                    .any(|reservation| reservation.state == BudgetReservationState::Reserved)
                || run.attempts.values().any(|attempt| {
                    attempt_status_is_settled(attempt.status) && !attempt.usage_reconciled
                })
                || expected_settlement(run) != Some(*status)
            {
                return Err(illegal("The run is not ready for this terminal settlement"));
            }
        }
    }
    Ok(())
}

fn apply_kind(
    run: &mut Run,
    event: &RunEvent,
) -> Result<Vec<OrchestrationEffectKind>, OrchestrationError> {
    let mut effects = Vec::new();
    match &event.kind {
        RunEventKind::PlanAccepted { seed, plan } => {
            run.validated_plan = Some(plan.clone());
            run.tasks = plan.tasks.clone();
            run.edges = plan.edges.clone();
            run.budget.limit = seed.budget_limit;
            run.max_parallel_tasks = seed.max_parallel_tasks;
            run.status = if seed.mode == pandamux_core::OrchestrationMode::ApproveFirst {
                RunStatus::AwaitingApproval
            } else {
                RunStatus::Ready
            };
            effects.push(OrchestrationEffectKind::NotifyRunState {
                run_id: run.id.clone(),
                status: run.status,
            });
        }
        RunEventKind::ApprovalRequested { approval } => {
            let superseded = run
                .approvals
                .values()
                .filter(|existing| {
                    existing.task_id == approval.task_id && existing.kind == approval.kind
                })
                .map(|existing| existing.id.clone())
                .collect::<Vec<_>>();
            run.approvals.insert(approval.id.clone(), approval.clone());
            if let Some(task) = run.tasks.get_mut(&approval.task_id) {
                for approval_id in superseded {
                    task.approval_ids.remove(&approval_id);
                }
                task.approval_ids.insert(approval.id.clone());
                task.status = RunTaskStatus::AwaitingApproval;
            }
            if approval.kind == RunApprovalKind::Plan {
                run.status = RunStatus::AwaitingApproval;
            }
            effects.push(OrchestrationEffectKind::RequestApproval {
                approval: approval.clone(),
            });
        }
        RunEventKind::ApprovalResolved {
            approval_id,
            action_digest,
            status,
            provenance,
        } => {
            let Some(approval) = run.approvals.get_mut(approval_id) else {
                return Ok(effects);
            };
            let is_current = run
                .tasks
                .get(&approval.task_id)
                .is_some_and(|task| task.approval_ids.contains(approval_id));
            if approval.action_digest != *action_digest
                || approval.status != RunApprovalStatus::Pending
                || !is_current
            {
                return Ok(effects);
            }
            approval.status = if event.at_ms >= approval.expires_at_ms {
                RunApprovalStatus::Expired
            } else {
                *status
            };
            approval.decision = Some(provenance.clone());
            let approval_kind = approval.kind;
            let task_id = approval.task_id.clone();
            if approval.status == RunApprovalStatus::Approved {
                if approval_kind == RunApprovalKind::Plan {
                    run.status = RunStatus::Ready;
                }
                refresh_task_after_approval(run, &task_id);
            } else {
                if approval_kind == RunApprovalKind::Plan {
                    run.status = RunStatus::Paused;
                }
                if let Some(task) = run.tasks.get_mut(&task_id) {
                    task.status = RunTaskStatus::Blocked;
                }
            }
        }
        RunEventKind::BudgetReserved { reservation } => {
            run.budget.reserved = checked_add_amount(run.budget.reserved, reservation.amount)?;
            run.reservations
                .insert(reservation.id.clone(), reservation.clone());
            run.tasks
                .get_mut(&reservation.task_id)
                .expect("validated task")
                .budget_reservation_id = Some(reservation.id.clone());
        }
        RunEventKind::BudgetReconciled {
            reservation_id,
            actual,
        } => {
            let reservation = run
                .reservations
                .get_mut(reservation_id)
                .expect("validated reservation");
            run.budget.reserved = checked_sub_amount(run.budget.reserved, reservation.amount)?;
            run.budget.reconciled = checked_add_amount(run.budget.reconciled, *actual)?;
            reservation.state = BudgetReservationState::Reconciled;
        }
        RunEventKind::BudgetReleased { reservation_id } => {
            let reservation = run
                .reservations
                .get_mut(reservation_id)
                .expect("validated reservation");
            run.budget.reserved = checked_sub_amount(run.budget.reserved, reservation.amount)?;
            reservation.state = BudgetReservationState::Released;
            run.tasks
                .get_mut(&reservation.task_id)
                .expect("reservation task")
                .budget_reservation_id = None;
        }
        RunEventKind::RouteSelected { decision } => {
            let task = run
                .tasks
                .get_mut(&decision.task_id)
                .expect("validated task");
            task.selected_agent_id
                .clone_from(&decision.selected_agent_id);
            task.route = Some(decision.clone());
            run.routes
                .insert(decision.task_id.clone(), decision.clone());
        }
        RunEventKind::DispatchRequested { attempt } => {
            run.attempts.insert(attempt.id.clone(), attempt.clone());
            let task = run.tasks.get_mut(&attempt.task_id).expect("validated task");
            task.status = RunTaskStatus::DispatchRequested;
            run.status = RunStatus::Running;
            effects.push(OrchestrationEffectKind::DispatchTask {
                run_id: run.id.clone(),
                task_id: attempt.task_id.clone(),
                attempt_id: attempt.id.clone(),
                route: attempt.route.clone(),
            });
        }
        RunEventKind::AttemptStarted {
            attempt_id,
            thread_id,
            turn_attempt_id,
        } => {
            let attempt = run.attempts.get_mut(attempt_id).expect("validated attempt");
            attempt.status = AttemptStatus::Running;
            attempt.thread_id = Some(thread_id.clone());
            attempt.turn_attempt_id = Some(turn_attempt_id.clone());
            run.tasks
                .get_mut(&attempt.task_id)
                .expect("attempt task")
                .status = RunTaskStatus::Running;
        }
        RunEventKind::AttemptBoundaryAdvanced {
            attempt_id,
            boundary,
        } => {
            run.attempts
                .get_mut(attempt_id)
                .expect("validated attempt")
                .boundary = *boundary;
        }
        RunEventKind::AttemptSettled {
            attempt_id,
            status,
            failure,
            usage_reconciled,
        } => {
            let attempt = run.attempts.get_mut(attempt_id).expect("validated attempt");
            attempt.status = *status;
            attempt.failure = failure.clone();
            attempt.usage_reconciled = *usage_reconciled;
            attempt.ended_at_ms = Some(event.at_ms);
            let task_id = attempt.task_id.clone();
            let boundary = attempt.boundary;
            let ambiguous_effect = attempt.ambiguous_effect;
            if matches!(status, AttemptStatus::Failed | AttemptStatus::Interrupted) {
                let completed_attempts = u32::try_from(
                    run.attempts
                        .values()
                        .filter(|candidate| candidate.task_id == task_id)
                        .count(),
                )
                .unwrap_or(u32::MAX);
                let task = run.tasks.get_mut(&task_id).expect("attempt task");
                task.status = if fallback_eligibility(
                    boundary,
                    ambiguous_effect,
                    completed_attempts,
                    task.max_attempts,
                ) == FallbackEligibility::Allowed
                {
                    RunTaskStatus::Ready
                } else {
                    RunTaskStatus::Blocked
                };
            }
        }
        RunEventKind::BlackboardPublished { entry } => {
            run.blackboard.insert(entry.id.clone(), entry.clone());
            effects.push(OrchestrationEffectKind::PublishBlackboardEntry {
                entry: entry.clone(),
            });
        }
        RunEventKind::TaskSettled {
            task_id,
            status,
            summary: _,
        } => {
            run.tasks.get_mut(task_id).expect("validated task").status = *status;
            if *status == RunTaskStatus::Failed {
                skip_success_descendants(run, task_id);
            }
            refresh_readiness(run);
        }
        RunEventKind::CancellationRequested { task_ids } => {
            run.status = RunStatus::Cancelling;
            let targets = task_ids.iter().cloned().collect::<BTreeSet<_>>();
            for task_id in task_ids {
                let has_active_attempt = run.attempts.values().any(|attempt| {
                    attempt.task_id == *task_id
                        && matches!(
                            attempt.status,
                            AttemptStatus::DispatchRequested | AttemptStatus::Running
                        )
                });
                if !has_active_attempt {
                    run.tasks.get_mut(task_id).expect("validated task").status =
                        RunTaskStatus::Cancelled;
                }
            }
            for attempt in run.attempts.values().filter(|attempt| {
                targets.contains(&attempt.task_id)
                    && matches!(
                        attempt.status,
                        AttemptStatus::DispatchRequested | AttemptStatus::Running
                    )
            }) {
                effects.push(OrchestrationEffectKind::CancelAttempt {
                    run_id: run.id.clone(),
                    task_id: attempt.task_id.clone(),
                    attempt_id: attempt.id.clone(),
                });
            }
            effects.push(OrchestrationEffectKind::NotifyRunState {
                run_id: run.id.clone(),
                status: RunStatus::Cancelling,
            });
        }
        RunEventKind::Paused { .. } => {
            run.status = RunStatus::Paused;
            effects.push(OrchestrationEffectKind::NotifyRunState {
                run_id: run.id.clone(),
                status: RunStatus::Paused,
            });
        }
        RunEventKind::Resumed => {
            run.status = if run.attempts.values().any(|attempt| {
                matches!(
                    attempt.status,
                    AttemptStatus::DispatchRequested | AttemptStatus::Running
                )
            }) {
                RunStatus::Running
            } else {
                RunStatus::Ready
            };
            effects.push(OrchestrationEffectKind::NotifyRunState {
                run_id: run.id.clone(),
                status: run.status,
            });
        }
        RunEventKind::EffectAcknowledged { effect_id, receipt } => {
            if run.pending_effects.remove(effect_id).is_some() {
                run.settled_effects
                    .insert(effect_id.clone(), receipt.clone());
            }
        }
        RunEventKind::RunSettled { status, outcome } => {
            run.status = *status;
            run.outcome = Some(outcome.clone());
            run.ended_at_ms = Some(event.at_ms);
        }
    }
    Ok(effects)
}

fn validate_dispatch(
    run: &Run,
    attempt: &Attempt,
    at_ms: Option<u64>,
) -> Result<(), OrchestrationError> {
    let task = run
        .tasks
        .get(&attempt.task_id)
        .ok_or_else(|| missing("The dispatched task does not exist"))?;
    if !matches!(run.status, RunStatus::Ready | RunStatus::Running)
        || task.status != RunTaskStatus::Ready
        || run.attempts.contains_key(&attempt.id)
        || attempt.status != AttemptStatus::DispatchRequested
        || attempt.boundary != AttemptBoundary::SafeToRetry
        || attempt.thread_id.is_some()
        || attempt.turn_attempt_id.is_some()
        || task.route.is_none()
        || attempt.route.task_id != attempt.task_id
        || attempt.route.selected_agent_id.is_none()
        || attempt
            .route
            .provider
            .selected_provider_instance_id
            .is_none()
    {
        return Err(illegal("The task cannot dispatch this attempt"));
    }
    let active_attempts = run
        .attempts
        .values()
        .filter(|attempt| {
            matches!(
                attempt.status,
                AttemptStatus::DispatchRequested | AttemptStatus::Running
            )
        })
        .count();
    if active_attempts >= usize::try_from(run.max_parallel_tasks).unwrap_or(usize::MAX) {
        return Err(illegal("The run parallel task limit is already reached"));
    }
    let reservation_id = task
        .budget_reservation_id
        .as_ref()
        .ok_or_else(|| illegal("Dispatch requires a durable budget reservation"))?;
    if run
        .reservations
        .get(reservation_id)
        .is_none_or(|reservation| reservation.state != BudgetReservationState::Reserved)
    {
        return Err(illegal("Dispatch requires an active budget reservation"));
    }
    if task.approval_ids.iter().any(|id| {
        run.approvals.get(id).is_none_or(|approval| {
            approval.status != RunApprovalStatus::Approved
                || at_ms.is_some_and(|at| approval.expires_at_ms <= at)
        })
    }) {
        return Err(illegal("Dispatch requires every current approval"));
    }
    let attempt_count = run
        .attempts
        .values()
        .filter(|existing| existing.task_id == task.id)
        .count();
    let expected_ordinal = u32::try_from(attempt_count)
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    if attempt.ordinal != expected_ordinal || attempt.ordinal > task.max_attempts {
        return Err(illegal("The attempt ordinal or retry budget is invalid"));
    }
    if attempt.ordinal == 1 {
        if task.route.as_ref() != Some(&attempt.route) {
            return Err(illegal(
                "The first attempt must use the recorded task route",
            ));
        }
    } else {
        let previous = run
            .attempts
            .values()
            .filter(|existing| existing.task_id == task.id)
            .max_by_key(|existing| existing.ordinal)
            .expect("retry has previous attempt");
        if fallback_eligibility(
            previous.boundary,
            previous.ambiguous_effect,
            u32::try_from(attempt_count).unwrap_or(u32::MAX),
            task.max_attempts,
        ) != FallbackEligibility::Allowed
            || !matches!(
                previous.status,
                AttemptStatus::Failed | AttemptStatus::Interrupted
            )
        {
            return Err(illegal(
                "The previous attempt is not safe or eligible for retry",
            ));
        }
    }
    Ok(())
}

fn validate_blackboard(run: &Run, entry: &BlackboardEntry) -> Result<(), OrchestrationError> {
    if run.blackboard.contains_key(&entry.id)
        || entry.version == 0
        || entry.content_sha256 != canonical_blackboard_sha256(&entry.content)
    {
        return Err(orchestration_error(
            OrchestrationErrorCode::HashMismatch,
            "The blackboard entry identity, version, or content hash is invalid",
        ));
    }
    let producer = run
        .tasks
        .get(&entry.producer)
        .ok_or_else(|| missing("The blackboard producer task does not exist"))?;
    if producer.status != RunTaskStatus::Succeeded
        || !producer
            .outputs
            .iter()
            .any(|output| output == &entry.output_contract)
        || entry
            .consumers
            .iter()
            .any(|consumer| !run.tasks.contains_key(consumer))
    {
        return Err(invalid(
            "The blackboard producer, consumer, or output contract is invalid",
        ));
    }
    Ok(())
}

fn refresh_task_after_approval(run: &mut Run, task_id: &RunTaskId) {
    let approvals_complete = run.tasks.get(task_id).is_some_and(|task| {
        task.approval_ids.iter().all(|id| {
            run.approvals
                .get(id)
                .is_some_and(|approval| approval.status == RunApprovalStatus::Approved)
        })
    });
    if !approvals_complete {
        return;
    }
    if let Some(task) = run.tasks.get_mut(task_id) {
        task.status = RunTaskStatus::Blocked;
    }
    let snapshot = run.tasks.clone();
    if let Some(task) = run.tasks.get_mut(task_id) {
        task.status = if is_task_ready(task, &snapshot, &run.edges) {
            RunTaskStatus::Ready
        } else {
            RunTaskStatus::Blocked
        };
    }
}

fn skip_success_descendants(run: &mut Run, failed: &RunTaskId) {
    let mut queue = VecDeque::from([failed.clone()]);
    let mut visited = BTreeSet::new();
    while let Some(source) = queue.pop_front() {
        if !visited.insert(source.clone()) {
            continue;
        }
        let targets = run
            .edges
            .values()
            .filter(|edge| {
                edge.source == source && edge.requirement == DependencyRequirement::RequiresSuccess
            })
            .map(|edge| edge.target.clone())
            .collect::<BTreeSet<_>>();
        for target in targets {
            let task = run.tasks.get_mut(&target).expect("validated edge target");
            if !task.status.is_terminal()
                && !matches!(
                    task.status,
                    RunTaskStatus::Running | RunTaskStatus::DispatchRequested
                )
            {
                task.status = RunTaskStatus::Skipped;
                queue.push_back(target);
            }
        }
    }
}

fn refresh_readiness(run: &mut Run) {
    let snapshot = run.tasks.clone();
    for task in run.tasks.values_mut() {
        if matches!(
            task.status,
            RunTaskStatus::Pending | RunTaskStatus::Blocked | RunTaskStatus::Ready
        ) {
            task.status = if is_task_ready(task, &snapshot, &run.edges) {
                RunTaskStatus::Ready
            } else {
                RunTaskStatus::Blocked
            };
        }
    }
}

fn expected_settlement(run: &Run) -> Option<RunStatus> {
    if run.tasks.values().any(|task| !task.status.is_terminal()) {
        return None;
    }
    if run.status == RunStatus::Cancelling
        || run
            .tasks
            .values()
            .any(|task| task.status == RunTaskStatus::Cancelled)
    {
        Some(RunStatus::Cancelled)
    } else if run
        .tasks
        .values()
        .any(|task| matches!(task.status, RunTaskStatus::Failed | RunTaskStatus::Skipped))
    {
        Some(RunStatus::Failed)
    } else {
        Some(RunStatus::Succeeded)
    }
}

fn checked_budget_after_reserve(run: &Run, amount: BudgetAmount) -> Result<(), OrchestrationError> {
    let reserved = checked_add_amount(run.budget.reserved, amount)?;
    let committed = checked_add_amount(reserved, run.budget.reconciled)?;
    if !amount_within(committed, run.budget.limit) {
        return Err(orchestration_error(
            OrchestrationErrorCode::BudgetExceeded,
            "The reservation exceeds the remaining run budget",
        ));
    }
    Ok(())
}

fn checked_budget_after_reconcile(
    run: &Run,
    reserved_amount: BudgetAmount,
    actual: BudgetAmount,
) -> Result<(), OrchestrationError> {
    let remaining_reserved = checked_sub_amount(run.budget.reserved, reserved_amount)?;
    let reconciled = checked_add_amount(run.budget.reconciled, actual)?;
    let committed = checked_add_amount(remaining_reserved, reconciled)?;
    if !amount_within(committed, run.budget.limit) {
        return Err(orchestration_error(
            OrchestrationErrorCode::BudgetExceeded,
            "Reconciled usage exceeds the trusted run budget",
        ));
    }
    Ok(())
}

fn checked_add_amount(
    left: BudgetAmount,
    right: BudgetAmount,
) -> Result<BudgetAmount, OrchestrationError> {
    Ok(BudgetAmount {
        tokens: left
            .tokens
            .checked_add(right.tokens)
            .ok_or_else(budget_overflow)?,
        cost_micro_usd: left
            .cost_micro_usd
            .checked_add(right.cost_micro_usd)
            .ok_or_else(budget_overflow)?,
        runtime_ms: left
            .runtime_ms
            .checked_add(right.runtime_ms)
            .ok_or_else(budget_overflow)?,
    })
}

fn checked_sub_amount(
    left: BudgetAmount,
    right: BudgetAmount,
) -> Result<BudgetAmount, OrchestrationError> {
    Ok(BudgetAmount {
        tokens: left
            .tokens
            .checked_sub(right.tokens)
            .ok_or_else(budget_overflow)?,
        cost_micro_usd: left
            .cost_micro_usd
            .checked_sub(right.cost_micro_usd)
            .ok_or_else(budget_overflow)?,
        runtime_ms: left
            .runtime_ms
            .checked_sub(right.runtime_ms)
            .ok_or_else(budget_overflow)?,
    })
}

fn amount_within(value: BudgetAmount, limit: BudgetAmount) -> bool {
    value.tokens <= limit.tokens
        && value.cost_micro_usd <= limit.cost_micro_usd
        && value.runtime_ms <= limit.runtime_ms
}

fn budget_overflow() -> OrchestrationError {
    orchestration_error(
        OrchestrationErrorCode::BudgetOverflow,
        "Budget arithmetic overflowed or underflowed",
    )
}

fn attempt_status_is_settled(status: AttemptStatus) -> bool {
    matches!(
        status,
        AttemptStatus::Succeeded
            | AttemptStatus::Failed
            | AttemptStatus::Cancelled
            | AttemptStatus::Interrupted
    )
}

fn event_digest(event: &RunEvent) -> String {
    sha256_hex(&canonical_json_bytes(event))
}

fn derived_effect_id(
    run_id: &pandamux_core::RunId,
    event_id: &RunEventId,
    index: usize,
    kind: &OrchestrationEffectKind,
) -> OrchestrationEffectId {
    let bytes = canonical_json_bytes(&(run_id.as_str(), event_id.as_str(), index, kind));
    OrchestrationEffectId::new(format!("effect-{}", sha256_hex(&bytes)))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonicalize_json(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

fn canonical_json_bytes(value: &impl Serialize) -> Vec<u8> {
    let value = serde_json::to_value(value).expect("canonical value is serializable");
    serde_json::to_vec(&canonicalize_json(&value)).expect("canonical value is serializable")
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn orchestration_error(code: OrchestrationErrorCode, explanation: &str) -> OrchestrationError {
    OrchestrationError {
        code,
        explanation: SafeText::new(explanation),
    }
}

fn illegal(explanation: &str) -> OrchestrationError {
    orchestration_error(OrchestrationErrorCode::IllegalTransition, explanation)
}

fn invalid(explanation: &str) -> OrchestrationError {
    orchestration_error(OrchestrationErrorCode::InvalidEvent, explanation)
}

fn missing(explanation: &str) -> OrchestrationError {
    orchestration_error(OrchestrationErrorCode::MissingRecord, explanation)
}
