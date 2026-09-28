use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client;
#[cfg(windows)]
use russh::keys::agent::AgentIdentity;
#[cfg(windows)]
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKeyWithHashAlg, load_secret_key};

use crate::config::{SshAuth, SshConfig, SshErrorCategory, SshFailure};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HostKey {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: String,
    pub proxy_jump: Option<Box<HostKey>>,
}

impl HostKey {
    pub fn from_config(config: &SshConfig) -> Self {
        let auth = match &config.auth {
            SshAuth::KeyFile { path, .. } => format!("key:{}", path.display()),
            SshAuth::Agent { pipe_path } => format!("agent:{pipe_path}"),
            SshAuth::Password { .. } => "password".to_string(),
        };
        let proxy_jump = config
            .proxy_jump
            .as_ref()
            .map(|jump| Box::new(HostKey::from_config(jump)));
        Self {
            host: config.host.clone(),
            port: config.port,
            user: config.user.clone(),
            auth,
            proxy_jump,
        }
    }
}

pub type PooledHandle = Arc<client::Handle<ClientHandler>>;
pub type HostSlot = Arc<tokio::sync::Mutex<Option<PooledHandle>>>;

/// One authenticated `client::Handle` per host identity, shared by every
/// consumer (exec proxy tunnels, SFTP uploads, folder browsing).
/// Clones share the same underlying pool.
#[derive(Clone, Default)]
pub struct SshConnectionPool {
    slots: Arc<tokio::sync::Mutex<HashMap<HostKey, HostSlot>>>,
}

impl SshConnectionPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquires a live, authenticated handle for `config`. Reuses existing open
    /// handles, or dials a new connection on demand. If `config.proxy_jump` is
    /// specified, the bastion connection is also acquired and reused from the pool.
    pub async fn acquire(&self, config: &SshConfig) -> Result<PooledHandle, SshFailure> {
        let slot = {
            let mut slots = self.slots.lock().await;
            slots
                .entry(HostKey::from_config(config))
                .or_default()
                .clone()
        };
        let mut guard = slot.lock().await;
        if let Some(handle) = guard.as_ref() {
            if !handle.is_closed() {
                return Ok(Arc::clone(handle));
            }
            *guard = None;
        }

        let handle = if let Some(jump) = &config.proxy_jump {
            let jump_handle = Box::pin(self.acquire(jump)).await?;
            let channel = jump_handle
                .channel_open_direct_tcpip(&config.host, config.port as u32, "127.0.0.1", 0)
                .await
                .map_err(|error| SshFailure {
                    code: "ssh_proxy_jump_failed",
                    category: SshErrorCategory::ProxyJump,
                    message: format!(
                        "ProxyJump through {} to {}:{} failed: {error}",
                        jump.host, config.host, config.port
                    ),
                    retryable: true,
                    fingerprint: None,
                    known_hosts_line: None,
                })?;
            Arc::new(connect_client_stream(config, channel.into_stream()).await?)
        } else {
            Arc::new(connect_client(config).await?)
        };

        *guard = Some(Arc::clone(&handle));
        Ok(handle)
    }

    pub async fn active_connections_count(&self) -> usize {
        let slots = self.slots.lock().await;
        let mut count = 0;
        for slot in slots.values() {
            let guard = slot.lock().await;
            if let Some(h) = guard.as_ref()
                && !h.is_closed()
            {
                count += 1;
            }
        }
        count
    }
}

pub struct ClientHandler {
    pub host: String,
    pub port: u16,
    pub trust_unknown_host: bool,
    pub failure: Arc<Mutex<Option<SshFailure>>>,
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

pub async fn connect_client(
    config: &SshConfig,
) -> Result<client::Handle<ClientHandler>, SshFailure> {
    if let Some(jump) = &config.proxy_jump {
        let jump_handle = Box::pin(connect_client(jump)).await?;
        let channel = jump_handle
            .channel_open_direct_tcpip(&config.host, config.port as u32, "127.0.0.1", 0)
            .await
            .map_err(|error| SshFailure {
                code: "ssh_proxy_jump_failed",
                category: SshErrorCategory::ProxyJump,
                message: format!(
                    "ProxyJump through {} to {}:{} failed: {error}",
                    jump.host, config.host, config.port
                ),
                retryable: true,
                fingerprint: None,
                known_hosts_line: None,
            })?;
        return connect_client_stream(config, channel.into_stream()).await;
    }

    let client_config = client::Config {
        inactivity_timeout: None,
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

pub async fn connect_client_stream<S>(
    config: &SshConfig,
    stream: S,
) -> Result<client::Handle<ClientHandler>, SshFailure>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let client_config = client::Config {
        inactivity_timeout: None,
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
    let mut handle = client::connect_stream(Arc::new(client_config), stream, handler)
        .await
        .map_err(|error| {
            host_failure
                .lock()
                .expect("host failure lock")
                .clone()
                .unwrap_or_else(|| SshFailure {
                    code: "ssh_connection_failed",
                    category: SshErrorCategory::Connection,
                    message: format!("SSH stream connection to {} failed: {error}", config.host),
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
    Err("agent auth over named pipe is only supported on Windows in this build".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_key_from_config() {
        let cfg = SshConfig::new(
            "bastion.internal",
            "admin",
            SshAuth::Password {
                password: "p".to_string(),
            },
        );
        let key = HostKey::from_config(&cfg);
        assert_eq!(key.host, "bastion.internal");
        assert_eq!(key.port, 22);
        assert_eq!(key.user, "admin");
        assert_eq!(key.auth, "password");
        assert!(key.proxy_jump.is_none());
    }

    #[test]
    fn test_host_key_with_proxy_jump() {
        let bastion = SshConfig::new(
            "bastion.corp.net",
            "jumpuser",
            SshAuth::Password {
                password: "secret".to_string(),
            },
        )
        .with_port(2222);

        let target_direct = SshConfig::new(
            "target.internal",
            "appuser",
            SshAuth::Password {
                password: "pass".to_string(),
            },
        );

        let target_jumped = target_direct.clone().with_proxy_jump(bastion);

        let key_direct = HostKey::from_config(&target_direct);
        let key_jumped = HostKey::from_config(&target_jumped);

        assert_ne!(key_direct, key_jumped);
        assert!(key_jumped.proxy_jump.is_some());
        let jump_key = key_jumped.proxy_jump.as_ref().unwrap();
        assert_eq!(jump_key.host, "bastion.corp.net");
        assert_eq!(jump_key.port, 2222);
        assert_eq!(jump_key.user, "jumpuser");
    }

    #[tokio::test]
    async fn test_pool_creation_and_slots() {
        let pool = SshConnectionPool::new();
        assert_eq!(pool.active_connections_count().await, 0);
    }
}
