use crate::ids::{
    AgentId, EnvironmentId, ProviderInstanceId, ScheduleId, SubscriptionKey, ThreadId,
    UsageObservationId,
};
use crate::provider_config::ProviderKind;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;

/// Provider identity retained on normalized usage facts.
///
/// Known providers stay strongly typed while an unknown string can survive a newer
/// node crossing an older wire or store boundary.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UsageProvider {
    Claude,
    Codex,
    Antigravity,
    Cursor,
    Grok,
    OpenCode,
    Custom,
    Unknown(String),
}

impl UsageProvider {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Antigravity => "antigravity",
            Self::Cursor => "cursor",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
            Self::Custom => "custom",
            Self::Unknown(value) => value,
        }
    }

    pub fn known_kind(&self) -> Option<ProviderKind> {
        match self {
            Self::Claude => Some(ProviderKind::Claude),
            Self::Codex => Some(ProviderKind::Codex),
            Self::Antigravity => Some(ProviderKind::Antigravity),
            Self::Cursor => Some(ProviderKind::Cursor),
            Self::Grok => Some(ProviderKind::Grok),
            Self::OpenCode => Some(ProviderKind::OpenCode),
            Self::Custom => Some(ProviderKind::Custom),
            Self::Unknown(_) => None,
        }
    }
}

impl From<ProviderKind> for UsageProvider {
    fn from(value: ProviderKind) -> Self {
        match value {
            ProviderKind::Claude => Self::Claude,
            ProviderKind::Codex => Self::Codex,
            ProviderKind::Antigravity => Self::Antigravity,
            ProviderKind::Cursor => Self::Cursor,
            ProviderKind::Grok => Self::Grok,
            ProviderKind::OpenCode => Self::OpenCode,
            ProviderKind::Custom => Self::Custom,
        }
    }
}

impl From<String> for UsageProvider {
    fn from(value: String) -> Self {
        match value.as_str() {
            "claude" => Self::Claude,
            "codex" => Self::Codex,
            "antigravity" => Self::Antigravity,
            "cursor" => Self::Cursor,
            "grok" => Self::Grok,
            "opencode" => Self::OpenCode,
            "custom" => Self::Custom,
            _ => Self::Unknown(value),
        }
    }
}

impl Serialize for UsageProvider {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UsageProvider {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::from)
    }
}

/// Origin of a normalized usage observation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UsageSourceKind {
    ManagedTurn,
    ClaudeTranscript,
    CodexTranscript,
    ProviderReport,
    LegacyBucket,
    Unknown(String),
}

impl UsageSourceKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::ManagedTurn => "managed_turn",
            Self::ClaudeTranscript => "claude_transcript",
            Self::CodexTranscript => "codex_transcript",
            Self::ProviderReport => "provider_report",
            Self::LegacyBucket => "legacy_bucket",
            Self::Unknown(value) => value,
        }
    }
}

impl From<String> for UsageSourceKind {
    fn from(value: String) -> Self {
        match value.as_str() {
            "managed_turn" => Self::ManagedTurn,
            "claude_transcript" => Self::ClaudeTranscript,
            "codex_transcript" => Self::CodexTranscript,
            "provider_report" => Self::ProviderReport,
            "legacy_bucket" => Self::LegacyBucket,
            _ => Self::Unknown(value),
        }
    }
}

impl Serialize for UsageSourceKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UsageSourceKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::from)
    }
}

/// Fine-grained token counts with forward-compatible named categories.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounts {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub cached_input: u64,
    #[serde(default)]
    pub cache_write: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub reasoning: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub other: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageArithmeticError {
    pub category: String,
}

impl fmt::Display for UsageArithmeticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "usage count overflow in category {}", self.category)
    }
}

impl std::error::Error for UsageArithmeticError {}

impl TokenCounts {
    pub fn is_zero(&self) -> bool {
        self.input == 0
            && self.cached_input == 0
            && self.cache_write == 0
            && self.output == 0
            && self.reasoning == 0
            && self.other.values().all(|value| *value == 0)
    }

    pub fn checked_total(&self) -> Result<u64, UsageArithmeticError> {
        let mut total = 0_u64;
        for (category, value) in self.categories() {
            total = total
                .checked_add(value)
                .ok_or(UsageArithmeticError { category })?;
        }
        Ok(total)
    }

    pub fn checked_add(&self, other: &Self) -> Result<Self, UsageArithmeticError> {
        let add = |category: &str, left: u64, right: u64| {
            left.checked_add(right).ok_or_else(|| UsageArithmeticError {
                category: category.to_string(),
            })
        };

        let mut combined = Self {
            input: add("input", self.input, other.input)?,
            cached_input: add("cachedInput", self.cached_input, other.cached_input)?,
            cache_write: add("cacheWrite", self.cache_write, other.cache_write)?,
            output: add("output", self.output, other.output)?,
            reasoning: add("reasoning", self.reasoning, other.reasoning)?,
            other: self.other.clone(),
        };

        for (category, value) in &other.other {
            let current = combined.other.get(category).copied().unwrap_or(0);
            combined
                .other
                .insert(category.clone(), add(category, current, *value)?);
        }
        Ok(combined)
    }

    pub fn checked_accumulate(&mut self, other: &Self) -> Result<(), UsageArithmeticError> {
        *self = self.checked_add(other)?;
        Ok(())
    }

    fn categories(&self) -> impl Iterator<Item = (String, u64)> + '_ {
        [
            ("input".to_string(), self.input),
            ("cachedInput".to_string(), self.cached_input),
            ("cacheWrite".to_string(), self.cache_write),
            ("output".to_string(), self.output),
            ("reasoning".to_string(), self.reasoning),
        ]
        .into_iter()
        .chain(self.other.iter().map(|(key, value)| (key.clone(), *value)))
    }
}

/// Immutable ingestion fact. Environment identity is assigned by the authenticated
/// transport and is intentionally not caller-controlled here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageObservation {
    pub id: UsageObservationId,
    pub source_kind: UsageSourceKind,
    pub parser_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_native_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_native_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at_ms: Option<u64>,
    pub provider: UsageProvider,
    pub provider_instance_id: ProviderInstanceId,
    pub subscription_key: SubscriptionKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub tokens: TokenCounts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reported_micro_usd: Option<u64>,
    #[serde(default = "default_source_record_count")]
    pub source_record_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<ScheduleId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageObservationError {
    EmptyIdentifier(&'static str),
    EmptyUnknownValue(&'static str),
    EmptyModel,
    ZeroParserVersion,
    ZeroSourceRecordCount,
    EmptyFact,
    Arithmetic(UsageArithmeticError),
}

impl fmt::Display for UsageObservationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyIdentifier(field) => write!(f, "{field} must not be empty"),
            Self::EmptyUnknownValue(field) => write!(f, "unknown {field} must not be empty"),
            Self::EmptyModel => f.write_str("model must be absent or non-empty"),
            Self::ZeroParserVersion => f.write_str("parser version must be greater than zero"),
            Self::ZeroSourceRecordCount => {
                f.write_str("source record count must be greater than zero")
            }
            Self::EmptyFact => f.write_str("usage observation must contain tokens or spend"),
            Self::Arithmetic(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for UsageObservationError {}

impl UsageObservation {
    pub fn validate(&self) -> Result<(), UsageObservationError> {
        for (field, value) in [
            ("observation id", self.id.as_str()),
            ("provider instance id", self.provider_instance_id.as_str()),
            ("subscription key", self.subscription_key.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(UsageObservationError::EmptyIdentifier(field));
            }
        }
        if self.parser_version == 0 {
            return Err(UsageObservationError::ZeroParserVersion);
        }
        if self.source_record_count == 0 {
            return Err(UsageObservationError::ZeroSourceRecordCount);
        }
        if matches!(&self.provider, UsageProvider::Unknown(value) if value.trim().is_empty()) {
            return Err(UsageObservationError::EmptyUnknownValue("provider"));
        }
        if matches!(&self.source_kind, UsageSourceKind::Unknown(value) if value.trim().is_empty()) {
            return Err(UsageObservationError::EmptyUnknownValue("source kind"));
        }
        if self
            .model
            .as_ref()
            .is_some_and(|model| model.trim().is_empty())
        {
            return Err(UsageObservationError::EmptyModel);
        }
        let total = self
            .tokens
            .checked_total()
            .map_err(UsageObservationError::Arithmetic)?;
        if total == 0 && self.provider_reported_micro_usd.is_none() {
            return Err(UsageObservationError::EmptyFact);
        }
        Ok(())
    }
}

fn default_source_record_count() -> u64 {
    1
}

/// User-named quota boundary. The key is opaque and account labels stay masked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSubscription {
    pub key: SubscriptionKey,
    pub provider: UsageProvider,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masked_account_label: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacityConfidence {
    Reported,
    Estimated,
    Unknown,
}

/// Subscription quota window represented without floating-point fractions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacitySnapshot {
    pub subscription_key: SubscriptionKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_start_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_end_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_millionths: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_millionths: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at_ms: Option<u64>,
    pub source: String,
    pub confidence: CapacityConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_version: Option<String>,
    pub as_of_ms: u64,
}

impl CapacitySnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.subscription_key.as_str().trim().is_empty() {
            return Err("subscription key must not be empty".to_string());
        }
        if self.source.trim().is_empty() {
            return Err("capacity source must not be empty".to_string());
        }
        for (field, value) in [
            ("usedMillionths", self.used_millionths),
            ("remainingMillionths", self.remaining_millionths),
        ] {
            if value.is_some_and(|value| value > 1_000_000) {
                return Err(format!("{field} must be between 0 and 1000000"));
            }
        }
        if matches!(
            (self.window_start_ms, self.window_end_ms),
            (Some(start), Some(end)) if end < start
        ) {
            return Err("capacity window end must not precede its start".to_string());
        }
        Ok(())
    }
}

/// Query projection derived from observations. It is never accepted as ingestion input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub bucket_start_ms: u64,
    pub bucket_end_ms: u64,
    pub environment_id: EnvironmentId,
    pub provider: UsageProvider,
    pub provider_instance_id: ProviderInstanceId,
    pub subscription_key: SubscriptionKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<ScheduleId>,
    pub tokens: TokenCounts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reported_micro_usd: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_equivalent_estimate_micro_usd: Option<u64>,
    pub records: u64,
}

/// Daily query record retained for client projection compatibility.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub date: String,
    #[serde(default)]
    pub tokens: TokenCounts,
    #[serde(default)]
    pub provider_reported_micro_usd: Option<u64>,
    #[serde(default)]
    pub api_equivalent_estimate_micro_usd: Option<u64>,
    #[serde(default)]
    pub turn_count: u64,
}

/// Integer budget ledger for one day.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyBudgetLedger {
    pub date: String,
    #[serde(default)]
    pub current_micro_usd: u64,
    #[serde(default)]
    pub max_micro_usd: Option<u64>,
    #[serde(default)]
    pub exceeded: bool,
}

/// Aggregate token and spend projection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageSummary {
    #[serde(default)]
    pub tokens: TokenCounts,
    #[serde(default)]
    pub provider_reported_micro_usd: Option<u64>,
    #[serde(default)]
    pub api_equivalent_estimate_micro_usd: Option<u64>,
    #[serde(default)]
    pub records: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> UsageObservation {
        UsageObservation {
            id: UsageObservationId::new("usage-fixture"),
            source_kind: UsageSourceKind::ManagedTurn,
            parser_version: 1,
            provider_native_session_id: Some("session-1".to_string()),
            provider_native_turn_id: Some("provider-turn-1".to_string()),
            occurred_at_ms: Some(1_700_000_000_000),
            provider: UsageProvider::Codex,
            provider_instance_id: ProviderInstanceId::new("codex-default"),
            subscription_key: SubscriptionKey::new("subkey-work"),
            model: Some("gpt-5".to_string()),
            tokens: TokenCounts {
                input: 10,
                cached_input: 2,
                cache_write: 1,
                output: 4,
                reasoning: 3,
                other: BTreeMap::from([("futureCategory".to_string(), 5)]),
            },
            provider_reported_micro_usd: Some(42),
            source_record_count: 1,
            thread_id: Some(ThreadId::new("thread-1")),
            agent_id: None,
            schedule_id: None,
        }
    }

    #[test]
    fn token_counts_add_every_category_with_overflow_detection() {
        let left = observation().tokens;
        let right = TokenCounts {
            input: 1,
            cached_input: 2,
            cache_write: 3,
            output: 4,
            reasoning: 5,
            other: BTreeMap::from([
                ("futureCategory".to_string(), 6),
                ("anotherCategory".to_string(), 7),
            ]),
        };
        let combined = left.checked_add(&right).expect("checked addition");
        assert_eq!(combined.input, 11);
        assert_eq!(combined.cached_input, 4);
        assert_eq!(combined.cache_write, 4);
        assert_eq!(combined.output, 8);
        assert_eq!(combined.reasoning, 8);
        assert_eq!(combined.other["futureCategory"], 11);
        assert_eq!(combined.other["anotherCategory"], 7);
        assert_eq!(combined.checked_total().expect("checked total"), 53);

        let overflow = TokenCounts {
            input: u64::MAX,
            ..TokenCounts::default()
        }
        .checked_add(&TokenCounts {
            input: 1,
            ..TokenCounts::default()
        })
        .expect_err("overflow must fail");
        assert_eq!(overflow.category, "input");
    }

    #[test]
    fn unknown_provider_and_source_round_trip_without_loss() {
        let mut value = observation();
        value.provider = UsageProvider::Unknown("future-provider".to_string());
        value.source_kind = UsageSourceKind::Unknown("future-source".to_string());

        let json = serde_json::to_string(&value).expect("serialize observation");
        let parsed: UsageObservation =
            serde_json::from_str(&json).expect("deserialize observation");
        assert_eq!(parsed, value);
        assert!(json.contains("\"provider\":\"future-provider\""));
        assert!(json.contains("\"sourceKind\":\"future-source\""));
    }

    #[test]
    fn observation_serialization_is_stable_and_validated() {
        let value = observation();
        value.validate().expect("valid observation");
        let json = serde_json::to_string(&value).expect("serialize observation");
        assert_eq!(
            json,
            r#"{"id":"usage-fixture","sourceKind":"managed_turn","parserVersion":1,"providerNativeSessionId":"session-1","providerNativeTurnId":"provider-turn-1","occurredAtMs":1700000000000,"provider":"codex","providerInstanceId":"codex-default","subscriptionKey":"subkey-work","model":"gpt-5","tokens":{"input":10,"cachedInput":2,"cacheWrite":1,"output":4,"reasoning":3,"other":{"futureCategory":5}},"providerReportedMicroUsd":42,"sourceRecordCount":1,"threadId":"thread-1"}"#
        );

        let mut empty = value;
        empty.tokens = TokenCounts::default();
        empty.provider_reported_micro_usd = None;
        assert_eq!(empty.validate(), Err(UsageObservationError::EmptyFact));
    }
}
