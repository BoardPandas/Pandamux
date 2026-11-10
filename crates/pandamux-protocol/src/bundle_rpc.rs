use pandamux_core::{
    ArchiveSha256, BundleDistributionState, BundleSha, BundleSourceManifest, BundleTransferId,
    EnvironmentId,
};
use serde::{Deserialize, Serialize};

pub const BUNDLE_PUSH_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundlePrepareUploadParams {
    pub schema_version: u32,
    pub transfer_id: BundleTransferId,
    pub environment_id: EnvironmentId,
    pub archive_sha256: ArchiveSha256,
    pub archive_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundlePrepareUploadResult {
    pub transfer_id: BundleTransferId,
    pub private_artifact_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleDiscardUploadParams {
    pub transfer_id: BundleTransferId,
    pub environment_id: EnvironmentId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundlePushParams {
    pub schema_version: u32,
    pub transfer_id: BundleTransferId,
    pub bundle_sha: BundleSha,
    pub archive_sha256: ArchiveSha256,
    pub archive_bytes: u64,
    pub source_manifest: BundleSourceManifest,
    pub compiler_version: String,
    pub private_artifact_ref: String,
    pub expected_prior_bundle_sha: Option<BundleSha>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundlePushResult {
    pub transfer_id: BundleTransferId,
    pub stored_bundle_sha: BundleSha,
    pub active_bundle_sha: BundleSha,
    pub prior_bundle_sha: Option<BundleSha>,
    pub received_at_ms: u64,
    pub status: BundleDistributionState,
}
