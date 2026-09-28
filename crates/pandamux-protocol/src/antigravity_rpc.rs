use pandamux_core::EnvironmentId;
use serde::{Deserialize, Serialize};

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
    pub entries: Vec<String>,
}

/// Parameters for `antigravity.install`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityInstallParams {
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    pub manifest: ManagedBundleManifest,
    #[serde(default)]
    pub fallback_bytes_base64: Option<String>,
    #[serde(default)]
    pub force: bool,
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
}

/// Result returned from `antigravity.status`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityStatusResult {
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
}

/// Result returned from `antigravity.oauth_start`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityOAuthStartResult {
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
    pub callback_url: String,
    pub state: String,
    pub redirect_port: u16,
}

/// Result returned from `antigravity.oauth_relay`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityOAuthRelayResult {
    pub success: bool,
    pub message: String,
}
