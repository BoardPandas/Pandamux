use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(windows)]
use russh::keys::agent::AgentIdentity;
#[cfg(windows)]
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKeyWithHashAlg, load_secret_key};
use russh::client;

use crate::config::{SshAuth, SshConfig, SshErrorCategory, SshFailure};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HostKey {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: String,
}

impl HostKey {
    pub fn from_config(config: &SshConfig) -> Self {
        let auth = match &config.auth {
            SshAuth::KeyFile { path, .. } => format!("key:{}", path.display()),
            SshAuth::Agent { pipe_path } => format!("agent:{pipe_path}"),
            SshAuth::Password { .. } => "password".to_string(),
        };
        Self {
            host: config.host.clone(),
            port: config.port,
            user: config.user.clone(),
            auth,
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
    /// handles, or dials a new connection on demand.
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
        let handle = Arc::new(connect_client(config).await?);
        *guard = Some(Arc::clone(&handle));
        Ok(handle)
    }

    pub async fn active_connections_count(&self) -> usize {
        let slots = self.slots.lock().await;
        let mut count = 0;
        for slot in slots.values() {
            let guard = slot.lock().await;
            if let Some(h) = guard.as_ref() {
                if !h.is_closed() {
                    count += 1;
                }
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

pub async fn connect_client(config: &SshConfig) -> Result<client::Handle<ClientHandler>, SshFailure> {
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
        let cfg = SshConfig::new("bastion.internal", "admin", SshAuth::Password { password: "p".to_string() });
        let key = HostKey::from_config(&cfg);
        assert_eq!(key.host, "bastion.internal");
        assert_eq!(key.port, 22);
        assert_eq!(key.user, "admin");
        assert_eq!(key.auth, "password");
    }

    #[tokio::test]
    async fn test_pool_creation_and_slots() {
        let pool = SshConnectionPool::new();
        assert_eq!(pool.active_connections_count().await, 0);
    }
}
