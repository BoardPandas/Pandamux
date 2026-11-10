use crate::ids::{
    AgentId, EnvironmentId, PricingVersionId, ProviderInstanceId, ScheduleId, SubscriptionKey,
    ThreadId, UsageObservationId,
};
use crate::provider_config::ProviderKind;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
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

/// Semantic role of one normalized usage fact.
///
/// Aggregate is the backward-compatible default for observations written before
/// component-aware accounting was introduced.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UsageFactKind {
    #[default]
    Aggregate,
    TokenComponent,
    TurnCost,
    Unknown(String),
}

impl UsageFactKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Aggregate => "aggregate",
            Self::TokenComponent => "token_component",
            Self::TurnCost => "turn_cost",
            Self::Unknown(value) => value,
        }
    }

    pub fn is_aggregate(&self) -> bool {
        self == &Self::Aggregate
    }
}

impl From<String> for UsageFactKind {
    fn from(value: String) -> Self {
        match value.as_str() {
            "aggregate" => Self::Aggregate,
            "token_component" => Self::TokenComponent,
            "turn_cost" => Self::TurnCost,
            _ => Self::Unknown(value),
        }
    }
}

impl Serialize for UsageFactKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UsageFactKind {
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

/// Provider fields that were absent from an otherwise usable usage record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageUnknownField {
    OccurredAt,
    Model,
    Identity,
    InputTokens,
    OutputTokens,
    CacheReadTokens,
    CacheWriteTokens,
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

/// Per-million-token rates expressed in integer micro-USD.
///
/// An absent rate is deliberately unknown. A zero rate is a known free category.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePricingRates {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_micro_usd_per_million: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_micro_usd_per_million: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_micro_usd_per_million: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_micro_usd_per_million: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_micro_usd_per_million: Option<u64>,
}

/// Exact canonical model match and its explicitly declared aliases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePricingRule {
    pub canonical_model_id: String,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub aliases: BTreeSet<String>,
    pub rates: UsagePricingRates,
}

/// Source-dated, effective-dated provider pricing revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePricingVersion {
    pub id: PricingVersionId,
    pub provider: UsageProvider,
    pub source_published_at_ms: u64,
    pub effective_from_ms: u64,
    pub rules: Vec<UsagePricingRule>,
}

impl UsagePricingVersion {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.as_str().trim().is_empty() {
            return Err("pricing version id must not be empty".to_string());
        }
        if matches!(&self.provider, UsageProvider::Unknown(value) if value.trim().is_empty()) {
            return Err("unknown pricing provider must not be empty".to_string());
        }
        if self.rules.is_empty() {
            return Err("pricing version must contain at least one rule".to_string());
        }

        let mut model_ids = BTreeSet::new();
        for rule in &self.rules {
            let canonical = rule.canonical_model_id.trim();
            if canonical.is_empty() {
                return Err("canonical pricing model id must not be empty".to_string());
            }
            if !model_ids.insert(canonical.to_string()) {
                return Err(format!("duplicate pricing model id {canonical}"));
            }
            if !pricing_rates_have_known_category(&rule.rates) {
                return Err(format!(
                    "pricing rule {canonical} must contain at least one known rate"
                ));
            }
            for alias in &rule.aliases {
                let alias = alias.trim();
                if alias.is_empty() {
                    return Err("pricing alias must not be empty".to_string());
                }
                if !model_ids.insert(alias.to_string()) {
                    return Err(format!("duplicate pricing model id or alias {alias}"));
                }
            }
        }
        Ok(())
    }
}

fn pricing_rates_have_known_category(rates: &UsagePricingRates) -> bool {
    rates.input_micro_usd_per_million.is_some()
        || rates.cached_input_micro_usd_per_million.is_some()
        || rates.cache_write_micro_usd_per_million.is_some()
        || rates.output_micro_usd_per_million.is_some()
        || rates.reasoning_micro_usd_per_million.is_some()
}

/// Prices one already-aggregated rate group with checked integer arithmetic.
///
/// Products are summed at full precision, then rounded once to the nearest micro-USD.
/// Exact half-micro-USD values round upward. A nonzero category without an explicit rate,
/// including any provider-specific `other` category, makes the estimate unknown.
pub fn estimate_usage_micro_usd(
    tokens: &TokenCounts,
    rates: &UsagePricingRates,
) -> Result<Option<u64>, UsageArithmeticError> {
    if tokens.other.values().any(|value| *value != 0) {
        return Ok(None);
    }
    let categories = [
        (
            "inputPrice",
            tokens.input,
            rates.input_micro_usd_per_million,
        ),
        (
            "cachedInputPrice",
            tokens.cached_input,
            rates.cached_input_micro_usd_per_million,
        ),
        (
            "cacheWritePrice",
            tokens.cache_write,
            rates.cache_write_micro_usd_per_million,
        ),
        (
            "outputPrice",
            tokens.output,
            rates.output_micro_usd_per_million,
        ),
        (
            "reasoningPrice",
            tokens.reasoning,
            rates.reasoning_micro_usd_per_million,
        ),
    ];
    let mut numerator = 0_u128;
    for (category, count, rate) in categories {
        if count == 0 {
            continue;
        }
        let Some(rate) = rate else {
            return Ok(None);
        };
        let product = u128::from(count)
            .checked_mul(u128::from(rate))
            .ok_or_else(|| UsageArithmeticError {
                category: category.to_string(),
            })?;
        numerator = numerator
            .checked_add(product)
            .ok_or_else(|| UsageArithmeticError {
                category: category.to_string(),
            })?;
    }

    const TOKENS_PER_MILLION: u128 = 1_000_000;
    let rounded = numerator
        .checked_add(TOKENS_PER_MILLION / 2)
        .ok_or_else(|| UsageArithmeticError {
            category: "priceRounding".to_string(),
        })?
        / TOKENS_PER_MILLION;
    u64::try_from(rounded)
        .map(Some)
        .map_err(|_| UsageArithmeticError {
            category: "priceTotal".to_string(),
        })
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
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub unknown_fields: BTreeSet<UsageUnknownField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reported_micro_usd: Option<u64>,
    #[serde(default = "default_source_record_count")]
    pub source_record_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<crate::ids::TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<ScheduleId>,
    #[serde(default, skip_serializing_if = "UsageFactKind::is_aggregate")]
    pub fact_kind: UsageFactKind,
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
        if matches!(&self.fact_kind, UsageFactKind::Unknown(value) if value.trim().is_empty()) {
            return Err(UsageObservationError::EmptyUnknownValue("fact kind"));
        }
        if self
            .model
            .as_ref()
            .is_some_and(|model| model.trim().is_empty())
        {
            return Err(UsageObservationError::EmptyModel);
        }
        if self
            .turn_id
            .as_ref()
            .is_some_and(|turn_id| turn_id.as_str().trim().is_empty())
        {
            return Err(UsageObservationError::EmptyIdentifier("turn id"));
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacityConfidence {
    Reported,
    Estimated,
    #[default]
    Unknown,
}

pub const CAPACITY_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_CAPACITY_MAX_AGE_MS: u64 = 15 * 60 * 1000;

fn capacity_snapshot_schema_version() -> u32 {
    CAPACITY_SNAPSHOT_SCHEMA_VERSION
}

fn default_capacity_max_age_ms() -> u64 {
    DEFAULT_CAPACITY_MAX_AGE_MS
}

/// Whether evidence was observed on this node or received from the Hub.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CapacityEvidenceScope {
    #[default]
    Local,
    Hub,
}

/// Freshness classification used wherever capacity affects a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacityFreshness {
    Fresh,
    Aging,
    Stale,
    Reset,
    ClockSkew,
    HardLimited,
}

/// Subscription quota window represented without floating-point fractions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacitySnapshot {
    #[serde(default = "capacity_snapshot_schema_version")]
    pub schema_version: u32,
    pub subscription_key: SubscriptionKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<EnvironmentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_instance_id: Option<ProviderInstanceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_name: Option<String>,
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
    #[serde(default)]
    pub evidence_scope: CapacityEvidenceScope,
    #[serde(default)]
    pub hard_limit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_version: Option<String>,
    pub as_of_ms: u64,
    #[serde(default = "default_capacity_max_age_ms")]
    pub max_age_ms: u64,
}

impl CapacitySnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.subscription_key.as_str().trim().is_empty() {
            return Err("subscription key must not be empty".to_string());
        }
        if self.source.trim().is_empty() {
            return Err("capacity source must not be empty".to_string());
        }
        if self.schema_version != CAPACITY_SNAPSHOT_SCHEMA_VERSION {
            return Err(format!(
                "unsupported capacity schema version {}",
                self.schema_version
            ));
        }
        if self.max_age_ms == 0 {
            return Err("capacity max age must be greater than zero".to_string());
        }
        for (field, value) in [
            (
                "environmentId",
                self.environment_id.as_ref().map(EnvironmentId::as_str),
            ),
            (
                "providerInstanceId",
                self.provider_instance_id
                    .as_ref()
                    .map(ProviderInstanceId::as_str),
            ),
            ("windowName", self.window_name.as_deref()),
        ] {
            if value.is_some_and(|value| value.trim().is_empty()) {
                return Err(format!("{field} must be absent or non-empty"));
            }
        }
        for (field, value) in [
            ("usedMillionths", self.used_millionths),
            ("remainingMillionths", self.remaining_millionths),
        ] {
            if value.is_some_and(|value| value > 1_000_000) {
                return Err(format!("{field} must be between 0 and 1000000"));
            }
        }
        if let (Some(used), Some(remaining)) = (self.used_millionths, self.remaining_millionths)
            && used + remaining != 1_000_000
        {
            return Err("usedMillionths and remainingMillionths must total 1000000".to_string());
        }
        if matches!(
            (self.window_start_ms, self.window_end_ms),
            (Some(start), Some(end)) if end < start
        ) {
            return Err("capacity window end must not precede its start".to_string());
        }
        if self.hard_limit {
            if self.remaining_millionths != Some(0) {
                return Err("hard-limit evidence must report zero remaining capacity".to_string());
            }
            if self.reset_at_ms.is_none() {
                return Err("hard-limit evidence must include its verified reset".to_string());
            }
        }
        Ok(())
    }
}

/// Safe projection of the latest revisions for one subscription quota window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityProjection {
    pub subscription_key: SubscriptionKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_millionths: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_millionths: Option<u32>,
    pub confidence: CapacityConfidence,
    pub freshness: CapacityFreshness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at_ms: Option<u64>,
    pub hard_limited: bool,
    pub conflicting_evidence: bool,
    pub evidence_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

/// Projects immutable capacity revisions conservatively at one wall-clock instant.
///
/// The latest revision from each evidence identity participates. A local hard limit
/// remains authoritative until its verified reset, even after its ordinary freshness
/// budget expires. Other conflicts choose the lowest known remaining fraction.
pub fn project_capacity_revisions(
    snapshots: &[CapacitySnapshot],
    now_ms: u64,
    allowed_future_skew_ms: u64,
) -> Vec<CapacityProjection> {
    type WindowKey = (SubscriptionKey, Option<String>, Option<u64>, Option<u64>);
    type EvidenceKey = (
        Option<EnvironmentId>,
        Option<ProviderInstanceId>,
        CapacityEvidenceScope,
        String,
    );

    let mut latest: BTreeMap<(WindowKey, EvidenceKey), &CapacitySnapshot> = BTreeMap::new();
    for snapshot in snapshots {
        if snapshot.validate().is_err() {
            continue;
        }
        let unnamed_window = snapshot.window_name.is_none();
        let window = (
            snapshot.subscription_key.clone(),
            snapshot.window_name.clone(),
            unnamed_window.then_some(snapshot.window_start_ms).flatten(),
            unnamed_window.then_some(snapshot.window_end_ms).flatten(),
        );
        let evidence = (
            snapshot.environment_id.clone(),
            snapshot.provider_instance_id.clone(),
            snapshot.evidence_scope,
            snapshot.source.clone(),
        );
        let key = (window, evidence);
        if latest
            .get(&key)
            .is_none_or(|existing| existing.as_of_ms <= snapshot.as_of_ms)
        {
            latest.insert(key, snapshot);
        }
    }

    let mut grouped: BTreeMap<WindowKey, Vec<&CapacitySnapshot>> = BTreeMap::new();
    for ((window, _), snapshot) in latest {
        grouped.entry(window).or_default().push(snapshot);
    }

    grouped
        .into_iter()
        .map(|((subscription_key, window_name, _, _), evidence)| {
            project_capacity_window(
                subscription_key,
                window_name,
                &evidence,
                now_ms,
                allowed_future_skew_ms,
            )
        })
        .collect()
}

fn project_capacity_window(
    subscription_key: SubscriptionKey,
    window_name: Option<String>,
    evidence: &[&CapacitySnapshot],
    now_ms: u64,
    allowed_future_skew_ms: u64,
) -> CapacityProjection {
    let mut reasons = BTreeSet::new();
    let mut eligible = Vec::new();
    let mut saw_reset = false;
    let mut saw_clock_skew = false;
    let mut saw_stale = false;

    for snapshot in evidence {
        if snapshot.reset_at_ms.is_some_and(|reset| reset <= now_ms) {
            saw_reset = true;
            reasons.insert("capacity_window_reset".to_string());
            continue;
        }

        let local_hard_limit =
            snapshot.hard_limit && snapshot.evidence_scope == CapacityEvidenceScope::Local;
        if snapshot.as_of_ms > now_ms.saturating_add(allowed_future_skew_ms) {
            saw_clock_skew = true;
            reasons.insert("capacity_clock_skew".to_string());
            if !local_hard_limit {
                continue;
            }
        }

        let age_ms = now_ms.saturating_sub(snapshot.as_of_ms);
        let stale = age_ms > snapshot.max_age_ms;
        if stale {
            saw_stale = true;
            reasons.insert("capacity_snapshot_stale".to_string());
            if !local_hard_limit {
                continue;
            }
        }
        eligible.push((*snapshot, age_ms, local_hard_limit));
    }

    let active_local_hard_limit = eligible
        .iter()
        .filter(|(_, _, hard_limit)| *hard_limit)
        .max_by_key(|(snapshot, _, _)| snapshot.as_of_ms)
        .map(|(snapshot, _, _)| *snapshot);
    let selected = active_local_hard_limit.or_else(|| {
        eligible
            .iter()
            .filter(|(snapshot, _, _)| snapshot.remaining_millionths.is_some())
            .min_by_key(|(snapshot, _, _)| {
                (
                    snapshot.remaining_millionths.unwrap_or(u32::MAX),
                    std::cmp::Reverse(snapshot.as_of_ms),
                )
            })
            .map(|(snapshot, _, _)| *snapshot)
    });
    let known_remaining = eligible
        .iter()
        .filter_map(|(snapshot, _, _)| snapshot.remaining_millionths)
        .collect::<BTreeSet<_>>();
    let conflicting_evidence = known_remaining.len() > 1;
    if conflicting_evidence {
        reasons.insert("capacity_evidence_conflict".to_string());
    }
    if active_local_hard_limit.is_some() {
        reasons.insert("local_hard_limit_active".to_string());
    }

    let (used_millionths, remaining_millionths, confidence, freshness, as_of_ms, reset_at_ms) =
        selected.map_or_else(
            || {
                let freshness = if saw_clock_skew {
                    CapacityFreshness::ClockSkew
                } else if saw_stale {
                    CapacityFreshness::Stale
                } else if saw_reset {
                    CapacityFreshness::Reset
                } else {
                    CapacityFreshness::Stale
                };
                (
                    None,
                    None,
                    CapacityConfidence::Unknown,
                    freshness,
                    None,
                    None,
                )
            },
            |snapshot| {
                let age_ms = now_ms.saturating_sub(snapshot.as_of_ms);
                let hard_limited = active_local_hard_limit.is_some();
                let aging = age_ms > snapshot.max_age_ms / 2;
                let confidence = if hard_limited {
                    CapacityConfidence::Reported
                } else if aging && snapshot.confidence == CapacityConfidence::Reported {
                    reasons.insert("capacity_confidence_age_decayed".to_string());
                    CapacityConfidence::Estimated
                } else {
                    snapshot.confidence
                };
                let freshness = if hard_limited {
                    CapacityFreshness::HardLimited
                } else if aging {
                    CapacityFreshness::Aging
                } else {
                    CapacityFreshness::Fresh
                };
                (
                    snapshot.used_millionths,
                    snapshot.remaining_millionths,
                    confidence,
                    freshness,
                    Some(snapshot.as_of_ms),
                    snapshot.reset_at_ms,
                )
            },
        );

    CapacityProjection {
        subscription_key,
        window_name,
        used_millionths,
        remaining_millionths,
        confidence,
        freshness,
        as_of_ms,
        reset_at_ms,
        hard_limited: active_local_hard_limit.is_some(),
        conflicting_evidence,
        evidence_count: u32::try_from(evidence.len()).unwrap_or(u32::MAX),
        reasons: reasons.into_iter().collect(),
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
            unknown_fields: BTreeSet::new(),
            provider_reported_micro_usd: Some(42),
            source_record_count: 1,
            thread_id: Some(ThreadId::new("thread-1")),
            turn_id: None,
            agent_id: None,
            schedule_id: None,
            fact_kind: UsageFactKind::Aggregate,
        }
    }

    fn capacity_snapshot(
        source: &str,
        as_of_ms: u64,
        remaining_millionths: Option<u32>,
    ) -> CapacitySnapshot {
        CapacitySnapshot {
            schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION,
            subscription_key: SubscriptionKey::new("subkey-capacity"),
            environment_id: Some(EnvironmentId::new("env-capacity")),
            provider_instance_id: Some(ProviderInstanceId::new("codex-capacity")),
            window_name: Some("primary".to_string()),
            window_start_ms: None,
            window_end_ms: None,
            used_millionths: remaining_millionths.map(|value| 1_000_000 - value),
            remaining_millionths,
            reset_at_ms: None,
            source: source.to_string(),
            confidence: CapacityConfidence::Reported,
            evidence_scope: CapacityEvidenceScope::Local,
            hard_limit: false,
            provider_version: Some("fixture".to_string()),
            as_of_ms,
            max_age_ms: 100_000,
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

    #[test]
    fn pricing_is_exact_half_up_and_unknown_stays_unknown() {
        let rates = UsagePricingRates {
            input_micro_usd_per_million: Some(500_000),
            output_micro_usd_per_million: Some(1_000_000),
            ..UsagePricingRates::default()
        };
        assert_eq!(
            estimate_usage_micro_usd(
                &TokenCounts {
                    input: 1,
                    ..TokenCounts::default()
                },
                &rates,
            )
            .expect("price one token"),
            Some(1)
        );
        assert_eq!(
            estimate_usage_micro_usd(
                &TokenCounts {
                    cached_input: 1,
                    ..TokenCounts::default()
                },
                &rates,
            )
            .expect("unknown cached rate"),
            None
        );
        assert_eq!(
            estimate_usage_micro_usd(
                &TokenCounts {
                    other: BTreeMap::from([("future".to_string(), 1)]),
                    ..TokenCounts::default()
                },
                &rates,
            )
            .expect("unknown other category"),
            None
        );
    }

    #[test]
    fn pricing_rejects_alias_collisions_and_checked_overflow() {
        let version = UsagePricingVersion {
            id: PricingVersionId::new("price-one"),
            provider: UsageProvider::Codex,
            source_published_at_ms: 10,
            effective_from_ms: 20,
            rules: vec![
                UsagePricingRule {
                    canonical_model_id: "gpt-one".to_string(),
                    aliases: BTreeSet::from(["shared".to_string()]),
                    rates: UsagePricingRates {
                        input_micro_usd_per_million: Some(1),
                        ..UsagePricingRates::default()
                    },
                },
                UsagePricingRule {
                    canonical_model_id: "gpt-two".to_string(),
                    aliases: BTreeSet::from(["shared".to_string()]),
                    rates: UsagePricingRates {
                        input_micro_usd_per_million: Some(1),
                        ..UsagePricingRates::default()
                    },
                },
            ],
        };
        assert!(version.validate().is_err());

        let overflow = estimate_usage_micro_usd(
            &TokenCounts {
                input: u64::MAX,
                output: u64::MAX,
                ..TokenCounts::default()
            },
            &UsagePricingRates {
                input_micro_usd_per_million: Some(u64::MAX),
                output_micro_usd_per_million: Some(u64::MAX),
                ..UsagePricingRates::default()
            },
        )
        .expect_err("price numerator overflow must fail");
        assert_eq!(overflow.category, "outputPrice");
    }

    #[test]
    fn capacity_confidence_decays_and_reset_stale_unknown_and_clock_skew_stay_visible() {
        let now = 1_000_000;
        let fresh = project_capacity_revisions(
            &[capacity_snapshot("fresh", now - 10_000, Some(800_000))],
            now,
            5_000,
        );
        assert_eq!(fresh[0].confidence, CapacityConfidence::Reported);
        assert_eq!(fresh[0].freshness, CapacityFreshness::Fresh);

        let aging = project_capacity_revisions(
            &[capacity_snapshot("aging", now - 60_000, Some(700_000))],
            now,
            5_000,
        );
        assert_eq!(aging[0].confidence, CapacityConfidence::Estimated);
        assert_eq!(aging[0].freshness, CapacityFreshness::Aging);

        let mut estimated = capacity_snapshot("estimated", now - 10_000, Some(650_000));
        estimated.confidence = CapacityConfidence::Estimated;
        let estimated = project_capacity_revisions(&[estimated], now, 5_000);
        assert_eq!(estimated[0].confidence, CapacityConfidence::Estimated);
        assert_eq!(estimated[0].freshness, CapacityFreshness::Fresh);
        assert_eq!(estimated[0].remaining_millionths, Some(650_000));

        let stale = project_capacity_revisions(
            &[capacity_snapshot("stale", now - 100_001, Some(600_000))],
            now,
            5_000,
        );
        assert_eq!(stale[0].confidence, CapacityConfidence::Unknown);
        assert_eq!(stale[0].freshness, CapacityFreshness::Stale);
        assert_eq!(stale[0].remaining_millionths, None);

        let mut unknown = capacity_snapshot("unknown", now, None);
        unknown.confidence = CapacityConfidence::Unknown;
        let unknown = project_capacity_revisions(&[unknown], now, 5_000);
        assert_eq!(unknown[0].confidence, CapacityConfidence::Unknown);
        assert_eq!(unknown[0].remaining_millionths, None);

        let mut reset = capacity_snapshot("reset", now - 10_000, Some(0));
        reset.reset_at_ms = Some(now);
        let reset = project_capacity_revisions(&[reset], now, 5_000);
        assert_eq!(reset[0].freshness, CapacityFreshness::Reset);
        assert_eq!(reset[0].remaining_millionths, None);

        let future = project_capacity_revisions(
            &[capacity_snapshot("future", now + 5_001, Some(900_000))],
            now,
            5_000,
        );
        assert_eq!(future[0].freshness, CapacityFreshness::ClockSkew);
        assert_eq!(future[0].remaining_millionths, None);

        let mut inconsistent = capacity_snapshot("inconsistent", now, Some(800_000));
        inconsistent.used_millionths = Some(300_000);
        assert!(inconsistent.validate().is_err());

        let mut unnamed_first = capacity_snapshot("unnamed", now, Some(800_000));
        unnamed_first.window_name = None;
        unnamed_first.window_start_ms = Some(100);
        unnamed_first.window_end_ms = Some(200);
        let mut unnamed_second = unnamed_first.clone();
        unnamed_second.window_start_ms = Some(300);
        unnamed_second.window_end_ms = Some(400);
        assert_eq!(
            project_capacity_revisions(&[unnamed_first, unnamed_second], now, 5_000).len(),
            2
        );
    }

    #[test]
    fn local_hard_limit_beats_fresh_hub_evidence_until_verified_reset() {
        let now = 2_000_000;
        let mut local = capacity_snapshot("local_rate_limit", now - 500_000, Some(0));
        local.hard_limit = true;
        local.reset_at_ms = Some(now + 60_000);
        local.max_age_ms = 1;
        let mut hub = capacity_snapshot("hub_snapshot", now, Some(900_000));
        hub.evidence_scope = CapacityEvidenceScope::Hub;

        let projected = project_capacity_revisions(&[local.clone(), hub.clone()], now, 5_000);
        assert_eq!(projected[0].remaining_millionths, Some(0));
        assert_eq!(projected[0].freshness, CapacityFreshness::HardLimited);
        assert!(projected[0].hard_limited);
        assert!(projected[0].conflicting_evidence);

        local.reset_at_ms = Some(now);
        let after_reset = project_capacity_revisions(&[local, hub], now, 5_000);
        assert_eq!(after_reset[0].remaining_millionths, Some(900_000));
        assert!(!after_reset[0].hard_limited);
    }
}
