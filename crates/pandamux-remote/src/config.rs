use std::path::PathBuf;

/// How to authenticate an SSH connection: a private key file,
/// the OpenSSH agent named pipe, or a password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshAuth {
    KeyFile {
        path: PathBuf,
        passphrase: Option<String>,
    },
    Agent {
        /// Named-pipe or socket path to the OpenSSH-compatible agent
        pipe_path: String,
    },
    Password {
        password: String,
    },
}

/// A remote host connection target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    pub remote_cwd: Option<String>,
    pub trust_unknown_host: bool,
}

impl SshConfig {
    pub fn new(host: impl Into<String>, user: impl Into<String>, auth: SshAuth) -> Self {
        Self {
            host: host.into(),
            port: 22,
            user: user.into(),
            auth,
            remote_cwd: None,
            trust_unknown_host: false,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn with_remote_cwd(mut self, remote_cwd: impl Into<String>) -> Self {
        self.remote_cwd = Some(remote_cwd.into());
        self
    }

    pub fn with_unknown_host_trust(mut self, trust: bool) -> Self {
        self.trust_unknown_host = trust;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshErrorCategory {
    Connection,
    HostKeyUnknown,
    HostKeyChanged,
    Authentication,
    RemotePath,
    CommandExec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshFailure {
    pub code: &'static str,
    pub category: SshErrorCategory,
    pub message: String,
    pub retryable: bool,
    pub fingerprint: Option<String>,
    pub known_hosts_line: Option<usize>,
}

impl std::fmt::Display for SshFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SshFailure {}
