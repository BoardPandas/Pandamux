use pandamux_protocol::ServerRole;
use pandamux_server::{ClientIdentity, Server, ServerConfig};
#[cfg(not(windows))]
use pandamux_server::{RuntimeInfo, serve_proxy_connection};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "antigravity-browser-helper")
    {
        return run_antigravity_browser_helper(args.get(1));
    }
    let proxy_mode = args.iter().any(|arg| arg == "proxy");
    let node_mode = args.iter().any(|arg| arg == "daemon")
        || args
            .windows(2)
            .any(|pair| pair[0] == "--role" && pair[1] == "node");
    let launcher_owned = args.iter().any(|arg| arg == "--launcher-owned");

    let mut role = if node_mode {
        ServerRole::Node
    } else {
        ServerRole::Hub
    };
    let mut runtime_dir = default_runtime_dir(node_mode || proxy_mode);
    let mut db_path = None;
    let mut environment_id = "env-local".to_string();

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--role" => {
                if let Some(value) = args.get(index + 1) {
                    role = if value == "node" {
                        ServerRole::Node
                    } else {
                        ServerRole::Hub
                    };
                    index += 2;
                } else {
                    index += 1;
                }
            }
            "--runtime-dir" => {
                if let Some(dir) = args.get(index + 1) {
                    runtime_dir = PathBuf::from(dir);
                    index += 2;
                } else {
                    index += 1;
                }
            }
            "--db" => {
                if let Some(db) = args.get(index + 1) {
                    db_path = Some(PathBuf::from(db));
                    index += 2;
                } else {
                    index += 1;
                }
            }
            "--environment-id" => {
                if let Some(id) = args.get(index + 1) {
                    environment_id = id.clone();
                    index += 2;
                } else {
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }

    if proxy_mode {
        return run_proxy_bridge(&runtime_dir).await;
    }

    if role == ServerRole::Node && db_path.is_none() {
        let data_dir = runtime_dir
            .parent()
            .unwrap_or(runtime_dir.as_path())
            .join("data");
        std::fs::create_dir_all(&data_dir)?;
        db_path = Some(data_dir.join("node.db"));
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
    let token = uuid::Uuid::new_v4().to_string();
    server.register_runtime_with_proxy_and_ownership(
        &pipe_path,
        proxy_path.as_ref().and_then(|path| path.to_str()),
        &token,
        launcher_owned,
    )?;

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

    server.shutdown();
    server.shutdown_provider_sessions().await;
    server.shutdown_remote_environments().await;
    server.cleanup_runtime();
    cleanup_socket_path(&pipe_path);
    if let Some(proxy_path) = proxy_path.as_ref() {
        cleanup_socket_path(&proxy_path.to_string_lossy());
    }
    println!("PandaMUX Server stopped gracefully");
    listener_result?;
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

fn run_antigravity_browser_helper(url: Option<&String>) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    let url = url.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Antigravity browser helper requires one URL",
        )
    })?;
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
    let mut socket = tokio::net::UnixStream::connect(proxy_path).await?;
    let mut stdio = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    match tokio::io::copy_bidirectional(&mut stdio, &mut socket).await {
        Ok(_) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::UnexpectedEof
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
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

#[cfg(all(test, not(windows)))]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

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
