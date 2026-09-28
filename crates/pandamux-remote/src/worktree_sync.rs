use std::path::Path;
use std::process::Command;

/// Creates a git bundle byte vector containing the commits in `rev_range` (for example, "HEAD~1..HEAD").
pub fn create_worktree_bundle(repo_root: &Path, rev_range: &str) -> Result<Vec<u8>, anyhow::Error> {
    if !repo_root.exists() {
        anyhow::bail!("Repository root does not exist: {}", repo_root.display());
    }

    let temp_bundle =
        std::env::temp_dir().join(format!("pmux-bundle-out-{}.bundle", uuid::Uuid::new_v4()));

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("bundle")
        .arg("create")
        .arg(&temp_bundle)
        .arg(rev_range)
        .output()?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&temp_bundle);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git bundle create failed: {stderr}");
    }

    let bundle_bytes = std::fs::read(&temp_bundle)?;
    let _ = std::fs::remove_file(&temp_bundle);
    Ok(bundle_bytes)
}

/// Verifies and applies a git bundle byte vector into `target_repo` at `target_branch`.
/// Returns the number of commits applied or fetched.
pub fn apply_worktree_bundle(
    target_repo: &Path,
    bundle_bytes: &[u8],
    target_branch: &str,
) -> Result<usize, anyhow::Error> {
    if !target_repo.exists() {
        anyhow::bail!(
            "Target repository does not exist: {}",
            target_repo.display()
        );
    }

    let temp_bundle =
        std::env::temp_dir().join(format!("pmux-bundle-in-{}.bundle", uuid::Uuid::new_v4()));
    std::fs::write(&temp_bundle, bundle_bytes)?;

    // 1. Verify bundle integrity
    let verify_output = Command::new("git")
        .arg("-C")
        .arg(target_repo)
        .arg("bundle")
        .arg("verify")
        .arg(&temp_bundle)
        .output()?;

    if !verify_output.status.success() {
        let _ = std::fs::remove_file(&temp_bundle);
        let stderr = String::from_utf8_lossy(&verify_output.stderr);
        anyhow::bail!("git bundle verify failed: {stderr}");
    }

    let source_ref = {
        let stdout = String::from_utf8_lossy(&verify_output.stdout);
        let mut found = None;
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() == 2
                && parts[0].len() >= 40
                && parts[0].chars().all(|c| c.is_ascii_hexdigit())
            {
                found = Some(parts[1].to_string());
                break;
            }
        }
        found.unwrap_or_else(|| "HEAD".to_string())
    };

    // 2. Fetch from bundle into target branch ref
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
        .output()?;

    let _ = std::fs::remove_file(&temp_bundle);

    if !fetch_output.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_output.stderr);
        anyhow::bail!("git fetch from bundle failed: {stderr}");
    }

    // Count commits on target branch
    let count_output = Command::new("git")
        .arg("-C")
        .arg(target_repo)
        .arg("rev-list")
        .arg("--count")
        .arg(target_branch)
        .output()?;

    let commit_count = if count_output.status.success() {
        String::from_utf8_lossy(&count_output.stdout)
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
    fn test_create_and_apply_git_bundle_round_trip() {
        let origin_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();

        // 1. Initialize origin repository and add commits
        let run = |dir: &Path, args: &[&str]| {
            let res = Command::new("git")
                .current_dir(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(
                res.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&res.stderr)
            );
        };

        run(origin_dir.path(), &["init"]);
        run(origin_dir.path(), &["config", "user.name", "Test User"]);
        run(
            origin_dir.path(),
            &["config", "user.email", "test@example.com"],
        );
        std::fs::write(origin_dir.path().join("file1.txt"), "hello").unwrap();
        run(origin_dir.path(), &["add", "file1.txt"]);
        run(origin_dir.path(), &["commit", "-m", "Initial commit"]);

        std::fs::write(origin_dir.path().join("file2.txt"), "world").unwrap();
        run(origin_dir.path(), &["add", "file2.txt"]);
        run(origin_dir.path(), &["commit", "-m", "Second commit"]);

        // 2. Initialize target repository
        run(target_dir.path(), &["init"]);
        run(target_dir.path(), &["config", "user.name", "Test User"]);
        run(
            target_dir.path(),
            &["config", "user.email", "test@example.com"],
        );

        // 3. Create bundle with all commits from origin
        let bundle_bytes =
            create_worktree_bundle(origin_dir.path(), "HEAD").expect("create bundle");
        assert!(!bundle_bytes.is_empty());

        // 4. Apply bundle into target repository at branch `feature/delta`
        let applied_count =
            apply_worktree_bundle(target_dir.path(), &bundle_bytes, "feature/delta")
                .expect("apply bundle");
        assert_eq!(applied_count, 2);

        // 5. Verify target repository has the branch
        let branch_res = Command::new("git")
            .current_dir(target_dir.path())
            .args(["branch", "--list", "feature/delta"])
            .output()
            .unwrap();
        assert!(branch_res.status.success());
        assert!(String::from_utf8_lossy(&branch_res.stdout).contains("feature/delta"));
    }
}
