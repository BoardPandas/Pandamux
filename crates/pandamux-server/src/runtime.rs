use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const RUNTIME_FILENAME: &str = "server.json";

/// Metadata written into `server.json` for client discovery and authentication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub pid: u32,
    pub role: String,
    pub protocol_version: u32,
    pub server_version: String,
    pub environment_id: String,
    pub pipe_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_path: Option<String>,
    pub token: String,
    #[serde(default)]
    pub mcp_token: String,
    #[serde(default)]
    pub launcher_owned: bool,
    pub started_at_ms: u64,
}

impl RuntimeInfo {
    /// Write the runtime discovery record to disk.
    pub fn write_to_dir(&self, dir: &Path) -> io::Result<PathBuf> {
        ensure_private_runtime_dir(dir)?;
        let path = dir.join(RUNTIME_FILENAME);
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let temporary = dir.join(format!(
            ".{RUNTIME_FILENAME}.{}.tmp",
            uuid::Uuid::new_v4().simple()
        ));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(&temporary, &path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(path)
    }

    /// Read the runtime discovery record from disk if it exists.
    pub fn read_from_dir(dir: &Path) -> io::Result<Option<Self>> {
        let path = dir.join(RUNTIME_FILENAME);
        if !path.exists() {
            return Ok(None);
        }
        validate_runtime_file(dir, &path)?;
        let content = fs::read_to_string(&path)?;
        let info = serde_json::from_str(&content)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Some(info))
    }

    /// Remove the runtime discovery record from disk upon shutdown.
    pub fn remove_from_dir(dir: &Path) {
        let path = dir.join(RUNTIME_FILENAME);
        let _ = fs::remove_file(path);
    }
}

fn ensure_private_runtime_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::symlink_metadata(dir)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("runtime path is not a private directory: {}", dir.display()),
            ));
        }
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn validate_runtime_file(dir: &Path, path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir_metadata = fs::symlink_metadata(dir)?;
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("runtime record is not a regular file: {}", path.display()),
            ));
        }
        if metadata.uid() != dir_metadata.uid() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "runtime record ownership or permissions are unsafe: {}",
                    path.display()
                ),
            ));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, path);
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    fn runtime_info() -> RuntimeInfo {
        RuntimeInfo {
            pid: 42,
            role: "hub".to_string(),
            protocol_version: 3,
            server_version: "test".to_string(),
            environment_id: "env-local".to_string(),
            pipe_path: "/tmp/pandamux-test.sock".to_string(),
            proxy_path: None,
            token: "control-token".to_string(),
            mcp_token: "mcp-token".to_string(),
            launcher_owned: false,
            started_at_ms: 1,
        }
    }

    #[test]
    fn runtime_directory_and_record_are_private() {
        let temp = tempfile::tempdir().expect("temp dir");
        let runtime_dir = temp.path().join("run");
        let path = runtime_info()
            .write_to_dir(&runtime_dir)
            .expect("write runtime");
        let dir_mode = fs::metadata(&runtime_dir)
            .expect("runtime dir metadata")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = fs::metadata(&path)
            .expect("runtime file metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
        assert_eq!(
            RuntimeInfo::read_from_dir(&runtime_dir)
                .expect("read runtime")
                .expect("runtime present"),
            runtime_info()
        );
    }

    #[test]
    fn runtime_paths_reject_symlink_surprises() {
        let temp = tempfile::tempdir().expect("temp dir");
        let target_dir = temp.path().join("target");
        fs::create_dir(&target_dir).expect("target dir");
        let linked_dir = temp.path().join("linked-run");
        symlink(&target_dir, &linked_dir).expect("runtime dir symlink");
        assert_eq!(
            runtime_info()
                .write_to_dir(&linked_dir)
                .expect_err("symlink runtime directory must fail")
                .kind(),
            io::ErrorKind::PermissionDenied
        );

        let runtime_dir = temp.path().join("run");
        fs::create_dir(&runtime_dir).expect("runtime dir");
        let outside = temp.path().join("outside.json");
        fs::write(&outside, "{}").expect("outside file");
        symlink(&outside, runtime_dir.join(RUNTIME_FILENAME)).expect("runtime file symlink");
        assert_eq!(
            RuntimeInfo::read_from_dir(&runtime_dir)
                .expect_err("symlink runtime record must fail")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
