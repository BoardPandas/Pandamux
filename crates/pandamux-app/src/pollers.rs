//! Background pollers that feed the status bar: git branch/ahead-count and
//! listening dev ports. Both run off the Iced timer via `Task::perform`, using
//! async `tokio` I/O only (no blocking `std::fs`/process calls on the runtime,
//! per the LL-G Tokio gotcha).
//!
//! Scope: git is polled in the focused session's working directory when it is
//! known (from shell-integration OSC / `report_pwd` reporting, see
//! `pandamux-term::cwd`), falling back to the process working directory; ports
//! are scanned on localhost.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

/// A single poll's result, applied to the chrome status bar.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PollResult {
    pub git_branch: Option<String>,
    pub git_ahead: u32,
    pub ports: Vec<u16>,
}

/// Common local dev-server ports to probe.
const CANDIDATE_PORTS: &[u16] = &[
    3000, 3001, 4000, 4200, 5000, 5173, 5199, 8000, 8080, 8081, 9000,
];

/// Run git + port polls concurrently and combine them. `cwd` is the focused
/// session's directory when known; otherwise the process working directory.
pub async fn poll_all(cwd: Option<PathBuf>) -> PollResult {
    let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let (git, ports) = tokio::join!(poll_git(cwd), poll_ports());
    let (git_branch, git_ahead) = git.unwrap_or((None, 0));
    PollResult {
        git_branch,
        git_ahead,
        ports,
    }
}

async fn poll_git(cwd: PathBuf) -> Option<(Option<String>, u32)> {
    let branch = run_git(&cwd, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    let branch = branch.trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        return Some((None, 0));
    }
    let ahead = run_git(&cwd, &["rev-list", "--count", "@{u}..HEAD"])
        .await
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(0);
    Some((Some(branch), ahead))
}

async fn run_git(cwd: &PathBuf, args: &[&str]) -> Option<String> {
    let mut command = tokio::process::Command::new("git");
    command.args(args).current_dir(cwd);
    // pandamux.exe ships as a GUI-subsystem binary (no inherited console), so
    // each console-subsystem child (git.exe) would otherwise pop a fresh console
    // window on every poll and steal keyboard focus. CREATE_NO_WINDOW suppresses
    // it. 0x0800_0000 == CREATE_NO_WINDOW.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = command.output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn poll_ports() -> Vec<u16> {
    let mut open = Vec::new();
    for &port in CANDIDATE_PORTS {
        let connect = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port));
        if let Ok(Ok(_stream)) = tokio::time::timeout(Duration::from_millis(120), connect).await {
            open.push(port);
        }
    }
    open
}
