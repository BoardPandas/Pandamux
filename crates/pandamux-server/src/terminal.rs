use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pandamux_protocol::terminal_rpc::{
    TerminalAttachParams, TerminalAttachResult, TerminalCloseParams, TerminalInfo,
    TerminalInputParams, TerminalListResult, TerminalOpenParams, TerminalOpenResult,
    TerminalResizeParams,
};
use pandamux_term::grid::GridSize;
use pandamux_term::pty::PtyCommand;
use pandamux_term::ring_buffer::{DEFAULT_RING_BUFFER_CAPACITY, TerminalRingBuffer};
use pandamux_term::session::PtySessionManager;
use pandamux_term::shell::resolve_shell;

/// Represents an active or recent terminal session managed by the server.
struct ServerTerminalSession {
    terminal_id: String,
    cwd: String,
    rows: u16,
    cols: u16,
    ring_buffer: Arc<Mutex<TerminalRingBuffer>>,
    running: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<u32>>>,
    pty_manager: Arc<Mutex<PtySessionManager>>,
}

/// Server terminal manager coordinating local PTY sessions and ring buffers.
#[derive(Clone, Default)]
pub struct TerminalServerManager {
    sessions: Arc<Mutex<HashMap<String, ServerTerminalSession>>>,
}

impl TerminalServerManager {
    /// Creates a new empty TerminalServerManager.
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Opens a new local terminal session, spawning the shell in the specified directory.
    pub fn open(&self, params: TerminalOpenParams) -> Result<TerminalOpenResult, String> {
        let terminal_id = params
            .terminal_id
            .unwrap_or_else(|| format!("term-{}", &uuid::Uuid::new_v4().to_string()[..8]));

        let rows = params.rows.unwrap_or(30).max(1);
        let cols = params.cols.unwrap_or(120).max(1);

        let cwd = params.cwd.unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

        let shell_prog = resolve_shell(params.shell.as_deref());

        let mut pty_cmd = PtyCommand::new(&shell_prog);
        pty_cmd.cwd = Some(cwd.clone());

        if let Some(env_map) = params.env {
            for (k, v) in env_map {
                pty_cmd.env.push((k, v));
            }
        }

        let pty_manager = Arc::new(Mutex::new(PtySessionManager::new()));
        let size = GridSize {
            rows: rows as usize,
            columns: cols as usize,
        };

        {
            let mut mgr = pty_manager.lock().map_err(|e| format!("Lock error: {e}"))?;
            mgr.spawn(&terminal_id, &pty_cmd, size)
                .map_err(|e| format!("Failed to spawn PTY: {e}"))?;
        }

        let ring_buffer = Arc::new(Mutex::new(TerminalRingBuffer::new(
            DEFAULT_RING_BUFFER_CAPACITY,
        )));
        let running = Arc::new(AtomicBool::new(true));
        let exit_code = Arc::new(Mutex::new(None));

        // Background drain thread pulling from PTY and writing to ring buffer
        let bg_term_id = terminal_id.clone();
        let bg_pty = Arc::clone(&pty_manager);
        let bg_rb = Arc::clone(&ring_buffer);
        let bg_running = Arc::clone(&running);
        let bg_exit = Arc::clone(&exit_code);

        std::thread::spawn(move || {
            loop {
                if !bg_running.load(Ordering::Relaxed) {
                    break;
                }

                let mut drain_bytes = Vec::new();
                let mut status_check = true;

                if let Ok(mut mgr) = bg_pty.lock() {
                    if let Ok(bytes) = mgr.drain_new_bytes(&bg_term_id) {
                        drain_bytes = bytes;
                    }

                    if let Ok(Some(code)) = mgr.try_wait(&bg_term_id) {
                        if let Ok(mut lock) = bg_exit.lock() {
                            *lock = Some(code);
                        }
                        status_check = false;
                    }
                }

                if !drain_bytes.is_empty()
                    && let Ok(mut rb) = bg_rb.lock()
                {
                    rb.write(&drain_bytes);
                }

                if !status_check {
                    bg_running.store(false, Ordering::Relaxed);
                    break;
                }

                std::thread::sleep(Duration::from_millis(16));
            }
        });

        let session = ServerTerminalSession {
            terminal_id: terminal_id.clone(),
            cwd: cwd.clone(),
            rows,
            cols,
            ring_buffer,
            running,
            exit_code,
            pty_manager,
        };

        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|e| format!("Lock error: {e}"))?;
            sessions.insert(terminal_id.clone(), session);
        }

        Ok(TerminalOpenResult {
            terminal_id,
            cwd,
            rows,
            cols,
        })
    }

    /// Lists all terminal sessions currently registered on this server.
    pub fn list(&self) -> TerminalListResult {
        let sessions = match self.sessions.lock() {
            Ok(s) => s,
            Err(_) => return TerminalListResult::default(),
        };

        let mut list = Vec::with_capacity(sessions.len());
        for session in sessions.values() {
            let head_offset = session
                .ring_buffer
                .lock()
                .map(|rb| rb.head_offset())
                .unwrap_or(0);

            list.push(TerminalInfo {
                terminal_id: session.terminal_id.clone(),
                cwd: session.cwd.clone(),
                rows: session.rows,
                cols: session.cols,
                running: session.running.load(Ordering::Relaxed),
                head_offset,
            });
        }

        TerminalListResult { terminals: list }
    }

    /// Attaches to a terminal session and retrieves buffered output since the specified offset.
    pub fn attach(&self, params: TerminalAttachParams) -> Result<TerminalAttachResult, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;

        let session = sessions
            .get(&params.terminal_id)
            .ok_or_else(|| format!("Terminal not found: {}", params.terminal_id))?;

        let rb = session
            .ring_buffer
            .lock()
            .map_err(|e| format!("Ring buffer lock error: {e}"))?;

        let (head_offset, bytes, truncated) = rb.read_since(params.since_offset.unwrap_or(0));
        let data = String::from_utf8_lossy(&bytes).to_string();

        Ok(TerminalAttachResult {
            terminal_id: session.terminal_id.clone(),
            rows: session.rows,
            cols: session.cols,
            offset: head_offset,
            data,
            truncated,
        })
    }

    /// Sends user keyboard or paste input to the terminal PTY.
    pub fn input(&self, params: TerminalInputParams) -> Result<(), String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;

        let session = sessions
            .get(&params.terminal_id)
            .ok_or_else(|| format!("Terminal not found: {}", params.terminal_id))?;

        let mut mgr = session
            .pty_manager
            .lock()
            .map_err(|e| format!("PTY manager lock error: {e}"))?;

        mgr.write_all(&params.terminal_id, params.data.as_bytes())
            .map_err(|e| format!("PTY write error: {e}"))?;

        Ok(())
    }

    /// Resizes the terminal PTY grid dimensions.
    pub fn resize(&self, params: TerminalResizeParams) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;

        let session = sessions
            .get_mut(&params.terminal_id)
            .ok_or_else(|| format!("Terminal not found: {}", params.terminal_id))?;

        session.rows = params.rows;
        session.cols = params.cols;

        let mut mgr = session
            .pty_manager
            .lock()
            .map_err(|e| format!("PTY manager lock error: {e}"))?;

        let size = GridSize {
            rows: params.rows as usize,
            columns: params.cols as usize,
        };

        mgr.resize(&params.terminal_id, size)
            .map_err(|e| format!("PTY resize error: {e}"))?;

        Ok(())
    }

    /// Closes and terminates a terminal session.
    pub fn close(&self, params: TerminalCloseParams) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;

        if let Some(session) = sessions.remove(&params.terminal_id) {
            session.running.store(false, Ordering::Relaxed);
            if let Ok(mut mgr) = session.pty_manager.lock() {
                let _ = mgr.kill(&params.terminal_id);
            }
        }

        Ok(())
    }

    /// Returns the exit code of the session if it has terminated.
    pub fn exit_code(&self, terminal_id: &str) -> Option<u32> {
        let sessions = self.sessions.lock().ok()?;
        let session = sessions.get(terminal_id)?;
        session.exit_code.lock().ok().and_then(|code| *code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_server_manager_lifecycle() {
        let manager = TerminalServerManager::new();

        let open_res = manager
            .open(TerminalOpenParams {
                terminal_id: Some("test-term-1".to_string()),
                cwd: Some(".".to_string()),
                shell: None,
                rows: Some(24),
                cols: Some(80),
                env: None,
            })
            .expect("open terminal");

        assert_eq!(open_res.terminal_id, "test-term-1");
        assert_eq!(open_res.rows, 24);
        assert_eq!(open_res.cols, 80);

        // Verify listed
        let list = manager.list();
        assert_eq!(list.terminals.len(), 1);
        assert_eq!(list.terminals[0].terminal_id, "test-term-1");

        // Attach
        let attach_res = manager
            .attach(TerminalAttachParams {
                terminal_id: "test-term-1".to_string(),
                since_offset: Some(0),
            })
            .expect("attach");
        assert_eq!(attach_res.terminal_id, "test-term-1");

        // Input
        manager
            .input(TerminalInputParams {
                terminal_id: "test-term-1".to_string(),
                data: "echo test\n".to_string(),
            })
            .expect("input");

        // Resize
        manager
            .resize(TerminalResizeParams {
                terminal_id: "test-term-1".to_string(),
                rows: 35,
                cols: 100,
            })
            .expect("resize");

        // Close
        manager
            .close(TerminalCloseParams {
                terminal_id: "test-term-1".to_string(),
            })
            .expect("close");

        let list_after = manager.list();
        assert_eq!(list_after.terminals.len(), 0);
    }
}
