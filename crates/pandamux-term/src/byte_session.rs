use crate::GridSize;
use crate::cwd::CwdScanner;
use crate::pty::{PtyCommand, PtyResult};
use crate::shell;
use portable_pty::{PtySize, native_pty_system};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;

pub struct PtyByteSessionManager {
    sessions: HashMap<String, PtyByteSession>,
}

struct PtyByteSession {
    size: GridSize,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    rx: Receiver<Result<Vec<u8>, String>>,
    output: Vec<u8>,
    drained_bytes: usize,
    cpr_answered: bool,
    cwd_scanner: CwdScanner,
}

impl PtyByteSessionManager {
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
            PtyByteSession {
                size,
                _master: pair.master,
                writer,
                child,
                rx,
                output: Vec::new(),
                drained_bytes: 0,
                cpr_answered: false,
                cwd_scanner: CwdScanner::new(),
            },
        );
        Ok(())
    }

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
        Ok(())
    }

    pub fn drain_new_bytes(&mut self, session_id: &str) -> PtyResult<Vec<u8>> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("pty session not found: {session_id}"))?;
        session.poll()?;
        if session.drained_bytes < session.output.len() {
            let bytes = session.output[session.drained_bytes..].to_vec();
            session.drained_bytes = session.output.len();
            Ok(bytes)
        } else {
            Ok(Vec::new())
        }
    }

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

    pub fn try_wait(&mut self, session_id: &str) -> PtyResult<Option<u32>> {
        match self.sessions.get_mut(session_id) {
            Some(session) => match session.child.try_wait()? {
                Some(status) => Ok(Some(status.exit_code())),
                None => Ok(None),
            },
            None => Err(format!("pty session not found: {session_id}").into()),
        }
    }
}

impl Default for PtyByteSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PtyByteSessionManager {
    fn drop(&mut self) {
        let session_ids = self.sessions.keys().cloned().collect::<Vec<_>>();
        for session_id in session_ids {
            let _ = self.kill(&session_id);
        }
    }
}

impl PtyByteSession {
    fn poll(&mut self) -> PtyResult<()> {
        while let Ok(chunk) = self.rx.try_recv() {
            match chunk {
                Ok(chunk) => {
                    self.cwd_scanner.feed(&chunk);
                    self.output.extend_from_slice(&chunk);
                    if shell::contains_da1_query(&chunk) {
                        self.writer.write_all(shell::DA1_REPLY)?;
                        self.writer.flush()?;
                    }
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

fn tree_kill(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
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
            .creation_flags(0x0800_0000)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
    }
}
