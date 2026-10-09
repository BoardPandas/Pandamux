use crate::store::{
    EffectClaimOutcome, EffectClaimRequest, EffectDispatchOutcome, EffectDispatchRequest,
    EffectOutcomeRecord, EffectOutcomeRequest, EffectOwnership, EffectPreparationOutcome,
    OrchestrationExecutionFailpoint, OrchestrationReplayAudit, RunTransition, RunTransitionRequest,
    Store, StoreError, StoredOrchestrationEffect,
};
use pandamux_core::{OrchestrationEffectId, RunEventId, RunId, RunStatus};
use tokio::sync::broadcast;

const RUN_PROJECTION_BROADCAST_CAPACITY: usize = 256;

/// A bounded, payload-free projection hint emitted only after a durable run
/// transition is observable. Consumers must load canonical state from SQLite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunTransitionNotice {
    pub run_id: RunId,
    pub last_seq: u64,
    pub snapshot_sha256: String,
    pub status: RunStatus,
    pub event_ids: Vec<RunEventId>,
    pub effect_ids: Vec<OrchestrationEffectId>,
    pub duplicate: bool,
}

/// The sole server-side adapter between the pure orchestration engine and
/// durable effect delivery. Provider, filesystem, and node execution are added
/// behind this owner in later Phase 5C checkpoints.
#[derive(Clone)]
pub struct RunExecutionService {
    store: Store,
    projections: broadcast::Sender<RunTransitionNotice>,
}

impl RunExecutionService {
    pub fn new(store: Store) -> Self {
        let (projections, _) = broadcast::channel(RUN_PROJECTION_BROADCAST_CAPACITY);
        Self { store, projections }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RunTransitionNotice> {
        self.projections.subscribe()
    }

    pub fn commit_transition(
        &self,
        request: &RunTransitionRequest,
        failpoint: Option<OrchestrationExecutionFailpoint>,
    ) -> Result<RunTransition, StoreError> {
        let transition = self.store.commit_run_transition(request, failpoint)?;
        let notice = RunTransitionNotice {
            run_id: transition.run.id.clone(),
            last_seq: transition.run.last_seq,
            snapshot_sha256: transition.snapshot_sha256.clone(),
            status: transition.run.status,
            event_ids: transition
                .events
                .iter()
                .map(|event| event.event_id.clone())
                .collect(),
            effect_ids: transition.effect_ids.clone(),
            duplicate: transition.duplicate,
        };
        let _ = self.projections.send(notice);
        Ok(transition)
    }

    pub fn audit_replay(&self, run_id: &RunId) -> Result<OrchestrationReplayAudit, StoreError> {
        self.store.audit_orchestration_replay(run_id)
    }

    pub fn pending_effects(
        &self,
        limit: u32,
    ) -> Result<Vec<StoredOrchestrationEffect>, StoreError> {
        self.store.list_pending_orchestration_effects(limit)
    }

    pub fn claim_effect(
        &self,
        request: &EffectClaimRequest,
        failpoint: Option<OrchestrationExecutionFailpoint>,
    ) -> Result<EffectClaimOutcome, StoreError> {
        self.store.claim_orchestration_effect(request, failpoint)
    }

    pub fn mark_effect_prepared(
        &self,
        ownership: &EffectOwnership,
        prepared_receipt_digest: &str,
        prepared_at_ms: u64,
    ) -> Result<EffectPreparationOutcome, StoreError> {
        self.store.mark_orchestration_effect_prepared(
            ownership,
            prepared_receipt_digest,
            prepared_at_ms,
        )
    }

    pub fn mark_effect_dispatch_started(
        &self,
        request: &EffectDispatchRequest,
        failpoint: Option<OrchestrationExecutionFailpoint>,
    ) -> Result<EffectDispatchOutcome, StoreError> {
        self.store
            .mark_orchestration_effect_dispatch_started(request, failpoint)
    }

    pub fn record_effect_outcome(
        &self,
        request: &EffectOutcomeRequest,
        failpoint: Option<OrchestrationExecutionFailpoint>,
    ) -> Result<EffectOutcomeRecord, StoreError> {
        self.store
            .record_orchestration_effect_outcome(request, failpoint)
    }
}
