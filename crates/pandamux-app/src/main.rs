mod backend;
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
        iced_runtime::run_iced_shell()?;
        return Ok(());
    }

    #[cfg(feature = "iced-runtime")]
    if std::env::args().any(|arg| arg == "--iced-shell-smoke") {
        iced_runtime::run_iced_shell_smoke()?;
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
