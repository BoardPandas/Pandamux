use super::*;
use crate::bundle_store::tests::materialization_fixture;
use crate::{
    AttemptDispatchState, EffectClaimRequest, EffectDispatchRequest, EffectOwnership,
    OrchestrationRunRecord, PlanExecutionBudgetRequest,
};
use crate::{
    CapabilityAuthority, DriverRegistry, EnqueueOrchestratedAdmissionRequest, EnvironmentRouter,
    ExecutionAdmissionService, ExecutionBudgetService, ProviderResolver, RecoveryDisposition,
    Router, RunExecutionService, RunRecoveryService,
    environment_router::{BoxFuture, EnvironmentTransport, EnvironmentTransportState},
    request_context::{CapabilityScope, ClientIdentity},
};
use pandamux_core::{
    AccessMode, AdvancedSettings, AgentId, AgentTask, ArchiveSha256, Attempt, AttemptBoundary,
    AttemptStatus, BroadsidePolicy, BudgetAmount, BundleSha, CleanupState, ConfigurationRevision,
    ContentSha256, Environment, EnvironmentKind, EnvironmentStatus, ExecutionLease,
    ExecutionLeaseId, LeaseResourceKind, OrchestrationEffectKind, OrchestrationMode,
    OrganizationId, OrganizationPolicy, OutputContract, ProjectId, ProjectRepositoryId,
    ProjectRepositoryRecord, ProviderInstanceConfig, ProviderInstanceId, ProviderKind,
    RoutingDecision, Run, RunAttemptId, RunCommand, RunEvent, RunEventId, RunId, RunSeed,
    RunTaskId, RunTaskStatus, RunTrigger, ServerIncarnationId, SourceCommitSha, SubscriptionKey,
    TaskRouteDecision, UserSettings, ValidatedPlan,
};
use pandamux_protocol::{EventEnvelope, RpcRequest, RpcResponse, ServerRole};
use pandamux_providers::MockProviderDriver;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

const ENVIRONMENT_ID: &str = "local";
const PROVIDER_ID: &str = "provider-test";
const AGENT_ID: &str = "builder";

struct Harness {
    _temp: tempfile::TempDir,
    paths: DataPaths,
    store: Store,
    compiled_bundle: pandamux_core::CompiledInstructionBundleV1,
    service: NodeExecutionService,
    drivers: Arc<DriverRegistry>,
    driver: Arc<MockProviderDriver>,
    context: RequestContext,
    baseline: SourceCommitSha,
    baseline_bundle: Vec<u8>,
    bundle_sha: BundleSha,
}

struct LoopbackNodeTransport {
    router: Arc<Router>,
    context: RequestContext,
    events: broadcast::Sender<EventEnvelope>,
    connected: AtomicBool,
    drop_next_response: AtomicBool,
}

impl LoopbackNodeTransport {
    fn new(router: Arc<Router>, context: RequestContext) -> Self {
        let (events, _) = broadcast::channel(16);
        Self {
            router,
            context,
            events,
            connected: AtomicBool::new(true),
            drop_next_response: AtomicBool::new(false),
        }
    }

    fn drop_next_response(&self) {
        self.drop_next_response.store(true, Ordering::Release);
    }
}

impl EnvironmentTransport for LoopbackNodeTransport {
    fn forward_request<'a>(
        &'a self,
        request: &'a RpcRequest,
    ) -> BoxFuture<'a, Result<RpcResponse, RpcError>> {
        Box::pin(async move {
            if !self.connected.load(Ordering::Acquire) {
                return Err(RpcError::internal_error("loopback node disconnected"));
            }
            let response = self
                .router
                .handle_request_with_context(&self.context, request.clone())
                .await
                .ok_or_else(|| RpcError::internal_error("loopback request returned no response"))?;
            if self.drop_next_response.swap(false, Ordering::AcqRel) {
                return Err(RpcError::with_data(
                    -32010,
                    "Loopback response was intentionally dropped",
                    json!({ "reason": "loopback_response_lost" }),
                ));
            }
            Ok(response)
        })
    }

    fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.events.subscribe()
    }

    fn state(&self) -> EnvironmentTransportState {
        if self.connected.load(Ordering::Acquire) {
            EnvironmentTransportState::Connected
        } else {
            EnvironmentTransportState::Closed
        }
    }

    fn shutdown<'a>(&'a self) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.connected.store(false, Ordering::Release);
        })
    }
}

impl Harness {
    fn new(provider_settings: serde_json::Value) -> Self {
        let temp = tempfile::tempdir().expect("temporary remote execution fixture");
        let root = fs::canonicalize(temp.path()).expect("canonical temporary fixture root");
        let paths = DataPaths::from_root(root.join("app-data")).expect("data paths");
        paths.ensure_private_roots().expect("private roots");
        let store = Store::open(paths.database()).expect("node store");
        let bundle_store = BundleStore::new(paths.clone(), store.clone());
        let input = materialization_fixture(&root.join("bundle-fixture"));
        let compiled_bundle = input.bundle.clone();
        let materialized = bundle_store
            .materialize(&input, now_ms())
            .expect("materialize pinned bundle");
        store
            .save_settings(&settings(provider_settings))
            .expect("node provider settings");

        let source = root.join("source-repository");
        fs::create_dir(&source).expect("source repository");
        git(&source, &["init", "-b", "main"]);
        git(&source, &["config", "user.name", "PandaMUX Test"]);
        git(
            &source,
            &["config", "user.email", "pandamux-test@localhost"],
        );
        fs::write(source.join("tracked.txt"), b"remote execution baseline\n")
            .expect("baseline file");
        git(&source, &["add", "--", "tracked.txt"]);
        git(&source, &["commit", "-m", "baseline"]);
        let baseline = SourceCommitSha::parse(git_text(&source, &["rev-parse", "HEAD"]))
            .expect("baseline identity");
        let bundle_path = source.join("baseline.bundle");
        git(
            &source,
            &[
                "bundle",
                "create",
                bundle_path.to_str().expect("bundle path"),
                "refs/heads/main",
            ],
        );
        let baseline_bundle = fs::read(bundle_path).expect("baseline bundle");

        let driver = Arc::new(MockProviderDriver::new(
            ProviderKind::Custom,
            "Mock Remote Provider",
        ));
        let drivers = Arc::new(DriverRegistry::new());
        drivers.register(ProviderKind::Custom, driver.clone());
        let resolver = Arc::new(ProviderResolver::new(
            store.clone(),
            drivers.clone(),
            EnvironmentId::new(ENVIRONMENT_ID),
            paths.root().to_path_buf(),
        ));
        let capability = Arc::new(CapabilityAuthority::new(EnvironmentId::new(ENVIRONMENT_ID)));
        let thread_manager = ThreadManager::new(
            store.clone(),
            resolver,
            ENVIRONMENT_ID.to_string(),
            capability,
        );
        let service = NodeExecutionService::new(
            store.clone(),
            paths.clone(),
            bundle_store.clone(),
            thread_manager,
            EnvironmentId::new(ENVIRONMENT_ID),
        );
        let context = RequestContext::node_proxy(
            EnvironmentId::new(ENVIRONMENT_ID),
            ClientIdentity {
                description: "authenticated-hub".to_string(),
                ..ClientIdentity::default()
            },
        );
        Self {
            _temp: temp,
            paths,
            store,
            compiled_bundle,
            service,
            drivers,
            driver,
            context,
            baseline,
            baseline_bundle,
            bundle_sha: materialized.object.bundle_sha,
        }
    }

    fn identity(&self, suffix: &str, idempotency_key: &str) -> OrchestrationExecutionIdentity {
        let task_id = RunTaskId::new(format!("task-remote-{suffix}"));
        OrchestrationExecutionIdentity {
            schema_version: ORCHESTRATION_EXECUTION_SCHEMA_VERSION,
            run_id: RunId::new(format!("run-remote-{suffix}")),
            task_id: task_id.clone(),
            attempt_id: RunAttemptId::new(format!("run-attempt-remote-{suffix}")),
            effect_id: pandamux_core::OrchestrationEffectId::new(format!("effect-remote-{suffix}")),
            payload_sha256: digest('a'),
            repository_id: ProjectRepositoryId::new(format!("repository-remote-{suffix}")),
            baseline_commit: self.baseline.clone(),
            bundle_sha: self.bundle_sha.clone(),
            route: TaskRouteDecision {
                task_id,
                selected_agent_id: Some(AgentId::new(AGENT_ID)),
                agent_evaluations: Vec::new(),
                provider: RoutingDecision {
                    selected_provider_instance_id: Some(ProviderInstanceId::new(PROVIDER_ID)),
                    selected_subscription_key: Some(SubscriptionKey::new(
                        "subscription-remote-test",
                    )),
                    selected_model: Some("test-model".to_string()),
                    evaluations: Vec::new(),
                },
            },
            lease_generation: 1,
            idempotency_key: idempotency_key.to_string(),
        }
    }

    fn repository_params(
        &self,
        identity: OrchestrationExecutionIdentity,
        inline: bool,
    ) -> OrchestrationRepositoryPrepareParams {
        let bytes_sha = sha256_hex(&self.baseline_bundle);
        OrchestrationRepositoryPrepareParams {
            identity,
            baseline_bundle_sha256: ArchiveSha256::parse(bytes_sha.clone())
                .expect("archive digest"),
            baseline_bundle_bytes_sha256: ContentSha256::parse(bytes_sha).expect("byte digest"),
            baseline_bundle_bytes_base64: inline.then(|| BASE64.encode(&self.baseline_bundle)),
        }
    }

    async fn prepare_repository(
        &self,
        identity: OrchestrationExecutionIdentity,
    ) -> OrchestrationRepositoryPrepareResult {
        self.service
            .repository_prepare(&self.context, self.repository_params(identity, true), None)
            .await
            .expect("prepare repository")
    }

    fn attempt_params(
        &self,
        identity: OrchestrationExecutionIdentity,
        prompt: &str,
    ) -> OrchestrationAttemptPrepareParams {
        let (worktree_id, thread_id, turn_id, turn_attempt_id) = remote_execution_ids(&identity);
        let output_contract = Some("{\"kind\":\"change_set\"}".to_string());
        let (instructions, tools, prompt_sha) = remote_preparation_hashes(
            &self.compiled_bundle,
            &AgentId::new(AGENT_ID),
            prompt,
            output_contract.as_deref(),
        )
        .expect("remote execution digests");
        OrchestrationAttemptPrepareParams {
            identity,
            worktree_id,
            thread_id,
            turn_id,
            turn_attempt_id,
            agent_id: AgentId::new(AGENT_ID),
            access_ceiling: AccessMode::ReadOnly,
            compiled_instructions_sha256: instructions,
            tool_policy_sha256: tools,
            prompt_sha256: prompt_sha,
            task_prompt: prompt.to_string(),
            output_contract,
            prepared_at_ms: now_ms(),
            attempt_ordinal: 1,
            run_concurrency_limit: 2,
        }
    }

    async fn prepare_attempt(
        &self,
        suffix: &str,
        prompt: &str,
    ) -> (
        OrchestrationExecutionIdentity,
        OrchestrationAttemptPrepareResult,
    ) {
        let repository_identity = self.identity(suffix, &format!("repo-{suffix}"));
        self.prepare_repository(repository_identity).await;
        let attempt_identity = self.identity(suffix, &format!("prepare-{suffix}"));
        let result = self
            .service
            .attempt_prepare(
                &self.context,
                self.attempt_params(attempt_identity.clone(), prompt),
                None,
            )
            .await
            .expect("prepare remote attempt");
        (attempt_identity, result)
    }

    fn router(&self) -> Router {
        Router::with_drivers(
            self.store.clone(),
            ServerRole::Node,
            ENVIRONMENT_ID.to_string(),
            self.drivers.clone(),
        )
        .with_data_paths(self.paths.clone())
    }
}

#[tokio::test]
async fn private_rpc_is_node_proxy_environment_bound_and_replay_strict() {
    let harness = Harness::new(json!({}));
    let params = harness.repository_params(harness.identity("auth", "repo-auth"), true);
    let router = harness.router();

    let control = RequestContext {
        peer_role: PeerRole::LocalControl,
        authenticated_environment_id: EnvironmentId::new(ENVIRONMENT_ID),
        client_identity: ClientIdentity::default(),
        capability_scope: CapabilityScope::Control,
        connection_id: "control-test".to_string(),
    };
    let denied = router
        .handle_request_with_context(
            &control,
            RpcRequest::new(
                1,
                "orchestration.repository.prepare",
                Some(serde_json::to_value(&params).expect("params")),
            ),
        )
        .await
        .expect("RPC response");
    assert_eq!(
        denied.error.and_then(|error| error.data),
        Some(json!({ "reason": "orchestration_node_proxy_required" }))
    );

    let wrong_environment = RequestContext::node_proxy(
        EnvironmentId::new("other-environment"),
        ClientIdentity::default(),
    );
    let denied = router
        .handle_request_with_context(
            &wrong_environment,
            RpcRequest::new(
                2,
                "orchestration.repository.prepare",
                Some(serde_json::to_value(&params).expect("params")),
            ),
        )
        .await
        .expect("RPC response");
    assert!(denied.error.is_some());

    let mut forged = serde_json::to_value(&params).expect("params");
    forged
        .as_object_mut()
        .expect("object")
        .insert("environmentId".to_string(), json!("forged-environment"));
    let denied = router
        .handle_request_with_context(
            &harness.context,
            RpcRequest::new(3, "orchestration.repository.prepare", Some(forged)),
        )
        .await
        .expect("RPC response");
    assert!(denied.error.is_some());

    let first = harness
        .service
        .repository_prepare(&harness.context, params.clone(), None)
        .await
        .expect("first prepare");
    let replay = harness
        .service
        .repository_prepare(&harness.context, params.clone(), None)
        .await
        .expect("identical replay");
    assert_eq!(first, replay);
    let mut conflicting = params;
    conflicting.identity.payload_sha256 = digest('b');
    let error = harness
        .service
        .repository_prepare(&harness.context, conflicting, None)
        .await
        .expect_err("changed bytes under one key must fail");
    assert_reason(&error, "remote_idempotency_payload_conflict");
    assert_eq!(harness.driver.session_start_count(), 0);
}

#[tokio::test]
async fn authenticated_loopback_disconnect_joins_relay_and_never_falls_back_local() {
    let harness = Harness::new(json!({}));
    let node_router = Arc::new(harness.router());
    let transport = Arc::new(LoopbackNodeTransport::new(
        node_router,
        harness.context.clone(),
    ));
    let hub_router = EnvironmentRouter::new(EnvironmentId::new("hub-local"));
    let node_environment = EnvironmentId::new(ENVIRONMENT_ID);
    let (hub_events, _) = broadcast::channel(16);
    hub_router
        .register_transport(
            node_environment.clone(),
            transport.clone(),
            Some(hub_events),
        )
        .await;
    let params = harness.repository_params(harness.identity("loopback", "repo-loopback"), true);
    let receipt = hub_router
        .orchestration_repository_prepare(&node_environment, &params)
        .await
        .expect("authenticated loopback prepare");
    assert!(receipt.baseline_present);
    assert_eq!(receipt.environment_id, node_environment);

    hub_router.unregister_transport(&node_environment).await;
    assert!(!transport.connected.load(Ordering::Acquire));
    assert!(!hub_router.is_connected(&node_environment).await);
    let error = hub_router
        .orchestration_repository_prepare(&node_environment, &params)
        .await
        .expect_err("disconnected node never falls back to the Hub");
    assert_eq!(error.code, -32010);
    hub_router.shutdown_all().await;
}

#[tokio::test]
async fn two_store_prepare_and_dispatch_crashes_reconcile_without_duplicate_start() {
    let node = Harness::new(json!({}));
    let node_environment = EnvironmentId::new(ENVIRONMENT_ID);
    let transport = Arc::new(LoopbackNodeTransport::new(
        Arc::new(node.router()),
        node.context.clone(),
    ));
    let environment_router = Arc::new(EnvironmentRouter::new(EnvironmentId::new("hub-local")));
    environment_router
        .register_transport(node_environment.clone(), transport, None)
        .await;
    let (hub_store, hub_service, ownership, base_identity, dispatch_at_ms) =
        hub_execution(&node, environment_router.clone(), "two-store").await;

    let mut wrong_generation = base_identity.clone();
    wrong_generation.idempotency_key = "hub-wrong-lease-generation".to_string();
    wrong_generation.lease_generation += 1;
    let error = hub_service
        .prepare_remote_repository(
            &ownership,
            &node_environment,
            node.repository_params(wrong_generation, true),
            dispatch_at_ms,
        )
        .await
        .expect_err("Hub rejects a request outside the claimed lease generation");
    assert_reason(&error, "remote_effect_ownership_mismatch");

    let repository_identity = OrchestrationExecutionIdentity {
        idempotency_key: "hub-repository-prepare".to_string(),
        ..base_identity.clone()
    };
    let repository = hub_service
        .prepare_remote_repository(
            &ownership,
            &node_environment,
            node.repository_params(repository_identity, true),
            dispatch_at_ms,
        )
        .await
        .expect("Hub persists repository receipt");
    assert!(repository.baseline_present);

    let prepare_identity = OrchestrationExecutionIdentity {
        idempotency_key: "hub-attempt-prepare".to_string(),
        ..base_identity.clone()
    };
    let prepare_params =
        node.attempt_params(prepare_identity.clone(), "Two-store dispatch fixture");
    let node_prepare = environment_router
        .orchestration_attempt_prepare(&node_environment, &prepare_params)
        .await
        .expect("node commits prepare before simulated Hub crash");
    assert_eq!(node.driver.session_start_count(), 0);
    assert!(
        hub_store
            .list_node_receipts_for_attempt(&base_identity.attempt_id)
            .expect("Hub receipts")
            .iter()
            .all(|receipt| receipt.request_kind != NodeReceiptKind::AttemptPrepare)
    );

    let hub_prepare = hub_service
        .prepare_remote_provider_attempt(&ownership, &node_environment, prepare_params)
        .await
        .expect("Hub retry persists replayed node prepare");
    assert_eq!(hub_prepare, node_prepare);
    assert_eq!(
        hub_store
            .list_node_receipts_for_attempt(&base_identity.attempt_id)
            .expect("Hub receipts")
            .iter()
            .filter(|receipt| receipt.request_kind == NodeReceiptKind::AttemptPrepare)
            .count(),
        1
    );
    let effect = hub_store
        .get_orchestration_effect(&ownership.effect_id)
        .expect("load Hub effect")
        .expect("Hub effect");
    assert_eq!(
        effect.delivery.state,
        pandamux_core::EffectDeliveryState::Prepared
    );

    let mut dispatch_identity = base_identity;
    let dispatch_key =
        remote_dispatch_identity(&dispatch_identity, &hub_prepare.receipt.receipt_sha256)
            .expect("stable dispatch identity");
    dispatch_identity.idempotency_key = dispatch_key.clone();
    let dispatch_params = OrchestrationAttemptDispatchParams {
        identity: dispatch_identity,
        prepare_receipt_sha256: hub_prepare.receipt.receipt_sha256,
        dispatch_identity: dispatch_key,
    };
    let marker = hub_service
        .mark_effect_dispatch_started(
            &EffectDispatchRequest {
                ownership: ownership.clone(),
                dispatch_started_at_ms: dispatch_at_ms,
            },
            None,
        )
        .expect("Hub marker commits before simulated crash");
    assert!(marker.dispatch_allowed);
    assert_eq!(node.driver.session_start_count(), 0);

    let node_marker = hub_service
        .dispatch_remote_provider_attempt(
            &ownership,
            &node_environment,
            dispatch_params.clone(),
            dispatch_at_ms,
            None,
        )
        .await
        .expect("prepared node receives one stable first dispatch");
    assert_eq!(
        node_marker.state,
        OrchestrationNodeAttemptState::DispatchStarted
    );
    assert_eq!(node.driver.session_start_count(), 1);
    let replay = hub_service
        .dispatch_remote_provider_attempt(
            &ownership,
            &node_environment,
            dispatch_params,
            dispatch_at_ms,
            None,
        )
        .await
        .expect("Hub dispatch receipt replay");
    assert_eq!(replay, node_marker);
    assert_eq!(node.driver.session_start_count(), 1);
    environment_router.shutdown_all().await;
}

#[tokio::test]
async fn hub_response_loss_uses_status_and_never_forwards_a_replacement_dispatch() {
    let node = Harness::new(json!({}));
    let node_environment = EnvironmentId::new(ENVIRONMENT_ID);
    let transport = Arc::new(LoopbackNodeTransport::new(
        Arc::new(node.router()),
        node.context.clone(),
    ));
    let environment_router = Arc::new(EnvironmentRouter::new(EnvironmentId::new("hub-local")));
    environment_router
        .register_transport(node_environment.clone(), transport.clone(), None)
        .await;
    let (_hub_store, hub_service, ownership, base_identity, dispatch_at_ms) =
        hub_execution(&node, environment_router.clone(), "response-loss").await;

    let repository_identity = OrchestrationExecutionIdentity {
        idempotency_key: "response-loss-repository".to_string(),
        ..base_identity.clone()
    };
    hub_service
        .prepare_remote_repository(
            &ownership,
            &node_environment,
            node.repository_params(repository_identity, true),
            dispatch_at_ms,
        )
        .await
        .expect("prepare remote repository");
    let prepare_identity = OrchestrationExecutionIdentity {
        idempotency_key: "response-loss-prepare".to_string(),
        ..base_identity.clone()
    };
    let prepared = hub_service
        .prepare_remote_provider_attempt(
            &ownership,
            &node_environment,
            node.attempt_params(prepare_identity, "request_approval: response loss"),
        )
        .await
        .expect("prepare remote attempt");
    let mut dispatch_identity = base_identity.clone();
    let dispatch_key =
        remote_dispatch_identity(&dispatch_identity, &prepared.receipt.receipt_sha256)
            .expect("stable dispatch identity");
    dispatch_identity.idempotency_key = dispatch_key.clone();
    let dispatch_params = OrchestrationAttemptDispatchParams {
        identity: dispatch_identity,
        prepare_receipt_sha256: prepared.receipt.receipt_sha256,
        dispatch_identity: dispatch_key,
    };

    transport.drop_next_response();
    let error = hub_service
        .dispatch_remote_provider_attempt(
            &ownership,
            &node_environment,
            dispatch_params.clone(),
            dispatch_at_ms,
            None,
        )
        .await
        .expect_err("transport loses the processed dispatch response");
    assert_reason(&error, "loopback_response_lost");
    assert_eq!(node.driver.session_start_count(), 1);

    let reconciled = hub_service
        .dispatch_remote_provider_attempt(
            &ownership,
            &node_environment,
            dispatch_params,
            dispatch_at_ms,
            None,
        )
        .await
        .expect("Hub reconciles exact node status");
    assert!(matches!(
        reconciled.state,
        OrchestrationNodeAttemptState::Accepted | OrchestrationNodeAttemptState::Terminal
    ));
    assert_eq!(node.driver.session_start_count(), 1);

    let mut cancel_identity = base_identity;
    cancel_identity.idempotency_key = "response-loss-cancel".to_string();
    hub_service
        .cancel_remote_provider_attempt(
            &node_environment,
            OrchestrationAttemptCancelParams {
                identity: cancel_identity,
                reason_code: "test_cleanup".to_string(),
            },
            now_ms(),
        )
        .await
        .expect("clean up the loopback session");
    environment_router.shutdown_all().await;
}

#[tokio::test]
async fn rec_10_and_11_startup_recovery_reattaches_exact_response_loss() {
    let node = Harness::new(json!({}));
    let node_environment = EnvironmentId::new(ENVIRONMENT_ID);
    let transport = Arc::new(LoopbackNodeTransport::new(
        Arc::new(node.router()),
        node.context.clone(),
    ));
    let environment_router = Arc::new(EnvironmentRouter::new(EnvironmentId::new("hub-local")));
    environment_router
        .register_transport(node_environment.clone(), transport.clone(), None)
        .await;
    let (hub_store, hub_service, ownership, base_identity, dispatch_at_ms) =
        hub_execution(&node, environment_router.clone(), "recovery-response-loss").await;

    let repository_identity = OrchestrationExecutionIdentity {
        idempotency_key: "recovery-response-loss-repository".to_string(),
        ..base_identity.clone()
    };
    hub_service
        .prepare_remote_repository(
            &ownership,
            &node_environment,
            node.repository_params(repository_identity, true),
            dispatch_at_ms,
        )
        .await
        .expect("prepare recovery repository");
    let prepare_identity = OrchestrationExecutionIdentity {
        idempotency_key: "recovery-response-loss-prepare".to_string(),
        ..base_identity.clone()
    };
    let prepared = hub_service
        .prepare_remote_provider_attempt(
            &ownership,
            &node_environment,
            node.attempt_params(
                prepare_identity,
                "request_approval: hold remote recovery attempt open",
            ),
        )
        .await
        .expect("prepare recovery attempt");
    let mut dispatch_identity = base_identity.clone();
    let dispatch_key =
        remote_dispatch_identity(&dispatch_identity, &prepared.receipt.receipt_sha256)
            .expect("stable recovery dispatch identity");
    dispatch_identity.idempotency_key = dispatch_key.clone();
    let dispatch_params = OrchestrationAttemptDispatchParams {
        identity: dispatch_identity,
        prepare_receipt_sha256: prepared.receipt.receipt_sha256,
        dispatch_identity: dispatch_key,
    };

    transport.drop_next_response();
    let error = hub_service
        .dispatch_remote_provider_attempt(
            &ownership,
            &node_environment,
            dispatch_params,
            dispatch_at_ms,
            None,
        )
        .await
        .expect_err("processed dispatch response is lost before Hub acceptance");
    assert_reason(&error, "loopback_response_lost");
    assert_eq!(node.driver.session_start_count(), 1);

    let recovery_owner = ServerIncarnationId::new("hub-recovery-owner");
    let report = RunRecoveryService::new(hub_store.clone())
        .with_environment_router(environment_router.clone())
        .recover_nonterminal_runs(recovery_owner.clone(), dispatch_at_ms + 1)
        .await
        .expect("recover exact remote attempt");
    assert!(report.intake_open);
    assert_eq!(report.attempts_reattached, 1);
    assert_eq!(report.reviews_created, 0);
    assert!(report.actions.iter().any(|action| {
        action.attempt_id.as_ref() == Some(&base_identity.attempt_id)
            && action.disposition == RecoveryDisposition::ExactAttemptReattached
    }));
    assert_eq!(node.driver.session_start_count(), 1);

    let effect = hub_store
        .get_orchestration_effect(&base_identity.effect_id)
        .expect("load recovered effect")
        .expect("recovered effect exists");
    assert_eq!(
        effect.delivery.state,
        pandamux_core::EffectDeliveryState::InFlight
    );
    assert_eq!(
        effect.delivery.owner_incarnation_id.as_ref(),
        Some(&recovery_owner)
    );
    let dispatch = hub_store
        .get_attempt_dispatch(&base_identity.attempt_id)
        .expect("load recovered dispatch")
        .expect("recovered dispatch exists");
    assert_eq!(dispatch.state, AttemptDispatchState::Accepted);
    let run = hub_store
        .load_verified_orchestration_run(&base_identity.run_id)
        .expect("load recovered run")
        .expect("recovered run exists");
    assert_eq!(
        run.run
            .attempts
            .get(&base_identity.attempt_id)
            .expect("recovered canonical attempt")
            .status,
        AttemptStatus::Running
    );
    let queue = hub_store
        .load_orchestration_runtime_records(&base_identity.run_id)
        .expect("load recovered runtime")
        .queue_records
        .into_iter()
        .find(|record| record.attempt_id == base_identity.attempt_id)
        .expect("recovered capacity owner");
    assert_eq!(queue.state, QueueState::Dispatched);
    assert_eq!(queue.owner_incarnation_id.as_ref(), Some(&recovery_owner));
    assert!(queue.lease_id.is_some());

    let mut cancel_identity = base_identity;
    cancel_identity.idempotency_key = "recovery-response-loss-cancel".to_string();
    hub_service
        .cancel_remote_provider_attempt(
            &node_environment,
            OrchestrationAttemptCancelParams {
                identity: cancel_identity,
                reason_code: "test_cleanup".to_string(),
            },
            now_ms(),
        )
        .await
        .expect("clean up recovered loopback session");
    environment_router.shutdown_all().await;
}

#[tokio::test]
async fn crash_pre_marker_recovery_pauses_once_without_starting_provider() {
    let node = Harness::new(json!({}));
    let node_environment = EnvironmentId::new(ENVIRONMENT_ID);
    let transport = Arc::new(LoopbackNodeTransport::new(
        Arc::new(node.router()),
        node.context.clone(),
    ));
    let environment_router = Arc::new(EnvironmentRouter::new(EnvironmentId::new("hub-local")));
    environment_router
        .register_transport(node_environment.clone(), transport, None)
        .await;
    let (hub_store, hub_service, ownership, base_identity, dispatch_at_ms) =
        hub_execution(&node, environment_router.clone(), "recovery-pre-marker").await;

    let repository_identity = OrchestrationExecutionIdentity {
        idempotency_key: "recovery-pre-marker-repository".to_string(),
        ..base_identity.clone()
    };
    hub_service
        .prepare_remote_repository(
            &ownership,
            &node_environment,
            node.repository_params(repository_identity, true),
            dispatch_at_ms,
        )
        .await
        .expect("prepare pre-marker recovery repository");
    let prepare_identity = OrchestrationExecutionIdentity {
        idempotency_key: "recovery-pre-marker-prepare".to_string(),
        ..base_identity.clone()
    };
    hub_service
        .prepare_remote_provider_attempt(
            &ownership,
            &node_environment,
            node.attempt_params(prepare_identity, "prepared but never dispatched"),
        )
        .await
        .expect("persist remote preparation");
    assert_eq!(node.driver.session_start_count(), 0);

    let recovery = RunRecoveryService::new(hub_store.clone())
        .with_environment_router(environment_router.clone());
    let first = recovery
        .recover_nonterminal_runs(
            ServerIncarnationId::new("hub-pre-marker-recovery"),
            dispatch_at_ms - 1,
        )
        .await
        .expect("classify durable prepared attempt");
    assert!(first.intake_open);
    assert_eq!(first.reviews_created, 1);
    assert!(first.actions.iter().any(|action| {
        action.attempt_id.as_ref() == Some(&base_identity.attempt_id)
            && action.disposition == RecoveryDisposition::ReviewRequired
            && action.reason_code.as_deref() == Some("recovery_prepared_attempt_requires_resume")
    }));
    assert_eq!(node.driver.session_start_count(), 0);
    let run = hub_store
        .load_verified_orchestration_run(&base_identity.run_id)
        .expect("load paused pre-marker run")
        .expect("paused pre-marker run exists");
    assert_eq!(run.run.status, pandamux_core::RunStatus::Paused);

    let replay = recovery
        .recover_nonterminal_runs(
            ServerIncarnationId::new("hub-pre-marker-recovery-replay"),
            dispatch_at_ms - 1,
        )
        .await
        .expect("replay prepared recovery classification");
    assert_eq!(replay.reviews_created, 0);
    assert_eq!(replay.reviews_deduplicated, 1);
    assert_eq!(hub_store.list_pending_recovery_reviews().unwrap().len(), 1);
    assert_eq!(node.driver.session_start_count(), 0);
    environment_router.shutdown_all().await;
}

#[tokio::test]
async fn repository_prepare_supports_staged_content_and_rejects_missing_baseline() {
    let harness = Harness::new(json!({}));
    let missing = harness
        .service
        .repository_prepare(
            &harness.context,
            harness.repository_params(harness.identity("staged", "repo-staged-missing"), false),
            None,
        )
        .await
        .expect("staging receipt");
    assert!(!missing.baseline_present);
    let staged = PathBuf::from(
        missing
            .private_artifact_ref
            .as_deref()
            .expect("private staged path"),
    );
    assert!(staged.starts_with(harness.paths.root()));
    fs::write(&staged, &harness.baseline_bundle).expect("stage baseline bundle");

    let uploaded = harness
        .service
        .repository_prepare(
            &harness.context,
            harness.repository_params(harness.identity("staged", "repo-staged-uploaded"), false),
            None,
        )
        .await
        .expect("consume staged bundle");
    assert!(uploaded.baseline_present);
    assert_eq!(
        uploaded.repository_key_sha256,
        missing.repository_key_sha256
    );
    let inline = harness
        .service
        .repository_prepare(
            &harness.context,
            harness.repository_params(harness.identity("staged", "repo-staged-inline"), true),
            None,
        )
        .await
        .expect("content-addressed replay from inline bytes");
    assert_eq!(uploaded.repository_key_sha256, inline.repository_key_sha256);

    let mut digest_mismatch = harness.repository_params(
        harness.identity("archive-digest", "repo-archive-digest"),
        true,
    );
    digest_mismatch.baseline_bundle_sha256 =
        ArchiveSha256::parse("e".repeat(64)).expect("different archive digest");
    let error = harness
        .service
        .repository_prepare(&harness.context, digest_mismatch, None)
        .await
        .expect_err("declared archive digest must match the transferred bytes");
    assert_reason(&error, "remote_repository_bundle_digest_mismatch");

    let mut wrong = harness.identity("wrong-baseline", "repo-wrong-baseline");
    wrong.baseline_commit = SourceCommitSha::parse("f".repeat(40)).expect("missing commit");
    let error = harness
        .service
        .repository_prepare(
            &harness.context,
            harness.repository_params(wrong, true),
            None,
        )
        .await
        .expect_err("bundle cannot materialize an absent baseline");
    assert!(matches!(error.code, -32602 | -32042));
    assert_eq!(harness.driver.session_start_count(), 0);
}

#[tokio::test]
async fn attempt_prepare_is_deterministic_crash_replayable_and_never_starts_provider() {
    let harness = Harness::new(json!({}));
    let repository_identity = harness.identity("prepare", "repo-prepare");
    harness.prepare_repository(repository_identity).await;
    let identity = harness.identity("prepare", "attempt-prepare");
    let params = harness.attempt_params(identity.clone(), "Implement the remote fixture");
    let error = harness
        .service
        .attempt_prepare(
            &harness.context,
            params.clone(),
            Some(NodeExecutionFailpoint::AfterAttemptWorktreeMaterialized),
        )
        .await
        .expect_err("injected worktree boundary");
    assert_reason(&error, "remote_execution_failpoint");
    assert_eq!(harness.driver.session_start_count(), 0);

    let prepared = harness
        .service
        .attempt_prepare(&harness.context, params.clone(), None)
        .await
        .expect("retry adopts exact worktree");
    assert_eq!(
        prepared.receipt.state,
        OrchestrationNodeAttemptState::Prepared
    );
    assert_eq!(prepared.receipt.thread_id, params.thread_id);
    assert_eq!(prepared.prepared_execution.worktree_id, params.worktree_id);
    let replay = harness
        .service
        .attempt_prepare(&harness.context, params.clone(), None)
        .await
        .expect("identical attempt replay");
    assert_eq!(prepared, replay);
    assert_eq!(harness.driver.session_start_count(), 0);

    let worktree = harness
        .paths
        .worktrees()
        .join("orchestration")
        .join(params.worktree_id.as_str());
    assert_eq!(
        git_text(&worktree, &["rev-parse", "HEAD"]),
        harness.baseline.as_str()
    );
    let thread = harness
        .store
        .get_thread(&params.thread_id)
        .expect("thread lookup")
        .expect("prepared thread");
    let branch_ref = thread
        .workspace
        .worktree
        .expect("owned worktree metadata")
        .branch;
    assert!(branch_ref.starts_with("refs/heads/pandamux/orchestration/"));
    assert_eq!(
        git_text(&worktree, &["rev-parse", "--symbolic-full-name", "HEAD"]),
        branch_ref
    );
    let mut conflict = params.clone();
    conflict.task_prompt.push_str(" changed");
    let error = harness
        .service
        .attempt_prepare(&harness.context, conflict, None)
        .await
        .expect_err("same prepare key cannot change payload");
    assert_reason(&error, "remote_idempotency_payload_conflict");

    let mut missing_bundle_identity = harness.identity("missing-bundle", "repo-missing-bundle");
    missing_bundle_identity.bundle_sha = BundleSha::parse("d".repeat(64)).expect("bundle digest");
    harness
        .service
        .repository_prepare(
            &harness.context,
            harness.repository_params(missing_bundle_identity.clone(), true),
            None,
        )
        .await
        .expect("baseline is independent of instruction bundle presence");
    missing_bundle_identity.idempotency_key = "attempt-missing-bundle".to_string();
    let error = harness
        .service
        .attempt_prepare(
            &harness.context,
            harness.attempt_params(missing_bundle_identity, "Missing bundle"),
            None,
        )
        .await
        .expect_err("missing pinned bundle must fail");
    assert_reason(&error, "remote_pinned_bundle_missing");

    let absent_baseline = harness.identity("absent-baseline", "attempt-absent-baseline");
    let error = harness
        .service
        .attempt_prepare(
            &harness.context,
            harness.attempt_params(absent_baseline, "Missing baseline"),
            None,
        )
        .await
        .expect_err("unprepared baseline must fail");
    assert_reason(&error, "remote_baseline_not_prepared");
}

#[tokio::test]
async fn prepare_receipt_crash_repairs_admission_without_provider_start() {
    let harness = Harness::new(json!({}));
    harness
        .prepare_repository(harness.identity("receipt-crash", "repo-receipt-crash"))
        .await;
    let identity = harness.identity("receipt-crash", "attempt-receipt-crash");
    let params = harness.attempt_params(identity, "Receipt crash fixture");
    let error = harness
        .service
        .attempt_prepare(
            &harness.context,
            params.clone(),
            Some(NodeExecutionFailpoint::AfterAttemptReceipt),
        )
        .await
        .expect_err("crash after prepare receipt");
    assert_reason(&error, "remote_execution_failpoint");
    assert!(
        harness
            .store
            .list_execution_queue_records()
            .expect("queue rows")
            .is_empty()
    );
    let replay = harness
        .service
        .attempt_prepare(&harness.context, params, None)
        .await
        .expect("replay repairs admission");
    assert_eq!(
        replay.receipt.state,
        OrchestrationNodeAttemptState::Prepared
    );
    assert_eq!(
        harness
            .store
            .list_execution_queue_records()
            .expect("queue rows")
            .len(),
        1
    );
    assert_eq!(harness.driver.session_start_count(), 0);
}

#[tokio::test]
async fn dispatch_marker_replay_status_cancel_events_and_ack_are_exact() {
    let harness = Harness::new(json!({}));
    let (prepare_identity, prepared) = harness
        .prepare_attempt("dispatch", "request_approval: echo remote")
        .await;
    let mut dispatch_identity = prepare_identity.clone();
    let dispatch_key =
        remote_dispatch_identity(&dispatch_identity, &prepared.receipt.receipt_sha256)
            .expect("dispatch identity");
    dispatch_identity.idempotency_key = dispatch_key.clone();
    let dispatch = OrchestrationAttemptDispatchParams {
        identity: dispatch_identity.clone(),
        prepare_receipt_sha256: prepared.receipt.receipt_sha256.clone(),
        dispatch_identity: dispatch_key,
    };
    let error = harness
        .service
        .attempt_dispatch(
            &harness.context,
            dispatch.clone(),
            Some(NodeExecutionFailpoint::BeforeDispatchMarker),
        )
        .await
        .expect_err("pre-marker crash");
    assert_reason(&error, "remote_execution_failpoint");
    assert_eq!(harness.driver.session_start_count(), 0);

    let marker = harness
        .service
        .attempt_dispatch(&harness.context, dispatch.clone(), None)
        .await
        .expect("dispatch once");
    assert_eq!(marker.state, OrchestrationNodeAttemptState::DispatchStarted);
    assert_eq!(harness.driver.session_start_count(), 1);
    let replay = harness
        .service
        .attempt_dispatch(&harness.context, dispatch, None)
        .await
        .expect("dispatch replay");
    assert_eq!(marker, replay);
    assert_eq!(harness.driver.session_start_count(), 1);

    tokio::task::yield_now().await;
    let mut status_identity = prepare_identity.clone();
    status_identity.idempotency_key = "status-after-response-loss".to_string();
    let status = harness
        .service
        .attempt_status(
            &harness.context,
            OrchestrationAttemptStatusParams {
                identity: status_identity,
            },
        )
        .await
        .expect("reattach exact status");
    assert!(matches!(
        status.receipt.state,
        OrchestrationNodeAttemptState::Accepted | OrchestrationNodeAttemptState::DispatchStarted
    ));
    assert_eq!(status.receipt.thread_id, prepared.receipt.thread_id);

    let mut cancel_identity = prepare_identity.clone();
    cancel_identity.idempotency_key = "cancel-remote".to_string();
    let cancel_params = OrchestrationAttemptCancelParams {
        identity: cancel_identity,
        reason_code: "test_cancel".to_string(),
    };
    let cancelled = harness
        .service
        .attempt_cancel(&harness.context, cancel_params.clone())
        .await
        .expect("cancel remote attempt");
    let cancel_replay = harness
        .service
        .attempt_cancel(&harness.context, cancel_params)
        .await
        .expect("cancel replay");
    assert_eq!(cancelled, cancel_replay);
    assert_eq!(
        cancelled.receipt.state,
        OrchestrationNodeAttemptState::Cancelled
    );

    tokio::task::yield_now().await;
    let mut event_identity = prepare_identity.clone();
    event_identity.idempotency_key = "events-remote".to_string();
    let events = harness
        .service
        .attempt_events(
            &harness.context,
            OrchestrationAttemptEventsParams {
                identity: event_identity,
                after_sequence: 0,
                limit: 256,
            },
        )
        .await
        .expect("ordered remote events");
    assert!(!events.events.is_empty());
    assert!(
        events
            .events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    let through_sequence = events.events.last().expect("last event").sequence;
    let mut ack_identity = prepare_identity;
    ack_identity.idempotency_key = "ack-remote".to_string();
    let ack_params = OrchestrationAttemptAckParams {
        identity: ack_identity,
        through_sequence,
        events_sha256: events.events_sha256,
    };
    let ack = harness
        .service
        .attempt_ack(&harness.context, ack_params.clone())
        .await
        .expect("ack event page");
    let replay = harness
        .service
        .attempt_ack(&harness.context, ack_params)
        .await
        .expect("ack replay");
    assert_eq!(ack, replay);
    assert_eq!(ack.acknowledged_through_sequence, through_sequence);
}

#[tokio::test]
async fn verified_not_started_remote_dispatch_releases_node_capacity_once() {
    let harness = Harness::new(json!({
        "startFailure": "verified_not_started"
    }));
    let (prepare_identity, prepared) = harness
        .prepare_attempt("not-started", "Provider must not accept this prompt")
        .await;
    let mut dispatch_identity = prepare_identity.clone();
    let dispatch_key =
        remote_dispatch_identity(&dispatch_identity, &prepared.receipt.receipt_sha256)
            .expect("dispatch identity");
    dispatch_identity.idempotency_key = dispatch_key.clone();
    let dispatch = OrchestrationAttemptDispatchParams {
        identity: dispatch_identity,
        prepare_receipt_sha256: prepared.receipt.receipt_sha256,
        dispatch_identity: dispatch_key,
    };

    let marker = harness
        .service
        .attempt_dispatch(&harness.context, dispatch.clone(), None)
        .await
        .expect("persist typed not-started result");
    assert_eq!(marker.state, OrchestrationNodeAttemptState::DispatchStarted);
    let replay = harness
        .service
        .attempt_dispatch(&harness.context, dispatch, None)
        .await
        .expect("replay dispatch marker");
    assert_eq!(replay, marker);

    let mut status_identity = prepare_identity.clone();
    status_identity.idempotency_key = "not-started-status".to_string();
    let status = harness
        .service
        .attempt_status(
            &harness.context,
            OrchestrationAttemptStatusParams {
                identity: status_identity,
            },
        )
        .await
        .expect("load typed not-started status");
    assert_eq!(
        status.receipt.state,
        OrchestrationNodeAttemptState::NotStarted
    );
    assert_eq!(
        status.receipt.safe_error_code.as_deref(),
        Some("mock_start_not_started")
    );

    let queue = harness
        .store
        .list_execution_queue_records()
        .expect("node queue rows")
        .into_iter()
        .find(|record| record.attempt_id == prepare_identity.attempt_id)
        .expect("node capacity row");
    assert_eq!(queue.state, QueueState::Dispatched);
    assert!(queue.owner_incarnation_id.is_none());
    assert!(queue.lease_id.is_none());
}

#[tokio::test]
async fn late_cancel_preserves_an_existing_terminal_receipt() {
    let harness = Harness::new(json!({}));
    let (prepare_identity, prepared) = harness
        .prepare_attempt("late-cancel", "Complete the remote fixture")
        .await;
    let mut dispatch_identity = prepare_identity.clone();
    let dispatch_key =
        remote_dispatch_identity(&dispatch_identity, &prepared.receipt.receipt_sha256)
            .expect("dispatch identity");
    dispatch_identity.idempotency_key = dispatch_key.clone();
    harness
        .service
        .attempt_dispatch(
            &harness.context,
            OrchestrationAttemptDispatchParams {
                identity: dispatch_identity,
                prepare_receipt_sha256: prepared.receipt.receipt_sha256,
                dispatch_identity: dispatch_key,
            },
            None,
        )
        .await
        .expect("dispatch terminal fixture");

    let terminal = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let receipt = harness
                .service
                .current_attempt_receipt(&prepare_identity, digest('e'))
                .expect("current attempt receipt");
            if receipt.state == OrchestrationNodeAttemptState::Terminal {
                break receipt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("mock provider reaches a terminal receipt");
    assert_eq!(terminal.terminal_status, Some(AttemptStatus::Succeeded));

    let mut cancel_identity = prepare_identity;
    cancel_identity.idempotency_key = "late-cancel-terminal".to_string();
    let cancel_params = OrchestrationAttemptCancelParams {
        identity: cancel_identity,
        reason_code: "late_cancel".to_string(),
    };
    let result = harness
        .service
        .attempt_cancel(&harness.context, cancel_params.clone())
        .await
        .expect("late cancel returns the terminal authority");
    let replay = harness
        .service
        .attempt_cancel(&harness.context, cancel_params)
        .await
        .expect("late cancel receipt replays");
    assert_eq!(result, replay);
    assert_eq!(
        result.receipt.state,
        OrchestrationNodeAttemptState::Terminal
    );
    assert_eq!(
        result.receipt.terminal_status,
        Some(AttemptStatus::Succeeded)
    );
    assert_eq!(harness.driver.session_start_count(), 1);
}

#[tokio::test]
async fn post_marker_crash_never_starts_a_replacement_provider() {
    let harness = Harness::new(json!({}));
    let (prepare_identity, prepared) = harness
        .prepare_attempt("marker-crash", "Marker crash fixture")
        .await;
    let mut dispatch_identity = prepare_identity.clone();
    let dispatch_key =
        remote_dispatch_identity(&dispatch_identity, &prepared.receipt.receipt_sha256)
            .expect("dispatch identity");
    dispatch_identity.idempotency_key = dispatch_key.clone();
    let params = OrchestrationAttemptDispatchParams {
        identity: dispatch_identity,
        prepare_receipt_sha256: prepared.receipt.receipt_sha256,
        dispatch_identity: dispatch_key,
    };
    let error = harness
        .service
        .attempt_dispatch(
            &harness.context,
            params.clone(),
            Some(NodeExecutionFailpoint::AfterDispatchMarker),
        )
        .await
        .expect_err("post-marker crash");
    assert_reason(&error, "remote_execution_failpoint");
    assert_eq!(harness.driver.session_start_count(), 0);
    let marker = harness
        .service
        .attempt_dispatch(&harness.context, params, None)
        .await
        .expect("exact marker replay");
    assert_eq!(marker.state, OrchestrationNodeAttemptState::DispatchStarted);
    assert_eq!(harness.driver.session_start_count(), 0);

    let mut status_identity = prepare_identity.clone();
    status_identity.idempotency_key = "status-marker-crash".to_string();
    let status = harness
        .service
        .attempt_status(
            &harness.context,
            OrchestrationAttemptStatusParams {
                identity: status_identity,
            },
        )
        .await
        .expect("marker status");
    assert_eq!(
        status.receipt.state,
        OrchestrationNodeAttemptState::DispatchStarted
    );

    let mut cancel_identity = prepare_identity;
    cancel_identity.idempotency_key = "cancel-marker-crash".to_string();
    let cancel = harness
        .service
        .attempt_cancel(
            &harness.context,
            OrchestrationAttemptCancelParams {
                identity: cancel_identity,
                reason_code: "operator_cancel".to_string(),
            },
        )
        .await
        .expect("unverified marked dispatch returns a durable receipt");
    assert_eq!(
        cancel.receipt.state,
        OrchestrationNodeAttemptState::Ambiguous
    );
    assert_eq!(
        cancel.receipt.safe_error_code.as_deref(),
        Some("remote_cancel_session_unverified")
    );
}

#[tokio::test]
async fn post_provider_crash_reattaches_the_exact_live_turn_without_replacement() {
    let harness = Harness::new(json!({}));
    let (prepare_identity, prepared) = harness
        .prepare_attempt(
            "provider-crash",
            "request_approval: keep the exact remote turn active",
        )
        .await;
    let mut dispatch_identity = prepare_identity.clone();
    let dispatch_key =
        remote_dispatch_identity(&dispatch_identity, &prepared.receipt.receipt_sha256)
            .expect("dispatch identity");
    dispatch_identity.idempotency_key = dispatch_key.clone();
    let params = OrchestrationAttemptDispatchParams {
        identity: dispatch_identity,
        prepare_receipt_sha256: prepared.receipt.receipt_sha256,
        dispatch_identity: dispatch_key,
    };
    let error = harness
        .service
        .attempt_dispatch(
            &harness.context,
            params.clone(),
            Some(NodeExecutionFailpoint::AfterProviderDispatch),
        )
        .await
        .expect_err("crash after exact provider prompt acceptance");
    assert_reason(&error, "remote_execution_failpoint");
    assert_eq!(harness.driver.session_start_count(), 1);

    let mut status_identity = prepare_identity.clone();
    status_identity.idempotency_key = "status-provider-crash".to_string();
    let status_params = OrchestrationAttemptStatusParams {
        identity: status_identity,
    };
    let status = harness
        .service
        .attempt_status(&harness.context, status_params.clone())
        .await
        .expect("status reattaches exact accepted turn");
    let replay = harness
        .service
        .attempt_status(&harness.context, status_params)
        .await
        .expect("reattached status replays");
    assert_eq!(status, replay);
    assert_eq!(
        status.receipt.state,
        OrchestrationNodeAttemptState::Accepted
    );
    assert_eq!(status.receipt.thread_id, prepared.receipt.thread_id);
    assert_eq!(status.receipt.turn_id, prepared.receipt.turn_id);
    assert_eq!(
        status.receipt.turn_attempt_id,
        prepared.receipt.turn_attempt_id
    );

    let marker = harness
        .service
        .attempt_dispatch(&harness.context, params, None)
        .await
        .expect("dispatch identity replays its pre-provider marker");
    assert_eq!(marker.state, OrchestrationNodeAttemptState::DispatchStarted);
    assert_eq!(harness.driver.session_start_count(), 1);

    let mut cancel_identity = prepare_identity;
    cancel_identity.idempotency_key = "cancel-provider-crash".to_string();
    harness
        .service
        .attempt_cancel(
            &harness.context,
            OrchestrationAttemptCancelParams {
                identity: cancel_identity,
                reason_code: "test_cleanup".to_string(),
            },
        )
        .await
        .expect("clean up reattached session");
}

async fn hub_execution(
    node: &Harness,
    environment_router: Arc<EnvironmentRouter>,
    suffix: &str,
) -> (
    Store,
    RunExecutionService,
    EffectOwnership,
    OrchestrationExecutionIdentity,
    u64,
) {
    let store = Store::in_memory().expect("Hub store");
    store
        .save_settings(&settings(json!({})))
        .expect("Hub settings");
    let base = node.identity(suffix, "hub-base");
    let at_ms = now_ms();
    store
        .save_project_repository(&ProjectRepositoryRecord {
            id: base.repository_id.clone(),
            project_id: ProjectId::new("project-bundle-test"),
            organization_id: OrganizationId::new("organization-bundle-test"),
            canonical_remote_identity: "example.invalid/pandamux/remote-test".to_string(),
            canonical_repository_root_identity: format!("git-common-dir-sha256:{}", "d".repeat(64)),
            display_path: "remote-node-owned".to_string(),
            revision: ConfigurationRevision::INITIAL,
            created_at_ms: at_ms,
            updated_at_ms: at_ms,
        })
        .expect("Hub repository record");
    let task = AgentTask {
        id: base.task_id.clone(),
        summary: "Execute the two-store fixture".to_string(),
        class: "build".to_string(),
        project_id: ProjectId::new("project-bundle-test"),
        organization_id: OrganizationId::new("organization-bundle-test"),
        required_capabilities: BTreeSet::new(),
        access_ceiling: AccessMode::ReadOnly,
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
        budget_limit: BudgetAmount {
            tokens: 10_000,
            cost_micro_usd: 100_000,
            runtime_ms: 600_000,
        },
        budget_reservation_id: None,
    };
    let seed = RunSeed {
        trigger: RunTrigger::Manual {
            client_id: "hub-remote-test".to_string(),
        },
        policy_revision: "hub-remote-policy".to_string(),
        bundle_sha: node.bundle_sha.as_str().to_string(),
        organization_id: OrganizationId::new("organization-bundle-test"),
        mode: OrchestrationMode::Auto,
        budget_limit: BudgetAmount {
            tokens: 10_000,
            cost_micro_usd: 100_000,
            runtime_ms: 600_000,
        },
        max_parallel_tasks: 2,
    };
    let plan = ValidatedPlan {
        schema_version: 1,
        source_digest: "c".repeat(64),
        tasks: BTreeMap::from([(base.task_id.clone(), task)]),
        edges: BTreeMap::new(),
        topological_order: vec![base.task_id.clone()],
        total_budget: BudgetAmount {
            tokens: 10_000,
            cost_micro_usd: 100_000,
            runtime_ms: 600_000,
        },
    };
    let mut run = Run::draft(base.run_id.clone(), seed.clone(), at_ms);
    let mut events = Vec::new();
    apply_run(
        &mut run,
        &mut events,
        RunCommand::AcceptPlan { seed, plan },
        at_ms,
    );
    apply_run(
        &mut run,
        &mut events,
        RunCommand::SelectRoute {
            decision: base.route.clone(),
        },
        at_ms,
    );
    store
        .insert_orchestration_run(
            &OrchestrationRunRecord {
                run,
                project_id: ProjectId::new("project-bundle-test"),
                repository_id: base.repository_id.clone(),
                settings_revision: 1,
            },
            &events,
        )
        .expect("accepted Hub run");

    let attempt = Attempt {
        id: base.attempt_id.clone(),
        task_id: base.task_id.clone(),
        ordinal: 1,
        route: base.route.clone(),
        thread_id: None,
        turn_attempt_id: None,
        status: AttemptStatus::DispatchRequested,
        boundary: AttemptBoundary::SafeToRetry,
        started_at_ms: at_ms,
        ended_at_ms: None,
        failure: None,
        usage_reconciled: false,
        ambiguous_effect: false,
        ambiguity_reason: None,
        worktree_id: None,
        checkpoint_ids: BTreeSet::new(),
        cleanup_state: CleanupState::NotStarted,
    };
    let transition = ExecutionBudgetService::new(store.clone())
        .plan_admission(PlanExecutionBudgetRequest {
            run_id: base.run_id.clone(),
            attempt,
            agent_id: AgentId::new(AGENT_ID),
            organization_id: OrganizationId::new("organization-bundle-test"),
            agent_budget_limit: BudgetAmount {
                tokens: 400_000,
                cost_micro_usd: 5_000_000,
                runtime_ms: 1_800_000,
            },
            trusted_estimate: Some(BudgetAmount {
                tokens: 1_000,
                cost_micro_usd: 10_000,
                runtime_ms: 60_000,
            }),
            daily_budget_limit_micro_usd: 10_000_000,
            claimed_at_ms: at_ms,
        })
        .await
        .expect("Hub budget plan");
    let admission = ExecutionAdmissionService::with_owner(
        store.clone(),
        ServerIncarnationId::new("hub-admission-owner"),
    );
    let queue = admission
        .enqueue_orchestrated(EnqueueOrchestratedAdmissionRequest {
            queue_id: format!("hub-remote-queue-{suffix}"),
            run_id: base.run_id.clone(),
            task_id: base.task_id.clone(),
            attempt_id: base.attempt_id.clone(),
            agent_id: AgentId::new(AGENT_ID),
            organization_id: OrganizationId::new("organization-bundle-test"),
            agent_concurrency_limit: 2,
            provider_instance_id: ProviderInstanceId::new(PROVIDER_ID),
            environment_id: EnvironmentId::new(ENVIRONMENT_ID),
            priority: 0,
            task_enqueued_at_ms: at_ms,
            enqueued_at_ms: at_ms,
            not_before_ms: at_ms,
        })
        .await
        .expect("Hub remote queue");
    admission
        .try_claim_record(&queue, at_ms, Some(transition))
        .await
        .expect("Hub admission claim")
        .expect("Hub remote attempt has capacity");

    let service =
        RunExecutionService::new(store.clone()).with_environment_router(environment_router);
    let effect = service
        .pending_effects(16)
        .expect("pending Hub effects")
        .into_iter()
        .find(|candidate| {
            matches!(
                &candidate.effect.kind,
                OrchestrationEffectKind::DispatchTask { attempt_id, .. }
                    if attempt_id == &base.attempt_id
            )
        })
        .expect("remote dispatch effect");
    let owner = ServerIncarnationId::new("hub-effect-owner");
    let lease_id = ExecutionLeaseId::new("hub-effect-lease");
    let claim_revision = effect.delivery.claim_revision + 1;
    service
        .claim_effect(
            &EffectClaimRequest {
                effect_id: effect.effect.id.clone(),
                expected_payload_sha256: effect.effect.payload_sha256.clone(),
                owner_incarnation_id: owner.clone(),
                lease: ExecutionLease {
                    id: lease_id.clone(),
                    resource_kind: LeaseResourceKind::Effect,
                    resource_id: effect.effect.id.as_str().to_string(),
                    owner_incarnation_id: owner.clone(),
                    expires_at_ms: at_ms + 120_000,
                    last_heartbeat_ms: at_ms,
                    revision: claim_revision,
                },
                claimed_at_ms: at_ms,
            },
            None,
        )
        .expect("claim Hub remote effect");
    let ownership = EffectOwnership {
        effect_id: effect.effect.id.clone(),
        expected_payload_sha256: effect.effect.payload_sha256.clone(),
        owner_incarnation_id: owner,
        lease_id,
        claim_revision,
    };
    let identity = OrchestrationExecutionIdentity {
        effect_id: effect.effect.id,
        payload_sha256: ContentSha256::parse(effect.effect.payload_sha256)
            .expect("effect payload digest"),
        ..base
    };
    (store, service, ownership, identity, at_ms + 60_000)
}

fn apply_run(run: &mut Run, events: &mut Vec<RunEvent>, command: RunCommand, at_ms: u64) {
    let kinds = pandamux_orchestrator::decide(run, command).expect("legal Hub run transition");
    for kind in kinds {
        let seq = run.last_seq + 1;
        let event = RunEvent {
            event_id: RunEventId::new(format!("hub-remote-event-{seq}")),
            run_id: run.id.clone(),
            seq,
            at_ms,
            idempotency_key: format!("hub-remote:{seq}"),
            kind,
        };
        *run = pandamux_orchestrator::apply(run, &event)
            .expect("apply Hub run event")
            .run;
        events.push(event);
    }
}

fn settings(provider_settings: serde_json::Value) -> UserSettings {
    UserSettings {
        environments: vec![Environment {
            id: EnvironmentId::new(ENVIRONMENT_ID),
            kind: EnvironmentKind::Local,
            display_name: "Remote node test".to_string(),
            status: EnvironmentStatus::Ready,
            platform: None,
            server_version: None,
            schedules_count: Some(0),
            last_error: None,
            execution_limit: 2,
            provider_overrides: BTreeMap::new(),
        }],
        providers: vec![ProviderInstanceConfig {
            id: ProviderInstanceId::new(PROVIDER_ID),
            provider: ProviderKind::Custom,
            display_name: "Remote mock".to_string(),
            profile_dir: "profiles/remote-mock".to_string(),
            environment_id: Some(EnvironmentId::new(ENVIRONMENT_ID)),
            binary_path: Some("mock-provider".to_string()),
            concurrency_limit: 2,
            env_overrides: BTreeMap::new(),
            settings: provider_settings,
            enabled: true,
            disabled_reason: None,
        }],
        organizations: vec![OrganizationPolicy {
            id: OrganizationId::new("organization-bundle-test"),
            display_name: "Remote organization".to_string(),
            project_ids: vec![ProjectId::new("project-bundle-test")],
            allowed_provider_instances: vec![ProviderInstanceId::new(PROVIDER_ID)],
            allowed_subscriptions: Vec::new(),
            failover_pool_ids: Vec::new(),
            default_tier_mapping: None,
            max_concurrency: Some(2),
            daily_budget_micro_usd: None,
            broadside: BroadsidePolicy::default(),
            revision: 1,
        }],
        advanced: AdvancedSettings {
            global_concurrency_limit: 2,
            ..AdvancedSettings::default()
        },
        ..UserSettings::default()
    }
}

fn digest(value: char) -> ContentSha256 {
    ContentSha256::parse(value.to_string().repeat(64)).expect("digest")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn git(repository: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .current_dir(repository)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .expect("run fixture Git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn git_text(repository: &Path, args: &[&str]) -> String {
    String::from_utf8(git(repository, args))
        .expect("UTF-8 Git output")
        .trim()
        .to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_millis() as u64
}

fn assert_reason(error: &RpcError, expected: &str) {
    assert_eq!(
        error
            .data
            .as_ref()
            .and_then(|data| data.get("reason"))
            .and_then(serde_json::Value::as_str),
        Some(expected),
        "{error:?}"
    );
}
