use super::*;
use crate::bundle_store::tests::materialization_fixture;
use crate::{
    AttemptWorktreeService, CapabilityAuthority, CreateAttemptCheckpointRequest,
    CreateAttemptWorktreeRequest, DataPaths, DriverRegistry, EnqueueOrchestratedAdmissionRequest,
    ExecutionAdmissionService, ExecutionBudgetService, OrchestrationRunRecord,
    PlanExecutionBudgetRequest, ProviderResolver,
};
use pandamux_core::{
    AccessMode, AgentId, AgentTask, Attempt, AttemptBoundary, AttemptStatus, AttemptWorktreeId,
    BroadsidePolicy, BudgetAmount, CheckpointId, CheckpointKind, CleanupState,
    ConfigurationRevision, EffectDeliveryState, Environment, EnvironmentId, EnvironmentKind,
    EnvironmentStatus, ExecutionLease, ExecutionLeaseId, LeaseResourceKind, OrchestrationMode,
    OrganizationId, OrganizationPolicy, OutputContract, ProjectId, ProjectRepositoryId,
    ProjectRepositoryRecord, ProviderInstanceConfig, ProviderInstanceId, ProviderKind,
    RoutingDecision, Run, RunAttemptId, RunCommand, RunEvent, RunEventId, RunId, RunSeed,
    RunStatus, RunTaskId, RunTaskStatus, RunTrigger, ServerIncarnationId, SubscriptionKey,
    TaskRouteDecision, ToolLedgerStatus, UsageProvider, UsageSubscription, UserSettings,
    ValidatedPlan,
};
use pandamux_providers::MockProviderDriver;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ORGANIZATION_ID: &str = "organization-bundle-test";
const PROJECT_ID: &str = "project-bundle-test";
const REPOSITORY_ID: &str = "repository-local-provider";
const ENVIRONMENT_ID: &str = "environment-local-provider";
const AGENT_ID: &str = "builder";
const PRIMARY_PROVIDER_ID: &str = "provider-test";
const FALLBACK_PROVIDER_ID: &str = "provider-fallback";
const MODEL: &str = "test-model";

struct LocalProviderHarness {
    store: Store,
    bundle_store: BundleStore,
    service: RunExecutionService,
    admission: ExecutionAdmissionService,
    worktrees: AttemptWorktreeService,
    driver: Arc<MockProviderDriver>,
    resolver: Arc<ProviderResolver>,
    repository: PathBuf,
    baseline: String,
    run_id: RunId,
    task_id: RunTaskId,
    _temp: tempfile::TempDir,
}

impl LocalProviderHarness {
    async fn new(primary_settings: Value) -> Self {
        let temp = tempfile::tempdir().expect("temporary local provider fixture");
        let root = fs::canonicalize(temp.path()).expect("canonical temporary fixture root");
        let paths = DataPaths::from_root(root.join("app-data")).expect("data paths");
        paths.ensure_private_roots().expect("private data roots");
        let store = Store::open(paths.database()).expect("fixture store");
        let bundle_store = BundleStore::new(paths.clone(), store.clone());
        let bundle_input = materialization_fixture(&root);
        let materialized = bundle_store
            .materialize(&bundle_input, now_ms())
            .expect("materialize exact bundle");

        let repository = root.join("repository");
        fs::create_dir(&repository).expect("repository directory");
        git(&repository, &["init", "-b", "main"]);
        git(&repository, &["config", "user.name", "PandaMUX Test"]);
        git(
            &repository,
            &["config", "user.email", "pandamux-test@localhost"],
        );
        fs::write(repository.join("tracked.txt"), b"local provider baseline\n")
            .expect("baseline file");
        git(&repository, &["add", "--", "tracked.txt"]);
        git(&repository, &["commit", "-m", "baseline"]);
        let baseline = git_text(&repository, &["rev-parse", "HEAD"]);
        let common_identity = format!(
            "git-common-dir-sha256:{}",
            sha256_hex(
                canonical_common_dir(&repository)
                    .to_string_lossy()
                    .as_bytes()
            )
        );
        store
            .save_project_repository(&ProjectRepositoryRecord {
                id: ProjectRepositoryId::new(REPOSITORY_ID),
                project_id: ProjectId::new(PROJECT_ID),
                organization_id: OrganizationId::new(ORGANIZATION_ID),
                canonical_remote_identity: "example.invalid/pandamux/local-provider".to_string(),
                canonical_repository_root_identity: common_identity,
                display_path: repository.to_string_lossy().into_owned(),
                revision: ConfigurationRevision::INITIAL,
                created_at_ms: now_ms(),
                updated_at_ms: now_ms(),
            })
            .expect("repository record");
        store
            .save_settings(&settings(primary_settings))
            .expect("provider settings");

        let run_id = RunId::new("run-local-provider");
        let task_id = RunTaskId::new("task-local-provider");
        let route = route(&task_id, PRIMARY_PROVIDER_ID);
        let task = AgentTask {
            id: task_id.clone(),
            summary: "Implement the local provider fixture".to_string(),
            class: "build".to_string(),
            project_id: ProjectId::new(PROJECT_ID),
            organization_id: OrganizationId::new(ORGANIZATION_ID),
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
                runtime_ms: 1_000_000,
            },
            budget_reservation_id: None,
        };
        let seed = RunSeed {
            trigger: RunTrigger::Manual {
                client_id: "client-local-provider".to_string(),
            },
            policy_revision: "policy-local-provider".to_string(),
            bundle_sha: materialized.object.bundle_sha.as_str().to_string(),
            organization_id: OrganizationId::new(ORGANIZATION_ID),
            mode: OrchestrationMode::Auto,
            budget_limit: BudgetAmount {
                tokens: 10_000,
                cost_micro_usd: 100_000,
                runtime_ms: 1_000_000,
            },
            max_parallel_tasks: 1,
        };
        let plan = ValidatedPlan {
            schema_version: 1,
            source_digest: "c".repeat(64),
            tasks: BTreeMap::from([(task_id.clone(), task)]),
            edges: BTreeMap::new(),
            topological_order: vec![task_id.clone()],
            total_budget: BudgetAmount {
                tokens: 10_000,
                cost_micro_usd: 100_000,
                runtime_ms: 1_000_000,
            },
        };
        let base_time = now_ms().saturating_sub(1_000);
        let mut run = Run::draft(run_id.clone(), seed.clone(), base_time);
        let mut events = Vec::new();
        apply(
            &mut run,
            &mut events,
            RunCommand::AcceptPlan { seed, plan },
            base_time,
        );
        apply(
            &mut run,
            &mut events,
            RunCommand::SelectRoute { decision: route },
            base_time,
        );
        store
            .insert_orchestration_run(
                &OrchestrationRunRecord {
                    run,
                    project_id: ProjectId::new(PROJECT_ID),
                    repository_id: ProjectRepositoryId::new(REPOSITORY_ID),
                    settings_revision: 1,
                },
                &events,
            )
            .expect("insert accepted run");

        let driver = Arc::new(MockProviderDriver::new(
            ProviderKind::Custom,
            "Mock Orchestration Provider",
        ));
        let drivers = Arc::new(DriverRegistry::new());
        drivers.register(ProviderKind::Custom, driver.clone());
        let resolver = Arc::new(ProviderResolver::new(
            store.clone(),
            drivers,
            EnvironmentId::new(ENVIRONMENT_ID),
            paths.root().to_path_buf(),
        ));
        let thread_manager = ThreadManager::new(
            store.clone(),
            resolver.clone(),
            ENVIRONMENT_ID.to_string(),
            Arc::new(CapabilityAuthority::new(EnvironmentId::new(ENVIRONMENT_ID))),
        );
        let service = RunExecutionService::with_local_provider(
            store.clone(),
            thread_manager,
            bundle_store.clone(),
        );

        Self {
            store: store.clone(),
            bundle_store,
            service,
            admission: ExecutionAdmissionService::with_owner(
                store.clone(),
                ServerIncarnationId::new("server-local-provider"),
            ),
            worktrees: AttemptWorktreeService::new(store, paths),
            driver,
            resolver,
            repository,
            baseline,
            run_id,
            task_id,
            _temp: temp,
        }
    }

    async fn prepare_attempt(
        &self,
        ordinal: u32,
        provider_id: &str,
        continuation_id: Option<ContinuationId>,
    ) -> PreparedLocalProviderAttempt {
        let at_ms = now_ms();
        let attempt_id = RunAttemptId::new(format!("attempt-local-provider-{ordinal}"));
        let attempt = Attempt {
            id: attempt_id.clone(),
            task_id: self.task_id.clone(),
            ordinal,
            route: route(&self.task_id, provider_id),
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
        let transition = ExecutionBudgetService::new(self.store.clone())
            .plan_admission(PlanExecutionBudgetRequest {
                run_id: self.run_id.clone(),
                attempt: attempt.clone(),
                agent_id: AgentId::new(AGENT_ID),
                organization_id: OrganizationId::new(ORGANIZATION_ID),
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
            .expect("plan exact attempt budget");
        let queue = self
            .admission
            .enqueue_orchestrated(EnqueueOrchestratedAdmissionRequest {
                queue_id: format!("queue-local-provider-{ordinal}"),
                run_id: self.run_id.clone(),
                task_id: self.task_id.clone(),
                attempt_id: attempt_id.clone(),
                agent_id: AgentId::new(AGENT_ID),
                organization_id: OrganizationId::new(ORGANIZATION_ID),
                agent_concurrency_limit: 2,
                provider_instance_id: ProviderInstanceId::new(provider_id),
                environment_id: EnvironmentId::new(ENVIRONMENT_ID),
                priority: 0,
                task_enqueued_at_ms: at_ms,
                enqueued_at_ms: at_ms,
                not_before_ms: at_ms,
            })
            .await
            .expect("enqueue exact attempt");
        let admission_claim = self
            .admission
            .try_claim_record(&queue, at_ms, Some(transition))
            .await
            .expect("claim attempt admission")
            .expect("attempt has capacity");

        let worktree_id = AttemptWorktreeId::new(format!("worktree-local-provider-{ordinal}"));
        let worktree_lease_id =
            ExecutionLeaseId::new(format!("worktree-lease-local-provider-{ordinal}"));
        self.worktrees
            .create(
                CreateAttemptWorktreeRequest {
                    run_id: self.run_id.clone(),
                    task_id: self.task_id.clone(),
                    attempt_id: attempt_id.clone(),
                    worktree_id: worktree_id.clone(),
                    repository_id: ProjectRepositoryId::new(REPOSITORY_ID),
                    environment_id: EnvironmentId::new(ENVIRONMENT_ID),
                    repository_root: self.repository.clone(),
                    baseline_commit: self.baseline.clone(),
                    lease: ExecutionLease {
                        id: worktree_lease_id.clone(),
                        resource_kind: LeaseResourceKind::Worktree,
                        resource_id: worktree_id.as_str().to_string(),
                        owner_incarnation_id: ServerIncarnationId::new("server-local-provider"),
                        expires_at_ms: at_ms + 30_000,
                        last_heartbeat_ms: at_ms,
                        revision: 1,
                    },
                    intent_at_ms: at_ms,
                    materialized_at_ms: at_ms,
                },
                None,
            )
            .await
            .expect("create owned attempt worktree");
        self.worktrees
            .checkpoint(
                CreateAttemptCheckpointRequest {
                    worktree_id,
                    lease_id: worktree_lease_id,
                    checkpoint_id: CheckpointId::new(format!(
                        "checkpoint-local-provider-{ordinal}"
                    )),
                    kind: CheckpointKind::BeforeAttempt,
                    created_at_ms: at_ms,
                },
                None,
            )
            .await
            .expect("create before-attempt checkpoint");

        let effect = self
            .service
            .pending_effects(16)
            .expect("pending effects")
            .into_iter()
            .find(|candidate| {
                matches!(
                    &candidate.effect.kind,
                    OrchestrationEffectKind::DispatchTask { attempt_id: id, .. }
                        if id == &attempt_id
                )
            })
            .expect("attempt dispatch effect");
        let effect_owner = ServerIncarnationId::new(format!("effect-owner-{ordinal}"));
        let effect_lease_id = ExecutionLeaseId::new(format!("effect-lease-{ordinal}"));
        let claim_revision = effect.delivery.claim_revision + 1;
        let claim = self
            .service
            .claim_effect(
                &EffectClaimRequest {
                    effect_id: effect.effect.id.clone(),
                    expected_payload_sha256: effect.effect.payload_sha256.clone(),
                    owner_incarnation_id: effect_owner.clone(),
                    lease: ExecutionLease {
                        id: effect_lease_id.clone(),
                        resource_kind: LeaseResourceKind::Effect,
                        resource_id: effect.effect.id.as_str().to_string(),
                        owner_incarnation_id: effect_owner.clone(),
                        expires_at_ms: at_ms + 30_000,
                        last_heartbeat_ms: at_ms,
                        revision: claim_revision,
                    },
                    claimed_at_ms: at_ms,
                },
                None,
            )
            .expect("claim exact effect");
        assert!(claim.acquired);
        let starts_before = self.driver.session_start_count();
        let prepared = self
            .service
            .prepare_local_provider_attempt(PrepareLocalProviderAttemptRequest {
                ownership: EffectOwnership {
                    effect_id: effect.effect.id,
                    expected_payload_sha256: effect.effect.payload_sha256,
                    owner_incarnation_id: effect_owner,
                    lease_id: effect_lease_id,
                    claim_revision,
                },
                admission_claim,
                prepared_at_ms: at_ms,
                continuation_id,
            })
            .await
            .expect("prepare local provider attempt");
        assert_eq!(
            self.driver.session_start_count(),
            starts_before,
            "preparation must not start a provider session"
        );
        prepared
    }

    async fn dispatch(
        &self,
        prepared: PreparedLocalProviderAttempt,
        failpoint: Option<OrchestrationExecutionFailpoint>,
    ) -> Result<LocalProviderDispatch, LocalProviderExecutionError> {
        let dispatch_started_at_ms = prepared.dispatch.updated_at_ms.saturating_add(1);
        self.service
            .dispatch_local_provider_attempt(prepared, dispatch_started_at_ms, failpoint)
            .await
    }

    fn run(&self) -> Run {
        self.store
            .load_verified_orchestration_run(&self.run_id)
            .expect("load run")
            .expect("run exists")
            .run
    }
}

impl Drop for LocalProviderHarness {
    fn drop(&mut self) {
        self.bundle_store.shutdown();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn efx_01_persists_before_process_and_runs_with_exact_pinned_authority() {
    let harness = LocalProviderHarness::new(json!({})).await;
    let prepared = harness.prepare_attempt(1, PRIMARY_PROVIDER_ID, None).await;
    assert_eq!(harness.driver.session_start_count(), 0);
    assert_eq!(
        harness
            .store
            .get_attempt_dispatch(&prepared.run_attempt_id)
            .expect("load dispatch")
            .expect("dispatch intent")
            .state,
        AttemptDispatchState::Prepared
    );

    let error = harness
        .dispatch(
            prepared.clone(),
            Some(OrchestrationExecutionFailpoint::BeforeDispatchMarker),
        )
        .await
        .expect_err("pre-marker crash is injected");
    assert!(error.to_string().contains("before_dispatch_marker"));
    assert_eq!(harness.driver.session_start_count(), 0);

    let mut dispatch = harness
        .dispatch(prepared.clone(), None)
        .await
        .expect("dispatch exact prepared turn");
    assert_eq!(dispatch.status, LocalProviderDispatchStatus::Accepted);
    let outcome = finish(&mut dispatch).await;
    assert_eq!(outcome.status, AttemptDispatchState::Terminal);
    assert_eq!(outcome.boundary, AttemptBoundary::VisibleOutputBegun);
    assert_eq!(harness.driver.session_start_count(), 1);
    assert_eq!(
        harness.resolver.active_processes(
            &ProviderInstanceId::new(PRIMARY_PROVIDER_ID),
            &EnvironmentId::new(ENVIRONMENT_ID),
        ),
        0,
        "the final provider process permit must be released"
    );

    let specs = harness.driver.received_specs();
    assert_eq!(specs.len(), 1);
    let spec = &specs[0];
    assert_eq!(spec, &prepared.session_spec);
    assert_eq!(spec.access, AccessMode::ReadOnly);
    assert_eq!(spec.model, MODEL);
    assert_eq!(
        spec.cwd,
        PathBuf::from(prepared.thread.workspace.effective_path())
    );
    let instructions = spec.instructions.as_ref().expect("compiled instructions");
    assert!(instructions.role.contains("Agent identity: builder"));
    assert!(
        instructions
            .role
            .contains("Build the requested change without widening authority.")
    );
    assert_eq!(instructions.knowledge_files.len(), 1);
    assert_eq!(
        spec.tool_policy.as_ref().expect("tool policy").allowed,
        vec!["grep".to_string(), "read".to_string()]
    );
    assert_eq!(
        spec.tool_policy.as_ref().expect("tool policy").disallowed,
        vec!["shell".to_string(), "write".to_string()]
    );
    assert_eq!(
        harness
            .store
            .get_orchestration_effect(&prepared.ownership.effect_id)
            .expect("load effect")
            .expect("effect exists")
            .delivery
            .state,
        EffectDeliveryState::Settled
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn efx_02_verified_not_started_releases_budget_and_falls_back_once() {
    let harness = LocalProviderHarness::new(json!({
        "startFailure": "verified_not_started"
    }))
    .await;
    let first = harness.prepare_attempt(1, PRIMARY_PROVIDER_ID, None).await;
    let first_dispatch = harness
        .dispatch(first.clone(), None)
        .await
        .expect("typed not-started outcome");
    assert_eq!(
        first_dispatch.status,
        LocalProviderDispatchStatus::NotStarted
    );
    assert!(first_dispatch.completion.is_none());
    let first_run = harness.run();
    assert_eq!(
        first_run.attempts[&first.run_attempt_id].status,
        AttemptStatus::Failed
    );
    assert!(first_run.attempts[&first.run_attempt_id].usage_reconciled);
    assert_eq!(
        first_run.tasks[&harness.task_id].status,
        RunTaskStatus::Ready
    );
    let first_runtime = harness
        .store
        .load_orchestration_runtime_records(&harness.run_id)
        .expect("load verified not-started runtime");
    let first_queue = first_runtime
        .queue_records
        .iter()
        .find(|record| record.attempt_id == first.run_attempt_id)
        .expect("verified not-started capacity record");
    assert!(first_queue.owner_incarnation_id.is_none());
    assert!(first_queue.lease_id.is_none());
    assert_eq!(
        first_run.attempts[&first.run_attempt_id].cleanup_state,
        CleanupState::Verified
    );

    let second = harness.prepare_attempt(2, FALLBACK_PROVIDER_ID, None).await;
    let mut second_dispatch = harness
        .dispatch(second.clone(), None)
        .await
        .expect("fallback dispatch");
    assert_eq!(
        second_dispatch.status,
        LocalProviderDispatchStatus::Accepted
    );
    let outcome = finish(&mut second_dispatch).await;
    assert_eq!(outcome.status, AttemptDispatchState::Terminal);
    let final_run = harness.run();
    assert_eq!(
        final_run.attempts[&first.run_attempt_id]
            .route
            .provider
            .selected_provider_instance_id
            .as_ref(),
        Some(&ProviderInstanceId::new(PRIMARY_PROVIDER_ID))
    );
    assert_eq!(
        final_run.attempts[&second.run_attempt_id]
            .route
            .provider
            .selected_provider_instance_id
            .as_ref(),
        Some(&ProviderInstanceId::new(FALLBACK_PROVIDER_ID))
    );
    assert_eq!(
        final_run.tasks[&harness.task_id]
            .route
            .as_ref()
            .and_then(|route| route.provider.selected_provider_instance_id.as_ref()),
        Some(&ProviderInstanceId::new(PRIMARY_PROVIDER_ID)),
        "the accepted first route remains immutable while the retry owns its own route"
    );
    assert_eq!(harness.driver.session_start_count(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verified_not_started_dirty_cleanup_retains_worktree_and_capacity_debt() {
    let harness = LocalProviderHarness::new(json!({
        "startFailure": "verified_not_started"
    }))
    .await;
    let prepared = harness.prepare_attempt(1, PRIMARY_PROVIDER_ID, None).await;
    let worktree_path = PathBuf::from(prepared.thread.workspace.effective_path());
    fs::write(worktree_path.join("retained-untracked.txt"), b"retain me\n")
        .expect("dirty the test-owned attempt worktree");

    let error = harness
        .dispatch(prepared.clone(), None)
        .await
        .expect_err("dirty verified-not-started cleanup remains recovery debt");
    assert!(error.to_string().contains("retained for recovery review"));
    let run = harness.run();
    assert_eq!(
        run.attempts[&prepared.run_attempt_id].status,
        AttemptStatus::Failed
    );
    assert!(run.attempts[&prepared.run_attempt_id].usage_reconciled);
    assert_eq!(
        run.attempts[&prepared.run_attempt_id].cleanup_state,
        CleanupState::Quarantined
    );
    let runtime = harness
        .store
        .load_orchestration_runtime_records(&harness.run_id)
        .expect("load retained not-started runtime");
    let queue = runtime
        .queue_records
        .iter()
        .find(|record| record.attempt_id == prepared.run_attempt_id)
        .expect("retained not-started capacity record");
    assert!(queue.owner_incarnation_id.is_some());
    assert!(queue.lease_id.is_some());
    assert!(runtime.leases.iter().any(|lease| {
        queue
            .lease_id
            .as_ref()
            .is_some_and(|lease_id| lease.id == *lease_id)
    }));
    assert!(worktree_path.exists());
    assert!(runtime.recovery_reviews.iter().any(|review| {
        review.attempt_id == prepared.run_attempt_id
            && review.reason_code == "attempt_not_started_cleanup_failed"
            && review.status == RecoveryReviewStatus::Pending
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_start_and_prompt_receipts_fail_closed_by_typed_disposition() {
    let ambiguous_start = LocalProviderHarness::new(json!({
        "startFailure": "ambiguous"
    }))
    .await;
    let prepared = ambiguous_start
        .prepare_attempt(1, PRIMARY_PROVIDER_ID, None)
        .await;
    let dispatch = ambiguous_start
        .dispatch(prepared.clone(), None)
        .await
        .expect("ambiguous start receipt");
    assert_eq!(dispatch.status, LocalProviderDispatchStatus::Ambiguous);
    assert!(dispatch.completion.is_none());
    assert!(ambiguous_start.run().attempts[&prepared.run_attempt_id].ambiguous_effect);
    assert_eq!(
        ambiguous_start
            .store
            .get_orchestration_effect(&prepared.ownership.effect_id)
            .expect("load start effect")
            .expect("start effect exists")
            .delivery
            .state,
        EffectDeliveryState::ReviewRequired
    );

    let safe_prompt = LocalProviderHarness::new(json!({
        "sendFailure": "verified_not_started"
    }))
    .await;
    let prepared = safe_prompt
        .prepare_attempt(1, PRIMARY_PROVIDER_ID, None)
        .await;
    let dispatch = safe_prompt
        .dispatch(prepared.clone(), None)
        .await
        .expect("safe prompt rejection");
    assert_eq!(dispatch.status, LocalProviderDispatchStatus::NotStarted);
    assert_eq!(
        safe_prompt.resolver.active_processes(
            &ProviderInstanceId::new(PRIMARY_PROVIDER_ID),
            &EnvironmentId::new(ENVIRONMENT_ID),
        ),
        0
    );
    assert_eq!(
        safe_prompt.run().tasks[&safe_prompt.task_id].status,
        RunTaskStatus::Ready
    );

    let ambiguous_prompt = LocalProviderHarness::new(json!({
        "sendFailure": "ambiguous"
    }))
    .await;
    let prepared = ambiguous_prompt
        .prepare_attempt(1, PRIMARY_PROVIDER_ID, None)
        .await;
    let dispatch = ambiguous_prompt
        .dispatch(prepared.clone(), None)
        .await
        .expect("ambiguous prompt receipt");
    assert_eq!(dispatch.status, LocalProviderDispatchStatus::Ambiguous);
    assert!(ambiguous_prompt.run().attempts[&prepared.run_attempt_id].ambiguous_effect);
    assert_eq!(
        ambiguous_prompt.resolver.active_processes(
            &ProviderInstanceId::new(PRIMARY_PROVIDER_ID),
            &EnvironmentId::new(ENVIRONMENT_ID),
        ),
        1,
        "an ambiguous prompt keeps the exact provider owner quarantined"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn efx_03_post_marker_response_loss_is_suppressed_without_provider_start() {
    let harness = LocalProviderHarness::new(json!({})).await;
    let prepared = harness.prepare_attempt(1, PRIMARY_PROVIDER_ID, None).await;
    let error = harness
        .dispatch(
            prepared.clone(),
            Some(OrchestrationExecutionFailpoint::AfterDispatchMarker),
        )
        .await
        .expect_err("post-marker response loss");
    assert!(error.to_string().contains("after_dispatch_marker"));
    assert_eq!(harness.driver.session_start_count(), 0);

    let replay = harness
        .dispatch(prepared.clone(), None)
        .await
        .expect("post-marker replay is suppressed");
    assert_eq!(
        replay.status,
        LocalProviderDispatchStatus::SuppressedAfterMarker
    );
    assert!(replay.receipt.is_none());
    assert!(replay.completion.is_none());
    assert_eq!(harness.driver.session_start_count(), 0);
    assert_eq!(
        harness
            .store
            .get_attempt_dispatch(&prepared.run_attempt_id)
            .expect("load dispatch")
            .expect("dispatch exists")
            .state,
        AttemptDispatchState::DispatchStarted
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn efx_04_duplicate_events_are_idempotent_and_efx_05_false_success_is_ambiguous() {
    let duplicate = LocalProviderHarness::new(json!({
        "eventScript": "duplicate_tool_result:read"
    }))
    .await;
    let prepared = duplicate
        .prepare_attempt(1, PRIMARY_PROVIDER_ID, None)
        .await;
    let mut dispatch = duplicate
        .dispatch(prepared.clone(), None)
        .await
        .expect("duplicate event dispatch");
    let outcome = finish(&mut dispatch).await;
    assert_eq!(outcome.status, AttemptDispatchState::Terminal);
    let ledger = duplicate
        .store
        .list_tool_ledger(&prepared.turn.id)
        .expect("duplicate ledger");
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].status, ToolLedgerStatus::Succeeded);

    let unresolved = LocalProviderHarness::new(json!({
        "eventScript": "unresolved_tool_success:write"
    }))
    .await;
    let prepared = unresolved
        .prepare_attempt(1, PRIMARY_PROVIDER_ID, None)
        .await;
    let mut dispatch = unresolved
        .dispatch(prepared.clone(), None)
        .await
        .expect("unresolved tool dispatch");
    let outcome = finish(&mut dispatch).await;
    assert_eq!(outcome.status, AttemptDispatchState::Ambiguous);
    assert_eq!(outcome.boundary, AttemptBoundary::ToolSideEffectBegun);
    let run = unresolved.run();
    assert!(run.attempts[&prepared.run_attempt_id].ambiguous_effect);
    assert_eq!(
        run.tasks[&unresolved.task_id].status,
        RunTaskStatus::Blocked
    );
    assert_eq!(
        unresolved
            .store
            .get_orchestration_effect(&prepared.ownership.effect_id)
            .expect("load effect")
            .expect("effect exists")
            .delivery
            .state,
        EffectDeliveryState::ReviewRequired
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn efx_06_to_08_capacity_continuations_preserve_retry_boundaries() {
    for (mode, expected_boundary, expected_task_status) in [
        (
            "before_output",
            AttemptBoundary::SafeToRetry,
            RunTaskStatus::Blocked,
        ),
        (
            "after_text",
            AttemptBoundary::VisibleOutputBegun,
            RunTaskStatus::Blocked,
        ),
        (
            "after_approval",
            AttemptBoundary::ExternallyCommitted,
            RunTaskStatus::Blocked,
        ),
    ] {
        let harness = LocalProviderHarness::new(json!({
            "capacityFailure": mode
        }))
        .await;
        let prepared = harness.prepare_attempt(1, PRIMARY_PROVIDER_ID, None).await;
        let mut dispatch = harness
            .dispatch(prepared.clone(), None)
            .await
            .expect("capacity dispatch");
        let outcome = finish(&mut dispatch).await;
        assert_eq!(outcome.status, AttemptDispatchState::Terminal, "{mode}");
        assert_eq!(outcome.boundary, expected_boundary, "{mode}");
        assert!(outcome.continuation_id.is_some(), "{mode}");
        let run = harness.run();
        assert_eq!(
            run.attempts[&prepared.run_attempt_id].boundary, expected_boundary,
            "{mode}"
        );
        assert_eq!(
            run.tasks[&harness.task_id].status, expected_task_status,
            "{mode}"
        );
        assert_eq!(harness.driver.session_start_count(), 1, "{mode}");
        assert_eq!(
            harness.resolver.active_processes(
                &ProviderInstanceId::new(PRIMARY_PROVIDER_ID),
                &EnvironmentId::new(ENVIRONMENT_ID),
            ),
            0,
            "{mode}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn life_local_cancellation_verifies_cleanup_before_terminal_run_settlement() {
    let harness = LocalProviderHarness::new(json!({
        "eventScript": "request_approval: hold cancellation fixture open",
        "interruptUsage": {
            "inputTokens": 15,
            "outputTokens": 25,
            "providerReportedMicroUsd": 1_000
        }
    }))
    .await;
    map_primary_usage(&harness);
    let prepared = harness.prepare_attempt(1, PRIMARY_PROVIDER_ID, None).await;
    let worktree = harness
        .store
        .load_attempt_worktree_for_attempt(&harness.run_id, &prepared.run_attempt_id)
        .expect("load cancellation worktree")
        .expect("cancellation worktree exists");
    let worktree_path = PathBuf::from(&worktree.worktree.path);
    let mut dispatch = harness
        .dispatch(prepared.clone(), None)
        .await
        .expect("dispatch cancellable provider attempt");
    assert_eq!(dispatch.status, LocalProviderDispatchStatus::Accepted);

    let reports = harness
        .service
        .cancel_run(&harness.run_id, "life-local-cancel", now_ms())
        .await
        .expect("cancel local orchestration run");
    let cancellation_run = harness.run();
    let cancellation_runtime = harness
        .store
        .load_orchestration_runtime_records(&harness.run_id)
        .expect("load cancellation diagnostics");
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].cleanup_state,
        CleanupState::Verified,
        "reports={reports:?}; attempt={:?}; reviews={:?}",
        cancellation_run.attempts.get(&prepared.run_attempt_id),
        cancellation_runtime.recovery_reviews,
    );
    assert!(reports[0].death_verified);
    assert!(reports[0].writers_joined);
    assert!(reports[0].leases_released);
    let outcome = finish(&mut dispatch).await;
    assert_eq!(outcome.status, AttemptDispatchState::Cancelled);

    let run = harness.run();
    assert_eq!(run.status, RunStatus::Cancelling);
    assert_eq!(
        run.attempts[&prepared.run_attempt_id].cleanup_state,
        CleanupState::Verified
    );
    assert!(run.attempts[&prepared.run_attempt_id].usage_reconciled);
    assert_eq!(harness.service.live_owned_attempts().await, 0);
    assert!(!worktree_path.exists());
    let runtime = harness
        .store
        .load_orchestration_runtime_records(&harness.run_id)
        .expect("load cancelled runtime");
    assert!(!runtime.recovery_reviews.iter().any(|review| {
        review.attempt_id == prepared.run_attempt_id
            && review.reason_code == "recovery_usage_reconciliation_unknown"
    }));
    let queue = runtime
        .queue_records
        .into_iter()
        .find(|record| record.attempt_id == prepared.run_attempt_id)
        .expect("cancelled capacity record");
    assert!(queue.owner_incarnation_id.is_none());
    assert!(queue.lease_id.is_none());
    let effects = harness
        .store
        .load_orchestration_effects(&harness.run_id)
        .expect("load cancellation effects");
    assert_eq!(
        effects
            .iter()
            .find(|effect| effect.effect.id == prepared.ownership.effect_id)
            .expect("provider dispatch effect")
            .delivery
            .state,
        EffectDeliveryState::Settled
    );
    assert_eq!(
        effects
            .iter()
            .find(|effect| {
                matches!(
                    &effect.effect.kind,
                    OrchestrationEffectKind::CancelAttempt { attempt_id, .. }
                        if attempt_id == &prepared.run_attempt_id
                )
            })
            .expect("cancel worker effect")
            .delivery
            .state,
        EffectDeliveryState::Settled
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn life_service_shutdown_drains_active_owner_and_reports_zero_debt() {
    let harness = LocalProviderHarness::new(json!({
        "eventScript": "request_approval: hold shutdown fixture open",
        "interruptUsage": {
            "inputTokens": 15,
            "outputTokens": 25,
            "providerReportedMicroUsd": 1_000
        }
    }))
    .await;
    map_primary_usage(&harness);
    let prepared = harness.prepare_attempt(1, PRIMARY_PROVIDER_ID, None).await;
    let worktree = harness
        .store
        .load_attempt_worktree_for_attempt(&harness.run_id, &prepared.run_attempt_id)
        .expect("load shutdown worktree")
        .expect("shutdown worktree exists");
    let worktree_path = PathBuf::from(&worktree.worktree.path);
    let mut dispatch = harness
        .dispatch(prepared.clone(), None)
        .await
        .expect("dispatch shutdown provider attempt");

    let report = harness.service.shutdown(Duration::from_secs(5)).await;
    let shutdown_run = harness.run();
    let shutdown_runtime = harness
        .store
        .load_orchestration_runtime_records(&harness.run_id)
        .expect("load shutdown diagnostics");
    let shutdown_turn = harness
        .store
        .get_turn(&prepared.turn.id)
        .expect("load shutdown turn");
    let shutdown_turn_attempt = harness
        .store
        .get_turn_attempt(&prepared.turn_attempt.id)
        .expect("load shutdown turn attempt");
    assert!(report.intake_closed);
    assert!(report.claims_stopped);
    assert_eq!(report.attempts_total, 1);
    assert_eq!(
        report.attempts_verified,
        1,
        "{report:?}; attempt={:?}; turn={shutdown_turn:?}; turn_attempt={shutdown_turn_attempt:?}; progress={:?}; reviews={:?}",
        shutdown_run.attempts[&prepared.run_attempt_id],
        shutdown_runtime.progress,
        shutdown_runtime.recovery_reviews
    );
    assert_eq!(report.attempts_quarantined, 0);
    assert!(!report.timed_out);
    assert_eq!(report.live_owned_tasks, 0);
    assert!(report.sqlite_checkpointed);
    assert_eq!(harness.service.live_owned_attempts().await, 0);
    assert_eq!(
        finish(&mut dispatch).await.status,
        AttemptDispatchState::Cancelled
    );
    assert_eq!(
        harness.run().attempts[&prepared.run_attempt_id].cleanup_state,
        CleanupState::Verified
    );
    assert!(!worktree_path.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn life_unknown_cancellation_usage_retains_worktree_and_capacity_debt() {
    let harness = LocalProviderHarness::new(json!({
        "eventScript": "request_approval: hold unknown-usage cancellation open"
    }))
    .await;
    map_primary_usage(&harness);
    let prepared = harness.prepare_attempt(1, PRIMARY_PROVIDER_ID, None).await;
    let worktree = harness
        .store
        .load_attempt_worktree_for_attempt(&harness.run_id, &prepared.run_attempt_id)
        .expect("load unknown-usage worktree")
        .expect("unknown-usage worktree exists");
    let worktree_path = PathBuf::from(&worktree.worktree.path);
    let mut dispatch = harness
        .dispatch(prepared.clone(), None)
        .await
        .expect("dispatch unknown-usage cancellation attempt");

    let first_service = harness.service.clone();
    let second_service = harness.service.clone();
    let first_attempt_id = prepared.run_attempt_id.clone();
    let second_attempt_id = prepared.run_attempt_id.clone();
    let cancellation_at_ms = now_ms();
    let (first_report, second_report) = tokio::join!(
        first_service.cancel_attempt(&first_attempt_id, cancellation_at_ms),
        second_service.cancel_attempt(&second_attempt_id, cancellation_at_ms),
    );
    let report = first_report.expect("first concurrent cancellation caller");
    assert_eq!(
        report,
        second_report.expect("second concurrent cancellation caller")
    );
    assert_eq!(report.cleanup_state, CleanupState::Quarantined);
    assert_eq!(report.reason_code, Some("attempt_cleanup_receipt_failed"));
    assert!(report.death_verified);
    assert!(report.writers_joined);
    assert!(!report.leases_released);
    assert_eq!(
        finish(&mut dispatch).await.status,
        AttemptDispatchState::Cancelled
    );

    let run = harness.run();
    assert_eq!(run.status, RunStatus::Cancelling);
    assert!(!run.attempts[&prepared.run_attempt_id].usage_reconciled);
    assert_eq!(
        run.attempts[&prepared.run_attempt_id].cleanup_state,
        CleanupState::Quarantined
    );
    assert_eq!(harness.service.live_owned_attempts().await, 1);
    assert!(worktree_path.exists());
    let runtime = harness
        .store
        .load_orchestration_runtime_records(&harness.run_id)
        .expect("load unknown-usage runtime");
    assert!(runtime.recovery_reviews.iter().any(|review| {
        review.attempt_id == prepared.run_attempt_id
            && review.reason_code == "recovery_usage_reconciliation_unknown"
            && review.status == RecoveryReviewStatus::Pending
    }));
    let queue = runtime
        .queue_records
        .iter()
        .find(|record| record.attempt_id == prepared.run_attempt_id)
        .expect("unknown-usage capacity record");
    assert!(queue.owner_incarnation_id.is_some());
    assert!(queue.lease_id.is_some());
    assert!(runtime.leases.iter().any(|lease| {
        queue
            .lease_id
            .as_ref()
            .is_some_and(|lease_id| lease.id == *lease_id)
    }));

    let owner_before_recovery = queue.owner_incarnation_id.clone();
    let lease_before_recovery = queue.lease_id.clone();
    let recovery = crate::RunRecoveryService::new(harness.store.clone());
    let recovery_report = recovery
        .recover_nonterminal_runs(
            ServerIncarnationId::new("server-cancellation-debt-recovery"),
            now_ms(),
        )
        .await
        .expect("classify retained cancellation debt");
    assert!(recovery_report.actions.iter().any(|action| {
        action.attempt_id.as_ref() == Some(&prepared.run_attempt_id)
            && action.disposition == crate::RecoveryDisposition::ReviewRequired
    }));
    assert!(!recovery_report.actions.iter().any(|action| {
        action.run_id == harness.run_id
            && action.disposition == crate::RecoveryDisposition::Verified
    }));
    let recovered_runtime = harness
        .store
        .load_orchestration_runtime_records(&harness.run_id)
        .expect("load recovered cancellation debt");
    let recovered_queue = recovered_runtime
        .queue_records
        .iter()
        .find(|record| record.attempt_id == prepared.run_attempt_id)
        .expect("recovered cancellation capacity record");
    assert_eq!(recovered_queue.owner_incarnation_id, owner_before_recovery);
    assert_eq!(recovered_queue.lease_id, lease_before_recovery);
    assert!(recovered_runtime.worktrees.iter().any(|record| {
        record.worktree.attempt_id == prepared.run_attempt_id
            && Path::new(&record.worktree.path) == worktree_path
    }));
    assert!(worktree_path.exists());
}

fn map_primary_usage(harness: &LocalProviderHarness) {
    harness
        .store
        .save_usage_subscription_mapping(
            &UsageSubscription {
                key: SubscriptionKey::new(format!("subscription-{PRIMARY_PROVIDER_ID}")),
                provider: UsageProvider::Custom,
                display_name: "Cancellation usage fixture".to_string(),
                masked_account_label: None,
            },
            &EnvironmentId::new(ENVIRONMENT_ID),
            &ProviderInstanceId::new(PRIMARY_PROVIDER_ID),
            now_ms(),
        )
        .expect("save cancellation usage mapping");
}

async fn finish(dispatch: &mut LocalProviderDispatch) -> LocalProviderTerminalOutcome {
    let completion = dispatch.completion.take().expect("completion owner");
    tokio::time::timeout(Duration::from_secs(5), completion)
        .await
        .expect("provider completion timeout")
        .expect("provider completion task")
        .expect("durable provider outcome")
}

fn settings(primary_settings: Value) -> UserSettings {
    UserSettings {
        environments: vec![Environment {
            id: EnvironmentId::new(ENVIRONMENT_ID),
            kind: EnvironmentKind::Local,
            display_name: "Local provider test".to_string(),
            status: EnvironmentStatus::Ready,
            platform: None,
            server_version: None,
            schedules_count: Some(0),
            last_error: None,
            execution_limit: 2,
            provider_overrides: BTreeMap::new(),
        }],
        providers: vec![
            provider_config(PRIMARY_PROVIDER_ID, primary_settings),
            provider_config(FALLBACK_PROVIDER_ID, json!({})),
        ],
        organizations: vec![OrganizationPolicy {
            id: OrganizationId::new(ORGANIZATION_ID),
            display_name: "Local provider test".to_string(),
            project_ids: vec![ProjectId::new(PROJECT_ID)],
            allowed_provider_instances: vec![
                ProviderInstanceId::new(PRIMARY_PROVIDER_ID),
                ProviderInstanceId::new(FALLBACK_PROVIDER_ID),
            ],
            allowed_subscriptions: Vec::new(),
            failover_pool_ids: Vec::new(),
            default_tier_mapping: None,
            max_concurrency: Some(2),
            daily_budget_micro_usd: Some(10_000_000),
            broadside: BroadsidePolicy::default(),
            revision: 1,
        }],
        advanced: pandamux_core::AdvancedSettings {
            global_concurrency_limit: 2,
            ..pandamux_core::AdvancedSettings::default()
        },
        ..UserSettings::default()
    }
}

fn provider_config(id: &str, settings: Value) -> ProviderInstanceConfig {
    ProviderInstanceConfig {
        id: ProviderInstanceId::new(id),
        provider: ProviderKind::Custom,
        display_name: id.to_string(),
        profile_dir: format!("profiles/{id}"),
        environment_id: Some(EnvironmentId::new(ENVIRONMENT_ID)),
        binary_path: Some(format!("/test/{id}")),
        concurrency_limit: 1,
        env_overrides: BTreeMap::new(),
        settings,
        enabled: true,
        disabled_reason: None,
    }
}

fn route(task_id: &RunTaskId, provider_id: &str) -> TaskRouteDecision {
    TaskRouteDecision {
        task_id: task_id.clone(),
        selected_agent_id: Some(AgentId::new(AGENT_ID)),
        agent_evaluations: Vec::new(),
        provider: RoutingDecision {
            selected_provider_instance_id: Some(ProviderInstanceId::new(provider_id)),
            selected_subscription_key: Some(SubscriptionKey::new(format!(
                "subscription-{provider_id}"
            ))),
            selected_model: Some(MODEL.to_string()),
            evaluations: Vec::new(),
        },
    }
}

fn apply(run: &mut Run, events: &mut Vec<RunEvent>, command: RunCommand, at_ms: u64) {
    let kinds = pandamux_orchestrator::decide(run, command).expect("legal fixture command");
    for kind in kinds {
        let seq = run.last_seq + 1;
        let event = RunEvent {
            event_id: RunEventId::new(format!("event-local-provider-{seq}")),
            run_id: run.id.clone(),
            seq,
            at_ms,
            idempotency_key: format!("local-provider:{seq}"),
            kind,
        };
        *run = pandamux_orchestrator::apply(run, &event)
            .expect("apply fixture event")
            .run;
        events.push(event);
    }
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

fn canonical_common_dir(repository: &Path) -> PathBuf {
    fs::canonicalize(git_text(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .expect("canonical common directory")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_millis() as u64
}
