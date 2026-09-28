use pandamux_core::{ThreadId, WorktreeRef};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Creates a new git worktree for a thread at the specified target directory.
pub fn create_git_worktree(
    repo_root: &Path,
    branch: &str,
    base_ref: &str,
    worktree_dir: &Path,
) -> Result<WorktreeRef, String> {
    if !repo_root.exists() {
        return Err(format!(
            "Repository root does not exist: {}",
            repo_root.display()
        ));
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

/// Creates a git bundle byte vector containing the commits in `rev_range`.
pub fn create_git_bundle(repo_root: &Path, rev_range: &str) -> Result<Vec<u8>, String> {
    if !repo_root.exists() {
        return Err(format!(
            "Repository root does not exist: {}",
            repo_root.display()
        ));
    }

    let temp_bundle = std::env::temp_dir().join(format!(
        "pmux-srv-bundle-out-{}.bundle",
        uuid::Uuid::new_v4()
    ));

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("bundle")
        .arg("create")
        .arg(&temp_bundle)
        .arg(rev_range)
        .output()
        .map_err(|e| format!("Failed to execute git bundle create: {e}"))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&temp_bundle);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git bundle create failed: {stderr}"));
    }

    let bundle_bytes = std::fs::read(&temp_bundle)
        .map_err(|e| format!("Failed to read generated bundle file: {e}"))?;
    let _ = std::fs::remove_file(&temp_bundle);
    Ok(bundle_bytes)
}

/// Verifies a git bundle byte vector against a repository, returning the source ref name.
pub fn verify_git_bundle(repo_root: &Path, bundle_bytes: &[u8]) -> Result<String, String> {
    if !repo_root.exists() {
        return Err(format!(
            "Repository root does not exist: {}",
            repo_root.display()
        ));
    }

    let temp_bundle = std::env::temp_dir().join(format!(
        "pmux-srv-bundle-ver-{}.bundle",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&temp_bundle, bundle_bytes)
        .map_err(|e| format!("Failed to write temporary bundle file: {e}"))?;

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("bundle")
        .arg("verify")
        .arg(&temp_bundle)
        .output()
        .map_err(|e| format!("Failed to execute git bundle verify: {e}"))?;

    let _ = std::fs::remove_file(&temp_bundle);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git bundle verify failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 2
            && parts[0].len() >= 40
            && parts[0].chars().all(|c| c.is_ascii_hexdigit())
        {
            return Ok(parts[1].to_string());
        }
    }

    Ok("HEAD".to_string())
}

/// Applies a git bundle byte vector into `target_repo` at `target_branch`.
pub fn apply_git_bundle(
    target_repo: &Path,
    bundle_bytes: &[u8],
    target_branch: &str,
) -> Result<usize, String> {
    let source_ref = verify_git_bundle(target_repo, bundle_bytes)?;

    let temp_bundle = std::env::temp_dir().join(format!(
        "pmux-srv-bundle-in-{}.bundle",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&temp_bundle, bundle_bytes)
        .map_err(|e| format!("Failed to write temporary bundle file: {e}"))?;

    let target_ref = if target_branch.starts_with("refs/") {
        target_branch.to_string()
    } else {
        format!("refs/heads/{target_branch}")
    };
    let refspec = format!("{source_ref}:{target_ref}");
    let fetch_output = Command::new("git")
        .arg("-C")
        .arg(target_repo)
        .arg("fetch")
        .arg(&temp_bundle)
        .arg(&refspec)
        .output()
        .map_err(|e| format!("Failed to execute git fetch: {e}"))?;

    let _ = std::fs::remove_file(&temp_bundle);

    if !fetch_output.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_output.stderr);
        return Err(format!("git fetch from bundle failed: {stderr}"));
    }

    let count_output = Command::new("git")
        .arg("-C")
        .arg(target_repo)
        .arg("rev-list")
        .arg("--count")
        .arg(target_branch)
        .output();

    let commit_count = if let Ok(out) = count_output
        && out.status.success()
    {
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<usize>()
            .unwrap_or(1)
    } else {
        1
    };

    Ok(commit_count)
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

    #[test]
    fn test_git_bundle_create_and_apply() {
        let origin = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();

        let run = |dir: &Path, args: &[&str]| {
            let res = Command::new("git")
                .current_dir(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(res.status.success());
        };

        run(origin.path(), &["init"]);
        run(origin.path(), &["config", "user.name", "Test"]);
        run(origin.path(), &["config", "user.email", "test@test.com"]);
        std::fs::write(origin.path().join("a.txt"), "hello").unwrap();
        run(origin.path(), &["add", "a.txt"]);
        run(origin.path(), &["commit", "-m", "initial"]);

        run(target.path(), &["init"]);
        run(target.path(), &["config", "user.name", "Test"]);
        run(target.path(), &["config", "user.email", "test@test.com"]);

        let bytes = create_git_bundle(origin.path(), "HEAD").unwrap();
        assert!(!bytes.is_empty());

        let count = apply_git_bundle(target.path(), &bytes, "synced-branch").unwrap();
        assert_eq!(count, 1);
    }
}
