use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use pandamux_core::{
    AgentInstanceRef, AttemptBoundary, AttemptStatus, AttemptWorktreeId, BundleSha, ContentSha256,
    EnvironmentId, ExecutionLeaseId, NodeStoreId, RunAttemptId, Thread, ThreadId, ThreadOrigin,
    ThreadStatus, ThreadWorkspace, Turn, TurnAttempt, TurnAttemptStatus, TurnId, TurnInput,
    TurnStatus, WorktreeRef,
};
use pandamux_protocol::{
    ORCHESTRATION_EXECUTION_SCHEMA_VERSION, OrchestrationAttemptAckParams,
    OrchestrationAttemptAckResult, OrchestrationAttemptCancelParams,
    OrchestrationAttemptCancelResult, OrchestrationAttemptDispatchParams,
    OrchestrationAttemptEventsParams, OrchestrationAttemptEventsResult,
    OrchestrationAttemptPrepareParams, OrchestrationAttemptPrepareResult,
    OrchestrationAttemptReceipt, OrchestrationAttemptStatusParams,
    OrchestrationAttemptStatusResult, OrchestrationExecutionIdentity,
    OrchestrationNodeAttemptState, OrchestrationNodeEvent, OrchestrationPreparedExecution,
    OrchestrationRepositoryPrepareParams, OrchestrationRepositoryPrepareResult, RpcError,
    ThreadCancelTurnParams,
};
use pandamux_providers::{InjectedInstructions, SessionSpec, ToolPolicy};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::DataPaths;
use crate::bundle_store::BundleStore;
use crate::execution_admission::{
    DEFAULT_EXECUTION_ADMISSION_TTL_MS, EnqueueRemoteOrchestratedAdmissionRequest,
    ExecutionAdmissionClaimRequest, ExecutionAdmissionService,
};
use crate::managed_git::{MAX_CONFIGURATION_GIT_BUNDLE_BYTES, ManagedGitCommand, ManagedGitRunner};
use crate::request_context::{PeerRole, RequestContext};
use crate::store::{NodeReceiptKind, NodeReceiptRecord, QueueState, Store};
use crate::thread_manager::{
    PreparedOrchestratedTurn, PreparedTurnDispatchDisposition, PreparedTurnEvent,
    RemotePreparedTurnAuthority, ThreadManager,
};

const MAX_REMOTE_TASK_PROMPT_BYTES: usize = 1024 * 1024;
const MAX_REMOTE_OUTPUT_CONTRACT_BYTES: usize = 256 * 1024;
const MAX_REMOTE_EVENTS: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeExecutionFailpoint {
    AfterRepositoryMaterialized,
    AfterAttemptWorktreeMaterialized,
    AfterAttemptReceipt,
    BeforeDispatchMarker,
    AfterDispatchMarker,
    AfterProviderDispatch,
}

#[derive(Clone)]
pub struct NodeExecutionService {
    store: Store,
    paths: DataPaths,
    bundle_store: BundleStore,
    thread_manager: ThreadManager,
    admission: ExecutionAdmissionService,
    managed_git: ManagedGitRunner,
    environment_id: EnvironmentId,
    node_store_id: NodeStoreId,
    completion_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

#[derive(Clone)]
struct PreparedMaterial {
    thread: Thread,
    turn: Turn,
    turn_attempt: TurnAttempt,
    session_spec: SessionSpec,
    queue_id: String,
}

struct PreparedMaterialScope {
    environment_id: EnvironmentId,
    project_id: pandamux_core::ProjectId,
    effort: Option<String>,
}

struct InternalAttemptUpdate {
    key: String,
    state: OrchestrationNodeAttemptState,
    safe_error_code: Option<String>,
    at_ms: u64,
}

struct TerminalReceiptAuthority {
    store: Store,
    node_store_id: NodeStoreId,
    environment_id: EnvironmentId,
}

impl NodeExecutionService {
    pub fn new(
        store: Store,
        paths: DataPaths,
        bundle_store: BundleStore,
        thread_manager: ThreadManager,
        environment_id: EnvironmentId,
    ) -> Self {
        let node_store_id = store
            .usage_node_store_id()
            .expect("a migrated PandaMUX store always has a durable node identity");
        Self {
            admission: ExecutionAdmissionService::new(store.clone()),
            managed_git: ManagedGitRunner::new(paths.clone()),
            store,
            paths,
            bundle_store,
            thread_manager,
            environment_id,
            node_store_id,
            completion_tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn node_store_id(&self) -> &NodeStoreId {
        &self.node_store_id
    }

    pub async fn repository_prepare(
        &self,
        context: &RequestContext,
        params: OrchestrationRepositoryPrepareParams,
        failpoint: Option<NodeExecutionFailpoint>,
    ) -> Result<OrchestrationRepositoryPrepareResult, RpcError> {
        self.authorize(context)?;
        validate_identity(&params.identity)?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || service.repository_prepare_blocking(params, failpoint))
            .await
            .map_err(remote_worker_error)?
    }

    fn repository_prepare_blocking(
        &self,
        params: OrchestrationRepositoryPrepareParams,
        failpoint: Option<NodeExecutionFailpoint>,
    ) -> Result<OrchestrationRepositoryPrepareResult, RpcError> {
        let request_sha256 = sha256_content(&params)?;
        if let Some(existing) = self.replay::<OrchestrationRepositoryPrepareResult>(
            NodeReceiptKind::RepositoryPrepare,
            &params.identity,
            &request_sha256,
        )? {
            if existing.baseline_present {
                self.verify_repository_result(&existing)?;
            }
            return Ok(existing);
        }

        self.paths
            .ensure_private_roots()
            .map_err(|_| remote_internal("remote_repository_roots_unavailable"))?;
        let identity_sha256 = identity_sha256(&params.identity)?;
        let repository_key_sha256 = remote_repository_key_sha256(
            &params.identity,
            &params.baseline_bundle_sha256,
            &params.baseline_bundle_bytes_sha256,
        )?;
        let staged_path = self.repository_staging_path(&repository_key_sha256);
        let repository_path = self.repository_path(&repository_key_sha256);
        ensure_private_directory(
            repository_path
                .parent()
                .ok_or_else(|| remote_internal("remote_repository_path_invalid"))?,
        )?;

        let bundle_bytes = match params.baseline_bundle_bytes_base64.as_deref() {
            Some(encoded) => {
                let max_encoded_len =
                    (MAX_CONFIGURATION_GIT_BUNDLE_BYTES as usize).saturating_add(2) / 3 * 4;
                if encoded.len() > max_encoded_len {
                    return Err(remote_invalid("remote_repository_bundle_too_large"));
                }
                let bytes = BASE64
                    .decode(encoded.as_bytes())
                    .map_err(|_| remote_invalid("remote_repository_bundle_base64_invalid"))?;
                if bytes.is_empty()
                    || bytes.len() as u64 > MAX_CONFIGURATION_GIT_BUNDLE_BYTES
                    || sha256_hex(&bytes) != params.baseline_bundle_bytes_sha256.as_str()
                    || sha256_hex(&bytes) != params.baseline_bundle_sha256.as_str()
                {
                    return Err(remote_invalid("remote_repository_bundle_digest_mismatch"));
                }
                persist_private_file(&staged_path, &bytes)?;
                Some(bytes)
            }
            None => read_optional_private_file(
                &staged_path,
                MAX_CONFIGURATION_GIT_BUNDLE_BYTES,
                params.baseline_bundle_bytes_sha256.as_str(),
            )?,
        };
        if bundle_bytes
            .as_ref()
            .is_some_and(|bytes| sha256_hex(bytes) != params.baseline_bundle_sha256.as_str())
        {
            return Err(remote_invalid("remote_repository_bundle_digest_mismatch"));
        }

        let baseline_present = if bundle_bytes.is_some() {
            self.materialize_repository(
                &repository_path,
                &staged_path,
                params.identity.baseline_commit.as_str(),
                repository_key_sha256.as_str(),
            )?;
            true
        } else {
            false
        };
        if failpoint == Some(NodeExecutionFailpoint::AfterRepositoryMaterialized) {
            return Err(remote_injected("after_repository_materialized"));
        }

        let mut result = OrchestrationRepositoryPrepareResult {
            schema_version: ORCHESTRATION_EXECUTION_SCHEMA_VERSION,
            request_sha256: request_sha256.clone(),
            receipt_sha256: zero_sha256(),
            node_store_id: self.node_store_id.clone(),
            environment_id: self.environment_id.clone(),
            identity_sha256,
            repository_key_sha256,
            repository_id: params.identity.repository_id.clone(),
            baseline_commit: params.identity.baseline_commit.clone(),
            baseline_present,
            private_artifact_ref: (!baseline_present)
                .then(|| staged_path.to_string_lossy().to_string()),
        };
        result.receipt_sha256 = sealed_sha256(&result, |value| {
            value.receipt_sha256 = zero_sha256();
        })?;
        self.persist_result(
            NodeReceiptKind::RepositoryPrepare,
            &params.identity,
            &request_sha256,
            &result,
            now_ms(),
        )?;
        Ok(result)
    }

    pub async fn attempt_prepare(
        &self,
        context: &RequestContext,
        params: OrchestrationAttemptPrepareParams,
        failpoint: Option<NodeExecutionFailpoint>,
    ) -> Result<OrchestrationAttemptPrepareResult, RpcError> {
        self.authorize(context)?;
        validate_identity(&params.identity)?;
        validate_attempt_prepare_params(&params)?;
        let service = self.clone();
        let (result, material) = tokio::task::spawn_blocking(move || {
            service.attempt_prepare_blocking(params, failpoint)
        })
        .await
        .map_err(remote_worker_error)??;
        self.enqueue_prepared(&result, &material).await?;
        Ok(result)
    }

    fn attempt_prepare_blocking(
        &self,
        params: OrchestrationAttemptPrepareParams,
        failpoint: Option<NodeExecutionFailpoint>,
    ) -> Result<(OrchestrationAttemptPrepareResult, PreparedMaterial), RpcError> {
        let request_sha256 = sha256_content(&params)?;
        if let Some(existing) = self.replay::<OrchestrationAttemptPrepareResult>(
            NodeReceiptKind::AttemptPrepare,
            &params.identity,
            &request_sha256,
        )? {
            let material = self.material_for_prepared(&params.identity, &existing)?;
            save_exact_thread(&self.store, &material.thread)?;
            save_exact_turn(&self.store, &material.turn)?;
            save_exact_turn_attempt(&self.store, &material.turn_attempt)?;
            return Ok((existing, material));
        }

        let repository = self.prepared_repository_for_identity(&params.identity)?;
        self.verify_repository_result(&repository)?;
        let (prepared_execution, material) = self.build_prepared_material(
            &params.identity,
            &params.worktree_id,
            &params.thread_id,
            &params.turn_id,
            &params.turn_attempt_id,
            &params.agent_id,
            params.access_ceiling,
            &params.compiled_instructions_sha256,
            &params.tool_policy_sha256,
            &params.prompt_sha256,
            &params.task_prompt,
            params.output_contract.as_deref(),
            params.prepared_at_ms,
            params.attempt_ordinal,
            params.run_concurrency_limit,
            &repository,
        )?;
        if failpoint == Some(NodeExecutionFailpoint::AfterAttemptWorktreeMaterialized) {
            return Err(remote_injected("after_attempt_worktree_materialized"));
        }

        save_exact_thread(&self.store, &material.thread)?;
        save_exact_turn(&self.store, &material.turn)?;
        save_exact_turn_attempt(&self.store, &material.turn_attempt)?;
        let prepared_execution_sha256 = sha256_content(&prepared_execution)?;
        let identity_sha256 = identity_sha256(&params.identity)?;
        let mut receipt = OrchestrationAttemptReceipt {
            schema_version: ORCHESTRATION_EXECUTION_SCHEMA_VERSION,
            request_sha256: request_sha256.clone(),
            receipt_sha256: zero_sha256(),
            node_store_id: self.node_store_id.clone(),
            environment_id: self.environment_id.clone(),
            identity_sha256,
            prepared_execution_sha256,
            run_id: params.identity.run_id.clone(),
            task_id: params.identity.task_id.clone(),
            attempt_id: params.identity.attempt_id.clone(),
            effect_id: params.identity.effect_id.clone(),
            state: OrchestrationNodeAttemptState::Prepared,
            thread_id: params.thread_id.clone(),
            turn_id: params.turn_id.clone(),
            turn_attempt_id: params.turn_attempt_id.clone(),
            boundary: AttemptBoundary::SafeToRetry,
            terminal_status: None,
            result_sha256: None,
            safe_error_code: None,
            updated_at_ms: params.prepared_at_ms,
        };
        seal_attempt_receipt(&mut receipt)?;
        let result = OrchestrationAttemptPrepareResult {
            receipt,
            prepared_execution,
        };
        self.persist_result(
            NodeReceiptKind::AttemptPrepare,
            &params.identity,
            &request_sha256,
            &result,
            params.prepared_at_ms,
        )?;
        if failpoint == Some(NodeExecutionFailpoint::AfterAttemptReceipt) {
            return Err(remote_injected("after_attempt_receipt"));
        }
        Ok((result, material))
    }

    pub async fn attempt_dispatch(
        &self,
        context: &RequestContext,
        params: OrchestrationAttemptDispatchParams,
        failpoint: Option<NodeExecutionFailpoint>,
    ) -> Result<OrchestrationAttemptReceipt, RpcError> {
        self.authorize(context)?;
        validate_identity(&params.identity)?;
        let expected_dispatch_identity =
            remote_dispatch_identity(&params.identity, &params.prepare_receipt_sha256)?;
        if params.dispatch_identity != expected_dispatch_identity
            || params.identity.idempotency_key != params.dispatch_identity
        {
            return Err(remote_invalid("remote_dispatch_identity_mismatch"));
        }
        let request_sha256 = sha256_content(&params)?;
        if let Some(existing) = self.replay::<OrchestrationAttemptReceipt>(
            NodeReceiptKind::AttemptDispatch,
            &params.identity,
            &request_sha256,
        )? {
            return Ok(existing);
        }
        let service = self.clone();
        let prepared_identity = params.identity.clone();
        let (prepared, material) = tokio::task::spawn_blocking(move || {
            let prepared = service.prepared_attempt_for_identity(&prepared_identity)?;
            let material = service.material_for_prepared(&prepared_identity, &prepared)?;
            Ok::<_, RpcError>((prepared, material))
        })
        .await
        .map_err(remote_worker_error)??;
        if prepared.receipt.receipt_sha256 != params.prepare_receipt_sha256 {
            return Err(remote_invalid("remote_prepare_receipt_mismatch"));
        }
        self.enqueue_prepared(&prepared, &material).await?;
        let queue = self
            .store
            .list_execution_queue_records()
            .map_err(store_rpc_error)?
            .into_iter()
            .find(|record| record.queue_id == material.queue_id)
            .ok_or_else(|| remote_invalid("remote_admission_missing"))?;
        if queue.state != QueueState::Queued {
            return Err(remote_conflict("remote_admission_not_claimable"));
        }
        if failpoint == Some(NodeExecutionFailpoint::BeforeDispatchMarker) {
            return Err(remote_injected("before_dispatch_marker"));
        }
        let claimed_at_ms = now_ms();
        let claim = self
            .admission
            .try_claim(ExecutionAdmissionClaimRequest {
                queue_id: queue.queue_id.clone(),
                expected_candidate_sha256: queue.candidate_sha256.clone(),
                expected_claim_revision: queue.claim_revision,
                lease_id: ExecutionLeaseId::new(format!(
                    "execution-lease-{}",
                    &sha256_hex(params.dispatch_identity.as_bytes())[..32]
                )),
                claimed_at_ms,
                ttl_ms: DEFAULT_EXECUTION_ADMISSION_TTL_MS,
                orchestration: None,
            })
            .await
            .map_err(admission_rpc_error)?;
        let Some(claim) = claim else {
            self.admission
                .cancel_queued(&queue)
                .await
                .map_err(admission_rpc_error)?;
            let mut receipt = self.attempt_receipt(
                &params.identity,
                &prepared,
                request_sha256.clone(),
                OrchestrationNodeAttemptState::NotStarted,
                Some("remote_capacity_unavailable".to_string()),
                claimed_at_ms,
            )?;
            receipt.result_sha256 = Some(sha256_content(&(
                "remote_capacity_unavailable",
                &params.identity.attempt_id,
            ))?);
            seal_attempt_receipt(&mut receipt)?;
            self.persist_result(
                NodeReceiptKind::AttemptDispatch,
                &params.identity,
                &request_sha256,
                &receipt,
                claimed_at_ms,
            )?;
            return Ok(receipt);
        };
        let mut marker = self.attempt_receipt(
            &params.identity,
            &prepared,
            request_sha256.clone(),
            OrchestrationNodeAttemptState::DispatchStarted,
            None,
            claimed_at_ms,
        )?;
        seal_attempt_receipt(&mut marker)?;
        self.persist_result(
            NodeReceiptKind::AttemptDispatch,
            &params.identity,
            &request_sha256,
            &marker,
            claimed_at_ms,
        )?;
        if failpoint == Some(NodeExecutionFailpoint::AfterDispatchMarker) {
            return Err(remote_injected("after_dispatch_marker"));
        }

        let dispatch = self
            .thread_manager
            .dispatch_prepared_turn(PreparedOrchestratedTurn {
                run_attempt_id: params.identity.attempt_id.clone(),
                thread: material.thread.clone(),
                turn: material.turn.clone(),
                turn_attempt: material.turn_attempt.clone(),
                session_spec: material.session_spec.clone(),
                admission_claim: claim,
                continuation_id: None,
                remote_authority: Some(RemotePreparedTurnAuthority {
                    node_store_id: self.node_store_id.clone(),
                    environment_id: self.environment_id.clone(),
                    identity_sha256: prepared.receipt.identity_sha256.clone(),
                    prepare_receipt_sha256: prepared.receipt.receipt_sha256.clone(),
                    dispatch_marker_idempotency_key: params.identity.idempotency_key.clone(),
                }),
            })
            .await;
        if failpoint == Some(NodeExecutionFailpoint::AfterProviderDispatch) {
            return Err(remote_injected("after_provider_dispatch"));
        }
        match dispatch {
            Ok(mut dispatch) => {
                let (state, safe_error_code) = match dispatch.receipt.disposition {
                    PreparedTurnDispatchDisposition::Accepted => {
                        (OrchestrationNodeAttemptState::Accepted, None)
                    }
                    PreparedTurnDispatchDisposition::NotStartedCleanupVerified => (
                        OrchestrationNodeAttemptState::NotStarted,
                        dispatch.receipt.safe_error_code.clone(),
                    ),
                    PreparedTurnDispatchDisposition::Ambiguous => {
                        (OrchestrationNodeAttemptState::Ambiguous, None)
                    }
                };
                self.persist_internal_attempt_receipt(
                    &params.identity,
                    &prepared,
                    &request_sha256,
                    InternalAttemptUpdate {
                        key: format!("{}:outcome", params.dispatch_identity),
                        state,
                        safe_error_code,
                        at_ms: now_ms(),
                    },
                )?;
                if state == OrchestrationNodeAttemptState::Accepted {
                    let authority = TerminalReceiptAuthority {
                        store: self.store.clone(),
                        node_store_id: self.node_store_id.clone(),
                        environment_id: self.environment_id.clone(),
                    };
                    let identity = params.identity.clone();
                    let prepared = prepared.clone();
                    let request_sha256 = request_sha256.clone();
                    let dispatch_identity = params.dispatch_identity.clone();
                    let handle = tokio::spawn(async move {
                        if let Some(event) = dispatch.events.recv().await {
                            let _ = persist_terminal_event(
                                &authority,
                                &identity,
                                &prepared,
                                &request_sha256,
                                &dispatch_identity,
                                event,
                            );
                        }
                    });
                    self.completion_tasks.lock().await.push(handle);
                }
            }
            Err(_) => {
                self.persist_internal_attempt_receipt(
                    &params.identity,
                    &prepared,
                    &request_sha256,
                    InternalAttemptUpdate {
                        key: format!("{}:outcome", params.dispatch_identity),
                        state: OrchestrationNodeAttemptState::Ambiguous,
                        safe_error_code: None,
                        at_ms: now_ms(),
                    },
                )?;
            }
        }
        Ok(marker)
    }

    pub async fn attempt_status(
        &self,
        context: &RequestContext,
        params: OrchestrationAttemptStatusParams,
    ) -> Result<OrchestrationAttemptStatusResult, RpcError> {
        self.authorize(context)?;
        validate_identity(&params.identity)?;
        let request_sha256 = sha256_content(&params)?;
        if let Some(existing) = self.replay::<OrchestrationAttemptStatusResult>(
            NodeReceiptKind::AttemptStatus,
            &params.identity,
            &request_sha256,
        )? {
            return Ok(existing);
        }
        let prepared = self.prepared_attempt_for_identity(&params.identity)?;
        let mut receipt = self.current_attempt_receipt(&params.identity, request_sha256.clone())?;
        if receipt.state == OrchestrationNodeAttemptState::DispatchStarted
            && self
                .thread_manager
                .prepared_turn_prompt_accepted(
                    &receipt.thread_id,
                    &receipt.turn_id,
                    &receipt.turn_attempt_id,
                )
                .await
        {
            receipt = self.persist_internal_attempt_receipt(
                &params.identity,
                &prepared,
                &request_sha256,
                InternalAttemptUpdate {
                    key: format!("{}:live-accepted", params.identity.idempotency_key),
                    state: OrchestrationNodeAttemptState::Accepted,
                    safe_error_code: None,
                    at_ms: now_ms(),
                },
            )?;
        }
        let result = OrchestrationAttemptStatusResult { receipt };
        self.persist_result(
            NodeReceiptKind::AttemptStatus,
            &params.identity,
            &request_sha256,
            &result,
            now_ms(),
        )?;
        Ok(result)
    }

    pub async fn attempt_cancel(
        &self,
        context: &RequestContext,
        params: OrchestrationAttemptCancelParams,
    ) -> Result<OrchestrationAttemptCancelResult, RpcError> {
        self.authorize(context)?;
        validate_identity(&params.identity)?;
        if params.reason_code.trim().is_empty() || params.reason_code.len() > 128 {
            return Err(remote_invalid("remote_cancel_reason_invalid"));
        }
        let request_sha256 = sha256_content(&params)?;
        if let Some(existing) = self.replay::<OrchestrationAttemptCancelResult>(
            NodeReceiptKind::AttemptCancel,
            &params.identity,
            &request_sha256,
        )? {
            return Ok(existing);
        }
        let prepared = self.prepared_attempt_for_identity(&params.identity)?;
        let current = self.current_attempt_receipt(&params.identity, request_sha256.clone())?;
        if matches!(
            current.state,
            OrchestrationNodeAttemptState::Terminal
                | OrchestrationNodeAttemptState::Cancelled
                | OrchestrationNodeAttemptState::NotStarted
                | OrchestrationNodeAttemptState::Ambiguous
        ) {
            let result = OrchestrationAttemptCancelResult { receipt: current };
            self.persist_result(
                NodeReceiptKind::AttemptCancel,
                &params.identity,
                &request_sha256,
                &result,
                now_ms(),
            )?;
            return Ok(result);
        }
        if matches!(
            current.state,
            OrchestrationNodeAttemptState::DispatchStarted
                | OrchestrationNodeAttemptState::Accepted
        ) && !self
            .thread_manager
            .prepared_turn_prompt_accepted(
                &current.thread_id,
                &current.turn_id,
                &current.turn_attempt_id,
            )
            .await
        {
            let mut receipt = self.attempt_receipt(
                &params.identity,
                &prepared,
                request_sha256.clone(),
                OrchestrationNodeAttemptState::Ambiguous,
                Some("remote_cancel_session_unverified".to_string()),
                now_ms(),
            )?;
            receipt.result_sha256 = Some(sha256_content(&(
                "remote_cancel_session_unverified",
                &params.identity.attempt_id,
                &params.reason_code,
            ))?);
            seal_attempt_receipt(&mut receipt)?;
            let result = OrchestrationAttemptCancelResult { receipt };
            self.persist_result(
                NodeReceiptKind::AttemptCancel,
                &params.identity,
                &request_sha256,
                &result,
                now_ms(),
            )?;
            return Ok(result);
        }
        if let Some(queue) = self
            .store
            .list_execution_queue_records()
            .map_err(store_rpc_error)?
            .into_iter()
            .find(|record| record.attempt_id == params.identity.attempt_id)
            && queue.state == QueueState::Queued
        {
            self.admission
                .cancel_queued(&queue)
                .await
                .map_err(admission_rpc_error)?;
        }
        self.thread_manager
            .interrupt_turn(ThreadCancelTurnParams {
                thread_id: prepared.receipt.thread_id.clone(),
                turn_id: Some(prepared.receipt.turn_id.clone()),
            })
            .await?;
        let mut receipt = self.attempt_receipt(
            &params.identity,
            &prepared,
            request_sha256.clone(),
            OrchestrationNodeAttemptState::Cancelled,
            Some(params.reason_code.clone()),
            now_ms(),
        )?;
        receipt.terminal_status = Some(AttemptStatus::Cancelled);
        receipt.result_sha256 = Some(sha256_content(&(
            "remote_cancelled",
            &params.identity.attempt_id,
            &params.reason_code,
        ))?);
        seal_attempt_receipt(&mut receipt)?;
        let result = OrchestrationAttemptCancelResult { receipt };
        self.persist_result(
            NodeReceiptKind::AttemptCancel,
            &params.identity,
            &request_sha256,
            &result,
            now_ms(),
        )?;
        Ok(result)
    }

    pub async fn attempt_events(
        &self,
        context: &RequestContext,
        params: OrchestrationAttemptEventsParams,
    ) -> Result<OrchestrationAttemptEventsResult, RpcError> {
        self.authorize(context)?;
        validate_identity(&params.identity)?;
        if params.limit == 0 || params.limit > MAX_REMOTE_EVENTS {
            return Err(remote_invalid("remote_events_limit_invalid"));
        }
        let request_sha256 = sha256_content(&params)?;
        if let Some(existing) = self.replay::<OrchestrationAttemptEventsResult>(
            NodeReceiptKind::AttemptEvents,
            &params.identity,
            &request_sha256,
        )? {
            return Ok(existing);
        }
        let prepared = self.prepared_attempt_for_identity(&params.identity)?;
        let events = self
            .store
            .get_events(&prepared.receipt.thread_id, Some(params.after_sequence))
            .map_err(store_rpc_error)?
            .into_iter()
            .take(params.limit as usize)
            .map(|event| {
                let payload = serde_json::to_value(&event.kind)
                    .map_err(|_| remote_internal("remote_event_encoding_failed"))?;
                Ok(OrchestrationNodeEvent {
                    sequence: event.seq,
                    at_ms: event.at_ms,
                    payload_sha256: sha256_content(&payload)?,
                    payload,
                })
            })
            .collect::<Result<Vec<_>, RpcError>>()?;
        if events
            .windows(2)
            .any(|pair| pair[0].sequence >= pair[1].sequence)
        {
            return Err(remote_internal("remote_event_order_invalid"));
        }
        let events_sha256 = sha256_content(&events)?;
        let terminal_receipt = self
            .current_attempt_receipt(&params.identity, request_sha256.clone())
            .ok()
            .filter(|receipt| {
                matches!(
                    receipt.state,
                    OrchestrationNodeAttemptState::Terminal
                        | OrchestrationNodeAttemptState::Cancelled
                        | OrchestrationNodeAttemptState::NotStarted
                        | OrchestrationNodeAttemptState::Ambiguous
                )
            });
        let result = OrchestrationAttemptEventsResult {
            schema_version: ORCHESTRATION_EXECUTION_SCHEMA_VERSION,
            request_sha256: request_sha256.clone(),
            events_sha256,
            node_store_id: self.node_store_id.clone(),
            environment_id: self.environment_id.clone(),
            identity_sha256: prepared.receipt.identity_sha256.clone(),
            events,
            terminal_receipt,
        };
        self.persist_result(
            NodeReceiptKind::AttemptEvents,
            &params.identity,
            &request_sha256,
            &result,
            now_ms(),
        )?;
        Ok(result)
    }

    pub async fn attempt_ack(
        &self,
        context: &RequestContext,
        params: OrchestrationAttemptAckParams,
    ) -> Result<OrchestrationAttemptAckResult, RpcError> {
        self.authorize(context)?;
        validate_identity(&params.identity)?;
        let request_sha256 = sha256_content(&params)?;
        if let Some(existing) = self.replay::<OrchestrationAttemptAckResult>(
            NodeReceiptKind::AttemptAck,
            &params.identity,
            &request_sha256,
        )? {
            return Ok(existing);
        }
        let prepared = self.prepared_attempt_for_identity(&params.identity)?;
        let mut matched = false;
        for record in self
            .store
            .list_node_receipts_for_attempt(&params.identity.attempt_id)
            .map_err(store_rpc_error)?
        {
            if record.node_store_id != self.node_store_id
                || record.request_kind != NodeReceiptKind::AttemptEvents
            {
                continue;
            }
            let result: OrchestrationAttemptEventsResult =
                serde_json::from_slice(&record.result_bytes)
                    .map_err(|_| remote_internal("remote_events_receipt_invalid"))?;
            if result.identity_sha256 == prepared.receipt.identity_sha256
                && result.events_sha256 == params.events_sha256
                && result
                    .events
                    .last()
                    .map_or(params.through_sequence == 0, |event| {
                        event.sequence == params.through_sequence
                    })
            {
                matched = true;
                break;
            }
        }
        if !matched {
            return Err(remote_invalid("remote_ack_events_mismatch"));
        }
        let mut result = OrchestrationAttemptAckResult {
            schema_version: ORCHESTRATION_EXECUTION_SCHEMA_VERSION,
            request_sha256: request_sha256.clone(),
            receipt_sha256: zero_sha256(),
            node_store_id: self.node_store_id.clone(),
            environment_id: self.environment_id.clone(),
            identity_sha256: prepared.receipt.identity_sha256.clone(),
            acknowledged_through_sequence: params.through_sequence,
        };
        result.receipt_sha256 = sealed_sha256(&result, |value| {
            value.receipt_sha256 = zero_sha256();
        })?;
        self.persist_result(
            NodeReceiptKind::AttemptAck,
            &params.identity,
            &request_sha256,
            &result,
            now_ms(),
        )?;
        Ok(result)
    }

    fn authorize(&self, context: &RequestContext) -> Result<(), RpcError> {
        if !matches!(context.peer_role, PeerRole::NodeProxy | PeerRole::Internal)
            || context.authenticated_environment_id != self.environment_id
        {
            return Err(RpcError::with_data(
                -32001,
                "Private orchestration execution requires this environment's authenticated node proxy",
                serde_json::json!({ "reason": "orchestration_node_proxy_required" }),
            ));
        }
        Ok(())
    }

    fn replay<T: serde::de::DeserializeOwned>(
        &self,
        kind: NodeReceiptKind,
        identity: &OrchestrationExecutionIdentity,
        request_sha256: &ContentSha256,
    ) -> Result<Option<T>, RpcError> {
        let Some(existing) = self
            .store
            .get_node_receipt(&self.node_store_id, kind, &identity.idempotency_key)
            .map_err(store_rpc_error)?
        else {
            return Ok(None);
        };
        if existing.run_id != identity.run_id
            || existing.task_id != identity.task_id
            || existing.attempt_id != identity.attempt_id
            || existing.effect_id != identity.effect_id
            || existing.request_sha256 != request_sha256.as_str()
        {
            return Err(remote_conflict("remote_idempotency_payload_conflict"));
        }
        let decoded = serde_json::from_slice(&existing.result_bytes)
            .map_err(|_| remote_internal("remote_receipt_decode_failed"))?;
        Ok(Some(decoded))
    }

    fn persist_result<T: Serialize>(
        &self,
        kind: NodeReceiptKind,
        identity: &OrchestrationExecutionIdentity,
        request_sha256: &ContentSha256,
        result: &T,
        at_ms: u64,
    ) -> Result<NodeReceiptRecord, RpcError> {
        let result_bytes = serde_json::to_vec(result)
            .map_err(|_| remote_internal("remote_receipt_encoding_failed"))?;
        let result_sha256 = sha256_hex(&result_bytes);
        let receipt_id = format!(
            "node-receipt-{}",
            &stable_digest(&[
                self.node_store_id.as_str(),
                node_receipt_kind_name(kind),
                &identity.idempotency_key,
                request_sha256.as_str(),
            ])[..32]
        );
        self.store
            .record_local_node_receipt(&NodeReceiptRecord {
                receipt_id,
                node_store_id: self.node_store_id.clone(),
                run_id: identity.run_id.clone(),
                task_id: identity.task_id.clone(),
                attempt_id: identity.attempt_id.clone(),
                effect_id: identity.effect_id.clone(),
                request_kind: kind,
                idempotency_key: identity.idempotency_key.clone(),
                request_sha256: request_sha256.as_str().to_string(),
                result_sha256,
                result_bytes,
                created_at_ms: at_ms,
                updated_at_ms: at_ms,
            })
            .map_err(store_rpc_error)
    }

    fn repository_path(&self, key: &ContentSha256) -> PathBuf {
        self.paths
            .recovery()
            .join("orchestration-repositories")
            .join(format!("{}.git", key.as_str()))
    }

    fn repository_staging_path(&self, key: &ContentSha256) -> PathBuf {
        self.paths
            .bundle_staging()
            .join(format!("orchestration-repository-{}.bundle", key.as_str()))
    }

    fn worktree_path(&self, worktree_id: &AttemptWorktreeId) -> PathBuf {
        self.paths
            .worktrees()
            .join("orchestration")
            .join(worktree_id.as_str())
    }

    fn materialize_repository(
        &self,
        repository_path: &Path,
        staged_path: &Path,
        baseline_commit: &str,
        repository_key: &str,
    ) -> Result<(), RpcError> {
        if !repository_path.exists() {
            self.managed_git
                .run(ManagedGitCommand::InitBare {
                    git_dir: repository_path.to_path_buf(),
                })
                .map_err(managed_git_rpc_error)?;
        }
        self.managed_git
            .run(ManagedGitCommand::VerifyBundle {
                git_dir: repository_path.to_path_buf(),
                bundle_path: staged_path.to_path_buf(),
            })
            .map_err(managed_git_rpc_error)?;
        let target = format!("refs/pandamux/baselines/{repository_key}");
        self.managed_git
            .run(ManagedGitCommand::Fetch {
                git_dir: repository_path.to_path_buf(),
                remote: crate::managed_git::git_process_path_string(staged_path),
                source: baseline_commit.to_string(),
                target,
            })
            .map_err(managed_git_rpc_error)?;
        verify_revision(
            &self.managed_git,
            repository_path,
            baseline_commit,
            "remote_baseline_missing",
        )
    }

    fn verify_repository_result(
        &self,
        result: &OrchestrationRepositoryPrepareResult,
    ) -> Result<(), RpcError> {
        if result.schema_version != ORCHESTRATION_EXECUTION_SCHEMA_VERSION
            || result.node_store_id != self.node_store_id
            || result.environment_id != self.environment_id
            || !result.baseline_present
        {
            return Err(remote_invalid("remote_repository_receipt_invalid"));
        }
        verify_revision(
            &self.managed_git,
            &self.repository_path(&result.repository_key_sha256),
            result.baseline_commit.as_str(),
            "remote_baseline_missing",
        )
    }

    fn prepared_repository_for_identity(
        &self,
        identity: &OrchestrationExecutionIdentity,
    ) -> Result<OrchestrationRepositoryPrepareResult, RpcError> {
        let expected_identity = identity_sha256(identity)?;
        let mut matching = Vec::new();
        for record in self
            .store
            .list_node_receipts_for_attempt(&identity.attempt_id)
            .map_err(store_rpc_error)?
        {
            if record.node_store_id != self.node_store_id
                || record.request_kind != NodeReceiptKind::RepositoryPrepare
            {
                continue;
            }
            let result: OrchestrationRepositoryPrepareResult =
                serde_json::from_slice(&record.result_bytes)
                    .map_err(|_| remote_internal("remote_repository_receipt_invalid"))?;
            if result.identity_sha256 == expected_identity
                && result.repository_id == identity.repository_id
                && result.baseline_commit == identity.baseline_commit
                && result.baseline_present
            {
                matching.push(result);
            }
        }
        matching
            .sort_by(|left, right| left.repository_key_sha256.cmp(&right.repository_key_sha256));
        matching
            .into_iter()
            .next()
            .ok_or_else(|| remote_invalid("remote_baseline_not_prepared"))
    }

    #[allow(clippy::too_many_arguments)]
    fn build_prepared_material(
        &self,
        identity: &OrchestrationExecutionIdentity,
        worktree_id: &AttemptWorktreeId,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        turn_attempt_id: &pandamux_core::TurnAttemptId,
        agent_id: &pandamux_core::AgentId,
        access_ceiling: pandamux_core::AccessMode,
        compiled_instructions_sha256: &ContentSha256,
        tool_policy_sha256: &ContentSha256,
        prompt_sha256: &ContentSha256,
        task_prompt: &str,
        output_contract: Option<&str>,
        prepared_at_ms: u64,
        attempt_ordinal: u32,
        run_concurrency_limit: u32,
        repository: &OrchestrationRepositoryPrepareResult,
    ) -> Result<(OrchestrationPreparedExecution, PreparedMaterial), RpcError> {
        let expected_worktree = AttemptWorktreeId::new(stable_execution_id(identity, "worktree"));
        let expected_thread = ThreadId::new(stable_execution_id(identity, "thread"));
        let expected_turn = TurnId::new(stable_execution_id(identity, "turn"));
        let expected_turn_attempt =
            pandamux_core::TurnAttemptId::new(stable_execution_id(identity, "turn-attempt"));
        if *worktree_id != expected_worktree
            || *thread_id != expected_thread
            || *turn_id != expected_turn
            || *turn_attempt_id != expected_turn_attempt
            || identity.route.task_id != identity.task_id
            || identity.route.selected_agent_id.as_ref() != Some(agent_id)
        {
            return Err(remote_invalid("remote_execution_identity_mismatch"));
        }
        let bundle = self
            .bundle_store
            .load_exact_compiled_bundle(&identity.bundle_sha)
            .map_err(|_| remote_invalid("remote_pinned_bundle_missing"))?;
        let agent = bundle
            .agents
            .get(agent_id)
            .ok_or_else(|| remote_invalid("remote_pinned_agent_missing"))?;
        if !agent.enabled
            || agent.bundle_sha != identity.bundle_sha.as_str()
            || access_ceiling.security_rank() > agent.access_mode.security_rank()
            || (!agent.environments.is_empty()
                && !agent.environments.contains(self.environment_id.as_str()))
            || agent.concurrency == 0
        {
            return Err(remote_invalid("remote_pinned_agent_authority_mismatch"));
        }
        let provider_instance_id = identity
            .route
            .provider
            .selected_provider_instance_id
            .clone()
            .ok_or_else(|| remote_invalid("remote_provider_route_missing"))?;
        let _model = identity
            .route
            .provider
            .selected_model
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| remote_invalid("remote_provider_model_missing"))?;
        let instructions = compiled_instructions(
            &bundle,
            agent,
            task_prompt.to_string(),
            output_contract.map(str::to_string),
        )?;
        let tool_policy = ToolPolicy {
            allowed: agent.tools.allow.iter().cloned().collect(),
            disallowed: agent.tools.deny.iter().cloned().collect(),
        };
        if sha256_content(task_prompt)? != *prompt_sha256
            || sha256_content(&instructions)? != *compiled_instructions_sha256
            || sha256_content(&tool_policy)? != *tool_policy_sha256
        {
            return Err(remote_invalid("remote_compiled_execution_digest_mismatch"));
        }
        let repository_path = self.repository_path(&repository.repository_key_sha256);
        let worktree_path = self.worktree_path(worktree_id);
        let branch_ref = remote_worktree_branch_ref(identity);
        ensure_private_directory(
            worktree_path
                .parent()
                .ok_or_else(|| remote_internal("remote_worktree_path_invalid"))?,
        )?;
        if !worktree_path.exists() {
            self.managed_git
                .run(ManagedGitCommand::WorktreeAdd {
                    git_dir: repository_path.clone(),
                    worktree_path: worktree_path.clone(),
                    branch_ref: branch_ref.clone(),
                    commit: identity.baseline_commit.as_str().to_string(),
                })
                .map_err(managed_git_rpc_error)?;
        }
        verify_worktree(
            &self.managed_git,
            &worktree_path,
            &repository_path,
            identity.baseline_commit.as_str(),
            &branch_ref,
        )?;
        let prepared_execution = OrchestrationPreparedExecution {
            worktree_id: worktree_id.clone(),
            agent_id: agent_id.clone(),
            organization_id: bundle.organization_id.clone(),
            provider_instance_id: provider_instance_id.clone(),
            access_ceiling,
            compiled_instructions_sha256: compiled_instructions_sha256.clone(),
            tool_policy_sha256: tool_policy_sha256.clone(),
            prompt_sha256: prompt_sha256.clone(),
            task_prompt: task_prompt.to_string(),
            output_contract: output_contract.map(str::to_string),
            prepared_at_ms,
            attempt_ordinal,
            run_concurrency_limit,
            agent_concurrency_limit: agent.concurrency,
        };
        let material = material_from_parts(
            identity,
            &prepared_execution,
            PreparedMaterialScope {
                environment_id: self.environment_id.clone(),
                project_id: bundle.project_id,
                effort: agent.effort.clone(),
            },
            instructions,
            tool_policy,
            worktree_path,
        );
        Ok((prepared_execution, material))
    }

    fn material_for_prepared(
        &self,
        identity: &OrchestrationExecutionIdentity,
        prepared: &OrchestrationAttemptPrepareResult,
    ) -> Result<PreparedMaterial, RpcError> {
        if prepared.receipt.schema_version != ORCHESTRATION_EXECUTION_SCHEMA_VERSION
            || prepared.receipt.node_store_id != self.node_store_id
            || prepared.receipt.environment_id != self.environment_id
            || prepared.receipt.identity_sha256 != identity_sha256(identity)?
            || prepared.receipt.run_id != identity.run_id
            || prepared.receipt.task_id != identity.task_id
            || prepared.receipt.attempt_id != identity.attempt_id
            || prepared.receipt.effect_id != identity.effect_id
            || prepared.receipt.state != OrchestrationNodeAttemptState::Prepared
            || prepared.receipt.prepared_execution_sha256
                != sha256_content(&prepared.prepared_execution)?
        {
            return Err(remote_invalid("remote_prepare_receipt_invalid"));
        }
        let bundle = self
            .bundle_store
            .load_exact_compiled_bundle(&identity.bundle_sha)
            .map_err(|_| remote_invalid("remote_pinned_bundle_missing"))?;
        let agent = bundle
            .agents
            .get(&prepared.prepared_execution.agent_id)
            .ok_or_else(|| remote_invalid("remote_pinned_agent_missing"))?;
        let instructions = compiled_instructions(
            &bundle,
            agent,
            prepared.prepared_execution.task_prompt.clone(),
            prepared.prepared_execution.output_contract.clone(),
        )?;
        let tool_policy = ToolPolicy {
            allowed: agent.tools.allow.iter().cloned().collect(),
            disallowed: agent.tools.deny.iter().cloned().collect(),
        };
        if sha256_content(&instructions)?
            != prepared.prepared_execution.compiled_instructions_sha256
            || sha256_content(&tool_policy)? != prepared.prepared_execution.tool_policy_sha256
            || sha256_content(&prepared.prepared_execution.task_prompt)?
                != prepared.prepared_execution.prompt_sha256
            || agent.concurrency != prepared.prepared_execution.agent_concurrency_limit
            || prepared.prepared_execution.agent_id != agent.id
            || prepared.prepared_execution.organization_id != bundle.organization_id
            || identity.route.selected_agent_id.as_ref() != Some(&agent.id)
            || identity
                .route
                .provider
                .selected_provider_instance_id
                .as_ref()
                != Some(&prepared.prepared_execution.provider_instance_id)
            || prepared.prepared_execution.access_ceiling.security_rank()
                > agent.access_mode.security_rank()
            || prepared.prepared_execution.run_concurrency_limit == 0
        {
            return Err(remote_invalid("remote_prepared_execution_changed"));
        }
        let repository = self.prepared_repository_for_identity(identity)?;
        self.verify_repository_result(&repository)?;
        let worktree_path = self.worktree_path(&prepared.prepared_execution.worktree_id);
        verify_worktree(
            &self.managed_git,
            &worktree_path,
            &self.repository_path(&repository.repository_key_sha256),
            identity.baseline_commit.as_str(),
            &remote_worktree_branch_ref(identity),
        )?;
        Ok(material_from_parts(
            identity,
            &prepared.prepared_execution,
            PreparedMaterialScope {
                environment_id: self.environment_id.clone(),
                project_id: bundle.project_id,
                effort: agent.effort.clone(),
            },
            instructions,
            tool_policy,
            worktree_path,
        ))
    }

    async fn enqueue_prepared(
        &self,
        prepared: &OrchestrationAttemptPrepareResult,
        material: &PreparedMaterial,
    ) -> Result<(), RpcError> {
        self.admission
            .enqueue_remote_orchestrated(EnqueueRemoteOrchestratedAdmissionRequest {
                queue_id: material.queue_id.clone(),
                run_id: prepared.receipt.run_id.clone(),
                task_id: prepared.receipt.task_id.clone(),
                attempt_id: prepared.receipt.attempt_id.clone(),
                agent_id: prepared.prepared_execution.agent_id.clone(),
                organization_id: prepared.prepared_execution.organization_id.clone(),
                agent_concurrency_limit: prepared.prepared_execution.agent_concurrency_limit,
                provider_instance_id: prepared.prepared_execution.provider_instance_id.clone(),
                environment_id: prepared.receipt.environment_id.clone(),
                enqueued_at_ms: prepared.prepared_execution.prepared_at_ms,
            })
            .await
            .map_err(admission_rpc_error)?;
        Ok(())
    }

    fn prepared_attempt_for_identity(
        &self,
        identity: &OrchestrationExecutionIdentity,
    ) -> Result<OrchestrationAttemptPrepareResult, RpcError> {
        let expected_identity = identity_sha256(identity)?;
        let mut matching: Option<OrchestrationAttemptPrepareResult> = None;
        for record in self
            .store
            .list_node_receipts_for_attempt(&identity.attempt_id)
            .map_err(store_rpc_error)?
        {
            if record.node_store_id != self.node_store_id
                || record.request_kind != NodeReceiptKind::AttemptPrepare
            {
                continue;
            }
            let result: OrchestrationAttemptPrepareResult =
                serde_json::from_slice(&record.result_bytes)
                    .map_err(|_| remote_internal("remote_prepare_receipt_invalid"))?;
            if result.receipt.identity_sha256 != expected_identity {
                continue;
            }
            if let Some(existing) = matching.as_ref()
                && existing != &result
            {
                return Err(remote_conflict("remote_prepare_receipts_conflict"));
            }
            matching = Some(result);
        }
        matching.ok_or_else(|| remote_invalid("remote_attempt_not_prepared"))
    }

    fn attempt_receipt(
        &self,
        identity: &OrchestrationExecutionIdentity,
        prepared: &OrchestrationAttemptPrepareResult,
        request_sha256: ContentSha256,
        state: OrchestrationNodeAttemptState,
        safe_error_code: Option<String>,
        at_ms: u64,
    ) -> Result<OrchestrationAttemptReceipt, RpcError> {
        let boundary = self
            .store
            .get_turn_attempt(&prepared.receipt.turn_attempt_id)
            .map_err(store_rpc_error)?
            .map_or(prepared.receipt.boundary, |attempt| attempt.boundary);
        Ok(OrchestrationAttemptReceipt {
            schema_version: ORCHESTRATION_EXECUTION_SCHEMA_VERSION,
            request_sha256,
            receipt_sha256: zero_sha256(),
            node_store_id: self.node_store_id.clone(),
            environment_id: self.environment_id.clone(),
            identity_sha256: prepared.receipt.identity_sha256.clone(),
            prepared_execution_sha256: prepared.receipt.prepared_execution_sha256.clone(),
            run_id: identity.run_id.clone(),
            task_id: identity.task_id.clone(),
            attempt_id: identity.attempt_id.clone(),
            effect_id: identity.effect_id.clone(),
            state,
            thread_id: prepared.receipt.thread_id.clone(),
            turn_id: prepared.receipt.turn_id.clone(),
            turn_attempt_id: prepared.receipt.turn_attempt_id.clone(),
            boundary,
            terminal_status: None,
            result_sha256: None,
            safe_error_code,
            updated_at_ms: at_ms,
        })
    }

    fn persist_internal_attempt_receipt(
        &self,
        identity: &OrchestrationExecutionIdentity,
        prepared: &OrchestrationAttemptPrepareResult,
        request_sha256: &ContentSha256,
        update: InternalAttemptUpdate,
    ) -> Result<OrchestrationAttemptReceipt, RpcError> {
        let mut receipt = self.attempt_receipt(
            identity,
            prepared,
            request_sha256.clone(),
            update.state,
            update.safe_error_code,
            update.at_ms,
        )?;
        receipt.result_sha256 = Some(sha256_content(&(
            "remote_attempt_state",
            identity.attempt_id.as_str(),
            update.state,
            receipt.boundary,
            &receipt.safe_error_code,
        ))?);
        seal_attempt_receipt(&mut receipt)?;
        let mut internal_identity = identity.clone();
        internal_identity.idempotency_key = update.key;
        self.persist_result(
            NodeReceiptKind::AttemptStatus,
            &internal_identity,
            request_sha256,
            &receipt,
            update.at_ms,
        )?;
        Ok(receipt)
    }

    fn current_attempt_receipt(
        &self,
        identity: &OrchestrationExecutionIdentity,
        request_sha256: ContentSha256,
    ) -> Result<OrchestrationAttemptReceipt, RpcError> {
        let prepared = self.prepared_attempt_for_identity(identity)?;
        let mut best: Option<OrchestrationAttemptReceipt> = None;
        let mut dispatch_started = false;
        for record in self
            .store
            .list_node_receipts_for_attempt(&identity.attempt_id)
            .map_err(store_rpc_error)?
        {
            if record.node_store_id != self.node_store_id {
                continue;
            }
            let receipt = match record.request_kind {
                NodeReceiptKind::AttemptDispatch | NodeReceiptKind::AttemptStatus => {
                    serde_json::from_slice::<OrchestrationAttemptReceipt>(&record.result_bytes).ok()
                }
                NodeReceiptKind::AttemptCancel => {
                    serde_json::from_slice::<OrchestrationAttemptCancelResult>(&record.result_bytes)
                        .ok()
                        .map(|value| value.receipt)
                }
                _ => None,
            };
            let Some(receipt) = receipt else {
                continue;
            };
            if receipt.identity_sha256 != prepared.receipt.identity_sha256 {
                continue;
            }
            dispatch_started |= receipt.state == OrchestrationNodeAttemptState::DispatchStarted;
            if best.as_ref().is_none_or(|current| {
                attempt_state_rank(receipt.state) > attempt_state_rank(current.state)
                    || (attempt_state_rank(receipt.state) == attempt_state_rank(current.state)
                        && receipt.updated_at_ms > current.updated_at_ms)
            }) {
                best = Some(receipt);
            }
        }
        let mut receipt = best.unwrap_or_else(|| prepared.receipt.clone());
        if dispatch_started && receipt.state == OrchestrationNodeAttemptState::Prepared {
            receipt.state = OrchestrationNodeAttemptState::DispatchStarted;
        }
        receipt.request_sha256 = request_sha256;
        receipt.updated_at_ms = now_ms();
        if let Some(attempt) = self
            .store
            .get_turn_attempt(&receipt.turn_attempt_id)
            .map_err(store_rpc_error)?
        {
            receipt.boundary = attempt.boundary;
        }
        if matches!(
            receipt.state,
            OrchestrationNodeAttemptState::DispatchStarted
                | OrchestrationNodeAttemptState::Accepted
        ) && let Some(turn) = self
            .store
            .get_turn(&receipt.turn_id)
            .map_err(store_rpc_error)?
        {
            let terminal_status = match turn.status {
                TurnStatus::Completed => Some(AttemptStatus::Succeeded),
                TurnStatus::Failed => Some(AttemptStatus::Failed),
                TurnStatus::Interrupted => Some(AttemptStatus::Interrupted),
                TurnStatus::Requested
                | TurnStatus::Running
                | TurnStatus::AwaitingApproval
                | TurnStatus::InterruptedForCapacity => None,
            };
            if let Some(terminal_status) = terminal_status {
                receipt.state = OrchestrationNodeAttemptState::Terminal;
                receipt.terminal_status = Some(terminal_status);
                receipt.updated_at_ms = turn.ended_at_ms.unwrap_or(receipt.updated_at_ms);
                receipt.result_sha256 = Some(sha256_content(&(
                    "remote_durable_turn_terminal",
                    identity.attempt_id.as_str(),
                    terminal_status,
                    receipt.boundary,
                    turn.ended_at_ms,
                    turn.checkpoint_after,
                ))?);
                if terminal_status == AttemptStatus::Failed {
                    receipt.safe_error_code = Some("remote_provider_failed".to_string());
                }
            }
        }
        seal_attempt_receipt(&mut receipt)?;
        Ok(receipt)
    }
}

pub fn remote_execution_ids(
    identity: &OrchestrationExecutionIdentity,
) -> (
    AttemptWorktreeId,
    ThreadId,
    TurnId,
    pandamux_core::TurnAttemptId,
) {
    (
        AttemptWorktreeId::new(stable_execution_id(identity, "worktree")),
        ThreadId::new(stable_execution_id(identity, "thread")),
        TurnId::new(stable_execution_id(identity, "turn")),
        pandamux_core::TurnAttemptId::new(stable_execution_id(identity, "turn-attempt")),
    )
}

pub fn remote_dispatch_identity(
    identity: &OrchestrationExecutionIdentity,
    prepare_receipt_sha256: &ContentSha256,
) -> Result<String, RpcError> {
    let identity_sha = identity_sha256(identity)?;
    Ok(format!(
        "orchestration-dispatch-{}",
        &stable_digest(&[
            identity_sha.as_str(),
            prepare_receipt_sha256.as_str(),
            &identity.lease_generation.to_string(),
        ])[..32]
    ))
}

pub fn remote_identity_sha256(
    identity: &OrchestrationExecutionIdentity,
) -> Result<ContentSha256, RpcError> {
    identity_sha256(identity)
}

pub fn remote_repository_key_sha256(
    identity: &OrchestrationExecutionIdentity,
    baseline_bundle_sha256: &pandamux_core::ArchiveSha256,
    baseline_bundle_bytes_sha256: &ContentSha256,
) -> Result<ContentSha256, RpcError> {
    sha256_content(&(
        "orchestration_repository_v1",
        &identity.repository_id,
        &identity.baseline_commit,
        &identity.bundle_sha,
        baseline_bundle_sha256,
        baseline_bundle_bytes_sha256,
    ))
}

pub fn remote_request_sha256<T: Serialize + ?Sized>(value: &T) -> Result<ContentSha256, RpcError> {
    sha256_content(value)
}

pub fn remote_preparation_hashes(
    bundle: &pandamux_core::CompiledInstructionBundleV1,
    agent_id: &pandamux_core::AgentId,
    task_prompt: &str,
    output_contract: Option<&str>,
) -> Result<(ContentSha256, ContentSha256, ContentSha256), RpcError> {
    let agent = bundle
        .agents
        .get(agent_id)
        .ok_or_else(|| remote_invalid("remote_pinned_agent_missing"))?;
    let instructions = compiled_instructions(
        bundle,
        agent,
        task_prompt.to_string(),
        output_contract.map(str::to_string),
    )?;
    let tool_policy = ToolPolicy {
        allowed: agent.tools.allow.iter().cloned().collect(),
        disallowed: agent.tools.deny.iter().cloned().collect(),
    };
    Ok((
        sha256_content(&instructions)?,
        sha256_content(&tool_policy)?,
        sha256_content(task_prompt)?,
    ))
}

fn material_from_parts(
    identity: &OrchestrationExecutionIdentity,
    prepared: &OrchestrationPreparedExecution,
    scope: PreparedMaterialScope,
    instructions: InjectedInstructions,
    tool_policy: ToolPolicy,
    worktree_path: PathBuf,
) -> PreparedMaterial {
    let model = identity
        .route
        .provider
        .selected_model
        .clone()
        .expect("validated remote route has a model");
    let worktree_path_text = worktree_path.to_string_lossy().to_string();
    let thread_id = ThreadId::new(stable_execution_id(identity, "thread"));
    let turn_id = TurnId::new(stable_execution_id(identity, "turn"));
    let turn_attempt_id =
        pandamux_core::TurnAttemptId::new(stable_execution_id(identity, "turn-attempt"));
    let branch_ref = remote_worktree_branch_ref(identity);
    let thread = Thread {
        id: thread_id.clone(),
        project_id: Some(scope.project_id),
        environment_id: scope.environment_id,
        parent_thread_id: None,
        title: format!("Orchestration task {}", identity.task_id.as_str()),
        provider_instance_id: prepared.provider_instance_id.clone(),
        model: model.clone(),
        effort: scope.effort.clone(),
        access_mode: prepared.access_ceiling,
        workspace: ThreadWorkspace {
            cwd: worktree_path_text.clone(),
            worktree: Some(WorktreeRef {
                path: worktree_path_text.clone(),
                branch: branch_ref,
                base_ref: identity.baseline_commit.as_str().to_string(),
            }),
        },
        status: ThreadStatus::Idle,
        agent: Some(AgentInstanceRef {
            agent_id: prepared.agent_id.clone(),
            bundle_sha: identity.bundle_sha.as_str().to_string(),
        }),
        origin: ThreadOrigin::Orchestrator {
            run_id: identity.run_id.clone(),
            task_id: identity.task_id.as_str().to_string(),
        },
        created_at_ms: prepared.prepared_at_ms,
        updated_at_ms: prepared.prepared_at_ms,
    };
    let turn = Turn {
        id: turn_id.clone(),
        thread_id: thread_id.clone(),
        environment_id: thread.environment_id.clone(),
        provider_instance_id: prepared.provider_instance_id.clone(),
        seq: 1,
        input: TurnInput {
            text: prepared.task_prompt.clone(),
            attachment_ids: Vec::new(),
            attachments: Vec::new(),
            model: Some(model.clone()),
            effort: scope.effort.clone(),
        },
        status: TurnStatus::Running,
        started_at_ms: prepared.prepared_at_ms,
        ended_at_ms: None,
        usage: None,
        checkpoint_before: Some(identity.baseline_commit.as_str().to_string()),
        checkpoint_after: None,
    };
    let turn_attempt = TurnAttempt {
        id: turn_attempt_id,
        turn_id: turn_id.clone(),
        thread_id: thread_id.clone(),
        ordinal: prepared.attempt_ordinal,
        environment_id: thread.environment_id.clone(),
        provider_instance_id: prepared.provider_instance_id.clone(),
        subscription_key: identity.route.provider.selected_subscription_key.clone(),
        model: model.clone(),
        boundary: AttemptBoundary::SafeToRetry,
        status: TurnAttemptStatus::Running,
        routing: identity.route.provider.clone(),
        execution: None,
        checkpoint: Some(identity.baseline_commit.as_str().to_string()),
        partial_output: String::new(),
        capacity_evidence: None,
        started_at_ms: prepared.prepared_at_ms,
        ended_at_ms: None,
    };
    PreparedMaterial {
        thread,
        turn,
        turn_attempt,
        session_spec: SessionSpec {
            cwd: worktree_path,
            model,
            effort: scope.effort,
            access: prepared.access_ceiling,
            instructions: Some(instructions),
            tool_policy: Some(tool_policy),
            resume: None,
        },
        queue_id: format!(
            "remote-orchestration-admission-{}",
            &sha256_hex(identity.attempt_id.as_str().as_bytes())[..32]
        ),
    }
}

fn compiled_instructions(
    bundle: &pandamux_core::CompiledInstructionBundleV1,
    agent: &pandamux_core::AgentDefinition,
    task_prompt: String,
    output_contract: Option<String>,
) -> Result<InjectedInstructions, RpcError> {
    let mut knowledge_files = Vec::new();
    for knowledge_path in &agent.knowledge {
        let reference = bundle
            .references
            .get(&agent.source_path)
            .and_then(|references| {
                references.iter().find(|reference| {
                    reference.expected_kind == "pandamux.knowledge"
                        && reference.path == *knowledge_path
                })
            })
            .ok_or_else(|| remote_invalid("remote_knowledge_reference_unresolved"))?;
        let knowledge = bundle
            .knowledge
            .get(&reference.resolved_id)
            .ok_or_else(|| remote_invalid("remote_knowledge_missing"))?;
        knowledge_files.push((knowledge.id.clone(), knowledge.content.clone()));
    }
    Ok(InjectedInstructions {
        role: format!(
            "Agent identity: {}\n\n{}",
            agent.id.as_str(),
            agent.instructions
        ),
        knowledge_files,
        task_prompt,
        output_contract,
    })
}

fn validate_attempt_prepare_params(
    params: &OrchestrationAttemptPrepareParams,
) -> Result<(), RpcError> {
    if params.prepared_at_ms == 0
        || params.attempt_ordinal == 0
        || params.run_concurrency_limit == 0
        || params.task_prompt.is_empty()
        || params.task_prompt.len() > MAX_REMOTE_TASK_PROMPT_BYTES
        || params
            .output_contract
            .as_ref()
            .is_some_and(|value| value.len() > MAX_REMOTE_OUTPUT_CONTRACT_BYTES)
    {
        return Err(remote_invalid("remote_attempt_prepare_invalid"));
    }
    Ok(())
}

fn validate_identity(identity: &OrchestrationExecutionIdentity) -> Result<(), RpcError> {
    if identity.schema_version != ORCHESTRATION_EXECUTION_SCHEMA_VERSION
        || identity.run_id.as_str().trim().is_empty()
        || identity.task_id.as_str().trim().is_empty()
        || identity.attempt_id.as_str().trim().is_empty()
        || identity.effect_id.as_str().trim().is_empty()
        || identity.repository_id.as_str().trim().is_empty()
        || identity.idempotency_key.trim().is_empty()
        || identity.idempotency_key.len() > 512
        || identity.lease_generation == 0
        || identity.route.task_id != identity.task_id
    {
        return Err(remote_invalid("remote_execution_identity_invalid"));
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IdentityBinding<'a> {
    schema_version: u32,
    run_id: &'a pandamux_core::RunId,
    task_id: &'a pandamux_core::RunTaskId,
    attempt_id: &'a RunAttemptId,
    effect_id: &'a pandamux_core::OrchestrationEffectId,
    payload_sha256: &'a ContentSha256,
    repository_id: &'a pandamux_core::ProjectRepositoryId,
    baseline_commit: &'a pandamux_core::SourceCommitSha,
    bundle_sha: &'a BundleSha,
    route: &'a pandamux_core::TaskRouteDecision,
    lease_generation: u64,
}

fn identity_sha256(identity: &OrchestrationExecutionIdentity) -> Result<ContentSha256, RpcError> {
    sha256_content(&IdentityBinding {
        schema_version: identity.schema_version,
        run_id: &identity.run_id,
        task_id: &identity.task_id,
        attempt_id: &identity.attempt_id,
        effect_id: &identity.effect_id,
        payload_sha256: &identity.payload_sha256,
        repository_id: &identity.repository_id,
        baseline_commit: &identity.baseline_commit,
        bundle_sha: &identity.bundle_sha,
        route: &identity.route,
        lease_generation: identity.lease_generation,
    })
}

fn stable_execution_id(identity: &OrchestrationExecutionIdentity, kind: &str) -> String {
    format!(
        "{kind}-orchestration-{}",
        &stable_digest(&[
            kind,
            identity.run_id.as_str(),
            identity.task_id.as_str(),
            identity.attempt_id.as_str(),
            identity.effect_id.as_str(),
        ])[..32]
    )
}

fn remote_worktree_branch_ref(identity: &OrchestrationExecutionIdentity) -> String {
    format!(
        "refs/heads/pandamux/orchestration/{}",
        &stable_digest(&[
            "worktree-branch",
            identity.run_id.as_str(),
            identity.task_id.as_str(),
            identity.attempt_id.as_str(),
            identity.effect_id.as_str(),
        ])[..32]
    )
}

fn persist_terminal_event(
    authority: &TerminalReceiptAuthority,
    identity: &OrchestrationExecutionIdentity,
    prepared: &OrchestrationAttemptPrepareResult,
    request_sha256: &ContentSha256,
    dispatch_identity: &str,
    event: PreparedTurnEvent,
) -> Result<(), RpcError> {
    let (state, terminal_status, safe_error_code, at_ms) = match event {
        PreparedTurnEvent::Terminal { outcome, at_ms } => match outcome {
            pandamux_core::TurnOutcome::Success => (
                OrchestrationNodeAttemptState::Terminal,
                Some(AttemptStatus::Succeeded),
                None,
                at_ms,
            ),
            pandamux_core::TurnOutcome::Cancelled => (
                OrchestrationNodeAttemptState::Cancelled,
                Some(AttemptStatus::Cancelled),
                None,
                at_ms,
            ),
            pandamux_core::TurnOutcome::Failed => (
                OrchestrationNodeAttemptState::Terminal,
                Some(AttemptStatus::Failed),
                None,
                at_ms,
            ),
            pandamux_core::TurnOutcome::Interrupted => (
                OrchestrationNodeAttemptState::Terminal,
                Some(AttemptStatus::Interrupted),
                None,
                at_ms,
            ),
        },
        PreparedTurnEvent::CapacityInterrupted { at_ms, .. } => (
            OrchestrationNodeAttemptState::Terminal,
            Some(AttemptStatus::Interrupted),
            Some("remote_capacity_interrupted".to_string()),
            at_ms,
        ),
        PreparedTurnEvent::StreamClosed { at_ms } => (
            OrchestrationNodeAttemptState::Ambiguous,
            None,
            Some("remote_provider_stream_closed".to_string()),
            at_ms,
        ),
    };
    let boundary = authority
        .store
        .get_turn_attempt(&prepared.receipt.turn_attempt_id)
        .map_err(store_rpc_error)?
        .map_or(prepared.receipt.boundary, |attempt| attempt.boundary);
    let mut receipt = OrchestrationAttemptReceipt {
        schema_version: ORCHESTRATION_EXECUTION_SCHEMA_VERSION,
        request_sha256: request_sha256.clone(),
        receipt_sha256: zero_sha256(),
        node_store_id: authority.node_store_id.clone(),
        environment_id: authority.environment_id.clone(),
        identity_sha256: prepared.receipt.identity_sha256.clone(),
        prepared_execution_sha256: prepared.receipt.prepared_execution_sha256.clone(),
        run_id: identity.run_id.clone(),
        task_id: identity.task_id.clone(),
        attempt_id: identity.attempt_id.clone(),
        effect_id: identity.effect_id.clone(),
        state,
        thread_id: prepared.receipt.thread_id.clone(),
        turn_id: prepared.receipt.turn_id.clone(),
        turn_attempt_id: prepared.receipt.turn_attempt_id.clone(),
        boundary,
        terminal_status,
        result_sha256: Some(sha256_content(&(
            "remote_terminal",
            identity.attempt_id.as_str(),
            state,
            boundary,
            terminal_status,
            &safe_error_code,
        ))?),
        safe_error_code,
        updated_at_ms: at_ms,
    };
    seal_attempt_receipt(&mut receipt)?;
    let result_bytes = serde_json::to_vec(&receipt)
        .map_err(|_| remote_internal("remote_receipt_encoding_failed"))?;
    let key = format!("{dispatch_identity}:terminal");
    authority
        .store
        .record_local_node_receipt(&NodeReceiptRecord {
            receipt_id: format!(
                "node-receipt-{}",
                &stable_digest(&[
                    authority.node_store_id.as_str(),
                    "attempt_status",
                    &key,
                    request_sha256.as_str(),
                ])[..32]
            ),
            node_store_id: authority.node_store_id.clone(),
            run_id: identity.run_id.clone(),
            task_id: identity.task_id.clone(),
            attempt_id: identity.attempt_id.clone(),
            effect_id: identity.effect_id.clone(),
            request_kind: NodeReceiptKind::AttemptStatus,
            idempotency_key: key,
            request_sha256: request_sha256.as_str().to_string(),
            result_sha256: sha256_hex(&result_bytes),
            result_bytes,
            created_at_ms: at_ms,
            updated_at_ms: at_ms,
        })
        .map_err(store_rpc_error)?;
    Ok(())
}

fn verify_worktree(
    git: &ManagedGitRunner,
    worktree_path: &Path,
    repository_path: &Path,
    baseline_commit: &str,
    branch_ref: &str,
) -> Result<(), RpcError> {
    let head = git
        .run(ManagedGitCommand::WorktreeHead {
            worktree_path: worktree_path.to_path_buf(),
        })
        .map_err(managed_git_rpc_error)?;
    if String::from_utf8_lossy(&head.stdout).trim() != baseline_commit {
        return Err(remote_conflict("remote_worktree_baseline_changed"));
    }
    let branch = git
        .run(ManagedGitCommand::WorktreeBranch {
            worktree_path: worktree_path.to_path_buf(),
        })
        .map_err(managed_git_rpc_error)?;
    if String::from_utf8_lossy(&branch.stdout).trim() != branch_ref {
        return Err(remote_conflict("remote_worktree_branch_changed"));
    }
    let common = git
        .run(ManagedGitCommand::WorktreeCommonDir {
            worktree_path: worktree_path.to_path_buf(),
        })
        .map_err(managed_git_rpc_error)?;
    let actual = PathBuf::from(String::from_utf8_lossy(&common.stdout).trim());
    let actual = actual
        .canonicalize()
        .map_err(|_| remote_internal("remote_worktree_common_dir_unavailable"))?;
    let expected = repository_path
        .canonicalize()
        .map_err(|_| remote_internal("remote_repository_path_unavailable"))?;
    if actual != expected {
        return Err(remote_conflict("remote_worktree_owner_changed"));
    }
    Ok(())
}

fn verify_revision(
    git: &ManagedGitRunner,
    repository_path: &Path,
    baseline_commit: &str,
    missing_reason: &'static str,
) -> Result<(), RpcError> {
    let output = git
        .run(ManagedGitCommand::RevParse {
            git_dir: repository_path.to_path_buf(),
            revision: baseline_commit.to_string(),
        })
        .map_err(|_| remote_invalid(missing_reason))?;
    if String::from_utf8_lossy(&output.stdout).trim() != baseline_commit {
        return Err(remote_invalid(missing_reason));
    }
    Ok(())
}

fn save_exact_thread(store: &Store, value: &Thread) -> Result<(), RpcError> {
    match store.get_thread(&value.id).map_err(store_rpc_error)? {
        Some(existing) if existing == *value => Ok(()),
        Some(_) => Err(remote_conflict("remote_thread_identity_conflict")),
        None => store.save_thread(value).map_err(store_rpc_error),
    }
}

fn save_exact_turn(store: &Store, value: &Turn) -> Result<(), RpcError> {
    match store.get_turn(&value.id).map_err(store_rpc_error)? {
        Some(existing) if existing == *value => Ok(()),
        Some(_) => Err(remote_conflict("remote_turn_identity_conflict")),
        None => store.save_turn(value).map_err(store_rpc_error),
    }
}

fn save_exact_turn_attempt(store: &Store, value: &TurnAttempt) -> Result<(), RpcError> {
    match store.get_turn_attempt(&value.id).map_err(store_rpc_error)? {
        Some(existing) if existing == *value => Ok(()),
        Some(_) => Err(remote_conflict("remote_turn_attempt_identity_conflict")),
        None => store.save_turn_attempt(value).map_err(store_rpc_error),
    }
}

fn persist_private_file(path: &Path, bytes: &[u8]) -> Result<(), RpcError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(remote_conflict("remote_repository_staging_unsafe"));
            }
            let existing = fs::read(path)
                .map_err(|_| remote_internal("remote_repository_staging_unavailable"))?;
            if existing != bytes {
                return Err(remote_conflict("remote_repository_staging_conflict"));
            }
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(remote_internal("remote_repository_staging_unavailable")),
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| remote_internal("remote_repository_staging_unavailable"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| remote_internal("remote_repository_staging_unavailable"))
}

fn read_optional_private_file(
    path: &Path,
    max_bytes: u64,
    expected_sha256: &str,
) -> Result<Option<Vec<u8>>, RpcError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(remote_internal("remote_repository_staging_unavailable")),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > max_bytes
    {
        return Err(remote_invalid("remote_repository_staging_invalid"));
    }
    let bytes =
        fs::read(path).map_err(|_| remote_internal("remote_repository_staging_unavailable"))?;
    if sha256_hex(&bytes) != expected_sha256 {
        return Err(remote_invalid("remote_repository_bundle_digest_mismatch"));
    }
    Ok(Some(bytes))
}

fn ensure_private_directory(path: &Path) -> Result<(), RpcError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(remote_conflict("remote_managed_directory_unsafe"));
        }
    } else {
        fs::create_dir_all(path)
            .map_err(|_| remote_internal("remote_managed_directory_unavailable"))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| remote_internal("remote_managed_directory_unavailable"))?;
    }
    Ok(())
}

fn seal_attempt_receipt(receipt: &mut OrchestrationAttemptReceipt) -> Result<(), RpcError> {
    receipt.receipt_sha256 = zero_sha256();
    receipt.receipt_sha256 = sha256_content(receipt)?;
    Ok(())
}

fn sealed_sha256<T: Serialize + Clone>(
    value: &T,
    clear: impl FnOnce(&mut T),
) -> Result<ContentSha256, RpcError> {
    let mut unsigned = value.clone();
    clear(&mut unsigned);
    sha256_content(&unsigned)
}

fn sha256_content<T: Serialize + ?Sized>(value: &T) -> Result<ContentSha256, RpcError> {
    let bytes =
        serde_json::to_vec(value).map_err(|_| remote_internal("remote_digest_encoding_failed"))?;
    ContentSha256::parse(sha256_hex(&bytes)).map_err(|_| remote_internal("remote_digest_invalid"))
}

fn zero_sha256() -> ContentSha256 {
    ContentSha256::parse("0".repeat(64)).expect("zero SHA-256 is valid")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn stable_digest(fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn attempt_state_rank(state: OrchestrationNodeAttemptState) -> u8 {
    match state {
        OrchestrationNodeAttemptState::Prepared => 0,
        OrchestrationNodeAttemptState::DispatchStarted => 1,
        OrchestrationNodeAttemptState::Accepted => 2,
        OrchestrationNodeAttemptState::NotStarted => 3,
        OrchestrationNodeAttemptState::Ambiguous => 4,
        OrchestrationNodeAttemptState::Terminal => 5,
        OrchestrationNodeAttemptState::Cancelled => 6,
    }
}

fn node_receipt_kind_name(kind: NodeReceiptKind) -> &'static str {
    match kind {
        NodeReceiptKind::RepositoryPrepare => "repository_prepare",
        NodeReceiptKind::AttemptPrepare => "attempt_prepare",
        NodeReceiptKind::AttemptDispatch => "attempt_dispatch",
        NodeReceiptKind::AttemptStatus => "attempt_status",
        NodeReceiptKind::AttemptCancel => "attempt_cancel",
        NodeReceiptKind::AttemptEvents => "attempt_events",
        NodeReceiptKind::AttemptAck => "attempt_ack",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn managed_git_rpc_error(error: crate::managed_git::ManagedGitError) -> RpcError {
    RpcError::with_data(
        -32042,
        error.safe_message,
        serde_json::json!({ "reason": error.reason, "retryable": error.retryable }),
    )
}

fn store_rpc_error(error: crate::store::StoreError) -> RpcError {
    RpcError::with_data(
        -32042,
        "Remote orchestration persistence failed",
        serde_json::json!({ "reason": "remote_execution_store_failed", "detail": error.to_string() }),
    )
}

fn admission_rpc_error(error: crate::execution_admission::ExecutionAdmissionError) -> RpcError {
    RpcError::with_data(
        -32042,
        "Remote execution admission failed",
        serde_json::json!({ "reason": "remote_execution_admission_failed", "detail": error.to_string() }),
    )
}

fn remote_invalid(reason: &'static str) -> RpcError {
    RpcError::with_data(
        -32602,
        "Remote orchestration request is invalid",
        serde_json::json!({ "reason": reason }),
    )
}

fn remote_conflict(reason: &'static str) -> RpcError {
    RpcError::with_data(
        -32043,
        "Remote orchestration identity conflicts with durable state",
        serde_json::json!({ "reason": reason }),
    )
}

fn remote_internal(reason: &'static str) -> RpcError {
    RpcError::with_data(
        -32603,
        "Remote orchestration execution failed",
        serde_json::json!({ "reason": reason }),
    )
}

fn remote_worker_error(_: tokio::task::JoinError) -> RpcError {
    remote_internal("remote_execution_worker_failed")
}

fn remote_injected(boundary: &'static str) -> RpcError {
    RpcError::with_data(
        -32603,
        "Injected remote orchestration crash boundary",
        serde_json::json!({ "reason": "remote_execution_failpoint", "boundary": boundary }),
    )
}

#[cfg(test)]
#[path = "remote_execution_tests.rs"]
mod tests;
