//! PTY lifecycle helpers ported from the Electron `pty-manager.ts` semantics:
//! shell resolution, shell-type classification, ConPTY-friendly write chunking,
//! DA1 query interception, and POSIX/WSL cwd detection. The pure logic here is
//! unit-tested; the effectful pieces (spawn, tree-kill) live in [`crate::session`].

use std::path::Path;
use std::process::{Command, Stdio};

/// Shell family, used to pick launch args and integration scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellType {
    PowerShell,
    Cmd,
    Wsl,
    Unknown,
}

/// Classify a shell command string into a [`ShellType`].
pub fn shell_type(shell: &str) -> ShellType {
    let lower = shell.to_ascii_lowercase();
    if lower.contains("pwsh") || lower.contains("powershell") {
        ShellType::PowerShell
    } else if lower.contains("cmd") {
        ShellType::Cmd
    } else if lower.contains("wsl") {
        ShellType::Wsl
    } else {
        ShellType::Unknown
    }
}

#[cfg(windows)]
const DEFAULT_CANDIDATES: &[&str] = &["pwsh.exe", "powershell.exe", "cmd.exe"];
#[cfg(windows)]
const FALLBACK_SHELL: &str = "cmd.exe";
#[cfg(not(windows))]
const FALLBACK_SHELL: &str = "/bin/sh";

/// Resolve the shell to spawn, falling back through pwsh -> powershell -> cmd on
/// Windows (or `$SHELL` -> `/bin/sh` elsewhere) when `preferred` is missing or
/// not found on PATH.
pub fn resolve_shell(preferred: Option<&str>) -> String {
    resolve_shell_with(preferred, &default_candidates(), is_shell_available)
}

fn resolve_shell_with(
    preferred: Option<&str>,
    candidates: &[String],
    available: impl Fn(&str) -> bool,
) -> String {
    if let Some(preferred) = preferred
        && !preferred.is_empty()
        && available(preferred)
    {
        return preferred.to_string();
    }
    for candidate in candidates {
        if available(candidate) {
            return candidate.clone();
        }
    }
    FALLBACK_SHELL.to_string()
}

#[cfg(windows)]
fn default_candidates() -> Vec<String> {
    DEFAULT_CANDIDATES.iter().map(|s| s.to_string()).collect()
}

#[cfg(not(windows))]
fn default_candidates() -> Vec<String> {
    vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())]
}

/// Whether a shell executable can be found. Absolute paths are checked on disk;
/// bare names are resolved via `where` (Windows) or `which` (Unix).
fn is_shell_available(shell: &str) -> bool {
    if shell.is_empty() {
        return false;
    }
    if Path::new(shell).is_absolute() {
        return Path::new(shell).exists();
    }
    let locator = if cfg!(windows) { "where" } else { "which" };
    Command::new(locator)
        .arg(shell)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Split `data` into ConPTY-friendly chunks of at most `chunk_size` bytes.
/// ConPTY's input pipe silently drops bytes when a single write outruns the
/// foreground process, so long pastes must be chunked (the Electron path used
/// 1 KB). Returns an empty vec for empty input.
pub fn chunk_write(data: &[u8], chunk_size: usize) -> Vec<&[u8]> {
    if data.is_empty() {
        return Vec::new();
    }
    if chunk_size == 0 {
        return vec![data];
    }
    data.chunks(chunk_size).collect()
}

/// Threshold below which a write bypasses chunking entirely (single keystrokes,
/// control sequences, short responses).
pub const CHUNK_THRESHOLD: usize = 1024;
/// Per-chunk size for long writes.
pub const CHUNK_SIZE: usize = 1024;

/// The DA1 (Primary Device Attributes) reply we answer in-process so oh-my-posh
/// / PSReadLine never stall or leak the reply onto the prompt.
pub const DA1_REPLY: &[u8] = b"\x1b[?62;4;9;22c";

/// True if `bytes` contains a DA1 query (`ESC [ <digits>? c`), and NOT a reply
/// or DA2/DA3 form (which carry a `?`, `>`, or `=` private marker after `[`).
pub fn contains_da1_query(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            if j < bytes.len() && matches!(bytes[j], b'?' | b'>' | b'=') {
                i += 1;
                continue;
            }
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'c' {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// The Cursor Position Report reply for a CPR query (`ESC [ 6 n`).
pub const CPR_REPLY: &[u8] = b"\x1b[1;1R";

/// True if `bytes` contains a CPR query (`ESC [ 6 n`).
pub fn contains_cpr_query(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|window| window == b"\x1b[6n")
}

/// A POSIX/WSL path (e.g. `/home/user` restored from a saved session) is never a
/// valid Win32 process working dir and makes spawn fail with error 267. Win32
/// paths are drive-rooted (`C:\...`) or UNC (`\\server\...`); a single leading
/// forward slash means POSIX.
pub fn is_posix_path(path: &str) -> bool {
    path.starts_with('/') && !path.starts_with("//")
}

/// Translate a requested cwd into one safe to hand a Win32 `spawn`. A POSIX/WSL
/// cwd is replaced by `home` (the Windows home dir) because it would fail spawn
/// error 267; WSL itself is still opened at the POSIX path via `--cd` elsewhere.
/// Returns `None` when no cwd was requested.
pub fn win32_spawn_cwd(cwd: Option<&str>, home: &str) -> Option<String> {
    let cwd = cwd?;
    if is_posix_path(cwd) {
        Some(home.to_string())
    } else {
        Some(cwd.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_shell_families() {
        assert_eq!(shell_type("pwsh.exe"), ShellType::PowerShell);
        assert_eq!(
            shell_type("C:/Windows/System32/powershell.exe"),
            ShellType::PowerShell
        );
        assert_eq!(shell_type("cmd.exe"), ShellType::Cmd);
        assert_eq!(shell_type("wsl.exe"), ShellType::Wsl);
        assert_eq!(shell_type("/bin/bash"), ShellType::Unknown);
    }

    #[test]
    fn prefers_available_preferred_shell() {
        let candidates = vec!["pwsh.exe".to_string(), "cmd.exe".to_string()];
        let resolved = resolve_shell_with(Some("powershell.exe"), &candidates, |_| true);
        assert_eq!(resolved, "powershell.exe");
    }

    #[test]
    fn falls_back_through_candidates_when_preferred_missing() {
        let candidates = vec!["pwsh.exe".to_string(), "cmd.exe".to_string()];
        // preferred not available; first candidate not available; second is.
        let resolved =
            resolve_shell_with(Some("nope.exe"), &candidates, |shell| shell == "cmd.exe");
        assert_eq!(resolved, "cmd.exe");
    }

    #[test]
    fn falls_back_to_default_when_nothing_available() {
        let resolved = resolve_shell_with(None, &[], |_| false);
        assert_eq!(resolved, FALLBACK_SHELL);
    }

    #[test]
    fn chunks_long_writes() {
        let data = vec![b'a'; 2500];
        let chunks = chunk_write(&data, 1024);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 1024);
        assert_eq!(chunks[2].len(), 2500 - 2048);
        assert!(chunk_write(b"", 1024).is_empty());
    }

    #[test]
    fn detects_da1_query_but_not_replies() {
        assert!(contains_da1_query(b"prompt \x1b[c done"));
        assert!(contains_da1_query(b"\x1b[0c"));
        // DA1 reply / DA2 forms carry a private marker and must NOT match.
        assert!(!contains_da1_query(b"\x1b[?62;4;9;22c"));
        assert!(!contains_da1_query(b"\x1b[>0;10;1c"));
        assert!(!contains_da1_query(b"no escape here"));
    }

    #[test]
    fn detects_cpr_query() {
        assert!(contains_cpr_query(b"before \x1b[6n after"));
        assert!(!contains_cpr_query(b"\x1b[2J"));
    }

    #[test]
    fn detects_posix_paths() {
        assert!(is_posix_path("/home/chaz/project"));
        assert!(!is_posix_path("C:\\Users\\chaz"));
        assert!(!is_posix_path("\\\\server\\share"));
        assert!(!is_posix_path("//unc/style"));
    }

    #[test]
    fn sanitizes_posix_cwd_for_win32_spawn() {
        let home = "C:\\Users\\chaz";
        assert_eq!(
            win32_spawn_cwd(Some("/home/chaz/project"), home),
            Some(home.to_string())
        );
        assert_eq!(
            win32_spawn_cwd(Some("D:\\code"), home),
            Some("D:\\code".to_string())
        );
        assert_eq!(win32_spawn_cwd(None, home), None);
    }
}
