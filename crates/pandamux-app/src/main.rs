#[cfg(feature = "iced-runtime")]
mod iced_runtime;
// The runtime uses the auto-session half now; the named-session API (save/load/
// list/delete) is a complete, tested surface the Phase 5 command palette and
// session panel will call, so its as-yet-unwired items are allowed to be idle.
#[allow(dead_code)]
mod persistence;
mod pipe_server;

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
