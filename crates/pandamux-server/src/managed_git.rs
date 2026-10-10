use crate::DataPaths;
use pandamux_core::SecretsPolicy;
use std::borrow::Cow;
use std::ffi::OsStr;
#[cfg(windows)]
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub const MAX_GIT_STDERR_BYTES: usize = 16 * 1024;
pub const MAX_GIT_TREE_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CONFIGURATION_GIT_BUNDLE_BYTES: u64 = 32 * 1024 * 1024;
pub const LOCAL_GIT_TIMEOUT: Duration = Duration::from_secs(30);
pub const FETCH_GIT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitTreeEntry {
    pub mode: &'static str,
    pub object_kind: &'static str,
    pub object_id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitCommitIdentity {
    pub name: String,
    pub email: String,
    pub timestamp_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagedGitCommand {
    InitBare {
        git_dir: PathBuf,
    },
    SymbolicRef {
        git_dir: PathBuf,
        name: String,
        target: String,
    },
    HashObject {
        git_dir: PathBuf,
        bytes: Vec<u8>,
    },
    MakeTree {
        git_dir: PathBuf,
        entries: Vec<GitTreeEntry>,
    },
    CommitTree {
        git_dir: PathBuf,
        tree: String,
        parent: Option<String>,
        message: String,
        identity: GitCommitIdentity,
    },
    UpdateRefs {
        git_dir: PathBuf,
        transaction: String,
    },
    RevParse {
        git_dir: PathBuf,
        revision: String,
    },
    RevList {
        git_dir: PathBuf,
        revision: String,
        max_count: usize,
    },
    ShowMessage {
        git_dir: PathBuf,
        commit: String,
    },
    ListTree {
        git_dir: PathBuf,
        treeish: String,
    },
    CatFile {
        git_dir: PathBuf,
        object: String,
        max_bytes: usize,
    },
    CatFileSize {
        git_dir: PathBuf,
        object: String,
    },
    ObjectType {
        git_dir: PathBuf,
        object: String,
    },
    IsBareRepository {
        git_dir: PathBuf,
    },
    IsAncestor {
        git_dir: PathBuf,
        ancestor: String,
        descendant: String,
    },
    LsRemote {
        git_dir: PathBuf,
        remote: String,
        references: Vec<String>,
    },
    Fetch {
        git_dir: PathBuf,
        remote: String,
        source: String,
        target: String,
    },
    VerifyCommit {
        git_dir: PathBuf,
        commit: String,
    },
    CreateBundle {
        git_dir: PathBuf,
        output_path: PathBuf,
        revision: String,
    },
    VerifyBundle {
        git_dir: PathBuf,
        bundle_path: PathBuf,
    },
    WorktreeAdd {
        git_dir: PathBuf,
        worktree_path: PathBuf,
        branch_ref: String,
        commit: String,
    },
    WorktreeHead {
        worktree_path: PathBuf,
    },
    WorktreeBranch {
        worktree_path: PathBuf,
    },
    WorktreeCommonDir {
        worktree_path: PathBuf,
    },
}

impl ManagedGitCommand {
    fn git_dir(&self) -> &Path {
        match self {
            Self::InitBare { git_dir }
            | Self::SymbolicRef { git_dir, .. }
            | Self::HashObject { git_dir, .. }
            | Self::MakeTree { git_dir, .. }
            | Self::CommitTree { git_dir, .. }
            | Self::UpdateRefs { git_dir, .. }
            | Self::RevParse { git_dir, .. }
            | Self::RevList { git_dir, .. }
            | Self::ShowMessage { git_dir, .. }
            | Self::ListTree { git_dir, .. }
            | Self::CatFile { git_dir, .. }
            | Self::CatFileSize { git_dir, .. }
            | Self::ObjectType { git_dir, .. }
            | Self::IsBareRepository { git_dir }
            | Self::IsAncestor { git_dir, .. }
            | Self::LsRemote { git_dir, .. }
            | Self::Fetch { git_dir, .. }
            | Self::VerifyCommit { git_dir, .. }
            | Self::CreateBundle { git_dir, .. }
            | Self::VerifyBundle { git_dir, .. }
            | Self::WorktreeAdd { git_dir, .. } => git_dir,
            Self::WorktreeHead { worktree_path }
            | Self::WorktreeBranch { worktree_path }
            | Self::WorktreeCommonDir { worktree_path } => worktree_path,
        }
    }

    fn additional_managed_path(&self) -> Option<&Path> {
        match self {
            Self::CreateBundle { output_path, .. } => Some(output_path),
            Self::VerifyBundle { bundle_path, .. } => Some(bundle_path),
            Self::WorktreeAdd { worktree_path, .. } => Some(worktree_path),
            _ => None,
        }
    }

    fn stdout_limit(&self) -> usize {
        match self {
            Self::ListTree { .. } => MAX_GIT_TREE_OUTPUT_BYTES,
            Self::CatFile { max_bytes, .. } => *max_bytes,
            Self::LsRemote { .. } => 4 * 1024 * 1024,
            _ => 1024 * 1024,
        }
    }

    fn remote(&self) -> Option<&str> {
        match self {
            Self::LsRemote { remote, .. } | Self::Fetch { remote, .. } => Some(remote),
            _ => None,
        }
    }

    fn accepts_exit_code(&self, code: Option<i32>) -> bool {
        matches!(self, Self::IsAncestor { .. }) && code == Some(1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedGitOutput {
    pub stdout: Vec<u8>,
    pub safe_stderr: String,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedGitError {
    pub reason: &'static str,
    pub safe_message: String,
    pub retryable: bool,
}

impl std::fmt::Display for ManagedGitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.safe_message)
    }
}

impl std::error::Error for ManagedGitError {}

#[derive(Clone)]
pub struct ManagedGitRunner {
    data_paths: DataPaths,
    shutdown: Arc<AtomicBool>,
    program: PathBuf,
    local_timeout: Duration,
    fetch_timeout: Duration,
}

impl ManagedGitRunner {
    pub fn new(data_paths: DataPaths) -> Self {
        Self {
            data_paths,
            shutdown: Arc::new(AtomicBool::new(false)),
            program: PathBuf::from("git"),
            local_timeout: LOCAL_GIT_TIMEOUT,
            fetch_timeout: FETCH_GIT_TIMEOUT,
        }
    }

    #[cfg(all(test, unix))]
    pub(crate) fn with_test_process(
        data_paths: DataPaths,
        program: PathBuf,
        local_timeout: Duration,
        fetch_timeout: Duration,
    ) -> Self {
        Self {
            data_paths,
            shutdown: Arc::new(AtomicBool::new(false)),
            program,
            local_timeout,
            fetch_timeout,
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    pub fn create_bundle_bytes(
        &self,
        git_dir: &Path,
        revision: &str,
    ) -> Result<Vec<u8>, ManagedGitError> {
        self.data_paths
            .ensure_private_roots()
            .map_err(|_| bundle_io_error())?;
        let output_path = self
            .data_paths
            .bundle_staging()
            .join(format!(".git-bundle-{}", uuid::Uuid::new_v4().simple()));
        let result = (|| {
            self.run(ManagedGitCommand::CreateBundle {
                git_dir: git_dir.to_path_buf(),
                output_path: output_path.clone(),
                revision: revision.to_string(),
            })?;
            let metadata = fs::symlink_metadata(&output_path).map_err(|_| bundle_io_error())?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() == 0
                || metadata.len() > MAX_CONFIGURATION_GIT_BUNDLE_BYTES
            {
                return Err(ManagedGitError {
                    reason: "git_bundle_size_invalid",
                    safe_message: "The configuration Git bundle has an invalid size".to_string(),
                    retryable: false,
                });
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&output_path, fs::Permissions::from_mode(0o600))
                    .map_err(|_| bundle_io_error())?;
            }
            fs::read(&output_path).map_err(|_| bundle_io_error())
        })();
        let _ = fs::remove_file(&output_path);
        result
    }

    pub fn run(&self, command: ManagedGitCommand) -> Result<ManagedGitOutput, ManagedGitError> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(ManagedGitError {
                reason: "git_command_cancelled",
                safe_message: "The managed Git command was cancelled during shutdown".to_string(),
                retryable: true,
            });
        }
        if !managed_path_is_safe(&self.data_paths, command.git_dir())
            || command
                .additional_managed_path()
                .is_some_and(|path| !managed_file_path_is_safe(&self.data_paths, path))
        {
            return Err(ManagedGitError {
                reason: "managed_path_unsafe",
                safe_message: "The managed Git path is outside the app-data root".to_string(),
                retryable: false,
            });
        }

        let stdout_limit = command.stdout_limit();
        let mut process = Command::new(&self.program);
        sanitize_git_environment(&mut process);
        process
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            process.creation_flags(CREATE_NO_WINDOW);
        }

        let stdin_bytes = configure_process(&mut process, &command)?;
        let mut child = process.spawn().map_err(|_| ManagedGitError {
            reason: "git_command_start_failed",
            safe_message: "The managed Git command could not be started".to_string(),
            retryable: true,
        })?;
        let stdout = child.stdout.take().ok_or_else(|| ManagedGitError {
            reason: "git_command_start_failed",
            safe_message: "The managed Git command did not expose standard output".to_string(),
            retryable: true,
        })?;
        let stderr = child.stderr.take().ok_or_else(|| ManagedGitError {
            reason: "git_command_start_failed",
            safe_message: "The managed Git command did not expose standard error".to_string(),
            retryable: true,
        })?;
        let stdout_thread = spawn_reader(stdout, stdout_limit);
        let stderr_thread = spawn_reader(stderr, MAX_GIT_STDERR_BYTES);
        if let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(&stdin_bytes).is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            return Err(ManagedGitError {
                reason: "git_command_input_failed",
                safe_message: "The managed Git command could not receive bounded input".to_string(),
                retryable: true,
            });
        }

        let started = Instant::now();
        let timeout = match command {
            ManagedGitCommand::Fetch { .. } | ManagedGitCommand::VerifyBundle { .. } => {
                self.fetch_timeout
            }
            _ => self.local_timeout,
        };
        let (status, cancelled, timed_out) = loop {
            if self.shutdown.load(Ordering::Acquire) {
                let _ = child.kill();
                break (child.wait().ok(), true, false);
            }
            match child.try_wait() {
                Ok(Some(status)) => break (Some(status), false, false),
                Ok(None) if started.elapsed() < timeout => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    break (child.wait().ok(), false, true);
                }
                Err(_) => {
                    let _ = child.kill();
                    break (child.wait().ok(), false, false);
                }
            }
        };
        let stdout = join_reader(stdout_thread)?;
        let stderr = join_reader(stderr_thread)?;
        if cancelled {
            return Err(ManagedGitError {
                reason: "git_command_cancelled",
                safe_message: "The managed Git command was cancelled during shutdown".to_string(),
                retryable: true,
            });
        }
        if timed_out {
            return Err(ManagedGitError {
                reason: "git_command_timed_out",
                safe_message: "The managed Git command exceeded its time limit".to_string(),
                retryable: true,
            });
        }
        if stdout.truncated {
            return Err(ManagedGitError {
                reason: "git_output_too_large",
                safe_message: "The managed Git command produced more output than allowed"
                    .to_string(),
                retryable: false,
            });
        }
        let safe_stderr = safe_git_stderr(
            &stderr.bytes,
            stderr.truncated,
            self.data_paths.root(),
            command.remote(),
        );
        let exit_code = status.as_ref().and_then(std::process::ExitStatus::code);
        if status.as_ref().is_none_or(|status| !status.success())
            && !command.accepts_exit_code(exit_code)
        {
            let (reason, retryable) = classify_git_failure(&command, &stderr.bytes, exit_code);
            return Err(ManagedGitError {
                reason,
                safe_message: if safe_stderr.is_empty() {
                    "The managed Git command failed".to_string()
                } else {
                    format!("The managed Git command failed: {safe_stderr}")
                },
                retryable,
            });
        }
        Ok(ManagedGitOutput {
            stdout: stdout.bytes,
            safe_stderr,
            exit_code,
        })
    }
}

fn sanitize_git_environment(process: &mut Command) {
    for variable in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_ASKPASS",
        "GIT_CEILING_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_GLOBAL",
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
        process.env_remove(variable);
    }
}

fn configure_process(
    process: &mut Command,
    command: &ManagedGitCommand,
) -> Result<Vec<u8>, ManagedGitError> {
    let mut stdin = Vec::new();
    match command {
        ManagedGitCommand::InitBare { git_dir } => {
            managed_git_config_args(process);
            process
                .arg("init")
                .arg("--bare")
                .arg(git_process_path(git_dir));
        }
        ManagedGitCommand::SymbolicRef {
            git_dir,
            name,
            target,
        } => {
            if name != "HEAD" {
                validate_ref(name)?;
            }
            validate_ref(target)?;
            git_dir_args(process, git_dir);
            process.arg("symbolic-ref").arg(name).arg(target);
        }
        ManagedGitCommand::HashObject { git_dir, bytes } => {
            git_dir_args(process, git_dir);
            process
                .arg("hash-object")
                .arg("-w")
                .arg("--stdin")
                .arg("--no-filters");
            stdin.extend_from_slice(bytes);
        }
        ManagedGitCommand::MakeTree { git_dir, entries } => {
            git_dir_args(process, git_dir);
            process.arg("mktree");
            for entry in entries {
                if !matches!(entry.mode, "100644" | "040000")
                    || !matches!(entry.object_kind, "blob" | "tree")
                    || entry.name.is_empty()
                    || entry.name.contains(['\0', '\n', '\r', '\t', '/'])
                    || !is_git_object_id(&entry.object_id)
                {
                    return Err(invalid_command("invalid Git tree entry"));
                }
                stdin.extend_from_slice(
                    format!(
                        "{} {} {}\t{}\n",
                        entry.mode, entry.object_kind, entry.object_id, entry.name
                    )
                    .as_bytes(),
                );
            }
        }
        ManagedGitCommand::CommitTree {
            git_dir,
            tree,
            parent,
            message,
            identity,
        } => {
            if !is_git_object_id(tree)
                || parent
                    .as_deref()
                    .is_some_and(|value| !is_git_object_id(value))
                || message.is_empty()
                || message.len() > 16 * 1024
                || identity.name.contains(['\0', '\n', '\r'])
                || identity.email.contains(['\0', '\n', '\r'])
            {
                return Err(invalid_command("invalid commit-tree input"));
            }
            git_dir_args(process, git_dir);
            process
                .env("GIT_AUTHOR_NAME", &identity.name)
                .env("GIT_AUTHOR_EMAIL", &identity.email)
                .env("GIT_COMMITTER_NAME", &identity.name)
                .env("GIT_COMMITTER_EMAIL", &identity.email)
                .env(
                    "GIT_AUTHOR_DATE",
                    format!("@{} +0000", identity.timestamp_seconds),
                )
                .env(
                    "GIT_COMMITTER_DATE",
                    format!("@{} +0000", identity.timestamp_seconds),
                )
                .arg("-c")
                .arg("commit.gpgsign=false")
                .arg("-c")
                .arg("core.hooksPath=")
                .arg("commit-tree")
                .arg(tree);
            if let Some(parent) = parent {
                process.arg("-p").arg(parent);
            }
            process.arg("-F").arg("-");
            stdin.extend_from_slice(message.as_bytes());
        }
        ManagedGitCommand::UpdateRefs {
            git_dir,
            transaction,
        } => {
            if transaction.len() > 64 * 1024 || transaction.contains('\0') {
                return Err(invalid_command("invalid update-ref transaction"));
            }
            git_dir_args(process, git_dir);
            process
                .arg("update-ref")
                .arg("--stdin")
                .arg("--no-create-reflog");
            stdin.extend_from_slice(transaction.as_bytes());
        }
        ManagedGitCommand::RevParse { git_dir, revision } => {
            validate_revision(revision)?;
            git_dir_args(process, git_dir);
            process.arg("rev-parse").arg("--verify").arg(revision);
        }
        ManagedGitCommand::RevList {
            git_dir,
            revision,
            max_count,
        } => {
            validate_revision(revision)?;
            if !(1..=1000).contains(max_count) {
                return Err(invalid_command("invalid revision-list limit"));
            }
            git_dir_args(process, git_dir);
            process
                .arg("rev-list")
                .arg("--first-parent")
                .arg(format!("--max-count={max_count}"))
                .arg(revision);
        }
        ManagedGitCommand::ShowMessage { git_dir, commit } => {
            validate_revision(commit)?;
            git_dir_args(process, git_dir);
            process.arg("show").arg("-s").arg("--format=%B").arg(commit);
        }
        ManagedGitCommand::ListTree { git_dir, treeish } => {
            validate_revision(treeish)?;
            git_dir_args(process, git_dir);
            process
                .arg("ls-tree")
                .arg("-r")
                .arg("-z")
                .arg("--full-tree")
                .arg(treeish);
        }
        ManagedGitCommand::CatFile {
            git_dir,
            object,
            max_bytes,
        } => {
            validate_revision(object)?;
            if *max_bytes > MAX_GIT_TREE_OUTPUT_BYTES {
                return Err(invalid_command("invalid object output limit"));
            }
            git_dir_args(process, git_dir);
            process.arg("cat-file").arg("blob").arg(object);
        }
        ManagedGitCommand::CatFileSize { git_dir, object } => {
            validate_revision(object)?;
            git_dir_args(process, git_dir);
            process.arg("cat-file").arg("-s").arg(object);
        }
        ManagedGitCommand::ObjectType { git_dir, object } => {
            validate_revision(object)?;
            git_dir_args(process, git_dir);
            process.arg("cat-file").arg("-t").arg(object);
        }
        ManagedGitCommand::IsBareRepository { git_dir } => {
            git_dir_args(process, git_dir);
            process.arg("rev-parse").arg("--is-bare-repository");
        }
        ManagedGitCommand::IsAncestor {
            git_dir,
            ancestor,
            descendant,
        } => {
            validate_revision(ancestor)?;
            validate_revision(descendant)?;
            git_dir_args(process, git_dir);
            process
                .arg("merge-base")
                .arg("--is-ancestor")
                .arg(ancestor)
                .arg(descendant);
        }
        ManagedGitCommand::LsRemote {
            git_dir,
            remote,
            references,
        } => {
            validate_remote(remote)?;
            if references.len() > 4 {
                return Err(invalid_command("too many managed Git discovery references"));
            }
            for reference in references {
                validate_remote_reference(reference)?;
            }
            git_dir_args(process, git_dir);
            process.arg("ls-remote").arg("--exit-code").arg(remote);
            process.args(references);
        }
        ManagedGitCommand::Fetch {
            git_dir,
            remote,
            source,
            target,
        } => {
            validate_remote(remote)?;
            validate_fetch_source(source)?;
            validate_ref(target)?;
            git_dir_args(process, git_dir);
            process
                .arg("fetch")
                .arg("--no-tags")
                .arg("--no-write-fetch-head")
                .arg("--no-recurse-submodules")
                .arg("--force")
                .arg(remote)
                .arg(format!("+{source}:{target}"));
        }
        ManagedGitCommand::VerifyCommit { git_dir, commit } => {
            validate_revision(commit)?;
            git_dir_args(process, git_dir);
            process.arg("verify-commit").arg("--raw").arg(commit);
        }
        ManagedGitCommand::CreateBundle {
            git_dir,
            output_path,
            revision,
        } => {
            validate_ref(revision)?;
            git_dir_args(process, git_dir);
            process
                .arg("bundle")
                .arg("create")
                .arg(git_process_path(output_path))
                .arg(revision);
        }
        ManagedGitCommand::VerifyBundle {
            git_dir,
            bundle_path,
        } => {
            git_dir_args(process, git_dir);
            process
                .arg("bundle")
                .arg("verify")
                .arg(git_process_path(bundle_path));
        }
        ManagedGitCommand::WorktreeAdd {
            git_dir,
            worktree_path,
            branch_ref,
            commit,
        } => {
            validate_revision(commit)?;
            let branch = branch_ref
                .strip_prefix("refs/heads/")
                .filter(|branch| !branch.is_empty())
                .ok_or_else(|| invalid_command("invalid managed Git branch reference"))?;
            validate_ref(branch_ref)?;
            git_dir_args(process, git_dir);
            process
                .arg("worktree")
                .arg("add")
                .arg("-b")
                .arg(branch)
                .arg(git_process_path(worktree_path))
                .arg(commit);
        }
        ManagedGitCommand::WorktreeHead { worktree_path } => {
            process.arg("-C").arg(git_process_path(worktree_path));
            managed_git_config_args(process);
            process.arg("rev-parse").arg("--verify").arg("HEAD");
        }
        ManagedGitCommand::WorktreeBranch { worktree_path } => {
            process.arg("-C").arg(git_process_path(worktree_path));
            managed_git_config_args(process);
            process
                .arg("rev-parse")
                .arg("--symbolic-full-name")
                .arg("HEAD");
        }
        ManagedGitCommand::WorktreeCommonDir { worktree_path } => {
            process.arg("-C").arg(git_process_path(worktree_path));
            managed_git_config_args(process);
            process
                .arg("rev-parse")
                .arg("--path-format=absolute")
                .arg("--git-common-dir");
        }
    }
    Ok(stdin)
}

fn git_dir_args(process: &mut Command, git_dir: &Path) {
    process
        .arg("--no-replace-objects")
        .arg("--git-dir")
        .arg(git_process_path(git_dir));
    managed_git_config_args(process);
}

fn git_process_path(path: &Path) -> Cow<'_, OsStr> {
    #[cfg(windows)]
    {
        let original = path.as_os_str();
        let text = original.to_string_lossy();
        let compatible = pandamux_core::strip_windows_verbatim(&text);
        if compatible.as_str() == text.as_ref() {
            Cow::Borrowed(original)
        } else {
            Cow::Owned(OsString::from(compatible))
        }
    }
    #[cfg(not(windows))]
    {
        Cow::Borrowed(path.as_os_str())
    }
}

pub(crate) fn git_process_path_string(path: &Path) -> String {
    git_process_path(path).to_string_lossy().into_owned()
}

fn managed_git_config_args(process: &mut Command) {
    process
        .arg("-c")
        .arg("core.logAllRefUpdates=false")
        .arg("-c")
        .arg("commit.gpgsign=false")
        .arg("-c")
        .arg("core.hooksPath=")
        .arg("-c")
        .arg("gc.auto=0")
        .arg("-c")
        .arg("maintenance.auto=false")
        .arg("-c")
        .arg("core.longpaths=true")
        .arg("-c")
        .arg("core.fsync=all")
        .arg("-c")
        .arg("core.fsyncMethod=fsync");
}

fn managed_path_is_safe(data_paths: &DataPaths, path: &Path) -> bool {
    if !data_paths.owns(path) {
        return false;
    }
    let Ok(relative) = path.strip_prefix(data_paths.root()) else {
        return false;
    };
    let Ok(root_metadata) = std::fs::symlink_metadata(data_paths.root()) else {
        return false;
    };
    if unsafe_directory_metadata(&root_metadata)
        || !managed_owner_is_safe(
            data_paths.root(),
            &root_metadata,
            data_paths.root(),
            &root_metadata,
        )
    {
        return false;
    }
    let mut current = data_paths.root().to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if unsafe_directory_metadata(&metadata)
                    || !managed_owner_is_safe(
                        data_paths.root(),
                        &root_metadata,
                        &current,
                        &metadata,
                    ) =>
            {
                return false;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return false,
        }
    }
    true
}

fn managed_file_path_is_safe(data_paths: &DataPaths, path: &Path) -> bool {
    if !data_paths.owns(path)
        || path
            .parent()
            .is_none_or(|parent| !managed_path_is_safe(data_paths, parent))
    {
        return false;
    }
    let Ok(root_metadata) = std::fs::symlink_metadata(data_paths.root()) else {
        return false;
    };
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || !managed_owner_is_safe(data_paths.root(), &root_metadata, path, &metadata)
            {
                return false;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
                if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return false;
                }
            }
            true
        }
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

fn managed_owner_is_safe(
    _root_path: &Path,
    root: &std::fs::Metadata,
    _candidate_path: &Path,
    candidate: &std::fs::Metadata,
) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        root.uid() == candidate.uid()
    }
    #[cfg(windows)]
    {
        let _ = (root, candidate);
        if _root_path == _candidate_path {
            crate::windows_security::validate_private_directory(_candidate_path).is_ok()
        } else {
            crate::windows_security::validate_private_descendant(_candidate_path).is_ok()
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (_root_path, root, _candidate_path, candidate);
        true
    }
}

fn unsafe_directory_metadata(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn validate_ref(reference: &str) -> Result<(), ManagedGitError> {
    if !reference.starts_with("refs/")
        || reference.len() > 512
        || reference
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(invalid_command("invalid managed Git reference"));
    }
    Ok(())
}

fn validate_revision(revision: &str) -> Result<(), ManagedGitError> {
    if revision.is_empty()
        || revision.len() > 600
        || revision.starts_with('-')
        || revision.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(invalid_command("invalid managed Git revision"));
    }
    Ok(())
}

fn validate_remote(remote: &str) -> Result<(), ManagedGitError> {
    if remote.trim() != remote
        || remote.is_empty()
        || remote.len() > 4_096
        || remote.starts_with('-')
        || remote.bytes().any(|byte| byte.is_ascii_control())
        || remote.contains("https://")
            && remote.split_once("https://").is_some_and(|(_, rest)| {
                rest.split('/')
                    .next()
                    .is_some_and(|authority| authority.contains('@'))
            })
        || remote.contains("http://")
            && remote.split_once("http://").is_some_and(|(_, rest)| {
                rest.split('/')
                    .next()
                    .is_some_and(|authority| authority.contains('@'))
            })
    {
        return Err(invalid_command("invalid or credential-bearing Git remote"));
    }
    Ok(())
}

fn validate_remote_reference(reference: &str) -> Result<(), ManagedGitError> {
    let base = reference.strip_suffix("^{}").unwrap_or(reference);
    validate_ref(base)
}

fn validate_fetch_source(source: &str) -> Result<(), ManagedGitError> {
    if is_git_object_id(source) {
        return Ok(());
    }
    validate_ref(source)
}

fn is_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn invalid_command(message: &str) -> ManagedGitError {
    ManagedGitError {
        reason: "git_command_invalid",
        safe_message: message.to_string(),
        retryable: false,
    }
}

fn bundle_io_error() -> ManagedGitError {
    ManagedGitError {
        reason: "git_bundle_io_failed",
        safe_message: "The configuration Git bundle could not be read or written".to_string(),
        retryable: true,
    }
}

struct BoundedRead {
    bytes: Vec<u8>,
    truncated: bool,
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> thread::JoinHandle<std::io::Result<BoundedRead>> {
    thread::spawn(move || {
        let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
        let mut buffer = [0_u8; 8192];
        let mut truncated = false;
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let remaining = limit.saturating_sub(bytes.len());
            let kept = remaining.min(read);
            bytes.extend_from_slice(&buffer[..kept]);
            truncated |= kept < read;
        }
        Ok(BoundedRead { bytes, truncated })
    })
}

fn join_reader(
    handle: thread::JoinHandle<std::io::Result<BoundedRead>>,
) -> Result<BoundedRead, ManagedGitError> {
    handle
        .join()
        .map_err(|_| ManagedGitError {
            reason: "git_output_join_failed",
            safe_message: "The managed Git output reader did not join cleanly".to_string(),
            retryable: true,
        })?
        .map_err(|_| ManagedGitError {
            reason: "git_output_read_failed",
            safe_message: "The managed Git output could not be read".to_string(),
            retryable: true,
        })
}

fn safe_git_stderr(
    bytes: &[u8],
    truncated: bool,
    managed_root: &Path,
    remote: Option<&str>,
) -> String {
    let raw = String::from_utf8_lossy(bytes);
    if SecretsPolicy::contains_potential_secret(&raw)
        || raw.contains("https://") && raw.contains('@')
    {
        return "sensitive Git diagnostic was redacted".to_string();
    }
    let mut safe = raw
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect::<String>();
    let root = managed_root.to_string_lossy();
    safe = safe.replace(root.as_ref(), "[managed-path]");
    let alternate_root = if root.contains('\\') {
        root.replace('\\', "/")
    } else {
        root.replace('/', "\\")
    };
    if alternate_root != root {
        safe = safe.replace(&alternate_root, "[managed-path]");
    }
    if let Some(remote) = remote {
        safe = safe.replace(remote, "[remote]");
        let alternate_remote = if remote.contains('\\') {
            remote.replace('\\', "/")
        } else {
            remote.replace('/', "\\")
        };
        if alternate_remote != remote {
            safe = safe.replace(&alternate_remote, "[remote]");
        }
    }
    safe = safe.trim().to_string();
    if truncated {
        const SUFFIX: &str = " [truncated]";
        truncate_utf8(&mut safe, MAX_GIT_STDERR_BYTES - SUFFIX.len());
        safe.push_str(SUFFIX);
    } else {
        truncate_utf8(&mut safe, MAX_GIT_STDERR_BYTES);
    }
    safe
}

fn classify_git_failure(
    command: &ManagedGitCommand,
    stderr: &[u8],
    exit_code: Option<i32>,
) -> (&'static str, bool) {
    if matches!(command, ManagedGitCommand::VerifyCommit { .. }) {
        return ("signature_verification_failed", false);
    }
    if matches!(command, ManagedGitCommand::LsRemote { .. }) && exit_code == Some(2) {
        return ("tracked_ref_not_found", false);
    }
    let diagnostic = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if diagnostic.contains("authentication failed")
        || diagnostic.contains("could not read username")
        || diagnostic.contains("permission denied")
        || diagnostic.contains("access denied")
    {
        return ("instructions_authentication_failed", true);
    }
    if diagnostic.contains("couldn't find remote ref")
        || diagnostic.contains("not our ref")
        || diagnostic.contains("unadvertised object")
    {
        return ("tracked_ref_not_found", false);
    }
    if matches!(
        command,
        ManagedGitCommand::LsRemote { .. } | ManagedGitCommand::Fetch { .. }
    ) && (diagnostic.contains("could not resolve host")
        || diagnostic.contains("connection refused")
        || diagnostic.contains("connection timed out")
        || diagnostic.contains("network is unreachable")
        || diagnostic.contains("does not appear to be a git repository"))
    {
        return ("instructions_remote_unavailable", true);
    }
    ("git_command_failed", false)
}

fn truncate_utf8(value: &mut String, limit: usize) {
    if value.len() <= limit {
        return;
    }
    let mut boundary = limit;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_rejects_paths_outside_the_injected_root() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("paths");
        let runner = ManagedGitRunner::new(paths);
        let error = runner
            .run(ManagedGitCommand::InitBare {
                git_dir: temp.path().join("outside.git"),
            })
            .expect_err("outside path must fail");
        assert_eq!(error.reason, "managed_path_unsafe");
    }

    #[cfg(unix)]
    #[test]
    fn runner_rejects_symlinked_managed_path_components() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("temporary root");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("paths");
        paths.ensure_private_roots().expect("private roots");
        let outside = temp.path().join("outside.git");
        std::fs::create_dir(&outside).expect("outside directory");
        let link = paths.instructions().join("linked.git");
        symlink(&outside, &link).expect("managed symlink");
        let runner = ManagedGitRunner::new(paths);
        let error = runner
            .run(ManagedGitCommand::InitBare { git_dir: link })
            .expect_err("symlinked path must fail");
        assert_eq!(error.reason, "managed_path_unsafe");
        assert!(
            std::fs::read_dir(outside)
                .expect("outside remains readable")
                .next()
                .is_none()
        );
    }

    #[test]
    fn shutdown_prevents_new_children() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("paths");
        paths.ensure_private_roots().expect("private roots");
        let target = paths.instructions().join("cancelled.git");
        let runner = ManagedGitRunner::new(paths);
        runner.shutdown();
        let error = runner
            .run(ManagedGitCommand::InitBare {
                git_dir: target.clone(),
            })
            .expect_err("shutdown runner must reject work");
        assert_eq!(error.reason, "git_command_cancelled");
        assert!(!target.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_runner_initializes_an_inherited_private_path() {
        let temp = tempfile::tempdir().expect("temporary root");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("paths");
        paths.ensure_private_roots().expect("private roots");
        let candidate = paths.root().join(".managed-git-windows-test");
        std::fs::create_dir(&candidate).expect("candidate directory");
        crate::windows_security::validate_private_descendant(&candidate)
            .expect("candidate inherits a private DACL");
        let git_dir = candidate.join("repository.git");
        ManagedGitRunner::new(paths)
            .run(ManagedGitCommand::InitBare {
                git_dir: git_dir.clone(),
            })
            .expect("initialize managed bare repository");
        assert!(git_dir.join("HEAD").is_file());
    }

    #[test]
    fn runner_removes_environment_redirects_and_trace_sinks() {
        let mut process = Command::new("git");
        process
            .env("GIT_OBJECT_DIRECTORY", "/outside/objects")
            .env("GIT_COMMON_DIR", "/outside/common")
            .env("GIT_TRACE2_EVENT", "/outside/trace.json");
        sanitize_git_environment(&mut process);
        let removed = process
            .get_envs()
            .filter_map(|(key, value)| {
                value
                    .is_none()
                    .then_some(key.to_string_lossy().into_owned())
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert!(removed.contains("GIT_OBJECT_DIRECTORY"));
        assert!(removed.contains("GIT_COMMON_DIR"));
        assert!(removed.contains("GIT_TRACE2_EVENT"));
    }

    #[test]
    fn managed_configuration_enforces_durability_and_long_paths() {
        let mut process = Command::new("git");
        managed_git_config_args(&mut process);
        let arguments = process
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["-c", "core.longpaths=true"])
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["-c", "core.fsync=all"])
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["-c", "core.fsyncMethod=fsync"])
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_git_arguments_remove_verbatim_drive_and_unc_prefixes() {
        let drive_path: &OsStr =
            &git_process_path(Path::new(r"\\?\C:\Users\runner\AppData\PandaMUX"));
        assert_eq!(drive_path, OsStr::new(r"C:\Users\runner\AppData\PandaMUX"));
        let unc_path: &OsStr = &git_process_path(Path::new(r"\\?\UNC\server\share\PandaMUX"));
        assert_eq!(unc_path, OsStr::new(r"\\server\share\PandaMUX"));

        let mut process = Command::new("git");
        configure_process(
            &mut process,
            &ManagedGitCommand::InitBare {
                git_dir: PathBuf::from(r"\\?\C:\Users\runner\repository.git"),
            },
        )
        .expect("configure Windows Git init");
        let arguments = process
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments.last().map(String::as_str),
            Some(r"C:\Users\runner\repository.git")
        );
        assert!(
            arguments
                .iter()
                .all(|argument| !argument.starts_with(r"\\?\"))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_git_arguments_preserve_literal_backslash_paths() {
        let path = Path::new(r"\\?\literal-unix-name");
        assert_eq!(git_process_path(path), path.as_os_str());
        assert_eq!(
            git_process_path_string(path),
            r"\\?\literal-unix-name".to_string()
        );
    }

    #[test]
    fn stderr_redaction_is_bounded_and_secret_safe() {
        let root = Path::new("/private/pandamux");
        let secret = format!("fatal: bearer {}", "s".repeat(32));
        assert_eq!(
            safe_git_stderr(secret.as_bytes(), false, root, None),
            "sensitive Git diagnostic was redacted"
        );
        let long = "x".repeat(MAX_GIT_STDERR_BYTES + 100);
        assert!(safe_git_stderr(long.as_bytes(), true, root, None).len() <= MAX_GIT_STDERR_BYTES);
        assert_eq!(
            safe_git_stderr(b"fatal: /private/pandamux/personal/.git", false, root, None,),
            "fatal: [managed-path]/personal/.git"
        );
        assert_eq!(
            safe_git_stderr(
                b"fatal: '/outside/private/repo' is unavailable",
                false,
                root,
                Some("/outside/private/repo"),
            ),
            "fatal: '[remote]' is unavailable"
        );
    }

    #[test]
    fn remote_failures_have_stable_safe_classifications() {
        let temp = tempfile::tempdir().expect("temporary root");
        let command = ManagedGitCommand::LsRemote {
            git_dir: temp.path().join("mirror.git"),
            remote: "/private/remote.git".to_string(),
            references: vec!["refs/heads/main".to_string()],
        };
        assert_eq!(
            classify_git_failure(
                &command,
                b"fatal: Authentication failed for '/private/remote.git'",
                Some(128),
            ),
            ("instructions_authentication_failed", true)
        );
        assert_eq!(
            classify_git_failure(
                &command,
                b"fatal: couldn't find remote ref refs/heads/main",
                Some(128),
            ),
            ("tracked_ref_not_found", false)
        );
    }

    #[cfg(unix)]
    #[test]
    fn timeout_terminates_and_joins_the_owned_child() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temporary root");
        let paths = DataPaths::from_root(temp.path().join("app")).expect("paths");
        paths.ensure_private_roots().expect("private roots");
        let program = temp.path().join("blocking-git");
        std::fs::write(&program, b"#!/bin/sh\nwhile :; do :; done\n").expect("write helper");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))
            .expect("make helper executable");
        let runner = ManagedGitRunner::with_test_process(
            paths.clone(),
            program,
            Duration::from_millis(40),
            Duration::from_millis(40),
        );
        let started = Instant::now();
        let error = runner
            .run(ManagedGitCommand::InitBare {
                git_dir: paths.instructions().join("timeout.git"),
            })
            .expect_err("blocking child must time out");
        assert_eq!(error.reason, "git_command_timed_out");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
