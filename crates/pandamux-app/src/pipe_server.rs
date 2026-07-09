//! The standalone (headless, no-UI) named-pipe transport. Accepts connections on
//! `\\.\pipe\pandamux`, reads one line per connection, and hands it to the shared
//! [`crate::backend`] dispatcher. When the Iced runtime is running it embeds an
//! equivalent server (see `iced_runtime`) so the live UI is the single writer;
//! this standalone path is used for headless operation and tests.

use crate::backend::Backend;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

type SharedBackend = Arc<Mutex<Backend>>;

#[cfg(windows)]
pub async fn run(pipe_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let backend = Arc::new(Mutex::new(Backend::new(true)));

    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(false)
            .create(pipe_name)?;
        server.connect().await?;
        let backend = backend.clone();

        tokio::spawn(async move {
            if let Err(error) = handle_connection(server, backend).await {
                eprintln!("pipe connection error: {error}");
            }
        });
    }
}

#[cfg(not(windows))]
pub async fn run(_pipe_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    Err("named pipes are only implemented on Windows".into())
}

async fn handle_connection<T>(stream: T, backend: SharedBackend) -> std::io::Result<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let reply = {
        let mut backend = backend.lock().await;
        let reply = backend.handle_line(line.trim());
        // Forward any OSC 52 copies captured while advancing grids to the OS
        // clipboard (best-effort; plan F1).
        backend.drain_clipboards();
        reply
    };
    let mut stream = reader.into_inner();
    stream.write_all(reply.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.shutdown().await
}
