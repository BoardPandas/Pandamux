use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::{SshConfig, SshErrorCategory, SshFailure};
use crate::exec::{ExecOutput, execute_remote_command as execute_raw_remote_command};
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
    #[serde(alias = "serverVersion")]
    pub version: String,
    #[serde(default, alias = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(default, alias = "environmentId")]
    pub environment_id: String,
    #[serde(alias = "pipePath")]
    pub socket_path: String,
    #[serde(default, alias = "proxyPath")]
    pub proxy_path: Option<String>,
    pub token: String,
    #[serde(default, alias = "launcherOwned")]
    pub launcher_owned: bool,
    #[serde(default, alias = "startedAtMs")]
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

impl RemoteBootstrapResult {
    pub fn proxy_command(&self) -> String {
        let run_dir = format!("{}/.pandamux/run", self.home.trim_end_matches('/'));
        format!(
            "{} proxy --runtime-dir {}",
            shell_quote(&self.server_path),
            shell_quote(&run_dir)
        )
    }
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

async fn execute_bootstrap_command(
    pool: &SshConnectionPool,
    config: &SshConfig,
    command: &str,
) -> Result<ExecOutput, SshFailure> {
    execute_raw_remote_command(pool, config, &format!("sh -lc {}", shell_quote(command))).await
}

/// Detect remote OS and architecture via `uname -sm` and retrieve `$HOME`.
pub async fn detect_remote_platform(
    pool: &SshConnectionPool,
    config: &SshConfig,
) -> Result<(RemotePlatform, String), SshFailure> {
    let uname_res = execute_bootstrap_command(pool, config, "uname -sm").await?;
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

    let home_res = execute_bootstrap_command(pool, config, "printf %s \"$HOME\"").await?;
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
    execute_bootstrap_command(pool, config, &mkdir_cmd).await?;

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
    let check_res = execute_bootstrap_command(pool, config, &verify_cmd).await?;

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
    let prep_res = execute_bootstrap_command(pool, config, &prep_cmd).await?;
    if prep_res.exit_code != 0 {
        let _ = execute_bootstrap_command(pool, config, &format!("rm -f \"{temp_path}\"")).await;
        return Err(bootstrap_failure(
            SshErrorCategory::CommandExec,
            format!(
                "remote SHA-256 verification failed for {kind:?} at {temp_path}: expected {expected_sha256}"
            ),
        ));
    }

    let move_cmd = RemoteBinaryManifest::remote_install_cmd(&temp_path, target_path);
    let move_res = execute_bootstrap_command(pool, config, &move_cmd).await?;
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
    environment_id: &str,
) -> Result<(DaemonOwnership, RemoteServerRuntime), SshFailure> {
    let run_dir = format!("{home}/.pandamux/run");
    let runtime_path = format!("{run_dir}/server.json");

    // Check if server.json exists and daemon is alive
    let cat_res =
        execute_bootstrap_command(pool, config, &format!("cat \"{runtime_path}\" 2>/dev/null"))
            .await?;
    if cat_res.exit_code == 0
        && let Ok(runtime) = serde_json::from_slice::<RemoteServerRuntime>(&cat_res.stdout)
    {
        let pid_check = execute_bootstrap_command(
            pool,
            config,
            &format!("kill -0 {} 2>/dev/null", runtime.pid),
        )
        .await?;
        if pid_check.exit_code == 0 {
            if runtime.environment_id != environment_id {
                return Err(SshFailure {
                    code: "ssh_daemon_environment_conflict",
                    category: SshErrorCategory::Connection,
                    message: format!(
                        "running PandaMUX node belongs to environment '{}', expected '{environment_id}'",
                        runtime.environment_id
                    ),
                    retryable: false,
                    fingerprint: None,
                    known_hosts_line: None,
                });
            }
            if runtime.version == version {
                let ownership = if runtime.launcher_owned {
                    DaemonOwnership::LauncherOwned
                } else {
                    DaemonOwnership::Discovered
                };
                return Ok((ownership, runtime));
            }
            if !runtime.launcher_owned {
                if runtime.protocol_version != pandamux_protocol::PROTOCOL_VERSION {
                    return Err(SshFailure {
                        code: "ssh_daemon_upgrade_required",
                        category: SshErrorCategory::Connection,
                        message: format!(
                            "discovered PandaMUX node {} uses protocol {}, while this Hub requires protocol {}",
                            runtime.version,
                            runtime.protocol_version,
                            pandamux_protocol::PROTOCOL_VERSION
                        ),
                        retryable: false,
                        fingerprint: None,
                        known_hosts_line: None,
                    });
                }
                return Ok((DaemonOwnership::Discovered, runtime));
            }

            stop_idle_launcher_owned_daemon(pool, config, &runtime).await?;
        }
    }

    // Otherwise start launcher-owned daemon with setsid nohup
    let data_dir = format!("{home}/.pandamux/data");
    let mkdir_cmd = format!(
        "mkdir -p {} {}",
        shell_quote(&run_dir),
        shell_quote(&data_dir)
    );
    execute_bootstrap_command(pool, config, &mkdir_cmd).await?;

    let log_path = format!("{run_dir}/daemon.log");
    let db_path = format!("{data_dir}/node.db");
    let launch_cmd = format!(
        "setsid nohup {} daemon --launcher-owned --environment-id {} --runtime-dir {} --db {} > {} 2>&1 &",
        shell_quote(server_path),
        shell_quote(environment_id),
        shell_quote(&run_dir),
        shell_quote(&db_path),
        shell_quote(&log_path)
    );
    let launch_res = execute_bootstrap_command(pool, config, &launch_cmd).await?;
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
            execute_bootstrap_command(pool, config, &format!("cat \"{runtime_path}\" 2>/dev/null"))
                .await?;
        if read_res.exit_code == 0
            && let Ok(runtime) = serde_json::from_slice::<RemoteServerRuntime>(&read_res.stdout)
            && runtime.version == version
            && runtime.environment_id == environment_id
            && runtime.launcher_owned
        {
            return Ok((DaemonOwnership::LauncherOwned, runtime));
        }
    }

    Err(bootstrap_failure(
        SshErrorCategory::Connection,
        format!("timed out waiting for remote daemon runtime file at {runtime_path}"),
    ))
}

async fn stop_idle_launcher_owned_daemon(
    pool: &SshConnectionPool,
    config: &SshConfig,
    runtime: &RemoteServerRuntime,
) -> Result<(), SshFailure> {
    let activity_command = format!(
        concat!(
            "children=$(ps -o pid= --ppid {} 2>/dev/null) || exit 2; ",
            "test -z \"$children\" || exit 1; ",
            "test -r /proc/net/unix || exit 2; ",
            "awk -v p={} '$6 == \"03\" && $8 == p {{ found=1 }} END {{ exit found ? 0 : 1 }}' /proc/net/unix; ",
            "socket_status=$?; test $socket_status -eq 1 || {{ test $socket_status -eq 0 && exit 1; exit 2; }}; ",
            "awk -v p={} '$6 == \"03\" && $8 == p {{ found=1 }} END {{ exit found ? 0 : 1 }}' /proc/net/unix; ",
            "socket_status=$?; test $socket_status -eq 1 || {{ test $socket_status -eq 0 && exit 1; exit 2; }}; ",
            "exit 0"
        ),
        runtime.pid,
        shell_quote(&runtime.socket_path),
        shell_quote(runtime.proxy_path.as_deref().unwrap_or(""))
    );
    let activity = execute_bootstrap_command(pool, config, &activity_command).await?;
    match activity.exit_code {
        0 => {}
        1 => {
            return Err(SshFailure {
                code: "ssh_daemon_upgrade_deferred",
                category: SshErrorCategory::Connection,
                message: format!(
                    "PandaMUX node {} owns active work; upgrade to the requested version is deferred",
                    runtime.version
                ),
                retryable: true,
                fingerprint: None,
                known_hosts_line: None,
            });
        }
        code => {
            return Err(bootstrap_failure(
                SshErrorCategory::CommandExec,
                format!(
                    "could not determine whether PandaMUX node {} is idle: remote check exited {code}",
                    runtime.version
                ),
            ));
        }
    }

    let terminate =
        execute_bootstrap_command(pool, config, &format!("kill {}", runtime.pid)).await?;
    if terminate.exit_code != 0 {
        return Err(bootstrap_failure(
            SshErrorCategory::CommandExec,
            format!("could not terminate idle PandaMUX node {}", runtime.pid),
        ));
    }
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let check = execute_bootstrap_command(
            pool,
            config,
            &format!("kill -0 {} 2>/dev/null", runtime.pid),
        )
        .await?;
        if check.exit_code != 0 {
            return Ok(());
        }
    }
    Err(bootstrap_failure(
        SshErrorCategory::Connection,
        format!(
            "timed out waiting for idle PandaMUX node {} to stop gracefully",
            runtime.pid
        ),
    ))
}

/// Teardown a launcher-owned remote daemon by sending SIGTERM and cleaning runtime file.
pub async fn teardown_remote_daemon(
    pool: &SshConnectionPool,
    config: &SshConfig,
    home: &str,
    pid: u32,
) -> Result<(), SshFailure> {
    let terminate =
        execute_bootstrap_command(pool, config, &format!("kill {pid} 2>/dev/null")).await?;
    if terminate.exit_code != 0 {
        return Err(bootstrap_failure(
            SshErrorCategory::CommandExec,
            format!("could not terminate launcher-owned PandaMUX node {pid}"),
        ));
    }
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let check =
            execute_bootstrap_command(pool, config, &format!("kill -0 {pid} 2>/dev/null")).await?;
        if check.exit_code != 0 {
            let runtime_path = format!("{home}/.pandamux/run/server.json");
            let _ = execute_bootstrap_command(
                pool,
                config,
                &format!("rm -f {}", shell_quote(&runtime_path)),
            )
            .await;
            return Ok(());
        }
    }
    Err(bootstrap_failure(
        SshErrorCategory::Connection,
        format!("timed out waiting for PandaMUX node {pid} to stop gracefully"),
    ))
}

/// High-level orchestration for bootstrapping a remote environment.
pub async fn bootstrap_remote_environment(
    pool: &SshConnectionPool,
    config: &SshConfig,
    manifest: &RemoteBinaryManifest,
    binary_source: &impl RemoteBinarySource,
    environment_id: &str,
) -> Result<RemoteBootstrapResult, SshFailure> {
    let (platform, home) = detect_remote_platform(pool, config).await?;
    let RemotePlatform::Linux(arch) = platform;

    let (server_path, cli_path) =
        install_remote_binaries_if_needed(pool, config, manifest, binary_source, arch, &home)
            .await?;

    let (ownership, runtime) = start_or_discover_daemon(
        pool,
        config,
        &home,
        &server_path,
        &manifest.version,
        environment_id,
    )
    .await?;

    Ok(RemoteBootstrapResult {
        platform,
        home,
        server_path,
        cli_path,
        ownership,
        runtime,
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_serialization() {
        let json = r#"{
            "pid": 12345,
            "version": "0.53.36",
            "protocolVersion": 3,
            "environmentId": "env-remote",
            "socketPath": "/home/panda/.pandamux/run/server.sock",
            "proxyPath": "/home/panda/.pandamux/run/proxy.sock",
            "token": "tok_xyz789",
            "launcherOwned": true,
            "startedAt": 1700000000
        }"#;

        let parsed: RemoteServerRuntime = serde_json::from_str(json).expect("valid parse");
        assert_eq!(parsed.pid, 12345);
        assert_eq!(parsed.environment_id, "env-remote");
        assert_eq!(parsed.version, "0.53.36");
        assert_eq!(parsed.protocol_version, 3);
        assert_eq!(parsed.socket_path, "/home/panda/.pandamux/run/server.sock");
        assert_eq!(
            parsed.proxy_path.as_deref(),
            Some("/home/panda/.pandamux/run/proxy.sock")
        );
        assert_eq!(parsed.token, "tok_xyz789");
        assert!(parsed.launcher_owned);
    }

    #[test]
    fn legacy_runtime_is_never_assumed_launcher_owned() {
        let parsed: RemoteServerRuntime = serde_json::from_str(
            r#"{
                "pid": 7,
                "serverVersion": "0.53.41",
                "environmentId": "env-remote",
                "pipePath": "/home/panda/.pandamux/run/server.sock",
                "token": "legacy-token"
            }"#,
        )
        .expect("legacy runtime parses");
        assert!(!parsed.launcher_owned);
        assert_eq!(parsed.protocol_version, 0);
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
