use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKeyWithHashAlg, load_secret_key};
use russh::{ChannelMsg, Disconnect, client};
use tokio::net::windows::named_pipe::NamedPipeClient;

#[derive(Debug, Clone)]
pub struct RusshSmokeConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: RusshAuthMode,
}

#[derive(Debug, Clone)]
pub enum RusshAuthMode {
    DirectKey {
        key_path: PathBuf,
    },
    WindowsOpenSshAgent {
        pipe_path: String,
        provider_label: String,
    },
    Password {
        password: String,
    },
}

#[derive(Debug)]
pub struct RusshSmokeReport {
    pub setup_output: String,
    pub reattach_output: String,
    pub claude_output: String,
}

impl RusshSmokeReport {
    pub fn summary(&self) -> String {
        format!(
            "PANDAMUX_RUSSH_SMOKE_OK\nsetup_bytes={}\nreattach_bytes={}\nclaude_bytes={}",
            self.setup_output.len(),
            self.reattach_output.len(),
            self.claude_output.len()
        )
    }
}

struct SmokeClient;

impl client::Handler for SmokeClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

struct RusshSession {
    handle: client::Handle<SmokeClient>,
}

impl RusshSession {
    async fn connect(config: &RusshSmokeConfig) -> Result<Self> {
        let client_config = client::Config {
            inactivity_timeout: Some(Duration::from_secs(15)),
            ..Default::default()
        };

        let mut handle = client::connect(
            Arc::new(client_config),
            (config.host.as_str(), config.port),
            SmokeClient,
        )
        .await?;

        match &config.auth {
            RusshAuthMode::DirectKey { key_path } => {
                authenticate_with_key_file(&mut handle, &config.user, key_path).await?;
            }
            RusshAuthMode::WindowsOpenSshAgent {
                pipe_path,
                provider_label,
            } => {
                authenticate_with_windows_agent(
                    &mut handle,
                    &config.user,
                    pipe_path,
                    provider_label,
                )
                .await?;
            }
            RusshAuthMode::Password { password } => {
                authenticate_with_password(&mut handle, &config.user, password).await?;
            }
        }

        Ok(Self { handle })
    }

    async fn run_pty_command(&mut self, command: &str, marker: &str) -> Result<String> {
        let mut channel = self.handle.channel_open_session().await?;
        channel
            .request_pty(false, "xterm-256color", 120, 40, 0, 0, &[])
            .await?;
        channel.exec(true, command).await?;

        let deadline = Instant::now() + Duration::from_secs(20);
        let mut output = String::new();
        let mut exit_status = None;

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = match tokio::time::timeout(remaining, channel.wait()).await {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(_) => bail!("timed out waiting for russh channel output: {output:?}"),
            };

            match message {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    output.push_str(&String::from_utf8_lossy(&data));
                }
                ChannelMsg::ExitStatus {
                    exit_status: status,
                } => {
                    exit_status = Some(status);
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }

            if output.contains(marker) && exit_status == Some(0) {
                return Ok(output);
            }
        }

        if !output.contains(marker) {
            bail!("russh command did not produce marker {marker}, output was: {output:?}");
        }

        match exit_status {
            Some(0) => Ok(output),
            Some(status) => {
                bail!("russh command exited with status {status}, output was: {output:?}")
            }
            None => Ok(output),
        }
    }

    async fn close(&mut self) -> Result<()> {
        self.handle
            .disconnect(
                Disconnect::ByApplication,
                "phase2 smoke complete",
                "English",
            )
            .await?;
        Ok(())
    }
}

async fn authenticate_with_key_file(
    handle: &mut client::Handle<SmokeClient>,
    user: &str,
    key_path: &Path,
) -> Result<()> {
    let key_pair = load_secret_key(key_path, None)
        .map_err(|error| anyhow!("load key {}: {error}", key_path.display()))?;

    let authenticated = handle
        .authenticate_publickey(
            user,
            PrivateKeyWithHashAlg::new(
                Arc::new(key_pair),
                handle.best_supported_rsa_hash().await?.flatten(),
            ),
        )
        .await?;

    if authenticated.success() {
        Ok(())
    } else {
        bail!("russh public-key authentication failed for {user}")
    }
}

async fn authenticate_with_windows_agent(
    handle: &mut client::Handle<SmokeClient>,
    user: &str,
    pipe_path: &str,
    provider_label: &str,
) -> Result<()> {
    let mut agent = AgentClient::<NamedPipeClient>::connect_named_pipe(pipe_path)
        .await
        .map_err(|error| anyhow!("connect {provider_label} pipe {pipe_path}: {error}"))?;
    let identities = agent
        .request_identities()
        .await
        .map_err(|error| anyhow!("request identities from Windows OpenSSH agent: {error}"))?;

    if identities.is_empty() {
        bail!("{provider_label} pipe {pipe_path} returned no identities");
    }

    let hash_alg = handle.best_supported_rsa_hash().await?.flatten();
    let mut attempted = Vec::new();

    for identity in identities {
        attempted.push(identity.comment().to_string());
        let result = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                handle
                    .authenticate_publickey_with(user, key, hash_alg, &mut agent)
                    .await
            }
            AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(user, certificate, hash_alg, &mut agent)
                    .await
            }
        };

        match result {
            Ok(auth_result) if auth_result.success() => return Ok(()),
            Ok(_) => {}
            Err(error) => {
                attempted.push(format!("signing error: {error}"));
            }
        }
    }

    bail!(
        "{provider_label} did not authenticate {user}; attempted identities: {}",
        attempted.join(", ")
    )
}

async fn authenticate_with_password(
    handle: &mut client::Handle<SmokeClient>,
    user: &str,
    password: &str,
) -> Result<()> {
    let authenticated = handle.authenticate_password(user, password).await?;
    if authenticated.success() {
        Ok(())
    } else {
        bail!("russh password authentication failed for {user}")
    }
}

pub fn run_galahad_smoke(config: RusshSmokeConfig) -> Result<RusshSmokeReport> {
    tokio::runtime::Runtime::new()?.block_on(async move { run_galahad_smoke_async(config).await })
}

async fn run_galahad_smoke_async(config: RusshSmokeConfig) -> Result<RusshSmokeReport> {
    let mut first = RusshSession::connect(&config).await?;
    let setup_output = first
        .run_pty_command(
            "tmux kill-session -t pandamux-russh-smoke 2>/dev/null || true; \
             tmux new-session -d -s pandamux-russh-smoke 'bash -lc \"echo PANDAMUX_RUSSH_TMUX_START; echo PANDAMUX_RUSSH_TMUX_READY; sleep 300\"'; \
             sleep 1; tmux capture-pane -pt pandamux-russh-smoke; tmux has-session -t pandamux-russh-smoke; echo PANDAMUX_RUSSH_CONNECTED",
            "PANDAMUX_RUSSH_CONNECTED",
        )
        .await?;
    first.close().await?;

    let mut second = RusshSession::connect(&config).await?;
    let reattach_output = second
        .run_pty_command(
            "tmux capture-pane -pt pandamux-russh-smoke; \
             tmux has-session -t pandamux-russh-smoke; \
             tmux kill-session -t pandamux-russh-smoke; \
             echo PANDAMUX_RUSSH_REATTACHED",
            "PANDAMUX_RUSSH_REATTACHED",
        )
        .await?;

    let claude_output = second
        .run_pty_command(
            "tmux kill-session -t pandamux-russh-claude 2>/dev/null || true; \
             tmux new-session -d -s pandamux-russh-claude 'bash -lc \"TERM=xterm-256color claude\"'; \
             sleep 4; \
             tmux capture-pane -pt pandamux-russh-claude -S -120 || true; \
             tmux kill-session -t pandamux-russh-claude 2>/dev/null || true; \
             echo PANDAMUX_RUSSH_CLAUDE_LAUNCHED",
            "PANDAMUX_RUSSH_CLAUDE_LAUNCHED",
        )
        .await?;
    second.close().await?;

    if !setup_output.contains("PANDAMUX_RUSSH_TMUX_READY")
        || !reattach_output.contains("PANDAMUX_RUSSH_TMUX_READY")
        || !claude_output.contains("Claude Code")
    {
        bail!(
            "russh smoke completed but output was missing expected content: setup={setup_output:?}, reattach={reattach_output:?}, claude={claude_output:?}"
        );
    }

    Ok(RusshSmokeReport {
        setup_output,
        reattach_output,
        claude_output,
    })
}

pub fn default_galahad_config(key_path: impl AsRef<Path>) -> RusshSmokeConfig {
    RusshSmokeConfig {
        host: "10.55.88.48".to_string(),
        port: 22,
        user: "chaz".to_string(),
        auth: RusshAuthMode::DirectKey {
            key_path: key_path.as_ref().to_path_buf(),
        },
    }
}

pub fn default_galahad_agent_config() -> RusshSmokeConfig {
    RusshSmokeConfig {
        host: "10.55.88.48".to_string(),
        port: 22,
        user: "chaz".to_string(),
        auth: windows_open_ssh_agent_auth("Windows OpenSSH agent"),
    }
}

pub fn default_galahad_one_password_config() -> RusshSmokeConfig {
    RusshSmokeConfig {
        host: "10.55.88.48".to_string(),
        port: 22,
        user: "chaz".to_string(),
        auth: windows_open_ssh_agent_auth("1Password OpenSSH-compatible agent"),
    }
}

pub fn default_galahad_password_config(password: impl Into<String>) -> RusshSmokeConfig {
    RusshSmokeConfig {
        host: "10.55.88.48".to_string(),
        port: 22,
        user: "chaz".to_string(),
        auth: RusshAuthMode::Password {
            password: password.into(),
        },
    }
}

fn windows_open_ssh_agent_auth(provider_label: &str) -> RusshAuthMode {
    RusshAuthMode::WindowsOpenSshAgent {
        pipe_path: r"\\.\pipe\openssh-ssh-agent".to_string(),
        provider_label: provider_label.to_string(),
    }
}
