use pandamux_core::EnvironmentId;
use serde::{Deserialize, Serialize};

/// Parameters for reading a file under workspace root confinement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsReadParams {
    pub root_path: String,
    pub path: String,
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
}

impl FsReadParams {
    pub fn new(root_path: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            root_path: root_path.into(),
            path: path.into(),
            environment_id: None,
        }
    }
}

/// Result returned from reading a confined file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsReadResult {
    pub relative_path: String,
    pub content: String,
    pub is_binary: bool,
    pub size_bytes: u64,
}

/// A directory entry in the confined file tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub relative_path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size_bytes: u64,
}

/// Parameters for listing files in a directory under root confinement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsListParams {
    pub root_path: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
}

impl FsListParams {
    pub fn new(root_path: impl Into<String>, path: Option<impl Into<String>>) -> Self {
        Self {
            root_path: root_path.into(),
            path: path.map(Into::into),
            environment_id: None,
        }
    }
}

/// Result of listing a directory under root confinement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsListResult {
    pub root_path: String,
    pub relative_path: String,
    pub entries: Vec<FsEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_rpc_serialization_round_trip() {
        let read_params = FsReadParams::new("/workspace", "src/main.rs");
        let json = serde_json::to_string(&read_params).unwrap();
        let decoded: FsReadParams = serde_json::from_str(&json).unwrap();
        assert_eq!(read_params, decoded);

        let read_res = FsReadResult {
            relative_path: "src/main.rs".into(),
            content: "fn main() {}".into(),
            is_binary: false,
            size_bytes: 12,
        };
        let json_res = serde_json::to_string(&read_res).unwrap();
        let decoded_res: FsReadResult = serde_json::from_str(&json_res).unwrap();
        assert_eq!(read_res, decoded_res);

        let list_params = FsListParams::new("/workspace", Some("src"));
        let json_list = serde_json::to_string(&list_params).unwrap();
        let decoded_list: FsListParams = serde_json::from_str(&json_list).unwrap();
        assert_eq!(list_params, decoded_list);
    }
}
