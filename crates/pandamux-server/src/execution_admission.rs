use crate::store::{
    AdmissionCapacityReconciliation, AdmissionClaimStoreOutcome, AdmissionClaimStoreRequest,
    AdmissionOwnership, AdmissionPolicySnapshot, AdmissionRenewStoreRequest,
    AdmissionRequeueStoreRequest, BudgetExecutionFailpoint, OrchestratedAdmissionTransition,
    OrchestrationQueueRecord, QueueState, Store, StoreError,
};
use pandamux_core::{
    AgentId, EnvironmentId, ExecutionLease, ExecutionLeaseId, LeaseResourceKind, OrganizationId,
    ProviderInstanceId, RunAttemptId, RunId, RunTaskId, ServerIncarnationId, ThreadId, TurnId,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_EXECUTION_ADMISSION_TTL_MS: u64 = 60_000;
pub const DEFAULT_EXECUTION_ADMISSION_RENEWAL_MS: u64 = 20_000;
pub const MIN_EXECUTION_ADMISSION_TTL_MS: u64 = 1_000;
pub const MAX_EXECUTION_ADMISSION_TTL_MS: u64 = 5 * 60_000;
pub const BASE_EXECUTION_RETRY_DELAY_MS: u64 = 250;
pub const MAX_EXECUTION_RETRY_DELAY_MS: u64 = 30_000;

#[derive(Debug)]
pub enum ExecutionAdmissionError {
    Store(StoreError),
    Worker(String),
}

impl std::fmt::Display for ExecutionAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::Worker(error) => write!(f, "execution admission worker failed: {error}"),
        }
    }
}

impl std::error::Error for ExecutionAdmissionError {}

impl From<StoreError> for ExecutionAdmissionError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnqueueManualAdmissionRequest {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub agent_id: Option<AgentId>,
    pub organization_id: Option<OrganizationId>,
    pub provider_instance_id: ProviderInstanceId,
    pub environment_id: EnvironmentId,
    pub enqueued_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnqueueOrchestratedAdmissionRequest {
    pub queue_id: String,
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub attempt_id: RunAttemptId,
    pub agent_id: AgentId,
    pub organization_id: OrganizationId,
    pub agent_concurrency_limit: u32,
    pub provider_instance_id: ProviderInstanceId,
    pub environment_id: EnvironmentId,
    pub priority: i64,
    pub task_enqueued_at_ms: u64,
    pub enqueued_at_ms: u64,
    pub not_before_ms: u64,
}

/// A node-side orchestration candidate whose authority is the durable private
/// attempt-prepare receipt rather than a duplicated Hub run snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnqueueRemoteOrchestratedAdmissionRequest {
    pub queue_id: String,
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub attempt_id: RunAttemptId,
    pub agent_id: AgentId,
    pub organization_id: OrganizationId,
    pub agent_concurrency_limit: u32,
    pub provider_instance_id: ProviderInstanceId,
    pub environment_id: EnvironmentId,
    pub enqueued_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionAdmissionClaimRequest {
    pub queue_id: String,
    pub expected_candidate_sha256: String,
    pub expected_claim_revision: u64,
    pub lease_id: ExecutionLeaseId,
    pub claimed_at_ms: u64,
    pub ttl_ms: u64,
    pub orchestration: Option<OrchestratedAdmissionTransition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionAdmissionClaim {
    pub record: OrchestrationQueueRecord,
    pub lease: ExecutionLease,
    pub acquired: bool,
}

impl ExecutionAdmissionClaim {
    pub fn ownership(&self) -> AdmissionOwnership {
        AdmissionOwnership {
            queue_id: self.record.queue_id.clone(),
            expected_candidate_sha256: self.record.candidate_sha256.clone(),
            owner_incarnation_id: self.lease.owner_incarnation_id.clone(),
            lease_id: self.lease.id.clone(),
            claim_revision: self.record.claim_revision,
        }
    }
}

#[derive(Clone)]
pub struct ExecutionAdmissionService {
    store: Store,
    owner_incarnation_id: ServerIncarnationId,
}

impl ExecutionAdmissionService {
    pub fn new(store: Store) -> Self {
        Self::with_owner(store, ServerIncarnationId::generate())
    }

    pub fn with_owner(store: Store, owner_incarnation_id: ServerIncarnationId) -> Self {
        Self {
            store,
            owner_incarnation_id,
        }
    }

    pub fn owner_incarnation_id(&self) -> &ServerIncarnationId {
        &self.owner_incarnation_id
    }

    pub async fn enqueue_manual(
        &self,
        request: EnqueueManualAdmissionRequest,
    ) -> Result<OrchestrationQueueRecord, ExecutionAdmissionError> {
        let record = manual_queue_record(request);
        self.insert_record(record).await
    }

    pub async fn enqueue_orchestrated(
        &self,
        request: EnqueueOrchestratedAdmissionRequest,
    ) -> Result<OrchestrationQueueRecord, ExecutionAdmissionError> {
        let record = orchestrated_queue_record(request)?;
        self.insert_record(record).await
    }

    pub async fn enqueue_remote_orchestrated(
        &self,
        request: EnqueueRemoteOrchestratedAdmissionRequest,
    ) -> Result<OrchestrationQueueRecord, ExecutionAdmissionError> {
        let record = remote_orchestrated_queue_record(request)?;
        self.insert_record(record).await
    }

    pub async fn try_claim(
        &self,
        request: ExecutionAdmissionClaimRequest,
    ) -> Result<Option<ExecutionAdmissionClaim>, ExecutionAdmissionError> {
        if !(MIN_EXECUTION_ADMISSION_TTL_MS..=MAX_EXECUTION_ADMISSION_TTL_MS)
            .contains(&request.ttl_ms)
        {
            return Err(ExecutionAdmissionError::Store(StoreError::Data(
                "execution admission TTL is outside its bounded range".to_string(),
            )));
        }
        let expires_at_ms = request
            .claimed_at_ms
            .checked_add(request.ttl_ms)
            .ok_or_else(|| {
                ExecutionAdmissionError::Store(StoreError::Data(
                    "execution admission lease expiry overflow".to_string(),
                ))
            })?;
        let policy = self.policy_snapshot().await?;
        let store_request = AdmissionClaimStoreRequest {
            queue_id: request.queue_id,
            expected_candidate_sha256: request.expected_candidate_sha256,
            expected_claim_revision: request.expected_claim_revision,
            owner_incarnation_id: self.owner_incarnation_id.clone(),
            lease_id: request.lease_id,
            claimed_at_ms: request.claimed_at_ms,
            lease_expires_at_ms: expires_at_ms,
            policy,
            orchestration: request.orchestration,
        };
        let store = self.store.clone();
        let outcome =
            run_blocking(move || store.try_claim_execution_admission(&store_request)).await?;
        Ok(outcome.map(claim_from_store))
    }

    pub async fn try_claim_record(
        &self,
        record: &OrchestrationQueueRecord,
        claimed_at_ms: u64,
        orchestration: Option<OrchestratedAdmissionTransition>,
    ) -> Result<Option<ExecutionAdmissionClaim>, ExecutionAdmissionError> {
        self.try_claim(ExecutionAdmissionClaimRequest {
            queue_id: record.queue_id.clone(),
            expected_candidate_sha256: record.candidate_sha256.clone(),
            expected_claim_revision: record.claim_revision,
            lease_id: ExecutionLeaseId::generate(),
            claimed_at_ms,
            ttl_ms: DEFAULT_EXECUTION_ADMISSION_TTL_MS,
            orchestration,
        })
        .await
    }

    pub async fn admit_manual_immediately(
        &self,
        request: EnqueueManualAdmissionRequest,
    ) -> Result<Option<ExecutionAdmissionClaim>, ExecutionAdmissionError> {
        let at_ms = request.enqueued_at_ms;
        let record = self.enqueue_manual(request).await?;
        match self.try_claim_record(&record, at_ms, None).await? {
            Some(claim) => Ok(Some(claim)),
            None => {
                self.cancel_queued(&record).await?;
                Ok(None)
            }
        }
    }

    pub async fn mark_dispatched(
        &self,
        claim: &ExecutionAdmissionClaim,
    ) -> Result<ExecutionAdmissionClaim, ExecutionAdmissionError> {
        let ownership = claim.ownership();
        let store = self.store.clone();
        let record =
            run_blocking(move || store.mark_execution_admission_dispatched(&ownership)).await?;
        Ok(ExecutionAdmissionClaim {
            record,
            lease: claim.lease.clone(),
            acquired: false,
        })
    }

    /// Reconstructs the exact still-owned capacity claim for one orchestrated
    /// attempt. Recovery and remote Hub dispatch use this instead of bypassing
    /// the shared admission owner with ad-hoc queue updates.
    pub async fn load_owned_attempt_claim(
        &self,
        run_id: &RunId,
        attempt_id: &RunAttemptId,
    ) -> Result<ExecutionAdmissionClaim, ExecutionAdmissionError> {
        let store = self.store.clone();
        let run_id = run_id.clone();
        let attempt_id = attempt_id.clone();
        run_blocking(move || {
            let runtime = store.load_orchestration_runtime_records(&run_id)?;
            let mut queues = runtime
                .queue_records
                .into_iter()
                .filter(|record| record.attempt_id == attempt_id);
            let record = queues.next().ok_or_else(|| {
                StoreError::Data("execution admission attempt owner was not found".to_string())
            })?;
            if queues.next().is_some()
                || record.run_id != run_id
                || !matches!(record.state, QueueState::Claimed | QueueState::Dispatched)
            {
                return Err(StoreError::Data(
                    "execution admission attempt owner is not unique and active".to_string(),
                ));
            }
            let owner = record.owner_incarnation_id.as_ref().ok_or_else(|| {
                StoreError::Data("execution admission attempt owner is missing".to_string())
            })?;
            let lease_id = record.lease_id.as_ref().ok_or_else(|| {
                StoreError::Data("execution admission attempt lease is missing".to_string())
            })?;
            let mut leases = runtime
                .leases
                .into_iter()
                .filter(|lease| lease.id == *lease_id);
            let lease = leases.next().ok_or_else(|| {
                StoreError::Data("execution admission attempt lease was not found".to_string())
            })?;
            if leases.next().is_some()
                || lease.resource_kind != LeaseResourceKind::Capacity
                || lease.resource_id != record.queue_id
                || lease.owner_incarnation_id != *owner
                || lease.revision != record.claim_revision
            {
                return Err(StoreError::Data(
                    "execution admission attempt lease identity changed".to_string(),
                ));
            }
            Ok(ExecutionAdmissionClaim {
                record,
                lease,
                acquired: false,
            })
        })
        .await
    }

    pub async fn renew(
        &self,
        claim: &ExecutionAdmissionClaim,
        renewed_at_ms: u64,
        ttl_ms: u64,
    ) -> Result<ExecutionAdmissionClaim, ExecutionAdmissionError> {
        if !(MIN_EXECUTION_ADMISSION_TTL_MS..=MAX_EXECUTION_ADMISSION_TTL_MS).contains(&ttl_ms) {
            return Err(ExecutionAdmissionError::Store(StoreError::Data(
                "execution admission TTL is outside its bounded range".to_string(),
            )));
        }
        let lease_expires_at_ms = renewed_at_ms.checked_add(ttl_ms).ok_or_else(|| {
            ExecutionAdmissionError::Store(StoreError::Data(
                "execution admission lease expiry overflow".to_string(),
            ))
        })?;
        let request = AdmissionRenewStoreRequest {
            ownership: claim.ownership(),
            renewed_at_ms,
            lease_expires_at_ms,
        };
        let store = self.store.clone();
        let outcome = run_blocking(move || store.renew_execution_admission(&request)).await?;
        Ok(claim_from_store(outcome))
    }

    pub async fn requeue_expired(
        &self,
        claim: &ExecutionAdmissionClaim,
        reclaimed_at_ms: u64,
        retry_ordinal: u32,
    ) -> Result<OrchestrationQueueRecord, ExecutionAdmissionError> {
        let not_before_ms = reclaimed_at_ms
            .checked_add(bounded_execution_retry_delay_ms(retry_ordinal))
            .ok_or_else(|| {
                ExecutionAdmissionError::Store(StoreError::Data(
                    "execution admission retry timestamp overflow".to_string(),
                ))
            })?;
        let request = AdmissionRequeueStoreRequest {
            ownership: claim.ownership(),
            reclaimed_at_ms,
            not_before_ms,
        };
        let store = self.store.clone();
        run_blocking(move || store.requeue_expired_execution_admission(&request)).await
    }

    pub async fn release(
        &self,
        claim: &ExecutionAdmissionClaim,
    ) -> Result<OrchestrationQueueRecord, ExecutionAdmissionError> {
        let ownership = claim.ownership();
        let store = self.store.clone();
        run_blocking(move || store.release_execution_admission(&ownership)).await
    }

    pub async fn release_at(
        &self,
        claim: &ExecutionAdmissionClaim,
        released_at_ms: u64,
        failpoint: Option<BudgetExecutionFailpoint>,
    ) -> Result<OrchestrationQueueRecord, ExecutionAdmissionError> {
        let ownership = claim.ownership();
        let store = self.store.clone();
        run_blocking(move || {
            store.release_execution_admission_with_budget(&ownership, released_at_ms, failpoint)
        })
        .await
    }

    pub async fn cancel_queued(
        &self,
        record: &OrchestrationQueueRecord,
    ) -> Result<OrchestrationQueueRecord, ExecutionAdmissionError> {
        let store = self.store.clone();
        let queue_id = record.queue_id.clone();
        let candidate = record.candidate_sha256.clone();
        run_blocking(move || store.cancel_queued_execution_admission(&queue_id, &candidate)).await
    }

    pub async fn reconcile_capacity(
        &self,
    ) -> Result<AdmissionCapacityReconciliation, ExecutionAdmissionError> {
        let policy = self.policy_snapshot().await?;
        let store = self.store.clone();
        run_blocking(move || store.reconcile_execution_capacity(&policy)).await
    }

    async fn insert_record(
        &self,
        record: OrchestrationQueueRecord,
    ) -> Result<OrchestrationQueueRecord, ExecutionAdmissionError> {
        let store = self.store.clone();
        run_blocking(move || store.insert_execution_queue_record(&record)).await
    }

    async fn policy_snapshot(&self) -> Result<AdmissionPolicySnapshot, ExecutionAdmissionError> {
        let store = self.store.clone();
        run_blocking(move || {
            let settings_record = store.get_settings_record()?;
            let records = store.list_execution_queue_records()?;
            let (settings, settings_revision) = settings_record.map_or_else(
                || (pandamux_core::UserSettings::default(), None),
                |record| (record.settings, Some(record.revision.get())),
            );
            let global_limit = settings.advanced.global_concurrency_limit.max(1);
            let mut environment_limits = settings
                .environments
                .iter()
                .map(|environment| {
                    (
                        environment.id.clone(),
                        environment.execution_limit.max(1).min(global_limit),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let required_environments = records
                .iter()
                .map(|record| record.environment_id.clone())
                .collect::<BTreeSet<_>>();
            for environment_id in required_environments {
                environment_limits.entry(environment_id).or_insert_with(|| {
                    pandamux_core::DEFAULT_ENVIRONMENT_EXECUTION_LIMIT.min(global_limit)
                });
            }
            let mut provider_limits = BTreeMap::new();
            for record in &records {
                let key = (
                    record.environment_id.clone(),
                    record.provider_instance_id.clone(),
                );
                if provider_limits.contains_key(&key) {
                    continue;
                }
                let Some(provider) = settings.providers.iter().find(|provider| {
                    provider.id == record.provider_instance_id
                        && provider
                            .environment_id
                            .as_ref()
                            .is_none_or(|environment| environment == &record.environment_id)
                }) else {
                    continue;
                };
                let environment_override = settings
                    .environments
                    .iter()
                    .find(|environment| environment.id == record.environment_id)
                    .and_then(|environment| {
                        environment
                            .provider_overrides
                            .get(record.provider_instance_id.as_str())
                    });
                let resolved =
                    provider.resolve_for_environment(&record.environment_id, environment_override);
                if resolved.enabled && resolved.concurrency_limit > 0 {
                    provider_limits.insert(key, resolved.concurrency_limit);
                }
            }
            let organization_limits = settings
                .organizations
                .into_iter()
                .filter_map(|organization| {
                    organization
                        .max_concurrency
                        .map(|limit| (organization.id, limit))
                })
                .collect();
            Ok(AdmissionPolicySnapshot {
                settings_revision,
                global_limit,
                environment_limits,
                provider_limits,
                organization_limits,
            })
        })
        .await
    }
}

pub fn bounded_execution_retry_delay_ms(retry_ordinal: u32) -> u64 {
    let shift = retry_ordinal.saturating_sub(1).min(16);
    BASE_EXECUTION_RETRY_DELAY_MS
        .saturating_mul(1_u64 << shift)
        .min(MAX_EXECUTION_RETRY_DELAY_MS)
}

fn manual_queue_record(request: EnqueueManualAdmissionRequest) -> OrchestrationQueueRecord {
    let identity = admission_digest(&[
        "manual",
        request.thread_id.as_str(),
        request.turn_id.as_str(),
        request.provider_instance_id.as_str(),
        request.environment_id.as_str(),
    ]);
    let queue_id = format!("manual-admission-{identity}");
    let run_id = RunId::new(format!("manual-run-{}", request.thread_id.as_str()));
    let task_id = RunTaskId::new(format!("manual-task-{}", request.turn_id.as_str()));
    let attempt_id = RunAttemptId::new(format!("manual-attempt-{identity}"));
    let agent_id = request
        .agent_id
        .unwrap_or_else(|| AgentId::new("manual-interactive"));
    let candidate_sha256 = admission_digest(&[
        "interactive",
        &queue_id,
        run_id.as_str(),
        task_id.as_str(),
        attempt_id.as_str(),
        agent_id.as_str(),
        request
            .organization_id
            .as_ref()
            .map(OrganizationId::as_str)
            .unwrap_or(""),
        request.provider_instance_id.as_str(),
        request.environment_id.as_str(),
        &request.enqueued_at_ms.to_string(),
    ]);
    OrchestrationQueueRecord {
        queue_id,
        run_id,
        task_id,
        attempt_id,
        agent_id,
        organization_id: request.organization_id,
        agent_concurrency_limit: None,
        provider_instance_id: request.provider_instance_id,
        environment_id: request.environment_id,
        candidate_sha256,
        priority: 0,
        task_enqueued_at_ms: request.enqueued_at_ms,
        enqueued_at_ms: request.enqueued_at_ms,
        not_before_ms: request.enqueued_at_ms,
        state: QueueState::Queued,
        owner_incarnation_id: None,
        lease_id: None,
        claim_revision: 0,
        interactive: true,
        claimed_at_ms: None,
        claim_order: None,
        interactive_streak: None,
    }
}

fn orchestrated_queue_record(
    request: EnqueueOrchestratedAdmissionRequest,
) -> Result<OrchestrationQueueRecord, ExecutionAdmissionError> {
    if request.queue_id.trim().is_empty()
        || request.agent_concurrency_limit == 0
        || request.enqueued_at_ms < request.task_enqueued_at_ms
        || request.not_before_ms < request.enqueued_at_ms
    {
        return Err(ExecutionAdmissionError::Store(StoreError::Data(
            "orchestrated execution admission request is malformed".to_string(),
        )));
    }
    let candidate_sha256 = admission_digest(&[
        "orchestrated",
        &request.queue_id,
        request.run_id.as_str(),
        request.task_id.as_str(),
        request.attempt_id.as_str(),
        request.agent_id.as_str(),
        request.organization_id.as_str(),
        &request.agent_concurrency_limit.to_string(),
        request.provider_instance_id.as_str(),
        request.environment_id.as_str(),
        &request.priority.to_string(),
        &request.task_enqueued_at_ms.to_string(),
        &request.enqueued_at_ms.to_string(),
        &request.not_before_ms.to_string(),
    ]);
    Ok(OrchestrationQueueRecord {
        queue_id: request.queue_id,
        run_id: request.run_id,
        task_id: request.task_id,
        attempt_id: request.attempt_id,
        agent_id: request.agent_id,
        organization_id: Some(request.organization_id),
        agent_concurrency_limit: Some(request.agent_concurrency_limit),
        provider_instance_id: request.provider_instance_id,
        environment_id: request.environment_id,
        candidate_sha256,
        priority: request.priority,
        task_enqueued_at_ms: request.task_enqueued_at_ms,
        enqueued_at_ms: request.enqueued_at_ms,
        not_before_ms: request.not_before_ms,
        state: QueueState::Queued,
        owner_incarnation_id: None,
        lease_id: None,
        claim_revision: 0,
        interactive: false,
        claimed_at_ms: None,
        claim_order: None,
        interactive_streak: None,
    })
}

fn remote_orchestrated_queue_record(
    request: EnqueueRemoteOrchestratedAdmissionRequest,
) -> Result<OrchestrationQueueRecord, ExecutionAdmissionError> {
    if request.queue_id.trim().is_empty() || request.agent_concurrency_limit == 0 {
        return Err(ExecutionAdmissionError::Store(StoreError::Data(
            "remote orchestrated execution admission request is malformed".to_string(),
        )));
    }
    let candidate_sha256 = admission_digest(&[
        "remote_orchestrated",
        &request.queue_id,
        request.run_id.as_str(),
        request.task_id.as_str(),
        request.attempt_id.as_str(),
        request.agent_id.as_str(),
        request.organization_id.as_str(),
        &request.agent_concurrency_limit.to_string(),
        request.provider_instance_id.as_str(),
        request.environment_id.as_str(),
        &request.enqueued_at_ms.to_string(),
    ]);
    Ok(OrchestrationQueueRecord {
        queue_id: request.queue_id,
        run_id: request.run_id,
        task_id: request.task_id,
        attempt_id: request.attempt_id,
        agent_id: request.agent_id,
        organization_id: Some(request.organization_id),
        agent_concurrency_limit: Some(request.agent_concurrency_limit),
        provider_instance_id: request.provider_instance_id,
        environment_id: request.environment_id,
        candidate_sha256,
        priority: 0,
        task_enqueued_at_ms: request.enqueued_at_ms,
        enqueued_at_ms: request.enqueued_at_ms,
        not_before_ms: request.enqueued_at_ms,
        state: QueueState::Queued,
        owner_incarnation_id: None,
        lease_id: None,
        claim_revision: 0,
        interactive: false,
        claimed_at_ms: None,
        claim_order: None,
        interactive_streak: None,
    })
}

fn admission_digest(fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn claim_from_store(outcome: AdmissionClaimStoreOutcome) -> ExecutionAdmissionClaim {
    ExecutionAdmissionClaim {
        record: outcome.record,
        lease: outcome.lease,
        acquired: outcome.acquired,
    }
}

async fn run_blocking<T, F>(operation: F) -> Result<T, ExecutionAdmissionError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, StoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| ExecutionAdmissionError::Worker(error.to_string()))?
        .map_err(ExecutionAdmissionError::Store)
}
