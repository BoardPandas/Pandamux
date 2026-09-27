use pandamux_core::{FileChangeKind, ThreadId, TurnId};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Phase of turn execution at which a checkpoint was recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointPhase {
    Before,
    After,
}

impl CheckpointPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
        }
    }
}

/// Statistics for a single changed file between checkpoints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFileStat {
    pub path: String,
    pub kind: FileChangeKind,
    pub additions: usize,
    pub deletions: usize,
}

/// Checks whether a given directory is located within a valid git work tree.
pub fn is_git_repository(worktree_dir: &Path) -> bool {
    if !worktree_dir.exists() {
        return false;
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output();

    matches!(
        output,
        Ok(out) if out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
    )
}

/// Captures a repository checkpoint before or after a turn using a temporary git index.
///
/// Uses standard git plumbing (read-tree, add -A, write-tree, commit-tree, update-ref)
/// under an isolated `GIT_INDEX_FILE` to avoid touching or modifying the user's `.git/index`.
/// If `worktree_dir` is not a git repository, falls back gracefully by returning `Ok(None)`.
pub fn create_checkpoint(
    worktree_dir: &Path,
    thread_id: &ThreadId,
    turn_id: &TurnId,
    phase: CheckpointPhase,
) -> Result<Option<String>, String> {
    if !is_git_repository(worktree_dir) {
        return Ok(None);
    }

    // Resolve git directory for temporary index location
    let git_dir_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("rev-parse")
        .arg("--git-dir")
        .output()
        .map_err(|e| format!("Failed to resolve git-dir: {e}"))?;

    if !git_dir_out.status.success() {
        return Ok(None);
    }

    let git_dir_str = String::from_utf8_lossy(&git_dir_out.stdout)
        .trim()
        .to_string();
    let git_dir = if Path::new(&git_dir_str).is_absolute() {
        PathBuf::from(git_dir_str)
    } else {
        worktree_dir.join(git_dir_str)
    };

    let temp_index = git_dir.join(format!(
        "pandamux_idx_{}_{}_{}",
        thread_id.as_str(),
        turn_id.as_str(),
        phase.as_str()
    ));

    // Ensure stale temp index from a previous crash is removed
    let _ = std::fs::remove_file(&temp_index);

    // If HEAD exists, populate the temporary index with current HEAD tree
    let _ = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .env("GIT_INDEX_FILE", &temp_index)
        .arg("read-tree")
        .arg("HEAD")
        .output();

    // Stage current working tree state into temporary index (respects .gitignore)
    let add_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .env("GIT_INDEX_FILE", &temp_index)
        .arg("add")
        .arg("-A")
        .output()
        .map_err(|e| format!("git add to temp index failed: {e}"))?;

    if !add_out.status.success() {
        let _ = std::fs::remove_file(&temp_index);
        let err = String::from_utf8_lossy(&add_out.stderr);
        return Err(format!("git add failed: {err}"));
    }

    // Write tree from temporary index
    let write_tree_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .env("GIT_INDEX_FILE", &temp_index)
        .arg("write-tree")
        .output()
        .map_err(|e| format!("git write-tree failed: {e}"))?;

    let _ = std::fs::remove_file(&temp_index);

    if !write_tree_out.status.success() {
        let err = String::from_utf8_lossy(&write_tree_out.stderr);
        return Err(format!("git write-tree failed: {err}"));
    }

    let tree_sha = String::from_utf8_lossy(&write_tree_out.stdout)
        .trim()
        .to_string();

    // Determine parent commit if HEAD exists
    let head_rev_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("rev-parse")
        .arg("--verify")
        .arg("HEAD")
        .output();

    let parent_commit = match head_rev_out {
        Ok(out) if out.status.success() => {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if sha.is_empty() { None } else { Some(sha) }
        }
        _ => None,
    };

    // Commit tree to create a standalone commit object
    let message = format!(
        "PandaMUX checkpoint {} for turn {} (thread {})",
        phase.as_str(),
        turn_id.as_str(),
        thread_id.as_str()
    );

    let mut commit_cmd = Command::new("git");
    commit_cmd
        .arg("-C")
        .arg(worktree_dir)
        .env("GIT_AUTHOR_NAME", "PandaMUX")
        .env("GIT_AUTHOR_EMAIL", "pandamux@local")
        .env("GIT_COMMITTER_NAME", "PandaMUX")
        .env("GIT_COMMITTER_EMAIL", "pandamux@local")
        .arg("commit-tree")
        .arg(&tree_sha);

    if let Some(parent) = &parent_commit {
        commit_cmd.arg("-p").arg(parent);
    }

    commit_cmd.arg("-m").arg(&message);

    let commit_out = commit_cmd
        .output()
        .map_err(|e| format!("git commit-tree failed: {e}"))?;

    if !commit_out.status.success() {
        let err = String::from_utf8_lossy(&commit_out.stderr);
        return Err(format!("git commit-tree failed: {err}"));
    }

    let commit_sha = String::from_utf8_lossy(&commit_out.stdout)
        .trim()
        .to_string();

    // Update hidden ref: refs/pandamux/checkpoints/<thread>/<turn>/<phase>
    let ref_name = format!(
        "refs/pandamux/checkpoints/{}/{}/{}",
        thread_id.as_str(),
        turn_id.as_str(),
        phase.as_str()
    );

    let update_ref_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("update-ref")
        .arg(&ref_name)
        .arg(&commit_sha)
        .output()
        .map_err(|e| format!("git update-ref failed: {e}"))?;

    if !update_ref_out.status.success() {
        let err = String::from_utf8_lossy(&update_ref_out.stderr);
        return Err(format!("git update-ref failed: {err}"));
    }

    Ok(Some(ref_name))
}

/// Computes file diff statistics between two checkpoint refs.
pub fn compute_changed_files(
    worktree_dir: &Path,
    before_ref: &str,
    after_ref: &str,
) -> Result<Vec<ChangedFileStat>, String> {
    if !is_git_repository(worktree_dir) || before_ref == after_ref {
        return Ok(Vec::new());
    }

    // 1. Fetch numstat for additions/deletions
    let numstat_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("diff")
        .arg("--numstat")
        .arg(before_ref)
        .arg(after_ref)
        .output()
        .map_err(|e| format!("git diff --numstat failed: {e}"))?;

    if !numstat_out.status.success() {
        let err = String::from_utf8_lossy(&numstat_out.stderr);
        return Err(format!("git diff --numstat failed: {err}"));
    }

    let numstat_str = String::from_utf8_lossy(&numstat_out.stdout);

    // 2. Fetch name-status for modification classification (A, M, D)
    let namestat_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("diff")
        .arg("--name-status")
        .arg(before_ref)
        .arg(after_ref)
        .output()
        .map_err(|e| format!("git diff --name-status failed: {e}"))?;

    let namestat_str = String::from_utf8_lossy(&namestat_out.stdout);

    let mut kind_map = std::collections::HashMap::new();
    for line in namestat_str.lines() {
        let mut parts = line.split('\t');
        if let (Some(status_code), Some(path)) = (parts.next(), parts.next()) {
            let kind = match status_code.chars().next().unwrap_or('M') {
                'A' => FileChangeKind::Created,
                'D' => FileChangeKind::Deleted,
                _ => FileChangeKind::Modified,
            };
            kind_map.insert(path.trim().to_string(), kind);
        }
    }

    let mut result = Vec::new();
    for line in numstat_str.lines() {
        let mut parts = line.split('\t');
        if let (Some(adds_str), Some(dels_str), Some(path_str)) =
            (parts.next(), parts.next(), parts.next())
        {
            let path = path_str.trim().to_string();
            let additions = adds_str.parse::<usize>().unwrap_or(0);
            let deletions = dels_str.parse::<usize>().unwrap_or(0);
            let kind = kind_map
                .get(&path)
                .copied()
                .unwrap_or(FileChangeKind::Modified);

            result.push(ChangedFileStat {
                path,
                kind,
                additions,
                deletions,
            });
        }
    }

    Ok(result)
}

/// Computes raw unified diff patch between two checkpoint refs, optionally filtered by file path.
pub fn compute_checkpoint_patch(
    worktree_dir: &Path,
    before_ref: &str,
    after_ref: &str,
    file_path: Option<&str>,
) -> Result<String, String> {
    if !is_git_repository(worktree_dir) || before_ref == after_ref {
        return Ok(String::new());
    }

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(worktree_dir)
        .arg("diff")
        .arg("-U3")
        .arg(before_ref)
        .arg(after_ref);

    if let Some(file) = file_path {
        cmd.arg("--").arg(file);
    }

    let out = cmd
        .output()
        .map_err(|e| format!("git diff patch failed: {e}"))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("git diff patch failed: {err}"));
    }

    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Prunes old checkpoint refs for a thread, keeping only the most recent N checkpoints.
pub fn prune_old_checkpoints(
    worktree_dir: &Path,
    thread_id: &ThreadId,
    keep_last: usize,
) -> Result<usize, String> {
    if !is_git_repository(worktree_dir) {
        return Ok(0);
    }

    let prefix = format!("refs/pandamux/checkpoints/{}/", thread_id.as_str());
    let list_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("for-each-ref")
        .arg("--format=%(refname)")
        .arg(&prefix)
        .output()
        .map_err(|e| format!("git for-each-ref failed: {e}"))?;

    if !list_out.status.success() {
        return Ok(0);
    }

    let refs_str = String::from_utf8_lossy(&list_out.stdout);
    let all_refs: Vec<&str> = refs_str.lines().filter(|l| !l.trim().is_empty()).collect();

    if all_refs.len() <= keep_last {
        return Ok(0);
    }

    let prune_count = all_refs.len() - keep_last;
    let mut deleted = 0;

    for ref_name in &all_refs[..prune_count] {
        let del_out = Command::new("git")
            .arg("-C")
            .arg(worktree_dir)
            .arg("update-ref")
            .arg("-d")
            .arg(ref_name)
            .output();

        if let Ok(out) = del_out
            && out.status.success()
        {
            deleted += 1;
        }
    }

    Ok(deleted)
}

/// Prunes all checkpoint refs associated with a deleted or archived thread.
pub fn prune_thread_checkpoints(
    worktree_dir: &Path,
    thread_id: &ThreadId,
) -> Result<usize, String> {
    prune_old_checkpoints(worktree_dir, thread_id, 0)
}

/// Restores the working tree and index to a specific checkpoint ref.
pub fn rollback_to_checkpoint(worktree_dir: &Path, checkpoint_ref: &str) -> Result<(), String> {
    if !is_git_repository(worktree_dir) {
        return Err("Cannot rollback: target directory is not a git repository".to_string());
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("read-tree")
        .arg("-u")
        .arg("--reset")
        .arg(checkpoint_ref)
        .output()
        .map_err(|e| format!("git read-tree rollback failed: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git rollback failed: {err}"));
    }

    // Clean untracked files and directories introduced after the checkpoint
    let clean_output = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("clean")
        .arg("-f")
        .arg("-d")
        .output()
        .map_err(|e| format!("git clean rollback failed: {e}"))?;

    if !clean_output.status.success() {
        let err = String::from_utf8_lossy(&clean_output.stderr);
        return Err(format!("git clean failed: {err}"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("pandamux_cp_test_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("create test dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn create_temp_git_repo() -> TestDir {
        let temp = TestDir::new();
        let path = temp.path();

        let _ = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("init")
            .output()
            .expect("git init");
        let _ = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("config")
            .arg("user.name")
            .arg("Test")
            .output();
        let _ = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("config")
            .arg("user.email")
            .arg("test@local")
            .output();

        std::fs::write(path.join("README.md"), "# Initial\n").expect("write readme");
        let _ = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("add")
            .arg(".")
            .output();
        let _ = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("commit")
            .arg("-m")
            .arg("init")
            .output();

        temp
    }

    #[test]
    fn test_is_git_repository() {
        let repo = create_temp_git_repo();
        assert!(is_git_repository(repo.path()));

        let non_git = TestDir::new();
        assert!(!is_git_repository(non_git.path()));
    }

    #[test]
    fn test_create_checkpoint_and_diff() {
        let repo = create_temp_git_repo();
        let path = repo.path();
        let thread_id = ThreadId::from("thread-cp-test");
        let turn_id = TurnId::from("turn-cp-test");

        // 1. Create before checkpoint
        let cp_before = create_checkpoint(path, &thread_id, &turn_id, CheckpointPhase::Before)
            .expect("checkpoint created")
            .expect("is git repo");
        assert!(
            cp_before.starts_with("refs/pandamux/checkpoints/thread-cp-test/turn-cp-test/before")
        );

        // 2. Modify files
        std::fs::write(path.join("README.md"), "# Initial\nNew line added\n").expect("write");
        std::fs::write(path.join("hello.rs"), "fn main() {}\n").expect("write hello");

        // 3. Create after checkpoint
        let cp_after = create_checkpoint(path, &thread_id, &turn_id, CheckpointPhase::After)
            .expect("after checkpoint created")
            .expect("is git repo");
        assert!(
            cp_after.starts_with("refs/pandamux/checkpoints/thread-cp-test/turn-cp-test/after")
        );

        // 4. Compute diff between checkpoints
        let changes = compute_changed_files(path, &cp_before, &cp_after).expect("compute changes");
        assert_eq!(changes.len(), 2);

        let readme_change = changes
            .iter()
            .find(|c| c.path == "README.md")
            .expect("readme changed");
        assert_eq!(readme_change.kind, FileChangeKind::Modified);
        assert_eq!(readme_change.additions, 1);

        let hello_change = changes
            .iter()
            .find(|c| c.path == "hello.rs")
            .expect("hello changed");
        assert_eq!(hello_change.kind, FileChangeKind::Created);

        // 5. Test rollback to before checkpoint
        rollback_to_checkpoint(path, &cp_before).expect("rollback");
        assert_eq!(
            std::fs::read_to_string(path.join("README.md")).unwrap(),
            "# Initial\n"
        );
        assert!(!path.join("hello.rs").exists());

        // 6. Test prune checkpoints
        let pruned = prune_thread_checkpoints(path, &thread_id).expect("prune");
        assert_eq!(pruned, 2);
    }

    #[test]
    fn test_non_git_fallback() {
        let non_git = TestDir::new();
        let thread_id = ThreadId::from("thread-non-git");
        let turn_id = TurnId::from("turn-non-git");

        let cp = create_checkpoint(
            non_git.path(),
            &thread_id,
            &turn_id,
            CheckpointPhase::Before,
        )
        .expect("checkpoint call succeeds on non-git");
        assert_eq!(cp, None);

        let changes = compute_changed_files(non_git.path(), "ref-a", "ref-b")
            .expect("diff succeeds on non-git");
        assert!(changes.is_empty());

        let pruned = prune_thread_checkpoints(non_git.path(), &thread_id)
            .expect("prune succeeds on non-git");
        assert_eq!(pruned, 0);
    }
}
