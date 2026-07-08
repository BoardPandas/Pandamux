use crate::grid::{GridSize, TerminalGrid};
use crate::links::DetectedLink;
use crate::pty::{PtyCommand, PtyResult};
use crate::search::{SearchMatch, SearchOptions};
use crate::shell;
use portable_pty::{PtySize, native_pty_system};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;

pub struct PtySessionManager {
    sessions: HashMap<String, PtySession>,
}

struct PtySession {
    grid: TerminalGrid,
    size: GridSize,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    rx: Receiver<Result<Vec<u8>, String>>,
    output: Vec<u8>,
    cpr_answered: bool,
}

impl PtySessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub fn spawn(
        &mut self,
        session_id: impl Into<String>,
        command: &PtyCommand,
        size: GridSize,
    ) -> PtyResult<()> {
        let session_id = session_id.into();
        if self.sessions.contains_key(&session_id) {
            return Err(format!("pty session already exists: {session_id}").into());
        }

        let pty = native_pty_system();
        let pair = pty.openpty(PtySize {
            rows: size.rows as u16,
            cols: size.columns as u16,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let child = pair.slave.spawn_command(command.to_builder())?;
        drop(pair.slave);

        let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();
        thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(Ok(buffer[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = tx.send(Err(error.to_string()));
                        break;
                    }
                }
            }
        });

        self.sessions.insert(
            session_id,
            PtySession {
                grid: TerminalGrid::new(size),
                size,
                _master: pair.master,
                writer,
                child,
                rx,
                output: Vec::new(),
                cpr_answered: false,
            },
        );
        Ok(())
    }

    pub fn has(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    /// Write to the PTY, chunking long writes into ConPTY-friendly pieces so a
    /// single large paste cannot outrun the input pipe (see `shell::chunk_write`).
    pub fn write_all(&mut self, session_id: &str, bytes: &[u8]) -> PtyResult<()> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() <= shell::CHUNK_THRESHOLD {
            session.writer.write_all(bytes)?;
        } else {
            for chunk in shell::chunk_write(bytes, shell::CHUNK_SIZE) {
                session.writer.write_all(chunk)?;
                session.writer.flush()?;
            }
        }
        session.writer.flush()?;
        Ok(())
    }

    /// Resize the PTY. A same-size resize is dropped: forwarding it still makes
    /// the shell redraw its prompt for no reason (a doubled-prompt cause).
    pub fn resize(&mut self, session_id: &str, size: GridSize) -> PtyResult<()> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        if session.size == size {
            return Ok(());
        }
        session._master.resize(PtySize {
            rows: size.rows as u16,
            cols: size.columns as u16,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        session.size = size;
        session.grid = TerminalGrid::new(size);
        Ok(())
    }

    pub fn poll(&mut self, session_id: &str) -> PtyResult<()> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        session.poll()
    }

    pub fn screen_text(&mut self, session_id: &str) -> PtyResult<String> {
        self.poll(session_id)?;
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        Ok(session.grid.snapshot_text())
    }

    pub fn screen_text_lines(&mut self, session_id: &str, lines: usize) -> PtyResult<String> {
        let text = self.screen_text(session_id)?;
        if lines == 0 {
            return Ok(String::new());
        }
        let all_lines = text.lines().collect::<Vec<_>>();
        let start = all_lines.len().saturating_sub(lines);
        Ok(all_lines[start..].join("\n"))
    }

    pub fn output_text(&mut self, session_id: &str) -> PtyResult<String> {
        self.poll(session_id)?;
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        Ok(String::from_utf8_lossy(&session.output).to_string())
    }

    /// Full serialization (scrollback + visible) of the session's grid, the
    /// native equivalent of the xterm serialize addon.
    pub fn serialize(&mut self, session_id: &str) -> PtyResult<String> {
        self.poll(session_id)?;
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        Ok(session.grid.serialize())
    }

    /// Search the session's scrollback + visible buffer.
    pub fn search(
        &mut self,
        session_id: &str,
        query: &str,
        options: SearchOptions,
    ) -> PtyResult<Vec<SearchMatch>> {
        self.poll(session_id)?;
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        Ok(session.grid.search(query, options))
    }

    /// Detect links on the session's visible screen.
    pub fn links(&mut self, session_id: &str) -> PtyResult<Vec<DetectedLink>> {
        self.poll(session_id)?;
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        Ok(session.grid.links())
    }

    /// The session's write-cursor position as (row, column).
    pub fn cursor(&mut self, session_id: &str) -> PtyResult<(usize, usize)> {
        self.poll(session_id)?;
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        Ok(session.grid.cursor())
    }

    /// Kill the session, tree-killing the shell's whole process subtree first on
    /// Windows so grandchildren (e.g. Claude Code's persistent backend) do not
    /// orphan when the pseudoconsole closes.
    pub fn kill(&mut self, session_id: &str) -> PtyResult<()> {
        let mut session = self
            .sessions
            .remove(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        if let Some(pid) = session.child.process_id() {
            tree_kill(pid);
        }
        session.child.kill()?;
        Ok(())
    }

    pub fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }
}

impl Default for PtySessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PtySession {
    fn poll(&mut self) -> PtyResult<()> {
        while let Ok(chunk) = self.rx.try_recv() {
            match chunk {
                Ok(chunk) => {
                    self.grid.advance(&chunk);
                    self.output.extend_from_slice(&chunk);
                    // Answer DA1 (Primary Device Attributes) probes in-process so
                    // oh-my-posh / PSReadLine never stall or leak the reply onto
                    // the prompt.
                    if shell::contains_da1_query(&chunk) {
                        self.writer.write_all(shell::DA1_REPLY)?;
                        self.writer.flush()?;
                    }
                    // Answer the first CPR query so a bounded probe does not hang.
                    if !self.cpr_answered && shell::contains_cpr_query(&self.output) {
                        self.writer.write_all(shell::CPR_REPLY)?;
                        self.writer.flush()?;
                        self.cpr_answered = true;
                    }
                }
                Err(error) => return Err(format!("read pty output: {error}").into()),
            }
        }
        Ok(())
    }
}

/// Force-kill a process subtree. On Windows this walks the parent->child
/// snapshot with `taskkill /T /F`, spawned detached and best-effort, before the
/// pseudoconsole closes. A no-op on other platforms (portable-pty's own kill
/// already signals the process group there).
fn tree_kill(pid: u32) {
    #[cfg(windows)]
    {
        use std::path::Path;
        use std::process::{Command, Stdio};
        let system_root = std::env::var("SystemRoot")
            .or_else(|_| std::env::var("windir"))
            .unwrap_or_else(|_| "C:\\Windows".to_string());
        let taskkill = Path::new(&system_root)
            .join("System32")
            .join("taskkill.exe");
        let _ = Command::new(taskkill)
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::shell_marker_command;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "spawns a real shell through ConPTY, run manually during terminal work"]
    fn pty_session_manager_captures_marker() {
        let marker = "PANDAMUX_TERM_SESSION_OK";
        let mut manager = PtySessionManager::new();
        manager
            .spawn(
                "surf-test",
                &shell_marker_command(marker),
                GridSize::new(120, 24),
            )
            .expect("session should spawn");

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut screen = String::new();
        let mut output = String::new();
        while Instant::now() < deadline {
            output = manager
                .output_text("surf-test")
                .expect("output should be readable");
            screen = manager
                .screen_text("surf-test")
                .expect("screen should be readable");
            if output.contains(marker) && screen.contains(marker) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(output.contains(marker), "output was {output:?}");
        assert!(
            screen.contains(marker),
            "screen was {screen:?}, output was {output:?}"
        );
    }
}
