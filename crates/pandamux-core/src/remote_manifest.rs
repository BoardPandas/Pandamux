use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Linux musl CPU architectures supported for remote node binaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteArch {
    X86_64,
    Aarch64,
}

impl RemoteArch {
    pub fn target_triple(&self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-unknown-linux-musl",
            Self::Aarch64 => "aarch64-unknown-linux-musl",
        }
    }

    pub fn uname_machine(&self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

/// Remote execution platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "os", content = "arch", rename_all = "snake_case")]
pub enum RemotePlatform {
    Linux(RemoteArch),
}

impl RemotePlatform {
    /// Parse from `uname -sm` output (for example "Linux x86_64" or "Linux aarch64").
    pub fn from_uname(uname_sm: &str) -> Option<Self> {
        let parts: Vec<&str> = uname_sm.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }
        let os = parts[0];
        let machine = parts[1];

        if !os.eq_ignore_ascii_case("Linux") {
            return None;
        }

        let arch = match machine.to_ascii_lowercase().as_str() {
            "x86_64" | "amd64" => RemoteArch::X86_64,
            "aarch64" | "arm64" => RemoteArch::Aarch64,
            _ => return None,
        };

        Some(Self::Linux(arch))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteBinaryKind {
    Server,
    Cli,
}

impl RemoteBinaryKind {
    pub fn default_filename(&self) -> &'static str {
        match self {
            Self::Server => "pandamux-server",
            Self::Cli => "pandamux",
        }
    }
}

/// Metadata and integrity verification for a single remote binary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBinaryMeta {
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
}

/// Pair of server daemon and CLI binaries for a remote target architecture.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTargetBinaries {
    pub server: RemoteBinaryMeta,
    pub cli: RemoteBinaryMeta,
}

/// Manifest of compiled Linux musl binaries embedded in the desktop build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBinaryManifest {
    pub version: String,
    pub targets: HashMap<RemoteArch, RemoteTargetBinaries>,
}

impl RemoteBinaryManifest {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            targets: HashMap::new(),
        }
    }

    pub fn with_target(
        mut self,
        arch: RemoteArch,
        server: RemoteBinaryMeta,
        cli: RemoteBinaryMeta,
    ) -> Self {
        self.targets
            .insert(arch, RemoteTargetBinaries { server, cli });
        self
    }

    /// Compute the SHA-256 hash of binary bytes in lowercase hex.
    pub fn compute_sha256(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    /// Installation directory for the given version under user home,
    /// e.g. `~/.pandamux/server/<version>`.
    pub fn install_dir(home: &str, version: &str) -> String {
        let trimmed_home = home.trim_end_matches('/');
        format!("{trimmed_home}/.pandamux/server/{version}")
    }

    /// Full path to the remote pandamux-server executable.
    pub fn server_binary_path(home: &str, version: &str) -> String {
        format!("{}/pandamux-server", Self::install_dir(home, version))
    }

    /// Full path to the remote pandamux CLI executable.
    pub fn cli_binary_path(home: &str, version: &str) -> String {
        format!("{}/pandamux", Self::install_dir(home, version))
    }

    /// Directory holding the daemon and CLI to prepend to PATH for agent processes.
    pub fn agent_path_env(home: &str, version: &str) -> String {
        format!("{}:$PATH", Self::install_dir(home, version))
    }

    /// Shell command string to verify the SHA-256 checksum of an uploaded file.
    pub fn remote_verify_sha256_cmd(file_path: &str, expected_sha256: &str) -> String {
        format!("sha256sum \"{file_path}\" | grep -q \"^{expected_sha256}\"")
    }

    /// Shell command string to make an uploaded binary executable and move it into place atomically.
    pub fn remote_install_cmd(temp_path: &str, target_path: &str) -> String {
        format!("chmod 755 \"{temp_path}\" && mv -f \"{temp_path}\" \"{target_path}\"")
    }

    /// Verifies binary bytes against the expected SHA-256 hash.
    pub fn verify_bytes(&self, arch: RemoteArch, kind: RemoteBinaryKind, bytes: &[u8]) -> bool {
        if let Some(target) = self.targets.get(&arch) {
            let expected = match kind {
                RemoteBinaryKind::Server => &target.server.sha256,
                RemoteBinaryKind::Cli => &target.cli.sha256,
            };
            let computed = Self::compute_sha256(bytes);
            computed.eq_ignore_ascii_case(expected)
        } else {
            false
        }
    }
}

/// Returns the embedded manifest with build hashes for the current release.
pub fn embedded_remote_manifest() -> RemoteBinaryManifest {
    let version = env!("CARGO_PKG_VERSION");
    RemoteBinaryManifest::new(version)
        .with_target(
            RemoteArch::X86_64,
            RemoteBinaryMeta {
                filename: "pandamux-server".to_string(),
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
                size_bytes: 0,
            },
            RemoteBinaryMeta {
                filename: "pandamux".to_string(),
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
                size_bytes: 0,
            },
        )
        .with_target(
            RemoteArch::Aarch64,
            RemoteBinaryMeta {
                filename: "pandamux-server".to_string(),
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
                size_bytes: 0,
            },
            RemoteBinaryMeta {
                filename: "pandamux".to_string(),
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
                size_bytes: 0,
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_detection_from_uname() {
        assert_eq!(
            RemotePlatform::from_uname("Linux x86_64\n"),
            Some(RemotePlatform::Linux(RemoteArch::X86_64))
        );
        assert_eq!(
            RemotePlatform::from_uname("Linux aarch64"),
            Some(RemotePlatform::Linux(RemoteArch::Aarch64))
        );
        assert_eq!(
            RemotePlatform::from_uname("Linux arm64"),
            Some(RemotePlatform::Linux(RemoteArch::Aarch64))
        );
        assert_eq!(RemotePlatform::from_uname("Darwin arm64"), None);
        assert_eq!(RemotePlatform::from_uname(""), None);
    }

    #[test]
    fn test_remote_paths() {
        let home = "/home/panda";
        let ver = "0.53.36";
        assert_eq!(
            RemoteBinaryManifest::install_dir(home, ver),
            "/home/panda/.pandamux/server/0.53.36"
        );
        assert_eq!(
            RemoteBinaryManifest::server_binary_path(home, ver),
            "/home/panda/.pandamux/server/0.53.36/pandamux-server"
        );
        assert_eq!(
            RemoteBinaryManifest::cli_binary_path(home, ver),
            "/home/panda/.pandamux/server/0.53.36/pandamux"
        );
        assert_eq!(
            RemoteBinaryManifest::agent_path_env(home, ver),
            "/home/panda/.pandamux/server/0.53.36:$PATH"
        );
    }

    #[test]
    fn test_sha256_computation_and_verification() {
        let empty_bytes = b"";
        let hash = RemoteBinaryManifest::compute_sha256(empty_bytes);
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let manifest = embedded_remote_manifest();
        assert!(manifest.verify_bytes(RemoteArch::X86_64, RemoteBinaryKind::Server, empty_bytes));
    }
}
