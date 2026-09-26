use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AntigravityProcessEnv {
    pub vars: HashMap<String, String>,
    pub gemini_home: PathBuf,
    pub scratch_dir: PathBuf,
    #[allow(dead_code)]
    pub harness_path: PathBuf,
}

/// Constructs a replaced, isolated process environment for Antigravity ACP processes.
/// Rules enforced:
/// 1. `GEMINI_HOME` is set to an isolated instance profile directory, NEVER user ~/.gemini
/// 2. `ANTIGRAVITY_HARNESS_PATH` points to the companion localharness_external binary
/// 3. `TEMP`/`TMP` points to an app-owned sibling scratch directory, respecting MAX_PATH
/// 4. `AGY_ACP_FORCE_FILE_STORAGE=1` forces deterministic local persistence
/// 5. `PYTHONUNBUFFERED=1` prevents stdout/stderr buffering
/// 6. `BROWSER` helper set to suppress OS browser popups and capture OAuth URL
/// 7. Inherited API keys, credentials, and AGY_ACP tokens are strictly scrubbed
pub fn build_antigravity_env(
    instance_id: &str,
    app_data_base: &Path,
    harness_path: &Path,
    browser_helper_cmd: &str,
) -> AntigravityProcessEnv {
    let gemini_home = app_data_base
        .join("profiles/antigravity")
        .join(instance_id);

    // Keep scratch temp dir a SIBLING of the profile dir to stay under Windows MAX_PATH
    let scratch_dir = app_data_base
        .join("scratch/antigravity")
        .join(instance_id);

    let mut vars = HashMap::new();

    // Standard required system environment variables on Windows
    if let Ok(sys_root) = std::env::var("SystemRoot") {
        vars.insert("SystemRoot".into(), sys_root);
    }
    if let Ok(windir) = std::env::var("windir") {
        vars.insert("windir".into(), windir);
    }
    if let Ok(path) = std::env::var("PATH") {
        vars.insert("PATH".into(), path);
    }
    if let Ok(comspec) = std::env::var("COMSPEC") {
        vars.insert("COMSPEC".into(), comspec);
    }

    // Antigravity isolation variables
    vars.insert(
        "GEMINI_HOME".into(),
        gemini_home.to_string_lossy().to_string(),
    );
    vars.insert(
        "ANTIGRAVITY_HARNESS_PATH".into(),
        harness_path.to_string_lossy().to_string(),
    );
    vars.insert("AGY_ACP_FORCE_FILE_STORAGE".into(), "1".into());
    vars.insert("PYTHONUNBUFFERED".into(), "1".into());
    vars.insert("BROWSER".into(), browser_helper_cmd.to_string());

    // Sibling scratch temp paths
    let scratch_str = scratch_dir.to_string_lossy().to_string();
    vars.insert("TEMP".into(), scratch_str.clone());
    vars.insert("TMP".into(), scratch_str.clone());
    vars.insert("TMPDIR".into(), scratch_str);

    AntigravityProcessEnv {
        vars,
        gemini_home,
        scratch_dir,
        harness_path: harness_path.to_path_buf(),
    }
}

/// Validates that an environment map contains zero leaked API keys or credentials.
pub fn validate_sanitized_environment(vars: &HashMap<String, String>) -> bool {
    let forbidden_prefixes = [
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "GOOGLE_CLOUD_",
        "AGY_ACP_AUTH_",
        "AGY_ACP_TOKEN",
    ];

    for key in vars.keys() {
        let upper = key.to_uppercase();
        for forbidden in &forbidden_prefixes {
            if upper.starts_with(forbidden) {
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_environment_isolation_and_sibling_temp() {
        let base = Path::new("C:\\Users\\User\\AppData\\Local\\pandamux");
        let harness = Path::new("C:\\Users\\User\\AppData\\Local\\pandamux\\tools\\localharness.exe");
        let env = build_antigravity_env("personal_max", base, harness, "pandamux-browser-helper");

        assert_eq!(
            env.gemini_home,
            base.join("profiles/antigravity/personal_max")
        );
        assert_eq!(
            env.scratch_dir,
            base.join("scratch/antigravity/personal_max")
        );

        // Confirm sibling relationship (neither is parent of the other)
        assert!(!env.scratch_dir.starts_with(&env.gemini_home));
        assert!(!env.gemini_home.starts_with(&env.scratch_dir));

        assert_eq!(env.vars.get("AGY_ACP_FORCE_FILE_STORAGE").unwrap(), "1");
        assert_eq!(env.vars.get("PYTHONUNBUFFERED").unwrap(), "1");
        assert_eq!(env.vars.get("BROWSER").unwrap(), "pandamux-browser-helper");

        assert!(validate_sanitized_environment(&env.vars));
    }
}
