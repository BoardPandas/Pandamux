use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, broadcast, mpsc};

use pandamux_core::{
    AttemptBoundary, AttemptExecutionEvidence, CAPACITY_SNAPSHOT_SCHEMA_VERSION,
    CapacityConfidence, CapacityEvidenceScope, CapacityFreshness, CapacityInterruptionEvidence,
    CapacitySnapshot, ContinuationDelivery, ContinuationDeliveryKind, ContinuationDeliveryState,
    ContinuationId, ContinuationRecord, ContinuationState, DEFAULT_CAPACITY_MAX_AGE_MS,
    EnvironmentId, ProjectId, ProviderInstanceId, ProviderKind, RoutingAuthentication,
    RoutingCandidate, RoutingDecision, RoutingEvaluation, RoutingHealth, RoutingScore,
    SubscriptionKey, ToolLedgerEntry, ToolLedgerStatus, TurnAttempt, TurnAttemptId,
    TurnAttemptStatus,
    event::{ThreadEvent, ThreadEventKind, ToolCallStatus, TurnOutcome},
    ids::{ThreadId, TurnId},
    project_capacity_revisions, select_route,
    thread::{Thread, ThreadStatus, ThreadWorkspace, Turn, TurnInput, TurnStatus},
};
use pandamux_protocol::{
    EventEnvelope, GitCommitParams, GitCommitResult, GitCreatePrParams, GitCreatePrResult,
    GitPushParams, GitPushResult, GitStatusParams, GitStatusResult, HubRoutingContext, RpcError,
    ThreadCancelTurnParams, ThreadContinueParams, ThreadContinueResult, ThreadCreateParams,
    ThreadGetParams, ThreadGetResult, ThreadListParams, ThreadRespondApprovalParams,
    ThreadResumeParams, ThreadResumeResult, ThreadSendTurnParams, ThreadSendTurnResult,
};
use pandamux_providers::traits::ProviderSession;
use pandamux_providers::{
    ProviderAuthStatus, ProviderEvent, ProviderHealth, ProviderSnapshot, SessionSpec,
};

use crate::provider_resolver::{ProviderResolver, ResolvedProviderExecution};
use crate::request_context::CapabilityAuthority;
use crate::store::{ProviderExecutionAudit, Store};
use crate::usage_scanner::{UsageScanTrigger, UsageScannerHandle};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn organization_policy_error(project_id: &ProjectId, message: impl Into<String>) -> RpcError {
    RpcError::with_data(
        -32030,
        message,
        serde_json::json!({
            "reason": "organization_policy_denied",
            "projectId": project_id,
        }),
    )
}

fn record_local_codex_capacity(
    store: &Store,
    environment_id: &EnvironmentId,
    provider_instance_id: &pandamux_core::ProviderInstanceId,
    provider_version: Option<&str>,
    details: &serde_json::Value,
    observed_at_ms: u64,
) -> Result<(), crate::store::StoreError> {
    let Some((subscription_key, _)) =
        store.usage_subscription_for_provider(environment_id, provider_instance_id)?
    else {
        return Ok(());
    };
    let limits = pandamux_providers::parse_rate_limits_value(details).map_err(|error| {
        crate::store::StoreError::Data(format!("invalid Codex rate-limit event: {error}"))
    })?;
    let snapshots = limits
        .into_iter()
        .map(|limit| {
            let duration_ms = u64::from(limit.window_duration_mins).saturating_mul(60 * 1000);
            CapacitySnapshot {
                schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION,
                subscription_key: subscription_key.clone(),
                environment_id: Some(environment_id.clone()),
                provider_instance_id: Some(provider_instance_id.clone()),
                window_name: Some(limit.limit_name),
                window_start_ms: limit
                    .resets_at_ms
                    .map(|reset| reset.saturating_sub(duration_ms)),
                window_end_ms: limit.resets_at_ms,
                used_millionths: limit.used_millionths,
                remaining_millionths: limit.remaining_millionths,
                reset_at_ms: limit.resets_at_ms,
                source: "codex_rate_limit_event".to_string(),
                confidence: CapacityConfidence::Reported,
                evidence_scope: CapacityEvidenceScope::Local,
                hard_limit: limit.hard_limit
                    && limit.remaining_millionths == Some(0)
                    && limit.resets_at_ms.is_some(),
                provider_version: provider_version.map(str::to_string),
                as_of_ms: observed_at_ms,
                max_age_ms: DEFAULT_CAPACITY_MAX_AGE_MS,
            }
        })
        .collect::<Vec<_>>();
    store.save_capacity_snapshots(&snapshots)
}

/// An active supervised session paired with its event channel receiver.
#[derive(Clone)]
pub struct ActiveThreadSession {
    pub session: Arc<Mutex<Option<Box<dyn ProviderSession>>>>,
    pub events_rx: Arc<Mutex<Option<mpsc::Receiver<ProviderEvent>>>>,
    pub event_writer: Arc<Mutex<()>>,
    pub execution: Arc<ResolvedProviderExecution>,
    pub generation: uuid::Uuid,
    pub child_capability_token: String,
    pub capability_authority: Arc<CapabilityAuthority>,
}

async fn shutdown_active_session(active: ActiveThreadSession, interrupt: bool) {
    let session = {
        let mut session = active.session.lock().await;
        session.take()
    };
    if let Some(mut session) = session {
        if interrupt {
            let _ = session.interrupt().await;
        }
        session.shutdown().await;
    }
    active
        .capability_authority
        .revoke(&active.child_capability_token);
}

async fn remove_active_generation(
    active_sessions: &Arc<Mutex<HashMap<ThreadId, ActiveThreadSession>>>,
    thread_id: &ThreadId,
    generation: uuid::Uuid,
) -> Option<ActiveThreadSession> {
    let mut active = active_sessions.lock().await;
    if active
        .get(thread_id)
        .is_some_and(|session| session.generation == generation)
    {
        active.remove(thread_id)
    } else {
        None
    }
}

/// Central manager orchestrating thread execution, streaming, and state persistence.
#[derive(Clone)]
pub struct ThreadManager {
    store: Store,
    provider_resolver: Arc<ProviderResolver>,
    capability_authority: Arc<CapabilityAuthority>,
    environment_id: String,
    active_sessions: Arc<Mutex<HashMap<ThreadId, ActiveThreadSession>>>,
    continuation_dispatch: Arc<Mutex<()>>,
    broadcaster: broadcast::Sender<EventEnvelope>,
    usage_scanner: Arc<std::sync::RwLock<Option<UsageScannerHandle>>>,
}

#[derive(Clone, Debug)]
struct SelectedTurnRoute {
    decision: RoutingDecision,
    provider_instance_id: ProviderInstanceId,
    subscription_key: Option<SubscriptionKey>,
    model: String,
}

#[derive(Clone, Copy, Debug)]
struct CandidateCapacity {
    confidence: CapacityConfidence,
    freshness: CapacityFreshness,
    remaining_millionths: Option<u32>,
    hard_limited: bool,
}

fn routing_authentication(snapshot: &ProviderSnapshot) -> RoutingAuthentication {
    match snapshot.auth {
        ProviderAuthStatus::Authenticated { .. } => RoutingAuthentication::Authenticated,
        ProviderAuthStatus::Configured => RoutingAuthentication::Configured,
        ProviderAuthStatus::Unauthenticated => RoutingAuthentication::Unauthenticated,
        ProviderAuthStatus::Unknown => RoutingAuthentication::Unknown,
    }
}

fn routing_health(snapshot: &ProviderSnapshot) -> RoutingHealth {
    match &snapshot.health {
        ProviderHealth::Healthy => RoutingHealth::Healthy,
        ProviderHealth::Degraded { reason } => RoutingHealth::Degraded {
            reason: reason.clone(),
        },
        ProviderHealth::Unavailable { reason } => RoutingHealth::Unavailable {
            reason: reason.clone(),
        },
    }
}

fn capacity_for_candidate(
    snapshots: &[CapacitySnapshot],
    subscription_key: &SubscriptionKey,
    provider_instance_id: &ProviderInstanceId,
    at_ms: u64,
) -> CandidateCapacity {
    let relevant = snapshots
        .iter()
        .filter(|snapshot| {
            &snapshot.subscription_key == subscription_key
                && snapshot
                    .provider_instance_id
                    .as_ref()
                    .is_none_or(|provider| provider == provider_instance_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    let projections = project_capacity_revisions(&relevant, at_ms, 60_000);
    let selected = projections.iter().min_by_key(|projection| {
        (
            !projection.hard_limited,
            projection.remaining_millionths.unwrap_or(u32::MAX),
            capacity_freshness_rank(projection.freshness),
            capacity_confidence_rank(projection.confidence),
        )
    });
    selected.map_or(
        CandidateCapacity {
            confidence: CapacityConfidence::Unknown,
            freshness: CapacityFreshness::Stale,
            remaining_millionths: None,
            hard_limited: false,
        },
        |projection| CandidateCapacity {
            confidence: projection.confidence,
            freshness: projection.freshness,
            remaining_millionths: projection.remaining_millionths,
            hard_limited: projection.hard_limited,
        },
    )
}

fn capacity_freshness_rank(freshness: CapacityFreshness) -> u8 {
    match freshness {
        CapacityFreshness::Fresh => 0,
        CapacityFreshness::Aging => 1,
        CapacityFreshness::Stale => 2,
        CapacityFreshness::Reset => 3,
        CapacityFreshness::ClockSkew => 4,
        CapacityFreshness::HardLimited => 5,
    }
}

fn capacity_confidence_rank(confidence: CapacityConfidence) -> u8 {
    match confidence {
        CapacityConfidence::Reported => 0,
        CapacityConfidence::Estimated => 1,
        CapacityConfidence::Unknown => 2,
    }
}

fn bounded_capacity_handoff(
    turn: &Turn,
    attempt: &TurnAttempt,
    ledger: &[ToolLedgerEntry],
) -> String {
    const MAX_HANDOFF_BYTES: usize = 16 * 1024;
    const TRUNCATION_MARKER: &str = "\n[handoff truncated]";
    let ledger_json = serde_json::to_string(ledger).unwrap_or_else(|_| "[]".to_string());
    let checkpoint = attempt.checkpoint.as_deref().unwrap_or("unavailable");
    let mut handoff = format!(
        "Continue the interrupted PandaMUX turn in a fresh provider session.\n\nOriginal request:\n{}\n\nSaved checkpoint:\n{}\n\nPartial provider output:\n{}\n\nDurable tool ledger:\n{}\n\nDo not repeat completed or started side effects. Inspect the checkpoint and ledger before acting.",
        turn.input.text, checkpoint, attempt.partial_output, ledger_json
    );
    if handoff.len() > MAX_HANDOFF_BYTES {
        let mut boundary = MAX_HANDOFF_BYTES.saturating_sub(TRUNCATION_MARKER.len());
        while !handoff.is_char_boundary(boundary) {
            boundary = boundary.saturating_sub(1);
        }
        handoff.truncate(boundary);
        handoff.push_str(TRUNCATION_MARKER);
    }
    handoff
}

fn ensure_attempt_boundary(
    store: &Store,
    attempt_id: &TurnAttemptId,
    required: AttemptBoundary,
) -> Result<TurnAttempt, crate::store::StoreError> {
    let attempt = store.get_turn_attempt(attempt_id)?.ok_or_else(|| {
        crate::store::StoreError::Data(format!("turn attempt {attempt_id} does not exist"))
    })?;
    if attempt.boundary >= required {
        Ok(attempt)
    } else {
        store.advance_turn_attempt_boundary(attempt_id, required)
    }
}

fn replay_approval_delivery(
    existing: &ContinuationDelivery,
    params: &ThreadRespondApprovalParams,
    payload_sha256: &str,
    payload_json: &str,
) -> Result<(), RpcError> {
    if existing.thread_id != params.thread_id
        || existing.kind != ContinuationDeliveryKind::Approval
        || existing.item_id != params.request_id
        || existing.payload_sha256 != payload_sha256
        || existing.payload_json != payload_json
    {
        return Err(RpcError::invalid_params(
            "Approval delivery identity already represents another payload",
        ));
    }
    match existing.state {
        ContinuationDeliveryState::Delivered => Ok(()),
        ContinuationDeliveryState::Pending => Err(RpcError::with_data(
            -32022,
            "Approval delivery is already pending and will not be dispatched twice",
            serde_json::json!({ "reason": "approval_delivery_pending" }),
        )),
    }
}

impl ThreadManager {
    pub fn new(
        store: Store,
        provider_resolver: Arc<ProviderResolver>,
        environment_id: String,
        capability_authority: Arc<CapabilityAuthority>,
    ) -> Self {
        // Reconcile any in-flight turns left unfinalized from a prior server run
        let _ = store.reconcile_active_turns();

        let (broadcaster, _) = broadcast::channel(512);

        Self {
            store,
            provider_resolver,
            capability_authority,
            environment_id,
            active_sessions: Arc::new(Mutex::new(HashMap::new())),
            continuation_dispatch: Arc::new(Mutex::new(())),
            broadcaster,
            usage_scanner: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    pub fn set_usage_scanner(&self, scanner: Option<UsageScannerHandle>) {
        *self
            .usage_scanner
            .write()
            .expect("usage scanner handle lock") = scanner;
    }

    /// Subscribes to the broadcast stream of thread event envelopes.
    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.broadcaster.subscribe()
    }

    /// Access the raw broadcast channel sender for event relaying.
    pub fn broadcaster(&self) -> &broadcast::Sender<EventEnvelope> {
        &self.broadcaster
    }

    pub async fn shutdown_all(&self) {
        let sessions = {
            let mut active = self.active_sessions.lock().await;
            active
                .drain()
                .map(|(_, session)| session)
                .collect::<Vec<_>>()
        };
        for session in sessions {
            shutdown_active_session(session, true).await;
        }
    }

    async fn select_turn_route(
        &self,
        thread: &Thread,
        requested_model: Option<&str>,
        excluded: &BTreeSet<ProviderInstanceId>,
        pinned_provider: Option<&ProviderInstanceId>,
        hub_routing_context: Option<&HubRoutingContext>,
        use_local_policy: bool,
    ) -> Result<SelectedTurnRoute, RpcError> {
        let Some(project_id) = thread.project_id.as_ref() else {
            return self.select_explicit_route(thread, requested_model, excluded, pinned_provider);
        };
        let (policy, pools) = if let Some(context) = hub_routing_context {
            context
                .organization
                .validate()
                .map_err(|error| organization_policy_error(project_id, error.to_string()))?;
            if !context.organization.project_ids.contains(project_id) {
                return Err(organization_policy_error(
                    project_id,
                    "Hub routing context does not authorize this project",
                ));
            }
            let mut pool_ids = BTreeSet::new();
            for pool in &context.failover_pools {
                pool.validate()
                    .map_err(|error| organization_policy_error(project_id, error.to_string()))?;
                if !pool_ids.insert(pool.id.clone()) {
                    return Err(organization_policy_error(
                        project_id,
                        "Hub routing context contains a duplicate failover pool",
                    ));
                }
            }
            let pools = context
                .organization
                .failover_pool_ids
                .iter()
                .map(|pool_id| {
                    context
                        .failover_pools
                        .iter()
                        .find(|pool| &pool.id == pool_id)
                        .cloned()
                        .ok_or_else(|| {
                            organization_policy_error(
                                project_id,
                                "Hub routing context is missing a referenced failover pool",
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            for pool in &pools {
                for member in &pool.members {
                    context
                        .organization
                        .authorize_route(&member.provider_instance_id, &member.subscription_key)
                        .map_err(|error| {
                            organization_policy_error(project_id, error.to_string())
                        })?;
                }
            }
            (context.organization.clone(), pools)
        } else if use_local_policy {
            let policy = self
                .store
                .organization_policy_for_project(project_id)
                .map_err(|error| organization_policy_error(project_id, error.to_string()))?;
            let pools = policy
                .failover_pool_ids
                .iter()
                .map(|pool_id| {
                    self.store
                        .get_failover_pool(pool_id)
                        .map_err(|error| organization_policy_error(project_id, error.to_string()))?
                        .ok_or_else(|| {
                            organization_policy_error(
                                project_id,
                                "organization references a missing failover pool",
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            (policy, pools)
        } else {
            return Err(organization_policy_error(
                project_id,
                "Remote project routing requires a canonical Hub policy snapshot",
            ));
        };
        let Some(pool) = pools
            .iter()
            .find(|pool| {
                pool.members
                    .iter()
                    .any(|member| member.provider_instance_id == thread.provider_instance_id)
            })
            .or_else(|| pools.first())
        else {
            return self.select_explicit_route(thread, requested_model, excluded, pinned_provider);
        };

        let mut requirements = pool.requirements;
        if thread.access_mode.security_rank() > requirements.access_mode.security_rank() {
            requirements.access_mode = thread.access_mode;
        }
        let settings = self
            .store
            .get_settings()
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .unwrap_or_default();
        let capacity_snapshots = if settings.advanced.capacity_routing_enabled {
            self.store
                .latest_capacity_snapshots_for_environment(&thread.environment_id, 512)
                .map_err(|error| RpcError::internal_error(error.to_string()))?
        } else {
            Vec::new()
        };
        let mut candidates = Vec::with_capacity(pool.members.len());
        for member in &pool.members {
            let mut local_hard_blocks = Vec::new();
            if excluded.contains(&member.provider_instance_id) {
                local_hard_blocks.push("provider_already_attempted".to_string());
            }
            if pinned_provider.is_some_and(|pinned| pinned != &member.provider_instance_id) {
                local_hard_blocks.push("not_requested_continuation_target".to_string());
            }
            let policy_allowed = policy
                .allowed_provider_instances
                .contains(&member.provider_instance_id);
            let subscription_allowed = policy
                .allowed_subscriptions
                .contains(&member.subscription_key);
            let config_kind = settings
                .providers
                .iter()
                .find(|provider| provider.id == member.provider_instance_id)
                .map(|provider| provider.provider)
                .unwrap_or(ProviderKind::Custom);
            let resolved = self
                .provider_resolver
                .resolve(&member.provider_instance_id, &thread.environment_id);
            let (provider_kind, capabilities, model_compatible, installed, authentication, health) =
                match resolved {
                    Ok(resolved) => {
                        if resolved.capabilities != member.declared_capabilities {
                            local_hard_blocks.push("capability_declaration_drift".to_string());
                        }
                        let mut declared_models = resolved
                            .supported_models
                            .iter()
                            .map(|model| model.id.as_str())
                            .collect::<Vec<_>>();
                        if let Some(default_model) = resolved
                            .config
                            .settings
                            .get("defaultModel")
                            .and_then(serde_json::Value::as_str)
                        {
                            declared_models.push(default_model);
                        }
                        let model_compatible = declared_models
                            .into_iter()
                            .any(|model| model == member.model);
                        let snapshot = resolved.driver.probe(&resolved.config).await;
                        let authentication = routing_authentication(&snapshot);
                        let health = routing_health(&snapshot);
                        (
                            resolved.config.provider,
                            resolved.capabilities,
                            model_compatible,
                            snapshot.installed,
                            authentication,
                            health,
                        )
                    }
                    Err(_) => (
                        config_kind,
                        member.declared_capabilities,
                        false,
                        false,
                        RoutingAuthentication::Unknown,
                        RoutingHealth::Unavailable {
                            reason: "provider resolution failed".to_string(),
                        },
                    ),
                };
            let capacity = if settings.advanced.capacity_routing_enabled {
                capacity_for_candidate(
                    &capacity_snapshots,
                    &member.subscription_key,
                    &member.provider_instance_id,
                    now_ms(),
                )
            } else {
                CandidateCapacity {
                    confidence: CapacityConfidence::Unknown,
                    freshness: CapacityFreshness::Stale,
                    remaining_millionths: None,
                    hard_limited: false,
                }
            };
            candidates.push(RoutingCandidate {
                provider_instance_id: member.provider_instance_id.clone(),
                subscription_key: member.subscription_key.clone(),
                provider_kind,
                model: member.model.clone(),
                capabilities,
                policy_allowed,
                subscription_allowed,
                model_compatible,
                installed,
                authentication,
                health,
                capacity_confidence: capacity.confidence,
                capacity_freshness: capacity.freshness,
                remaining_millionths: capacity.remaining_millionths,
                hard_limited: capacity.hard_limited,
                active_concurrency: u32::try_from(
                    self.provider_resolver
                        .active_processes(&member.provider_instance_id, &thread.environment_id),
                )
                .unwrap_or(u32::MAX),
                concurrency_limit: settings
                    .providers
                    .iter()
                    .find(|provider| provider.id == member.provider_instance_id)
                    .map(|provider| provider.concurrency_limit)
                    .unwrap_or(0),
                local: &thread.environment_id == self.provider_resolver.local_environment_id(),
                user_preference: member.user_preference,
                local_hard_blocks,
            });
        }
        let decision = select_route(requirements, candidates);
        let provider_instance_id =
            decision
                .selected_provider_instance_id
                .clone()
                .ok_or_else(|| {
                    RpcError::with_data(
                        -32021,
                        "No failover-pool member passed every hard routing filter",
                        serde_json::json!({
                            "reason": "no_eligible_failover_route",
                            "routing": &decision,
                        }),
                    )
                })?;
        Ok(SelectedTurnRoute {
            subscription_key: decision.selected_subscription_key.clone(),
            model: decision
                .selected_model
                .clone()
                .expect("selected failover route includes a model"),
            provider_instance_id,
            decision,
        })
    }

    fn select_explicit_route(
        &self,
        thread: &Thread,
        requested_model: Option<&str>,
        excluded: &BTreeSet<ProviderInstanceId>,
        pinned_provider: Option<&ProviderInstanceId>,
    ) -> Result<SelectedTurnRoute, RpcError> {
        if excluded.contains(&thread.provider_instance_id)
            || pinned_provider.is_some_and(|pinned| pinned != &thread.provider_instance_id)
        {
            return Err(RpcError::with_data(
                -32021,
                "No alternate provider is available outside an explicit failover pool",
                serde_json::json!({ "reason": "no_eligible_failover_route" }),
            ));
        }
        let resolved = self
            .provider_resolver
            .resolve(&thread.provider_instance_id, &thread.environment_id)?;
        if !resolved
            .capabilities
            .supports_access_mode(thread.access_mode)
        {
            return Err(RpcError::invalid_params(
                "The selected provider does not declare the thread access ceiling",
            ));
        }
        let model = requested_model.unwrap_or(&thread.model).to_string();
        let subscription_key = self
            .store
            .usage_subscription_for_provider(&thread.environment_id, &thread.provider_instance_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .map(|(key, _)| key);
        let evaluations = subscription_key
            .as_ref()
            .map(|key| {
                vec![RoutingEvaluation {
                    provider_instance_id: thread.provider_instance_id.clone(),
                    subscription_key: key.clone(),
                    model: model.clone(),
                    eligible: true,
                    rejection_codes: Vec::new(),
                    score: Some(RoutingScore::default()),
                    explanation: "selected explicit provider outside a failover pool".to_string(),
                }]
            })
            .unwrap_or_default();
        Ok(SelectedTurnRoute {
            decision: RoutingDecision {
                selected_provider_instance_id: Some(thread.provider_instance_id.clone()),
                selected_subscription_key: subscription_key.clone(),
                selected_model: Some(model.clone()),
                evaluations,
            },
            provider_instance_id: thread.provider_instance_id.clone(),
            subscription_key,
            model,
        })
    }

    fn persist_capacity_interruption(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        attempt_id: &TurnAttemptId,
        evidence: CapacityInterruptionEvidence,
    ) -> Result<ContinuationId, RpcError> {
        let at = evidence.observed_at_ms;
        let mut attempt = self
            .store
            .get_turn_attempt(attempt_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .ok_or_else(|| RpcError::internal_error("turn attempt disappeared"))?;
        let mut turn = self
            .store
            .get_turn(turn_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .ok_or_else(|| RpcError::internal_error("turn disappeared"))?;
        let mut thread = self
            .store
            .get_thread(thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .ok_or_else(|| RpcError::internal_error("thread disappeared"))?;
        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        let checkpoint = crate::checkpoint::create_checkpoint(
            &worktree_dir,
            thread_id,
            turn_id,
            crate::checkpoint::CheckpointPhase::After,
        )
        .unwrap_or(None)
        .or_else(|| attempt.checkpoint.clone());
        attempt.status = TurnAttemptStatus::InterruptedForCapacity;
        attempt.capacity_evidence = Some(evidence.clone());
        attempt.checkpoint = checkpoint.clone();
        attempt.ended_at_ms = Some(at);
        self.store
            .save_turn_attempt(&attempt)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;

        turn.status = TurnStatus::InterruptedForCapacity;
        turn.checkpoint_after = checkpoint.clone();
        turn.ended_at_ms = Some(at);
        self.store
            .save_turn(&turn)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        thread.status = ThreadStatus::Idle;
        thread.updated_at_ms = at;
        self.store
            .save_thread(&thread)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;

        let ledger = self
            .store
            .list_tool_ledger(turn_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        let handoff = bounded_capacity_handoff(&turn, &attempt, &ledger);
        let continuation_id = ContinuationId::new(format!(
            "capacity-{}-{}",
            turn_id.as_str(),
            attempt_id.as_str()
        ));
        self.store
            .save_continuation(&ContinuationRecord {
                id: continuation_id.clone(),
                idempotency_key: format!("capacity:{}:{}", turn_id.as_str(), attempt_id.as_str()),
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                source_attempt_id: attempt_id.clone(),
                target_provider_instance_id: None,
                target_turn_id: None,
                state: ContinuationState::Offered,
                handoff,
                checkpoint,
                created_at_ms: at,
                updated_at_ms: at,
            })
            .map_err(|error| RpcError::internal_error(error.to_string()))?;

        let seq = self
            .store
            .get_latest_seq(thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .checked_add(1)
            .ok_or_else(|| RpcError::internal_error("Thread event sequence overflow"))?;
        let event = ThreadEvent {
            thread_id: thread_id.clone(),
            seq,
            at_ms: at,
            kind: ThreadEventKind::CapacityInterrupted {
                turn_id: turn_id.clone(),
                attempt_id: attempt_id.clone(),
                continuation_id: continuation_id.clone(),
                provider_instance_id: evidence.provider_instance_id,
                message: "Capacity was exhausted after work became unsafe to replay. An audited continuation is available on another eligible provider."
                    .to_string(),
            },
        };
        self.store
            .append_event(&event)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        let _ = self.broadcaster.send(EventEnvelope {
            subscription_id: String::new(),
            environment_id: thread.environment_id,
            thread_id: Some(thread_id.clone()),
            run_id: None,
            seq,
            at_ms: at,
            kind: event.kind,
        });
        Ok(continuation_id)
    }

    async fn get_or_start_session_with_resume(
        &self,
        thread: &Thread,
        allow_resume: bool,
    ) -> Result<ActiveThreadSession, RpcError> {
        if !allow_resume {
            self.store
                .delete_resume_token(&thread.id)
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
        }
        let resolved = self
            .provider_resolver
            .resolve(&thread.provider_instance_id, &thread.environment_id)?;
        let existing = {
            let active = self.active_sessions.lock().await;
            active.get(&thread.id).cloned()
        };
        if let Some(existing) = existing {
            if allow_resume && existing.execution.instance.config == resolved.config {
                return Ok(existing);
            }
            if let Some(stale) =
                remove_active_generation(&self.active_sessions, &thread.id, existing.generation)
                    .await
            {
                shutdown_active_session(stale, false).await;
            }
        }

        let execution = Arc::new(
            self.provider_resolver
                .acquire_execution(&thread.provider_instance_id, &thread.environment_id)
                .await?,
        );
        let resume_token = if allow_resume {
            self.store
                .get_resume_token(&thread.id)
                .map_err(|error| RpcError::internal_error(error.to_string()))?
        } else {
            None
        };
        let spec = SessionSpec {
            cwd: PathBuf::from(thread.workspace.effective_path()),
            model: thread.model.clone(),
            effort: thread.effort.clone(),
            access: thread.access_mode,
            instructions: None,
            tool_policy: None,
            resume: resume_token,
        };
        let child_capability_token = self.capability_authority.issue_provider_child(
            thread.id.clone(),
            [
                "notification.post".to_string(),
                "memory.list".to_string(),
                "memory.search".to_string(),
                "memory.add".to_string(),
                "memory.update".to_string(),
            ],
            24 * 60 * 60 * 1000,
        );
        let mut launch_config = execution.instance.config.clone();
        launch_config.env_overrides.insert(
            "PANDAMUX_CAPABILITY".to_string(),
            child_capability_token.clone(),
        );
        launch_config.env_overrides.insert(
            "PANDAMUX_THREAD_ID".to_string(),
            thread.id.as_str().to_string(),
        );
        if let Some(endpoint) = self.capability_authority.endpoint() {
            launch_config
                .env_overrides
                .insert("PANDAMUX_PIPE".to_string(), endpoint);
        }
        let start_result = execution
            .instance
            .driver
            .start_session(&launch_config, spec)
            .await;
        let mut session = match start_result {
            Ok(session) => session,
            Err(error) => {
                self.capability_authority.revoke(&child_capability_token);
                return Err(RpcError::internal_error(error.to_string()));
            }
        };
        let events_rx = session.take_event_receiver();
        let candidate = ActiveThreadSession {
            session: Arc::new(Mutex::new(Some(session))),
            events_rx: Arc::new(Mutex::new(events_rx)),
            event_writer: Arc::new(Mutex::new(())),
            execution,
            generation: uuid::Uuid::new_v4(),
            child_capability_token,
            capability_authority: self.capability_authority.clone(),
        };

        let raced = {
            let mut active = self.active_sessions.lock().await;
            if let Some(existing) = active.get(&thread.id) {
                Some(existing.clone())
            } else {
                active.insert(thread.id.clone(), candidate.clone());
                None
            }
        };
        if let Some(existing) = raced {
            shutdown_active_session(candidate, false).await;
            Ok(existing)
        } else {
            Ok(candidate)
        }
    }

    /// Creates and persists a new thread with optional git worktree.
    pub fn create_thread(&self, params: ThreadCreateParams) -> Result<Thread, RpcError> {
        self.create_thread_with_policy(params, true)
    }

    /// Creates a thread after an authenticated Hub has already applied the
    /// canonical organization policy and pinned the provider instance.
    pub fn create_thread_from_hub(&self, params: ThreadCreateParams) -> Result<Thread, RpcError> {
        self.create_thread_with_policy(params, false)
    }

    fn create_thread_with_policy(
        &self,
        params: ThreadCreateParams,
        enforce_organization_policy: bool,
    ) -> Result<Thread, RpcError> {
        let now = now_ms();
        let default_cwd = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let cwd = params.cwd.unwrap_or(default_cwd);
        let workspace = ThreadWorkspace {
            cwd,
            worktree: params.worktree,
        };

        let environment_id = params
            .environment_id
            .unwrap_or_else(|| EnvironmentId::from(self.environment_id.as_str()));
        let provider_instance_id = if enforce_organization_policy
            && let Some(project_id) = params.project_id.as_ref()
        {
            let policy = self
                .store
                .organization_policy_for_project(project_id)
                .map_err(|error| organization_policy_error(project_id, error.to_string()))?;
            match params.provider_instance_id {
                Some(instance_id) => {
                    self.store
                        .authorize_provider_for_project(project_id, &environment_id, &instance_id)
                        .map_err(|error| {
                            organization_policy_error(project_id, error.to_string())
                        })?;
                    instance_id
                }
                None => {
                    let mut selected = None;
                    for instance_id in &policy.allowed_provider_instances {
                        if self
                            .store
                            .authorize_provider_for_project(
                                project_id,
                                &environment_id,
                                instance_id,
                            )
                            .is_ok()
                            && self
                                .provider_resolver
                                .resolve(instance_id, &environment_id)
                                .is_ok()
                        {
                            selected = Some(instance_id.clone());
                            break;
                        }
                    }
                    selected.ok_or_else(|| {
                        organization_policy_error(
                            project_id,
                            "organization has no enabled provider with an allowed named subscription",
                        )
                    })?
                }
            }
        } else {
            match params.provider_instance_id {
                Some(instance_id) => instance_id,
                None => self
                    .provider_resolver
                    .default_instance_id(&environment_id)?,
            }
        };
        self.provider_resolver
            .resolve(&provider_instance_id, &environment_id)?;

        let thread = Thread {
            id: ThreadId::generate(),
            project_id: params.project_id,
            environment_id,
            parent_thread_id: None,
            title: params.title.unwrap_or_else(|| "New Thread".to_string()),
            provider_instance_id,
            model: params.model.unwrap_or_else(|| "default".to_string()),
            effort: params.effort,
            access_mode: params.access_mode.unwrap_or_default(),
            workspace,
            status: ThreadStatus::Idle,
            agent: None,
            origin: Default::default(),
            created_at_ms: now,
            updated_at_ms: now,
        };

        self.store
            .save_thread(&thread)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        Ok(thread)
    }

    /// Retrieves a thread and its turn history.
    pub fn get_thread(&self, params: ThreadGetParams) -> Result<ThreadGetResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| {
                RpcError::not_found(format!("Thread not found: {}", params.thread_id))
            })?;

        let turns = self
            .store
            .list_turns(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let attempts = turns
            .iter()
            .map(|turn| self.store.list_turn_attempts(&turn.id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .into_iter()
            .flatten()
            .collect();
        let continuations = self
            .store
            .list_continuations(&params.thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;

        Ok(ThreadGetResult {
            thread,
            turns,
            attempts,
            continuations,
        })
    }

    /// Lists threads optionally filtered by project or status.
    pub fn list_threads(&self, params: ThreadListParams) -> Result<Vec<Thread>, RpcError> {
        let threads = self
            .store
            .list_threads()
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let filtered = threads
            .into_iter()
            .filter(|t| {
                if let Some(ref p) = params.project_id
                    && t.project_id.as_ref() != Some(p)
                {
                    return false;
                }
                if let Some(st) = params.status
                    && t.status != st
                {
                    return false;
                }
                if let Some(ref parent) = params.parent_thread_id
                    && t.parent_thread_id.as_ref() != Some(parent)
                {
                    return false;
                }
                true
            })
            .collect();

        Ok(filtered)
    }

    /// Dispatches a prompt turn to an agent provider and streams events into SQLite.
    pub async fn send_turn(
        &self,
        mut params: ThreadSendTurnParams,
    ) -> Result<ThreadSendTurnResult, RpcError> {
        params.hub_routing_context = None;
        self.send_turn_with_policy(params, true, None, true, None, None)
            .await
    }

    /// Dispatches a turn after an authenticated Hub has revalidated the
    /// mirrored thread against its canonical organization policy.
    pub async fn send_turn_from_hub(
        &self,
        mut params: ThreadSendTurnParams,
    ) -> Result<ThreadSendTurnResult, RpcError> {
        let hub_routing_context = params.hub_routing_context.take();
        self.send_turn_with_policy(params, false, None, true, hub_routing_context, None)
            .await
    }

    async fn send_turn_with_policy(
        &self,
        params: ThreadSendTurnParams,
        enforce_organization_policy: bool,
        pinned_provider: Option<&ProviderInstanceId>,
        allow_native_resume: bool,
        hub_routing_context: Option<HubRoutingContext>,
        continuation_id: Option<&ContinuationId>,
    ) -> Result<ThreadSendTurnResult, RpcError> {
        let mut thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| {
                RpcError::not_found(format!("Thread not found: {}", params.thread_id))
            })?;

        if thread.status == ThreadStatus::Working {
            return Err(RpcError::invalid_params(
                "Thread is currently executing another turn",
            ));
        }
        if thread.status == ThreadStatus::AwaitingApproval {
            return Err(RpcError::invalid_params(
                "Thread is awaiting approval response",
            ));
        }
        if enforce_organization_policy && let Some(project_id) = thread.project_id.as_ref() {
            self.store
                .authorize_provider_for_project(
                    project_id,
                    &thread.environment_id,
                    &thread.provider_instance_id,
                )
                .map_err(|error| organization_policy_error(project_id, error.to_string()))?;
        }

        let turn_id = TurnId::generate();
        let base_seq = self
            .store
            .get_latest_seq(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let turn_seq = base_seq + 1;
        let started_ms = now_ms();

        // Resolve and validate attachments if provided
        let resolved_attachments = if !params.attachment_ids.is_empty() {
            let records = self
                .store
                .get_attachments_by_ids(&params.thread_id, &params.attachment_ids)
                .map_err(|e| RpcError::internal_error(e.to_string()))?;

            for id in &params.attachment_ids {
                if !records.iter().any(|r| &r.id == id) {
                    return Err(RpcError::invalid_params(format!(
                        "Attachment '{id}' not found for thread '{}'",
                        params.thread_id.as_str()
                    )));
                }
            }

            pandamux_core::validate_total_attachments_size(&records)
                .map_err(|e| RpcError::invalid_params(e.to_string()))?;

            records
        } else {
            Vec::new()
        };

        let previous_provider_instance_id = thread.provider_instance_id.clone();
        let route = self
            .select_turn_route(
                &thread,
                params.model.as_deref(),
                &BTreeSet::new(),
                pinned_provider,
                hub_routing_context.as_ref(),
                enforce_organization_policy,
            )
            .await?;
        let provider_changed = previous_provider_instance_id != route.provider_instance_id;
        thread.provider_instance_id = route.provider_instance_id.clone();
        thread.model = route.model.clone();

        let turn_input = TurnInput {
            text: params.text.clone(),
            attachment_ids: params.attachment_ids.clone(),
            attachments: resolved_attachments,
            model: Some(route.model.clone()),
            effort: params.effort.clone().or_else(|| thread.effort.clone()),
        };

        let effective_path = PathBuf::from(thread.workspace.effective_path());
        let cp_before = crate::checkpoint::create_checkpoint(
            &effective_path,
            &params.thread_id,
            &turn_id,
            crate::checkpoint::CheckpointPhase::Before,
        )
        .unwrap_or(None);

        let active_sess = self
            .get_or_start_session_with_resume(&thread, allow_native_resume && !provider_changed)
            .await?;

        let turn = Turn {
            id: turn_id.clone(),
            thread_id: params.thread_id.clone(),
            environment_id: thread.environment_id.clone(),
            provider_instance_id: thread.provider_instance_id.clone(),
            seq: turn_seq,
            input: turn_input.clone(),
            status: TurnStatus::Running,
            started_at_ms: started_ms,
            ended_at_ms: None,
            usage: None,
            checkpoint_before: cp_before,
            checkpoint_after: None,
        };

        self.store
            .save_turn(&turn)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;
        let attempt_id = TurnAttemptId::generate();
        self.store
            .save_turn_attempt(&TurnAttempt {
                id: attempt_id.clone(),
                turn_id: turn_id.clone(),
                thread_id: params.thread_id.clone(),
                ordinal: 1,
                environment_id: thread.environment_id.clone(),
                provider_instance_id: route.provider_instance_id.clone(),
                subscription_key: route.subscription_key.clone(),
                model: route.model.clone(),
                boundary: AttemptBoundary::SafeToRetry,
                status: TurnAttemptStatus::Running,
                routing: route.decision.clone(),
                execution: Some(AttemptExecutionEvidence {
                    provider_kind: active_sess.execution.instance.config.provider,
                    profile_dir: active_sess.execution.instance.config.profile_dir.clone(),
                    binary_path: active_sess.execution.instance.config.binary_path.clone(),
                    capabilities: active_sess.execution.instance.capabilities,
                    provider_version: active_sess.execution.snapshot.version.clone(),
                }),
                checkpoint: turn.checkpoint_before.clone(),
                partial_output: String::new(),
                capacity_evidence: None,
                started_at_ms: started_ms,
                ended_at_ms: None,
            })
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        let execution_config = &active_sess.execution.instance.config;
        self.store
            .record_provider_execution(&ProviderExecutionAudit {
                turn_id: turn_id.clone(),
                thread_id: params.thread_id.clone(),
                environment_id: thread.environment_id.clone(),
                provider_instance_id: execution_config.id.clone(),
                provider_kind: execution_config.provider,
                profile_dir: execution_config.profile_dir.clone(),
                binary_path: execution_config.binary_path.clone(),
                capabilities: active_sess.execution.instance.capabilities,
                model: turn_input
                    .model
                    .clone()
                    .unwrap_or_else(|| thread.model.clone()),
                created_at_ms: started_ms,
            })
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        thread.status = ThreadStatus::Working;
        thread.updated_at_ms = started_ms;
        self.store
            .save_thread(&thread)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        // Emit TurnRequested event
        let req_event = ThreadEvent {
            thread_id: params.thread_id.clone(),
            seq: turn_seq,
            at_ms: started_ms,
            kind: ThreadEventKind::TurnRequested {
                turn_id: turn_id.clone(),
                input: turn_input.clone(),
            },
        };
        self.store
            .append_event(&req_event)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let _ = self.broadcaster.send(EventEnvelope {
            subscription_id: String::new(),
            environment_id: self.environment_id.clone().into(),
            thread_id: Some(params.thread_id.clone()),
            run_id: None,
            seq: turn_seq,
            at_ms: started_ms,
            kind: req_event.kind,
        });

        // Emit TurnStarted event
        let started_seq = turn_seq + 1;
        let start_event = ThreadEvent {
            thread_id: params.thread_id.clone(),
            seq: started_seq,
            at_ms: started_ms,
            kind: ThreadEventKind::TurnStarted {
                turn_id: turn_id.clone(),
            },
        };
        self.store
            .append_event(&start_event)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let _ = self.broadcaster.send(EventEnvelope {
            subscription_id: String::new(),
            environment_id: self.environment_id.clone().into(),
            thread_id: Some(params.thread_id.clone()),
            run_id: None,
            seq: started_seq,
            at_ms: started_ms,
            kind: start_event.kind,
        });

        if let Some(continuation_id) = continuation_id {
            self.store
                .link_continuation_target_turn(continuation_id, &turn_id, now_ms())
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
        }

        // Send turn prompt into provider session
        let send_result = {
            let mut sess = active_sess.session.lock().await;
            sess.as_mut()
                .ok_or_else(|| RpcError::internal_error("Provider session is closed"))?
                .send_turn(turn_input.clone())
                .await
                .map_err(|e| RpcError::internal_error(e.to_string()))
        };
        if let Err(error) = send_result {
            if let Some(active) = remove_active_generation(
                &self.active_sessions,
                &params.thread_id,
                active_sess.generation,
            )
            .await
            {
                shutdown_active_session(active, false).await;
            }
            let failed_at = now_ms();
            if let Ok(Some(mut failed_turn)) = self.store.get_turn(&turn_id) {
                failed_turn.status = TurnStatus::Failed;
                failed_turn.ended_at_ms = Some(failed_at);
                let _ = self.store.save_turn(&failed_turn);
            }
            if let Ok(Some(mut failed_attempt)) = self.store.get_turn_attempt(&attempt_id) {
                failed_attempt.status = TurnAttemptStatus::Failed;
                failed_attempt.ended_at_ms = Some(failed_at);
                let _ = self.store.save_turn_attempt(&failed_attempt);
            }
            thread.status = ThreadStatus::Errored;
            thread.updated_at_ms = failed_at;
            let _ = self.store.save_thread(&thread);
            return Err(error);
        }

        // Take receiver for lock-free streaming in background task
        let rx = {
            let mut rx_guard = active_sess.events_rx.lock().await;
            rx_guard.take()
        }
        .ok_or_else(|| RpcError::internal_error("Provider event stream is already active"))?;

        // Spawn background task to stream events into SQLite and broadcast
        let store = self.store.clone();
        let broadcaster = self.broadcaster.clone();
        let thread_id = params.thread_id.clone();
        let env_id = self.environment_id.clone();
        let session_clone = active_sess.session.clone();
        let events_rx_holder = active_sess.events_rx.clone();
        let active_sessions = self.active_sessions.clone();
        let generation = active_sess.generation;
        let turn_id_for_task = turn_id.clone();
        let event_writer = active_sess.event_writer.clone();
        let usage_scanner = self.usage_scanner.clone();
        let provider_kind = active_sess.execution.instance.config.provider;
        let provider_instance_id = active_sess.execution.instance.config.id.clone();
        let provider_version = active_sess.execution.snapshot.version.clone();
        let attempt_id_for_task = attempt_id.clone();
        let route_for_result = route.decision.clone();
        let manager = self.clone();
        let turn_input_for_retry = turn_input.clone();
        let hub_routing_context_for_retry = hub_routing_context.clone();
        let use_local_policy_for_retry = enforce_organization_policy;

        tokio::spawn(async move {
            let turn_id = turn_id_for_task;
            let mut rx = rx;
            let mut session_clone = session_clone;
            let mut events_rx_holder = events_rx_holder;
            let mut generation = generation;
            let mut event_writer = event_writer;
            let mut provider_kind = provider_kind;
            let mut provider_instance_id = provider_instance_id;
            let mut provider_version = provider_version;
            let mut attempt_id = attempt_id_for_task;
            let mut terminal_outcome = None;
            while let Some(prov_event) = rx.recv().await {
                let _event_guard = event_writer.lock().await;
                let still_active = {
                    let active = active_sessions.lock().await;
                    active
                        .get(&thread_id)
                        .is_some_and(|session| session.generation == generation)
                };
                if !still_active {
                    break;
                }
                let mut stream_seq = match store
                    .get_latest_seq(&thread_id)
                    .ok()
                    .and_then(|seq| seq.checked_add(1))
                {
                    Some(seq) => seq,
                    None => break,
                };
                let at = now_ms();

                match prov_event {
                    ProviderEvent::AttemptBoundaryAdvanced { boundary, .. } => {
                        if ensure_attempt_boundary(&store, &attempt_id, boundary).is_err() {
                            break;
                        }
                    }
                    ProviderEvent::TextDelta { delta } => {
                        if ensure_attempt_boundary(
                            &store,
                            &attempt_id,
                            AttemptBoundary::VisibleOutputBegun,
                        )
                        .is_err()
                            || store
                                .append_turn_attempt_output(&attempt_id, &delta)
                                .is_err()
                        {
                            break;
                        }
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::AssistantText {
                                item_id: format!("{}-text", turn_id.as_str()),
                                text: delta,
                            },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::ReasoningDelta { delta } => {
                        if ensure_attempt_boundary(
                            &store,
                            &attempt_id,
                            AttemptBoundary::VisibleOutputBegun,
                        )
                        .is_err()
                            || store
                                .append_turn_attempt_output(&attempt_id, &delta)
                                .is_err()
                        {
                            break;
                        }
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::Reasoning {
                                item_id: format!("{}-reasoning", turn_id.as_str()),
                                text: delta,
                            },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::ToolCallStarted { id, name, input } => {
                        if ensure_attempt_boundary(
                            &store,
                            &attempt_id,
                            AttemptBoundary::ToolSideEffectBegun,
                        )
                        .is_err()
                            || store
                                .save_tool_ledger_entry(&ToolLedgerEntry {
                                    attempt_id: attempt_id.clone(),
                                    turn_id: turn_id.clone(),
                                    provider_item_id: id.clone(),
                                    name: name.clone(),
                                    input: input.clone(),
                                    status: ToolLedgerStatus::Started,
                                    output: None,
                                    error: None,
                                    updated_at_ms: at,
                                })
                                .is_err()
                        {
                            break;
                        }
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::ToolCall {
                                item_id: id,
                                name,
                                input,
                                status: ToolCallStatus::Executing,
                                output: None,
                            },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::ToolCallFinished { id, output, error } => {
                        if ensure_attempt_boundary(
                            &store,
                            &attempt_id,
                            AttemptBoundary::ToolSideEffectBegun,
                        )
                        .is_err()
                        {
                            break;
                        }
                        let status = if error.is_some() {
                            ToolCallStatus::Failed
                        } else {
                            ToolCallStatus::Success
                        };
                        let prior = store.list_tool_ledger(&turn_id).ok().and_then(|entries| {
                            entries.into_iter().find(|entry| {
                                entry.attempt_id == attempt_id && entry.provider_item_id == id
                            })
                        });
                        let ledger_status = if error.is_some() {
                            ToolLedgerStatus::Failed
                        } else {
                            ToolLedgerStatus::Succeeded
                        };
                        let ledger_entry = ToolLedgerEntry {
                            attempt_id: attempt_id.clone(),
                            turn_id: turn_id.clone(),
                            provider_item_id: id.clone(),
                            name: prior
                                .as_ref()
                                .map(|entry| entry.name.clone())
                                .unwrap_or_else(|| "provider_tool".to_string()),
                            input: prior
                                .as_ref()
                                .map(|entry| entry.input.clone())
                                .unwrap_or(serde_json::Value::Null),
                            status: ledger_status,
                            output: output.clone(),
                            error: error.clone(),
                            updated_at_ms: at,
                        };
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::ToolCall {
                                item_id: id.clone(),
                                name: String::new(),
                                input: serde_json::Value::Null,
                                status,
                                output,
                            },
                        };
                        let continuation_id = match store.continuation_for_target_turn(&turn_id) {
                            Ok(continuation) => continuation.map(|value| value.id),
                            Err(_) => break,
                        };
                        let delivery_key = format!(
                            "tool-result:{}:{}:{}:{}",
                            thread_id.as_str(),
                            turn_id.as_str(),
                            attempt_id.as_str(),
                            id
                        );
                        match store.record_tool_result_event(
                            &delivery_key,
                            continuation_id.as_ref(),
                            &ledger_entry,
                            &ev,
                        ) {
                            Ok(true) => {
                                let _ = broadcaster.send(EventEnvelope {
                                    subscription_id: String::new(),
                                    environment_id: env_id.clone().into(),
                                    thread_id: Some(thread_id.clone()),
                                    run_id: None,
                                    seq: stream_seq,
                                    at_ms: at,
                                    kind: ev.kind,
                                });
                            }
                            Ok(false) => {}
                            Err(_) => break,
                        }
                    }
                    ProviderEvent::ApprovalRequested {
                        request_id,
                        kind,
                        detail,
                        ..
                    } => {
                        if ensure_attempt_boundary(
                            &store,
                            &attempt_id,
                            AttemptBoundary::ExternallyCommitted,
                        )
                        .is_err()
                        {
                            break;
                        }
                        if let Ok(Some(mut th)) = store.get_thread(&thread_id) {
                            th.status = ThreadStatus::AwaitingApproval;
                            th.updated_at_ms = at;
                            let _ = store.save_thread(&th);
                        }
                        if let Ok(Some(mut tr)) = store.get_turn(&turn_id) {
                            tr.status = TurnStatus::AwaitingApproval;
                            let _ = store.save_turn(&tr);
                        }

                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::ApprovalRequested {
                                request_id,
                                kind,
                                detail,
                            },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::SubAgentSpawned {
                        tool_use_id,
                        parent_tool_use_id,
                        agent_type,
                        description,
                        model,
                    } => {
                        if ensure_attempt_boundary(
                            &store,
                            &attempt_id,
                            AttemptBoundary::ToolSideEffectBegun,
                        )
                        .is_err()
                        {
                            break;
                        }
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::SubAgentSpawned {
                                sub_agent_id: tool_use_id,
                                parent_sub_agent_id: parent_tool_use_id,
                                parent_item_id: None,
                                title: description,
                                agent_type,
                                model: model.unwrap_or_else(|| "default".to_string()),
                                effort: None,
                            },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::TurnCompleted { outcome, usage } => {
                        let unresolved_tool_error = if outcome == TurnOutcome::Success {
                            match store.list_tool_ledger(&turn_id) {
                                Ok(entries) => {
                                    let unresolved_count = entries
                                        .iter()
                                        .filter(|entry| entry.status == ToolLedgerStatus::Started)
                                        .count();
                                    (unresolved_count > 0).then(|| {
                                        format!(
                                            "Provider reported success while {unresolved_count} side-effecting tool result(s) remained unresolved. PandaMUX marked the turn failed and preserved the tool ledger for manual recovery."
                                        )
                                    })
                                }
                                Err(error) => Some(format!(
                                    "Provider reported success, but PandaMUX could not verify the durable tool ledger: {error}. PandaMUX marked the turn failed to prevent an unsafe false-success state."
                                )),
                            }
                        } else {
                            None
                        };
                        let outcome = if let Some(message) = unresolved_tool_error {
                            let error = ThreadEvent {
                                thread_id: thread_id.clone(),
                                seq: stream_seq,
                                at_ms: at,
                                kind: ThreadEventKind::Error {
                                    class: "unresolved_tool_result".to_string(),
                                    message,
                                    recoverable: false,
                                },
                            };
                            if store.append_event(&error).is_err() {
                                break;
                            }
                            let _ = broadcaster.send(EventEnvelope {
                                subscription_id: String::new(),
                                environment_id: env_id.clone().into(),
                                thread_id: Some(thread_id.clone()),
                                run_id: None,
                                seq: stream_seq,
                                at_ms: at,
                                kind: error.kind,
                            });
                            let Some(completed_seq) = stream_seq.checked_add(1) else {
                                break;
                            };
                            stream_seq = completed_seq;
                            TurnOutcome::Failed
                        } else {
                            outcome
                        };

                        let res_tok = session_clone
                            .lock()
                            .await
                            .as_ref()
                            .and_then(|session| session.resume_token());
                        if let Some(res_tok) = res_tok {
                            let _ = store.save_resume_token(&thread_id, &res_tok);
                        }

                        if let Ok(Some(mut attempt)) = store.get_turn_attempt(&attempt_id) {
                            attempt.status = match outcome {
                                TurnOutcome::Success => TurnAttemptStatus::Completed,
                                TurnOutcome::Cancelled | TurnOutcome::Interrupted => {
                                    TurnAttemptStatus::Failed
                                }
                                TurnOutcome::Failed => TurnAttemptStatus::Failed,
                            };
                            attempt.ended_at_ms = Some(at);
                            let _ = store.save_turn_attempt(&attempt);
                        }
                        if let Ok(Some(continuation)) = store.continuation_for_target_turn(&turn_id)
                        {
                            let state = if outcome == TurnOutcome::Success {
                                ContinuationState::Completed
                            } else {
                                ContinuationState::Failed
                            };
                            let _ = store.set_continuation_state(&continuation.id, state, at);
                        }

                        if let Ok(Some(mut tr)) = store.get_turn(&turn_id) {
                            tr.status = match outcome {
                                TurnOutcome::Success => TurnStatus::Completed,
                                TurnOutcome::Cancelled => TurnStatus::Interrupted,
                                TurnOutcome::Failed => TurnStatus::Failed,
                                TurnOutcome::Interrupted => TurnStatus::Interrupted,
                            };
                            tr.ended_at_ms = Some(at);
                            tr.usage = usage;
                            let _ = store.save_turn(&tr);
                        }
                        if let Ok(Some(mut thread)) = store.get_thread(&thread_id) {
                            thread.status = match outcome {
                                TurnOutcome::Success
                                | TurnOutcome::Cancelled
                                | TurnOutcome::Interrupted => ThreadStatus::Idle,
                                TurnOutcome::Failed => ThreadStatus::Errored,
                            };
                            thread.updated_at_ms = at;
                            let _ = store.save_thread(&thread);
                        }
                        if let Some(scanner) = usage_scanner
                            .read()
                            .expect("usage scanner handle lock")
                            .as_ref()
                        {
                            let _ = scanner.trigger(UsageScanTrigger::TurnCompleted);
                        }

                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::TurnCompleted { outcome },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });

                        let worktree_dir = store
                            .get_thread(&thread_id)
                            .ok()
                            .flatten()
                            .map(|th| PathBuf::from(th.workspace.effective_path()))
                            .unwrap_or_default();

                        let cp_after = crate::checkpoint::create_checkpoint(
                            &worktree_dir,
                            &thread_id,
                            &turn_id,
                            crate::checkpoint::CheckpointPhase::After,
                        )
                        .unwrap_or(None);

                        let mut changed_files = Vec::new();
                        if let Ok(Some(mut tr)) = store.get_turn(&turn_id) {
                            tr.checkpoint_after = cp_after.clone();
                            if let (Some(before), Some(after)) = (&tr.checkpoint_before, &cp_after)
                                && let Ok(stats) = crate::checkpoint::compute_changed_files(
                                    &worktree_dir,
                                    before,
                                    after,
                                )
                            {
                                changed_files = stats
                                    .iter()
                                    .map(|s| {
                                        if s.additions > 0 || s.deletions > 0 {
                                            format!(
                                                "{} (+{} -{})",
                                                s.path, s.additions, s.deletions
                                            )
                                        } else {
                                            s.path.clone()
                                        }
                                    })
                                    .collect();
                            }
                            let _ = store.save_turn(&tr);
                        }

                        let _ =
                            crate::checkpoint::prune_old_checkpoints(&worktree_dir, &thread_id, 50);

                        let Some(settle_seq) = stream_seq.checked_add(1) else {
                            break;
                        };
                        stream_seq = settle_seq;
                        let settle = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::TurnSettled { changed_files },
                        };
                        if store.append_event(&settle).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: settle.kind,
                        });

                        terminal_outcome = Some(outcome);
                        break;
                    }
                    ProviderEvent::CapacityExhausted { details } => {
                        if provider_kind == ProviderKind::Codex
                            && details.get("source").and_then(serde_json::Value::as_str)
                                == Some("provider_rate_limit")
                            && let Some(report) = details.get("report")
                        {
                            let environment_id = EnvironmentId::new(env_id.clone());
                            let _ = record_local_codex_capacity(
                                &store,
                                &environment_id,
                                &provider_instance_id,
                                provider_version.as_deref(),
                                report,
                                at,
                            );
                        }
                        let Some(mut current_attempt) =
                            store.get_turn_attempt(&attempt_id).ok().flatten()
                        else {
                            break;
                        };
                        let attempt_provider_kind = current_attempt
                            .execution
                            .as_ref()
                            .map(|execution| execution.provider_kind)
                            .unwrap_or(provider_kind);
                        let attempt_provider_version = current_attempt
                            .execution
                            .as_ref()
                            .and_then(|execution| execution.provider_version.clone())
                            .or_else(|| provider_version.clone());
                        let evidence = CapacityInterruptionEvidence {
                            environment_id: current_attempt.environment_id.clone(),
                            provider_instance_id: current_attempt.provider_instance_id.clone(),
                            provider_kind: attempt_provider_kind,
                            subscription_key: current_attempt.subscription_key.clone(),
                            provider_version: attempt_provider_version,
                            details,
                            observed_at_ms: at,
                        };
                        current_attempt.capacity_evidence = Some(evidence.clone());

                        let automatic_failover_enabled = match store.get_settings() {
                            Ok(Some(settings)) => settings.advanced.automatic_failover_enabled,
                            Ok(None) => true,
                            Err(_) => false,
                        };
                        if automatic_failover_enabled
                            && current_attempt.boundary == AttemptBoundary::SafeToRetry
                            && current_attempt.ordinal == 1
                        {
                            current_attempt.status = TurnAttemptStatus::Retrying;
                            current_attempt.ended_at_ms = Some(at);
                            if store.save_turn_attempt(&current_attempt).is_err() {
                                break;
                            }
                            let excluded = store
                                .list_turn_attempts(&turn_id)
                                .map(|attempts| {
                                    attempts
                                        .into_iter()
                                        .map(|attempt| attempt.provider_instance_id)
                                        .collect::<BTreeSet<_>>()
                                })
                                .unwrap_or_else(|_| BTreeSet::from([provider_instance_id.clone()]));
                            let retry_thread = store.get_thread(&thread_id).ok().flatten();
                            let retry_route = if let Some(retry_thread) = retry_thread.as_ref() {
                                manager
                                    .select_turn_route(
                                        retry_thread,
                                        None,
                                        &excluded,
                                        None,
                                        hub_routing_context_for_retry.as_ref(),
                                        use_local_policy_for_retry,
                                    )
                                    .await
                                    .ok()
                            } else {
                                None
                            };

                            if let (Some(mut retry_thread), Some(retry_route)) =
                                (retry_thread, retry_route)
                            {
                                drop(_event_guard);
                                let active = active_sessions.lock().await.remove(&thread_id);
                                if let Some(active) = active {
                                    shutdown_active_session(active, false).await;
                                }
                                retry_thread.provider_instance_id =
                                    retry_route.provider_instance_id.clone();
                                retry_thread.model = retry_route.model.clone();
                                retry_thread.status = ThreadStatus::Working;
                                retry_thread.updated_at_ms = now_ms();
                                if store.save_thread(&retry_thread).is_ok()
                                    && let Ok(retry_session) = manager
                                        .get_or_start_session_with_resume(&retry_thread, false)
                                        .await
                                {
                                    let retry_attempt_id = TurnAttemptId::generate();
                                    let retry_started_at = now_ms();
                                    let checkpoint = store
                                        .get_turn(&turn_id)
                                        .ok()
                                        .flatten()
                                        .and_then(|turn| turn.checkpoint_before);
                                    let retry_attempt = TurnAttempt {
                                        id: retry_attempt_id.clone(),
                                        turn_id: turn_id.clone(),
                                        thread_id: thread_id.clone(),
                                        ordinal: 2,
                                        environment_id: retry_thread.environment_id.clone(),
                                        provider_instance_id: retry_route
                                            .provider_instance_id
                                            .clone(),
                                        subscription_key: retry_route.subscription_key.clone(),
                                        model: retry_route.model.clone(),
                                        boundary: AttemptBoundary::SafeToRetry,
                                        status: TurnAttemptStatus::Running,
                                        routing: retry_route.decision.clone(),
                                        execution: Some(AttemptExecutionEvidence {
                                            provider_kind: retry_session
                                                .execution
                                                .instance
                                                .config
                                                .provider,
                                            profile_dir: retry_session
                                                .execution
                                                .instance
                                                .config
                                                .profile_dir
                                                .clone(),
                                            binary_path: retry_session
                                                .execution
                                                .instance
                                                .config
                                                .binary_path
                                                .clone(),
                                            capabilities: retry_session
                                                .execution
                                                .instance
                                                .capabilities,
                                            provider_version: retry_session
                                                .execution
                                                .snapshot
                                                .version
                                                .clone(),
                                        }),
                                        checkpoint,
                                        partial_output: String::new(),
                                        capacity_evidence: None,
                                        started_at_ms: retry_started_at,
                                        ended_at_ms: None,
                                    };
                                    if store.save_turn_attempt(&retry_attempt).is_ok() {
                                        current_attempt.status =
                                            TurnAttemptStatus::InterruptedForCapacity;
                                        if store.save_turn_attempt(&current_attempt).is_ok() {
                                            let mut retry_input = turn_input_for_retry.clone();
                                            retry_input.model = Some(retry_route.model.clone());
                                            let send_ok = {
                                                let mut session =
                                                    retry_session.session.lock().await;
                                                if let Some(session) = session.as_mut() {
                                                    session.send_turn(retry_input).await.is_ok()
                                                } else {
                                                    false
                                                }
                                            };
                                            if send_ok {
                                                let retry_rx = {
                                                    let mut holder =
                                                        retry_session.events_rx.lock().await;
                                                    holder.take()
                                                };
                                                if let Some(retry_rx) = retry_rx
                                                    && store
                                                        .retarget_running_turn_for_safe_retry(
                                                            &turn_id,
                                                            &provider_instance_id,
                                                            &retry_route.provider_instance_id,
                                                            &retry_route.model,
                                                        )
                                                        .is_ok()
                                                {
                                                    let notice_at = now_ms();
                                                    let notice_writer =
                                                        retry_session.event_writer.clone();
                                                    let _notice_guard = notice_writer.lock().await;
                                                    if let Some(notice_seq) = store
                                                        .get_latest_seq(&thread_id)
                                                        .ok()
                                                        .and_then(|seq| seq.checked_add(1))
                                                    {
                                                        let notice = ThreadEvent {
                                                            thread_id: thread_id.clone(),
                                                            seq: notice_seq,
                                                            at_ms: notice_at,
                                                            kind: ThreadEventKind::Notice {
                                                                level: "warning".to_string(),
                                                                message: format!(
                                                                    "Capacity exhausted before output. Retrying once on {}.",
                                                                    retry_route
                                                                        .provider_instance_id
                                                                ),
                                                            },
                                                        };
                                                        if store.append_event(&notice).is_ok() {
                                                            let _ =
                                                                broadcaster.send(EventEnvelope {
                                                                    subscription_id: String::new(),
                                                                    environment_id: env_id
                                                                        .clone()
                                                                        .into(),
                                                                    thread_id: Some(
                                                                        thread_id.clone(),
                                                                    ),
                                                                    run_id: None,
                                                                    seq: notice_seq,
                                                                    at_ms: notice_at,
                                                                    kind: notice.kind,
                                                                });
                                                        }
                                                    }
                                                    rx = retry_rx;
                                                    session_clone = retry_session.session.clone();
                                                    events_rx_holder =
                                                        retry_session.events_rx.clone();
                                                    generation = retry_session.generation;
                                                    event_writer =
                                                        retry_session.event_writer.clone();
                                                    provider_kind = retry_session
                                                        .execution
                                                        .instance
                                                        .config
                                                        .provider;
                                                    provider_instance_id = retry_session
                                                        .execution
                                                        .instance
                                                        .config
                                                        .id
                                                        .clone();
                                                    provider_version = retry_session
                                                        .execution
                                                        .snapshot
                                                        .version
                                                        .clone();
                                                    attempt_id = retry_attempt_id;
                                                    continue;
                                                }
                                            }
                                        }
                                        if let Ok(Some(mut failed_retry)) =
                                            store.get_turn_attempt(&retry_attempt_id)
                                        {
                                            failed_retry.status = TurnAttemptStatus::Failed;
                                            failed_retry.ended_at_ms = Some(now_ms());
                                            let _ = store.save_turn_attempt(&failed_retry);
                                        }
                                    }
                                }

                                let _ = manager.persist_capacity_interruption(
                                    &thread_id,
                                    &turn_id,
                                    &current_attempt.id,
                                    evidence,
                                );
                                let active = active_sessions.lock().await.remove(&thread_id);
                                if let Some(active) = active {
                                    shutdown_active_session(active, false).await;
                                }
                                return;
                            }
                        }

                        let _ = manager.persist_capacity_interruption(
                            &thread_id,
                            &turn_id,
                            &current_attempt.id,
                            evidence,
                        );
                        drop(_event_guard);
                        if let Some(active) =
                            remove_active_generation(&active_sessions, &thread_id, generation).await
                        {
                            shutdown_active_session(active, false).await;
                        }
                        return;
                    }
                    ProviderEvent::Error {
                        message,
                        recoverable,
                    } => {
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::Error {
                                class: "provider".to_string(),
                                message,
                                recoverable,
                            },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::RateLimitObserved { details } => {
                        if provider_kind == ProviderKind::Codex {
                            let environment_id = EnvironmentId::new(env_id.clone());
                            let _ = record_local_codex_capacity(
                                &store,
                                &environment_id,
                                &provider_instance_id,
                                provider_version.as_deref(),
                                &details,
                                at,
                            );
                        }
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::RateLimitObserved {
                                provider: "agent".to_string(),
                                details,
                            },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                    ProviderEvent::Notice { level, message } => {
                        let ev = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: stream_seq,
                            at_ms: at,
                            kind: ThreadEventKind::Notice { level, message },
                        };
                        if store.append_event(&ev).is_err() {
                            break;
                        }
                        let _ = broadcaster.send(EventEnvelope {
                            subscription_id: String::new(),
                            environment_id: env_id.clone().into(),
                            thread_id: Some(thread_id.clone()),
                            run_id: None,
                            seq: stream_seq,
                            at_ms: at,
                            kind: ev.kind,
                        });
                    }
                }
            }

            let finished_at = now_ms();
            match terminal_outcome {
                Some(TurnOutcome::Success) => {
                    let still_active = {
                        let active = active_sessions.lock().await;
                        active
                            .get(&thread_id)
                            .is_some_and(|session| session.generation == generation)
                    };
                    if still_active {
                        let mut rx_guard = events_rx_holder.lock().await;
                        *rx_guard = Some(rx);
                    }
                    if let Ok(Some(mut thread)) = store.get_thread(&thread_id) {
                        thread.status = ThreadStatus::Idle;
                        thread.updated_at_ms = finished_at;
                        let _ = store.save_thread(&thread);
                    }
                }
                Some(TurnOutcome::Cancelled | TurnOutcome::Interrupted) => {
                    if let Some(active) =
                        remove_active_generation(&active_sessions, &thread_id, generation).await
                    {
                        shutdown_active_session(active, false).await;
                    }
                    if let Ok(Some(mut thread)) = store.get_thread(&thread_id) {
                        thread.status = ThreadStatus::Idle;
                        thread.updated_at_ms = finished_at;
                        let _ = store.save_thread(&thread);
                    }
                }
                Some(TurnOutcome::Failed) => {
                    if let Some(active) =
                        remove_active_generation(&active_sessions, &thread_id, generation).await
                    {
                        shutdown_active_session(active, false).await;
                    }
                    if let Ok(Some(mut thread)) = store.get_thread(&thread_id) {
                        thread.status = ThreadStatus::Errored;
                        thread.updated_at_ms = finished_at;
                        let _ = store.save_thread(&thread);
                    }
                }
                None => {
                    let still_active = {
                        let active = active_sessions.lock().await;
                        active
                            .get(&thread_id)
                            .is_some_and(|session| session.generation == generation)
                    };
                    if !still_active {
                        return;
                    }

                    let worktree_dir = store
                        .get_thread(&thread_id)
                        .ok()
                        .flatten()
                        .map(|thread| PathBuf::from(thread.workspace.effective_path()))
                        .unwrap_or_default();
                    let checkpoint_after = crate::checkpoint::create_checkpoint(
                        &worktree_dir,
                        &thread_id,
                        &turn_id,
                        crate::checkpoint::CheckpointPhase::After,
                    )
                    .unwrap_or(None);
                    let mut changed_files = Vec::new();
                    if let Ok(Some(mut turn)) = store.get_turn(&turn_id) {
                        turn.checkpoint_after = checkpoint_after.clone();
                        if let (Some(before), Some(after)) =
                            (&turn.checkpoint_before, &checkpoint_after)
                            && let Ok(stats) = crate::checkpoint::compute_changed_files(
                                &worktree_dir,
                                before,
                                after,
                            )
                        {
                            changed_files = stats
                                .iter()
                                .map(|stat| {
                                    if stat.additions > 0 || stat.deletions > 0 {
                                        format!(
                                            "{} (+{} -{})",
                                            stat.path, stat.additions, stat.deletions
                                        )
                                    } else {
                                        stat.path.clone()
                                    }
                                })
                                .collect();
                        }
                        turn.status = TurnStatus::Failed;
                        turn.ended_at_ms = Some(finished_at);
                        let _ = store.save_turn(&turn);
                    }
                    if let Ok(Some(mut attempt)) = store.get_turn_attempt(&attempt_id) {
                        attempt.status = TurnAttemptStatus::Failed;
                        attempt.ended_at_ms = Some(finished_at);
                        let _ = store.save_turn_attempt(&attempt);
                    }
                    if let Ok(Some(continuation)) = store.continuation_for_target_turn(&turn_id) {
                        let _ = store.set_continuation_state(
                            &continuation.id,
                            ContinuationState::Failed,
                            finished_at,
                        );
                    }
                    if let Ok(Some(mut thread)) = store.get_thread(&thread_id) {
                        thread.status = ThreadStatus::Errored;
                        thread.updated_at_ms = finished_at;
                        let _ = store.save_thread(&thread);
                    }
                    let _ = crate::checkpoint::prune_old_checkpoints(&worktree_dir, &thread_id, 50);

                    let _event_guard = event_writer.lock().await;
                    if let Some(error_seq) = store
                        .get_latest_seq(&thread_id)
                        .ok()
                        .and_then(|seq| seq.checked_add(1))
                    {
                        let event = ThreadEvent {
                            thread_id: thread_id.clone(),
                            seq: error_seq,
                            at_ms: finished_at,
                            kind: ThreadEventKind::Error {
                                class: "provider_process".to_string(),
                                message: "Provider event stream closed before turn completion"
                                    .to_string(),
                                recoverable: false,
                            },
                        };
                        if store.append_event(&event).is_ok() {
                            let _ = broadcaster.send(EventEnvelope {
                                subscription_id: String::new(),
                                environment_id: env_id.clone().into(),
                                thread_id: Some(thread_id.clone()),
                                run_id: None,
                                seq: error_seq,
                                at_ms: finished_at,
                                kind: event.kind,
                            });

                            if let Some(completed_seq) = error_seq.checked_add(1) {
                                let completed = ThreadEvent {
                                    thread_id: thread_id.clone(),
                                    seq: completed_seq,
                                    at_ms: finished_at,
                                    kind: ThreadEventKind::TurnCompleted {
                                        outcome: TurnOutcome::Failed,
                                    },
                                };
                                if store.append_event(&completed).is_ok() {
                                    let _ = broadcaster.send(EventEnvelope {
                                        subscription_id: String::new(),
                                        environment_id: env_id.clone().into(),
                                        thread_id: Some(thread_id.clone()),
                                        run_id: None,
                                        seq: completed_seq,
                                        at_ms: finished_at,
                                        kind: completed.kind,
                                    });

                                    if let Some(settled_seq) = completed_seq.checked_add(1) {
                                        let settled = ThreadEvent {
                                            thread_id: thread_id.clone(),
                                            seq: settled_seq,
                                            at_ms: finished_at,
                                            kind: ThreadEventKind::TurnSettled { changed_files },
                                        };
                                        if store.append_event(&settled).is_ok() {
                                            let _ = broadcaster.send(EventEnvelope {
                                                subscription_id: String::new(),
                                                environment_id: env_id.into(),
                                                thread_id: Some(thread_id.clone()),
                                                run_id: None,
                                                seq: settled_seq,
                                                at_ms: finished_at,
                                                kind: settled.kind,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(active) =
                        remove_active_generation(&active_sessions, &thread_id, generation).await
                    {
                        shutdown_active_session(active, false).await;
                    }
                }
            }
        });

        Ok(ThreadSendTurnResult {
            turn_id,
            attempt_id,
            seq: started_seq,
            routing: route_for_result,
        })
    }

    /// Starts an audited continuation in a fresh provider session. Native
    /// provider resume tokens are deliberately not reused across instances.
    pub async fn continue_after_capacity(
        &self,
        mut params: ThreadContinueParams,
    ) -> Result<ThreadContinueResult, RpcError> {
        params.hub_routing_context = None;
        self.continue_after_capacity_with_policy(params, true, None)
            .await
    }

    pub async fn continue_after_capacity_from_hub(
        &self,
        mut params: ThreadContinueParams,
    ) -> Result<ThreadContinueResult, RpcError> {
        let hub_routing_context = params.hub_routing_context.take();
        self.continue_after_capacity_with_policy(params, false, hub_routing_context)
            .await
    }

    async fn continue_after_capacity_with_policy(
        &self,
        params: ThreadContinueParams,
        use_local_policy: bool,
        hub_routing_context: Option<HubRoutingContext>,
    ) -> Result<ThreadContinueResult, RpcError> {
        let _dispatch_guard = self.continuation_dispatch.lock().await;
        let continuation = self
            .store
            .get_continuation(&params.continuation_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .ok_or_else(|| RpcError::not_found("Capacity continuation not found"))?;
        if params
            .thread_id
            .as_ref()
            .is_some_and(|thread_id| thread_id != &continuation.thread_id)
        {
            return Err(RpcError::invalid_params(
                "Capacity continuation does not belong to the requested thread",
            ));
        }
        if continuation.state == ContinuationState::Failed {
            return Err(RpcError::invalid_params(
                "This capacity continuation has already failed",
            ));
        }
        if let Some(target_turn_id) = continuation.target_turn_id.as_ref() {
            let target = self
                .store
                .get_turn(target_turn_id)
                .map_err(|error| RpcError::internal_error(error.to_string()))?
                .ok_or_else(|| RpcError::internal_error("Continuation target turn is missing"))?;
            if matches!(
                target.status,
                TurnStatus::Failed | TurnStatus::Interrupted | TurnStatus::InterruptedForCapacity
            ) {
                let _ = self.store.set_continuation_state(
                    &continuation.id,
                    ContinuationState::Failed,
                    now_ms(),
                );
                return Err(RpcError::with_data(
                    -32023,
                    "The durable continuation target did not complete",
                    serde_json::json!({
                        "reason": "continuation_target_failed",
                        "targetTurnId": target_turn_id,
                    }),
                ));
            }
            let attempt = self
                .store
                .list_turn_attempts(target_turn_id)
                .map_err(|error| RpcError::internal_error(error.to_string()))?
                .into_iter()
                .max_by_key(|attempt| attempt.ordinal)
                .ok_or_else(|| RpcError::internal_error("Continuation target has no attempt"))?;
            return Ok(ThreadContinueResult {
                turn_id: target_turn_id.clone(),
                attempt_id: attempt.id,
                seq: self
                    .store
                    .get_latest_seq(&continuation.thread_id)
                    .map_err(|error| RpcError::internal_error(error.to_string()))?,
                routing: attempt.routing,
            });
        }

        let mut thread = self
            .store
            .get_thread(&continuation.thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .ok_or_else(|| RpcError::not_found("Continuation thread not found"))?;
        if matches!(
            thread.status,
            ThreadStatus::Working | ThreadStatus::AwaitingApproval
        ) {
            return Err(RpcError::invalid_params(
                "Thread is already executing or awaiting approval",
            ));
        }
        let checkpoint = continuation.checkpoint.as_deref().ok_or_else(|| {
            RpcError::with_data(
                -32023,
                "The interrupted turn has no portable worktree checkpoint",
                serde_json::json!({ "reason": "continuation_checkpoint_missing" }),
            )
        })?;
        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        let verification_turn_id = TurnId::generate();
        let current_checkpoint = crate::checkpoint::create_checkpoint(
            &worktree_dir,
            &thread.id,
            &verification_turn_id,
            crate::checkpoint::CheckpointPhase::Before,
        )
        .map_err(RpcError::internal_error)?
        .ok_or_else(|| {
            RpcError::with_data(
                -32023,
                "The worktree can no longer produce a portable checkpoint",
                serde_json::json!({ "reason": "continuation_checkpoint_unavailable" }),
            )
        })?;
        let changed = crate::checkpoint::compute_changed_files(
            &worktree_dir,
            checkpoint,
            &current_checkpoint,
        )
        .map_err(RpcError::internal_error)?;
        if !changed.is_empty() {
            return Err(RpcError::with_data(
                -32023,
                "The worktree changed after interruption; continuation was not started",
                serde_json::json!({
                    "reason": "continuation_checkpoint_changed",
                    "changedFiles": changed.iter().map(|item| item.path.clone()).collect::<Vec<_>>(),
                }),
            ));
        }

        let excluded = self
            .store
            .list_turn_attempts(&continuation.turn_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .into_iter()
            .map(|attempt| attempt.provider_instance_id)
            .collect::<BTreeSet<_>>();
        let pinned = params
            .provider_instance_id
            .as_ref()
            .or(continuation.target_provider_instance_id.as_ref());
        let route = self
            .select_turn_route(
                &thread,
                None,
                &excluded,
                pinned,
                hub_routing_context.as_ref(),
                use_local_policy,
            )
            .await?;
        self.store
            .claim_continuation(&continuation.id, &route.provider_instance_id, now_ms())
            .map_err(|error| RpcError::invalid_params(error.to_string()))?;
        thread.provider_instance_id = route.provider_instance_id.clone();
        thread.model = route.model.clone();
        self.store
            .save_thread(&thread)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;

        let send_result = self
            .send_turn_with_policy(
                ThreadSendTurnParams {
                    thread_id: thread.id,
                    text: continuation.handoff,
                    attachment_ids: Vec::new(),
                    model: Some(route.model),
                    effort: None,
                    hub_routing_context: None,
                },
                use_local_policy,
                Some(&route.provider_instance_id),
                false,
                hub_routing_context,
                Some(&continuation.id),
            )
            .await;
        let send_result = match send_result {
            Ok(result) => result,
            Err(error) => {
                let _ = self.store.set_continuation_state(
                    &continuation.id,
                    ContinuationState::Failed,
                    now_ms(),
                );
                return Err(error);
            }
        };
        if let Some(target) = self
            .store
            .get_turn(&send_result.turn_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
        {
            let terminal_state = match target.status {
                TurnStatus::Completed => Some(ContinuationState::Completed),
                TurnStatus::Failed
                | TurnStatus::Interrupted
                | TurnStatus::InterruptedForCapacity => Some(ContinuationState::Failed),
                TurnStatus::Requested | TurnStatus::Running | TurnStatus::AwaitingApproval => None,
            };
            if let Some(state) = terminal_state {
                self.store
                    .set_continuation_state(&continuation.id, state, now_ms())
                    .map_err(|error| RpcError::internal_error(error.to_string()))?;
            }
        }
        Ok(ThreadContinueResult {
            turn_id: send_result.turn_id,
            attempt_id: send_result.attempt_id,
            seq: send_result.seq,
            routing: send_result.routing,
        })
    }

    /// Responds to a pending approval request and resumes the turn.
    pub async fn respond_approval(
        &self,
        params: ThreadRespondApprovalParams,
    ) -> Result<(), RpcError> {
        let payload_json = serde_json::to_string(&params.decision)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        let payload_sha256 = format!("{:x}", Sha256::digest(payload_json.as_bytes()));

        let session_opt = {
            let active = self.active_sessions.lock().await;
            active.get(&params.thread_id).cloned()
        };
        let Some(active_sess) = session_opt else {
            if let Some(existing) = self
                .store
                .latest_continuation_delivery(
                    &params.thread_id,
                    ContinuationDeliveryKind::Approval,
                    &params.request_id,
                    &payload_sha256,
                    &payload_json,
                )
                .map_err(|error| RpcError::internal_error(error.to_string()))?
            {
                return replay_approval_delivery(
                    &existing,
                    &params,
                    &payload_sha256,
                    &payload_json,
                );
            }
            return Err(RpcError::invalid_params(format!(
                "No active session found for thread {}",
                params.thread_id
            )));
        };

        let _event_guard = active_sess.event_writer.lock().await;

        let active_turn = self
            .store
            .list_turns(&params.thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .into_iter()
            .rev()
            .find(|turn| turn.status == TurnStatus::AwaitingApproval);
        let Some(turn) = active_turn else {
            if let Some(existing) = self
                .store
                .latest_continuation_delivery(
                    &params.thread_id,
                    ContinuationDeliveryKind::Approval,
                    &params.request_id,
                    &payload_sha256,
                    &payload_json,
                )
                .map_err(|error| RpcError::internal_error(error.to_string()))?
            {
                return replay_approval_delivery(
                    &existing,
                    &params,
                    &payload_sha256,
                    &payload_json,
                );
            }
            return Err(RpcError::invalid_params(
                "No active turn is awaiting this approval",
            ));
        };
        let delivery_key = format!(
            "approval:{}:{}:{}",
            params.thread_id.as_str(),
            turn.id.as_str(),
            params.request_id
        );
        if let Some(existing) = self
            .store
            .get_continuation_delivery(&delivery_key)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
        {
            return replay_approval_delivery(&existing, &params, &payload_sha256, &payload_json);
        }
        let attempt = self
            .store
            .list_turn_attempts(&turn.id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?
            .into_iter()
            .max_by_key(|attempt| attempt.ordinal)
            .ok_or_else(|| RpcError::internal_error("Active turn has no durable attempt"))?;
        ensure_attempt_boundary(
            &self.store,
            &attempt.id,
            AttemptBoundary::ExternallyCommitted,
        )
        .map_err(|error| RpcError::internal_error(error.to_string()))?;

        let created_at_ms = now_ms();
        let delivery = ContinuationDelivery {
            delivery_key: delivery_key.clone(),
            continuation_id: None,
            thread_id: params.thread_id.clone(),
            turn_id: turn.id.clone(),
            kind: ContinuationDeliveryKind::Approval,
            item_id: params.request_id.clone(),
            payload_sha256,
            payload_json,
            state: ContinuationDeliveryState::Pending,
            created_at_ms,
            delivered_at_ms: None,
        };
        self.store
            .prepare_continuation_delivery(&delivery)
            .map_err(|error| RpcError::invalid_params(error.to_string()))?;

        {
            let mut sess = active_sess.session.lock().await;
            sess.as_mut()
                .ok_or_else(|| RpcError::internal_error("Provider session is closed"))?
                .respond_approval(params.request_id.clone(), params.decision)
                .await
                .map_err(|e| RpcError::internal_error(e.to_string()))?;
        }

        let now = now_ms();
        self.store
            .mark_continuation_delivery_delivered(&delivery_key, now)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        let next_seq = self
            .store
            .get_latest_seq(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            + 1;

        let ev = ThreadEvent {
            thread_id: params.thread_id.clone(),
            seq: next_seq,
            at_ms: now,
            kind: ThreadEventKind::ApprovalResolved {
                request_id: params.request_id,
                decision: params.decision,
                by: "user".to_string(),
            },
        };
        self.store
            .append_event(&ev)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let _ = self.broadcaster.send(EventEnvelope {
            subscription_id: String::new(),
            environment_id: self.environment_id.clone().into(),
            thread_id: Some(params.thread_id.clone()),
            run_id: None,
            seq: next_seq,
            at_ms: now,
            kind: ev.kind,
        });

        if let Ok(Some(mut th)) = self.store.get_thread(&params.thread_id) {
            th.status = ThreadStatus::Working;
            th.updated_at_ms = now;
            let _ = self.store.save_thread(&th);
        }
        if let Ok(Some(mut turn)) = self.store.get_turn(&turn.id) {
            turn.status = TurnStatus::Running;
            let _ = self.store.save_turn(&turn);
        }

        Ok(())
    }

    /// Interrupts the active turn on a thread.
    pub async fn interrupt_turn(&self, params: ThreadCancelTurnParams) -> Result<(), RpcError> {
        let session = {
            let mut active = self.active_sessions.lock().await;
            active.remove(&params.thread_id)
        };

        let event_writer = session
            .as_ref()
            .map(|active_session| active_session.event_writer.clone());
        let _event_guard = if let Some(event_writer) = event_writer.as_ref() {
            Some(event_writer.lock().await)
        } else {
            None
        };

        if let Some(active_session) = session {
            shutdown_active_session(active_session, true).await;
        }

        let now = now_ms();
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        if let Some(mut th) = thread.clone() {
            th.status = ThreadStatus::Idle;
            th.updated_at_ms = now;
            self.store
                .save_thread(&th)
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
        }

        let mut interrupted = false;
        let mut changed_files = Vec::new();
        let worktree_dir = thread
            .as_ref()
            .map(|thread| PathBuf::from(thread.workspace.effective_path()));
        let turns = self
            .store
            .list_turns(&params.thread_id)
            .map_err(|error| RpcError::internal_error(error.to_string()))?;
        for mut turn in turns {
            if matches!(
                turn.status,
                TurnStatus::Running | TurnStatus::AwaitingApproval | TurnStatus::Requested
            ) {
                if let Some(worktree_dir) = worktree_dir.as_ref() {
                    let checkpoint_after = crate::checkpoint::create_checkpoint(
                        worktree_dir,
                        &params.thread_id,
                        &turn.id,
                        crate::checkpoint::CheckpointPhase::After,
                    )
                    .unwrap_or(None);
                    if let (Some(before), Some(after)) =
                        (&turn.checkpoint_before, &checkpoint_after)
                        && let Ok(stats) =
                            crate::checkpoint::compute_changed_files(worktree_dir, before, after)
                    {
                        changed_files = stats
                            .iter()
                            .map(|stat| {
                                if stat.additions > 0 || stat.deletions > 0 {
                                    format!(
                                        "{} (+{} -{})",
                                        stat.path, stat.additions, stat.deletions
                                    )
                                } else {
                                    stat.path.clone()
                                }
                            })
                            .collect();
                    }
                    turn.checkpoint_after = checkpoint_after;
                }
                turn.status = TurnStatus::Interrupted;
                turn.ended_at_ms = Some(now);
                self.store
                    .save_turn(&turn)
                    .map_err(|error| RpcError::internal_error(error.to_string()))?;
                for mut attempt in self
                    .store
                    .list_turn_attempts(&turn.id)
                    .map_err(|error| RpcError::internal_error(error.to_string()))?
                {
                    if matches!(
                        attempt.status,
                        TurnAttemptStatus::Running | TurnAttemptStatus::Retrying
                    ) {
                        attempt.status = TurnAttemptStatus::Failed;
                        attempt.ended_at_ms = Some(now);
                        self.store
                            .save_turn_attempt(&attempt)
                            .map_err(|error| RpcError::internal_error(error.to_string()))?;
                    }
                }
                interrupted = true;
            }
        }

        if interrupted {
            let completed_seq = self
                .store
                .get_latest_seq(&params.thread_id)
                .map_err(|error| RpcError::internal_error(error.to_string()))?
                .checked_add(1)
                .ok_or_else(|| RpcError::internal_error("Thread event sequence overflow"))?;
            let completed = ThreadEvent {
                thread_id: params.thread_id.clone(),
                seq: completed_seq,
                at_ms: now,
                kind: ThreadEventKind::TurnCompleted {
                    outcome: TurnOutcome::Interrupted,
                },
            };
            self.store
                .append_event(&completed)
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
            let _ = self.broadcaster.send(EventEnvelope {
                subscription_id: String::new(),
                environment_id: self.environment_id.clone().into(),
                thread_id: Some(params.thread_id.clone()),
                run_id: None,
                seq: completed_seq,
                at_ms: now,
                kind: completed.kind,
            });

            let settled_seq = completed_seq
                .checked_add(1)
                .ok_or_else(|| RpcError::internal_error("Thread event sequence overflow"))?;
            let settled = ThreadEvent {
                thread_id: params.thread_id.clone(),
                seq: settled_seq,
                at_ms: now,
                kind: ThreadEventKind::TurnSettled { changed_files },
            };
            self.store
                .append_event(&settled)
                .map_err(|error| RpcError::internal_error(error.to_string()))?;
            let _ = self.broadcaster.send(EventEnvelope {
                subscription_id: String::new(),
                environment_id: self.environment_id.clone().into(),
                thread_id: Some(params.thread_id.clone()),
                run_id: None,
                seq: settled_seq,
                at_ms: now,
                kind: settled.kind,
            });
        }

        Ok(())
    }

    /// Queries the saved resumption token for continuing a thread across restarts.
    pub fn resume_thread(
        &self,
        params: ThreadResumeParams,
    ) -> Result<ThreadResumeResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| {
                RpcError::not_found(format!("Thread not found: {}", params.thread_id))
            })?;

        let resume_token = self
            .store
            .get_resume_token(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        Ok(ThreadResumeResult {
            thread_id: thread.id,
            resume_token,
        })
    }

    /// Lists checkpoint refs for a thread.
    pub fn list_checkpoints(&self, thread_id: &ThreadId) -> Result<Vec<String>, RpcError> {
        let thread = self
            .store
            .get_thread(thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {thread_id} not found")))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        if !crate::checkpoint::is_git_repository(&worktree_dir) {
            return Ok(Vec::new());
        }

        let prefix = format!("refs/pandamux/checkpoints/{}/", thread_id.as_str());
        let list_out = std::process::Command::new("git")
            .arg("-C")
            .arg(&worktree_dir)
            .arg("for-each-ref")
            .arg("--format=%(refname)")
            .arg(&prefix)
            .output()
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        if !list_out.status.success() {
            return Ok(Vec::new());
        }

        let refs_str = String::from_utf8_lossy(&list_out.stdout);
        let refs: Vec<String> = refs_str
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(refs)
    }

    /// Computes diff stats between two checkpoint refs in a thread.
    pub fn diff_checkpoints(
        &self,
        thread_id: &ThreadId,
        before_ref: &str,
        after_ref: &str,
    ) -> Result<Vec<crate::checkpoint::ChangedFileStat>, RpcError> {
        let thread = self
            .store
            .get_thread(thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {thread_id} not found")))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::checkpoint::compute_changed_files(&worktree_dir, before_ref, after_ref)
            .map_err(RpcError::internal_error)
    }

    /// Computes unified diff patch between two checkpoint refs in a thread.
    pub fn diff_checkpoints_patch(
        &self,
        thread_id: &ThreadId,
        before_ref: &str,
        after_ref: &str,
        file_path: Option<&str>,
    ) -> Result<String, RpcError> {
        let thread = self
            .store
            .get_thread(thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {thread_id} not found")))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::checkpoint::compute_checkpoint_patch(&worktree_dir, before_ref, after_ref, file_path)
            .map_err(RpcError::internal_error)
    }

    /// Rolls back a thread's worktree to the specified checkpoint ref.
    pub fn rollback_checkpoint(
        &self,
        thread_id: &ThreadId,
        checkpoint_ref: &str,
    ) -> Result<(), RpcError> {
        let thread = self
            .store
            .get_thread(thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {thread_id} not found")))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::checkpoint::rollback_to_checkpoint(&worktree_dir, checkpoint_ref)
            .map_err(RpcError::internal_error)
    }

    /// Queries the Git status and file modifications for a thread's worktree.
    pub fn git_status(&self, params: GitStatusParams) -> Result<GitStatusResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {} not found", params.thread_id)))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::git::get_git_status(&worktree_dir).map_err(RpcError::internal_error)
    }

    /// Stages all changes and creates a Git commit in the thread's worktree.
    pub fn git_commit(&self, params: GitCommitParams) -> Result<GitCommitResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {} not found", params.thread_id)))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::git::git_commit(&worktree_dir, params.message).map_err(RpcError::internal_error)
    }

    /// Pushes committed changes in the thread's worktree to the remote repository.
    pub fn git_push(&self, params: GitPushParams) -> Result<GitPushResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {} not found", params.thread_id)))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::git::git_push(
            &worktree_dir,
            params.remote.as_deref(),
            params.branch.as_deref(),
        )
        .map_err(RpcError::internal_error)
    }

    /// Creates a Pull Request using gh CLI if installed, or generates a compare URL.
    pub fn git_create_pr(&self, params: GitCreatePrParams) -> Result<GitCreatePrResult, RpcError> {
        let thread = self
            .store
            .get_thread(&params.thread_id)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::not_found(format!("Thread {} not found", params.thread_id)))?;

        let worktree_dir = PathBuf::from(thread.workspace.effective_path());
        crate::git::git_create_pr(
            &worktree_dir,
            params.title.as_deref(),
            params.body.as_deref(),
        )
        .map_err(RpcError::internal_error)
    }
}
