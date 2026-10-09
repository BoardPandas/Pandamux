use crate::DataPaths;
use pandamux_core::SecretsPolicy;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub const MAX_GIT_STDERR_BYTES: usize = 16 * 1024;
pub const MAX_GIT_TREE_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
pub const LOCAL_GIT_TIMEOUT: Duration = Duration::from_secs(30);

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
            | Self::CatFile { git_dir, .. } => git_dir,
        }
    }

    fn stdout_limit(&self) -> usize {
        match self {
            Self::ListTree { .. } => MAX_GIT_TREE_OUTPUT_BYTES,
            Self::CatFile { max_bytes, .. } => *max_bytes,
            _ => 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedGitOutput {
    pub stdout: Vec<u8>,
    pub safe_stderr: String,
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
}

impl ManagedGitRunner {
    pub fn new(data_paths: DataPaths) -> Self {
        Self {
            data_paths,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    pub fn run(&self, command: ManagedGitCommand) -> Result<ManagedGitOutput, ManagedGitError> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(ManagedGitError {
                reason: "git_command_cancelled",
                safe_message: "The managed Git command was cancelled during shutdown".to_string(),
                retryable: true,
            });
        }
        if !managed_path_is_safe(&self.data_paths, command.git_dir()) {
            return Err(ManagedGitError {
                reason: "managed_path_unsafe",
                safe_message: "The managed Git path is outside the app-data root".to_string(),
                retryable: false,
            });
        }

        let stdout_limit = command.stdout_limit();
        let mut process = Command::new("git");
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
        let timeout = LOCAL_GIT_TIMEOUT;
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
        let safe_stderr = safe_git_stderr(&stderr.bytes, stderr.truncated, self.data_paths.root());
        if status.as_ref().is_none_or(|status| !status.success()) {
            return Err(ManagedGitError {
                reason: "git_command_failed",
                safe_message: if safe_stderr.is_empty() {
                    "The managed Git command failed".to_string()
                } else {
                    format!("The managed Git command failed: {safe_stderr}")
                },
                retryable: false,
            });
        }
        Ok(ManagedGitOutput {
            stdout: stdout.bytes,
            safe_stderr,
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
            process.arg("init").arg("--bare").arg(git_dir);
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
    }
    Ok(stdin)
}

fn git_dir_args(process: &mut Command, git_dir: &Path) {
    process
        .arg("--no-replace-objects")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("-c")
        .arg("core.logAllRefUpdates=false")
        .arg("-c")
        .arg("commit.gpgsign=false")
        .arg("-c")
        .arg("core.hooksPath=")
        .arg("-c")
        .arg("gc.auto=0")
        .arg("-c")
        .arg("maintenance.auto=false");
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
    if unsafe_directory_metadata(&root_metadata) {
        return false;
    }
    let mut current = data_paths.root().to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if unsafe_directory_metadata(&metadata)
                    || !same_managed_owner(&root_metadata, &metadata) =>
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

fn same_managed_owner(root: &std::fs::Metadata, candidate: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        root.uid() == candidate.uid()
    }
    #[cfg(not(unix))]
    {
        let _ = (root, candidate);
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

fn safe_git_stderr(bytes: &[u8], truncated: bool, managed_root: &Path) -> String {
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
    fn stderr_redaction_is_bounded_and_secret_safe() {
        let root = Path::new("/private/pandamux");
        let secret = format!("fatal: bearer {}", "s".repeat(32));
        assert_eq!(
            safe_git_stderr(secret.as_bytes(), false, root),
            "sensitive Git diagnostic was redacted"
        );
        let long = "x".repeat(MAX_GIT_STDERR_BYTES + 100);
        assert!(safe_git_stderr(long.as_bytes(), true, root).len() <= MAX_GIT_STDERR_BYTES);
        assert_eq!(
            safe_git_stderr(b"fatal: /private/pandamux/personal/.git", false, root),
            "fatal: [managed-path]/personal/.git"
        );
    }
}
