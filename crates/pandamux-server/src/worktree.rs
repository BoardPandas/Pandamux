use std::path::{Path, PathBuf};
use std::process::Command;
use pandamux_core::{ThreadId, WorktreeRef};

/// Creates a new git worktree for a thread at the specified target directory.
pub fn create_git_worktree(
    repo_root: &Path,
    branch: &str,
    base_ref: &str,
    worktree_dir: &Path,
) -> Result<WorktreeRef, String> {
    if !repo_root.exists() {
        return Err(format!("Repository root does not exist: {}", repo_root.display()));
    }

    if let Some(parent) = worktree_dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create parent directory for worktree: {e}"))?;
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("worktree")
        .arg("add")
        .arg("-b")
        .arg(branch)
        .arg(worktree_dir)
        .arg(base_ref)
        .output()
        .map_err(|e| format!("Failed to execute git worktree add: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree add failed: {stderr}"));
    }

    Ok(WorktreeRef {
        path: worktree_dir.to_string_lossy().to_string(),
        branch: branch.to_string(),
        base_ref: base_ref.to_string(),
    })
}

/// Removes a git worktree previously created for a thread.
pub fn remove_git_worktree(repo_root: &Path, worktree_dir: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(worktree_dir)
        .output()
        .map_err(|e| format!("Failed to execute git worktree remove: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree remove failed: {stderr}"));
    }

    Ok(())
}

/// Generates a standardized path for a thread's isolated worktree.
pub fn default_thread_worktree_path(repo_root: &Path, thread_id: &ThreadId) -> PathBuf {
    repo_root
        .join(".pandamux")
        .join("worktrees")
        .join(thread_id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_thread_worktree_path() {
        let root = Path::new("/workspace/project");
        let id = ThreadId::from("thread-wt-123");
        let path = default_thread_worktree_path(root, &id);
        assert!(path.ends_with(".pandamux/worktrees/thread-wt-123"));
    }
}
