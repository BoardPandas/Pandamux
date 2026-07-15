//! SSH remote terminal sessions (plan F2) and SFTP image upload (plan F3).
//!
//! A "remote surface" is a terminal surface whose byte source is an SSH channel
//! instead of a local PTY. Architecturally it is identical to a local PTY: bytes
//! flow into the same [`TerminalGrid`], input flows out the same way, and resize
//! forwards as an SSH `window-change`. [`RemoteSessionManager`] deliberately
//! mirrors `PtySessionManager`'s synchronous API so the backend dispatcher does
//! not care which kind of session a surface has.
//!
//! Durability: the remote command wraps the login shell in
//! `tmux new -A -s pandamux-<surface>` so Claude Code (or any long-running
//! process) survives a dropped connection. On disconnect the driver task
//! reconnects with backoff and re-attaches; the grid is reset before the
//! server's repaint so the local view reconciles cleanly rather than appending
//! to stale state (plan Section 5, reset-on-reattach).
//!
//! The russh work is async, but the manager exposes a blocking API by owning a
//! multi-thread tokio runtime and bridging bytes over channels (the same shape
//! as `PtySessionManager`'s reader thread).

use crate::grid::{GridSize, TerminalGrid};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKeyWithHashAlg, load_secret_key};
use russh::{ChannelMsg, Disconnect, client};

/// How to authenticate an SSH connection. Mirrors the auth matrix proven in the
/// Phase 2 spike: a private key file, the Windows OpenSSH-compatible agent named
/// pipe (covers 1Password when present), or a password.
#[derive(Clone, Debug)]
pub enum SshAuth {
    KeyFile {
        path: PathBuf,
        passphrase: Option<String>,
    },
    Agent {
        /// Named-pipe path to the OpenSSH-compatible agent
        /// (`\\.\pipe\openssh-ssh-agent`).
        pipe_path: String,
    },
    Password {
        password: String,
    },
}

/// A remote host connection target.
#[derive(Clone, Debug)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
}

impl SshConfig {
    pub fn new(host: impl Into<String>, user: impl Into<String>, auth: SshAuth) -> Self {
        Self {
            host: host.into(),
            port: 22,
            user: user.into(),
            auth,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }
}

pub type SshResult<T> = Result<T, String>;

/// Events sent from the async driver task to the synchronous manager.
enum RemoteEvent {
    Data(Vec<u8>),
    /// A reconnection re-attached to the durable session; the grid must reset so
    /// the server repaint reconciles rather than appends (reset-on-reattach).
    Reattached,
    /// The connection dropped and reconnection is in progress.
    Disconnected,
    /// The session ended permanently (auth failure, killed, or gave up).
    Closed(Option<String>),
}

/// Control messages sent from the manager to the async driver task.
enum RemoteControl {
    Write(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    Kill,
}

struct RemoteSession {
    grid: TerminalGrid,
    size: GridSize,
    rx: Receiver<RemoteEvent>,
    control: UnboundedSender<RemoteControl>,
    /// A oneshot-style request/response for SFTP uploads, serialized onto the
    /// same runtime that owns the connection.
    sftp: UnboundedSender<SftpRequest>,
    running: bool,
    last_error: Option<String>,
}

struct SftpRequest {
    local_path: PathBuf,
    remote_path: String,
    respond: tokio::sync::oneshot::Sender<SshResult<String>>,
}

/// Owns SSH remote terminal sessions keyed by surface id. Synchronous API over
/// an internally-owned tokio runtime, so the backend dispatcher treats it like
/// [`crate::PtySessionManager`].
pub struct RemoteSessionManager {
    runtime: Arc<Runtime>,
    sessions: HashMap<String, RemoteSession>,
}

impl RemoteSessionManager {
    pub fn new() -> SshResult<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("build ssh runtime: {error}"))?;
        Ok(Self {
            runtime: Arc::new(runtime),
            sessions: HashMap::new(),
        })
    }

    pub fn has(&self, surface_id: &str) -> bool {
        self.sessions.contains_key(surface_id)
    }

    pub fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// Open a durable remote terminal for `surface_id`. Returns immediately; the
    /// connection is established on the runtime and bytes stream in via [`poll`].
    pub fn connect(
        &mut self,
        surface_id: impl Into<String>,
        config: SshConfig,
        size: GridSize,
    ) -> SshResult<()> {
        let surface_id = surface_id.into();
        if self.sessions.contains_key(&surface_id) {
            return Err(format!("remote session already exists: {surface_id}"));
        }

        let (event_tx, event_rx) = std::sync::mpsc::channel::<RemoteEvent>();
        let (control_tx, control_rx) = unbounded_channel::<RemoteControl>();
        let (sftp_tx, sftp_rx) = unbounded_channel::<SftpRequest>();

        let tmux_session = format!("pandamux-{}", sanitize_tmux(&surface_id));
        let driver = RemoteDriver {
            config,
            size,
            tmux_session,
            events: event_tx,
            control: control_rx,
            sftp: sftp_rx,
        };
        self.runtime.spawn(driver.run());

        self.sessions.insert(
            surface_id,
            RemoteSession {
                grid: TerminalGrid::new(size),
                size,
                rx: event_rx,
                control: control_tx,
                sftp: sftp_tx,
                running: true,
                last_error: None,
            },
        );
        Ok(())
    }

    /// Drain pending events for a session into its grid. Non-blocking.
    pub fn poll(&mut self, surface_id: &str) -> SshResult<()> {
        let session = self
            .sessions
            .get_mut(surface_id)
            .ok_or_else(|| format!("remote session not found: {surface_id}"))?;
        loop {
            match session.rx.try_recv() {
                Ok(RemoteEvent::Data(bytes)) => session.grid.advance(&bytes),
                Ok(RemoteEvent::Reattached) => {
                    // Reset-on-reattach: discard stale local grid state so the
                    // server's full repaint lands on a clean buffer.
                    session.grid = TerminalGrid::new(session.size);
                }
                Ok(RemoteEvent::Disconnected) => {}
                Ok(RemoteEvent::Closed(error)) => {
                    session.running = false;
                    session.last_error = error;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    session.running = false;
                    break;
                }
            }
        }
        Ok(())
    }

    pub fn write_all(&mut self, surface_id: &str, bytes: &[u8]) -> SshResult<()> {
        let session = self
            .sessions
            .get(surface_id)
            .ok_or_else(|| format!("remote session not found: {surface_id}"))?;
        session
            .control
            .send(RemoteControl::Write(bytes.to_vec()))
            .map_err(|_| format!("remote session closed: {surface_id}"))
    }

    pub fn resize(&mut self, surface_id: &str, size: GridSize) -> SshResult<()> {
        let session = self
            .sessions
            .get_mut(surface_id)
            .ok_or_else(|| format!("remote session not found: {surface_id}"))?;
        if session.size == size {
            return Ok(());
        }
        session.size = size;
        session
            .control
            .send(RemoteControl::Resize {
                cols: size.columns as u32,
                rows: size.rows as u32,
            })
            .map_err(|_| format!("remote session closed: {surface_id}"))
    }

    pub fn screen_text(&mut self, surface_id: &str) -> SshResult<String> {
        self.poll(surface_id)?;
        let session = self
            .sessions
            .get(surface_id)
            .ok_or_else(|| format!("remote session not found: {surface_id}"))?;
        Ok(session.grid.snapshot_text())
    }

    /// Styled visible screen (per-cell color/attrs) plus cursor position for a
    /// remote surface, mirroring [`crate::PtySessionManager::screen_cells`].
    pub fn screen_cells(&mut self, surface_id: &str) -> SshResult<crate::grid::ScreenCells> {
        self.poll(surface_id)?;
        let session = self
            .sessions
            .get(surface_id)
            .ok_or_else(|| format!("remote session not found: {surface_id}"))?;
        Ok(session.grid.visible_cells())
    }

    pub fn screen_text_lines(&mut self, surface_id: &str, lines: usize) -> SshResult<String> {
        let text = self.screen_text(surface_id)?;
        if lines == 0 {
            return Ok(String::new());
        }
        let all_lines = text.lines().collect::<Vec<_>>();
        let start = all_lines.len().saturating_sub(lines);
        Ok(all_lines[start..].join("\n"))
    }

    /// Whether bracketed-paste mode is active for a remote surface.
    pub fn bracketed_paste_active(&self, surface_id: &str) -> bool {
        self.sessions
            .get(surface_id)
            .map(|session| session.grid.bracketed_paste_active())
            .unwrap_or(false)
    }

    /// Drain OSC 52 clipboard-store events captured from a remote surface (the
    /// copy-over-SSH path).
    pub fn take_clipboard_stores(&self, surface_id: &str) -> Vec<crate::clipboard::ClipboardStore> {
        self.sessions
            .get(surface_id)
            .map(|session| session.grid.take_clipboard_stores())
            .unwrap_or_default()
    }

    pub fn is_running(&mut self, surface_id: &str) -> bool {
        let _ = self.poll(surface_id);
        self.sessions
            .get(surface_id)
            .map(|session| session.running)
            .unwrap_or(false)
    }

    /// Upload a local image (or any file) to the remote host over SFTP and
    /// return the remote path (plan F3). Blocks on the runtime.
    pub fn upload_image(&mut self, surface_id: &str, local_path: &str) -> SshResult<String> {
        let session = self
            .sessions
            .get(surface_id)
            .ok_or_else(|| format!("remote session not found: {surface_id}"))?;
        let extension = std::path::Path::new(local_path)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("png");
        let remote_path = format!("/tmp/pandamux-paste-{}.{extension}", uuid::Uuid::new_v4());
        let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
        session
            .sftp
            .send(SftpRequest {
                local_path: PathBuf::from(local_path),
                remote_path,
                respond: respond_tx,
            })
            .map_err(|_| format!("remote session closed: {surface_id}"))?;
        self.runtime
            .block_on(respond_rx)
            .map_err(|_| "sftp upload task dropped".to_string())?
    }

    pub fn kill(&mut self, surface_id: &str) -> SshResult<()> {
        let session = self
            .sessions
            .remove(surface_id)
            .ok_or_else(|| format!("remote session not found: {surface_id}"))?;
        let _ = session.control.send(RemoteControl::Kill);
        Ok(())
    }
}

impl Default for RemoteSessionManager {
    fn default() -> Self {
        Self::new().expect("ssh runtime should build")
    }
}

/// Replace characters that are awkward inside a tmux session name.
fn sanitize_tmux(surface_id: &str) -> String {
    surface_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The tmux-wrapped remote command: attach-or-create a durable session, falling
/// back to a plain login shell when tmux is absent (degraded, no durability).
fn remote_command(tmux_session: &str) -> String {
    format!("tmux new-session -A -s {tmux_session} 2>/dev/null || exec \"${{SHELL:-/bin/sh}}\" -l")
}

// ---------------------------------------------------------------------------
// Async driver
// ---------------------------------------------------------------------------

struct RemoteDriver {
    config: SshConfig,
    size: GridSize,
    tmux_session: String,
    events: std::sync::mpsc::Sender<RemoteEvent>,
    control: UnboundedReceiver<RemoteControl>,
    sftp: UnboundedReceiver<SftpRequest>,
}

struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // TODO(phase6-followup): known-hosts verification. Accepting for now
        // matches the Phase 2 spike; the connection manager will add pinning.
        Ok(true)
    }
}

impl RemoteDriver {
    async fn run(mut self) {
        let mut backoff = Duration::from_millis(500);
        let mut first_attempt = true;

        loop {
            match self.connect().await {
                Ok(handle) => {
                    backoff = Duration::from_millis(500);
                    if !first_attempt {
                        let _ = self.events.send(RemoteEvent::Reattached);
                    }
                    first_attempt = false;
                    match self.session_loop(handle).await {
                        SessionOutcome::Killed => {
                            let _ = self.events.send(RemoteEvent::Closed(None));
                            return;
                        }
                        SessionOutcome::Disconnected => {
                            let _ = self.events.send(RemoteEvent::Disconnected);
                        }
                    }
                }
                Err(error) => {
                    if first_attempt {
                        // A failure on the very first attempt is terminal (bad
                        // host/auth); do not spin forever.
                        let _ = self.events.send(RemoteEvent::Closed(Some(error)));
                        return;
                    }
                    let _ = self.events.send(RemoteEvent::Disconnected);
                }
            }

            // Drain any Kill that arrived while disconnected.
            if self.drain_control_for_kill() {
                let _ = self.events.send(RemoteEvent::Closed(None));
                return;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(10));
        }
    }

    fn drain_control_for_kill(&mut self) -> bool {
        while let Ok(message) = self.control.try_recv() {
            if matches!(message, RemoteControl::Kill) {
                return true;
            }
        }
        false
    }

    async fn connect(&self) -> Result<client::Handle<ClientHandler>, String> {
        let client_config = client::Config {
            inactivity_timeout: Some(Duration::from_secs(60)),
            keepalive_interval: Some(Duration::from_secs(15)),
            ..Default::default()
        };
        let mut handle = client::connect(
            Arc::new(client_config),
            (self.config.host.as_str(), self.config.port),
            ClientHandler,
        )
        .await
        .map_err(|error| format!("ssh connect {}: {error}", self.config.host))?;

        authenticate(&mut handle, &self.config).await?;
        Ok(handle)
    }

    async fn session_loop(&mut self, handle: client::Handle<ClientHandler>) -> SessionOutcome {
        let mut channel = match handle.channel_open_session().await {
            Ok(channel) => channel,
            Err(_) => return SessionOutcome::Disconnected,
        };
        if channel
            .request_pty(
                false,
                "xterm-256color",
                self.size.columns as u32,
                self.size.rows as u32,
                0,
                0,
                &[],
            )
            .await
            .is_err()
        {
            return SessionOutcome::Disconnected;
        }
        if channel
            .exec(true, remote_command(&self.tmux_session).into_bytes())
            .await
            .is_err()
        {
            return SessionOutcome::Disconnected;
        }

        loop {
            tokio::select! {
                message = channel.wait() => match message {
                    Some(ChannelMsg::Data { data }) => {
                        if self.events.send(RemoteEvent::Data(data.to_vec())).is_err() {
                            return SessionOutcome::Killed;
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        let _ = self.events.send(RemoteEvent::Data(data.to_vec()));
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        return SessionOutcome::Disconnected;
                    }
                    Some(_) => {}
                },
                control = self.control.recv() => match control {
                    Some(RemoteControl::Write(bytes)) => {
                        let _ = channel.data(&bytes[..]).await;
                    }
                    Some(RemoteControl::Resize { cols, rows }) => {
                        let _ = channel.window_change(cols, rows, 0, 0).await;
                    }
                    Some(RemoteControl::Kill) | None => {
                        let _ = channel.eof().await;
                        let _ = handle
                            .disconnect(Disconnect::ByApplication, "pandamux close", "")
                            .await;
                        return SessionOutcome::Killed;
                    }
                },
                request = self.sftp.recv() => {
                    if let Some(request) = request {
                        let result = upload_via_sftp(&handle, &request).await;
                        let _ = request.respond.send(result);
                    }
                }
            }
        }
    }
}

enum SessionOutcome {
    Killed,
    Disconnected,
}

async fn authenticate(
    handle: &mut client::Handle<ClientHandler>,
    config: &SshConfig,
) -> Result<(), String> {
    match &config.auth {
        SshAuth::KeyFile { path, passphrase } => {
            let key = load_secret_key(path, passphrase.as_deref())
                .map_err(|error| format!("load key {}: {error}", path.display()))?;
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(|error| format!("rsa hash negotiation: {error}"))?
                .flatten();
            let result = handle
                .authenticate_publickey(
                    &config.user,
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(|error| format!("publickey auth: {error}"))?;
            if result.success() {
                Ok(())
            } else {
                Err(format!("publickey auth failed for {}", config.user))
            }
        }
        SshAuth::Agent { pipe_path } => {
            authenticate_with_agent(handle, &config.user, pipe_path).await
        }
        SshAuth::Password { password } => {
            let result = handle
                .authenticate_password(&config.user, password)
                .await
                .map_err(|error| format!("password auth: {error}"))?;
            if result.success() {
                Ok(())
            } else {
                Err(format!("password auth failed for {}", config.user))
            }
        }
    }
}

#[cfg(windows)]
async fn authenticate_with_agent(
    handle: &mut client::Handle<ClientHandler>,
    user: &str,
    pipe_path: &str,
) -> Result<(), String> {
    use tokio::net::windows::named_pipe::NamedPipeClient;

    let mut agent = AgentClient::<NamedPipeClient>::connect_named_pipe(pipe_path)
        .await
        .map_err(|error| format!("connect agent pipe {pipe_path}: {error}"))?;
    let identities = agent
        .request_identities()
        .await
        .map_err(|error| format!("request agent identities: {error}"))?;
    if identities.is_empty() {
        return Err(format!("agent pipe {pipe_path} returned no identities"));
    }
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(|error| format!("rsa hash negotiation: {error}"))?
        .flatten();

    let mut attempted = Vec::new();
    for identity in identities {
        attempted.push(identity.comment().to_string());
        let result = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                handle
                    .authenticate_publickey_with(user, key, hash, &mut agent)
                    .await
            }
            AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(user, certificate, hash, &mut agent)
                    .await
            }
        };
        if let Ok(result) = result
            && result.success()
        {
            return Ok(());
        }
    }
    Err(format!(
        "agent did not authenticate {user}; tried: {}",
        attempted.join(", ")
    ))
}

#[cfg(not(windows))]
async fn authenticate_with_agent(
    _handle: &mut client::Handle<ClientHandler>,
    _user: &str,
    _pipe_path: &str,
) -> Result<(), String> {
    Err("agent auth over named pipe is only implemented on Windows".to_string())
}

async fn upload_via_sftp(
    handle: &client::Handle<ClientHandler>,
    request: &SftpRequest,
) -> SshResult<String> {
    use russh_sftp::client::SftpSession;
    use tokio::io::AsyncWriteExt;

    let bytes = tokio::fs::read(&request.local_path)
        .await
        .map_err(|error| format!("read {}: {error}", request.local_path.display()))?;

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|error| format!("open sftp channel: {error}"))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| format!("request sftp subsystem: {error}"))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| format!("start sftp session: {error}"))?;
    let mut file = sftp
        .create(&request.remote_path)
        .await
        .map_err(|error| format!("create remote {}: {error}", request.remote_path))?;
    file.write_all(&bytes)
        .await
        .map_err(|error| format!("write remote {}: {error}", request.remote_path))?;
    file.shutdown()
        .await
        .map_err(|error| format!("finalize remote {}: {error}", request.remote_path))?;
    let _ = sftp.close().await;
    Ok(request.remote_path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_tmux_replaces_non_alphanumerics() {
        assert_eq!(sanitize_tmux("surf-abc_123"), "surf-abc-123");
    }

    #[test]
    fn remote_command_wraps_tmux_with_shell_fallback() {
        let command = remote_command("pandamux-surf-1");
        assert!(command.contains("tmux new-session -A -s pandamux-surf-1"));
        assert!(command.contains("|| exec"));
    }

    #[test]
    fn manager_starts_without_connecting() {
        let manager = RemoteSessionManager::new().expect("runtime builds");
        assert!(!manager.has("surf-1"));
        assert!(manager.session_ids().is_empty());
    }

    #[test]
    fn config_builder_defaults_to_port_22() {
        let config = SshConfig::new(
            "example.com",
            "chaz",
            SshAuth::Password {
                password: "x".to_string(),
            },
        );
        assert_eq!(config.port, 22);
        assert_eq!(config.with_port(2222).port, 2222);
    }
}
