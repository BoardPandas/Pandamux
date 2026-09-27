use std::path::PathBuf;
use std::sync::Arc;
use pandamux_protocol::ServerRole;
use pandamux_server::{Server, ServerConfig};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut role = ServerRole::Hub;
    let mut runtime_dir = default_runtime_dir();
    let mut db_path = None;
    let mut environment_id = "env-local".to_string();

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--role" => {
                if let Some(r) = args.get(index + 1) {
                    role = match r.as_str() {
                        "node" => ServerRole::Node,
                        _ => ServerRole::Hub,
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

    let config = ServerConfig {
        role,
        environment_id,
        runtime_dir: runtime_dir.clone(),
        db_path,
    };

    let server = Arc::new(Server::new(config)?);

    let token = uuid::Uuid::new_v4().to_string();
    let pipe_path = default_pipe_path();
    server.register_runtime(&pipe_path, &token)?;

    println!(
        "PandaMUX Server started ({:?}) at {}",
        role, pipe_path
    );

    let server_clone = server.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!("Shutting down PandaMUX Server gracefully...");
        server_clone.shutdown();
        server_clone.cleanup_runtime();
        std::process::exit(0);
    });

    // Run IPC loop
    run_ipc_listener(server, &pipe_path).await?;

    Ok(())
}

fn default_runtime_dir() -> PathBuf {
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_app_data).join("pandamux").join("run")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".pandamux").join("run")
    } else {
        std::env::temp_dir().join("pandamux").join("run")
    }
}

fn default_pipe_path() -> String {
    #[cfg(windows)]
    {
        std::env::var("PANDAMUX_PIPE")
            .unwrap_or_else(|_| r"\\.\pipe\pandamux-hub".to_string())
    }
    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            format!("{}/.pandamux/pandamux.sock", home)
        } else {
            "/tmp/pandamux.sock".to_string()
        }
    }
}

#[cfg(windows)]
async fn run_ipc_listener(
    server: Arc<Server>,
    pipe_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut pipe_server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(pipe_path)?;

    loop {
        pipe_server.connect().await?;
        let connected_client = pipe_server;

        pipe_server = ServerOptions::new().create(pipe_path)?;

        let server = server.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(connected_client);
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(res) = server.process_line(&line).await {
                    if writer.write_all(res.as_bytes()).await.is_err() {
                        break;
                    }
                }
            }
        });
    }
}

#[cfg(not(windows))]
async fn run_ipc_listener(
    server: Arc<Server>,
    socket_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::net::UnixListener;

    let path = std::path::Path::new(socket_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(path);

    let listener = UnixListener::bind(path)?;

    loop {
        let (stream, _) = listener.accept().await?;
        let server = server.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(stream);
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(res) = server.process_line(&line).await {
                    if writer.write_all(res.as_bytes()).await.is_err() {
                        break;
                    }
                }
            }
        });
    }
}
