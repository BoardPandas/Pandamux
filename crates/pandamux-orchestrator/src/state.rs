use crate::dag::is_task_ready;
use crate::diagnostics::SafeText;
use crate::policy::{canonical_json_bytes, canonical_json_sha256};
use crate::routing::{FallbackEligibility, fallback_eligibility};
use pandamux_core::{
    ArtifactKey, Attempt, AttemptBoundary, AttemptStatus, BlackboardArtifact, BlackboardEntry,
    BudgetAmount, BudgetDimension, BudgetIncomplete, BudgetReservation, BudgetReservationState,
    BudgetUsage, CheckpointKind, DependencyRequirement, MAX_ARTIFACT_BYTES,
    MAX_ARTIFACT_CONTAINER_ITEMS, MAX_ARTIFACT_JSON_DEPTH, MAX_ARTIFACT_STRING_BYTES,
    MAX_ARTIFACTS_PER_RUN, MAX_ARTIFACTS_PER_TASK, MAX_RUN_ARTIFACT_BYTES, OrchestrationEffect,
    OrchestrationEffectId, OrchestrationEffectKind, OutputContract, Run, RunApprovalKind,
    RunApprovalStatus, RunAttemptId, RunCommand, RunEvent, RunEventId, RunEventKind, RunStatus,
    RunTaskId, RunTaskStatus, SupplementalBudgetGrant, VerificationEvidence,
    VerificationEvidenceKind, VerificationEvidenceOutcome,
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
        RunCommand::GrantSupplementalBudget { grant } => {
            if let Some(existing) = run.budget_grants.get(&grant.id) {
                if existing == &grant {
                    return Ok(Vec::new());
                }
                return Err(orchestration_error(
                    OrchestrationErrorCode::ReplayConflict,
                    "A supplemental budget grant identity cannot change",
                ));
            }
            RunEventKind::SupplementalBudgetGranted { grant }
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
        RunCommand::RecordCheckpoint { checkpoint } => {
            if let Some(existing) = run.checkpoints.get(&checkpoint.id) {
                if existing == &checkpoint {
                    return Ok(Vec::new());
                }
                return Err(orchestration_error(
                    OrchestrationErrorCode::ReplayConflict,
                    "A checkpoint identity cannot change",
                ));
            }
            RunEventKind::CheckpointRecorded { checkpoint }
        }
        RunCommand::MarkAttemptAmbiguous {
            attempt_id,
            reason_code,
        } => {
            if run.attempts.get(&attempt_id).is_some_and(|attempt| {
                attempt.ambiguous_effect
                    && attempt.ambiguity_reason.as_deref() == Some(reason_code.as_str())
            }) {
                return Ok(Vec::new());
            }
            RunEventKind::AttemptMarkedAmbiguous {
                attempt_id,
                reason_code,
            }
        }
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
        RunCommand::RecordVerificationEvidence { evidence } => {
            if let Some(existing) = run.verification_evidence.get(&evidence.id) {
                if existing == &evidence {
                    return Ok(Vec::new());
                }
                return Err(orchestration_error(
                    OrchestrationErrorCode::ReplayConflict,
                    "A verification evidence identity cannot change",
                ));
            }
            RunEventKind::VerificationEvidenceRecorded { evidence }
        }
        RunCommand::CompleteTask {
            task_id,
            attempt_id,
            artifacts,
            evidence_ids,
            summary,
        } => RunEventKind::TaskSucceeded {
            task_id,
            attempt_id,
            artifacts,
            evidence_ids,
            summary,
        },
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
            payload_sha256: canonical_json_sha256(&kind),
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
    canonical_json_sha256(content)
}

pub fn canonical_artifact_json(content: &Value) -> Vec<u8> {
    canonical_json_bytes(content)
}

pub fn output_contract_schema_sha256(contract: &OutputContract) -> String {
    canonical_json_sha256(&contract.schema)
}

pub fn artifact_key(producer: &RunTaskId, contract: &OutputContract) -> ArtifactKey {
    let digest = canonical_json_sha256(&(
        producer.as_str(),
        contract.kind.as_str(),
        output_contract_schema_sha256(contract),
    ));
    ArtifactKey::new(format!("artifact-{digest}"))
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
                || if approval.kind == RunApprovalKind::Budget {
                    task.status.is_terminal()
                } else {
                    matches!(
                        task.status,
                        RunTaskStatus::DispatchRequested
                            | RunTaskStatus::Running
                            | RunTaskStatus::Succeeded
                            | RunTaskStatus::Failed
                            | RunTaskStatus::Skipped
                            | RunTaskStatus::Cancelled
                    )
                }
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
                || reservation.actual != BudgetUsage::default()
                || reservation.attempt_id.as_str().trim().is_empty()
                || run
                    .reservations
                    .values()
                    .any(|existing| existing.attempt_id == reservation.attempt_id)
                || task.budget_reservation_id.is_some()
                || task.status != RunTaskStatus::Ready
                || task.route.is_none()
                || !matches!(run.status, RunStatus::Ready | RunStatus::Running)
                || budget_incomplete(run.budget.incomplete)
                || amount_nonzero(run.budget.overage)
                || !valid_utc_date(&reservation.ledger_utc_date)
                || !task_reservation_within_limit(run, reservation, at_ms)?
            {
                return Err(invalid("The budget reservation is malformed or colliding"));
            }
            checked_budget_after_reserve(run, reservation, at_ms)?;
        }
        RunEventKind::BudgetReconciled {
            reservation_id,
            actual,
        } => {
            let reservation = run
                .reservations
                .get(reservation_id)
                .ok_or_else(|| missing("The budget reservation does not exist"))?;
            if !matches!(
                reservation.state,
                BudgetReservationState::Reserved | BudgetReservationState::PartiallyReconciled
            ) || !valid_usage_update(reservation, *actual)
                || run
                    .attempts
                    .get(&reservation.attempt_id)
                    .is_none_or(|attempt| !attempt_status_is_settled(attempt.status))
            {
                return Err(illegal(
                    "Only new truthful dimensions can reconcile an active budget",
                ));
            }
            checked_budget_after_reconcile(run, reservation, *actual)?;
        }
        RunEventKind::BudgetReleased { reservation_id } => {
            let reservation = run
                .reservations
                .get(reservation_id)
                .ok_or_else(|| missing("The budget reservation does not exist"))?;
            if reservation.state != BudgetReservationState::Reserved
                || run
                    .attempts
                    .get(&reservation.attempt_id)
                    .is_some_and(|attempt| attempt.status != AttemptStatus::DispatchRequested)
            {
                return Err(illegal(
                    "Only a reserved pre-dispatch budget can be released",
                ));
            }
        }
        RunEventKind::SupplementalBudgetGranted { grant } => {
            validate_supplemental_grant(run, grant, at_ms)?;
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
        RunEventKind::CheckpointRecorded { checkpoint } => {
            validate_checkpoint(run, checkpoint, at_ms)?;
        }
        RunEventKind::AttemptMarkedAmbiguous {
            attempt_id,
            reason_code,
        } => {
            let attempt = run
                .attempts
                .get(attempt_id)
                .ok_or_else(|| missing("The ambiguous attempt does not exist"))?;
            if !valid_reason_code(reason_code)
                || attempt.ambiguous_effect
                || attempt_status_is_settled(attempt.status)
                    && attempt.status == AttemptStatus::Succeeded
            {
                return Err(illegal("The attempt cannot enter this ambiguous state"));
            }
        }
        RunEventKind::AttemptSettled {
            attempt_id,
            status,
            failure,
            usage_reconciled,
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
                || *usage_reconciled != reservation_settled_for_attempt(run, attempt_id)
            {
                return Err(illegal("The attempt settlement is inconsistent"));
            }
        }
        RunEventKind::VerificationEvidenceRecorded { evidence } => {
            validate_verification_evidence(run, evidence, at_ms)?;
        }
        RunEventKind::TaskSucceeded {
            task_id,
            attempt_id,
            artifacts,
            evidence_ids,
            ..
        } => validate_task_success(run, task_id, attempt_id, artifacts, evidence_ids)?,
        RunEventKind::TaskSettled {
            task_id, status, ..
        } => {
            let task = run
                .tasks
                .get(task_id)
                .ok_or_else(|| missing("The settled task does not exist"))?;
            if task.status.is_terminal()
                || !status.is_terminal()
                || *status == RunTaskStatus::Succeeded
            {
                return Err(illegal(
                    "Non-success task settlement requires a nonterminal task and terminal failure state",
                ));
            }
            let latest_attempt = run
                .attempts
                .values()
                .filter(|attempt| attempt.task_id == *task_id)
                .max_by_key(|attempt| attempt.ordinal);
            match status {
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
                || budget_incomplete(run.budget.incomplete)
                || amount_nonzero(run.budget.overage)
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
                || run.reservations.values().any(|reservation| {
                    matches!(
                        reservation.state,
                        BudgetReservationState::Reserved
                            | BudgetReservationState::PartiallyReconciled
                    )
                })
                || budget_incomplete(run.budget.incomplete)
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
            let release = reservation_release_for_update(reservation, *actual);
            let delta = usage_delta(reservation.actual, *actual);
            run.budget.reserved = checked_sub_amount(run.budget.reserved, release)?;
            run.budget.reconciled = checked_add_amount(run.budget.reconciled, delta)?;
            reservation.actual = merge_usage(reservation.actual, *actual);
            reservation.state = if usage_complete(reservation.actual) {
                BudgetReservationState::Reconciled
            } else {
                BudgetReservationState::PartiallyReconciled
            };
            let task_id = reservation.task_id.clone();
            let attempt_id = reservation.attempt_id.clone();
            let complete = reservation.state == BudgetReservationState::Reconciled;
            if complete {
                run.tasks
                    .get_mut(&task_id)
                    .expect("reservation task")
                    .budget_reservation_id = None;
            }
            run.attempts
                .get_mut(&attempt_id)
                .expect("reservation attempt")
                .usage_reconciled = complete;
            recompute_budget_state(run)?;
            refresh_retry_after_budget(run, &task_id);
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
            let task_id = reservation.task_id.clone();
            recompute_budget_state(run)?;
            refresh_retry_after_budget(run, &task_id);
        }
        RunEventKind::SupplementalBudgetGranted { grant } => {
            add_budget_dimension(&mut run.budget.granted, grant.dimension, grant.amount)?;
            run.budget_grants.insert(grant.id.clone(), grant.clone());
            recompute_budget_state(run)?;
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
        RunEventKind::CheckpointRecorded { checkpoint } => {
            run.checkpoints
                .insert(checkpoint.id.clone(), checkpoint.clone());
            run.attempts
                .get_mut(&checkpoint.attempt_id)
                .expect("validated attempt")
                .checkpoint_ids
                .insert(checkpoint.id.clone());
        }
        RunEventKind::AttemptMarkedAmbiguous {
            attempt_id,
            reason_code,
        } => {
            let attempt = run.attempts.get_mut(attempt_id).expect("validated attempt");
            attempt.ambiguous_effect = true;
            attempt.ambiguity_reason = Some(reason_code.clone());
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
                task.status = if task.budget_reservation_id.is_none()
                    && run.status != RunStatus::Paused
                    && fallback_eligibility(
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
        RunEventKind::VerificationEvidenceRecorded { evidence } => {
            run.verification_evidence
                .insert(evidence.id.clone(), evidence.clone());
        }
        RunEventKind::TaskSucceeded {
            task_id, artifacts, ..
        } => {
            for artifact in artifacts {
                let entry = BlackboardEntry {
                    id: artifact.id.clone(),
                    run_id: artifact.run_id.clone(),
                    artifact_key: artifact.artifact_key.clone(),
                    version: artifact.version,
                    previous_entry_id: artifact.previous_entry_id.clone(),
                    producer: artifact.producer.clone(),
                    consumers: artifact.consumers.clone(),
                    output_contract: artifact.output_contract.clone(),
                    schema_sha256: artifact.schema_sha256.clone(),
                    canonical_json: artifact.canonical_json.clone(),
                    byte_len: artifact.byte_len,
                    content_sha256: artifact.content_sha256.clone(),
                    created_event_id: event.event_id.clone(),
                    provenance: artifact.provenance.clone(),
                };
                bind_artifact_to_consumers(run, &entry);
                run.blackboard.insert(entry.id.clone(), entry.clone());
                effects.push(OrchestrationEffectKind::NotifyBlackboardPublished {
                    run_id: run.id.clone(),
                    entry_id: entry.id,
                    content_sha256: entry.content_sha256,
                });
            }
            run.tasks.get_mut(task_id).expect("validated task").status = RunTaskStatus::Succeeded;
            refresh_readiness(run);
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
        .is_none_or(|reservation| {
            reservation.state != BudgetReservationState::Reserved
                || reservation.attempt_id != attempt.id
        })
    {
        return Err(illegal(
            "Dispatch requires an active reservation bound to the exact attempt",
        ));
    }
    if budget_incomplete(run.budget.incomplete) || amount_nonzero(run.budget.overage) {
        return Err(illegal(
            "Dispatch is blocked while budget accounting is incomplete or over limit",
        ));
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

fn validate_checkpoint(
    run: &Run,
    checkpoint: &pandamux_core::CheckpointRef,
    at_ms: Option<u64>,
) -> Result<(), OrchestrationError> {
    let attempt = run
        .attempts
        .get(&checkpoint.attempt_id)
        .ok_or_else(|| missing("The checkpoint attempt does not exist"))?;
    if run.checkpoints.contains_key(&checkpoint.id)
        || checkpoint.run_id != run.id
        || checkpoint.task_id != attempt.task_id
        || checkpoint.value.trim().is_empty()
        || checkpoint.value.len() > 256
        || checkpoint.created_at_ms != at_ms.unwrap_or(checkpoint.created_at_ms)
        || checkpoint
            .thread_id
            .as_ref()
            .is_some_and(|thread_id| attempt.thread_id.as_ref() != Some(thread_id))
    {
        return Err(invalid(
            "The checkpoint identity or attempt ownership is invalid",
        ));
    }
    match checkpoint.kind {
        CheckpointKind::BeforeAttempt
            if attempt.status != AttemptStatus::DispatchRequested
                || checkpoint.thread_id.is_some() =>
        {
            Err(illegal(
                "A before-attempt checkpoint precedes provider thread creation",
            ))
        }
        CheckpointKind::AfterAttempt
            if !attempt_status_is_settled(attempt.status)
                || checkpoint.thread_id.as_ref() != attempt.thread_id.as_ref() =>
        {
            Err(illegal(
                "An after-attempt checkpoint requires the exact terminal thread",
            ))
        }
        CheckpointKind::Recovery if !attempt.ambiguous_effect => Err(illegal(
            "A recovery checkpoint requires an ambiguous attempt",
        )),
        _ => Ok(()),
    }
}

fn validate_verification_evidence(
    run: &Run,
    evidence: &VerificationEvidence,
    at_ms: Option<u64>,
) -> Result<(), OrchestrationError> {
    let attempt = run
        .attempts
        .get(&evidence.attempt_id)
        .ok_or_else(|| missing("The verification attempt does not exist"))?;
    if run.verification_evidence.contains_key(&evidence.id)
        || evidence.run_id != run.id
        || evidence.task_id != attempt.task_id
        || evidence.recorded_at_ms != at_ms.unwrap_or(evidence.recorded_at_ms)
        || evidence
            .command_digest
            .as_deref()
            .is_some_and(|digest| !is_sha256(digest))
        || evidence
            .arguments_digest
            .as_deref()
            .is_some_and(|digest| !is_sha256(digest))
        || !is_sha256(&evidence.bounded_output_digest)
        || evidence
            .checkpoint_diff_digest
            .as_deref()
            .is_some_and(|digest| !is_sha256(digest))
        || attempt
            .route
            .provider
            .selected_provider_instance_id
            .as_ref()
            != Some(&evidence.provider_instance_id)
        || attempt.thread_id.as_ref() != Some(&evidence.thread_id)
        || attempt.turn_attempt_id.as_ref() != Some(&evidence.turn_attempt_id)
        || evidence
            .checkpoint_id
            .as_ref()
            .is_some_and(|checkpoint_id| {
                run.checkpoints.get(checkpoint_id).is_none_or(|checkpoint| {
                    checkpoint.attempt_id != attempt.id || checkpoint.task_id != attempt.task_id
                })
            })
    {
        return Err(invalid(
            "Verification evidence identity, digest, or attempt ownership is invalid",
        ));
    }
    let kind_fields_match = match evidence.kind {
        VerificationEvidenceKind::Command => {
            evidence.command_digest.is_some()
                && evidence.arguments_digest.is_some()
                && evidence.exit_status.is_some()
                && evidence.checkpoint_id.is_none()
                && evidence.checkpoint_diff_digest.is_none()
                && evidence.tool_item_id.is_none()
                && evidence.usage_observation_id.is_none()
        }
        VerificationEvidenceKind::Checkpoint => {
            evidence.command_digest.is_none()
                && evidence.arguments_digest.is_none()
                && evidence.exit_status.is_none()
                && evidence.checkpoint_id.is_some()
                && evidence.checkpoint_diff_digest.is_some()
                && evidence.tool_item_id.is_none()
                && evidence.usage_observation_id.is_none()
        }
        VerificationEvidenceKind::ToolResult => {
            evidence.command_digest.is_none()
                && evidence.arguments_digest.is_none()
                && evidence.exit_status.is_none()
                && evidence.checkpoint_id.is_none()
                && evidence.checkpoint_diff_digest.is_none()
                && evidence
                    .tool_item_id
                    .as_ref()
                    .is_some_and(|item| !item.trim().is_empty())
                && evidence.usage_observation_id.is_none()
        }
        VerificationEvidenceKind::Usage => {
            evidence.command_digest.is_none()
                && evidence.arguments_digest.is_none()
                && evidence.exit_status.is_none()
                && evidence.checkpoint_id.is_none()
                && evidence.checkpoint_diff_digest.is_none()
                && evidence.tool_item_id.is_none()
                && evidence.usage_observation_id.is_some()
        }
    };
    if !kind_fields_match {
        return Err(invalid(
            "Verification evidence lacks the durable fact required by its kind",
        ));
    }
    Ok(())
}

fn validate_supplemental_grant(
    run: &Run,
    grant: &SupplementalBudgetGrant,
    at_ms: Option<u64>,
) -> Result<(), OrchestrationError> {
    let approval = run
        .approvals
        .get(&grant.approval_id)
        .ok_or_else(|| missing("The supplemental budget approval does not exist"))?;
    if run.budget_grants.contains_key(&grant.id)
        || run
            .budget_grants
            .values()
            .any(|existing| existing.approval_id == grant.approval_id)
        || grant.run_id != run.id
        || grant.amount == 0
        || !run.tasks.contains_key(&grant.task_id)
        || grant.policy_revision != run.policy_revision
        || !is_sha256(&grant.action_digest)
        || at_ms.is_some_and(|at| grant.expires_at_ms <= at)
        || approval.run_id != run.id
        || approval.task_id != grant.task_id
        || approval.kind != RunApprovalKind::Budget
        || approval.policy_revision != grant.policy_revision
        || approval.action_digest != grant.action_digest
        || approval.status != RunApprovalStatus::Approved
        || approval.decision.is_none()
        || at_ms.is_some_and(|at| approval.expires_at_ms <= at)
        || grant.expires_at_ms > approval.expires_at_ms
        || run
            .attempts
            .get(&grant.attempt_id)
            .is_some_and(|attempt| attempt.task_id != grant.task_id)
        || run
            .reservations
            .values()
            .find(|reservation| reservation.attempt_id == grant.attempt_id)
            .is_some_and(|reservation| reservation.task_id != grant.task_id)
        || run
            .tasks
            .get(&grant.task_id)
            .and_then(|task| task.budget_reservation_id.as_ref())
            .and_then(|reservation_id| run.reservations.get(reservation_id))
            .is_some_and(|reservation| reservation.attempt_id != grant.attempt_id)
    {
        return Err(invalid(
            "The supplemental budget grant is stale, unapproved, or identity-mismatched",
        ));
    }
    let mut granted = run.budget.granted;
    add_budget_dimension(&mut granted, grant.dimension, grant.amount)?;
    Ok(())
}

fn validate_task_success(
    run: &Run,
    task_id: &RunTaskId,
    attempt_id: &RunAttemptId,
    artifacts: &[BlackboardArtifact],
    evidence_ids: &BTreeSet<pandamux_core::VerificationEvidenceId>,
) -> Result<(), OrchestrationError> {
    let task = run
        .tasks
        .get(task_id)
        .ok_or_else(|| missing("The completed task does not exist"))?;
    let attempt = run
        .attempts
        .get(attempt_id)
        .ok_or_else(|| missing("The completed attempt does not exist"))?;
    let latest_attempt = run
        .attempts
        .values()
        .filter(|candidate| candidate.task_id == *task_id)
        .max_by_key(|candidate| candidate.ordinal);
    if task.status.is_terminal()
        || attempt.task_id != *task_id
        || latest_attempt.map(|candidate| &candidate.id) != Some(attempt_id)
        || attempt.status != AttemptStatus::Succeeded
        || !attempt.usage_reconciled
        || task.budget_reservation_id.is_some()
    {
        return Err(illegal(
            "Atomic task success requires the latest successful usage-reconciled attempt",
        ));
    }
    let existing_task_artifacts = run
        .blackboard
        .values()
        .filter(|entry| entry.producer == *task_id)
        .count();
    if existing_task_artifacts
        .checked_add(artifacts.len())
        .is_none_or(|count| count > MAX_ARTIFACTS_PER_TASK)
        || run
            .blackboard
            .len()
            .checked_add(artifacts.len())
            .is_none_or(|count| count > MAX_ARTIFACTS_PER_RUN)
    {
        return Err(invalid(
            "The task or run artifact count exceeds its hard limit",
        ));
    }

    let expected_keys = task
        .outputs
        .iter()
        .map(|contract| artifact_key(task_id, contract))
        .collect::<BTreeSet<_>>();
    let actual_keys = artifacts
        .iter()
        .map(|artifact| artifact.artifact_key.clone())
        .collect::<BTreeSet<_>>();
    if artifacts.len() != actual_keys.len() || actual_keys != expected_keys {
        return Err(invalid(
            "Atomic task success must publish exactly one artifact for every declared output",
        ));
    }

    let existing_bytes = run.blackboard.values().try_fold(0usize, |total, entry| {
        let bytes = usize::try_from(entry.byte_len).map_err(|_| budget_overflow())?;
        total.checked_add(bytes).ok_or_else(budget_overflow)
    })?;
    let mut batch_ids = BTreeSet::new();
    let mut batch_bytes = 0usize;
    for artifact in artifacts {
        if !batch_ids.insert(artifact.id.clone()) || run.blackboard.contains_key(&artifact.id) {
            return Err(orchestration_error(
                OrchestrationErrorCode::IdentityCollision,
                "The artifact entry identity is already present",
            ));
        }
        validate_artifact(run, task, attempt, artifact)?;
        let bytes = usize::try_from(artifact.byte_len).map_err(|_| budget_overflow())?;
        batch_bytes = batch_bytes.checked_add(bytes).ok_or_else(budget_overflow)?;
    }
    if existing_bytes
        .checked_add(batch_bytes)
        .is_none_or(|bytes| bytes > MAX_RUN_ARTIFACT_BYTES)
    {
        return Err(invalid(
            "The run artifact bytes exceed the 16 MiB hard limit",
        ));
    }

    let mut recorded_kinds = BTreeSet::new();
    for evidence_id in evidence_ids {
        let evidence = run
            .verification_evidence
            .get(evidence_id)
            .ok_or_else(|| missing("The task references missing verification evidence"))?;
        if evidence.run_id != run.id
            || evidence.outcome != VerificationEvidenceOutcome::Passed
            || !evidence_origin_allowed(run, task, attempt_id, evidence)
        {
            return Err(invalid(
                "Task success can use only passing evidence from its exact attempt or a successful declared producer",
            ));
        }
        recorded_kinds.insert(evidence.kind);
    }
    if !recorded_kinds.is_superset(&task.required_verification) {
        return Err(illegal(
            "The task cannot succeed before every configured verification record exists",
        ));
    }
    Ok(())
}

fn evidence_origin_allowed(
    run: &Run,
    task: &pandamux_core::AgentTask,
    attempt_id: &RunAttemptId,
    evidence: &VerificationEvidence,
) -> bool {
    if evidence.task_id == task.id {
        return evidence.attempt_id == *attempt_id;
    }
    let declared_producer = task
        .inputs
        .iter()
        .any(|input| input.producer_task_id.as_ref() == Some(&evidence.task_id))
        || run
            .edges
            .values()
            .any(|edge| edge.source == evidence.task_id && edge.target == task.id);
    if !declared_producer
        || run
            .tasks
            .get(&evidence.task_id)
            .is_none_or(|producer| producer.status != RunTaskStatus::Succeeded)
    {
        return false;
    }
    run.attempts
        .values()
        .filter(|attempt| attempt.task_id == evidence.task_id)
        .max_by_key(|attempt| attempt.ordinal)
        .is_some_and(|attempt| {
            attempt.id == evidence.attempt_id
                && attempt.status == AttemptStatus::Succeeded
                && attempt.usage_reconciled
        })
}

fn validate_artifact(
    run: &Run,
    task: &pandamux_core::AgentTask,
    attempt: &Attempt,
    artifact: &BlackboardArtifact,
) -> Result<(), OrchestrationError> {
    let expected_key = artifact_key(&task.id, &artifact.output_contract);
    let expected_consumers = run
        .tasks
        .values()
        .filter(|consumer| {
            consumer.inputs.iter().any(|input| {
                input.producer_task_id.as_ref() == Some(&task.id)
                    && input.contract == artifact.output_contract
            })
        })
        .map(|consumer| consumer.id.clone())
        .collect::<BTreeSet<_>>();
    if artifact.run_id != run.id
        || artifact.producer != task.id
        || artifact.artifact_key != expected_key
        || !task.outputs.contains(&artifact.output_contract)
        || artifact.consumers != expected_consumers
        || artifact.schema_sha256 != output_contract_schema_sha256(&artifact.output_contract)
        || artifact.byte_len != u64::try_from(artifact.canonical_json.len()).unwrap_or(u64::MAX)
        || artifact.content_sha256 != sha256_hex(&artifact.canonical_json)
    {
        return Err(orchestration_error(
            OrchestrationErrorCode::HashMismatch,
            "The artifact identity, contract, scope, size, or digest is invalid",
        ));
    }
    if artifact.canonical_json.len() > MAX_ARTIFACT_BYTES {
        return Err(invalid(
            "Artifact canonical JSON exceeds the 1 MiB hard limit",
        ));
    }
    let content: Value = serde_json::from_slice(&artifact.canonical_json)
        .map_err(|_| invalid("Artifact bytes must contain one valid JSON value"))?;
    if canonical_artifact_json(&content) != artifact.canonical_json {
        return Err(invalid("Artifact JSON bytes must already be canonical"));
    }
    validate_json_bounds(&content, 0)?;
    crate::dag::validate_output_value(&artifact.output_contract, &content)
        .map_err(|_| invalid("Artifact JSON does not satisfy its supported output schema"))?;

    let prior = run
        .blackboard
        .values()
        .filter(|entry| entry.artifact_key == artifact.artifact_key)
        .max_by_key(|entry| entry.version);
    match prior {
        None if artifact.version == 1 && artifact.previous_entry_id.is_none() => {}
        Some(previous)
            if previous.version.checked_add(1) == Some(artifact.version)
                && artifact.previous_entry_id.as_ref() == Some(&previous.id) => {}
        _ => {
            return Err(invalid(
                "Artifact versions must begin at one and name the immediate predecessor",
            ));
        }
    }
    if let Some(provenance) = &artifact.provenance {
        validate_artifact_provenance(run, task, attempt, provenance)?;
    }
    Ok(())
}

fn validate_artifact_provenance(
    run: &Run,
    task: &pandamux_core::AgentTask,
    attempt: &Attempt,
    provenance: &pandamux_core::ArtifactProvenance,
) -> Result<(), OrchestrationError> {
    match provenance {
        pandamux_core::ArtifactProvenance::Checkpoint { checkpoint_id } => {
            if run.checkpoints.get(checkpoint_id).is_none_or(|checkpoint| {
                checkpoint.task_id != task.id || checkpoint.attempt_id != attempt.id
            }) {
                return Err(invalid(
                    "Artifact checkpoint provenance does not match the attempt",
                ));
            }
        }
        pandamux_core::ArtifactProvenance::Command { evidence_id } => {
            if run
                .verification_evidence
                .get(evidence_id)
                .is_none_or(|evidence| {
                    evidence.task_id != task.id || evidence.attempt_id != attempt.id
                })
            {
                return Err(invalid(
                    "Artifact command provenance does not match the attempt",
                ));
            }
        }
        pandamux_core::ArtifactProvenance::ToolResult {
            thread_id,
            turn_attempt_id,
            tool_item_id,
        } => {
            if attempt.thread_id.as_ref() != Some(thread_id)
                || attempt.turn_attempt_id.as_ref() != Some(turn_attempt_id)
                || tool_item_id.trim().is_empty()
            {
                return Err(invalid(
                    "Artifact tool provenance does not match the attempt",
                ));
            }
        }
        pandamux_core::ArtifactProvenance::Usage { observation_id } => {
            if observation_id.as_str().trim().is_empty() {
                return Err(invalid("Artifact usage provenance must be identified"));
            }
        }
    }
    Ok(())
}

fn validate_json_bounds(value: &Value, depth: usize) -> Result<(), OrchestrationError> {
    match value {
        Value::String(text) if text.len() > MAX_ARTIFACT_STRING_BYTES => {
            Err(invalid("Artifact JSON contains an oversized string"))
        }
        Value::Array(items) => {
            let child_depth = depth
                .checked_add(1)
                .ok_or_else(|| invalid("Artifact JSON exceeds the maximum nesting depth"))?;
            if child_depth > MAX_ARTIFACT_JSON_DEPTH {
                return Err(invalid("Artifact JSON exceeds the maximum nesting depth"));
            }
            if items.len() > MAX_ARTIFACT_CONTAINER_ITEMS {
                return Err(invalid("Artifact JSON contains an oversized array"));
            }
            for item in items {
                validate_json_bounds(item, child_depth)?;
            }
            Ok(())
        }
        Value::Object(entries) => {
            let child_depth = depth
                .checked_add(1)
                .ok_or_else(|| invalid("Artifact JSON exceeds the maximum nesting depth"))?;
            if child_depth > MAX_ARTIFACT_JSON_DEPTH {
                return Err(invalid("Artifact JSON exceeds the maximum nesting depth"));
            }
            if entries.len() > MAX_ARTIFACT_CONTAINER_ITEMS {
                return Err(invalid("Artifact JSON contains an oversized object"));
            }
            for (key, item) in entries {
                if key.len() > MAX_ARTIFACT_STRING_BYTES {
                    return Err(invalid("Artifact JSON contains an oversized object key"));
                }
                validate_json_bounds(item, child_depth)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn bind_artifact_to_consumers(run: &mut Run, entry: &BlackboardEntry) {
    for consumer_id in &entry.consumers {
        let consumer = run.tasks.get_mut(consumer_id).expect("validated consumer");
        for input in &mut consumer.inputs {
            if input.producer_task_id.as_ref() == Some(&entry.producer)
                && input.contract == entry.output_contract
            {
                input.blackboard_entry_id = Some(entry.id.clone());
            }
        }
    }
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

fn checked_budget_after_reserve(
    run: &Run,
    reservation: &BudgetReservation,
    at_ms: Option<u64>,
) -> Result<(), OrchestrationError> {
    let existing = scope_base_required(run, None)?;
    let grants = grants_for_attempt(run, &reservation.attempt_id, at_ms)?;
    let required = checked_add_amount(existing, amount_saturating_sub(reservation.amount, grants))?;
    if !amount_within(required, run.budget.limit) {
        return Err(orchestration_error(
            OrchestrationErrorCode::BudgetExceeded,
            "The reservation exceeds the remaining run budget",
        ));
    }
    Ok(())
}

fn checked_budget_after_reconcile(
    run: &Run,
    reservation: &BudgetReservation,
    actual: BudgetUsage,
) -> Result<(), OrchestrationError> {
    let release = reservation_release_for_update(reservation, actual);
    let remaining_reserved = checked_sub_amount(run.budget.reserved, release)?;
    let _ = checked_add_amount(
        run.budget.reconciled,
        usage_delta(reservation.actual, actual),
    )?;
    let _ = checked_add_amount(remaining_reserved, run.budget.reconciled)?;
    Ok(())
}

fn task_reservation_within_limit(
    run: &Run,
    reservation: &BudgetReservation,
    at_ms: Option<u64>,
) -> Result<bool, OrchestrationError> {
    let task = run
        .tasks
        .get(&reservation.task_id)
        .expect("reservation task was validated");
    let existing = scope_base_required(run, Some(&reservation.task_id))?;
    let grants = grants_for_attempt(run, &reservation.attempt_id, at_ms)?;
    let required = checked_add_amount(existing, amount_saturating_sub(reservation.amount, grants))?;
    Ok(amount_within(required, task.budget_limit))
}

fn reservation_settled_for_attempt(run: &Run, attempt_id: &RunAttemptId) -> bool {
    run.reservations
        .values()
        .find(|reservation| reservation.attempt_id == *attempt_id)
        .is_some_and(|reservation| {
            matches!(
                reservation.state,
                BudgetReservationState::Reconciled | BudgetReservationState::Released
            )
        })
}

fn valid_usage_update(reservation: &BudgetReservation, update: BudgetUsage) -> bool {
    let compatible = |existing: Option<u64>, incoming: Option<u64>| match (existing, incoming) {
        (Some(left), Some(right)) => left == right,
        (Some(_), None) => false,
        (None, _) => true,
    };
    if !compatible(reservation.actual.tokens, update.tokens)
        || !compatible(reservation.actual.cost_micro_usd, update.cost_micro_usd)
        || !compatible(reservation.actual.runtime_ms, update.runtime_ms)
    {
        return false;
    }
    reservation.state == BudgetReservationState::Reserved
        || (reservation.actual.tokens.is_none() && update.tokens.is_some())
        || (reservation.actual.cost_micro_usd.is_none() && update.cost_micro_usd.is_some())
        || (reservation.actual.runtime_ms.is_none() && update.runtime_ms.is_some())
}

fn usage_delta(existing: BudgetUsage, update: BudgetUsage) -> BudgetAmount {
    BudgetAmount {
        tokens: if existing.tokens.is_none() {
            update.tokens.unwrap_or_default()
        } else {
            0
        },
        cost_micro_usd: if existing.cost_micro_usd.is_none() {
            update.cost_micro_usd.unwrap_or_default()
        } else {
            0
        },
        runtime_ms: if existing.runtime_ms.is_none() {
            update.runtime_ms.unwrap_or_default()
        } else {
            0
        },
    }
}

fn reservation_release_for_update(
    reservation: &BudgetReservation,
    update: BudgetUsage,
) -> BudgetAmount {
    BudgetAmount {
        tokens: if reservation.actual.tokens.is_none() && update.tokens.is_some() {
            reservation.amount.tokens
        } else {
            0
        },
        cost_micro_usd: if reservation.actual.cost_micro_usd.is_none()
            && update.cost_micro_usd.is_some()
        {
            reservation.amount.cost_micro_usd
        } else {
            0
        },
        runtime_ms: if reservation.actual.runtime_ms.is_none() && update.runtime_ms.is_some() {
            reservation.amount.runtime_ms
        } else {
            0
        },
    }
}

fn merge_usage(existing: BudgetUsage, update: BudgetUsage) -> BudgetUsage {
    BudgetUsage {
        tokens: existing.tokens.or(update.tokens),
        cost_micro_usd: existing.cost_micro_usd.or(update.cost_micro_usd),
        runtime_ms: existing.runtime_ms.or(update.runtime_ms),
    }
}

fn usage_complete(usage: BudgetUsage) -> bool {
    usage.tokens.is_some() && usage.cost_micro_usd.is_some() && usage.runtime_ms.is_some()
}

fn budget_incomplete(incomplete: BudgetIncomplete) -> bool {
    incomplete.tokens || incomplete.cost_micro_usd || incomplete.runtime_ms
}

fn amount_nonzero(amount: BudgetAmount) -> bool {
    amount.tokens != 0 || amount.cost_micro_usd != 0 || amount.runtime_ms != 0
}

fn recompute_budget_state(run: &mut Run) -> Result<(), OrchestrationError> {
    run.budget.incomplete = BudgetIncomplete {
        tokens: run.reservations.values().any(|reservation| {
            reservation.state == BudgetReservationState::PartiallyReconciled
                && reservation.actual.tokens.is_none()
        }),
        cost_micro_usd: run.reservations.values().any(|reservation| {
            reservation.state == BudgetReservationState::PartiallyReconciled
                && reservation.actual.cost_micro_usd.is_none()
        }),
        runtime_ms: run.reservations.values().any(|reservation| {
            reservation.state == BudgetReservationState::PartiallyReconciled
                && reservation.actual.runtime_ms.is_none()
        }),
    };
    let committed = scope_base_required(run, None)?;
    let run_overage = BudgetAmount {
        tokens: committed.tokens.saturating_sub(run.budget.limit.tokens),
        cost_micro_usd: committed
            .cost_micro_usd
            .saturating_sub(run.budget.limit.cost_micro_usd),
        runtime_ms: committed
            .runtime_ms
            .saturating_sub(run.budget.limit.runtime_ms),
    };
    let task_overage =
        run.tasks
            .values()
            .try_fold(BudgetAmount::default(), |total_overage, task| {
                let committed = scope_base_required(run, Some(&task.id))?;
                checked_add_amount(
                    total_overage,
                    BudgetAmount {
                        tokens: committed.tokens.saturating_sub(task.budget_limit.tokens),
                        cost_micro_usd: committed
                            .cost_micro_usd
                            .saturating_sub(task.budget_limit.cost_micro_usd),
                        runtime_ms: committed
                            .runtime_ms
                            .saturating_sub(task.budget_limit.runtime_ms),
                    },
                )
            })?;
    run.budget.overage = BudgetAmount {
        tokens: run_overage.tokens.max(task_overage.tokens),
        cost_micro_usd: run_overage.cost_micro_usd.max(task_overage.cost_micro_usd),
        runtime_ms: run_overage.runtime_ms.max(task_overage.runtime_ms),
    };
    if (budget_incomplete(run.budget.incomplete) || amount_nonzero(run.budget.overage))
        && !matches!(run.status, RunStatus::Cancelling)
        && !run.status.is_terminal()
    {
        run.status = RunStatus::Paused;
    }
    Ok(())
}

fn reservation_committed_amount(reservation: &BudgetReservation) -> BudgetAmount {
    let retained = matches!(
        reservation.state,
        BudgetReservationState::Reserved | BudgetReservationState::PartiallyReconciled
    );
    BudgetAmount {
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
    }
}

fn scope_base_required(
    run: &Run,
    task_id: Option<&RunTaskId>,
) -> Result<BudgetAmount, OrchestrationError> {
    run.reservations
        .values()
        .filter(|reservation| task_id.is_none_or(|task_id| reservation.task_id == *task_id))
        .try_fold(BudgetAmount::default(), |total, reservation| {
            let committed = reservation_committed_amount(reservation);
            let grants = grants_for_attempt(run, &reservation.attempt_id, None)?;
            checked_add_amount(total, amount_saturating_sub(committed, grants))
        })
}

fn grants_for_attempt(
    run: &Run,
    attempt_id: &RunAttemptId,
    at_ms: Option<u64>,
) -> Result<BudgetAmount, OrchestrationError> {
    run.budget_grants
        .values()
        .filter(|grant| {
            grant.attempt_id == *attempt_id && at_ms.is_none_or(|at| grant.expires_at_ms > at)
        })
        .try_fold(BudgetAmount::default(), |mut total, grant| {
            add_budget_dimension(&mut total, grant.dimension, grant.amount)?;
            Ok(total)
        })
}

fn amount_saturating_sub(left: BudgetAmount, right: BudgetAmount) -> BudgetAmount {
    BudgetAmount {
        tokens: left.tokens.saturating_sub(right.tokens),
        cost_micro_usd: left.cost_micro_usd.saturating_sub(right.cost_micro_usd),
        runtime_ms: left.runtime_ms.saturating_sub(right.runtime_ms),
    }
}

fn add_budget_dimension(
    amount: &mut BudgetAmount,
    dimension: BudgetDimension,
    value: u64,
) -> Result<(), OrchestrationError> {
    let target = match dimension {
        BudgetDimension::Tokens => &mut amount.tokens,
        BudgetDimension::CostMicroUsd => &mut amount.cost_micro_usd,
        BudgetDimension::RuntimeMs => &mut amount.runtime_ms,
    };
    *target = target.checked_add(value).ok_or_else(budget_overflow)?;
    Ok(())
}

fn refresh_retry_after_budget(run: &mut Run, task_id: &RunTaskId) {
    if run.status == RunStatus::Paused
        || run
            .tasks
            .get(task_id)
            .is_none_or(|task| task.budget_reservation_id.is_some())
    {
        return;
    }
    let latest = run
        .attempts
        .values()
        .filter(|attempt| attempt.task_id == *task_id)
        .max_by_key(|attempt| attempt.ordinal);
    let Some(attempt) = latest else {
        return;
    };
    let completed_attempts = u32::try_from(
        run.attempts
            .values()
            .filter(|candidate| candidate.task_id == *task_id)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let task = run.tasks.get_mut(task_id).expect("task exists");
    if matches!(
        attempt.status,
        AttemptStatus::Failed | AttemptStatus::Interrupted
    ) && fallback_eligibility(
        attempt.boundary,
        attempt.ambiguous_effect,
        completed_attempts,
        task.max_attempts,
    ) == FallbackEligibility::Allowed
    {
        task.status = RunTaskStatus::Ready;
    }
}

fn valid_utc_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return false;
    }
    let parse = |range: std::ops::Range<usize>| {
        value[range]
            .parse::<u32>()
            .expect("ASCII date digits fit u32")
    };
    let year = parse(0..4);
    let month = parse(5..7);
    let day = parse(8..10);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

fn valid_reason_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
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
