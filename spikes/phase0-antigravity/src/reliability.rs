use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Result, bail};
use serde_json::Value;

#[allow(dead_code)]
pub const MIN_FREE_DISK_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GB
pub const MAX_TOOL_PAYLOAD_BYTES: usize = 64 * 1024; // 64 KB

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AntigravityHealth {
    Healthy { version: String, auth_type: Option<String> },
    Uninstalled,
    #[allow(dead_code)]
    InvalidProfile,
}

/// Zero-spawn health check.
/// Rule: NEVER spawn the process for routine health checks because the PyInstaller bundle
/// unpacks ~1 GB per launch. Instead, validates on-disk resolution and cached install records.
pub fn probe_antigravity_health_offline(
    binary_path: &Path,
    gemini_home: &Path,
    cached_version: Option<&str>,
) -> AntigravityHealth {
    if !binary_path.is_file() {
        return AntigravityHealth::Uninstalled;
    }

    let auth_type = if gemini_home.is_dir() {
        let settings_path = gemini_home.join("settings.json");
        if let Ok(content) = fs::read_to_string(&settings_path) {
            serde_json::from_str::<Value>(&content)
                .ok()
                .and_then(|v| v.get("auth").and_then(|a| a.get("type")).and_then(|t| t.as_str()).map(|s| s.to_string()))
        } else {
            None
        }
    } else {
        None
    };

    AntigravityHealth::Healthy {
        version: cached_version.unwrap_or("1.1.1").to_string(),
        auth_type,
    }
}

/// Sweeps orphaned PyInstaller unpack temporary directories (_MEI* or agy_tmp_*)
/// from the application scratch directory.
pub fn sweep_orphan_temp_dirs(scratch_dir: &Path) -> Result<usize> {
    if !scratch_dir.exists() {
        return Ok(0);
    }

    let mut removed_count = 0;
    for entry in fs::read_dir(scratch_dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();

        if path.is_dir() && (name.starts_with("_MEI") || name.starts_with("agy_tmp_")) {
            if fs::remove_dir_all(&path).is_ok() {
                removed_count += 1;
            }
        }
    }

    Ok(removed_count)
}

/// Concurrency limiter enforcing max active Antigravity processes per environment (default 2).
pub struct AntigravityConcurrencyLimiter {
    active_count: AtomicUsize,
    max_processes: usize,
}

impl AntigravityConcurrencyLimiter {
    pub fn new(max_processes: usize) -> Self {
        Self {
            active_count: AtomicUsize::new(0),
            max_processes,
        }
    }

    pub fn try_acquire(&self) -> Result<ConcurrencyGuard<'_>> {
        let current = self.active_count.fetch_add(1, Ordering::SeqCst);
        if current >= self.max_processes {
            self.active_count.fetch_sub(1, Ordering::SeqCst);
            bail!(
                "Antigravity concurrency limit reached ({} of {} active)",
                current,
                self.max_processes
            );
        }

        Ok(ConcurrencyGuard {
            limiter: self,
        })
    }

    pub fn current_active(&self) -> usize {
        self.active_count.load(Ordering::SeqCst)
    }
}

pub struct ConcurrencyGuard<'a> {
    limiter: &'a AntigravityConcurrencyLimiter,
}

impl Drop for ConcurrencyGuard<'_> {
    fn drop(&mut self) {
        self.limiter.active_count.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Sanitizes tool payload and truncates excessive output to protect memory and context.
pub fn sanitize_tool_payload(raw_text: &str) -> String {
    if raw_text.len() <= MAX_TOOL_PAYLOAD_BYTES {
        raw_text.to_string()
    } else {
        let truncated = &raw_text[..MAX_TOOL_PAYLOAD_BYTES];
        format!("{}... [Truncated: exceeded 64 KB limit]", truncated)
    }
}

/// Normalizes alternate field casing for command line inputs across different tool definitions.
pub fn extract_normalized_command(params: &Value) -> Option<String> {
    let candidates = ["command", "command_line", "CommandLine", "commandLine", "cmd"];
    for key in &candidates {
        if let Some(cmd) = params.get(*key).and_then(|v| v.as_str()) {
            return Some(cmd.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_concurrency_limiter() {
        let limiter = AntigravityConcurrencyLimiter::new(2);
        let guard1 = limiter.try_acquire().unwrap();
        assert_eq!(limiter.current_active(), 1);

        let _guard2 = limiter.try_acquire().unwrap();
        assert_eq!(limiter.current_active(), 2);

        // Third must fail
        assert!(limiter.try_acquire().is_err());

        drop(guard1);
        assert_eq!(limiter.current_active(), 1);

        let _guard3 = limiter.try_acquire().unwrap();
        assert_eq!(limiter.current_active(), 2);
    }

    #[test]
    fn test_payload_sanitizing() {
        let short = "git status";
        assert_eq!(sanitize_tool_payload(short), short);

        let large = "A".repeat(70 * 1024);
        let sanitized = sanitize_tool_payload(&large);
        assert!(sanitized.contains("[Truncated"));
        assert!(sanitized.len() < 70 * 1024);
    }

    #[test]
    fn test_field_normalization() {
        let val1 = serde_json::json!({ "CommandLine": "cargo test" });
        assert_eq!(extract_normalized_command(&val1), Some("cargo test".into()));

        let val2 = serde_json::json!({ "command_line": "cargo check" });
        assert_eq!(extract_normalized_command(&val2), Some("cargo check".into()));
    }
}
