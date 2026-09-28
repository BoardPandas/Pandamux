use pandamux_core::{Environment, SshHostProfile};
use serde::{Deserialize, Serialize};

/// Parameters for `environment.import_ssh_config` or `environment.importSshConfig`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentImportSshConfigParams {
    /// Optional SSH configuration text. If omitted or empty, the server reads
    /// from the host default ~/.ssh/config file.
    #[serde(default)]
    pub custom_config: Option<String>,
    /// If true, parse and preview discovered environments without saving them to settings.
    #[serde(default)]
    pub dry_run: bool,
}

/// Result of `environment.import_ssh_config` or `environment.importSshConfig`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentImportSshConfigResult {
    /// Number of newly imported environments.
    pub imported_count: usize,
    /// Number of hosts in the SSH config skipped because an environment with that name or id already existed.
    pub skipped_existing_count: usize,
    /// List of newly imported environments.
    pub environments: Vec<Environment>,
    /// List of parsed SSH host profiles.
    pub profiles: Vec<SshHostProfile>,
}

/// Parameters for `environment.list`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentListParams {}

/// Result of `environment.list`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentListResult {
    pub environments: Vec<Environment>,
}
