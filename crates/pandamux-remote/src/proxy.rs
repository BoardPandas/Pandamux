use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::config::{SshConfig, SshErrorCategory, SshFailure};
use crate::pool::SshConnectionPool;

/// Bidirectional tunnel stream to a remote PandaMUX daemon socket,
/// backed by an SSH exec session or direct streamlocal channel.
pub struct ProxyTunnel {
    stream: russh::ChannelStream<russh::client::Msg>,
}

impl ProxyTunnel {
    pub fn new(stream: russh::ChannelStream<russh::client::Msg>) -> Self {
        Self { stream }
    }

    pub fn into_inner(self) -> russh::ChannelStream<russh::client::Msg> {
        self.stream
    }
}

impl AsyncRead for ProxyTunnel {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for ProxyTunnel {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// Open an exec channel running `proxy_command` (e.g. `pandamux-server proxy`) on the remote node.
/// This connects directly to the remote daemon socket without needing any sshd port forwarding configuration.
pub async fn open_exec_proxy_tunnel(
    pool: &SshConnectionPool,
    config: &SshConfig,
    proxy_command: &str,
) -> Result<ProxyTunnel, SshFailure> {
    let handle = pool.acquire(config).await?;
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|error| SshFailure {
            code: "ssh_proxy_channel_open_failed",
            category: SshErrorCategory::CommandExec,
            message: format!("open exec session channel for proxy tunnel: {error}"),
            retryable: true,
            fingerprint: None,
            known_hosts_line: None,
        })?;

    channel
        .exec(true, proxy_command)
        .await
        .map_err(|error| SshFailure {
            code: "ssh_proxy_exec_failed",
            category: SshErrorCategory::CommandExec,
            message: format!("exec proxy command '{proxy_command}': {error}"),
            retryable: true,
            fingerprint: None,
            known_hosts_line: None,
        })?;

    Ok(ProxyTunnel::new(channel.into_stream()))
}

/// Open a direct streamlocal channel to a Unix domain socket on the remote host.
/// Used when the remote OpenSSH daemon supports StreamLocal forwarding.
pub async fn open_streamlocal_proxy_tunnel(
    pool: &SshConnectionPool,
    config: &SshConfig,
    socket_path: &str,
) -> Result<ProxyTunnel, SshFailure> {
    let handle = pool.acquire(config).await?;
    let channel = handle
        .channel_open_direct_streamlocal(socket_path)
        .await
        .map_err(|error| SshFailure {
            code: "ssh_streamlocal_failed",
            category: SshErrorCategory::Connection,
            message: format!("open direct-streamlocal to '{socket_path}': {error}"),
            retryable: true,
            fingerprint: None,
            known_hosts_line: None,
        })?;

    Ok(ProxyTunnel::new(channel.into_stream()))
}
