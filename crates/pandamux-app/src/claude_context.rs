//! Claude Code startup integration, ported from the Electron `claude-context.ts`.
//!
//! On launch of the real GUI the native app makes Claude Code aware of PandaMUX
//! Everywhere and installs the orchestrator plugin, exactly as the Electron
//! build does:
//!
//! - [`ensure_claude_context`] injects a marker-delimited PandaMUX block into the
//!   user's `~/.claude/CLAUDE.md`, never touching content outside the
//!   `<!-- pandamux:start ... -->` / `<!-- pandamux:end -->` markers.
//! - [`ensure_orchestrator_plugin`] copies `resources/pandamux-orchestrator/`
//!   into `~/.claude/plugins/cache/pandamux-orchestrator/{version}/`, registers
//!   it in `installed_plugins.json`, and enables it in `settings.json`.
//!
//! Every function takes its base directory by value so tests run against a temp
//! dir; only [`run_startup_integration`] resolves the real `~/.claude` and
//! `resources` locations, and it is best-effort (a failure logs and never
//! aborts startup).
//!
//! Deferred (tracked): the hook wiring in `settings.json` and the live activity
//! observer are the observability half of the Electron integration; the
//! busy-agent status dot is already fed by the agent registry as the interim
//! signal (plan Section 7), so those land near the Phase 7 ship boundary.

use serde_json::{Value, json};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const START_MARKER: &str = "<!-- pandamux:start";
const END_MARKER: &str = "<!-- pandamux:end -->";
const PLUGIN_KEY: &str = "pandamux-orchestrator@pandamux";

/// Ensure `~/.claude/CLAUDE.md` (under `claude_dir`) contains the PandaMUX block.
/// Creates the file with just the block if absent, appends it if missing,
/// repairs broken markers, and replaces an outdated block, all idempotently.
pub fn ensure_claude_context(claude_dir: &Path, block: &str) -> io::Result<()> {
    fs::create_dir_all(claude_dir)?;
    let claude_md = claude_dir.join("CLAUDE.md");

    if !claude_md.exists() {
        fs::write(&claude_md, block)?;
        return Ok(());
    }

    let existing = fs::read_to_string(&claude_md)?;
    let start = existing.find(START_MARKER);
    let end = existing.find(END_MARKER);

    match (start, end) {
        // No block: append it (with a separating newline).
        (None, _) => {
            let separator = if existing.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            fs::write(&claude_md, format!("{existing}{separator}{block}"))?;
        }
        // Start but no end (broken markers): replace from the start marker to EOF.
        (Some(start), None) => {
            fs::write(&claude_md, format!("{}{block}", &existing[..start]))?;
        }
        // Both markers present: replace the block if it is out of date.
        (Some(start), Some(end)) => {
            let block_end = end + END_MARKER.len();
            let current = &existing[start..block_end];
            if current.trim() == block.trim() {
                return Ok(());
            }
            let before = &existing[..start];
            let after = &existing[block_end..];
            fs::write(&claude_md, format!("{before}{block}{after}"))?;
        }
    }
    Ok(())
}

/// Install the pandamux-orchestrator plugin under `claude_dir` from
/// `plugin_src_dir`. Skips the copy when the same version is already cached, but
/// still (re)registers it. Best-effort registration errors are swallowed.
pub fn ensure_orchestrator_plugin(claude_dir: &Path, plugin_src_dir: &Path) -> io::Result<()> {
    let plugin_json_src = plugin_src_dir.join(".claude-plugin").join("plugin.json");
    if !plugin_json_src.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("plugin.json not found at {}", plugin_json_src.display()),
        ));
    }

    let version = read_plugin_version(&plugin_json_src)?;
    let cache_dir = claude_dir
        .join("plugins")
        .join("cache")
        .join("pandamux-orchestrator")
        .join(&version);
    let target_plugin_json = cache_dir.join(".claude-plugin").join("plugin.json");

    // Already installed at this version: skip the copy, still ensure registration.
    if target_plugin_json.exists()
        && read_plugin_version(&target_plugin_json)
            .map(|existing| existing == version)
            .unwrap_or(false)
    {
        ensure_plugin_registered(&cache_dir, &version, claude_dir);
        return Ok(());
    }

    if cache_dir.exists() {
        fs::remove_dir_all(&cache_dir)?;
    }
    copy_dir(plugin_src_dir, &cache_dir)?;
    ensure_plugin_registered(&cache_dir, &version, claude_dir);
    Ok(())
}

fn read_plugin_version(plugin_json: &Path) -> io::Result<String> {
    let content = fs::read_to_string(plugin_json)?;
    let meta: Value = serde_json::from_str(&content)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(meta
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("0.0.0")
        .to_string())
}

/// Register the plugin in `installed_plugins.json` and enable it in
/// `settings.json` (the latter only if the settings file exists, matching the
/// Electron behavior of not creating settings out of thin air).
fn ensure_plugin_registered(install_path: &Path, version: &str, claude_dir: &Path) {
    let install_path_str = install_path.to_string_lossy().to_string();
    let now = iso_now();

    let installed_path = claude_dir.join("plugins").join("installed_plugins.json");
    let mut installed = installed_path
        .exists()
        .then(|| fs::read_to_string(&installed_path).ok())
        .flatten()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({}));
    if !installed.is_object() {
        installed = json!({});
    }

    let existing = installed.get(PLUGIN_KEY);
    let up_to_date = existing
        .map(|entry| {
            entry.get("version").and_then(Value::as_str) == Some(version)
                && entry.get("installPath").and_then(Value::as_str) == Some(&install_path_str)
        })
        .unwrap_or(false);
    if !up_to_date {
        let installed_at = existing
            .and_then(|entry| entry.get("installedAt"))
            .and_then(Value::as_str)
            .unwrap_or(&now)
            .to_string();
        installed[PLUGIN_KEY] = json!({
            "scope": "user",
            "installPath": install_path_str,
            "version": version,
            "installedAt": installed_at,
            "lastUpdated": now,
        });
        if let Some(parent) = installed_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&installed) {
            let _ = fs::write(&installed_path, text);
        }
    }

    // Enable in settings.json only when it already exists.
    let settings_path = claude_dir.join("settings.json");
    if let Ok(raw) = fs::read_to_string(&settings_path)
        && let Ok(mut settings) = serde_json::from_str::<Value>(&raw)
        && settings.is_object()
    {
        let enabled = settings
            .get("enabledPlugins")
            .and_then(|value| value.get(PLUGIN_KEY))
            .and_then(Value::as_bool)
            == Some(true);
        if !enabled {
            if !settings["enabledPlugins"].is_object() {
                settings["enabledPlugins"] = json!({});
            }
            settings["enabledPlugins"][PLUGIN_KEY] = json!(true);
            if let Ok(text) = serde_json::to_string_pretty(&settings) {
                let _ = fs::write(&settings_path, text);
            }
        }
    }
}

fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Run the startup integration against the real `~/.claude` and `resources`.
/// Best-effort: any failure is logged to stderr and never aborts launch.
pub fn run_startup_integration() {
    let Some(claude_dir) = home_dir().map(|home| home.join(".claude")) else {
        eprintln!("[pandamux] could not resolve home directory for Claude integration");
        return;
    };
    let Some(resources) = resources_dir() else {
        eprintln!("[pandamux] could not resolve resources directory for Claude integration");
        return;
    };

    let instructions = resources.join("claude-instructions.md");
    match fs::read_to_string(&instructions) {
        Ok(block) => {
            if let Err(error) = ensure_claude_context(&claude_dir, &block) {
                eprintln!("[pandamux] failed to update Claude context: {error}");
            }
        }
        Err(error) => eprintln!(
            "[pandamux] claude-instructions.md not found at {}: {error}",
            instructions.display()
        ),
    }

    let plugin_src = resources.join("pandamux-orchestrator");
    if let Err(error) = ensure_orchestrator_plugin(&claude_dir, &plugin_src) {
        eprintln!("[pandamux] failed to install pandamux-orchestrator plugin: {error}");
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// Resolve the `resources` directory: `PANDAMUX_RESOURCES_DIR` if set, else
/// `<exe dir>/resources`, else walk up from the cwd to a `resources` dir.
fn resources_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PANDAMUX_RESOURCES_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let candidate = parent.join("resources");
        if candidate.join("claude-instructions.md").is_file() {
            return Some(candidate);
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join("resources");
        if candidate.join("claude-instructions.md").is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) from the wall clock, dep-free.
fn iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's days-from-civil inverse: Unix day number -> (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pandamux-cc-{tag}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    const BLOCK: &str = "<!-- pandamux:start -->\n# PandaMUX\nhello\n<!-- pandamux:end -->";

    #[test]
    fn creates_claude_md_when_absent() {
        let dir = temp_dir("create");
        ensure_claude_context(&dir, BLOCK).unwrap();
        let content = fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert_eq!(content, BLOCK);
    }

    #[test]
    fn appends_block_and_preserves_user_content() {
        let dir = temp_dir("append");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("CLAUDE.md"), "# My rules\nkeep me\n").unwrap();
        ensure_claude_context(&dir, BLOCK).unwrap();
        let content = fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(content.starts_with("# My rules\nkeep me\n"));
        assert!(content.contains(BLOCK));
    }

    #[test]
    fn replaces_outdated_block_only() {
        let dir = temp_dir("replace");
        fs::create_dir_all(&dir).unwrap();
        let old = "before\n<!-- pandamux:start -->\nOLD\n<!-- pandamux:end -->\nafter\n";
        fs::write(dir.join("CLAUDE.md"), old).unwrap();
        ensure_claude_context(&dir, BLOCK).unwrap();
        let content = fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(content.starts_with("before\n"));
        assert!(content.ends_with("after\n"));
        assert!(content.contains("hello"));
        assert!(!content.contains("OLD"));
    }

    #[test]
    fn is_idempotent_when_up_to_date() {
        let dir = temp_dir("idem");
        ensure_claude_context(&dir, BLOCK).unwrap();
        ensure_claude_context(&dir, BLOCK).unwrap();
        let content = fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert_eq!(content.matches("pandamux:start").count(), 1);
    }

    #[test]
    fn installs_and_registers_plugin() {
        let dir = temp_dir("plugin");
        // Build a minimal plugin source tree.
        let src = temp_dir("plugin-src");
        fs::create_dir_all(src.join(".claude-plugin")).unwrap();
        fs::write(
            src.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"pandamux-orchestrator","version":"1.2.3"}"#,
        )
        .unwrap();
        fs::create_dir_all(src.join("scripts")).unwrap();
        fs::write(src.join("scripts").join("go.sh"), "echo hi").unwrap();
        // A pre-existing settings.json so the enable path runs.
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), "{}").unwrap();

        ensure_orchestrator_plugin(&dir, &src).unwrap();

        // Copied into cache/{version}/ with nested files.
        let cache = dir
            .join("plugins")
            .join("cache")
            .join("pandamux-orchestrator")
            .join("1.2.3");
        assert!(cache.join(".claude-plugin").join("plugin.json").exists());
        assert!(cache.join("scripts").join("go.sh").exists());

        // Registered in installed_plugins.json.
        let installed: Value = serde_json::from_str(
            &fs::read_to_string(dir.join("plugins").join("installed_plugins.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(installed[PLUGIN_KEY]["version"], "1.2.3");
        assert_eq!(installed[PLUGIN_KEY]["scope"], "user");

        // Enabled in settings.json.
        let settings: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["enabledPlugins"][PLUGIN_KEY], true);

        // Second run is a no-op that keeps the registration.
        ensure_orchestrator_plugin(&dir, &src).unwrap();
        assert!(cache.join(".claude-plugin").join("plugin.json").exists());
    }

    #[test]
    fn iso_now_has_expected_shape() {
        let stamp = iso_now();
        assert_eq!(stamp.len(), 20);
        assert!(stamp.ends_with('Z'));
        assert_eq!(&stamp[4..5], "-");
        // Epoch day 0 is 1970-01-01.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }
}
