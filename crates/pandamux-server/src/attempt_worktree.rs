use crate::data_paths::DataPaths;
use crate::store::{
    AssignedRunEvent, OrchestrationRunRecord, OwnedAttemptWorktreeRecord, RunTransitionRequest,
    RunTransitionRows, Store, StoreError, orchestration_snapshot_sha256,
};
use pandamux_core::{
    AttemptStatus, AttemptWorktreeId, CheckpointId, CheckpointKind, CheckpointRef, EnvironmentId,
    ExecutionLease, ExecutionLeaseId, LeaseResourceKind, OwnedAttemptWorktree, ProjectRepositoryId,
    RunAttemptId, RunCommand, RunEventId, RunId, RunTaskId, WorktreeCleanupEligibility,
    WorktreeDirtyState,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

const DEFAULT_BLOCKING_GIT_LIMIT: usize = 4;
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_GIT_STDOUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_GIT_STDERR_BYTES: usize = 16 * 1024;
const MAX_ATTRIBUTE_BYTES: usize = 1024 * 1024;
const MAX_MANIFEST_ENTRIES: usize = 1_000_000;
const ZERO_OBJECT_ID_40: &str = "0000000000000000000000000000000000000000";
const ZERO_OBJECT_ID_64: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptWorktreeFailpoint {
    AfterOwnershipIntent,
    BeforeWorktreeGit,
    AfterWorktreeGit,
    AfterCheckpointIntent,
    BeforeCheckpointGit,
    AfterCheckpointGit,
    AfterCleanupGit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptWorktreeError {
    pub reason_code: &'static str,
    pub safe_message: String,
}

impl std::fmt::Display for AttemptWorktreeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.safe_message)
    }
}

impl std::error::Error for AttemptWorktreeError {}

impl From<StoreError> for AttemptWorktreeError {
    fn from(_: StoreError) -> Self {
        worktree_error(
            "attempt_worktree_store_rejected",
            "The durable attempt worktree operation was rejected",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateAttemptWorktreeRequest {
    pub run_id: RunId,
    pub task_id: RunTaskId,
    pub attempt_id: RunAttemptId,
    pub worktree_id: AttemptWorktreeId,
    pub repository_id: ProjectRepositoryId,
    pub environment_id: EnvironmentId,
    pub repository_root: PathBuf,
    pub baseline_commit: String,
    pub lease: ExecutionLease,
    pub intent_at_ms: u64,
    pub materialized_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptWorktreeOutcome {
    pub record: OwnedAttemptWorktreeRecord,
    pub git_materialized: bool,
    pub recovered: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateAttemptCheckpointRequest {
    pub worktree_id: AttemptWorktreeId,
    pub lease_id: ExecutionLeaseId,
    pub checkpoint_id: CheckpointId,
    pub kind: CheckpointKind,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupAttemptWorktreeRequest {
    pub worktree_id: AttemptWorktreeId,
    pub lease_id: ExecutionLeaseId,
    pub verified_process_death_sha256: String,
    pub retained_evidence_sha256: String,
    pub cleaned_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptWorktreeCleanupOutcome {
    Removed(OwnedAttemptWorktreeRecord),
    Retained(OwnedAttemptWorktreeRecord),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptWorktreeAuditStatus {
    Exact { head: String },
    MissingWorktree,
    ChangedBaseline,
    MissingCheckpoint { checkpoint_id: CheckpointId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptWorktreeAudit {
    pub worktree_id: AttemptWorktreeId,
    pub status: AttemptWorktreeAuditStatus,
}

#[derive(Clone)]
pub struct AttemptWorktreeService {
    store: Store,
    data_paths: DataPaths,
    blocking: Arc<Semaphore>,
    // Checkpoint events carry the exact checkpoint timestamp. FIFO admission
    // prevents two local checkpoint workers from committing those timestamps
    // in reverse order while Git runs outside the SQLite transaction.
    checkpoint_serial: Arc<Semaphore>,
    git_program: PathBuf,
    git_timeout: Duration,
}

impl AttemptWorktreeService {
    pub fn new(store: Store, data_paths: DataPaths) -> Self {
        Self {
            store,
            data_paths,
            blocking: Arc::new(Semaphore::new(DEFAULT_BLOCKING_GIT_LIMIT)),
            checkpoint_serial: Arc::new(Semaphore::new(1)),
            git_program: PathBuf::from("git"),
            git_timeout: GIT_TIMEOUT,
        }
    }

    pub async fn create(
        &self,
        request: CreateAttemptWorktreeRequest,
        failpoint: Option<AttemptWorktreeFailpoint>,
    ) -> Result<AttemptWorktreeOutcome, AttemptWorktreeError> {
        validate_failpoint(
            failpoint,
            &[
                AttemptWorktreeFailpoint::AfterOwnershipIntent,
                AttemptWorktreeFailpoint::BeforeWorktreeGit,
                AttemptWorktreeFailpoint::AfterWorktreeGit,
            ],
        )?;
        let permit = self
            .blocking
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| cancelled_error())?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            service.create_sync(request, failpoint)
        })
        .await
        .map_err(|_| worker_error())?
    }

    pub async fn checkpoint(
        &self,
        request: CreateAttemptCheckpointRequest,
        failpoint: Option<AttemptWorktreeFailpoint>,
    ) -> Result<CheckpointRef, AttemptWorktreeError> {
        validate_failpoint(
            failpoint,
            &[
                AttemptWorktreeFailpoint::AfterCheckpointIntent,
                AttemptWorktreeFailpoint::BeforeCheckpointGit,
                AttemptWorktreeFailpoint::AfterCheckpointGit,
            ],
        )?;
        let serial = self
            .checkpoint_serial
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| cancelled_error())?;
        let permit = self
            .blocking
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| cancelled_error())?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _serial = serial;
            let _permit = permit;
            service.checkpoint_sync(request, failpoint)
        })
        .await
        .map_err(|_| worker_error())?
    }

    pub async fn cleanup(
        &self,
        request: CleanupAttemptWorktreeRequest,
        failpoint: Option<AttemptWorktreeFailpoint>,
    ) -> Result<AttemptWorktreeCleanupOutcome, AttemptWorktreeError> {
        validate_failpoint(failpoint, &[AttemptWorktreeFailpoint::AfterCleanupGit])?;
        let permit = self
            .blocking
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| cancelled_error())?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            service.cleanup_sync(request, failpoint)
        })
        .await
        .map_err(|_| worker_error())?
    }

    /// Read-only recovery audit for a durable worktree owner. It verifies path
    /// confinement, repository identity, baseline ancestry, branch ownership,
    /// and every durable checkpoint ref without mutating Git state.
    pub async fn audit(
        &self,
        record: OwnedAttemptWorktreeRecord,
    ) -> Result<AttemptWorktreeAudit, AttemptWorktreeError> {
        let permit = self
            .blocking
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| cancelled_error())?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            service.audit_sync(record)
        })
        .await
        .map_err(|_| worker_error())?
    }
}

impl AttemptWorktreeService {
    fn audit_sync(
        &self,
        record: OwnedAttemptWorktreeRecord,
    ) -> Result<AttemptWorktreeAudit, AttemptWorktreeError> {
        self.data_paths
            .ensure_private_roots()
            .map_err(|_| managed_path_error())?;
        let identity = identity_from_record(&self.data_paths, &record)?;
        if fs::symlink_metadata(&identity.path).is_err() {
            return Ok(AttemptWorktreeAudit {
                worktree_id: record.worktree.id,
                status: AttemptWorktreeAuditStatus::MissingWorktree,
            });
        }
        let run = self.load_run(&record.worktree.run_id)?;
        let repository = self
            .store
            .get_project_repository(&run.project_id, &run.run.organization_id)?
            .ok_or_else(owner_mismatch_error)?;
        let context = match self.inspect_repository(
            Path::new(&repository.display_path),
            &record.worktree.git_common_dir_identity,
            &record.worktree.baseline_commit,
        ) {
            Ok(context) => context,
            Err(_) => {
                return Ok(AttemptWorktreeAudit {
                    worktree_id: record.worktree.id,
                    status: AttemptWorktreeAuditStatus::ChangedBaseline,
                });
            }
        };
        let head = match self.verify_owned_worktree(
            &context,
            &identity,
            &record.worktree.baseline_commit,
            false,
        ) {
            Ok(head) => head,
            Err(_) => {
                return Ok(AttemptWorktreeAudit {
                    worktree_id: record.worktree.id,
                    status: AttemptWorktreeAuditStatus::ChangedBaseline,
                });
            }
        };
        for checkpoint_id in &record.worktree.checkpoint_ids {
            let Some(checkpoint) = run.run.checkpoints.get(checkpoint_id) else {
                return Ok(AttemptWorktreeAudit {
                    worktree_id: record.worktree.id,
                    status: AttemptWorktreeAuditStatus::MissingCheckpoint {
                        checkpoint_id: checkpoint_id.clone(),
                    },
                });
            };
            if checkpoint.run_id != record.worktree.run_id
                || checkpoint.task_id != record.worktree.task_id
                || checkpoint.attempt_id != record.worktree.attempt_id
                || !record.checkpoint_intent_sha256.contains_key(checkpoint_id)
                || self.read_ref(&context, &checkpoint.value)?.is_none()
            {
                return Ok(AttemptWorktreeAudit {
                    worktree_id: record.worktree.id,
                    status: AttemptWorktreeAuditStatus::MissingCheckpoint {
                        checkpoint_id: checkpoint_id.clone(),
                    },
                });
            }
        }
        Ok(AttemptWorktreeAudit {
            worktree_id: record.worktree.id,
            status: AttemptWorktreeAuditStatus::Exact { head },
        })
    }

    fn materialize_checkpoint(
        &self,
        context: &RepositoryContext,
        identity: &DerivedAttemptIdentity,
        checkpoint: &CheckpointRef,
        checkpoint_ref: &str,
        parent: &str,
    ) -> Result<String, AttemptWorktreeError> {
        self.reject_worktree_filters(context, &identity.path)?;
        let index_root = self
            .data_paths
            .recovery()
            .join("checkpoint-indices")
            .join(checkpoint.attempt_id.as_str())
            .join(checkpoint.id.as_str());
        ensure_managed_directory(&self.data_paths, &index_root)?;
        let index_path = index_root.join(format!("{}.index", uuid::Uuid::new_v4().simple()));
        ensure_path_absent(&index_path)?;
        let index_environment = vec![(
            OsString::from("GIT_INDEX_FILE"),
            crate::managed_git::git_process_path(&index_path).into_owned(),
        )];
        let result = (|| {
            let mut read_tree = GitInvocation::new(
                &identity.path,
                [OsString::from("read-tree"), OsString::from(parent)],
            );
            read_tree.environment.clone_from(&index_environment);
            self.run_git(context, read_tree)?;

            let mut add = GitInvocation::new(
                &identity.path,
                [
                    OsString::from("add"),
                    OsString::from("--all"),
                    OsString::from("--"),
                    OsString::from("."),
                ],
            );
            add.environment.clone_from(&index_environment);
            self.run_git(context, add)?;

            let mut write_tree = GitInvocation::new(&identity.path, [OsString::from("write-tree")]);
            write_tree.environment.clone_from(&index_environment);
            let tree = single_line(&self.run_git(context, write_tree)?.stdout)?.to_string();
            if !valid_object_id(&tree) {
                return Err(worktree_error(
                    "attempt_checkpoint_tree_invalid",
                    "The attempt checkpoint tree identity is invalid",
                ));
            }

            let message = format!(
                "PandaMUX {:?} checkpoint {}\n",
                checkpoint.kind,
                checkpoint.id.as_str()
            );
            let mut commit = GitInvocation::new(
                &identity.path,
                [
                    OsString::from("commit-tree"),
                    OsString::from(&tree),
                    OsString::from("-p"),
                    OsString::from(parent),
                    OsString::from("-F"),
                    OsString::from("-"),
                ],
            );
            commit.stdin = message.into_bytes();
            let timestamp = checkpoint.created_at_ms / 1000;
            commit.environment.extend([
                (
                    OsString::from("GIT_AUTHOR_NAME"),
                    OsString::from("PandaMUX"),
                ),
                (
                    OsString::from("GIT_AUTHOR_EMAIL"),
                    OsString::from("pandamux@localhost"),
                ),
                (
                    OsString::from("GIT_COMMITTER_NAME"),
                    OsString::from("PandaMUX"),
                ),
                (
                    OsString::from("GIT_COMMITTER_EMAIL"),
                    OsString::from("pandamux@localhost"),
                ),
                (
                    OsString::from("GIT_AUTHOR_DATE"),
                    OsString::from(format!("@{timestamp} +0000")),
                ),
                (
                    OsString::from("GIT_COMMITTER_DATE"),
                    OsString::from(format!("@{timestamp} +0000")),
                ),
            ]);
            let commit_id = single_line(&self.run_git(context, commit)?.stdout)?.to_string();
            if !valid_object_id(&commit_id) {
                return Err(worktree_error(
                    "attempt_checkpoint_commit_invalid",
                    "The attempt checkpoint commit identity is invalid",
                ));
            }
            match self.read_ref(context, checkpoint_ref)? {
                Some(existing) if existing.eq_ignore_ascii_case(&commit_id) => {}
                Some(_) => return Err(changed_replay_error()),
                None => {
                    let zero = if commit_id.len() == 64 {
                        ZERO_OBJECT_ID_64
                    } else {
                        ZERO_OBJECT_ID_40
                    };
                    self.run_git(
                        context,
                        GitInvocation::new(
                            &identity.path,
                            [
                                OsString::from("update-ref"),
                                OsString::from("--no-create-reflog"),
                                OsString::from(checkpoint_ref),
                                OsString::from(&commit_id),
                                OsString::from(zero),
                            ],
                        ),
                    )?;
                }
            }
            Ok(commit_id)
        })();
        let _ = fs::remove_file(&index_path);
        result
    }

    fn reject_worktree_filters(
        &self,
        context: &RepositoryContext,
        worktree: &Path,
    ) -> Result<(), AttemptWorktreeError> {
        let mut stack = vec![worktree.to_path_buf()];
        let mut inspected = 0usize;
        while let Some(directory) = stack.pop() {
            let entries = fs::read_dir(&directory).map_err(|_| owner_mismatch_error())?;
            for entry in entries {
                let entry = entry.map_err(|_| owner_mismatch_error())?;
                let path = entry.path();
                if directory == worktree && entry.file_name() == OsStr::new(".git") {
                    continue;
                }
                let metadata = fs::symlink_metadata(&path).map_err(|_| owner_mismatch_error())?;
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && !is_reparse_point(&metadata)
                {
                    stack.push(path);
                } else if metadata.is_file() && entry.file_name() == OsStr::new(".gitattributes") {
                    inspected = inspected
                        .checked_add(metadata.len() as usize)
                        .ok_or_else(filter_error)?;
                    if inspected > MAX_ATTRIBUTE_BYTES {
                        return Err(filter_error());
                    }
                    reject_filter_attributes(&fs::read(path).map_err(|_| filter_error())?)?;
                }
            }
        }
        let _ = context;
        Ok(())
    }

    fn run_git(
        &self,
        context: &RepositoryContext,
        invocation: GitInvocation,
    ) -> Result<GitOutput, AttemptWorktreeError> {
        let arguments = invocation
            .args
            .iter()
            .map(|argument| crate::managed_git::git_process_path(Path::new(argument)).into_owned())
            .collect::<Vec<_>>();
        let global_config =
            crate::managed_git::git_process_path(&context.empty_global_config).into_owned();
        let mut command = Command::new(&self.git_program);
        sanitize_git_environment(&mut command);
        command
            .current_dir(&invocation.cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", global_config)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .arg("-c")
            .arg(format!(
                "core.hooksPath={}",
                crate::managed_git::git_process_path_string(&context.hooks_dir)
            ))
            .arg("-c")
            .arg(format!(
                "core.attributesFile={}",
                crate::managed_git::git_process_path_string(&context.empty_attributes)
            ))
            .arg("-c")
            .arg("submodule.recurse=false")
            .arg("-c")
            .arg("fetch.recurseSubmodules=false")
            .arg("-c")
            .arg("commit.gpgsign=false")
            .arg("-c")
            .arg("core.autocrlf=false")
            .arg("-c")
            .arg("core.longpaths=true")
            .arg("-c")
            .arg("gc.auto=0")
            .arg("-c")
            .arg("maintenance.auto=false")
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in &invocation.environment {
            command.env(name, value);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().map_err(|_| {
            worktree_error(
                "attempt_git_start_failed",
                "The bounded attempt Git command could not be started",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(git_start_error)?;
        let stderr = child.stderr.take().ok_or_else(git_start_error)?;
        let stdout_reader = spawn_bounded_reader(stdout, invocation.stdout_limit);
        let stderr_reader = spawn_bounded_reader(stderr, MAX_GIT_STDERR_BYTES);
        if let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(&invocation.stdin).is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(worktree_error(
                "attempt_git_input_failed",
                "The bounded attempt Git command rejected its input",
            ));
        }
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < self.git_timeout => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(worktree_error(
                        "attempt_git_timed_out",
                        "The bounded attempt Git command exceeded its time limit",
                    ));
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(worktree_error(
                        "attempt_git_wait_failed",
                        "The bounded attempt Git command could not be joined",
                    ));
                }
            }
        };
        let stdout = join_bounded_reader(stdout_reader)?;
        let stderr = join_bounded_reader(stderr_reader)?;
        if stdout.truncated || stderr.truncated {
            return Err(worktree_error(
                "attempt_git_output_too_large",
                "The bounded attempt Git command produced too much output",
            ));
        }
        let exit_code = status.code().unwrap_or(-1);
        if !invocation.accepted_exit_codes.contains(&exit_code) {
            let operation = invocation
                .args
                .first()
                .and_then(|value| value.to_str())
                .unwrap_or("unknown");
            return Err(worktree_error(
                "attempt_git_failed",
                &format!(
                    "The bounded attempt Git {operation} command failed with status {exit_code}"
                ),
            ));
        }
        Ok(GitOutput {
            stdout: stdout.bytes,
            exit_code,
        })
    }
}

#[derive(Debug)]
struct BoundedRead {
    bytes: Vec<u8>,
    truncated: bool,
}

fn spawn_bounded_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> thread::JoinHandle<std::io::Result<BoundedRead>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut truncated = false;
        let mut buffer = [0u8; 8192];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let remaining = limit.saturating_sub(bytes.len());
            let retained = read.min(remaining);
            bytes.extend_from_slice(&buffer[..retained]);
            truncated |= retained != read;
        }
        Ok(BoundedRead { bytes, truncated })
    })
}

fn join_bounded_reader(
    reader: thread::JoinHandle<std::io::Result<BoundedRead>>,
) -> Result<BoundedRead, AttemptWorktreeError> {
    reader
        .join()
        .map_err(|_| git_start_error())?
        .map_err(|_| git_start_error())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TreeEntry {
    mode: String,
    object_id: String,
    path: String,
}

fn parse_tree_entries(bytes: &[u8]) -> Result<Vec<TreeEntry>, AttemptWorktreeError> {
    let mut entries = Vec::new();
    for raw in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let tab = raw.iter().position(|byte| *byte == b'\t').ok_or_else(|| {
            worktree_error(
                "attempt_tree_invalid",
                "The attempt baseline tree could not be parsed",
            )
        })?;
        let header = std::str::from_utf8(&raw[..tab]).map_err(|_| {
            worktree_error(
                "attempt_tree_invalid",
                "The attempt baseline tree could not be parsed",
            )
        })?;
        let mut fields = header.split(' ');
        let mode = fields.next().unwrap_or_default();
        let kind = fields.next().unwrap_or_default();
        let object_id = fields.next().unwrap_or_default();
        if fields.next().is_some()
            || !valid_object_id(object_id)
            || !matches!(kind, "blob" | "commit")
        {
            return Err(worktree_error(
                "attempt_tree_invalid",
                "The attempt baseline tree could not be parsed",
            ));
        }
        let path = std::str::from_utf8(&raw[tab + 1..]).map_err(|_| {
            worktree_error(
                "attempt_tree_path_invalid",
                "The attempt baseline contains a non-portable path",
            )
        })?;
        entries.push(TreeEntry {
            mode: mode.to_string(),
            object_id: object_id.to_ascii_lowercase(),
            path: path.to_string(),
        });
    }
    Ok(entries)
}

fn validate_create_request(
    request: &CreateAttemptWorktreeRequest,
) -> Result<(), AttemptWorktreeError> {
    for value in [
        request.run_id.as_str(),
        request.task_id.as_str(),
        request.attempt_id.as_str(),
        request.worktree_id.as_str(),
        request.repository_id.as_str(),
        request.environment_id.as_str(),
    ] {
        if !safe_component(value) {
            return Err(invalid_identity_error());
        }
    }
    if !valid_object_id(&request.baseline_commit)
        || request.lease.id.as_str().trim().is_empty()
        || request.lease.resource_kind != LeaseResourceKind::Worktree
        || request.lease.resource_id != request.worktree_id.as_str()
        || request
            .lease
            .owner_incarnation_id
            .as_str()
            .trim()
            .is_empty()
        || request.lease.revision == 0
        || request.lease.last_heartbeat_ms > request.intent_at_ms
        || request.intent_at_ms > request.materialized_at_ms
        || request.materialized_at_ms >= request.lease.expires_at_ms
    {
        return Err(worktree_error(
            "attempt_worktree_request_invalid",
            "The attempt worktree request is invalid",
        ));
    }
    Ok(())
}

fn derive_attempt_identity(
    data_paths: &DataPaths,
    request: &CreateAttemptWorktreeRequest,
    ordinal: u32,
) -> Result<DerivedAttemptIdentity, AttemptWorktreeError> {
    if ordinal == 0 {
        return Err(invalid_identity_error());
    }
    let branch_name = format!(
        "pandamux/run/{}/{}/{}",
        request.run_id.as_str(),
        request.task_id.as_str(),
        ordinal
    );
    let branch_ref = format!("refs/heads/{branch_name}");
    if branch_ref.len() > 512 {
        return Err(invalid_identity_error());
    }
    let path = data_paths
        .worktrees()
        .join(request.repository_id.as_str())
        .join(request.run_id.as_str())
        .join(request.attempt_id.as_str());
    if !path.starts_with(data_paths.worktrees()) {
        return Err(managed_path_error());
    }
    Ok(DerivedAttemptIdentity {
        branch_ref,
        branch_name,
        path,
    })
}

fn identity_from_record(
    data_paths: &DataPaths,
    record: &OwnedAttemptWorktreeRecord,
) -> Result<DerivedAttemptIdentity, AttemptWorktreeError> {
    let ordinal = record
        .worktree
        .branch_ref
        .rsplit('/')
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|ordinal| *ordinal > 0)
        .ok_or_else(owner_mismatch_error)?;
    let request = CreateAttemptWorktreeRequest {
        run_id: record.worktree.run_id.clone(),
        task_id: record.worktree.task_id.clone(),
        attempt_id: record.worktree.attempt_id.clone(),
        worktree_id: record.worktree.id.clone(),
        repository_id: record.worktree.repository_id.clone(),
        environment_id: record.worktree.environment_id.clone(),
        repository_root: PathBuf::new(),
        baseline_commit: record.worktree.baseline_commit.clone(),
        lease: ExecutionLease {
            id: record.worktree.lease_id.clone(),
            resource_kind: LeaseResourceKind::Worktree,
            resource_id: record.worktree.id.as_str().to_string(),
            owner_incarnation_id: pandamux_core::ServerIncarnationId::new("derived-only"),
            expires_at_ms: 1,
            last_heartbeat_ms: 0,
            revision: 1,
        },
        intent_at_ms: 0,
        materialized_at_ms: 0,
    };
    let identity = derive_attempt_identity(data_paths, &request, ordinal)?;
    if path_text(&identity.path)? != record.worktree.path
        || identity.branch_ref != record.worktree.branch_ref
    {
        return Err(owner_mismatch_error());
    }
    Ok(identity)
}

fn validate_existing_intent(
    existing: &OwnedAttemptWorktreeRecord,
    expected: &OwnedAttemptWorktreeRecord,
    lease: &ExecutionLease,
) -> Result<(), AttemptWorktreeError> {
    let mut normalized = existing.clone();
    normalized.worktree.dirty_state = WorktreeDirtyState::Ambiguous;
    normalized.worktree.cleanup_eligibility = WorktreeCleanupEligibility::Ineligible;
    normalized.worktree.checkpoint_ids.clear();
    normalized.checkpoint_intent_sha256.clear();
    normalized.verified_process_death_sha256 = None;
    normalized.retained_evidence_sha256 = None;
    normalized.updated_at_ms = normalized.created_at_ms;
    if &normalized != expected || existing.worktree.lease_id != lease.id {
        return Err(changed_replay_error());
    }
    Ok(())
}

fn checkpoint_ref(
    record: &OwnedAttemptWorktreeRecord,
    request: &CreateAttemptCheckpointRequest,
) -> Result<String, AttemptWorktreeError> {
    let kind = match request.kind {
        CheckpointKind::BeforeAttempt => "before",
        CheckpointKind::AfterAttempt => "after",
        CheckpointKind::Recovery => "recovery",
    };
    let reference = format!(
        "refs/pandamux/checkpoints/{}/{}/{}/{}",
        record.worktree.run_id.as_str(),
        record.worktree.attempt_id.as_str(),
        kind,
        request.checkpoint_id.as_str()
    );
    if reference.len() > 256 {
        return Err(invalid_identity_error());
    }
    Ok(reference)
}

fn checkpoint_payload_sha256(
    record: &OwnedAttemptWorktreeRecord,
    checkpoint: &CheckpointRef,
    head: &str,
    manifest_sha256: &str,
) -> String {
    let mut hasher = Sha256::new();
    for field in [
        record.worktree.id.as_str(),
        record.worktree.run_id.as_str(),
        record.worktree.task_id.as_str(),
        record.worktree.attempt_id.as_str(),
        checkpoint.id.as_str(),
        checkpoint.value.as_str(),
        match checkpoint.kind {
            CheckpointKind::BeforeAttempt => "before_attempt",
            CheckpointKind::AfterAttempt => "after_attempt",
            CheckpointKind::Recovery => "recovery",
        },
        head,
        manifest_sha256,
    ] {
        hash_field(&mut hasher, field.as_bytes());
    }
    hash_field(&mut hasher, &checkpoint.created_at_ms.to_be_bytes());
    hash_field(
        &mut hasher,
        checkpoint
            .thread_id
            .as_ref()
            .map(|id| id.as_str().as_bytes())
            .unwrap_or_default(),
    );
    format!("{:x}", hasher.finalize())
}

fn validate_portable_tree_path(path: &str) -> Result<(), AttemptWorktreeError> {
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return Err(nonportable_path_error());
    }
    for component in path.split('/') {
        let trimmed = component.trim_end_matches([' ', '.']);
        if component.is_empty()
            || matches!(component, "." | "..")
            || trimmed != component
            || component.contains(['<', '>', ':', '"', '|', '?', '*'])
            || windows_reserved_component(component)
        {
            return Err(nonportable_path_error());
        }
    }
    Ok(())
}

fn windows_reserved_component(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .and_then(|suffix| suffix.parse::<u8>().ok())
            .is_some_and(|number| (1..=9).contains(&number))
        || stem
            .strip_prefix("LPT")
            .and_then(|suffix| suffix.parse::<u8>().ok())
            .is_some_and(|number| (1..=9).contains(&number))
}

fn insert_casefold_path(folded: &mut BTreeMap<String, String>, path: &str) -> bool {
    let mut original_prefix = String::new();
    for component in path.split('/') {
        if !original_prefix.is_empty() {
            original_prefix.push('/');
        }
        original_prefix.push_str(component);
        let key = original_prefix.to_lowercase();
        if folded
            .insert(key, original_prefix.clone())
            .is_some_and(|existing| existing != original_prefix)
        {
            return true;
        }
    }
    false
}

fn validate_repository_symlink(path: &str, target: &[u8]) -> Result<(), AttemptWorktreeError> {
    let target = std::str::from_utf8(target).map_err(|_| unsafe_link_error())?;
    if target.is_empty()
        || target.contains('\0')
        || target.starts_with(['/', '\\'])
        || target.as_bytes().get(1) == Some(&b':')
    {
        return Err(unsafe_link_error());
    }
    let mut depth = path.split('/').count().saturating_sub(1);
    for component in target.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." if depth == 0 => return Err(unsafe_link_error()),
            ".." => depth -= 1,
            _ => depth = depth.saturating_add(1),
        }
    }
    Ok(())
}

fn reject_filter_attributes(bytes: &[u8]) -> Result<(), AttemptWorktreeError> {
    let text = std::str::from_utf8(bytes).map_err(|_| filter_error())?;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        for attribute in line.split_ascii_whitespace().skip(1) {
            if attribute == "filter" || attribute.starts_with("filter=") {
                return Err(filter_error());
            }
        }
    }
    Ok(())
}

fn worktree_manifest_sha256(root: &Path) -> Result<String, AttemptWorktreeError> {
    validate_worktree_contents(root)?;
    let mut paths = Vec::new();
    collect_manifest_paths(root, root, &mut paths)?;
    paths.sort();
    if paths.len() > MAX_MANIFEST_ENTRIES {
        return Err(worktree_error(
            "attempt_manifest_too_large",
            "The attempt worktree has too many entries to checkpoint safely",
        ));
    }
    let mut hasher = Sha256::new();
    for path in paths {
        let relative = path.strip_prefix(root).map_err(|_| managed_path_error())?;
        let relative = relative.to_str().ok_or_else(nonportable_path_error)?;
        let metadata = fs::symlink_metadata(&path).map_err(|_| owner_mismatch_error())?;
        hash_field(&mut hasher, relative.as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            hash_field(&mut hasher, &metadata.mode().to_be_bytes());
        }
        if metadata.file_type().is_symlink() {
            hash_field(&mut hasher, b"symlink");
            let target = fs::read_link(&path).map_err(|_| unsafe_link_error())?;
            hash_field(&mut hasher, target.as_os_str().as_encoded_bytes());
        } else if metadata.is_dir() {
            hash_field(&mut hasher, b"directory");
        } else if metadata.is_file() {
            hash_field(&mut hasher, b"file");
            hash_field(&mut hasher, &metadata.len().to_be_bytes());
            let mut file = fs::File::open(&path).map_err(|_| owner_mismatch_error())?;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = file.read(&mut buffer).map_err(|_| owner_mismatch_error())?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
        } else {
            return Err(worktree_error(
                "attempt_manifest_entry_unsafe",
                "The attempt worktree contains an unsafe filesystem entry",
            ));
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_manifest_paths(
    root: &Path,
    directory: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), AttemptWorktreeError> {
    for entry in fs::read_dir(directory).map_err(|_| owner_mismatch_error())? {
        let entry = entry.map_err(|_| owner_mismatch_error())?;
        if directory == root && entry.file_name() == OsStr::new(".git") {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| owner_mismatch_error())?;
        paths.push(path.clone());
        if metadata.is_dir() && !metadata.file_type().is_symlink() && !is_reparse_point(&metadata) {
            collect_manifest_paths(root, &path, paths)?;
        }
    }
    Ok(())
}

fn validate_worktree_contents(root: &Path) -> Result<(), AttemptWorktreeError> {
    let canonical_root = fs::canonicalize(root).map_err(|_| owner_mismatch_error())?;
    let mut stack = vec![root.to_path_buf()];
    let mut folded = BTreeMap::<String, String>::new();
    let mut entries = 0usize;
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).map_err(|_| owner_mismatch_error())? {
            let entry = entry.map_err(|_| owner_mismatch_error())?;
            if directory == root && entry.file_name() == OsStr::new(".git") {
                continue;
            }
            entries = entries.saturating_add(1);
            if entries > MAX_MANIFEST_ENTRIES {
                return Err(worktree_error(
                    "attempt_manifest_too_large",
                    "The attempt worktree has too many entries to inspect safely",
                ));
            }
            let path = entry.path();
            let relative = path.strip_prefix(root).map_err(|_| managed_path_error())?;
            let relative_text = relative.to_str().ok_or_else(nonportable_path_error)?;
            validate_portable_tree_path(&relative_text.replace(std::path::MAIN_SEPARATOR, "/"))?;
            if insert_casefold_path(
                &mut folded,
                &relative_text.replace(std::path::MAIN_SEPARATOR, "/"),
            ) {
                return Err(worktree_error(
                    "attempt_tree_case_collision",
                    "The attempt worktree contains case-colliding paths",
                ));
            }
            let metadata = fs::symlink_metadata(&path).map_err(|_| owner_mismatch_error())?;
            if is_reparse_point(&metadata) {
                return Err(unsafe_link_error());
            }
            if metadata.file_type().is_symlink() {
                let target = fs::read_link(&path).map_err(|_| unsafe_link_error())?;
                validate_repository_symlink(
                    &relative_text.replace(std::path::MAIN_SEPARATOR, "/"),
                    target.as_os_str().as_encoded_bytes(),
                )?;
                let canonical = fs::canonicalize(&path).map_err(|_| unsafe_link_error())?;
                if !canonical.starts_with(&canonical_root) {
                    return Err(unsafe_link_error());
                }
            } else if metadata.is_dir() {
                stack.push(path);
            } else if !metadata.is_file() {
                return Err(worktree_error(
                    "attempt_manifest_entry_unsafe",
                    "The attempt worktree contains an unsafe filesystem entry",
                ));
            } else {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if metadata.nlink() > 1 {
                        return Err(worktree_error(
                            "attempt_hardlink_unsafe",
                            "The attempt worktree contains a multiply linked file",
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn reject_git_operation_state(
    service: &AttemptWorktreeService,
    context: &RepositoryContext,
    worktree: &Path,
) -> Result<(), AttemptWorktreeError> {
    let output = service.run_git(
        context,
        GitInvocation::new(
            worktree,
            [
                OsString::from("rev-parse"),
                OsString::from("--path-format=absolute"),
                OsString::from("--git-dir"),
            ],
        ),
    )?;
    let git_dir =
        fs::canonicalize(single_path(&output.stdout)?).map_err(|_| owner_mismatch_error())?;
    if !git_dir.starts_with(&context.common_dir) {
        return Err(owner_mismatch_error());
    }
    for marker in [
        "rebase-merge",
        "rebase-apply",
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "BISECT_LOG",
        "index.lock",
    ] {
        if fs::symlink_metadata(git_dir.join(marker)).is_ok() {
            return Err(worktree_error(
                "attempt_git_operation_in_progress",
                "The attempt worktree has an interrupted Git operation",
            ));
        }
    }
    Ok(())
}

fn ensure_git_controls(
    data_paths: &DataPaths,
) -> Result<(PathBuf, PathBuf, PathBuf), AttemptWorktreeError> {
    let root = data_paths.recovery().join("attempt-git-controls");
    ensure_managed_directory(data_paths, &root)?;
    let hooks = root.join("empty-hooks");
    ensure_managed_directory(data_paths, &hooks)?;
    let global = root.join("empty-global-config");
    ensure_empty_managed_file(data_paths, &global)?;
    let attributes = root.join("empty-global-attributes");
    ensure_empty_managed_file(data_paths, &attributes)?;
    Ok((hooks, global, attributes))
}

fn ensure_empty_managed_file(
    data_paths: &DataPaths,
    path: &Path,
) -> Result<(), AttemptWorktreeError> {
    if !data_paths.owns(path) {
        return Err(managed_path_error());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && !is_reparse_point(&metadata)
                && metadata.len() == 0 => {}
        Ok(_) => return Err(managed_path_error()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(path).map_err(|_| managed_path_error())?;
                    if !metadata.is_file()
                        || metadata.file_type().is_symlink()
                        || is_reparse_point(&metadata)
                        || metadata.len() != 0
                    {
                        return Err(managed_path_error());
                    }
                }
                Err(_) => return Err(managed_path_error()),
            }
        }
        Err(_) => return Err(managed_path_error()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(path).map_err(|_| managed_path_error())?;
        let root = fs::symlink_metadata(data_paths.root()).map_err(|_| managed_path_error())?;
        if metadata.uid() != root.uid() {
            return Err(managed_path_error());
        }
    }
    #[cfg(windows)]
    crate::windows_security::validate_private_descendant(path).map_err(|_| managed_path_error())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| managed_path_error())?;
    }
    Ok(())
}

fn ensure_managed_parent_directories(
    data_paths: &DataPaths,
    leaf: &Path,
) -> Result<(), AttemptWorktreeError> {
    let parent = leaf.parent().ok_or_else(managed_path_error)?;
    ensure_managed_directory(data_paths, parent)
}

fn ensure_managed_directory(
    data_paths: &DataPaths,
    path: &Path,
) -> Result<(), AttemptWorktreeError> {
    if !path.starts_with(data_paths.root()) || !data_paths.owns(path) {
        return Err(managed_path_error());
    }
    let relative = path
        .strip_prefix(data_paths.root())
        .map_err(|_| managed_path_error())?;
    let mut current = data_paths.root().to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(managed_path_error());
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => validate_directory_metadata(data_paths, &current, &metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                #[cfg(windows)]
                let created = crate::windows_security::create_private_directory(&current);
                #[cfg(not(windows))]
                let created = fs::create_dir(&current);
                match created {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => return Err(managed_path_error()),
                }
                let metadata = fs::symlink_metadata(&current).map_err(|_| managed_path_error())?;
                validate_directory_metadata(data_paths, &current, &metadata)?;
            }
            Err(_) => return Err(managed_path_error()),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&current, fs::Permissions::from_mode(0o700))
                .map_err(|_| managed_path_error())?;
        }
    }
    Ok(())
}

fn ensure_empty_owned_tree_path(
    data_paths: &DataPaths,
    path: &Path,
) -> Result<(), AttemptWorktreeError> {
    ensure_managed_directory(data_paths, path)?;
    validate_owned_tree_path(data_paths, path)?;
    let mut entries = fs::read_dir(path).map_err(|_| managed_path_error())?;
    if entries
        .next()
        .transpose()
        .map_err(|_| managed_path_error())?
        .is_some()
    {
        return Err(foreign_path_error());
    }
    Ok(())
}

fn validate_owned_tree_path(
    data_paths: &DataPaths,
    path: &Path,
) -> Result<(), AttemptWorktreeError> {
    if !path.starts_with(data_paths.worktrees()) || !data_paths.owns(path) {
        return Err(managed_path_error());
    }
    reject_link_components(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| owner_mismatch_error())?;
    validate_directory_metadata(data_paths, path, &metadata)
}

fn validate_directory_metadata(
    data_paths: &DataPaths,
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), AttemptWorktreeError> {
    if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point(metadata) {
        return Err(managed_path_error());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let root = fs::symlink_metadata(data_paths.root()).map_err(|_| managed_path_error())?;
        if metadata.uid() != root.uid() {
            return Err(managed_path_error());
        }
    }
    #[cfg(windows)]
    crate::windows_security::validate_private_descendant(path).map_err(|_| managed_path_error())?;
    #[cfg(windows)]
    let _ = data_paths;
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

fn reject_link_components(path: &Path) -> Result<(), AttemptWorktreeError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || is_reparse_point(&metadata) => {
                return Err(unsafe_link_error());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err(managed_path_error()),
        }
    }
    Ok(())
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

fn ensure_path_absent(path: &Path) -> Result<(), AttemptWorktreeError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(foreign_path_error()),
        Err(_) => Err(managed_path_error()),
    }
}

fn sanitize_git_environment(command: &mut Command) {
    for variable in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_ASKPASS",
        "GIT_CEILING_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_SYSTEM",
        "GIT_DIR",
        "GIT_EXEC_PATH",
        "GIT_EXTERNAL_DIFF",
        "GIT_INDEX_FILE",
        "GIT_NAMESPACE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PROXY_COMMAND",
        "GIT_REPLACE_REF_BASE",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_TEMPLATE_DIR",
        "GIT_TRACE",
        "GIT_TRACE_CURL",
        "GIT_TRACE_CURL_NO_DATA",
        "GIT_TRACE_PACKET",
        "GIT_TRACE_PACK_ACCESS",
        "GIT_TRACE_PERFORMANCE",
        "GIT_TRACE_REFS",
        "GIT_TRACE_SETUP",
        "GIT_TRACE_SHALLOW",
        "GIT_TRACE2",
        "GIT_TRACE2_EVENT",
        "GIT_TRACE2_PERF",
        "GIT_WORK_TREE",
        "SSH_ASKPASS",
        "SSH_ASKPASS_REQUIRE",
    ] {
        command.env_remove(variable);
    }
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !matches!(value, "." | "..")
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn single_line(bytes: &[u8]) -> Result<&str, AttemptWorktreeError> {
    let value = std::str::from_utf8(bytes).map_err(|_| {
        worktree_error(
            "attempt_git_output_invalid",
            "The bounded attempt Git command returned invalid output",
        )
    })?;
    let value = value.trim();
    if value.is_empty() || value.lines().count() != 1 || value.chars().any(char::is_control) {
        return Err(worktree_error(
            "attempt_git_output_invalid",
            "The bounded attempt Git command returned invalid output",
        ));
    }
    Ok(value)
}

fn single_path(bytes: &[u8]) -> Result<PathBuf, AttemptWorktreeError> {
    Ok(PathBuf::from(single_line(bytes)?))
}

fn path_text(path: &Path) -> Result<String, AttemptWorktreeError> {
    path.to_str()
        .filter(|value| !value.chars().any(char::is_control))
        .map(ToOwned::to_owned)
        .ok_or_else(nonportable_path_error)
}

fn git_paths_equal(candidate: &str, expected: &Path) -> bool {
    let candidate = PathBuf::from(candidate);
    if let (Ok(candidate), Ok(expected)) =
        (fs::canonicalize(&candidate), fs::canonicalize(expected))
    {
        return candidate == expected;
    }
    #[cfg(windows)]
    {
        candidate
            .to_string_lossy()
            .replace('\\', "/")
            .eq_ignore_ascii_case(&expected.to_string_lossy().replace('\\', "/"))
    }
    #[cfg(not(windows))]
    {
        candidate == expected
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn validate_failpoint(
    failpoint: Option<AttemptWorktreeFailpoint>,
    allowed: &[AttemptWorktreeFailpoint],
) -> Result<(), AttemptWorktreeError> {
    if failpoint.is_some_and(|candidate| !allowed.contains(&candidate)) {
        return Err(worktree_error(
            "attempt_worktree_failpoint_invalid",
            "The attempt worktree failpoint is invalid for this operation",
        ));
    }
    Ok(())
}

fn worktree_error(reason_code: &'static str, safe_message: &str) -> AttemptWorktreeError {
    AttemptWorktreeError {
        reason_code,
        safe_message: safe_message.to_string(),
    }
}

fn owner_mismatch_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_worktree_owner_mismatch",
        "The attempt worktree no longer matches its durable owner",
    )
}

fn changed_replay_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_worktree_replay_conflict",
        "The attempt worktree identity was reused with different bytes",
    )
}

fn invalid_identity_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_worktree_identity_invalid",
        "The attempt worktree identity is invalid",
    )
}

#[track_caller]
fn managed_path_error() -> AttemptWorktreeError {
    #[cfg(all(test, windows))]
    {
        let caller = std::panic::Location::caller();
        return worktree_error(
            "attempt_worktree_path_unsafe",
            &format!(
                "The attempt worktree path is outside the private managed root [test diagnostic at {}:{}]",
                caller.file(),
                caller.line()
            ),
        );
    }
    #[cfg(not(all(test, windows)))]
    worktree_error(
        "attempt_worktree_path_unsafe",
        "The attempt worktree path is outside the private managed root",
    )
}

fn foreign_path_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_worktree_path_foreign",
        "The attempt worktree path is already owned by another resource",
    )
}

fn foreign_ref_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_worktree_ref_foreign",
        "The attempt worktree reference is already owned by another resource",
    )
}

fn filter_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_repository_filter_unsafe",
        "The attempt baseline uses an external or custom content filter",
    )
}

fn unsafe_link_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_worktree_link_unsafe",
        "The attempt worktree contains an unsafe link or reparse point",
    )
}

fn nonportable_path_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_tree_path_nonportable",
        "The attempt repository contains a non-portable path",
    )
}

fn cancelled_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_worktree_service_closed",
        "The attempt worktree service is shutting down",
    )
}

fn worker_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_worktree_worker_failed",
        "The bounded attempt worktree worker could not be joined",
    )
}

fn git_start_error() -> AttemptWorktreeError {
    worktree_error(
        "attempt_git_start_failed",
        "The bounded attempt Git command could not be started",
    )
}

fn injected_error(message: &str) -> AttemptWorktreeError {
    worktree_error("attempt_worktree_injected_failure", message)
}

impl AttemptWorktreeService {
    fn inspect_repository(
        &self,
        repository_root: &Path,
        expected_common_identity: &str,
        baseline_commit: &str,
    ) -> Result<RepositoryContext, AttemptWorktreeError> {
        if !repository_root.is_absolute() || !valid_object_id(baseline_commit) {
            return Err(worktree_error(
                "attempt_repository_invalid",
                "The attempt repository or baseline is invalid",
            ));
        }
        reject_link_components(repository_root)?;
        let root = fs::canonicalize(repository_root).map_err(|_| {
            worktree_error(
                "attempt_repository_missing",
                "The attempt repository could not be opened",
            )
        })?;
        reject_link_components(&root)?;
        let git_entry = root.join(".git");
        reject_link_components(&git_entry)?;
        let git_metadata = fs::symlink_metadata(&git_entry).map_err(|_| {
            worktree_error(
                "attempt_repository_identity_invalid",
                "The attempt repository identity could not be verified",
            )
        })?;
        if git_metadata.file_type().is_symlink()
            || is_reparse_point(&git_metadata)
            || !(git_metadata.is_dir() || git_metadata.is_file())
        {
            return Err(worktree_error(
                "attempt_repository_identity_invalid",
                "The attempt repository identity could not be verified",
            ));
        }
        let controls = ensure_git_controls(&self.data_paths)?;
        let provisional = RepositoryContext {
            root: root.clone(),
            common_dir: root.clone(),
            common_identity: String::new(),
            hooks_dir: controls.0,
            empty_global_config: controls.1,
            empty_attributes: controls.2,
        };
        let output = self.run_git(
            &provisional,
            GitInvocation::new(
                &root,
                [
                    OsString::from("rev-parse"),
                    OsString::from("--path-format=absolute"),
                    OsString::from("--git-common-dir"),
                ],
            ),
        )?;
        let common_dir = single_path(&output.stdout)?;
        reject_link_components(&common_dir)?;
        let common_dir = fs::canonicalize(common_dir).map_err(|_| {
            worktree_error(
                "attempt_repository_identity_invalid",
                "The attempt repository identity could not be verified",
            )
        })?;
        let common_identity = format!(
            "git-common-dir-sha256:{}",
            sha256_hex(common_dir.to_string_lossy().as_bytes())
        );
        if common_identity != expected_common_identity {
            return Err(owner_mismatch_error());
        }
        let context = RepositoryContext {
            root,
            common_dir,
            common_identity,
            ..provisional
        };
        let resolved = self.run_git(
            &context,
            GitInvocation::new(
                &context.root,
                [
                    OsString::from("rev-parse"),
                    OsString::from("--verify"),
                    OsString::from(format!("{}^{{commit}}", baseline_commit)),
                ],
            ),
        )?;
        let resolved = single_line(&resolved.stdout)?;
        if !resolved.eq_ignore_ascii_case(baseline_commit) {
            return Err(worktree_error(
                "attempt_baseline_not_exact",
                "The attempt baseline does not resolve to the exact requested commit",
            ));
        }
        self.reject_repository_filters(&context, baseline_commit)?;
        self.validate_baseline_tree(&context, baseline_commit)?;
        Ok(context)
    }

    fn reject_repository_filters(
        &self,
        context: &RepositoryContext,
        baseline_commit: &str,
    ) -> Result<(), AttemptWorktreeError> {
        let mut config = GitInvocation::new(
            &context.root,
            [
                OsString::from("config"),
                OsString::from("--local"),
                OsString::from("--null"),
                OsString::from("--get-regexp"),
                OsString::from("^filter\\."),
            ],
        );
        config.accepted_exit_codes.push(1);
        if !self.run_git(context, config)?.stdout.is_empty() {
            return Err(filter_error());
        }
        let info_attributes = context.common_dir.join("info").join("attributes");
        match fs::symlink_metadata(&info_attributes) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || is_reparse_point(&metadata)
                    || metadata.len() > MAX_ATTRIBUTE_BYTES as u64
                {
                    return Err(filter_error());
                }
                let bytes = fs::read(&info_attributes).map_err(|_| filter_error())?;
                reject_filter_attributes(&bytes)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(filter_error()),
        }
        let output = self.run_git(
            context,
            GitInvocation::new(
                &context.root,
                [
                    OsString::from("ls-tree"),
                    OsString::from("-rz"),
                    OsString::from("--full-tree"),
                    OsString::from(baseline_commit),
                ],
            ),
        )?;
        for entry in parse_tree_entries(&output.stdout)? {
            if entry.path == ".gitattributes" || entry.path.ends_with("/.gitattributes") {
                let mut invocation = GitInvocation::new(
                    &context.root,
                    [
                        OsString::from("cat-file"),
                        OsString::from("blob"),
                        OsString::from(&entry.object_id),
                    ],
                );
                invocation.stdout_limit = MAX_ATTRIBUTE_BYTES;
                let attributes = self.run_git(context, invocation)?.stdout;
                reject_filter_attributes(&attributes)?;
            }
        }
        Ok(())
    }

    fn validate_baseline_tree(
        &self,
        context: &RepositoryContext,
        baseline_commit: &str,
    ) -> Result<(), AttemptWorktreeError> {
        let output = self.run_git(
            context,
            GitInvocation::new(
                &context.root,
                [
                    OsString::from("ls-tree"),
                    OsString::from("-rz"),
                    OsString::from("--full-tree"),
                    OsString::from(baseline_commit),
                ],
            ),
        )?;
        let entries = parse_tree_entries(&output.stdout)?;
        let mut folded = BTreeMap::<String, String>::new();
        for entry in &entries {
            validate_portable_tree_path(&entry.path)?;
            if insert_casefold_path(&mut folded, &entry.path) {
                return Err(worktree_error(
                    "attempt_tree_case_collision",
                    "The attempt baseline contains case-colliding paths",
                ));
            }
            if entry.mode == "160000" {
                continue;
            }
            if !matches!(entry.mode.as_str(), "100644" | "100755" | "120000") {
                return Err(worktree_error(
                    "attempt_tree_mode_unsupported",
                    "The attempt baseline contains an unsupported entry",
                ));
            }
            if entry.mode == "120000" {
                let mut invocation = GitInvocation::new(
                    &context.root,
                    [
                        OsString::from("cat-file"),
                        OsString::from("blob"),
                        OsString::from(&entry.object_id),
                    ],
                );
                invocation.stdout_limit = 16 * 1024;
                let target = self.run_git(context, invocation)?.stdout;
                validate_repository_symlink(&entry.path, &target)?;
            }
        }
        Ok(())
    }

    fn materialize_or_recover(
        &self,
        context: &RepositoryContext,
        identity: &DerivedAttemptIdentity,
        baseline_commit: &str,
        durable_state: WorktreeDirtyState,
    ) -> Result<bool, AttemptWorktreeError> {
        ensure_managed_parent_directories(&self.data_paths, &identity.path)?;
        let path_exists = fs::symlink_metadata(&identity.path).is_ok();
        let reference = self.read_ref(context, &identity.branch_ref)?;
        if durable_state == WorktreeDirtyState::Clean {
            if !path_exists
                || reference
                    .as_deref()
                    .is_none_or(|value| !value.eq_ignore_ascii_case(baseline_commit))
            {
                return Err(owner_mismatch_error());
            }
            return Ok(false);
        }
        match (path_exists, reference) {
            (false, None) => {
                ensure_empty_owned_tree_path(&self.data_paths, &identity.path)?;
                self.run_git(
                    context,
                    GitInvocation::new(
                        &context.root,
                        [
                            OsString::from("worktree"),
                            OsString::from("add"),
                            OsString::from("--no-track"),
                            OsString::from("-b"),
                            OsString::from(&identity.branch_name),
                            identity.path.as_os_str().to_os_string(),
                            OsString::from(baseline_commit),
                        ],
                    ),
                )?;
                Ok(true)
            }
            (false, Some(reference)) if reference.eq_ignore_ascii_case(baseline_commit) => {
                ensure_empty_owned_tree_path(&self.data_paths, &identity.path)?;
                self.run_git(
                    context,
                    GitInvocation::new(
                        &context.root,
                        [
                            OsString::from("worktree"),
                            OsString::from("add"),
                            OsString::from("--no-track"),
                            identity.path.as_os_str().to_os_string(),
                            OsString::from(&identity.branch_name),
                        ],
                    ),
                )?;
                Ok(true)
            }
            (true, None) if durable_state == WorktreeDirtyState::Ambiguous => {
                ensure_empty_owned_tree_path(&self.data_paths, &identity.path)?;
                self.run_git(
                    context,
                    GitInvocation::new(
                        &context.root,
                        [
                            OsString::from("worktree"),
                            OsString::from("add"),
                            OsString::from("--no-track"),
                            OsString::from("-b"),
                            OsString::from(&identity.branch_name),
                            identity.path.as_os_str().to_os_string(),
                            OsString::from(baseline_commit),
                        ],
                    ),
                )?;
                Ok(true)
            }
            (true, Some(reference)) if reference.eq_ignore_ascii_case(baseline_commit) => Ok(false),
            (true, None) => Err(foreign_path_error()),
            (_, Some(_)) => Err(foreign_ref_error()),
        }
    }

    fn read_ref(
        &self,
        context: &RepositoryContext,
        reference: &str,
    ) -> Result<Option<String>, AttemptWorktreeError> {
        let mut invocation = GitInvocation::new(
            &context.root,
            [
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("--quiet"),
                OsString::from(reference),
            ],
        );
        invocation.accepted_exit_codes.push(1);
        let output = self.run_git(context, invocation)?;
        if output.exit_code == 1 {
            Ok(None)
        } else {
            Ok(Some(single_line(&output.stdout)?.to_ascii_lowercase()))
        }
    }

    fn verify_owned_worktree(
        &self,
        context: &RepositoryContext,
        identity: &DerivedAttemptIdentity,
        baseline_commit: &str,
        require_exact_baseline: bool,
    ) -> Result<String, AttemptWorktreeError> {
        validate_owned_tree_path(&self.data_paths, &identity.path)?;
        let git_entry = identity.path.join(".git");
        reject_link_components(&git_entry)?;
        let git_metadata = fs::symlink_metadata(&git_entry).map_err(|_| owner_mismatch_error())?;
        if !git_metadata.is_file()
            || git_metadata.file_type().is_symlink()
            || is_reparse_point(&git_metadata)
        {
            return Err(owner_mismatch_error());
        }
        let common = self.run_git(
            context,
            GitInvocation::new(
                &identity.path,
                [
                    OsString::from("rev-parse"),
                    OsString::from("--path-format=absolute"),
                    OsString::from("--git-common-dir"),
                ],
            ),
        )?;
        let common =
            fs::canonicalize(single_path(&common.stdout)?).map_err(|_| owner_mismatch_error())?;
        if common != context.common_dir {
            return Err(owner_mismatch_error());
        }
        let branch = self.run_git(
            context,
            GitInvocation::new(
                &identity.path,
                [
                    OsString::from("symbolic-ref"),
                    OsString::from("-q"),
                    OsString::from("HEAD"),
                ],
            ),
        )?;
        if single_line(&branch.stdout)? != identity.branch_ref {
            return Err(owner_mismatch_error());
        }
        let head = self.run_git(
            context,
            GitInvocation::new(
                &identity.path,
                [
                    OsString::from("rev-parse"),
                    OsString::from("--verify"),
                    OsString::from("HEAD"),
                ],
            ),
        )?;
        let head = single_line(&head.stdout)?.to_ascii_lowercase();
        if require_exact_baseline && !head.eq_ignore_ascii_case(baseline_commit) {
            return Err(owner_mismatch_error());
        }
        if !require_exact_baseline {
            let mut ancestry = GitInvocation::new(
                &identity.path,
                [
                    OsString::from("merge-base"),
                    OsString::from("--is-ancestor"),
                    OsString::from(baseline_commit),
                    OsString::from(&head),
                ],
            );
            ancestry.accepted_exit_codes.push(1);
            if self.run_git(context, ancestry)?.exit_code != 0 {
                return Err(owner_mismatch_error());
            }
        }
        reject_git_operation_state(self, context, &identity.path)?;
        validate_worktree_contents(&identity.path)?;
        if require_exact_baseline {
            let status = self.status_bytes(context, &identity.path)?;
            if !status.is_empty() {
                return Err(worktree_error(
                    "attempt_worktree_not_clean",
                    "The new attempt worktree is not clean",
                ));
            }
        }
        Ok(head)
    }

    fn classify_owned_worktree(
        &self,
        context: &RepositoryContext,
        identity: &DerivedAttemptIdentity,
        baseline_commit: &str,
    ) -> Result<WorktreeDirtyState, AttemptWorktreeError> {
        if self.worktree_is_locked(context, &identity.path)? {
            return Ok(WorktreeDirtyState::Locked);
        }
        self.verify_owned_worktree(context, identity, baseline_commit, false)?;
        if self.status_bytes(context, &identity.path)?.is_empty() {
            Ok(WorktreeDirtyState::Clean)
        } else {
            Ok(WorktreeDirtyState::Dirty)
        }
    }

    fn status_bytes(
        &self,
        context: &RepositoryContext,
        path: &Path,
    ) -> Result<Vec<u8>, AttemptWorktreeError> {
        Ok(self
            .run_git(
                context,
                GitInvocation::new(
                    path,
                    [
                        OsString::from("status"),
                        OsString::from("--porcelain=v1"),
                        OsString::from("-z"),
                        OsString::from("--untracked-files=all"),
                    ],
                ),
            )?
            .stdout)
    }

    fn worktree_is_locked(
        &self,
        context: &RepositoryContext,
        path: &Path,
    ) -> Result<bool, AttemptWorktreeError> {
        let output = self.run_git(
            context,
            GitInvocation::new(
                &context.root,
                [
                    OsString::from("worktree"),
                    OsString::from("list"),
                    OsString::from("--porcelain"),
                ],
            ),
        )?;
        let target = fs::canonicalize(path).map_err(|_| owner_mismatch_error())?;
        let text = std::str::from_utf8(&output.stdout).map_err(|_| owner_mismatch_error())?;
        let mut matches_target = false;
        for line in text.lines() {
            if line.is_empty() {
                matches_target = false;
            } else if let Some(candidate) = line.strip_prefix("worktree ") {
                matches_target = git_paths_equal(candidate, &target);
            } else if matches_target && line.starts_with("locked") {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn worktree_is_registered(
        &self,
        context: &RepositoryContext,
        path: &Path,
    ) -> Result<bool, AttemptWorktreeError> {
        let output = self.run_git(
            context,
            GitInvocation::new(
                &context.root,
                [
                    OsString::from("worktree"),
                    OsString::from("list"),
                    OsString::from("--porcelain"),
                ],
            ),
        )?;
        let text = std::str::from_utf8(&output.stdout).map_err(|_| owner_mismatch_error())?;
        Ok(text
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .any(|candidate| git_paths_equal(candidate, path)))
    }
}

#[derive(Clone, Debug)]
struct RepositoryContext {
    root: PathBuf,
    common_dir: PathBuf,
    common_identity: String,
    hooks_dir: PathBuf,
    empty_global_config: PathBuf,
    empty_attributes: PathBuf,
}

#[derive(Clone, Debug)]
struct DerivedAttemptIdentity {
    branch_ref: String,
    branch_name: String,
    path: PathBuf,
}

#[derive(Clone, Debug)]
struct GitOutput {
    stdout: Vec<u8>,
    exit_code: i32,
}

#[derive(Clone, Debug)]
struct GitInvocation {
    cwd: PathBuf,
    args: Vec<OsString>,
    stdin: Vec<u8>,
    environment: Vec<(OsString, OsString)>,
    accepted_exit_codes: Vec<i32>,
    stdout_limit: usize,
}

impl GitInvocation {
    fn new(cwd: &Path, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            args: args.into_iter().map(Into::into).collect(),
            stdin: Vec::new(),
            environment: Vec::new(),
            accepted_exit_codes: vec![0],
            stdout_limit: MAX_GIT_STDOUT_BYTES,
        }
    }
}

impl AttemptWorktreeService {
    fn create_sync(
        &self,
        request: CreateAttemptWorktreeRequest,
        failpoint: Option<AttemptWorktreeFailpoint>,
    ) -> Result<AttemptWorktreeOutcome, AttemptWorktreeError> {
        self.data_paths
            .ensure_private_roots()
            .map_err(|_| managed_path_error())?;
        validate_create_request(&request)?;
        let run = self.load_run(&request.run_id)?;
        let attempt = run.run.attempts.get(&request.attempt_id).ok_or_else(|| {
            worktree_error(
                "attempt_worktree_owner_missing",
                "The attempt worktree owner does not exist",
            )
        })?;
        if attempt.task_id != request.task_id {
            return Err(owner_mismatch_error());
        }
        let repository = self
            .store
            .get_project_repository(&run.project_id, &run.run.organization_id)?
            .ok_or_else(|| {
                worktree_error(
                    "attempt_repository_missing",
                    "The attempt repository is not registered",
                )
            })?;
        if run.repository_id != request.repository_id || repository.id != request.repository_id {
            return Err(owner_mismatch_error());
        }

        let identity = derive_attempt_identity(&self.data_paths, &request, attempt.ordinal)?;
        let preexisting = self
            .store
            .load_attempt_worktree_for_attempt(&request.run_id, &request.attempt_id)?;
        if preexisting.as_ref().is_some_and(|record| {
            record.worktree.id != request.worktree_id
                || record.worktree.repository_id != request.repository_id
                || record.worktree.run_id != request.run_id
                || record.worktree.task_id != request.task_id
                || record.worktree.attempt_id != request.attempt_id
                || record.worktree.environment_id != request.environment_id
                || !record
                    .worktree
                    .baseline_commit
                    .eq_ignore_ascii_case(&request.baseline_commit)
                || record.worktree.branch_ref != identity.branch_ref
                || record.worktree.path != path_text(&identity.path).unwrap_or_default()
                || record.worktree.lease_id != request.lease.id
                || record.created_at_ms != request.intent_at_ms
        }) {
            return Err(changed_replay_error());
        }
        let repository_context = self.inspect_repository(
            &request.repository_root,
            &repository.canonical_repository_root_identity,
            &request.baseline_commit,
        )?;
        let expected = OwnedAttemptWorktreeRecord {
            worktree: OwnedAttemptWorktree {
                id: request.worktree_id.clone(),
                repository_id: request.repository_id.clone(),
                run_id: request.run_id.clone(),
                task_id: request.task_id.clone(),
                attempt_id: request.attempt_id.clone(),
                environment_id: request.environment_id.clone(),
                git_common_dir_identity: repository_context.common_identity.clone(),
                baseline_commit: request.baseline_commit.to_ascii_lowercase(),
                branch_ref: identity.branch_ref.clone(),
                path: path_text(&identity.path)?,
                lease_id: request.lease.id.clone(),
                checkpoint_ids: BTreeSet::new(),
                dirty_state: WorktreeDirtyState::Ambiguous,
                cleanup_eligibility: WorktreeCleanupEligibility::Ineligible,
            },
            checkpoint_intent_sha256: BTreeMap::new(),
            verified_process_death_sha256: None,
            retained_evidence_sha256: None,
            created_at_ms: request.intent_at_ms,
            updated_at_ms: request.intent_at_ms,
        };

        let (record, intent_was_new) = match preexisting {
            Some(existing) => {
                validate_existing_intent(&existing, &expected, &request.lease)?;
                let (active, durable_lease) = self.store.load_active_attempt_worktree(
                    &request.worktree_id,
                    request.materialized_at_ms,
                )?;
                if active != existing || durable_lease != request.lease {
                    return Err(changed_replay_error());
                }
                (existing, false)
            }
            None => {
                if attempt.worktree_id.is_some() {
                    return Err(owner_mismatch_error());
                }
                reject_link_components(identity.path.parent().ok_or_else(managed_path_error)?)?;
                ensure_path_absent(&identity.path)?;
                if self
                    .read_ref(&repository_context, &identity.branch_ref)?
                    .is_some()
                {
                    return Err(foreign_ref_error());
                }
                let event = AssignedRunEvent {
                    event_id: RunEventId::new(format!(
                        "run-event-worktree-{}",
                        request.worktree_id.as_str()
                    )),
                    idempotency_key: format!("attempt-worktree:{}", request.worktree_id.as_str()),
                    at_ms: request.intent_at_ms,
                };
                let mut committed = None;
                for _ in 0..8 {
                    let latest = self.load_run(&request.run_id)?;
                    let latest_attempt = latest
                        .run
                        .attempts
                        .get(&request.attempt_id)
                        .ok_or_else(owner_mismatch_error)?;
                    if latest_attempt.task_id != request.task_id {
                        return Err(owner_mismatch_error());
                    }
                    if let Some(existing) = self
                        .store
                        .load_attempt_worktree_for_attempt(&request.run_id, &request.attempt_id)?
                    {
                        validate_existing_intent(&existing, &expected, &request.lease)?;
                        committed = Some((existing, false));
                        break;
                    }
                    if latest_attempt.worktree_id.is_some() {
                        return Err(owner_mismatch_error());
                    }
                    let transition = self.store.commit_run_transition(
                        &RunTransitionRequest {
                            run_id: request.run_id.clone(),
                            expected_last_seq: latest.run.last_seq,
                            expected_snapshot_sha256: orchestration_snapshot_sha256(&latest.run),
                            command: RunCommand::AssignAttemptWorktree {
                                attempt_id: request.attempt_id.clone(),
                                worktree_id: request.worktree_id.clone(),
                            },
                            assigned_events: vec![AssignedRunEvent {
                                at_ms: event.at_ms.max(latest.run.updated_at_ms),
                                ..event.clone()
                            }],
                            rows: RunTransitionRows {
                                leases: vec![request.lease.clone()],
                                worktrees: vec![expected.clone()],
                                ..RunTransitionRows::default()
                            },
                        },
                        None,
                    );
                    match transition {
                        Ok(transition) if !transition.duplicate => {
                            committed = Some((expected.clone(), true));
                            break;
                        }
                        Ok(_) => return Err(changed_replay_error()),
                        Err(_) => continue,
                    }
                }
                committed.ok_or_else(|| {
                    worktree_error(
                        "attempt_worktree_commit_contended",
                        "The attempt worktree ownership could not be committed without contention",
                    )
                })?
            }
        };
        if !intent_was_new {
            let (active, durable_lease) = self
                .store
                .load_active_attempt_worktree(&request.worktree_id, request.materialized_at_ms)?;
            if active != record || durable_lease != request.lease {
                return Err(changed_replay_error());
            }
        }

        if failpoint == Some(AttemptWorktreeFailpoint::AfterOwnershipIntent)
            || failpoint == Some(AttemptWorktreeFailpoint::BeforeWorktreeGit)
        {
            return Err(injected_error(
                "attempt worktree intent is durable before Git",
            ));
        }
        if record.worktree.cleanup_eligibility == WorktreeCleanupEligibility::Eligible {
            return Ok(AttemptWorktreeOutcome {
                record,
                git_materialized: false,
                recovered: true,
            });
        }
        if record.worktree.dirty_state == WorktreeDirtyState::Clean {
            self.verify_owned_worktree(
                &repository_context,
                &identity,
                &request.baseline_commit,
                false,
            )?;
            return Ok(AttemptWorktreeOutcome {
                record,
                git_materialized: false,
                recovered: true,
            });
        }

        let git_materialized = self.materialize_or_recover(
            &repository_context,
            &identity,
            &request.baseline_commit,
            record.worktree.dirty_state,
        )?;
        if failpoint == Some(AttemptWorktreeFailpoint::AfterWorktreeGit) {
            return Err(injected_error(
                "attempt worktree Git completion precedes its durable receipt",
            ));
        }
        self.verify_owned_worktree(
            &repository_context,
            &identity,
            &request.baseline_commit,
            true,
        )?;
        let record = self.store.mark_attempt_worktree_materialized(
            &request.worktree_id,
            &request.lease.id,
            request.materialized_at_ms,
        )?;
        Ok(AttemptWorktreeOutcome {
            record,
            git_materialized,
            recovered: !intent_was_new || !git_materialized,
        })
    }

    fn checkpoint_sync(
        &self,
        request: CreateAttemptCheckpointRequest,
        failpoint: Option<AttemptWorktreeFailpoint>,
    ) -> Result<CheckpointRef, AttemptWorktreeError> {
        self.data_paths
            .ensure_private_roots()
            .map_err(|_| managed_path_error())?;
        if !safe_component(request.checkpoint_id.as_str()) {
            return Err(invalid_identity_error());
        }
        let (record, _) = self
            .store
            .load_active_attempt_worktree(&request.worktree_id, request.created_at_ms)?;
        if record.worktree.lease_id != request.lease_id
            || record.worktree.cleanup_eligibility != WorktreeCleanupEligibility::Ineligible
        {
            return Err(owner_mismatch_error());
        }
        let run = self.load_run(&record.worktree.run_id)?;
        let attempt = run
            .run
            .attempts
            .get(&record.worktree.attempt_id)
            .ok_or_else(owner_mismatch_error)?;
        if attempt.worktree_id.as_ref() != Some(&record.worktree.id)
            || attempt.task_id != record.worktree.task_id
        {
            return Err(owner_mismatch_error());
        }
        let repository = self
            .store
            .get_project_repository(&run.project_id, &run.run.organization_id)?
            .ok_or_else(owner_mismatch_error)?;
        let repository_root = PathBuf::from(&repository.display_path);
        let repository_context = self.inspect_repository(
            &repository_root,
            &record.worktree.git_common_dir_identity,
            &record.worktree.baseline_commit,
        )?;
        let identity = identity_from_record(&self.data_paths, &record)?;
        let current_head = self.verify_owned_worktree(
            &repository_context,
            &identity,
            &record.worktree.baseline_commit,
            false,
        )?;
        let manifest_sha256 = worktree_manifest_sha256(&identity.path)?;
        let checkpoint_ref = checkpoint_ref(&record, &request)?;
        let checkpoint = CheckpointRef {
            id: request.checkpoint_id.clone(),
            run_id: record.worktree.run_id.clone(),
            value: checkpoint_ref.clone(),
            task_id: record.worktree.task_id.clone(),
            attempt_id: record.worktree.attempt_id.clone(),
            thread_id: match request.kind {
                CheckpointKind::BeforeAttempt => None,
                CheckpointKind::AfterAttempt | CheckpointKind::Recovery => {
                    attempt.thread_id.clone()
                }
            },
            kind: request.kind,
            created_at_ms: request.created_at_ms,
        };
        if let Some(existing) = run.run.checkpoints.get(&request.checkpoint_id) {
            if existing == &checkpoint
                && record
                    .checkpoint_intent_sha256
                    .contains_key(&request.checkpoint_id)
            {
                return Ok(existing.clone());
            }
            return Err(changed_replay_error());
        }
        let decided = pandamux_orchestrator::decide(
            &run.run,
            RunCommand::RecordCheckpoint {
                checkpoint: checkpoint.clone(),
            },
        )
        .map_err(|_| {
            worktree_error(
                "attempt_checkpoint_transition_invalid",
                "The attempt cannot record this checkpoint boundary",
            )
        })?;
        if decided.len() != 1 {
            return Err(worktree_error(
                "attempt_checkpoint_transition_invalid",
                "The attempt checkpoint did not produce one canonical event",
            ));
        }
        let payload_sha256 =
            checkpoint_payload_sha256(&record, &checkpoint, &current_head, &manifest_sha256);
        self.store.record_attempt_worktree_checkpoint_intent(
            &request.worktree_id,
            &request.lease_id,
            &request.checkpoint_id,
            &payload_sha256,
            request.created_at_ms,
        )?;
        if failpoint == Some(AttemptWorktreeFailpoint::AfterCheckpointIntent)
            || failpoint == Some(AttemptWorktreeFailpoint::BeforeCheckpointGit)
        {
            return Err(injected_error(
                "attempt checkpoint intent is durable before Git",
            ));
        }
        self.materialize_checkpoint(
            &repository_context,
            &identity,
            &checkpoint,
            &checkpoint_ref,
            &current_head,
        )?;
        let verified_head = self.verify_owned_worktree(
            &repository_context,
            &identity,
            &record.worktree.baseline_commit,
            false,
        )?;
        let verified_manifest = worktree_manifest_sha256(&identity.path)?;
        if verified_head != current_head || verified_manifest != manifest_sha256 {
            return Err(worktree_error(
                "attempt_checkpoint_source_changed",
                "The attempt worktree changed while its checkpoint was being captured",
            ));
        }
        if failpoint == Some(AttemptWorktreeFailpoint::AfterCheckpointGit) {
            return Err(injected_error(
                "attempt checkpoint Git completion precedes its canonical receipt",
            ));
        }
        for _ in 0..8 {
            let latest = self.load_run(&record.worktree.run_id)?;
            if let Some(existing) = latest.run.checkpoints.get(&request.checkpoint_id) {
                return if existing == &checkpoint {
                    Ok(existing.clone())
                } else {
                    Err(changed_replay_error())
                };
            }
            if latest.run.updated_at_ms > request.created_at_ms {
                return Err(worktree_error(
                    "attempt_checkpoint_timestamp_stale",
                    "The attempt checkpoint timestamp precedes canonical run state",
                ));
            }
            let transition = self.store.commit_run_transition(
                &RunTransitionRequest {
                    run_id: record.worktree.run_id.clone(),
                    expected_last_seq: latest.run.last_seq,
                    expected_snapshot_sha256: orchestration_snapshot_sha256(&latest.run),
                    command: RunCommand::RecordCheckpoint {
                        checkpoint: checkpoint.clone(),
                    },
                    assigned_events: vec![AssignedRunEvent {
                        event_id: RunEventId::new(format!(
                            "run-event-checkpoint-{}",
                            request.checkpoint_id.as_str()
                        )),
                        idempotency_key: format!(
                            "attempt-checkpoint:{}",
                            request.checkpoint_id.as_str()
                        ),
                        at_ms: request.created_at_ms,
                    }],
                    rows: RunTransitionRows::default(),
                },
                None,
            );
            if transition.is_ok() {
                return Ok(checkpoint);
            }
        }
        Err(worktree_error(
            "attempt_checkpoint_commit_contended",
            "The attempt checkpoint receipt could not be committed without contention",
        ))
    }

    fn cleanup_sync(
        &self,
        request: CleanupAttemptWorktreeRequest,
        failpoint: Option<AttemptWorktreeFailpoint>,
    ) -> Result<AttemptWorktreeCleanupOutcome, AttemptWorktreeError> {
        if !is_sha256(&request.verified_process_death_sha256)
            || !is_sha256(&request.retained_evidence_sha256)
        {
            return Err(worktree_error(
                "attempt_cleanup_evidence_invalid",
                "The attempt cleanup evidence is invalid",
            ));
        }
        self.data_paths
            .ensure_private_roots()
            .map_err(|_| managed_path_error())?;
        let record = self
            .store
            .load_owned_attempt_worktree(&request.worktree_id)?
            .ok_or_else(owner_mismatch_error)?;
        if record.worktree.lease_id != request.lease_id {
            return Err(owner_mismatch_error());
        }
        if record.worktree.cleanup_eligibility == WorktreeCleanupEligibility::Eligible {
            let exact = self.store.mark_attempt_worktree_cleaned(
                &request.worktree_id,
                &request.lease_id,
                &request.verified_process_death_sha256,
                &request.retained_evidence_sha256,
                request.cleaned_at_ms,
            )?;
            return Ok(AttemptWorktreeCleanupOutcome::Removed(exact));
        }
        let (active_record, _) = self
            .store
            .load_active_attempt_worktree(&request.worktree_id, request.cleaned_at_ms)?;
        if active_record != record {
            return Err(owner_mismatch_error());
        }
        let run = self.load_run(&record.worktree.run_id)?;
        let attempt = run
            .run
            .attempts
            .get(&record.worktree.attempt_id)
            .ok_or_else(owner_mismatch_error)?;
        if !matches!(
            attempt.status,
            AttemptStatus::Succeeded
                | AttemptStatus::Failed
                | AttemptStatus::Cancelled
                | AttemptStatus::Interrupted
        ) || attempt.worktree_id.as_ref() != Some(&record.worktree.id)
            || record.worktree.checkpoint_ids.is_empty()
            || !record
                .worktree
                .checkpoint_ids
                .iter()
                .all(|checkpoint_id| run.run.checkpoints.contains_key(checkpoint_id))
            || self.store.attempt_has_pending_recovery_review(
                &record.worktree.run_id,
                &record.worktree.task_id,
                &record.worktree.attempt_id,
            )?
        {
            return Err(worktree_error(
                "attempt_cleanup_not_eligible",
                "The attempt worktree does not have complete cleanup evidence",
            ));
        }
        let repository = self
            .store
            .get_project_repository(&run.project_id, &run.run.organization_id)?
            .ok_or_else(owner_mismatch_error)?;
        let repository_context = self.inspect_repository(
            Path::new(&repository.display_path),
            &record.worktree.git_common_dir_identity,
            &record.worktree.baseline_commit,
        )?;
        let identity = identity_from_record(&self.data_paths, &record)?;

        if !identity.path.exists() {
            if self.worktree_is_registered(&repository_context, &identity.path)? {
                let retained = self.store.retain_attempt_worktree(
                    &request.worktree_id,
                    &request.lease_id,
                    WorktreeDirtyState::Ambiguous,
                    request.cleaned_at_ms,
                )?;
                return Ok(AttemptWorktreeCleanupOutcome::Retained(retained));
            }
            let cleaned = self.store.mark_attempt_worktree_cleaned(
                &request.worktree_id,
                &request.lease_id,
                &request.verified_process_death_sha256,
                &request.retained_evidence_sha256,
                request.cleaned_at_ms,
            )?;
            return Ok(AttemptWorktreeCleanupOutcome::Removed(cleaned));
        }

        let classification = match self.classify_owned_worktree(
            &repository_context,
            &identity,
            &record.worktree.baseline_commit,
        ) {
            Ok(classification) => classification,
            Err(_) => WorktreeDirtyState::Ambiguous,
        };
        if classification != WorktreeDirtyState::Clean {
            let retained = self.store.retain_attempt_worktree(
                &request.worktree_id,
                &request.lease_id,
                classification,
                request.cleaned_at_ms,
            )?;
            return Ok(AttemptWorktreeCleanupOutcome::Retained(retained));
        }
        let removal = self.run_git(
            &repository_context,
            GitInvocation::new(
                &repository_context.root,
                [
                    OsString::from("worktree"),
                    OsString::from("remove"),
                    identity.path.as_os_str().to_os_string(),
                ],
            ),
        );
        if let Err(error) = removal {
            let classification = self
                .classify_owned_worktree(
                    &repository_context,
                    &identity,
                    &record.worktree.baseline_commit,
                )
                .unwrap_or(WorktreeDirtyState::Ambiguous);
            if classification != WorktreeDirtyState::Clean {
                let retained = self.store.retain_attempt_worktree(
                    &request.worktree_id,
                    &request.lease_id,
                    classification,
                    request.cleaned_at_ms,
                )?;
                return Ok(AttemptWorktreeCleanupOutcome::Retained(retained));
            }
            return Err(error);
        }
        if failpoint == Some(AttemptWorktreeFailpoint::AfterCleanupGit) {
            return Err(injected_error(
                "clean worktree removal precedes its durable cleanup receipt",
            ));
        }
        if identity.path.exists()
            || self.worktree_is_registered(&repository_context, &identity.path)?
        {
            return Err(worktree_error(
                "attempt_cleanup_unverified",
                "The attempt worktree removal could not be verified",
            ));
        }
        let cleaned = self.store.mark_attempt_worktree_cleaned(
            &request.worktree_id,
            &request.lease_id,
            &request.verified_process_death_sha256,
            &request.retained_evidence_sha256,
            request.cleaned_at_ms,
        )?;
        Ok(AttemptWorktreeCleanupOutcome::Removed(cleaned))
    }

    fn load_run(&self, run_id: &RunId) -> Result<OrchestrationRunRecord, AttemptWorktreeError> {
        self.store
            .load_verified_orchestration_run(run_id)?
            .ok_or_else(|| {
                worktree_error(
                    "attempt_run_missing",
                    "The attempt worktree run does not exist",
                )
            })
    }
}
