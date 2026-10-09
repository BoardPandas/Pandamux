use crate::ids::{
    BundleActivationId, BundleTransferId, ConfigurationAuditId, EnvironmentId,
    InstructionCandidateId, InstructionSourceId, OrganizationId, ProjectId, ProjectLayerTrustId,
    ProjectRepositoryId,
};
use crate::instruction_bundle::BundleSourceManifest;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt;
use std::num::NonZeroU64;
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidConfigurationIdentity {
    kind: &'static str,
    reason: &'static str,
}

impl InvalidConfigurationIdentity {
    fn new(kind: &'static str, reason: &'static str) -> Self {
        Self { kind, reason }
    }
}

impl fmt::Display for InvalidConfigurationIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.kind, self.reason)
    }
}

impl std::error::Error for InvalidConfigurationIdentity {}

macro_rules! lowercase_hex_identity {
    ($name:ident, $kind:literal, $valid_lengths:expr) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, InvalidConfigurationIdentity> {
                let value = value.into();
                if !$valid_lengths.contains(&value.len()) {
                    return Err(InvalidConfigurationIdentity::new(
                        $kind,
                        "has an unsupported length",
                    ));
                }
                if !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(InvalidConfigurationIdentity::new(
                        $kind,
                        "must contain lowercase hexadecimal characters only",
                    ));
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = InvalidConfigurationIdentity;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(de::Error::custom)
            }
        }
    };
}

lowercase_hex_identity!(SourceCommitSha, "source commit SHA", [40, 64]);
lowercase_hex_identity!(ContentSha256, "content SHA-256", [64]);
lowercase_hex_identity!(BundleSha, "bundle SHA-256", [64]);
lowercase_hex_identity!(ArchiveSha256, "archive SHA-256", [64]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigurationRevision(NonZeroU64);

impl ConfigurationRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub fn new(value: u64) -> Result<Self, InvalidConfigurationIdentity> {
        NonZeroU64::new(value).map(Self).ok_or_else(|| {
            InvalidConfigurationIdentity::new("configuration revision", "must be nonzero")
        })
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub fn checked_next(self) -> Result<Self, InvalidConfigurationIdentity> {
        self.get()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .map(Self)
            .ok_or_else(|| {
                InvalidConfigurationIdentity::new("configuration revision", "cannot be incremented")
            })
    }
}

impl fmt::Display for ConfigurationRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

impl TryFrom<u64> for ConfigurationRevision {
    type Error = InvalidConfigurationIdentity;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Serialize for ConfigurationRevision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.get())
    }
}

impl<'de> Deserialize<'de> for ConfigurationRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TrackingRef {
    Branch(String),
    Tag(String),
    Commit(SourceCommitSha),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionsUpdatePolicy {
    ApproveEach,
    TrustBranch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionCandidateState {
    Discovered,
    Inspecting,
    Blocked,
    AwaitingApproval,
    Approved,
    Activated,
    Superseded,
}

impl InstructionCandidateState {
    pub const fn is_unresolved(self) -> bool {
        matches!(
            self,
            Self::Discovered | Self::Inspecting | Self::AwaitingApproval
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationUpdateSafety {
    Ordinary,
    AccessWidening,
    ScheduledAgentChange,
    HistoryRewrite,
    RemoteIdentityChange,
    TrackingChange,
}

impl ConfigurationUpdateSafety {
    pub const fn requires_local_approval(self) -> bool {
        !matches!(self, Self::Ordinary)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleActivationKind {
    CandidateApproval,
    TrustedBranch,
    Revert,
    Recovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleDistributionState {
    Pending,
    Uploading,
    Active,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionsSourceRecord {
    pub id: InstructionSourceId,
    pub canonical_remote_identity: String,
    pub tracking: TrackingRef,
    pub policy: InstructionsUpdatePolicy,
    pub require_signed_commit: bool,
    pub revision: ConfigurationRevision,
    pub last_checked_at_ms: Option<u64>,
    pub last_error_reason: Option<String>,
    pub active_source_commit_sha: Option<SourceCommitSha>,
    pub active_bundle_sha: Option<BundleSha>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionCandidateRecord {
    pub id: InstructionCandidateId,
    pub source_id: InstructionSourceId,
    pub observed_source_commit_sha: SourceCommitSha,
    pub observed_tag_object_sha: Option<SourceCommitSha>,
    pub state: InstructionCandidateState,
    pub safety: ConfigurationUpdateSafety,
    pub diff_digest: ContentSha256,
    pub activatable: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectRepositoryRecord {
    pub id: ProjectRepositoryId,
    pub project_id: ProjectId,
    pub organization_id: OrganizationId,
    pub canonical_remote_identity: String,
    pub canonical_repository_root_identity: String,
    pub display_path: String,
    pub revision: ConfigurationRevision,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectLayerTrustRecord {
    pub id: ProjectLayerTrustId,
    pub organization_id: OrganizationId,
    pub project_id: ProjectId,
    pub repository_id: ProjectRepositoryId,
    pub project_content_sha256: ContentSha256,
    pub approved_by: String,
    pub approved_at_ms: u64,
    pub revoked_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationAuditRecord {
    pub id: ConfigurationAuditId,
    pub kind: String,
    pub actor: String,
    pub source_id: Option<InstructionSourceId>,
    pub candidate_id: Option<InstructionCandidateId>,
    pub bundle_sha: Option<BundleSha>,
    pub details: serde_json::Value,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleActivationRecord {
    pub id: BundleActivationId,
    pub bundle_sha: BundleSha,
    pub source_manifest: BundleSourceManifest,
    pub activated_at_ms: u64,
    pub kind: BundleActivationKind,
    pub actor: String,
    pub prior_bundle_sha: Option<BundleSha>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleDistributionReceipt {
    pub transfer_id: BundleTransferId,
    pub environment_id: EnvironmentId,
    pub bundle_sha: BundleSha,
    pub archive_sha256: ArchiveSha256,
    pub stored_bundle_sha: BundleSha,
    pub active_bundle_sha: BundleSha,
    pub prior_bundle_sha: Option<BundleSha>,
    pub state: BundleDistributionState,
    pub received_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_and_revision_wrappers_reject_malformed_values() {
        assert!(SourceCommitSha::parse("a".repeat(40)).is_ok());
        assert!(SourceCommitSha::parse("b".repeat(64)).is_ok());
        assert!(SourceCommitSha::parse("A".repeat(40)).is_err());
        assert!(ContentSha256::parse("0".repeat(63)).is_err());
        assert!(BundleSha::parse("g".repeat(64)).is_err());
        assert!(ArchiveSha256::parse("f".repeat(64)).is_ok());
        assert!(ConfigurationRevision::new(0).is_err());
        assert_eq!(ConfigurationRevision::INITIAL.get(), 1);
    }

    #[test]
    fn validated_wrappers_reject_bad_deserialized_values() {
        assert!(serde_json::from_str::<ContentSha256>(&format!("\"{}\"", "A".repeat(64))).is_err());
        assert!(serde_json::from_str::<ConfigurationRevision>("0").is_err());
    }
}
