use pandamux_core::{
    AccessMode, AgentId, AgentTask, Attempt, AttemptBoundary, AttemptFailure, AttemptStatus,
    AttemptWorktreeId, BudgetAmount, BudgetReservation, BudgetReservationId,
    BudgetReservationState, BudgetUsage, CheckpointId, CheckpointKind, CleanupState,
    ConfigurationRevision, EnvironmentId, ExecutionLease, ExecutionLeaseId, FailureClass,
    LeaseResourceKind, OrchestrationMode, OrganizationId, OutputContract, ProjectId,
    ProjectRepositoryId, ProjectRepositoryRecord, ProviderInstanceId, RoutingDecision, Run,
    RunAttemptId, RunCommand, RunEvent, RunEventId, RunId, RunSeed, RunTaskId, RunTaskStatus,
    RunTrigger, ServerIncarnationId, SubscriptionKey, TaskRouteDecision, ThreadId, TurnAttemptId,
    ValidatedPlan, WorktreeCleanupEligibility, WorktreeDirtyState,
};
use pandamux_server::{
    AssignedRunEvent, AttemptWorktreeAuditStatus, AttemptWorktreeCleanupOutcome,
    AttemptWorktreeFailpoint, AttemptWorktreeService, CleanupAttemptWorktreeRequest,
    CreateAttemptCheckpointRequest, CreateAttemptWorktreeRequest, DataPaths,
    OrchestrationRunRecord, RunTransitionRequest, RunTransitionRows, Store,
    orchestration_snapshot_sha256,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const PROJECT_ID: &str = "project-attempt-worktrees";
const REPOSITORY_ID: &str = "repository-attempt-worktrees";
const ORGANIZATION_ID: &str = "organization-attempt-worktrees";

struct Harness {
    _temp: tempfile::TempDir,
    repository: PathBuf,
    baseline: String,
    paths: DataPaths,
    store: Store,
    service: AttemptWorktreeService,
}

impl Harness {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temporary harness");
        let repository = temp.path().join("repository");
        fs::create_dir(&repository).expect("repository directory");
        git(&repository, &["init", "-b", "main"]);
        git(&repository, &["config", "user.name", "PandaMUX Test"]);
        git(
            &repository,
            &["config", "user.email", "pandamux-test@localhost"],
        );
        fs::write(repository.join("tracked.txt"), b"committed baseline\n").expect("baseline file");
        git(&repository, &["add", "--", "tracked.txt"]);
        git(&repository, &["commit", "-m", "baseline"]);
        let baseline = git_text(&repository, &["rev-parse", "HEAD"]);
        let common_dir = canonical_common_dir(&repository);
        let common_identity = format!(
            "git-common-dir-sha256:{}",
            sha256_hex(common_dir.to_string_lossy().as_bytes())
        );
        let paths = DataPaths::from_root(temp.path().join("app-data")).expect("data paths");
        paths.ensure_private_roots().expect("private roots");
        let store = Store::open(paths.database()).expect("store");
        store
            .save_project_repository(&ProjectRepositoryRecord {
                id: ProjectRepositoryId::new(REPOSITORY_ID),
                project_id: ProjectId::new(PROJECT_ID),
                organization_id: OrganizationId::new(ORGANIZATION_ID),
                canonical_remote_identity: "example.invalid/pandamux/attempt-worktrees".to_string(),
                canonical_repository_root_identity: common_identity,
                display_path: repository.to_string_lossy().into_owned(),
                revision: ConfigurationRevision::INITIAL,
                created_at_ms: 1_000,
                updated_at_ms: 1_000,
            })
            .expect("repository record");
        let service = AttemptWorktreeService::new(store.clone(), paths.clone());
        Self {
            _temp: temp,
            repository,
            baseline,
            paths,
            store,
            service,
        }
    }

    fn add_run(&self, label: &str, base_time: u64) -> CreateAttemptWorktreeRequest {
        self.add_run_for(
            label,
            base_time,
            PROJECT_ID,
            REPOSITORY_ID,
            &self.repository,
            &self.baseline,
        )
    }

    fn add_run_for(
        &self,
        label: &str,
        base_time: u64,
        project_id: &str,
        repository_id: &str,
        repository_root: &Path,
        baseline: &str,
    ) -> CreateAttemptWorktreeRequest {
        let run_id = RunId::new(format!("run-{label}"));
        let task_id = RunTaskId::new(format!("task-{label}"));
        let attempt_id = RunAttemptId::new(format!("attempt-{label}"));
        let worktree_id = AttemptWorktreeId::new(format!("worktree-{label}"));
        let route = route(&task_id, label);
        let task = AgentTask {
            id: task_id.clone(),
            summary: format!("Attempt worktree {label}"),
            class: "build".to_string(),
            project_id: ProjectId::new(project_id),
            organization_id: OrganizationId::new(ORGANIZATION_ID),
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
            max_attempts: 1,
            approval_ids: BTreeSet::new(),
            budget_limit: BudgetAmount {
                tokens: 100,
                cost_micro_usd: 200,
                runtime_ms: 300,
            },
            budget_reservation_id: None,
        };
        let seed = RunSeed {
            trigger: RunTrigger::Manual {
                client_id: format!("client-{label}"),
            },
            policy_revision: format!("policy-{label}"),
            bundle_sha: "a".repeat(64),
            organization_id: OrganizationId::new(ORGANIZATION_ID),
            mode: OrchestrationMode::Auto,
            budget_limit: BudgetAmount {
                tokens: 1_000,
                cost_micro_usd: 2_000,
                runtime_ms: 3_000,
            },
            max_parallel_tasks: 1,
        };
        let plan = ValidatedPlan {
            schema_version: 1,
            source_digest: "b".repeat(64),
            tasks: BTreeMap::from([(task_id.clone(), task)]),
            edges: BTreeMap::new(),
            topological_order: vec![task_id.clone()],
            total_budget: BudgetAmount {
                tokens: 100,
                cost_micro_usd: 200,
                runtime_ms: 300,
            },
        };
        let reservation = BudgetReservation {
            id: BudgetReservationId::new(format!("budget-{label}")),
            task_id: task_id.clone(),
            attempt_id: attempt_id.clone(),
            amount: BudgetAmount {
                tokens: 100,
                cost_micro_usd: 200,
                runtime_ms: 300,
            },
            actual: BudgetUsage::default(),
            ledger_utc_date: "2026-10-09".to_string(),
            state: BudgetReservationState::Reserved,
        };
        let attempt = Attempt {
            id: attempt_id.clone(),
            task_id: task_id.clone(),
            ordinal: 1,
            route: route.clone(),
            thread_id: None,
            turn_attempt_id: None,
            status: AttemptStatus::DispatchRequested,
            boundary: AttemptBoundary::SafeToRetry,
            started_at_ms: base_time + 4,
            ended_at_ms: None,
            failure: None,
            usage_reconciled: false,
            ambiguous_effect: false,
            ambiguity_reason: None,
            worktree_id: None,
            checkpoint_ids: BTreeSet::new(),
            cleanup_state: CleanupState::NotStarted,
        };
        let mut run = Run::draft(run_id.clone(), seed.clone(), base_time + 1);
        let mut events = Vec::new();
        apply_fixture(
            &mut run,
            &mut events,
            RunCommand::AcceptPlan { seed, plan },
            base_time + 1,
        );
        apply_fixture(
            &mut run,
            &mut events,
            RunCommand::SelectRoute { decision: route },
            base_time + 2,
        );
        apply_fixture(
            &mut run,
            &mut events,
            RunCommand::ReserveBudget { reservation },
            base_time + 3,
        );
        apply_fixture(
            &mut run,
            &mut events,
            RunCommand::RequestDispatch { attempt },
            base_time + 4,
        );
        self.store
            .insert_orchestration_run(
                &OrchestrationRunRecord {
                    run,
                    project_id: ProjectId::new(project_id),
                    repository_id: ProjectRepositoryId::new(repository_id),
                    settings_revision: 1,
                },
                &events,
            )
            .expect("insert run");
        CreateAttemptWorktreeRequest {
            run_id,
            task_id,
            attempt_id,
            worktree_id: worktree_id.clone(),
            repository_id: ProjectRepositoryId::new(repository_id),
            environment_id: EnvironmentId::new(format!("environment-{label}")),
            repository_root: repository_root.to_path_buf(),
            baseline_commit: baseline.to_string(),
            lease: ExecutionLease {
                id: ExecutionLeaseId::new(format!("lease-{label}")),
                resource_kind: LeaseResourceKind::Worktree,
                resource_id: worktree_id.as_str().to_string(),
                owner_incarnation_id: ServerIncarnationId::new("server-attempt-worktrees"),
                expires_at_ms: base_time + 100_000,
                last_heartbeat_ms: base_time + 4,
                revision: 1,
            },
            intent_at_ms: base_time + 10,
            materialized_at_ms: base_time + 11,
        }
    }

    fn add_parallel_run(
        &self,
        label: &str,
        base_time: u64,
    ) -> (CreateAttemptWorktreeRequest, CreateAttemptWorktreeRequest) {
        let run_id = RunId::new(format!("run-{label}"));
        let labels = [format!("{label}-one"), format!("{label}-two")];
        let mut tasks = BTreeMap::new();
        let mut task_ids = Vec::new();
        for item in &labels {
            let task_id = RunTaskId::new(format!("task-{item}"));
            tasks.insert(
                task_id.clone(),
                AgentTask {
                    id: task_id.clone(),
                    summary: format!("Parallel attempt {item}"),
                    class: "build".to_string(),
                    project_id: ProjectId::new(PROJECT_ID),
                    organization_id: OrganizationId::new(ORGANIZATION_ID),
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
                    max_attempts: 1,
                    approval_ids: BTreeSet::new(),
                    budget_limit: BudgetAmount {
                        tokens: 100,
                        cost_micro_usd: 200,
                        runtime_ms: 300,
                    },
                    budget_reservation_id: None,
                },
            );
            task_ids.push(task_id);
        }
        let seed = RunSeed {
            trigger: RunTrigger::Manual {
                client_id: format!("client-{label}"),
            },
            policy_revision: format!("policy-{label}"),
            bundle_sha: "a".repeat(64),
            organization_id: OrganizationId::new(ORGANIZATION_ID),
            mode: OrchestrationMode::Auto,
            budget_limit: BudgetAmount {
                tokens: 2_000,
                cost_micro_usd: 4_000,
                runtime_ms: 6_000,
            },
            max_parallel_tasks: 2,
        };
        let plan = ValidatedPlan {
            schema_version: 1,
            source_digest: "c".repeat(64),
            tasks,
            edges: BTreeMap::new(),
            topological_order: task_ids.clone(),
            total_budget: BudgetAmount {
                tokens: 200,
                cost_micro_usd: 400,
                runtime_ms: 600,
            },
        };
        let mut run = Run::draft(run_id.clone(), seed.clone(), base_time + 1);
        let mut events = Vec::new();
        let mut at_ms = base_time + 1;
        apply_fixture(
            &mut run,
            &mut events,
            RunCommand::AcceptPlan { seed, plan },
            at_ms,
        );
        let mut requests = Vec::new();
        for (index, (item, task_id)) in labels.iter().zip(&task_ids).enumerate() {
            let attempt_id = RunAttemptId::new(format!("attempt-{item}"));
            let route = route(task_id, item);
            at_ms += 1;
            apply_fixture(
                &mut run,
                &mut events,
                RunCommand::SelectRoute {
                    decision: route.clone(),
                },
                at_ms,
            );
            at_ms += 1;
            apply_fixture(
                &mut run,
                &mut events,
                RunCommand::ReserveBudget {
                    reservation: BudgetReservation {
                        id: BudgetReservationId::new(format!("budget-{item}")),
                        task_id: task_id.clone(),
                        attempt_id: attempt_id.clone(),
                        amount: BudgetAmount {
                            tokens: 100,
                            cost_micro_usd: 200,
                            runtime_ms: 300,
                        },
                        actual: BudgetUsage::default(),
                        ledger_utc_date: "2026-10-09".to_string(),
                        state: BudgetReservationState::Reserved,
                    },
                },
                at_ms,
            );
            at_ms += 1;
            apply_fixture(
                &mut run,
                &mut events,
                RunCommand::RequestDispatch {
                    attempt: Attempt {
                        id: attempt_id.clone(),
                        task_id: task_id.clone(),
                        ordinal: 1,
                        route,
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
                    },
                },
                at_ms,
            );
            let worktree_id = AttemptWorktreeId::new(format!("worktree-{item}"));
            requests.push(CreateAttemptWorktreeRequest {
                run_id: run_id.clone(),
                task_id: task_id.clone(),
                attempt_id,
                worktree_id: worktree_id.clone(),
                repository_id: ProjectRepositoryId::new(REPOSITORY_ID),
                environment_id: EnvironmentId::new(format!("environment-{item}")),
                repository_root: self.repository.clone(),
                baseline_commit: self.baseline.clone(),
                lease: ExecutionLease {
                    id: ExecutionLeaseId::new(format!("lease-{item}")),
                    resource_kind: LeaseResourceKind::Worktree,
                    resource_id: worktree_id.as_str().to_string(),
                    owner_incarnation_id: ServerIncarnationId::new("server-attempt-worktrees"),
                    expires_at_ms: base_time + 100_000,
                    last_heartbeat_ms: base_time + 10 + index as u64,
                    revision: 1,
                },
                intent_at_ms: base_time + 20 + index as u64,
                materialized_at_ms: base_time + 22 + index as u64,
            });
        }
        self.store
            .insert_orchestration_run(
                &OrchestrationRunRecord {
                    run,
                    project_id: ProjectId::new(PROJECT_ID),
                    repository_id: ProjectRepositoryId::new(REPOSITORY_ID),
                    settings_revision: 1,
                },
                &events,
            )
            .expect("insert parallel run");
        let mut requests = requests.into_iter();
        (
            requests.next().expect("first request"),
            requests.next().expect("second request"),
        )
    }

    fn transition(&self, run_id: &RunId, command: RunCommand, at_ms: u64, label: &str) {
        let current = self
            .store
            .load_verified_orchestration_run(run_id)
            .expect("load run")
            .expect("run exists");
        self.store
            .commit_run_transition(
                &RunTransitionRequest {
                    run_id: run_id.clone(),
                    expected_last_seq: current.run.last_seq,
                    expected_snapshot_sha256: orchestration_snapshot_sha256(&current.run),
                    command,
                    assigned_events: vec![AssignedRunEvent {
                        event_id: RunEventId::new(format!("event-{label}")),
                        idempotency_key: format!("idempotency-{label}"),
                        at_ms,
                    }],
                    rows: RunTransitionRows::default(),
                },
                None,
            )
            .expect("commit fixture transition");
    }

    fn settle_attempt(&self, request: &CreateAttemptWorktreeRequest, at_ms: u64) {
        self.transition(
            &request.run_id,
            RunCommand::StartAttempt {
                attempt_id: request.attempt_id.clone(),
                thread_id: ThreadId::new(format!("thread-{}", request.attempt_id.as_str())),
                turn_attempt_id: TurnAttemptId::new(format!(
                    "turn-attempt-{}",
                    request.attempt_id.as_str()
                )),
            },
            at_ms,
            &format!("start-{}", request.attempt_id.as_str()),
        );
        self.transition(
            &request.run_id,
            RunCommand::SettleAttempt {
                attempt_id: request.attempt_id.clone(),
                status: AttemptStatus::Failed,
                failure: Some(AttemptFailure {
                    class: FailureClass::Unknown,
                    code: "test_terminal".to_string(),
                    safe_message: "Test terminal outcome".to_string(),
                }),
                usage_reconciled: false,
            },
            at_ms + 1,
            &format!("settle-{}", request.attempt_id.as_str()),
        );
    }
}

fn route(task_id: &RunTaskId, label: &str) -> TaskRouteDecision {
    TaskRouteDecision {
        task_id: task_id.clone(),
        selected_agent_id: Some(AgentId::new(format!("agent-{label}"))),
        agent_evaluations: Vec::new(),
        provider: RoutingDecision {
            selected_provider_instance_id: Some(ProviderInstanceId::new(format!(
                "provider-{label}"
            ))),
            selected_subscription_key: Some(SubscriptionKey::new(format!("subscription-{label}"))),
            selected_model: Some("test-model".to_string()),
            evaluations: Vec::new(),
        },
    }
}

fn apply_fixture(run: &mut Run, events: &mut Vec<RunEvent>, command: RunCommand, at_ms: u64) {
    let kinds = pandamux_orchestrator::decide(run, command).expect("fixture command");
    for kind in kinds {
        let event = RunEvent {
            event_id: RunEventId::new(format!("event-{}-{}", run.id.as_str(), run.last_seq + 1)),
            run_id: run.id.clone(),
            seq: run.last_seq + 1,
            at_ms,
            idempotency_key: format!("fixture-{}-{}", run.id.as_str(), run.last_seq + 1),
            kind,
        };
        *run = pandamux_orchestrator::apply(run, &event)
            .expect("fixture event")
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
        .expect("utf8 Git output")
        .trim()
        .to_string()
}

fn canonical_common_dir(repository: &Path) -> PathBuf {
    fs::canonicalize(git_text(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .expect("canonical common dir")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn source_snapshot(repository: &Path) -> (Vec<u8>, Vec<u8>, String, String, Vec<u8>, Vec<u8>) {
    (
        fs::read(repository.join("tracked.txt")).expect("tracked bytes"),
        fs::read(repository.join("untracked.txt")).expect("untracked bytes"),
        git_text(repository, &["rev-parse", "HEAD"]),
        git_text(repository, &["symbolic-ref", "HEAD"]),
        git(
            repository,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        ),
        git(repository, &["ls-files", "--stage", "-z"]),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wt_01_through_08_simultaneous_attempts_are_isolated_and_preserve_the_source_checkout() {
    let harness = Harness::new();
    fs::write(
        harness.repository.join("tracked.txt"),
        b"staged user bytes\n",
    )
    .expect("user tracked edit");
    git(&harness.repository, &["add", "--", "tracked.txt"]);
    fs::write(
        harness.repository.join("tracked.txt"),
        b"unstaged user bytes\n",
    )
    .expect("unstaged user edit");
    fs::write(
        harness.repository.join("untracked.txt"),
        b"untracked user bytes\n",
    )
    .expect("untracked user file");
    let before = source_snapshot(&harness.repository);
    let (first, second) = harness.add_parallel_run("parallel", 10_000);

    let (first_outcome, second_outcome) = tokio::join!(
        harness.service.create(first.clone(), None),
        harness.service.create(second.clone(), None),
    );
    let first_outcome = first_outcome.expect("first worktree");
    let second_outcome = second_outcome.expect("second worktree");
    assert_ne!(
        first_outcome.record.worktree.path,
        second_outcome.record.worktree.path
    );
    assert_ne!(
        first_outcome.record.worktree.branch_ref,
        second_outcome.record.worktree.branch_ref
    );
    assert_ne!(
        first_outcome.record.worktree.lease_id,
        second_outcome.record.worktree.lease_id
    );
    assert_eq!(
        git_text(
            Path::new(&first_outcome.record.worktree.path),
            &["rev-parse", "HEAD"]
        ),
        harness.baseline
    );
    assert_eq!(
        git_text(
            Path::new(&second_outcome.record.worktree.path),
            &["rev-parse", "HEAD"]
        ),
        harness.baseline
    );
    assert_eq!(source_snapshot(&harness.repository), before);
    let first_index_before = git(
        Path::new(&first_outcome.record.worktree.path),
        &["ls-files", "--stage", "-z"],
    );
    let second_index_before = git(
        Path::new(&second_outcome.record.worktree.path),
        &["ls-files", "--stage", "-z"],
    );

    let first_checkpoint = CreateAttemptCheckpointRequest {
        worktree_id: first.worktree_id.clone(),
        lease_id: first.lease.id.clone(),
        checkpoint_id: CheckpointId::new("checkpoint-parallel-one"),
        kind: CheckpointKind::BeforeAttempt,
        created_at_ms: 30_000,
    };
    let second_checkpoint = CreateAttemptCheckpointRequest {
        worktree_id: second.worktree_id.clone(),
        lease_id: second.lease.id.clone(),
        checkpoint_id: CheckpointId::new("checkpoint-parallel-two"),
        kind: CheckpointKind::BeforeAttempt,
        created_at_ms: 30_001,
    };
    let (first_checkpoint, second_checkpoint) = tokio::join!(
        harness.service.checkpoint(first_checkpoint, None),
        harness.service.checkpoint(second_checkpoint, None),
    );
    let first_checkpoint = first_checkpoint.expect("first checkpoint");
    let second_checkpoint = second_checkpoint.expect("second checkpoint");
    assert_ne!(first_checkpoint.id, second_checkpoint.id);
    assert_ne!(first_checkpoint.value, second_checkpoint.value);
    assert_ne!(
        harness
            .paths
            .recovery()
            .join("checkpoint-indices")
            .join(first.attempt_id.as_str()),
        harness
            .paths
            .recovery()
            .join("checkpoint-indices")
            .join(second.attempt_id.as_str())
    );
    assert_eq!(
        git(
            Path::new(&first_outcome.record.worktree.path),
            &["ls-files", "--stage", "-z"]
        ),
        first_index_before
    );
    assert_eq!(
        git(
            Path::new(&second_outcome.record.worktree.path),
            &["ls-files", "--stage", "-z"]
        ),
        second_index_before
    );
    assert_eq!(source_snapshot(&harness.repository), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wt_04_simultaneous_attempts_for_different_repositories_keep_distinct_ownership() {
    let harness = Harness::new();
    let second_repository = harness._temp.path().join("repository-two");
    fs::create_dir(&second_repository).expect("second repository directory");
    git(&second_repository, &["init", "-b", "main"]);
    git(
        &second_repository,
        &["config", "user.name", "PandaMUX Test"],
    );
    git(
        &second_repository,
        &["config", "user.email", "pandamux-test@localhost"],
    );
    fs::write(second_repository.join("second.txt"), b"second baseline\n")
        .expect("second baseline file");
    git(&second_repository, &["add", "--", "second.txt"]);
    git(&second_repository, &["commit", "-m", "second baseline"]);
    let second_baseline = git_text(&second_repository, &["rev-parse", "HEAD"]);
    let second_common = canonical_common_dir(&second_repository);
    harness
        .store
        .save_project_repository(&ProjectRepositoryRecord {
            id: ProjectRepositoryId::new("repository-attempt-worktrees-two"),
            project_id: ProjectId::new("project-attempt-worktrees-two"),
            organization_id: OrganizationId::new(ORGANIZATION_ID),
            canonical_remote_identity: "example.invalid/pandamux/attempt-worktrees-two".to_string(),
            canonical_repository_root_identity: format!(
                "git-common-dir-sha256:{}",
                sha256_hex(second_common.to_string_lossy().as_bytes())
            ),
            display_path: second_repository.to_string_lossy().into_owned(),
            revision: ConfigurationRevision::INITIAL,
            created_at_ms: 2_000,
            updated_at_ms: 2_000,
        })
        .expect("second repository record");
    let first = harness.add_run("different-repository-one", 25_000);
    let second = harness.add_run_for(
        "different-repository-two",
        35_000,
        "project-attempt-worktrees-two",
        "repository-attempt-worktrees-two",
        &second_repository,
        &second_baseline,
    );
    let (first, second) = tokio::join!(
        harness.service.create(first, None),
        harness.service.create(second, None),
    );
    let first = first.expect("first repository worktree");
    let second = second.expect("second repository worktree");
    assert_ne!(
        first.record.worktree.repository_id,
        second.record.worktree.repository_id
    );
    assert_ne!(
        first.record.worktree.git_common_dir_identity,
        second.record.worktree.git_common_dir_identity
    );
    assert_ne!(first.record.worktree.path, second.record.worktree.path);
    assert_eq!(
        git_text(
            Path::new(&second.record.worktree.path),
            &["rev-parse", "HEAD"]
        ),
        second_baseline
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_04_and_05_recover_from_durable_intent_without_duplicate_ownership() {
    let harness = Harness::new();
    let before_git = harness.add_run("crash-before-git", 40_000);
    let error = harness
        .service
        .create(
            before_git.clone(),
            Some(AttemptWorktreeFailpoint::AfterOwnershipIntent),
        )
        .await
        .expect_err("intent failpoint");
    assert_eq!(
        error.reason_code, "attempt_worktree_injected_failure",
        "{error:?}"
    );
    let reopened_store = Store::open(harness.paths.database()).expect("reopen after intent");
    let reopened_service =
        AttemptWorktreeService::new(reopened_store.clone(), harness.paths.clone());
    let intent = reopened_store
        .load_owned_attempt_worktree(&before_git.worktree_id)
        .expect("load intent")
        .expect("intent exists");
    assert_eq!(intent.worktree.dirty_state, WorktreeDirtyState::Ambiguous);
    assert!(!Path::new(&intent.worktree.path).exists());
    let recovered = reopened_service
        .create(before_git.clone(), None)
        .await
        .expect("recover pre-Git crash");
    assert!(recovered.recovered);
    assert_eq!(
        recovered.record.worktree.dirty_state,
        WorktreeDirtyState::Clean
    );
    let mut changed_lease = before_git.clone();
    changed_lease.lease.owner_incarnation_id = ServerIncarnationId::new("foreign-owner");
    let conflict = reopened_service
        .create(changed_lease, None)
        .await
        .expect_err("changed lease owner replay");
    assert_eq!(conflict.reason_code, "attempt_worktree_replay_conflict");

    let after_git = harness.add_run("crash-after-git", 50_000);
    harness
        .service
        .create(
            after_git.clone(),
            Some(AttemptWorktreeFailpoint::AfterWorktreeGit),
        )
        .await
        .expect_err("post-Git failpoint");
    let reopened_store = Store::open(harness.paths.database()).expect("reopen after Git");
    let reopened_service =
        AttemptWorktreeService::new(reopened_store.clone(), harness.paths.clone());
    let intent = reopened_store
        .load_owned_attempt_worktree(&after_git.worktree_id)
        .expect("load post-Git intent")
        .expect("post-Git intent exists");
    assert_eq!(intent.worktree.dirty_state, WorktreeDirtyState::Ambiguous);
    assert!(Path::new(&intent.worktree.path).is_dir());
    let recovered = reopened_service
        .create(after_git.clone(), None)
        .await
        .expect("recover post-Git crash");
    assert!(recovered.recovered);
    assert!(!recovered.git_materialized);
    assert_eq!(
        recovered.record.worktree.dirty_state,
        WorktreeDirtyState::Clean
    );

    let mut changed = after_git;
    changed.baseline_commit = "f".repeat(40);
    let conflict = reopened_service
        .create(changed, None)
        .await
        .expect_err("changed identity replay");
    assert_eq!(conflict.reason_code, "attempt_worktree_replay_conflict");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_intent_and_git_crashes_replay_once_and_changed_bytes_conflict() {
    let harness = Harness::new();
    let request = harness.add_run("checkpoint-crash", 60_000);
    harness
        .service
        .create(request.clone(), None)
        .await
        .expect("worktree");
    let checkpoint = CreateAttemptCheckpointRequest {
        worktree_id: request.worktree_id.clone(),
        lease_id: request.lease.id.clone(),
        checkpoint_id: CheckpointId::new("checkpoint-crash-boundary"),
        kind: CheckpointKind::BeforeAttempt,
        created_at_ms: 60_020,
    };
    harness
        .service
        .checkpoint(
            checkpoint.clone(),
            Some(AttemptWorktreeFailpoint::AfterCheckpointIntent),
        )
        .await
        .expect_err("checkpoint intent failpoint");
    let reopened_store = Store::open(harness.paths.database()).expect("reopen checkpoint intent");
    let reopened_service =
        AttemptWorktreeService::new(reopened_store.clone(), harness.paths.clone());
    let intent = reopened_store
        .load_owned_attempt_worktree(&request.worktree_id)
        .expect("load worktree")
        .expect("worktree exists");
    assert!(
        intent
            .checkpoint_intent_sha256
            .contains_key(&checkpoint.checkpoint_id)
    );
    let mut changed = checkpoint.clone();
    changed.created_at_ms += 1;
    let conflict = reopened_service
        .checkpoint(changed, None)
        .await
        .expect_err("changed checkpoint replay");
    assert_eq!(conflict.reason_code, "attempt_worktree_store_rejected");

    reopened_service
        .checkpoint(
            checkpoint.clone(),
            Some(AttemptWorktreeFailpoint::AfterCheckpointGit),
        )
        .await
        .expect_err("checkpoint Git failpoint");
    let reopened_store = Store::open(harness.paths.database()).expect("reopen checkpoint Git");
    let reopened_service = AttemptWorktreeService::new(reopened_store, harness.paths.clone());
    let recovered = reopened_service
        .checkpoint(checkpoint.clone(), None)
        .await
        .expect("recover checkpoint Git receipt");
    assert_eq!(recovered.id, checkpoint.checkpoint_id);
    let duplicate = reopened_service
        .checkpoint(checkpoint, None)
        .await
        .expect("exact checkpoint replay");
    assert_eq!(duplicate, recovered);
    let events = harness
        .store
        .load_orchestration_events(&request.run_id)
        .expect("events");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.kind,
                pandamux_core::RunEventKind::CheckpointRecorded { .. }
            ))
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rec_05_through_07_worktree_and_checkpoint_audits_fail_closed() {
    let harness = Harness::new();

    let missing = harness.add_run("recovery-missing-worktree", 80_000);
    let missing_created = harness
        .service
        .create(missing.clone(), None)
        .await
        .expect("missing-worktree fixture");
    let exact = harness
        .service
        .audit(missing_created.record.clone())
        .await
        .expect("exact worktree audit");
    assert!(matches!(
        exact.status,
        AttemptWorktreeAuditStatus::Exact { .. }
    ));
    git(
        &harness.repository,
        &[
            "worktree",
            "remove",
            missing_created.record.worktree.path.as_str(),
        ],
    );
    let audit = harness
        .service
        .audit(missing_created.record)
        .await
        .expect("missing worktree audit");
    assert_eq!(audit.status, AttemptWorktreeAuditStatus::MissingWorktree);

    let changed = harness.add_run("recovery-changed-baseline", 90_000);
    let changed_created = harness
        .service
        .create(changed, None)
        .await
        .expect("changed-baseline fixture");
    let changed_path = Path::new(&changed_created.record.worktree.path);
    let baseline_tree = git_text(changed_path, &["rev-parse", "HEAD^{tree}"]);
    let foreign_head = git_text(
        changed_path,
        &["commit-tree", baseline_tree.as_str(), "-m", "foreign root"],
    );
    git(
        changed_path,
        &[
            "update-ref",
            changed_created.record.worktree.branch_ref.as_str(),
            foreign_head.as_str(),
        ],
    );
    let audit = harness
        .service
        .audit(changed_created.record)
        .await
        .expect("changed baseline audit");
    assert_eq!(audit.status, AttemptWorktreeAuditStatus::ChangedBaseline);

    let missing_checkpoint = harness.add_run("recovery-missing-checkpoint", 100_000);
    harness
        .service
        .create(missing_checkpoint.clone(), None)
        .await
        .expect("missing-checkpoint worktree");
    let checkpoint_id = CheckpointId::new("checkpoint-recovery-missing");
    harness
        .service
        .checkpoint(
            CreateAttemptCheckpointRequest {
                worktree_id: missing_checkpoint.worktree_id.clone(),
                lease_id: missing_checkpoint.lease.id.clone(),
                checkpoint_id: checkpoint_id.clone(),
                kind: CheckpointKind::BeforeAttempt,
                created_at_ms: 100_020,
            },
            Some(AttemptWorktreeFailpoint::AfterCheckpointIntent),
        )
        .await
        .expect_err("checkpoint receipt crash");
    let durable = harness
        .store
        .load_owned_attempt_worktree(&missing_checkpoint.worktree_id)
        .expect("load checkpoint intent")
        .expect("checkpoint intent exists");
    let audit = harness
        .service
        .audit(durable)
        .await
        .expect("missing checkpoint audit");
    assert_eq!(
        audit.status,
        AttemptWorktreeAuditStatus::MissingCheckpoint { checkpoint_id }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_boundaries_reject_early_and_after_and_recovery_receipts_bind_the_attempt() {
    let harness = Harness::new();
    let after = harness.add_run("after-boundary", 165_000);
    harness
        .service
        .create(after.clone(), None)
        .await
        .expect("after worktree");
    let after_checkpoint = CreateAttemptCheckpointRequest {
        worktree_id: after.worktree_id.clone(),
        lease_id: after.lease.id.clone(),
        checkpoint_id: CheckpointId::new("checkpoint-after-boundary"),
        kind: CheckpointKind::AfterAttempt,
        created_at_ms: 165_020,
    };
    let error = harness
        .service
        .checkpoint(after_checkpoint.clone(), None)
        .await
        .expect_err("after checkpoint before terminal rejected");
    assert_eq!(error.reason_code, "attempt_checkpoint_transition_invalid");
    assert!(
        harness
            .store
            .load_owned_attempt_worktree(&after.worktree_id)
            .expect("load worktree")
            .expect("worktree")
            .checkpoint_intent_sha256
            .is_empty()
    );
    harness.settle_attempt(&after, 165_030);
    let mut after_checkpoint = after_checkpoint;
    after_checkpoint.created_at_ms = 165_040;
    let recorded = harness
        .service
        .checkpoint(after_checkpoint, None)
        .await
        .expect("after checkpoint");
    assert_eq!(recorded.kind, CheckpointKind::AfterAttempt);
    assert!(recorded.thread_id.is_some());

    let recovery = harness.add_run("recovery-boundary", 175_000);
    harness
        .service
        .create(recovery.clone(), None)
        .await
        .expect("recovery worktree");
    let recovery_checkpoint = CreateAttemptCheckpointRequest {
        worktree_id: recovery.worktree_id.clone(),
        lease_id: recovery.lease.id.clone(),
        checkpoint_id: CheckpointId::new("checkpoint-recovery-boundary"),
        kind: CheckpointKind::Recovery,
        created_at_ms: 175_020,
    };
    let error = harness
        .service
        .checkpoint(recovery_checkpoint.clone(), None)
        .await
        .expect_err("recovery checkpoint before ambiguity rejected");
    assert_eq!(error.reason_code, "attempt_checkpoint_transition_invalid");
    harness.transition(
        &recovery.run_id,
        RunCommand::MarkAttemptAmbiguous {
            attempt_id: recovery.attempt_id.clone(),
            reason_code: "test_ambiguous_boundary".to_string(),
        },
        175_030,
        "mark-recovery-ambiguous",
    );
    let mut recovery_checkpoint = recovery_checkpoint;
    recovery_checkpoint.created_at_ms = 175_040;
    let recorded = harness
        .service
        .checkpoint(recovery_checkpoint, None)
        .await
        .expect("recovery checkpoint");
    assert_eq!(recorded.kind, CheckpointKind::Recovery);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wt_11_foreign_path_and_ref_are_rejected_without_reuse() {
    let harness = Harness::new();
    let foreign_path = harness.add_run("foreign-path", 70_000);
    let path = harness
        .paths
        .worktrees()
        .join(REPOSITORY_ID)
        .join(foreign_path.run_id.as_str())
        .join(foreign_path.attempt_id.as_str());
    fs::create_dir_all(&path).expect("foreign path");
    fs::write(path.join("owner.txt"), b"foreign\n").expect("foreign marker");
    let error = harness
        .service
        .create(foreign_path.clone(), None)
        .await
        .expect_err("foreign path rejected");
    assert_eq!(error.reason_code, "attempt_worktree_path_foreign");
    assert_eq!(
        fs::read(path.join("owner.txt")).expect("marker"),
        b"foreign\n"
    );
    assert!(
        harness
            .store
            .load_owned_attempt_worktree(&foreign_path.worktree_id)
            .expect("load worktree")
            .is_none()
    );

    let foreign_ref = harness.add_run("foreign-ref", 80_000);
    git(
        &harness.repository,
        &[
            "branch",
            "pandamux/run/run-foreign-ref/task-foreign-ref/1",
            &harness.baseline,
        ],
    );
    let error = harness
        .service
        .create(foreign_ref.clone(), None)
        .await
        .expect_err("foreign ref rejected");
    assert_eq!(error.reason_code, "attempt_worktree_ref_foreign");
    assert!(
        harness
            .store
            .load_owned_attempt_worktree(&foreign_ref.worktree_id)
            .expect("load worktree")
            .is_none()
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wt_12_symlinked_managed_component_and_escaping_baseline_link_are_rejected() {
    use std::os::unix::fs::symlink;

    let harness = Harness::new();
    let request = harness.add_run("managed-link", 90_000);
    let outside = harness._temp.path().join("outside");
    fs::create_dir(&outside).expect("outside directory");
    symlink(&outside, harness.paths.worktrees().join(REPOSITORY_ID)).expect("managed symlink");
    let error = harness
        .service
        .create(request, None)
        .await
        .expect_err("managed symlink rejected");
    assert_eq!(error.reason_code, "attempt_worktree_link_unsafe");
    assert!(
        fs::read_dir(&outside)
            .expect("outside listing")
            .next()
            .is_none()
    );

    let harness = Harness::new();
    symlink("../../outside", harness.repository.join("escape-link"))
        .expect("escaping repository link");
    git(&harness.repository, &["add", "--", "escape-link"]);
    git(&harness.repository, &["commit", "-m", "escaping symlink"]);
    let mut request = harness.add_run("baseline-link", 100_000);
    request.baseline_commit = git_text(&harness.repository, &["rev-parse", "HEAD"]);
    let error = harness
        .service
        .create(request, None)
        .await
        .expect_err("escaping baseline link rejected");
    assert_eq!(error.reason_code, "attempt_worktree_link_unsafe");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wt_13_case_collisions_and_content_filters_fail_closed() {
    let harness = Harness::new();
    fs::write(harness.repository.join("Case.txt"), b"upper\n").expect("upper file");
    fs::write(harness.repository.join("case.txt"), b"lower\n").expect("lower file");
    fs::create_dir(harness.repository.join("Directory")).expect("upper directory");
    fs::create_dir(harness.repository.join("directory")).expect("lower directory");
    fs::write(harness.repository.join("Directory/one.txt"), b"one\n").expect("upper child");
    fs::write(harness.repository.join("directory/two.txt"), b"two\n").expect("lower child");
    git(
        &harness.repository,
        &[
            "add",
            "--",
            "Case.txt",
            "case.txt",
            "Directory/one.txt",
            "directory/two.txt",
        ],
    );
    git(&harness.repository, &["commit", "-m", "case collision"]);
    let mut request = harness.add_run("case-collision", 110_000);
    request.baseline_commit = git_text(&harness.repository, &["rev-parse", "HEAD"]);
    let error = harness
        .service
        .create(request, None)
        .await
        .expect_err("case collision rejected");
    assert_eq!(error.reason_code, "attempt_tree_case_collision");

    let harness = Harness::new();
    fs::write(
        harness.repository.join(".gitattributes"),
        b"*.dat filter=unsafe\n",
    )
    .expect("attributes");
    fs::write(harness.repository.join("payload.dat"), b"payload\n").expect("payload");
    git(
        &harness.repository,
        &["add", "--", ".gitattributes", "payload.dat"],
    );
    git(&harness.repository, &["commit", "-m", "unsafe filter"]);
    let mut request = harness.add_run("content-filter", 120_000);
    request.baseline_commit = git_text(&harness.repository, &["rev-parse", "HEAD"]);
    let error = harness
        .service
        .create(request, None)
        .await
        .expect_err("content filter rejected");
    assert_eq!(error.reason_code, "attempt_repository_filter_unsafe");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wt_09_dirty_and_ambiguous_worktrees_are_retained() {
    let harness = Harness::new();
    let dirty = harness.add_run("dirty-retention", 130_000);
    let dirty_outcome = harness
        .service
        .create(dirty.clone(), None)
        .await
        .expect("dirty worktree");
    harness
        .service
        .checkpoint(
            CreateAttemptCheckpointRequest {
                worktree_id: dirty.worktree_id.clone(),
                lease_id: dirty.lease.id.clone(),
                checkpoint_id: CheckpointId::new("checkpoint-dirty-retention"),
                kind: CheckpointKind::BeforeAttempt,
                created_at_ms: 130_020,
            },
            None,
        )
        .await
        .expect("dirty checkpoint");
    harness.settle_attempt(&dirty, 130_030);
    fs::write(
        Path::new(&dirty_outcome.record.worktree.path).join("tracked.txt"),
        b"attempt output\n",
    )
    .expect("dirty attempt output");
    let outcome = harness
        .service
        .cleanup(
            CleanupAttemptWorktreeRequest {
                worktree_id: dirty.worktree_id.clone(),
                lease_id: dirty.lease.id.clone(),
                verified_process_death_sha256: "c".repeat(64),
                retained_evidence_sha256: "d".repeat(64),
                cleaned_at_ms: 130_040,
            },
            None,
        )
        .await
        .expect("dirty cleanup classification");
    let AttemptWorktreeCleanupOutcome::Retained(record) = outcome else {
        panic!("dirty worktree must be retained");
    };
    assert_eq!(record.worktree.dirty_state, WorktreeDirtyState::Dirty);
    assert_eq!(
        record.worktree.cleanup_eligibility,
        WorktreeCleanupEligibility::Retained
    );
    assert!(Path::new(&record.worktree.path).is_dir());

    let ambiguous = harness.add_run("ambiguous-retention", 140_000);
    let ambiguous_outcome = harness
        .service
        .create(ambiguous.clone(), None)
        .await
        .expect("ambiguous worktree");
    harness
        .service
        .checkpoint(
            CreateAttemptCheckpointRequest {
                worktree_id: ambiguous.worktree_id.clone(),
                lease_id: ambiguous.lease.id.clone(),
                checkpoint_id: CheckpointId::new("checkpoint-ambiguous-retention"),
                kind: CheckpointKind::BeforeAttempt,
                created_at_ms: 140_020,
            },
            None,
        )
        .await
        .expect("ambiguous checkpoint");
    harness.settle_attempt(&ambiguous, 140_030);
    fs::write(
        Path::new(&ambiguous_outcome.record.worktree.path).join(".git"),
        b"gitdir: /foreign/not-owned\n",
    )
    .expect("foreign Git pointer");
    let outcome = harness
        .service
        .cleanup(
            CleanupAttemptWorktreeRequest {
                worktree_id: ambiguous.worktree_id.clone(),
                lease_id: ambiguous.lease.id.clone(),
                verified_process_death_sha256: "e".repeat(64),
                retained_evidence_sha256: "f".repeat(64),
                cleaned_at_ms: 140_040,
            },
            None,
        )
        .await
        .expect("ambiguous cleanup classification");
    let AttemptWorktreeCleanupOutcome::Retained(record) = outcome else {
        panic!("ambiguous worktree must be retained");
    };
    assert_eq!(record.worktree.dirty_state, WorktreeDirtyState::Ambiguous);
    assert!(Path::new(&record.worktree.path).is_dir());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wt_10_locked_trees_are_retained_and_clean_removal_is_non_forced_and_replayable() {
    let harness = Harness::new();
    let locked = harness.add_run("locked-retention", 150_000);
    let locked_outcome = harness
        .service
        .create(locked.clone(), None)
        .await
        .expect("locked worktree");
    harness
        .service
        .checkpoint(
            CreateAttemptCheckpointRequest {
                worktree_id: locked.worktree_id.clone(),
                lease_id: locked.lease.id.clone(),
                checkpoint_id: CheckpointId::new("checkpoint-locked-retention"),
                kind: CheckpointKind::BeforeAttempt,
                created_at_ms: 150_020,
            },
            None,
        )
        .await
        .expect("locked checkpoint");
    harness.settle_attempt(&locked, 150_030);
    git(
        &harness.repository,
        &["worktree", "lock", &locked_outcome.record.worktree.path],
    );
    let outcome = harness
        .service
        .cleanup(
            CleanupAttemptWorktreeRequest {
                worktree_id: locked.worktree_id.clone(),
                lease_id: locked.lease.id.clone(),
                verified_process_death_sha256: "1".repeat(64),
                retained_evidence_sha256: "2".repeat(64),
                cleaned_at_ms: 150_040,
            },
            None,
        )
        .await
        .expect("locked cleanup classification");
    let AttemptWorktreeCleanupOutcome::Retained(record) = outcome else {
        panic!("locked worktree must be retained");
    };
    assert_eq!(record.worktree.dirty_state, WorktreeDirtyState::Locked);
    assert!(Path::new(&record.worktree.path).is_dir());

    let clean = harness.add_run("clean-removal", 160_000);
    let clean_outcome = harness
        .service
        .create(clean.clone(), None)
        .await
        .expect("clean worktree");
    harness
        .service
        .checkpoint(
            CreateAttemptCheckpointRequest {
                worktree_id: clean.worktree_id.clone(),
                lease_id: clean.lease.id.clone(),
                checkpoint_id: CheckpointId::new("checkpoint-clean-removal"),
                kind: CheckpointKind::BeforeAttempt,
                created_at_ms: 160_020,
            },
            None,
        )
        .await
        .expect("clean checkpoint");
    harness.settle_attempt(&clean, 160_030);
    let cleanup = CleanupAttemptWorktreeRequest {
        worktree_id: clean.worktree_id.clone(),
        lease_id: clean.lease.id.clone(),
        verified_process_death_sha256: "3".repeat(64),
        retained_evidence_sha256: "4".repeat(64),
        cleaned_at_ms: 160_040,
    };
    harness
        .service
        .cleanup(
            cleanup.clone(),
            Some(AttemptWorktreeFailpoint::AfterCleanupGit),
        )
        .await
        .expect_err("cleanup receipt crash");
    assert!(!Path::new(&clean_outcome.record.worktree.path).exists());
    let reopened = AttemptWorktreeService::new(
        Store::open(harness.paths.database()).expect("reopen cleanup receipt"),
        harness.paths.clone(),
    );
    let outcome = reopened
        .cleanup(cleanup.clone(), None)
        .await
        .expect("cleanup receipt recovery");
    let AttemptWorktreeCleanupOutcome::Removed(record) = outcome else {
        panic!("clean worktree should be removed");
    };
    assert_eq!(
        record.worktree.cleanup_eligibility,
        WorktreeCleanupEligibility::Eligible
    );
    assert_eq!(
        record.verified_process_death_sha256.as_deref(),
        Some(cleanup.verified_process_death_sha256.as_str())
    );
    assert_eq!(
        record.retained_evidence_sha256.as_deref(),
        Some(cleanup.retained_evidence_sha256.as_str())
    );
    assert_eq!(
        git_text(
            &harness.repository,
            &[
                "show-ref",
                "--verify",
                "--hash",
                &record.worktree.branch_ref
            ]
        ),
        harness.baseline
    );
}
