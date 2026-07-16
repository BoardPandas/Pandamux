// Ship the GUI build as a Windows GUI-subsystem binary so double-clicking the
// exe (or the Start Menu shortcut) does NOT pop a console/terminal window. The
// headless build (no `iced-runtime` feature) stays a console app so the pipe
// server and CLI-style invocations keep their stdout/stderr.
#![cfg_attr(all(windows, feature = "iced-runtime"), windows_subsystem = "windows")]

mod backend;
mod clipboard_os;
// In-app update check (Phase 7). The decision logic is always compiled/tested;
// the network fetch is gated behind `iced-runtime`.
#[cfg(feature = "iced-runtime")]
mod iced_runtime;
#[cfg(feature = "iced-runtime")]
mod pollers;
#[allow(dead_code)]
mod updater;
// The runtime uses the auto-session half now; the named-session API (save/load/
// list/delete) is a complete, tested surface the Phase 5 command palette and
// session panel will call, so its as-yet-unwired items are allowed to be idle.
#[allow(dead_code)]
mod persistence;
mod pipe_server;
#[cfg_attr(not(feature = "iced-runtime"), allow(dead_code))]
mod project_launcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    set_app_user_model_id();

    let has_flag = |name: &str| std::env::args().any(|arg| arg == name);

    // Noninteractive CI smoke: build the shell view once and exit. Must be
    // checked before the default-GUI launch below.
    #[cfg(feature = "iced-runtime")]
    if has_flag("--iced-shell-smoke") {
        iced_runtime::run_iced_shell_smoke()?;
        return Ok(());
    }

    // Opt-in live SSH smoke (plan F2): connect to a real host through the same
    // RemoteSessionManager the runtime uses, run a durable tmux command, and read
    // it back. Reuses the Phase 2 auth matrix. Never runs in CI (flag-gated).
    if has_flag("--ssh-smoke") {
        return run_ssh_smoke();
    }

    // In the GUI build, open the window by DEFAULT. The installed Start Menu
    // shortcut runs `pandamux.exe` with no arguments (cargo-packager's NSIS
    // config cannot pass shortcut args), so the argument-less path must be the
    // GUI, not the headless pipe server. `--iced-shell` is kept for backward
    // compatibility; `--headless`/`--pipe-server` forces the standalone server.
    #[cfg(feature = "iced-runtime")]
    if !has_flag("--headless") && !has_flag("--pipe-server") {
        iced_runtime::run_iced_shell()?;
        return Ok(());
    }

    let pipe_name =
        std::env::var("PANDAMUX_PIPE").unwrap_or_else(|_| r"\\.\pipe\pandamux".to_string());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(pipe_server::run(&pipe_name))?;
    Ok(())
}

/// Manual end-to-end SSH validation (plan F2). Configure with:
///   PANDAMUX_SSH_SMOKE_HOST, PANDAMUX_SSH_SMOKE_USER (required),
///   PANDAMUX_SSH_SMOKE_AUTH = agent (default) | password | key,
///   PANDAMUX_SSH_SMOKE_PASSWORD or PANDAMUX_SSH_SMOKE_KEY as applicable.
/// Prints `PANDAMUX_SSH_SMOKE_OK` on success.
fn run_ssh_smoke() -> Result<(), Box<dyn std::error::Error>> {
    use pandamux_term::{DEFAULT_GRID_SIZE, RemoteSessionManager, SshAuth, SshConfig};
    use std::time::{Duration, Instant};

    let host =
        std::env::var("PANDAMUX_SSH_SMOKE_HOST").map_err(|_| "set PANDAMUX_SSH_SMOKE_HOST")?;
    let user =
        std::env::var("PANDAMUX_SSH_SMOKE_USER").map_err(|_| "set PANDAMUX_SSH_SMOKE_USER")?;
    let auth = match std::env::var("PANDAMUX_SSH_SMOKE_AUTH")
        .unwrap_or_else(|_| "agent".to_string())
        .as_str()
    {
        "password" => SshAuth::Password {
            password: std::env::var("PANDAMUX_SSH_SMOKE_PASSWORD")
                .map_err(|_| "set PANDAMUX_SSH_SMOKE_PASSWORD")?,
        },
        "key" => SshAuth::KeyFile {
            path: std::env::var("PANDAMUX_SSH_SMOKE_KEY")
                .map_err(|_| "set PANDAMUX_SSH_SMOKE_KEY")?
                .into(),
            passphrase: std::env::var("PANDAMUX_SSH_SMOKE_PASSPHRASE").ok(),
        },
        _ => SshAuth::Agent {
            pipe_path: r"\\.\pipe\openssh-ssh-agent".to_string(),
        },
    };

    let remote_cwd =
        std::env::var("PANDAMUX_SSH_SMOKE_CWD").map_err(|_| "set PANDAMUX_SSH_SMOKE_CWD")?;
    let trust_unknown = std::env::var("PANDAMUX_SSH_SMOKE_TRUST_UNKNOWN")
        .is_ok_and(|value| value.eq_ignore_ascii_case("true"));
    let config = SshConfig::new(host.clone(), user, auth)
        .with_remote_cwd(remote_cwd.clone())
        .with_unknown_host_trust(trust_unknown);
    let mut manager = RemoteSessionManager::new()?;
    manager.connect_ready(
        "surf-ssh-smoke",
        config,
        DEFAULT_GRID_SIZE,
        Duration::from_secs(30),
    )?;
    println!("connecting to {host} ...");

    // Wait for the shell/tmux to come up, then run a marker command and read it.
    manager.write_all(
        "surf-ssh-smoke",
        b"printf 'PANDAMUX_SSH_SMOKE_MARKER\\n'; pwd\n",
    )?;

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut screen = String::new();
    while Instant::now() < deadline {
        screen = manager.screen_text("surf-ssh-smoke")?;
        if screen.contains("PANDAMUX_SSH_SMOKE_MARKER")
            && screen.matches("PANDAMUX_SSH_SMOKE_MARKER").count() >= 2
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = manager.kill("surf-ssh-smoke");

    if screen.contains("PANDAMUX_SSH_SMOKE_MARKER") && screen.contains(&remote_cwd) {
        println!("--- remote screen tail ---\n{screen}\n---");
        println!("PANDAMUX_SSH_SMOKE_OK");
        Ok(())
    } else {
        Err(format!("ssh smoke never saw the marker; screen was:\n{screen}").into())
    }
}

/// Set the Windows AppUserModelID so the taskbar groups the app under a stable
/// identity (`com.pandamux.app`, matching the Electron build). No-op off Windows.
fn set_app_user_model_id() {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn SetCurrentProcessExplicitAppUserModelID(app_id: *const u16) -> i32;
        }
        let id: Vec<u16> = std::ffi::OsStr::new("com.pandamux.app")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let _ = SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
        }
    }
}
