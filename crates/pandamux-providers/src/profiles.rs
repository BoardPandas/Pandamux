use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use pandamux_core::ids::ProviderInstanceId;
use pandamux_core::provider_config::ProviderKind;
use tokio::process::Command;
use crate::error::ProviderError;

/// Environment variable names used for provider configuration directories.
pub const ENV_CLAUDE_CONFIG_DIR: &str = "CLAUDE_CONFIG_DIR";
pub const ENV_CODEX_HOME: &str = "CODEX_HOME";
pub const ENV_GEMINI_HOME: &str = "GEMINI_HOME";

/// Returns the deterministic isolated profile directory for a given provider instance.
/// Path layout: `<base_dir>/profiles/<provider>/<instance_id>/`.
pub fn resolve_profile_dir(
    base_dir: &Path,
    provider: ProviderKind,
    instance_id: &ProviderInstanceId,
) -> PathBuf {
    base_dir
        .join("profiles")
        .join(provider.as_str())
        .join(instance_id.as_str())
}

/// Ensures the profile directory exists on disk, creating parent directories if necessary.
pub fn ensure_profile_dir(profile_path: &Path) -> Result<(), ProviderError> {
    std::fs::create_dir_all(profile_path).map_err(|e| ProviderError::ProfileError {
        path: profile_path.to_path_buf(),
        message: format!("Failed to create profile directory: {e}"),
    })
}

/// Applies isolated profile environment variables to a child command.
pub fn apply_profile_environment(
    cmd: &mut Command,
    provider: ProviderKind,
    profile_dir: &Path,
    env_overrides: &BTreeMap<String, String>,
) {
    match provider {
        ProviderKind::Claude => {
            cmd.env(ENV_CLAUDE_CONFIG_DIR, profile_dir);
        }
        ProviderKind::Codex => {
            cmd.env(ENV_CODEX_HOME, profile_dir);
        }
        ProviderKind::Antigravity => {
            cmd.env(ENV_GEMINI_HOME, profile_dir);
            cmd.env("AGY_ACP_FORCE_FILE_STORAGE", "1");
            cmd.env("PYTHONUNBUFFERED", "1");

            // Strip sensitive ambient keys that could interfere with isolated auth
            cmd.env_remove("GEMINI_API_KEY");
            cmd.env_remove("GOOGLE_API_KEY");
            cmd.env_remove("GOOGLE_APPLICATION_CREDENTIALS");
        }
        _ => {}
    }

    // Apply custom user/instance overrides
    for (k, v) in env_overrides {
        cmd.env(k, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_profile_dir_layout() {
        let base = Path::new("/var/data/pandamux");
        let inst = ProviderInstanceId::from("inst-prod-1");

        let claude_dir = resolve_profile_dir(base, ProviderKind::Claude, &inst);
        assert_eq!(
            claude_dir,
            PathBuf::from("/var/data/pandamux/profiles/claude/inst-prod-1")
        );

        let codex_dir = resolve_profile_dir(base, ProviderKind::Codex, &inst);
        assert_eq!(
            codex_dir,
            PathBuf::from("/var/data/pandamux/profiles/codex/inst-prod-1")
        );

        let gemini_dir = resolve_profile_dir(base, ProviderKind::Antigravity, &inst);
        assert_eq!(
            gemini_dir,
            PathBuf::from("/var/data/pandamux/profiles/antigravity/inst-prod-1")
        );
    }
}
