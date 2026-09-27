use std::path::Path;
use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::config::{SshConfig, SshErrorCategory, SshFailure};
use crate::pool::SshConnectionPool;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFolderEntry {
    pub name: String,
    pub canonical_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFolderListing {
    pub canonical_path: String,
    pub directories: Vec<RemoteFolderEntry>,
}

fn sftp_failure(message: String) -> SshFailure {
    SshFailure {
        code: "ssh_sftp_failed",
        category: SshErrorCategory::RemotePath,
        message,
        retryable: false,
        fingerprint: None,
        known_hosts_line: None,
    }
}

pub async fn browse_remote_folders(
    pool: &SshConnectionPool,
    config: SshConfig,
    path: String,
) -> Result<RemoteFolderListing, SshFailure> {
    let handle = pool.acquire(&config).await?;
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|error| sftp_failure(format!("open SFTP channel: {error}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| sftp_failure(format!("request SFTP subsystem: {error}")))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| sftp_failure(format!("start SFTP session: {error}")))?;

    let canonical_path = sftp.canonicalize(path.clone()).await.map_err(|error| {
        sftp_failure(format!("remote folder {path} is unavailable: {error}"))
    })?;

    let metadata = sftp
        .metadata(canonical_path.clone())
        .await
        .map_err(|error| sftp_failure(format!("read remote folder {canonical_path}: {error}")))?;
    if !metadata.is_dir() {
        return Err(sftp_failure(format!(
            "remote path {canonical_path} is not a directory"
        )));
    }

    let entries = sftp
        .read_dir(canonical_path.clone())
        .await
        .map_err(|error| sftp_failure(format!("list remote folder {canonical_path}: {error}")))?;
    let mut directories = Vec::new();
    for entry in entries {
        let entry_path = entry.path();
        let Ok(resolved) = sftp.canonicalize(entry_path).await else {
            continue;
        };
        let Ok(metadata) = sftp.metadata(resolved.clone()).await else {
            continue;
        };
        if metadata.is_dir() {
            directories.push(RemoteFolderEntry {
                name: entry.file_name(),
                canonical_path: resolved,
            });
        }
    }
    let _ = sftp.close().await;
    Ok(RemoteFolderListing {
        canonical_path,
        directories,
    })
}

pub async fn read_remote_file(
    pool: &SshConnectionPool,
    config: SshConfig,
    path: String,
    max_bytes: usize,
) -> Result<String, SshFailure> {
    let handle = pool.acquire(&config).await?;
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|error| sftp_failure(format!("open sftp channel: {error}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| sftp_failure(format!("request sftp subsystem: {error}")))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| sftp_failure(format!("start sftp session: {error}")))?;
    let mut file = sftp
        .open(&path)
        .await
        .map_err(|error| sftp_failure(format!("open remote {path}: {error}")))?;

    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut chunk)
            .await
            .map_err(|error| sftp_failure(format!("read remote {path}: {error}")))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > max_bytes {
            return Err(sftp_failure(format!(
                "remote file {path} exceeds {max_bytes} bytes"
            )));
        }
    }
    let _ = sftp.close().await;
    String::from_utf8(bytes).map_err(|error| sftp_failure(format!("decode {path}: {error}")))
}

pub async fn upload_file_sftp(
    pool: &SshConnectionPool,
    config: SshConfig,
    local_path: &Path,
    remote_path: &str,
) -> Result<(), SshFailure> {
    let bytes = tokio::fs::read(local_path)
        .await
        .map_err(|error| sftp_failure(format!("read local file {}: {error}", local_path.display())))?;

    let handle = pool.acquire(&config).await?;
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|error| sftp_failure(format!("open sftp channel: {error}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| sftp_failure(format!("request sftp subsystem: {error}")))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| sftp_failure(format!("start sftp session: {error}")))?;
    let mut file = sftp
        .create(remote_path)
        .await
        .map_err(|error| sftp_failure(format!("create remote {remote_path}: {error}")))?;
    file.write_all(&bytes)
        .await
        .map_err(|error| sftp_failure(format!("write remote {remote_path}: {error}")))?;
    file.shutdown()
        .await
        .map_err(|error| sftp_failure(format!("finalize remote {remote_path}: {error}")))?;
    let _ = sftp.close().await;
    Ok(())
}
