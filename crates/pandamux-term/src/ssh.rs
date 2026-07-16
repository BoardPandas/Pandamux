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
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKeyWithHashAlg, load_secret_key};
use russh::{ChannelMsg, Disconnect, client};

/// How to authenticate an SSH connection. Mirrors the auth matrix proven in the
/// Phase 2 spike: a private key file, the Windows OpenSSH-compatible agent named
/// pipe, or a password.
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
    /// Explicit remote Project root. `None` preserves the historical login-home
    /// behavior for legacy `ssh.connect` callers.
    pub remote_cwd: Option<String>,
    /// One-shot decision from an explicit fingerprint confirmation. Unknown
    /// keys are learned only when this is true. Changed keys remain blocked.
    pub trust_unknown_host: bool,
}

impl SshConfig {
    pub fn new(host: impl Into<String>, user: impl Into<String>, auth: SshAuth) -> Self {
        Self {
            host: host.into(),
            port: 22,
            user: user.into(),
            auth,
            remote_cwd: None,
            trust_unknown_host: false,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn with_remote_cwd(mut self, remote_cwd: impl Into<String>) -> Self {
        self.remote_cwd = Some(remote_cwd.into());
        self
    }

    pub fn with_unknown_host_trust(mut self, trust: bool) -> Self {
        self.trust_unknown_host = trust;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshErrorCategory {
    Connection,
    HostKeyUnknown,
    HostKeyChanged,
    Authentication,
    RemotePath,
    PtyStart,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshFailure {
    pub code: &'static str,
    pub category: SshErrorCategory,
    pub message: String,
    pub retryable: bool,
    pub fingerprint: Option<String>,
    pub known_hosts_line: Option<usize>,
}

impl std::fmt::Display for SshFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SshFailure {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteStatus {
    Connecting,
    Ready,
    Disconnected,
    Retrying,
    Failed,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFolderEntry {
    pub name: String,
    pub canonical_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFolderListing {
    pub canonical_path: String,
    pub directories: Vec<RemoteFolderEntry>,
}

pub type SshResult<T> = Result<T, String>;

/// Events sent from the async driver task to the synchronous manager.
enum RemoteEvent {
    Data(Vec<u8>),
    /// A reconnection re-attached to the durable session; the grid must reset so
    /// the server repaint reconciles rather than appends (reset-on-reattach).
    Reattached,
    Status(RemoteStatus),
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
    status: RemoteStatus,
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
                status: RemoteStatus::Connecting,
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
                Ok(RemoteEvent::Status(status)) => session.status = status,
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

    pub fn status(&self, surface_id: &str) -> Option<RemoteStatus> {
        self.sessions.get(surface_id).map(|session| session.status)
    }

    pub fn last_error(&self, surface_id: &str) -> Option<&str> {
        self.sessions
            .get(surface_id)
            .and_then(|session| session.last_error.as_deref())
    }

    pub fn browse_folders_blocking(
        &self,
        config: SshConfig,
        path: String,
        timeout: Duration,
    ) -> Result<RemoteFolderListing, SshFailure> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        self.runtime.spawn(async move {
            let _ = sender.send(browse_remote_folders(config, path).await);
        });
        receiver.recv_timeout(timeout).map_err(|_| SshFailure {
            code: "ssh_folder_list_timeout",
            category: SshErrorCategory::Connection,
            message: "SSH folder listing timed out".to_string(),
            retryable: true,
            fingerprint: None,
            known_hosts_line: None,
        })?
    }

    /// Start a remote surface and wait until authentication, PTY allocation and
    /// command acceptance have all succeeded. On failure the prestarted session
    /// is removed so callers can commit canonical state transactionally.
    pub fn connect_ready(
        &mut self,
        surface_id: impl Into<String>,
        config: SshConfig,
        size: GridSize,
        timeout: Duration,
    ) -> SshResult<()> {
        let surface_id = surface_id.into();
        self.connect(surface_id.clone(), config, size)?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                let _ = self.kill(&surface_id);
                return Err("ssh PTY start timed out before ready".to_string());
            }
            let event = {
                let session = self
                    .sessions
                    .get_mut(&surface_id)
                    .ok_or_else(|| format!("remote session not found: {surface_id}"))?;
                session.rx.recv_timeout(remaining)
            };
            match event {
                Ok(RemoteEvent::Data(bytes)) => {
                    if let Some(session) = self.sessions.get_mut(&surface_id) {
                        session.grid.advance(&bytes);
                    }
                }
                Ok(RemoteEvent::Reattached) => {}
                Ok(RemoteEvent::Status(RemoteStatus::Ready)) => {
                    if let Some(session) = self.sessions.get_mut(&surface_id) {
                        session.status = RemoteStatus::Ready;
                    }
                    return Ok(());
                }
                Ok(RemoteEvent::Status(status)) => {
                    if let Some(session) = self.sessions.get_mut(&surface_id) {
                        session.status = status;
                    }
                }
                Ok(RemoteEvent::Closed(error)) => {
                    let message = error.unwrap_or_else(|| "remote session closed".to_string());
                    self.sessions.remove(&surface_id);
                    return Err(message);
                }
                Err(_) => {
                    self.sessions.remove(&surface_id);
                    return Err("remote session ended before ready".to_string());
                }
            }
        }
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

fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Quote a path with POSIX double quotes for use inside the launch body.
/// Double quotes (not single) keep backslashes away from single quotes in
/// the final `sh -c` payload; fish treats a backslash-quote pair inside
/// single quotes as an escape, unlike POSIX shells, so that adjacency is
/// the one thing the payload must never contain.
fn quote_posix_double(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for c in value.chars() {
        if matches!(c, '"' | '$' | '`' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// The tmux-wrapped POSIX body of the startup command: attach-or-create a
/// durable session, falling back to a plain login shell when tmux is absent
/// (degraded, no durability).
fn posix_launch_command(tmux_session: &str, remote_cwd: Option<&str>) -> String {
    match remote_cwd {
        Some(remote_cwd) => {
            let cwd = quote_posix_double(remote_cwd);
            format!(
                "tmux new-session -A -s {tmux_session} -c {cwd} 2>/dev/null || {{ cd {cwd} && exec \"${{SHELL:-/bin/sh}}\" -l; }}"
            )
        }
        None => format!(
            "tmux new-session -A -s {tmux_session} 2>/dev/null || exec \"${{SHELL:-/bin/sh}}\" -l"
        ),
    }
}

/// The exec-request command sent over the SSH channel. sshd hands this string
/// to the user's login shell (`$SHELL -c ...`), which can be fish or csh;
/// neither parses the POSIX body (`${...}` expansion, `{ ...; }` grouping).
/// Wrapping the body in `sh -c` means the login shell only has to tokenize
/// `exec sh -c <one single-quoted word>`, which POSIX shells, fish, and csh
/// all read identically (including the `'\''` quote escapes).
fn remote_command(tmux_session: &str, remote_cwd: Option<&str>) -> String {
    format!(
        "exec sh -c {}",
        quote_posix(&posix_launch_command(tmux_session, remote_cwd))
    )
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

struct ClientHandler {
    host: String,
    port: u16,
    trust_unknown_host: bool,
    failure: Arc<Mutex<Option<SshFailure>>>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        match russh::keys::check_known_hosts(&self.host, self.port, server_public_key) {
            Ok(true) => Ok(true),
            Ok(false) if self.trust_unknown_host => {
                match russh::keys::known_hosts::learn_known_hosts(
                    &self.host,
                    self.port,
                    server_public_key,
                ) {
                    Ok(()) => Ok(true),
                    Err(error) => {
                        *self.failure.lock().expect("host failure lock") = Some(SshFailure {
                            code: "ssh_host_key_write_failed",
                            category: SshErrorCategory::HostKeyUnknown,
                            message: format!(
                                "could not save the confirmed host key for {}: {error}",
                                self.host
                            ),
                            retryable: true,
                            fingerprint: Some(
                                server_public_key
                                    .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
                                    .to_string(),
                            ),
                            known_hosts_line: None,
                        });
                        Ok(false)
                    }
                }
            }
            Ok(false) => {
                let fingerprint = server_public_key
                    .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
                    .to_string();
                *self.failure.lock().expect("host failure lock") = Some(SshFailure {
                    code: "ssh_host_key_unknown",
                    category: SshErrorCategory::HostKeyUnknown,
                    message: format!(
                        "{} is not in known_hosts; confirm fingerprint {fingerprint}",
                        self.host
                    ),
                    retryable: true,
                    fingerprint: Some(fingerprint),
                    known_hosts_line: None,
                });
                Ok(false)
            }
            Err(russh::keys::Error::KeyChanged { line }) => {
                *self.failure.lock().expect("host failure lock") = Some(SshFailure {
                    code: "ssh_host_key_changed",
                    category: SshErrorCategory::HostKeyChanged,
                    message: format!(
                        "host key for {} changed; remove or repair known_hosts line {line} outside PandaMUX",
                        self.host
                    ),
                    retryable: false,
                    fingerprint: None,
                    known_hosts_line: Some(line),
                });
                Ok(false)
            }
            Err(error) => {
                *self.failure.lock().expect("host failure lock") = Some(SshFailure {
                    code: "ssh_host_key_check_failed",
                    category: SshErrorCategory::Connection,
                    message: format!("check known_hosts for {}: {error}", self.host),
                    retryable: true,
                    fingerprint: None,
                    known_hosts_line: None,
                });
                Ok(false)
            }
        }
    }
}

impl RemoteDriver {
    async fn run(mut self) {
        let mut backoff = Duration::from_millis(500);
        let mut ever_ready = false;
        let _ = self
            .events
            .send(RemoteEvent::Status(RemoteStatus::Connecting));

        loop {
            match self.connect().await {
                Ok(handle) => {
                    backoff = Duration::from_millis(500);
                    match self.session_loop(handle, ever_ready).await {
                        Ok(SessionOutcome::Killed) => {
                            let _ = self.events.send(RemoteEvent::Status(RemoteStatus::Closed));
                            let _ = self.events.send(RemoteEvent::Closed(None));
                            return;
                        }
                        Ok(SessionOutcome::Disconnected) => {
                            ever_ready = true;
                            let _ = self
                                .events
                                .send(RemoteEvent::Status(RemoteStatus::Disconnected));
                        }
                        Err(error) if !ever_ready => {
                            let _ = self.events.send(RemoteEvent::Status(RemoteStatus::Failed));
                            let _ = self.events.send(RemoteEvent::Closed(Some(error)));
                            return;
                        }
                        Err(_) => {
                            let _ = self
                                .events
                                .send(RemoteEvent::Status(RemoteStatus::Disconnected));
                        }
                    }
                }
                Err(error) => {
                    if !ever_ready {
                        // A failure on the very first attempt is terminal (bad
                        // host/auth); do not spin forever.
                        let _ = self.events.send(RemoteEvent::Status(RemoteStatus::Failed));
                        let _ = self.events.send(RemoteEvent::Closed(Some(error)));
                        return;
                    }
                    let _ = self
                        .events
                        .send(RemoteEvent::Status(RemoteStatus::Disconnected));
                }
            }

            // Drain any Kill that arrived while disconnected.
            if self.drain_control_for_kill() {
                let _ = self.events.send(RemoteEvent::Closed(None));
                return;
            }
            let _ = self
                .events
                .send(RemoteEvent::Status(RemoteStatus::Retrying));
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
        connect_client(&self.config)
            .await
            .map_err(|failure| failure.to_string())
    }

    async fn session_loop(
        &mut self,
        handle: client::Handle<ClientHandler>,
        reattaching: bool,
    ) -> Result<SessionOutcome, String> {
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|error| format!("open SSH session channel: {error}"))?;
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
            return Err("remote server rejected the PTY request".to_string());
        }
        if channel
            .exec(
                true,
                remote_command(&self.tmux_session, self.config.remote_cwd.as_deref()).into_bytes(),
            )
            .await
            .is_err()
        {
            return Err("remote server rejected the startup command".to_string());
        }
        if reattaching {
            let _ = self.events.send(RemoteEvent::Reattached);
        }
        let _ = self.events.send(RemoteEvent::Status(RemoteStatus::Ready));

        loop {
            tokio::select! {
                message = channel.wait() => match message {
                    Some(ChannelMsg::Data { data }) => {
                        if self.events.send(RemoteEvent::Data(data.to_vec())).is_err() {
                            return Ok(SessionOutcome::Killed);
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        let _ = self.events.send(RemoteEvent::Data(data.to_vec()));
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        return Ok(SessionOutcome::Disconnected);
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
                        return Ok(SessionOutcome::Killed);
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

async fn connect_client(config: &SshConfig) -> Result<client::Handle<ClientHandler>, SshFailure> {
    let client_config = client::Config {
        inactivity_timeout: Some(Duration::from_secs(60)),
        keepalive_interval: Some(Duration::from_secs(15)),
        ..Default::default()
    };
    let host_failure = Arc::new(Mutex::new(None));
    let handler = ClientHandler {
        host: config.host.clone(),
        port: config.port,
        trust_unknown_host: config.trust_unknown_host,
        failure: Arc::clone(&host_failure),
    };
    let mut handle = client::connect(
        Arc::new(client_config),
        (config.host.as_str(), config.port),
        handler,
    )
    .await
    .map_err(|error| {
        host_failure
            .lock()
            .expect("host failure lock")
            .clone()
            .unwrap_or_else(|| SshFailure {
                code: "ssh_connection_failed",
                category: SshErrorCategory::Connection,
                message: format!("SSH connection to {} failed: {error}", config.host),
                retryable: true,
                fingerprint: None,
                known_hosts_line: None,
            })
    })?;

    authenticate(&mut handle, config)
        .await
        .map_err(|message| SshFailure {
            code: "ssh_auth_failed",
            category: SshErrorCategory::Authentication,
            message,
            retryable: true,
            fingerprint: None,
            known_hosts_line: None,
        })?;
    Ok(handle)
}

/// Open a temporary SSH/SFTP connection, canonicalize `path`, and list only
/// directories. This channel is independent from durable terminal sessions and
/// is closed before the result is returned.
pub async fn browse_remote_folders(
    config: SshConfig,
    path: String,
) -> Result<RemoteFolderListing, SshFailure> {
    use russh_sftp::client::SftpSession;

    let handle = connect_client(&config).await?;
    let result = async {
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|error| remote_path_failure(format!("open SFTP channel: {error}")))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|error| remote_path_failure(format!("request SFTP subsystem: {error}")))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| remote_path_failure(format!("start SFTP session: {error}")))?;
        let canonical_path = sftp.canonicalize(path.clone()).await.map_err(|error| {
            remote_path_failure(format!("remote folder {path} is unavailable: {error}"))
        })?;
        let metadata = sftp
            .metadata(canonical_path.clone())
            .await
            .map_err(|error| {
                remote_path_failure(format!("read remote folder {canonical_path}: {error}"))
            })?;
        if !metadata.is_dir() {
            return Err(remote_path_failure(format!(
                "remote path {canonical_path} is not a directory"
            )));
        }
        let entries = sftp
            .read_dir(canonical_path.clone())
            .await
            .map_err(|error| {
                remote_path_failure(format!("list remote folder {canonical_path}: {error}"))
            })?;
        let mut directories = Vec::new();
        for entry in entries {
            let entry_path = entry.path();
            let Ok(resolved) = sftp.canonicalize(entry_path).await else {
                continue;
            };
            let Ok(metadata) = sftp.metadata(resolved.clone()).await else {
                continue;
            };
            if metadata.is_dir() {
                directories.push(RemoteFolderEntry {
                    name: entry.file_name(),
                    canonical_path: resolved,
                });
            }
        }
        let _ = sftp.close().await;
        Ok(RemoteFolderListing {
            canonical_path,
            directories,
        })
    }
    .await;
    let _ = handle
        .disconnect(
            Disconnect::ByApplication,
            "pandamux folder browse complete",
            "",
        )
        .await;
    result
}

fn remote_path_failure(message: String) -> SshFailure {
    SshFailure {
        code: "ssh_remote_path_failed",
        category: SshErrorCategory::RemotePath,
        message,
        retryable: true,
        fingerprint: None,
        known_hosts_line: None,
    }
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
    fn posix_launch_command_wraps_tmux_with_shell_fallback() {
        let command = posix_launch_command("pandamux-surf-1", None);
        assert!(command.contains("tmux new-session -A -s pandamux-surf-1"));
        assert!(command.contains("|| exec"));
    }

    #[test]
    fn posix_launch_command_quotes_project_cwd_for_tmux_and_fallback() {
        let command = posix_launch_command(
            "pandamux-surf-2",
            Some("/srv/agent's projects/Panda $MUX; echo nope"),
        );
        assert!(command.contains("-c \"/srv/agent's projects/Panda \\$MUX; echo nope\""));
        assert!(command.contains("cd \"/srv/agent's projects/Panda \\$MUX; echo nope\""));
        assert_eq!(command.matches("echo nope").count(), 2);
    }

    /// Reverse of `quote_posix`: strips the outer quotes and collapses the
    /// `'\''` escapes, i.e. what any login shell hands to `sh -c`.
    fn unquote_posix(word: &str) -> String {
        word.strip_prefix('\'')
            .and_then(|rest| rest.strip_suffix('\''))
            .expect("payload is a single-quoted word")
            .replace("'\\''", "'")
    }

    #[test]
    fn remote_command_is_login_shell_agnostic() {
        // fish and csh cannot parse the POSIX body directly, so the exec
        // request must keep it inside one single-quoted `sh -c` payload.
        let cwd = Some("/srv/agent's projects/Panda MUX");
        let command = remote_command("pandamux-surf-3", cwd);
        let payload = command
            .strip_prefix("exec sh -c ")
            .expect("command starts with the sh -c wrapper");
        assert_eq!(
            unquote_posix(payload),
            posix_launch_command("pandamux-surf-3", cwd)
        );
        // fish tokenizes backslash-quote and backslash-backslash pairs inside
        // single quotes as escapes; POSIX shells take them literally. The
        // body must never contain either pair or the two login shells would
        // hand `sh` different payloads.
        let body = posix_launch_command("pandamux-surf-3", cwd);
        assert!(!body.contains("\\'"));
        assert!(!body.contains("\\\\"));
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

    #[test]
    #[ignore = "requires PANDAMUX_SSH_SMOKE_* environment and a reachable SSH host"]
    fn ssh_project_cwd_and_folder_listing_smoke() {
        let host = std::env::var("PANDAMUX_SSH_SMOKE_HOST").expect("set smoke host");
        let user = std::env::var("PANDAMUX_SSH_SMOKE_USER").expect("set smoke user");
        let cwd = std::env::var("PANDAMUX_SSH_SMOKE_CWD").expect("set smoke cwd");
        let auth = match std::env::var("PANDAMUX_SSH_SMOKE_AUTH")
            .unwrap_or_else(|_| "agent".to_string())
            .as_str()
        {
            "password" => SshAuth::Password {
                password: std::env::var("PANDAMUX_SSH_SMOKE_PASSWORD").expect("set smoke password"),
            },
            "key" => SshAuth::KeyFile {
                path: std::env::var("PANDAMUX_SSH_SMOKE_KEY")
                    .expect("set smoke key")
                    .into(),
                passphrase: std::env::var("PANDAMUX_SSH_SMOKE_PASSPHRASE").ok(),
            },
            _ => SshAuth::Agent {
                pipe_path: r"\\.\pipe\openssh-ssh-agent".to_string(),
            },
        };
        let trust_unknown = std::env::var("PANDAMUX_SSH_SMOKE_TRUST_UNKNOWN")
            .is_ok_and(|value| value.eq_ignore_ascii_case("true"));
        let config = SshConfig::new(host, user, auth).with_unknown_host_trust(trust_unknown);
        let mut manager = RemoteSessionManager::new().expect("SSH runtime");
        let listing = manager
            .browse_folders_blocking(config.clone(), cwd.clone(), Duration::from_secs(30))
            .expect("remote folder listing");
        let canonical_cwd = listing.canonical_path;
        let config = config.with_remote_cwd(canonical_cwd.clone());
        manager
            .connect_ready(
                "surf-ssh-project-cwd-smoke",
                config,
                GridSize::new(120, 30),
                Duration::from_secs(30),
            )
            .expect("remote PTY ready");
        manager
            .write_all("surf-ssh-project-cwd-smoke", b"pwd\n")
            .expect("write pwd");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut screen = String::new();
        while std::time::Instant::now() < deadline {
            screen = manager
                .screen_text("surf-ssh-project-cwd-smoke")
                .expect("read screen");
            if screen.contains(&canonical_cwd) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = manager.kill("surf-ssh-project-cwd-smoke");
        assert!(
            screen.contains(&canonical_cwd),
            "remote screen did not show {canonical_cwd}"
        );
    }
}
