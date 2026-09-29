use pandamux_core::{
    AgentId, CapacitySnapshot, EnvironmentId, NodeStoreId, PricingVersionId, ProviderInstanceId,
    ScheduleId, SubscriptionKey, TokenCounts, UsageObservation, UsageObservationId, UsageProvider,
    UsageSourceKind,
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_USAGE_PAGE_SIZE: u32 = 100;
pub const MAX_USAGE_PAGE_SIZE: u32 = 500;

fn default_page_size() -> u32 {
    DEFAULT_USAGE_PAGE_SIZE
}

fn validate_limit(limit: u32) -> Result<(), String> {
    if (1..=MAX_USAGE_PAGE_SIZE).contains(&limit) {
        Ok(())
    } else {
        Err(format!("limit must be between 1 and {MAX_USAGE_PAGE_SIZE}"))
    }
}

fn validate_persisted_u64(field: &str, value: u64) -> Result<(), String> {
    if i64::try_from(value).is_ok() {
        Ok(())
    } else {
        Err(format!("{field} exceeds the supported integer range"))
    }
}

fn validate_optional_id(field: &str, value: Option<&str>) -> Result<(), String> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        Err(format!("{field} must be absent or non-empty"))
    } else {
        Ok(())
    }
}

/// Hub-only request for one bounded node outbox page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageExportParams {
    #[serde(default)]
    pub after_seq: u64,
    #[serde(default = "default_page_size")]
    pub limit: u32,
}

impl Default for UsageExportParams {
    fn default() -> Self {
        Self {
            after_seq: 0,
            limit: DEFAULT_USAGE_PAGE_SIZE,
        }
    }
}

impl UsageExportParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_limit(self.limit)?;
        validate_persisted_u64("afterSeq", self.after_seq)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageExportItem {
    pub seq: u64,
    pub observation: UsageObservation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageExportResult {
    pub node_store_id: NodeStoreId,
    pub items: Vec<UsageExportItem>,
    pub next_after_seq: u64,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retained_from_seq: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub latest_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_gap: Option<UsageCoverageGap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_coverage: Option<UsageNodeCoverageSnapshot>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// Explicit notice that an incremental sequence is no longer retained on the node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCoverageGap {
    pub requested_after_seq: u64,
    pub retained_from_seq: u64,
    pub latest_seq: u64,
}

/// Stable keyset used while reconciling a bounded full-fact snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBackfillCursor {
    pub observation_id: UsageObservationId,
}

/// Hub-only request for one bounded snapshot page after a retention gap or store reset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBackfillParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_through_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<UsageBackfillCursor>,
    #[serde(default = "default_page_size")]
    pub limit: u32,
}

impl Default for UsageBackfillParams {
    fn default() -> Self {
        Self {
            snapshot_through_seq: None,
            after: None,
            limit: DEFAULT_USAGE_PAGE_SIZE,
        }
    }
}

impl UsageBackfillParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_limit(self.limit)?;
        if let Some(value) = self.snapshot_through_seq {
            validate_persisted_u64("snapshotThroughSeq", value)?;
        }
        if let Some(after) = &self.after {
            if self.snapshot_through_seq.is_none() {
                return Err("after requires snapshotThroughSeq".to_string());
            }
            validate_optional_id("after.observationId", Some(after.observation_id.as_str()))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBackfillItem {
    pub latest_seq: u64,
    pub observation: UsageObservation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBackfillResult {
    pub node_store_id: NodeStoreId,
    pub snapshot_through_seq: u64,
    pub items: Vec<UsageBackfillItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<UsageBackfillCursor>,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_coverage: Option<UsageNodeCoverageSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAckParams {
    pub through_seq: u64,
}

impl UsageAckParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_persisted_u64("throughSeq", self.through_seq)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAckResult {
    pub acknowledged: u64,
    pub through_seq: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub compacted: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retained_from_seq: u64,
}

/// Stable keyset cursor for observation queries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQueryCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at_ms: Option<u64>,
    pub environment_id: EnvironmentId,
    pub observation_id: UsageObservationId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQueryParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<EnvironmentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<UsageProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_instance_id: Option<ProviderInstanceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_key: Option<SubscriptionKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<ScheduleId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<UsageSourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<UsageQueryCursor>,
    #[serde(default = "default_page_size")]
    pub limit: u32,
}

impl Default for UsageQueryParams {
    fn default() -> Self {
        Self {
            from_ms: None,
            to_ms: None,
            environment_id: None,
            provider: None,
            provider_instance_id: None,
            subscription_key: None,
            agent_id: None,
            schedule_id: None,
            model: None,
            source_kind: None,
            after: None,
            limit: DEFAULT_USAGE_PAGE_SIZE,
        }
    }
}

impl UsageQueryParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_limit(self.limit)?;
        if let Some(value) = self.from_ms {
            validate_persisted_u64("fromMs", value)?;
        }
        if let Some(value) = self.to_ms {
            validate_persisted_u64("toMs", value)?;
        }
        if matches!((self.from_ms, self.to_ms), (Some(from), Some(to)) if to < from) {
            return Err("toMs must not precede fromMs".to_string());
        }
        validate_optional_id(
            "environmentId",
            self.environment_id.as_ref().map(EnvironmentId::as_str),
        )?;
        validate_optional_id(
            "providerInstanceId",
            self.provider_instance_id
                .as_ref()
                .map(ProviderInstanceId::as_str),
        )?;
        validate_optional_id(
            "subscriptionKey",
            self.subscription_key.as_ref().map(SubscriptionKey::as_str),
        )?;
        validate_optional_id(
            "agentId",
            self.agent_id.as_ref().map(|value| value.as_str()),
        )?;
        validate_optional_id(
            "scheduleId",
            self.schedule_id.as_ref().map(ScheduleId::as_str),
        )?;
        if matches!(&self.provider, Some(UsageProvider::Unknown(value)) if value.trim().is_empty())
        {
            return Err("unknown provider must not be empty".to_string());
        }
        if matches!(&self.source_kind, Some(UsageSourceKind::Unknown(value)) if value.trim().is_empty())
        {
            return Err("unknown sourceKind must not be empty".to_string());
        }
        if self
            .model
            .as_ref()
            .is_some_and(|model| model.trim().is_empty())
        {
            return Err("model must be absent or non-empty".to_string());
        }
        if let Some(cursor) = &self.after {
            if let Some(value) = cursor.occurred_at_ms {
                validate_persisted_u64("after.occurredAtMs", value)?;
            }
            validate_optional_id("after.environmentId", Some(cursor.environment_id.as_str()))?;
            validate_optional_id("after.observationId", Some(cursor.observation_id.as_str()))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQueryItem {
    pub environment_id: EnvironmentId,
    pub observation: UsageObservation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCoverage {
    pub quarantined_count: u64,
    #[serde(default)]
    pub included_environments: Vec<EnvironmentId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observation_sources: Vec<UsageSourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_source_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unknown_price_tokens: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub priceable_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_price_token_millionths: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment_sync: Vec<UsageEnvironmentSyncCoverage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scanner_diagnostics: Vec<UsageScannerDiagnostic>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageEnvironmentSyncState {
    Ready,
    Reconciling,
    CoverageGap,
    Stale,
    Unreachable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEnvironmentSyncCoverage {
    pub environment_id: EnvironmentId,
    pub state: UsageEnvironmentSyncState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_safe_error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_gap: Option<UsageCoverageGap>,
}

/// SQL-aggregated totals for the complete filtered result set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAggregate {
    pub observation_count: u64,
    pub tokens: TokenCounts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reported_micro_usd: Option<u64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub provider_reported_observation_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_equivalent_estimated_micro_usd: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pricing_version_ids: Vec<PricingVersionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSourceCoverageState {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageScannerDiagnostic {
    pub environment_id: EnvironmentId,
    pub provider_instance_id: ProviderInstanceId,
    pub source_kind: UsageSourceKind,
    pub coverage_state: UsageSourceCoverageState,
    pub parser_version: u32,
    pub last_scan_started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_source_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_lag_ms: Option<u64>,
    pub files_scanned: u64,
    pub bytes_scanned: u64,
    pub quarantined_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_safe_error_code: Option<String>,
}

/// Path-free node coverage carried alongside internal synchronization pages.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageNodeCoverageSnapshot {
    pub quarantined_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_source_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scanner_diagnostics: Vec<UsageScannerDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQueryResult {
    pub items: Vec<UsageQueryItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<UsageQueryCursor>,
    pub has_more: bool,
    #[serde(default)]
    pub aggregate: UsageAggregate,
    pub coverage: UsageCoverage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityQueryParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_key: Option<SubscriptionKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<CapacityQueryCursor>,
    #[serde(default = "default_page_size")]
    pub limit: u32,
}

impl Default for CapacityQueryParams {
    fn default() -> Self {
        Self {
            subscription_key: None,
            after: None,
            limit: DEFAULT_USAGE_PAGE_SIZE,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityQueryCursor {
    pub as_of_ms: u64,
    pub subscription_key: SubscriptionKey,
    pub sequence: u64,
}

impl CapacityQueryParams {
    pub fn validate(&self) -> Result<(), String> {
        validate_limit(self.limit)?;
        validate_optional_id(
            "subscriptionKey",
            self.subscription_key.as_ref().map(SubscriptionKey::as_str),
        )?;
        if let Some(cursor) = &self.after {
            validate_persisted_u64("after.asOfMs", cursor.as_of_ms)?;
            validate_persisted_u64("after.sequence", cursor.sequence)?;
            validate_optional_id(
                "after.subscriptionKey",
                Some(cursor.subscription_key.as_str()),
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityQueryResult {
    pub snapshots: Vec<CapacitySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<CapacityQueryCursor>,
    pub has_more: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{TokenCounts, UsageObservationId};

    fn observation() -> UsageObservation {
        UsageObservation {
            id: UsageObservationId::new("usage-one"),
            source_kind: UsageSourceKind::ManagedTurn,
            parser_version: 1,
            provider_native_session_id: None,
            provider_native_turn_id: Some("turn-native".to_string()),
            occurred_at_ms: Some(1000),
            provider: UsageProvider::Claude,
            provider_instance_id: ProviderInstanceId::new("claude-work"),
            subscription_key: SubscriptionKey::new("subkey-work"),
            model: Some("claude-model".to_string()),
            tokens: TokenCounts {
                input: 3,
                output: 2,
                ..TokenCounts::default()
            },
            unknown_fields: std::collections::BTreeSet::new(),
            provider_reported_micro_usd: None,
            source_record_count: 1,
            thread_id: None,
            turn_id: None,
            agent_id: None,
            schedule_id: None,
        }
    }

    #[test]
    fn export_fixture_is_version_stable_and_contains_no_provenance_paths() {
        let result = UsageExportResult {
            node_store_id: NodeStoreId::new("store-fixture"),
            items: vec![UsageExportItem {
                seq: 7,
                observation: observation(),
            }],
            next_after_seq: 7,
            has_more: false,
            retained_from_seq: 0,
            latest_seq: 0,
            coverage_gap: None,
            node_coverage: None,
        };
        let json = serde_json::to_string(&result).expect("serialize export");
        assert_eq!(
            json,
            r#"{"nodeStoreId":"store-fixture","items":[{"seq":7,"observation":{"id":"usage-one","sourceKind":"managed_turn","parserVersion":1,"providerNativeTurnId":"turn-native","occurredAtMs":1000,"provider":"claude","providerInstanceId":"claude-work","subscriptionKey":"subkey-work","model":"claude-model","tokens":{"input":3,"cachedInput":0,"cacheWrite":0,"output":2,"reasoning":0},"sourceRecordCount":1}}],"nextAfterSeq":7,"hasMore":false}"#
        );
        assert!(!json.contains("path"));
        assert!(!json.contains("transcript"));
        assert!(!json.contains("profile"));
    }

    #[test]
    fn every_usage_page_is_bounded() {
        assert!(UsageExportParams::default().validate().is_ok());
        assert!(UsageBackfillParams::default().validate().is_ok());
        assert!(UsageQueryParams::default().validate().is_ok());
        assert!(CapacityQueryParams::default().validate().is_ok());
        assert!(
            UsageExportParams {
                after_seq: 0,
                limit: 0
            }
            .validate()
            .is_err()
        );
        assert!(
            UsageBackfillParams {
                snapshot_through_seq: None,
                after: Some(UsageBackfillCursor {
                    observation_id: UsageObservationId::new("usage-cursor"),
                }),
                limit: 10,
            }
            .validate()
            .is_err()
        );
        assert!(
            UsageQueryParams {
                limit: MAX_USAGE_PAGE_SIZE + 1,
                ..UsageQueryParams::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            UsageAckParams {
                through_seq: u64::MAX
            }
            .validate()
            .is_err()
        );
        assert!(
            CapacityQueryParams {
                subscription_key: Some(SubscriptionKey::new("")),
                ..CapacityQueryParams::default()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn query_filters_round_trip_explicitly() {
        let params = UsageQueryParams {
            from_ms: Some(100),
            to_ms: Some(200),
            environment_id: Some(EnvironmentId::new("env-one")),
            provider: Some(UsageProvider::Unknown("future-provider".to_string())),
            provider_instance_id: Some(ProviderInstanceId::new("future-one")),
            subscription_key: Some(SubscriptionKey::new("subkey-one")),
            agent_id: Some(AgentId::new("agent-one")),
            schedule_id: Some(ScheduleId::new("schedule-one")),
            model: Some("future-model".to_string()),
            source_kind: Some(UsageSourceKind::Unknown("future-source".to_string())),
            after: None,
            limit: 50,
        };
        params.validate().expect("valid query");
        let json = serde_json::to_string(&params).expect("serialize query");
        let parsed: UsageQueryParams = serde_json::from_str(&json).expect("parse query");
        assert_eq!(parsed, params);
    }
}
