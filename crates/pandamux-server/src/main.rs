use pandamux_protocol::ServerRole;
use pandamux_server::{ClientIdentity, Server, ServerConfig};
#[cfg(not(windows))]
use pandamux_server::{RuntimeInfo, serve_proxy_connection};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
enum StartupAction {
    Run(ServerRunArgs),
    BrowserHelper(String),
    Help,
    Version,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ServerRunArgs {
    proxy_mode: bool,
    role: ServerRole,
    runtime_dir: Option<PathBuf>,
    db_path: Option<PathBuf>,
    environment_id: String,
    launcher_owned: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let run_args = match parse_startup_action(&args)
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?
    {
        StartupAction::Help => {
            print_help();
            return Ok(());
        }
        StartupAction::Version => {
            println!("pandamux-server v{}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        StartupAction::BrowserHelper(url) => {
            return run_antigravity_browser_helper(&url);
        }
        StartupAction::Run(run_args) => run_args,
    };
    let proxy_mode = run_args.proxy_mode;
    let role = run_args.role;
    let launcher_owned = run_args.launcher_owned;
    let runtime_dir = run_args
        .runtime_dir
        .unwrap_or_else(|| default_runtime_dir(role == ServerRole::Node || proxy_mode));
    let mut db_path = run_args.db_path;
    let environment_id = run_args.environment_id;

    if proxy_mode {
        match run_proxy_bridge(&runtime_dir).await {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("PandaMUX proxy bridge failed: {error}");
                std::process::exit(1);
            }
        }
    }

    if db_path.is_none() {
        let data_dir = runtime_dir
            .parent()
            .unwrap_or(runtime_dir.as_path())
            .join("data");
        std::fs::create_dir_all(&data_dir)?;
        db_path = Some(data_dir.join("pandamux.db"));
    }

    let pipe_path = default_pipe_path(&runtime_dir, role)?;
    let proxy_path = if role == ServerRole::Node {
        Some(runtime_dir.join("proxy.sock"))
    } else {
        None
    };
    let config = ServerConfig {
        role,
        environment_id,
        runtime_dir: runtime_dir.clone(),
        db_path,
    };
    let server = Arc::new(Server::new(config)?);
    let recovery = server.recover_orchestration().await?;
    if recovery.reviews_created != 0 || recovery.reviews_deduplicated != 0 {
        eprintln!(
            "PandaMUX recovery retained {} new and {} existing review item(s)",
            recovery.reviews_created, recovery.reviews_deduplicated
        );
    }
    server.initialize_personal_repository().await?;
    server.start_instructions_polling()?;
    server.start_mutation_sweep()?;
    let token = uuid::Uuid::new_v4().to_string();
    server.register_runtime_with_proxy_and_ownership(
        &pipe_path,
        proxy_path.as_ref().and_then(|path| path.to_str()),
        &token,
        launcher_owned,
    )?;
    server.start_usage_scanner()?;
    server.start_usage_sync()?;

    println!("PandaMUX Server started ({role:?}) at {pipe_path}");

    let listener_result = if let Some(proxy_path) = proxy_path.as_ref() {
        tokio::select! {
            result = run_ipc_listener(server.clone(), &pipe_path) => result,
            result = run_proxy_listener(server.clone(), proxy_path) => result,
            result = shutdown_signal() => result,
        }
    } else {
        tokio::select! {
            result = run_ipc_listener(server.clone(), &pipe_path) => result,
            result = shutdown_signal() => result,
        }
    };

    let orchestration_shutdown = server.shutdown_orchestration().await;
    server.shutdown();
    server.shutdown_provider_sessions().await;
    server.shutdown_usage_scanner().await?;
    server.shutdown_usage_sync().await?;
    server.shutdown_remote_environments().await;
    server.cleanup_runtime();
    cleanup_socket_path(&pipe_path);
    if let Some(proxy_path) = proxy_path.as_ref() {
        cleanup_socket_path(&proxy_path.to_string_lossy());
    }
    listener_result?;
    if orchestration_shutdown.timed_out
        || orchestration_shutdown.live_owned_tasks != 0
        || !orchestration_shutdown.sqlite_checkpointed
    {
        return Err(io::Error::other(format!(
            "PandaMUX shutdown retained lifecycle debt (timed_out={}, live_owned_tasks={}, sqlite_checkpointed={})",
            orchestration_shutdown.timed_out,
            orchestration_shutdown.live_owned_tasks,
            orchestration_shutdown.sqlite_checkpointed
        ))
        .into());
    }
    println!("PandaMUX Server stopped gracefully");
    Ok(())
}

async fn shutdown_signal() -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

fn parse_startup_action(args: &[String]) -> Result<StartupAction, String> {
    if let [action] = args {
        match action.as_str() {
            "-h" | "--help" | "help" => return Ok(StartupAction::Help),
            "-V" | "--version" | "version" => return Ok(StartupAction::Version),
            _ => {}
        }
    }
    if args
        .first()
        .is_some_and(|arg| arg == "antigravity-browser-helper")
    {
        if args.len() != 2 {
            return Err("antigravity-browser-helper requires exactly one URL".to_string());
        }
        return Ok(StartupAction::BrowserHelper(args[1].clone()));
    }

    let mut mode = None;
    let mut explicit_role = None;
    let mut runtime_dir = None;
    let mut db_path = None;
    let mut environment_id = "env-local".to_string();
    let mut launcher_owned = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => return Ok(StartupAction::Help),
            "-V" | "--version" => return Ok(StartupAction::Version),
            "daemon" | "proxy" => {
                if mode.replace(args[index].as_str()).is_some() {
                    return Err("server mode may be specified only once".to_string());
                }
                index += 1;
            }
            "--role" => {
                let value = required_value(args, index, "--role")?;
                explicit_role = Some(match value {
                    "node" => ServerRole::Node,
                    "hub" => ServerRole::Hub,
                    _ => return Err("--role must be 'hub' or 'node'".to_string()),
                });
                index += 2;
            }
            "--runtime-dir" => {
                runtime_dir = Some(PathBuf::from(required_value(args, index, "--runtime-dir")?));
                index += 2;
            }
            "--db" => {
                db_path = Some(PathBuf::from(required_value(args, index, "--db")?));
                index += 2;
            }
            "--environment-id" => {
                environment_id = required_value(args, index, "--environment-id")?.to_string();
                if environment_id.trim().is_empty() {
                    return Err("--environment-id must not be empty".to_string());
                }
                index += 2;
            }
            "--launcher-owned" => {
                launcher_owned = true;
                index += 1;
            }
            unknown => return Err(format!("unknown pandamux-server argument: {unknown}")),
        }
    }

    let proxy_mode = mode == Some("proxy");
    if mode == Some("daemon") && explicit_role == Some(ServerRole::Hub) {
        return Err("daemon mode requires role 'node'".to_string());
    }
    if proxy_mode
        && (explicit_role.is_some()
            || db_path.is_some()
            || environment_id != "env-local"
            || launcher_owned)
    {
        return Err("proxy mode accepts only --runtime-dir".to_string());
    }
    let role = explicit_role.unwrap_or(if mode == Some("daemon") {
        ServerRole::Node
    } else {
        ServerRole::Hub
    });
    Ok(StartupAction::Run(ServerRunArgs {
        proxy_mode,
        role,
        runtime_dir,
        db_path,
        environment_id,
        launcher_owned,
    }))
}

fn required_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    args.get(index + 1)
        .map(String::as_str)
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!(
        "PandaMUX Server {}\n\nUsage:\n  pandamux-server [--role <hub|node>] [--runtime-dir <DIR>] [--db <PATH>] [--environment-id <ID>] [--launcher-owned]\n  pandamux-server daemon [OPTIONS]\n  pandamux-server proxy [--runtime-dir <DIR>]\n  pandamux-server antigravity-browser-helper <URL>\n  pandamux-server --help\n  pandamux-server --version",
        env!("CARGO_PKG_VERSION")
    );
}

fn run_antigravity_browser_helper(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    if url.len() > 16_384 || url.contains(['\r', '\n', '\0']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Antigravity browser helper rejected an invalid URL",
        )
        .into());
    }
    let mut stderr = io::stderr().lock();
    writeln!(
        stderr,
        "{}{}",
        pandamux_providers::antigravity::ANTIGRAVITY_AUTH_BROWSER_MARKER,
        serde_json::to_string(url)?
    )?;
    stderr.flush()?;
    Ok(())
}

fn default_runtime_dir(_node_mode: bool) -> PathBuf {
    let base = if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_app_data).join("pandamux")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".pandamux")
    } else {
        std::env::temp_dir().join("pandamux")
    };
    base.join("run")
}

fn default_pipe_path(runtime_dir: &Path, role: ServerRole) -> io::Result<String> {
    #[cfg(windows)]
    {
        let _ = runtime_dir;
        let role = match role {
            ServerRole::Hub => "hub",
            ServerRole::Node => "node",
        };
        pandamux_server::windows_security::secure_pipe_name(role)
    }
    #[cfg(not(windows))]
    {
        let filename = if role == ServerRole::Node {
            "server.sock"
        } else {
            "pandamux.sock"
        };
        Ok(runtime_dir.join(filename).to_string_lossy().to_string())
    }
}

#[cfg(not(windows))]
async fn run_proxy_bridge(runtime_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::time::Duration;
    use tokio::io::{AsyncWriteExt, copy};

    let runtime = RuntimeInfo::read_from_dir(runtime_dir)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "node runtime record not found at {}",
                runtime_dir.join("server.json").display()
            ),
        )
    })?;
    if runtime.role != "node" {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "proxy mode requires a node daemon",
        )
        .into());
    }
    let proxy_path = runtime
        .proxy_path
        .unwrap_or_else(|| runtime_dir.join("proxy.sock").to_string_lossy().to_string());
    let socket = tokio::net::UnixStream::connect(proxy_path).await?;
    let (mut socket_reader, mut socket_writer) = socket.into_split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let hub_to_node = async {
        let copied = copy(&mut stdin, &mut socket_writer).await;
        normalize_proxy_copy(copied)?;
        normalize_proxy_write(socket_writer.shutdown().await)
    };
    let node_to_hub = async {
        let copied = copy(&mut socket_reader, &mut stdout).await;
        normalize_proxy_copy(copied)?;
        normalize_proxy_write(stdout.flush().await)
    };
    tokio::pin!(hub_to_node);
    tokio::pin!(node_to_hub);

    tokio::select! {
        result = &mut node_to_hub => result?,
        result = &mut hub_to_node => {
            result?;
            let _ = tokio::time::timeout(Duration::from_secs(2), &mut node_to_hub).await;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn normalize_proxy_copy(result: io::Result<u64>) -> io::Result<()> {
    match result {
        Ok(_) => Ok(()),
        Err(error) if expected_proxy_close(error.kind()) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(windows))]
fn normalize_proxy_write(result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if expected_proxy_close(error.kind()) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(windows))]
fn expected_proxy_close(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof
    )
}

#[cfg(windows)]
async fn run_proxy_bridge(_runtime_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "proxy mode is available on Linux nodes",
    )
    .into())
}

#[cfg(windows)]
async fn run_ipc_listener(server: Arc<Server>, pipe_path: &str) -> io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut security = pandamux_server::windows_security::UserOnlySecurity::for_current_user()?;
    let mut first_options = ServerOptions::new();
    first_options
        .first_pipe_instance(true)
        .reject_remote_clients(true);
    let mut pipe_server = unsafe {
        first_options.create_with_security_attributes_raw(pipe_path, security.as_mut_ptr())?
    };

    loop {
        pipe_server.connect().await?;
        let connected_client = pipe_server;
        let mut next_options = ServerOptions::new();
        next_options.reject_remote_clients(true);
        pipe_server = unsafe {
            next_options.create_with_security_attributes_raw(pipe_path, security.as_mut_ptr())?
        };
        let identity = match pandamux_server::windows_security::validate_named_pipe_client(
            &connected_client,
        ) {
            Ok(client) => ClientIdentity {
                description: "windows-local-pipe".to_string(),
                process_id: Some(client.process_id),
                user_id: None,
                executable: Some(client.executable),
            },
            Err(error) => {
                eprintln!("Rejected local pipe client: {error}");
                continue;
            }
        };
        let server = server.clone();
        tokio::spawn(async move {
            let _ = server
                .serve_local_connection(connected_client, identity)
                .await;
        });
    }
}

#[cfg(not(windows))]
async fn run_ipc_listener(server: Arc<Server>, socket_path: &str) -> io::Result<()> {
    use tokio::net::UnixListener;

    let path = Path::new(socket_path);
    prepare_socket_path(path)?;
    let listener = UnixListener::bind(path)?;
    set_socket_permissions(path)?;
    use std::os::unix::fs::MetadataExt;
    let socket_owner = std::fs::metadata(path)?.uid();

    loop {
        let (stream, _) = listener.accept().await?;
        let credentials = stream.peer_cred()?;
        if credentials.uid() != socket_owner {
            continue;
        }
        let identity = ClientIdentity {
            description: "unix-local-socket".to_string(),
            process_id: credentials.pid().and_then(|pid| u32::try_from(pid).ok()),
            user_id: Some(credentials.uid()),
            executable: None,
        };
        let server = server.clone();
        tokio::spawn(async move {
            let _ = server.serve_local_connection(stream, identity).await;
        });
    }
}

#[cfg(not(windows))]
async fn run_proxy_listener(server: Arc<Server>, socket_path: &Path) -> io::Result<()> {
    use tokio::net::UnixListener;

    prepare_socket_path(socket_path)?;
    let listener = UnixListener::bind(socket_path)?;
    set_socket_permissions(socket_path)?;
    use std::os::unix::fs::MetadataExt;
    let socket_owner = std::fs::metadata(socket_path)?.uid();
    loop {
        let (stream, _) = listener.accept().await?;
        if stream.peer_cred()?.uid() != socket_owner {
            continue;
        }
        let server = server.clone();
        tokio::spawn(async move {
            let _ = serve_proxy_connection(server, stream).await;
        });
    }
}

#[cfg(windows)]
async fn run_proxy_listener(_server: Arc<Server>, _socket_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "node proxy listener is available on Linux",
    ))
}

#[cfg(not(windows))]
fn prepare_socket_path(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "socket parent is not a private directory: {}",
                parent.display()
            ),
        ));
    }
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_socket() && metadata.uid() == parent_metadata.uid() =>
        {
            std::fs::remove_file(path)?
        }
        Ok(metadata) if metadata.file_type().is_socket() => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "refusing to replace socket owned by another user: {}",
                    path.display()
                ),
            ));
        }
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("refusing to replace non-socket path {}", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

#[cfg(not(windows))]
fn set_socket_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

fn cleanup_socket_path(path: &str) {
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::FileTypeExt;
        let path = Path::new(path);
        if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket()) {
            let _ = std::fs::remove_file(path);
        }
    }
    #[cfg(windows)]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod argument_tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn help_and_version_never_enter_server_run_mode() {
        assert_eq!(
            parse_startup_action(&args(&["--help"])).expect("parse help"),
            StartupAction::Help
        );
        assert_eq!(
            parse_startup_action(&args(&["--version"])).expect("parse version"),
            StartupAction::Version
        );
        assert_eq!(
            parse_startup_action(&args(&["daemon", "--help"])).expect("parse daemon help"),
            StartupAction::Help
        );
        let StartupAction::Run(run) = parse_startup_action(&args(&["--environment-id", "version"]))
            .expect("version is a valid option value")
        else {
            panic!("option value became a startup action");
        };
        assert_eq!(run.environment_id, "version");
    }

    #[test]
    fn daemon_options_are_typed_and_complete() {
        assert_eq!(
            parse_startup_action(&args(&[
                "daemon",
                "--launcher-owned",
                "--environment-id",
                "env-test",
                "--runtime-dir",
                "/tmp/pandamux-run",
                "--db",
                "/tmp/pandamux.db",
            ]))
            .expect("parse daemon"),
            StartupAction::Run(ServerRunArgs {
                proxy_mode: false,
                role: ServerRole::Node,
                runtime_dir: Some(PathBuf::from("/tmp/pandamux-run")),
                db_path: Some(PathBuf::from("/tmp/pandamux.db")),
                environment_id: "env-test".to_string(),
                launcher_owned: true,
            })
        );
    }

    #[test]
    fn proxy_and_browser_helper_modes_are_explicit() {
        assert_eq!(
            parse_startup_action(&args(&["proxy", "--runtime-dir", "/tmp/pandamux-run"]))
                .expect("parse proxy"),
            StartupAction::Run(ServerRunArgs {
                proxy_mode: true,
                role: ServerRole::Hub,
                runtime_dir: Some(PathBuf::from("/tmp/pandamux-run")),
                db_path: None,
                environment_id: "env-local".to_string(),
                launcher_owned: false,
            })
        );
        assert_eq!(
            parse_startup_action(&args(&[
                "antigravity-browser-helper",
                "https://example.test/callback"
            ]))
            .expect("parse browser helper"),
            StartupAction::BrowserHelper("https://example.test/callback".to_string())
        );
    }

    #[test]
    fn invalid_or_incomplete_arguments_fail_closed() {
        for invalid in [
            args(&["--headless"]),
            args(&["--versoin"]),
            args(&["--role", "worker"]),
            args(&["--runtime-dir"]),
            args(&["daemon", "proxy"]),
            args(&["daemon", "--role", "hub"]),
            args(&["proxy", "--db", "/tmp/proxy.db"]),
            args(&["antigravity-browser-helper"]),
            args(&[
                "antigravity-browser-helper",
                "https://example.test",
                "extra",
            ]),
        ] {
            assert!(
                parse_startup_action(&invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }
}

#[cfg(all(test, not(windows)))]
mod unix_tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    #[test]
    fn proxy_close_classification_accepts_half_close_errors_only() {
        for expected in [
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::NotConnected,
            io::ErrorKind::UnexpectedEof,
        ] {
            assert!(expected_proxy_close(expected), "rejected {expected:?}");
        }

        for unexpected in [
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::InvalidData,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::TimedOut,
        ] {
            assert!(!expected_proxy_close(unexpected), "accepted {unexpected:?}");
        }
    }

    #[test]
    fn unix_socket_parent_and_socket_are_private() {
        let temp = tempfile::tempdir().expect("temp dir");
        let socket = temp.path().join("run").join("server.sock");
        prepare_socket_path(&socket).expect("prepare socket");
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind socket");
        set_socket_permissions(&socket).expect("set socket permissions");
        let parent_mode = std::fs::metadata(socket.parent().expect("socket parent"))
            .expect("parent metadata")
            .permissions()
            .mode()
            & 0o777;
        let socket_mode = std::fs::metadata(&socket)
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(parent_mode, 0o700);
        assert_eq!(socket_mode, 0o600);
        drop(listener);
    }

    #[test]
    fn unix_socket_preparation_rejects_symlinks_and_non_sockets() {
        let temp = tempfile::tempdir().expect("temp dir");
        let target = temp.path().join("target");
        std::fs::create_dir(&target).expect("target dir");
        let linked_parent = temp.path().join("linked");
        symlink(&target, &linked_parent).expect("parent symlink");
        assert_eq!(
            prepare_socket_path(&linked_parent.join("server.sock"))
                .expect_err("symlink parent must fail")
                .kind(),
            io::ErrorKind::PermissionDenied
        );

        let parent = temp.path().join("run");
        std::fs::create_dir(&parent).expect("run dir");
        let occupied = parent.join("server.sock");
        std::fs::write(&occupied, "not a socket").expect("occupied path");
        assert_eq!(
            prepare_socket_path(&occupied)
                .expect_err("non-socket path must not be replaced")
                .kind(),
            io::ErrorKind::AlreadyExists
        );
    }
}
