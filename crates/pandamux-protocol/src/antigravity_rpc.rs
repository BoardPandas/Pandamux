use pandamux_core::{EnvironmentId, ProviderInstanceId};
use serde::{Deserialize, Serialize};

/// Pinned bundle manifest describing an Antigravity ACP release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedBundleFile {
    pub name: String,
    pub size: u64,
}

/// Pinned bundle manifest describing an Antigravity ACP release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedBundleManifest {
    pub version: String,
    pub platform: String,
    pub arch: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub executable: ManagedBundleFile,
    pub harness: ManagedBundleFile,
}

/// Parameters for `antigravity.install`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityInstallParams {
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    pub manifest: ManagedBundleManifest,
    #[serde(default)]
    pub staged_archive_path: Option<String>,
    #[serde(default)]
    pub force: bool,
}

/// Parameters for preparing a private node-side SFTP upload target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityPrepareUploadParams {
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    pub manifest: ManagedBundleManifest,
}

/// Private node-side path to which the Hub may upload a verified archive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityPrepareUploadResult {
    pub remote_path: String,
}

/// Parameters for deleting a failed private node-side SFTP upload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityDiscardUploadParams {
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    pub staged_archive_path: String,
}

/// Result returned from `antigravity.install`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityInstallResult {
    pub installed: bool,
    pub version: String,
    pub binary_path: String,
    pub source: String,
    pub free_disk_bytes: u64,
}

/// Parameters for `antigravity.status`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityStatusParams {
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    #[serde(default)]
    pub provider_instance_id: Option<ProviderInstanceId>,
}

/// Result returned from `antigravity.status`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityStatusResult {
    pub provider_instance_id: ProviderInstanceId,
    pub installed: bool,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub binary_path: Option<String>,
    pub authenticated: bool,
    pub active_processes: usize,
    pub max_processes: usize,
    pub free_disk_bytes: u64,
}

/// Parameters for `antigravity.oauth_start`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityOAuthStartParams {
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    #[serde(default)]
    pub provider_instance_id: Option<ProviderInstanceId>,
}

/// Result returned from `antigravity.oauth_start`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityOAuthStartResult {
    pub provider_instance_id: ProviderInstanceId,
    pub flow_id: String,
    pub auth_url: String,
    pub redirect_port: u16,
    pub state: String,
}

/// Parameters for `antigravity.oauth_relay`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityOAuthRelayParams {
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    #[serde(default)]
    pub provider_instance_id: Option<ProviderInstanceId>,
    pub flow_id: String,
    pub callback_url: String,
}

/// Result returned from `antigravity.oauth_relay`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityOAuthRelayResult {
    pub provider_instance_id: ProviderInstanceId,
    pub success: bool,
    pub message: String,
}
