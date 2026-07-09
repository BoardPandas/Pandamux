use crate::grid::{GridSize, TerminalGrid};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::error::Error;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub type PtyResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtyCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// Extra environment variables injected into the spawned child (e.g. the
    /// `PANDAMUX_*` set that shell integration, the CLI, and the orchestrator
    /// hooks read to find the pipe and identify their surface/agent).
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtyCapture {
    pub output: String,
    pub screen_text: String,
}

impl PtyCommand {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_cwd(mut self, cwd: Option<String>) -> Self {
        self.cwd = cwd;
        self
    }

    /// Replace the injected environment variables.
    pub fn with_env(mut self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        self.env = env.into_iter().collect();
        self
    }

    pub(crate) fn to_builder(&self) -> CommandBuilder {
        let mut command = CommandBuilder::new(&self.program);
        command.args(self.args.iter().map(String::as_str));
        if let Some(cwd) = &self.cwd {
            command.cwd(cwd);
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

pub fn shell_marker_command(marker: &str) -> PtyCommand {
    if cfg!(windows) {
        let command = format!("Write-Output {marker}");
        PtyCommand::new("pwsh.exe").with_args([
            "-NoLogo",
            "-NoProfile",
            "-Command",
            command.as_str(),
        ])
    } else {
        let command = format!("printf '%s\\n' {marker}");
        PtyCommand::new("sh").with_args(["-lc", command.as_str()])
    }
}

pub fn capture_pty_command(
    command: &PtyCommand,
    marker: &str,
    size: GridSize,
    timeout: Duration,
) -> PtyResult<PtyCapture> {
    let output = capture_output(&mut command.to_builder(), marker, size, timeout)?;
    let mut grid = TerminalGrid::new(size);
    grid.advance(output.as_bytes());
    Ok(PtyCapture {
        screen_text: grid.snapshot_text(),
        output,
    })
}

fn capture_output(
    command: &mut CommandBuilder,
    marker: &str,
    size: GridSize,
    timeout: Duration,
) -> PtyResult<String> {
    let pty = native_pty_system();
    let pair = pty.openpty(PtySize {
        rows: size.rows as u16,
        cols: size.columns as u16,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let mut child = pair.slave.spawn_command(command.clone())?;
    drop(pair.slave);

    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if tx
                        .send(Ok(String::from_utf8_lossy(&buffer[..n]).to_string()))
                        .is_err()
                    {
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

    let deadline = Instant::now() + timeout;
    let mut output = String::new();
    let mut status = None;
    let mut cpr_answered = false;

    while Instant::now() < deadline {
        while let Ok(chunk) = rx.try_recv() {
            match chunk {
                Ok(chunk) => output.push_str(&chunk),
                Err(error) => return Err(format!("read pty output: {error}").into()),
            }
        }

        if !cpr_answered && output.contains("\u{1b}[6n") {
            writer.write_all(b"\x1b[1;1R")?;
            writer.flush()?;
            cpr_answered = true;
        }

        if output.contains(marker) {
            status = child.try_wait()?;
            break;
        }

        if let Some(exit_status) = child.try_wait()? {
            status = Some(exit_status);
            break;
        }

        thread::sleep(Duration::from_millis(20));
    }

    while let Ok(chunk) = rx.try_recv() {
        match chunk {
            Ok(chunk) => output.push_str(&chunk),
            Err(error) => return Err(format!("read pty output: {error}").into()),
        }
    }

    if !output.contains(marker) {
        let _ = child.kill();
        return Err(format!("timed out waiting for pty marker, output was: {output:?}").into());
    }

    let status = match status {
        Some(status) => status,
        None => child.wait()?,
    };

    if !status.success() {
        return Err(format!("shell exited with {status:?}").into());
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "spawns a real shell through ConPTY, run manually during terminal work"]
    fn captures_shell_marker_into_grid() {
        let marker = "PANDAMUX_TERM_PTY_OK";
        let capture = capture_pty_command(
            &shell_marker_command(marker),
            marker,
            GridSize::new(120, 24),
            Duration::from_secs(10),
        )
        .expect("pty capture should work");

        assert!(capture.output.contains(marker));
        assert!(capture.screen_text.contains(marker));
    }
}
