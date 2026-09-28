use pandamux_protocol::{
    GitCommitResult, GitCreatePrResult, GitFileStatus, GitPushResult, GitStatusResult,
};
use std::path::Path;
use std::process::Command;

/// Queries the Git status of the specified directory.
pub fn get_git_status(worktree_dir: &Path) -> Result<GitStatusResult, String> {
    if !crate::checkpoint::is_git_repository(worktree_dir) {
        return Ok(GitStatusResult {
            is_repo: false,
            branch: String::new(),
            tracking_branch: None,
            ahead: 0,
            behind: 0,
            files: Vec::new(),
            drafted_commit_message: String::new(),
        });
    }

    // 1. Current Branch
    let branch_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()
        .map_err(|e| format!("Failed to query git branch: {e}"))?;

    let branch = String::from_utf8_lossy(&branch_out.stdout)
        .trim()
        .to_string();

    // 2. Tracking branch and ahead/behind counts
    let tracking_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("@{u}")
        .output();

    let mut tracking_branch = None;
    let mut ahead = 0;
    let mut behind = 0;

    if let Ok(out) = tracking_out
        && out.status.success()
    {
        let tb = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !tb.is_empty() {
            tracking_branch = Some(tb.clone());

            let count_out = Command::new("git")
                .arg("-C")
                .arg(worktree_dir)
                .arg("rev-list")
                .arg("--left-right")
                .arg("--count")
                .arg(format!("HEAD...{tb}"))
                .output();

            if let Ok(c_out) = count_out
                && c_out.status.success()
            {
                let count_str = String::from_utf8_lossy(&c_out.stdout);
                let parts: Vec<&str> = count_str.split_whitespace().collect();
                if parts.len() >= 2 {
                    ahead = parts[0].parse::<usize>().unwrap_or(0);
                    behind = parts[1].parse::<usize>().unwrap_or(0);
                }
            }
        }
    }

    // 3. File status list
    let status_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("status")
        .arg("--porcelain=v1")
        .arg("-uall")
        .output()
        .map_err(|e| format!("Failed to query git status: {e}"))?;

    let status_str = String::from_utf8_lossy(&status_out.stdout);
    let mut files = Vec::new();

    for line in status_str.lines() {
        if line.len() < 3 {
            continue;
        }
        let code = &line[0..2];
        let path = line[3..].trim();

        let status = if code == "??" {
            "untracked".to_string()
        } else if code.contains('D') {
            "deleted".to_string()
        } else if code.contains('A') {
            "added".to_string()
        } else if code.contains('R') {
            "renamed".to_string()
        } else {
            "modified".to_string()
        };

        // For renames (e.g. "old -> new"), keep the destination
        let final_path = if let Some((_, dest)) = path.split_once(" -> ") {
            dest.to_string()
        } else {
            path.to_string()
        };

        files.push(GitFileStatus {
            path: final_path,
            status,
        });
    }

    // 4. Drafted commit message
    let drafted_commit_message = draft_commit_message(&files);

    Ok(GitStatusResult {
        is_repo: true,
        branch,
        tracking_branch,
        ahead,
        behind,
        files,
        drafted_commit_message,
    })
}

/// Generates a sensible drafted commit message from file change status.
fn draft_commit_message(files: &[GitFileStatus]) -> String {
    if files.is_empty() {
        return "chore: clean working tree".to_string();
    }

    if files.len() == 1 {
        let f = &files[0];
        let filename = Path::new(&f.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&f.path);
        match f.status.as_str() {
            "added" | "untracked" => format!("feat: add {filename}"),
            "deleted" => format!("refactor: remove {filename}"),
            _ => format!("feat: update {filename}"),
        }
    } else if files.len() <= 3 {
        let names: Vec<&str> = files
            .iter()
            .map(|f| {
                Path::new(&f.path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&f.path)
            })
            .collect();
        format!("feat: update {}", names.join(", "))
    } else {
        let first_two: Vec<&str> = files
            .iter()
            .take(2)
            .map(|f| {
                Path::new(&f.path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&f.path)
            })
            .collect();
        let remaining = files.len() - 2;
        format!(
            "feat: update {}, and {} other files",
            first_two.join(", "),
            remaining
        )
    }
}

/// Stages all changes and creates a new Git commit.
pub fn git_commit(worktree_dir: &Path, message: Option<String>) -> Result<GitCommitResult, String> {
    if !crate::checkpoint::is_git_repository(worktree_dir) {
        return Err("Target directory is not a Git repository".to_string());
    }

    // 1. Stage all changes
    let add_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("add")
        .arg("-A")
        .output()
        .map_err(|e| format!("Failed to stage git changes: {e}"))?;

    if !add_out.status.success() {
        let stderr = String::from_utf8_lossy(&add_out.stderr);
        return Err(format!("git add failed: {stderr}"));
    }

    // 2. Count staged files
    let diff_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("diff")
        .arg("--cached")
        .arg("--name-only")
        .output()
        .map_err(|e| format!("Failed to inspect staged changes: {e}"))?;

    let diff_str = String::from_utf8_lossy(&diff_out.stdout);
    let staged_files: Vec<&str> = diff_str.lines().filter(|s| !s.trim().is_empty()).collect();

    if staged_files.is_empty() {
        return Err("No changes staged to commit".to_string());
    }

    // 3. Determine commit message
    let msg = match message {
        Some(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => {
            let status = get_git_status(worktree_dir)?;
            status.drafted_commit_message
        }
    };

    // 4. Run commit
    let commit_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("commit")
        .arg("-m")
        .arg(&msg)
        .output()
        .map_err(|e| format!("Failed to execute git commit: {e}"))?;

    if !commit_out.status.success() {
        let stderr = String::from_utf8_lossy(&commit_out.stderr);
        return Err(format!("git commit failed: {stderr}"));
    }

    // 5. Query new commit hash
    let rev_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .map_err(|e| format!("Failed to read HEAD commit hash: {e}"))?;

    let commit_hash = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();

    Ok(GitCommitResult {
        success: true,
        commit_hash,
        message: msg,
        files_committed: staged_files.len(),
    })
}

/// Pushes commits to the remote Git repository.
pub fn git_push(
    worktree_dir: &Path,
    remote: Option<&str>,
    branch: Option<&str>,
) -> Result<GitPushResult, String> {
    if !crate::checkpoint::is_git_repository(worktree_dir) {
        return Err("Target directory is not a Git repository".to_string());
    }

    let remote_target = remote.unwrap_or("origin");

    let branch_target = match branch {
        Some(b) => b.to_string(),
        None => {
            let out = Command::new("git")
                .arg("-C")
                .arg(worktree_dir)
                .arg("rev-parse")
                .arg("--abbrev-ref")
                .arg("HEAD")
                .output()
                .map_err(|e| format!("Failed to query current branch: {e}"))?;
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
    };

    let push_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("push")
        .arg("-u")
        .arg(remote_target)
        .arg(&branch_target)
        .output()
        .map_err(|e| format!("Failed to execute git push: {e}"))?;

    if !push_out.status.success() {
        let stderr = String::from_utf8_lossy(&push_out.stderr);
        return Err(format!("git push failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&push_out.stdout);
    let stderr = String::from_utf8_lossy(&push_out.stderr);
    let message = if !stdout.trim().is_empty() {
        stdout.trim().to_string()
    } else if !stderr.trim().is_empty() {
        stderr.trim().to_string()
    } else {
        format!("Pushed successfully to {remote_target}/{branch_target}")
    };

    Ok(GitPushResult {
        success: true,
        message,
    })
}

/// Creates a Pull Request using gh CLI if available, or generates a compare URL.
pub fn git_create_pr(
    worktree_dir: &Path,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<GitCreatePrResult, String> {
    if !crate::checkpoint::is_git_repository(worktree_dir) {
        return Err("Target directory is not a Git repository".to_string());
    }

    // 1. Get current branch
    let branch_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()
        .map_err(|e| format!("Failed to query current branch: {e}"))?;

    let branch = String::from_utf8_lossy(&branch_out.stdout)
        .trim()
        .to_string();

    // 2. Default Title from latest commit
    let resolved_title = match title {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            let log_out = Command::new("git")
                .arg("-C")
                .arg(worktree_dir)
                .arg("log")
                .arg("-1")
                .arg("--format=%s")
                .output();
            if let Ok(out) = log_out
                && out.status.success()
            {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            } else {
                format!("PR: {branch}")
            }
        }
    };

    let resolved_body = body.unwrap_or("Created automatically by PandaMUX.");

    // 3. Try GitHub CLI (gh)
    let gh_check = Command::new("gh").arg("--version").output();
    if let Ok(out) = gh_check
        && out.status.success()
    {
        let pr_out = Command::new("gh")
            .current_dir(worktree_dir)
            .arg("pr")
            .arg("create")
            .arg("--title")
            .arg(&resolved_title)
            .arg("--body")
            .arg(resolved_body)
            .output();

        if let Ok(p_out) = pr_out
            && p_out.status.success()
        {
            let url = String::from_utf8_lossy(&p_out.stdout).trim().to_string();
            return Ok(GitCreatePrResult {
                success: true,
                url,
                method: "gh".to_string(),
                message: "Pull request created successfully via GitHub CLI".to_string(),
            });
        }
    }

    // 4. Fallback: Parse remote origin URL and construct web compare URL
    let remote_out = Command::new("git")
        .arg("-C")
        .arg(worktree_dir)
        .arg("config")
        .arg("--get")
        .arg("remote.origin.url")
        .output()
        .map_err(|e| format!("Failed to query remote.origin.url: {e}"))?;

    let remote_url = String::from_utf8_lossy(&remote_out.stdout)
        .trim()
        .to_string();

    let compare_url = parse_github_compare_url(&remote_url, &branch)
        .unwrap_or_else(|| format!("https://github.com?compare={branch}"));

    Ok(GitCreatePrResult {
        success: true,
        url: compare_url,
        method: "compare_url".to_string(),
        message: "GitHub CLI unavailable; generated compare URL for web PR submission".to_string(),
    })
}

/// Parses a Git remote URL into a GitHub compare URL.
fn parse_github_compare_url(remote_url: &str, branch: &str) -> Option<String> {
    let clean = remote_url.trim().trim_end_matches(".git");
    let repo_path = clean
        .strip_prefix("https://github.com/")
        .or_else(|| clean.strip_prefix("git@github.com:"))?;

    Some(format!(
        "https://github.com/{repo_path}/compare/{branch}?expand=1"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_compare_url() {
        let https = "https://github.com/BoardPandas/Pandamux.git";
        let url = parse_github_compare_url(https, "feat-test").unwrap();
        assert_eq!(
            url,
            "https://github.com/BoardPandas/Pandamux/compare/feat-test?expand=1"
        );

        let ssh = "git@github.com:BoardPandas/Pandamux.git";
        let url2 = parse_github_compare_url(ssh, "main").unwrap();
        assert_eq!(
            url2,
            "https://github.com/BoardPandas/Pandamux/compare/main?expand=1"
        );
    }

    #[test]
    fn test_draft_commit_message() {
        let files = vec![
            GitFileStatus {
                path: "src/settings.rs".to_string(),
                status: "modified".to_string(),
            },
            GitFileStatus {
                path: "src/main.rs".to_string(),
                status: "modified".to_string(),
            },
        ];
        let msg = draft_commit_message(&files);
        assert_eq!(msg, "feat: update settings.rs, main.rs");
    }
}
