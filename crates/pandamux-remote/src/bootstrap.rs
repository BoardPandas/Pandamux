use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::{SshConfig, SshErrorCategory, SshFailure};
use crate::exec::execute_remote_command;
use crate::pool::SshConnectionPool;
use crate::sftp::upload_bytes_sftp;
use pandamux_core::remote_manifest::{
    RemoteArch, RemoteBinaryKind, RemoteBinaryManifest, RemotePlatform,
};

/// Runtime information written by a running daemon to `~/.pandamux/run/server.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteServerRuntime {
    pub pid: u32,
    pub version: String,
    pub socket_path: String,
    pub token: String,
    #[serde(default)]
    pub started_at: u64,
}

/// Ownership status of the remote server daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaemonOwnership {
    /// Daemon was started by our launcher and can be torn down when idle.
    LauncherOwned,
    /// Daemon was already running on the host when we attached.
    Discovered,
}

/// Outcome of remote node bootstrapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBootstrapResult {
    pub platform: RemotePlatform,
    pub home: String,
    pub server_path: String,
    pub cli_path: String,
    pub ownership: DaemonOwnership,
    pub runtime: RemoteServerRuntime,
}

/// Provider of compiled binary bytes for target architectures.
pub trait RemoteBinarySource: Send + Sync {
    fn get_binary_bytes(&self, arch: RemoteArch, kind: RemoteBinaryKind) -> Option<Vec<u8>>;
}

/// In-memory binary provider for testing or embedded resources.
#[derive(Default)]
pub struct InMemoryBinarySource {
    bytes: std::collections::HashMap<(RemoteArch, RemoteBinaryKind), Vec<u8>>,
}

impl InMemoryBinarySource {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_binary(mut self, arch: RemoteArch, kind: RemoteBinaryKind, bytes: Vec<u8>) -> Self {
        self.bytes.insert((arch, kind), bytes);
        self
    }
}

impl RemoteBinarySource for InMemoryBinarySource {
    fn get_binary_bytes(&self, arch: RemoteArch, kind: RemoteBinaryKind) -> Option<Vec<u8>> {
        self.bytes.get(&(arch, kind)).cloned()
    }
}

fn bootstrap_failure(category: SshErrorCategory, message: String) -> SshFailure {
    SshFailure {
        code: "ssh_bootstrap_failed",
        category,
        message,
        retryable: true,
        fingerprint: None,
        known_hosts_line: None,
    }
}

/// Detect remote OS and architecture via `uname -sm` and retrieve `$HOME`.
pub async fn detect_remote_platform(
    pool: &SshConnectionPool,
    config: &SshConfig,
) -> Result<(RemotePlatform, String), SshFailure> {
    let uname_res = execute_remote_command(pool, config, "uname -sm").await?;
    if uname_res.exit_code != 0 {
        return Err(bootstrap_failure(
            SshErrorCategory::CommandExec,
            format!(
                "uname -sm failed with code {}: {}",
                uname_res.exit_code,
                String::from_utf8_lossy(&uname_res.stderr)
            ),
        ));
    }

    let uname_str = String::from_utf8_lossy(&uname_res.stdout);
    let platform = RemotePlatform::from_uname(uname_str.trim()).ok_or_else(|| {
        bootstrap_failure(
            SshErrorCategory::RemotePath,
            format!("unsupported remote operating system or architecture: {uname_str}"),
        )
    })?;

    let home_res = execute_remote_command(pool, config, "printf %s \"$HOME\"").await?;
    let home = String::from_utf8_lossy(&home_res.stdout).trim().to_string();
    if home.is_empty() {
        return Err(bootstrap_failure(
            SshErrorCategory::RemotePath,
            "remote host returned an empty $HOME directory".to_string(),
        ));
    }

    Ok((platform, home))
}

/// Check if required binaries are installed remotely, uploading and verifying if missing or altered.
pub async fn install_remote_binaries_if_needed(
    pool: &SshConnectionPool,
    config: &SshConfig,
    manifest: &RemoteBinaryManifest,
    binary_source: &impl RemoteBinarySource,
    arch: RemoteArch,
    home: &str,
) -> Result<(String, String), SshFailure> {
    let install_dir = RemoteBinaryManifest::install_dir(home, &manifest.version);
    let server_path = RemoteBinaryManifest::server_binary_path(home, &manifest.version);
    let cli_path = RemoteBinaryManifest::cli_binary_path(home, &manifest.version);

    let target_binaries = manifest.targets.get(&arch).ok_or_else(|| {
        bootstrap_failure(
            SshErrorCategory::RemotePath,
            format!("no binary manifest entries for architecture {arch:?}"),
        )
    })?;

    // Ensure remote installation directory exists
    let mkdir_cmd = format!("mkdir -p \"{install_dir}\"");
    execute_remote_command(pool, config, &mkdir_cmd).await?;

    // Check and install server
    install_single_binary(
        pool,
        config,
        binary_source,
        arch,
        RemoteBinaryKind::Server,
        &server_path,
        &target_binaries.server.sha256,
    )
    .await?;

    // Check and install CLI
    install_single_binary(
        pool,
        config,
        binary_source,
        arch,
        RemoteBinaryKind::Cli,
        &cli_path,
        &target_binaries.cli.sha256,
    )
    .await?;

    Ok((server_path, cli_path))
}

async fn install_single_binary(
    pool: &SshConnectionPool,
    config: &SshConfig,
    binary_source: &impl RemoteBinarySource,
    arch: RemoteArch,
    kind: RemoteBinaryKind,
    target_path: &str,
    expected_sha256: &str,
) -> Result<(), SshFailure> {
    let verify_cmd = RemoteBinaryManifest::remote_verify_sha256_cmd(target_path, expected_sha256);
    let check_res = execute_remote_command(pool, config, &verify_cmd).await?;

    if check_res.exit_code == 0 {
        return Ok(());
    }

    let bytes = binary_source.get_binary_bytes(arch, kind).ok_or_else(|| {
        bootstrap_failure(
            SshErrorCategory::RemotePath,
            format!("binary bytes not provided for {kind:?} on {arch:?}"),
        )
    })?;

    let temp_path = format!("{target_path}.{}.tmp", uuid::Uuid::new_v4());
    upload_bytes_sftp(pool, config, &temp_path, &bytes).await?;

    let prep_cmd = format!(
        "chmod 755 \"{temp_path}\" && {}",
        RemoteBinaryManifest::remote_verify_sha256_cmd(&temp_path, expected_sha256)
    );
    let prep_res = execute_remote_command(pool, config, &prep_cmd).await?;
    if prep_res.exit_code != 0 {
        let _ = execute_remote_command(pool, config, &format!("rm -f \"{temp_path}\"")).await;
        return Err(bootstrap_failure(
            SshErrorCategory::CommandExec,
            format!(
                "remote SHA-256 verification failed for {kind:?} at {temp_path}: expected {expected_sha256}"
            ),
        ));
    }

    let move_cmd = RemoteBinaryManifest::remote_install_cmd(&temp_path, target_path);
    let move_res = execute_remote_command(pool, config, &move_cmd).await?;
    if move_res.exit_code != 0 {
        return Err(bootstrap_failure(
            SshErrorCategory::CommandExec,
            format!("failed to move verified binary into {target_path}"),
        ));
    }

    Ok(())
}

/// Attach to an existing daemon or start a new launcher-owned background daemon.
pub async fn start_or_discover_daemon(
    pool: &SshConnectionPool,
    config: &SshConfig,
    home: &str,
    server_path: &str,
    version: &str,
) -> Result<(DaemonOwnership, RemoteServerRuntime), SshFailure> {
    let run_dir = format!("{home}/.pandamux/run");
    let runtime_path = format!("{run_dir}/server.json");

    // Check if server.json exists and daemon is alive
    let cat_res =
        execute_remote_command(pool, config, &format!("cat \"{runtime_path}\" 2>/dev/null"))
            .await?;
    if cat_res.exit_code == 0
        && let Ok(runtime) = serde_json::from_slice::<RemoteServerRuntime>(&cat_res.stdout)
    {
        let pid_check = execute_remote_command(
            pool,
            config,
            &format!("kill -0 {} 2>/dev/null", runtime.pid),
        )
        .await?;
        if pid_check.exit_code == 0 && runtime.version == version {
            return Ok((DaemonOwnership::Discovered, runtime));
        }
    }

    // Otherwise start launcher-owned daemon with setsid nohup
    let mkdir_cmd = format!("mkdir -p \"{run_dir}\"");
    execute_remote_command(pool, config, &mkdir_cmd).await?;

    let log_path = format!("{run_dir}/daemon.log");
    let launch_cmd = format!("setsid nohup \"{server_path}\" daemon > \"{log_path}\" 2>&1 &");
    let launch_res = execute_remote_command(pool, config, &launch_cmd).await?;
    if launch_res.exit_code != 0 {
        return Err(bootstrap_failure(
            SshErrorCategory::CommandExec,
            format!(
                "failed to launch daemon at {server_path}: code {}",
                launch_res.exit_code
            ),
        ));
    }

    // Poll for runtime file to be produced
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let read_res =
            execute_remote_command(pool, config, &format!("cat \"{runtime_path}\" 2>/dev/null"))
                .await?;
        if read_res.exit_code == 0
            && let Ok(runtime) = serde_json::from_slice::<RemoteServerRuntime>(&read_res.stdout)
        {
            return Ok((DaemonOwnership::LauncherOwned, runtime));
        }
    }

    Err(bootstrap_failure(
        SshErrorCategory::Connection,
        format!("timed out waiting for remote daemon runtime file at {runtime_path}"),
    ))
}

/// Teardown a launcher-owned remote daemon by sending SIGTERM and cleaning runtime file.
pub async fn teardown_remote_daemon(
    pool: &SshConnectionPool,
    config: &SshConfig,
    home: &str,
    pid: u32,
) -> Result<(), SshFailure> {
    let kill_cmd = format!(
        "kill {} 2>/dev/null; rm -f \"{home}/.pandamux/run/server.json\"",
        pid
    );
    execute_remote_command(pool, config, &kill_cmd).await?;
    Ok(())
}

/// High-level orchestration for bootstrapping a remote environment.
pub async fn bootstrap_remote_environment(
    pool: &SshConnectionPool,
    config: &SshConfig,
    manifest: &RemoteBinaryManifest,
    binary_source: &impl RemoteBinarySource,
) -> Result<RemoteBootstrapResult, SshFailure> {
    let (platform, home) = detect_remote_platform(pool, config).await?;
    let RemotePlatform::Linux(arch) = platform;

    let (server_path, cli_path) =
        install_remote_binaries_if_needed(pool, config, manifest, binary_source, arch, &home)
            .await?;

    let (ownership, runtime) =
        start_or_discover_daemon(pool, config, &home, &server_path, &manifest.version).await?;

    Ok(RemoteBootstrapResult {
        platform,
        home,
        server_path,
        cli_path,
        ownership,
        runtime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_serialization() {
        let json = r#"{
            "pid": 12345,
            "version": "0.53.36",
            "socketPath": "/home/panda/.pandamux/run/server.sock",
            "token": "tok_xyz789",
            "startedAt": 1700000000
        }"#;

        let parsed: RemoteServerRuntime = serde_json::from_str(json).expect("valid parse");
        assert_eq!(parsed.pid, 12345);
        assert_eq!(parsed.version, "0.53.36");
        assert_eq!(parsed.socket_path, "/home/panda/.pandamux/run/server.sock");
        assert_eq!(parsed.token, "tok_xyz789");
    }

    #[test]
    fn test_in_memory_binary_source() {
        let source = InMemoryBinarySource::new().with_binary(
            RemoteArch::X86_64,
            RemoteBinaryKind::Server,
            vec![1, 2, 3, 4],
        );

        assert_eq!(
            source.get_binary_bytes(RemoteArch::X86_64, RemoteBinaryKind::Server),
            Some(vec![1, 2, 3, 4])
        );
        assert_eq!(
            source.get_binary_bytes(RemoteArch::Aarch64, RemoteBinaryKind::Server),
            None
        );
    }
}
