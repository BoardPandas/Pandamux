use crate::config::{SshConfig, SshErrorCategory, SshFailure};
use crate::pool::SshConnectionPool;
use russh::ChannelMsg;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub exit_code: u32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

fn exec_failure(message: String) -> SshFailure {
    SshFailure {
        code: "ssh_exec_failed",
        category: SshErrorCategory::CommandExec,
        message,
        retryable: true,
        fingerprint: None,
        known_hosts_line: None,
    }
}

/// Execute a command remotely over an SSH session channel without PTY allocation.
pub async fn execute_remote_command(
    pool: &SshConnectionPool,
    config: &SshConfig,
    command: &str,
) -> Result<ExecOutput, SshFailure> {
    let handle = pool.acquire(config).await?;
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|error| exec_failure(format!("open exec channel: {error}")))?;

    channel
        .exec(true, command)
        .await
        .map_err(|error| exec_failure(format!("exec command '{command}': {error}")))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = 0;

    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => {
                stdout.extend_from_slice(data);
            }
            ChannelMsg::ExtendedData { ref data, ext: 1 } => {
                stderr.extend_from_slice(data);
            }
            ChannelMsg::ExitStatus { exit_status } => {
                exit_code = exit_status;
            }
            ChannelMsg::Close => {
                break;
            }
            _ => {}
        }
    }

    Ok(ExecOutput {
        exit_code,
        stdout,
        stderr,
    })
}
