use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const MARKER: &str = "PANDAMUX_PHASE2_PTY_OK";

pub fn run_pty_smoke() -> Result<String> {
    let commands = shell_candidates();
    let mut last_error = None;

    for mut command in commands {
        match capture_command(&mut command, MARKER) {
            Ok(output) => return Ok(output),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("no shell candidates were available")))
}

pub fn run_pty_burst(lines: usize) -> Result<(String, Duration)> {
    let mut command = if cfg!(windows) {
        let mut command = CommandBuilder::new("pwsh.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-Command",
            &format!("1..{lines} | ForEach-Object {{ \"PANDAMUX_BURST_$_\" }}"),
        ]);
        command
    } else {
        let mut command = CommandBuilder::new("sh");
        command.args([
            "-lc",
            &format!("seq 1 {lines} | sed 's/^/PANDAMUX_BURST_/'"),
        ]);
        command
    };

    let start = Instant::now();
    let output = capture_command(&mut command, &format!("PANDAMUX_BURST_{lines}"))?;
    Ok((output, start.elapsed()))
}

fn shell_candidates() -> Vec<CommandBuilder> {
    if cfg!(windows) {
        vec![
            powershell_command("pwsh.exe"),
            powershell_command("powershell.exe"),
            cmd_command(),
        ]
    } else {
        vec![sh_command()]
    }
}

fn powershell_command(shell: &str) -> CommandBuilder {
    let mut command = CommandBuilder::new(shell);
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-Command",
        "Write-Output PANDAMUX_PHASE2_PTY_OK",
    ]);
    command
}

fn cmd_command() -> CommandBuilder {
    let mut command = CommandBuilder::new("cmd.exe");
    command.args(["/C", "echo PANDAMUX_PHASE2_PTY_OK"]);
    command
}

fn sh_command() -> CommandBuilder {
    let mut command = CommandBuilder::new("sh");
    command.args(["-lc", "printf '%s\\n' PANDAMUX_PHASE2_PTY_OK"]);
    command
}

fn capture_command(command: &mut CommandBuilder, marker: &str) -> Result<String> {
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("open pty")?;

    let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
    let mut writer = pair.master.take_writer().context("take pty writer")?;
    let mut child = pair
        .slave
        .spawn_command(command.clone())
        .context("spawn shell in pty")?;

    drop(pair.slave);

    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buffer[..n]).to_string();
                    if tx.send(Ok(chunk)).is_err() {
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

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = String::new();
    let mut status = None;
    let mut cpr_answered = false;

    while Instant::now() < deadline {
        while let Ok(chunk) = rx.try_recv() {
            match chunk {
                Ok(chunk) => output.push_str(&chunk),
                Err(error) => return Err(anyhow!("read pty output: {error}")),
            }
        }

        if !cpr_answered && output.contains("\u{1b}[6n") {
            writer
                .write_all(b"\x1b[1;1R")
                .context("answer cursor position request")?;
            writer.flush().context("flush cursor position response")?;
            cpr_answered = true;
        }

        if output.contains(marker) {
            status = child.try_wait().context("poll shell")?;
            break;
        }

        if let Some(exit_status) = child.try_wait().context("poll shell")? {
            status = Some(exit_status);
            if output.contains(marker) {
                break;
            }
        }

        thread::sleep(Duration::from_millis(20));
    }

    while let Ok(chunk) = rx.try_recv() {
        match chunk {
            Ok(chunk) => output.push_str(&chunk),
            Err(error) => return Err(anyhow!("read pty output: {error}")),
        }
    }

    if !output.contains(marker) {
        let _ = child.kill();
        return Err(anyhow!(
            "timed out waiting for pty marker, output was: {output:?}"
        ));
    }

    let status = match status {
        Some(status) => status,
        None => child.wait().context("wait for shell")?,
    };

    if !status.success() {
        return Err(anyhow!("shell exited with {status:?}"));
    }

    Ok(output)
}
