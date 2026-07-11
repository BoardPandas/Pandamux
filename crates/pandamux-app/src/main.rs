mod backend;
mod clipboard_os;
// In-app update check (Phase 7). The decision logic is always compiled/tested;
// the network fetch is gated behind `iced-runtime`.
#[allow(dead_code)]
mod updater;
// Claude Code startup integration (context injection + orchestrator plugin).
// Only invoked for the real GUI launch, but always compiled so its tests run.
#[allow(dead_code)]
mod claude_context;
#[cfg(feature = "iced-runtime")]
mod iced_runtime;
#[cfg(feature = "iced-runtime")]
mod pollers;
// The runtime uses the auto-session half now; the named-session API (save/load/
// list/delete) is a complete, tested surface the Phase 5 command palette and
// session panel will call, so its as-yet-unwired items are allowed to be idle.
#[allow(dead_code)]
mod persistence;
mod pipe_server;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    set_app_user_model_id();

    #[cfg(feature = "iced-runtime")]
    if std::env::args().any(|arg| arg == "--iced-shell") {
        // Make Claude Code aware of PandaMUX and install the
        // orchestrator plugin (best-effort; never aborts launch). Not run for
        // the headless pipe server or the `--iced-shell-smoke` CI path.
        claude_context::run_startup_integration();
        iced_runtime::run_iced_shell()?;
        return Ok(());
    }

    #[cfg(feature = "iced-runtime")]
    if std::env::args().any(|arg| arg == "--iced-shell-smoke") {
        iced_runtime::run_iced_shell_smoke()?;
        return Ok(());
    }

    // Opt-in live SSH smoke (plan F2): connect to a real host through the same
    // RemoteSessionManager the runtime uses, run a durable tmux command, and read
    // it back. Reuses the Phase 2 auth matrix. Never runs in CI (flag-gated).
    if std::env::args().any(|arg| arg == "--ssh-smoke") {
        return run_ssh_smoke();
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
    use pandamux_term::{GridSize, RemoteSessionManager, SshAuth, SshConfig};
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

    let config = SshConfig::new(host.clone(), user, auth);
    let mut manager = RemoteSessionManager::new()?;
    manager.connect("surf-ssh-smoke", config, GridSize::new(120, 30))?;
    println!("connecting to {host} ...");

    // Wait for the shell/tmux to come up, then run a marker command and read it.
    std::thread::sleep(Duration::from_secs(3));
    manager.write_all("surf-ssh-smoke", b"echo PANDAMUX_SSH_SMOKE_MARKER\n")?;

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

    if screen.contains("PANDAMUX_SSH_SMOKE_MARKER") {
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
