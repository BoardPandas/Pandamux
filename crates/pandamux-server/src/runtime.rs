use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

pub const RUNTIME_FILENAME: &str = "server.json";

/// Metadata written into `server.json` for client discovery and authentication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub pid: u32,
    pub role: String,
    pub protocol_version: u32,
    pub server_version: String,
    pub pipe_path: String,
    pub token: String,
    pub started_at_ms: u64,
}

impl RuntimeInfo {
    /// Write the runtime discovery record to disk.
    pub fn write_to_dir(&self, dir: &Path) -> io::Result<PathBuf> {
        fs::create_dir_all(dir)?;
        let path = dir.join(RUNTIME_FILENAME);
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(&path, json)?;
        Ok(path)
    }

    /// Read the runtime discovery record from disk if it exists.
    pub fn read_from_dir(dir: &Path) -> io::Result<Option<Self>> {
        let path = dir.join(RUNTIME_FILENAME);
        if !path.exists() {
            return Ok(None);
        }
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
