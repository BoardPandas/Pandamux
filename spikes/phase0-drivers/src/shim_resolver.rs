use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShimType {
    NativeExe,
    CmdScript,
    BatScript,
    PowerShell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub program: PathBuf,
    pub shim_type: ShimType,
    pub is_direct_executable: bool,
}

/// Resolves a command name on Windows by inspecting PATH and PATHEXT.
/// Handles .cmd and .bat wrappers commonly created by npm, pnpm, and cargo.
pub fn resolve_command_shim(cmd: &str) -> Option<ResolvedCommand> {
    let path_var = env::var_os("PATH")?;
    let paths = env::split_paths(&path_var);

    let pathext = env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD;.VBS;.JS;.WS;.MSC".into());
    let extensions: Vec<String> = pathext
        .split(';')
        .map(|ext| ext.trim().to_lowercase())
        .filter(|ext| !ext.is_empty())
        .collect();

    let cmd_path = Path::new(cmd);

    // If caller provided an explicit file that exists
    if cmd_path.is_file() {
        return Some(classify_resolved_path(cmd_path.to_path_buf()));
    }

    // If cmd already has an extension, search PATH directly
    if cmd_path.extension().is_some() {
        for dir in paths {
            let candidate = dir.join(cmd);
            if candidate.is_file() {
                return Some(classify_resolved_path(candidate));
            }
        }
        return None;
    }

    // Try extensions in PATHEXT order across PATH directories
    for dir in paths {
        for ext in &extensions {
            let candidate_name = format!("{}{}", cmd, ext);
            let candidate = dir.join(candidate_name);
            if candidate.is_file() {
                return Some(classify_resolved_path(candidate));
            }
        }
    }

    None
}

fn classify_resolved_path(path: PathBuf) -> ResolvedCommand {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let shim_type = match ext.as_str() {
        "cmd" => ShimType::CmdScript,
        "bat" => ShimType::BatScript,
        "ps1" => ShimType::PowerShell,
        _ => ShimType::NativeExe,
    };

    let is_direct_executable = matches!(shim_type, ShimType::NativeExe);

    ResolvedCommand {
        program: path,
        shim_type,
        is_direct_executable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolves_system_executables() {
        let resolved = resolve_command_shim("cmd").expect("cmd should resolve on Windows");
        assert!(resolved.program.exists());
        assert_eq!(resolved.shim_type, ShimType::NativeExe);
        assert!(resolved.is_direct_executable);
    }

    #[test]
    fn test_resolves_claude_if_present() {
        if let Some(resolved) = resolve_command_shim("claude") {
            assert!(resolved.program.exists());
            println!("Resolved claude: {:?}", resolved);
        }
    }
}
