use pandamux_core::{
    AgentId, CapacityConfidence, CapacityFreshness, EnvironmentId, ProviderInstanceId, ScheduleId,
    SubscriptionKey, UsageProvider, UsageSourceKind, UsageSubscription,
};
use pandamux_protocol::{
    CapacityQueryParams, CapacityQueryResult, UsageEnvironmentSyncState, UsageQueryParams,
    UsageQueryResult, UsageSourceCoverageState,
};

/// Age after which otherwise usable usage evidence is presented as stale.
pub const DEFAULT_USAGE_STALE_AFTER_MS: u64 = 15 * 60 * 1_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UsageViewStatus {
    #[default]
    Loading,
    Empty,
    Ready,
    Partial,
    Stale,
    Offline,
    MalformedSource,
    MigrationQuarantine,
    ServerError,
}

impl UsageViewStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Loading => "Loading",
            Self::Empty => "No usage in this range",
            Self::Ready => "Current",
            Self::Partial => "Partial coverage",
            Self::Stale => "Stale",
            Self::Offline => "Offline",
            Self::MalformedSource => "Malformed source",
            Self::MigrationQuarantine => "Migration quarantine",
            Self::ServerError => "Server error",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageFilters {
    pub from_ms: Option<u64>,
    pub to_ms: Option<u64>,
    pub environment_id: Option<EnvironmentId>,
    pub provider: Option<UsageProvider>,
    pub provider_instance_id: Option<ProviderInstanceId>,
    pub subscription_key: Option<SubscriptionKey>,
    pub agent_id: Option<AgentId>,
    pub schedule_id: Option<ScheduleId>,
    pub model: Option<String>,
    pub source_kind: Option<UsageSourceKind>,
}

/// Stable filter choices learned from successful result pages. Choices are
/// accumulated rather than replaced so narrowing a query does not make the
/// other previously visible values impossible to select.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageFilterOptions {
    pub environment_ids: Vec<EnvironmentId>,
    pub providers: Vec<UsageProvider>,
    pub provider_instance_ids: Vec<ProviderInstanceId>,
    pub subscription_keys: Vec<SubscriptionKey>,
    pub agent_ids: Vec<AgentId>,
    pub schedule_ids: Vec<ScheduleId>,
    pub models: Vec<String>,
    pub source_kinds: Vec<UsageSourceKind>,
}

impl UsageFilterOptions {
    fn absorb(&mut self, result: &UsageQueryResult) {
        for environment_id in &result.coverage.included_environments {
            push_unique(&mut self.environment_ids, environment_id.clone());
        }
        for item in &result.items {
            let observation = &item.observation;
            push_unique(&mut self.environment_ids, item.environment_id.clone());
            push_unique(&mut self.providers, observation.provider.clone());
            push_unique(
                &mut self.provider_instance_ids,
                observation.provider_instance_id.clone(),
            );
            push_unique(
                &mut self.subscription_keys,
                observation.subscription_key.clone(),
            );
            if let Some(agent_id) = &observation.agent_id {
                push_unique(&mut self.agent_ids, agent_id.clone());
            }
            if let Some(schedule_id) = &observation.schedule_id {
                push_unique(&mut self.schedule_ids, schedule_id.clone());
            }
            if let Some(model) = &observation.model {
                push_unique(&mut self.models, model.clone());
            }
            push_unique(&mut self.source_kinds, observation.source_kind.clone());
        }
    }
}

fn push_unique<T: PartialEq>(values: &mut Vec<T>, value: T) {
    if !values.contains(&value) {
        values.push(value);
    }
}

impl UsageFilters {
    pub fn to_query_params(&self) -> UsageQueryParams {
        UsageQueryParams {
            from_ms: self.from_ms,
            to_ms: self.to_ms,
            environment_id: self.environment_id.clone(),
            provider: self.provider.clone(),
            provider_instance_id: self.provider_instance_id.clone(),
            subscription_key: self.subscription_key.clone(),
            agent_id: self.agent_id.clone(),
            schedule_id: self.schedule_id.clone(),
            model: self.model.clone(),
            source_kind: self.source_kind.clone(),
            ..UsageQueryParams::default()
        }
    }

    pub fn to_capacity_params(&self) -> CapacityQueryParams {
        CapacityQueryParams {
            subscription_key: self.subscription_key.clone(),
            environment_id: self.environment_id.clone(),
            ..CapacityQueryParams::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageFilterChange {
    DateRange {
        from_ms: Option<u64>,
        to_ms: Option<u64>,
    },
    Environment(Option<EnvironmentId>),
    Provider(Option<UsageProvider>),
    ProviderInstance(Option<ProviderInstanceId>),
    Subscription(Option<SubscriptionKey>),
    Agent(Option<AgentId>),
    Schedule(Option<ScheduleId>),
    Model(Option<String>),
    Source(Option<UsageSourceKind>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageLoadFailure {
    Offline,
    MalformedSource,
    MigrationQuarantine,
    Server,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageAction {
    LoadStarted,
    Loaded {
        usage: Box<UsageQueryResult>,
        capacity: Box<CapacityQueryResult>,
        now_ms: u64,
    },
    LoadFailed {
        kind: UsageLoadFailure,
        message: String,
    },
    SetFilter(UsageFilterChange),
    ClearFilters,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageViewState {
    pub status: UsageViewStatus,
    pub filters: UsageFilters,
    pub filter_options: UsageFilterOptions,
    pub usage: Option<UsageQueryResult>,
    pub capacity: Option<CapacityQueryResult>,
    pub error_message: Option<String>,
    pub loaded_at_ms: Option<u64>,
}

impl UsageViewState {
    pub fn reduce(&mut self, action: UsageAction) {
        match action {
            UsageAction::LoadStarted => {
                self.status = UsageViewStatus::Loading;
                self.error_message = None;
            }
            UsageAction::Loaded {
                usage,
                capacity,
                now_ms,
            } => {
                self.status = classify_usage_result(&usage, &capacity, now_ms);
                self.filter_options.absorb(&usage);
                self.usage = Some(*usage);
                self.capacity = Some(*capacity);
                self.error_message = None;
                self.loaded_at_ms = Some(now_ms);
            }
            UsageAction::LoadFailed { kind, message } => {
                self.status = match kind {
                    UsageLoadFailure::Offline => UsageViewStatus::Offline,
                    UsageLoadFailure::MalformedSource => UsageViewStatus::MalformedSource,
                    UsageLoadFailure::MigrationQuarantine => UsageViewStatus::MigrationQuarantine,
                    UsageLoadFailure::Server => UsageViewStatus::ServerError,
                };
                self.error_message = Some(message);
            }
            UsageAction::SetFilter(change) => {
                match change {
                    UsageFilterChange::DateRange { from_ms, to_ms } => {
                        self.filters.from_ms = from_ms;
                        self.filters.to_ms = to_ms;
                    }
                    UsageFilterChange::Environment(value) => {
                        self.filters.environment_id = value;
                    }
                    UsageFilterChange::Provider(value) => self.filters.provider = value,
                    UsageFilterChange::ProviderInstance(value) => {
                        self.filters.provider_instance_id = value;
                    }
                    UsageFilterChange::Subscription(value) => {
                        self.filters.subscription_key = value;
                    }
                    UsageFilterChange::Agent(value) => self.filters.agent_id = value,
                    UsageFilterChange::Schedule(value) => self.filters.schedule_id = value,
                    UsageFilterChange::Model(value) => self.filters.model = normalized(value),
                    UsageFilterChange::Source(value) => self.filters.source_kind = value,
                }
                self.status = UsageViewStatus::Loading;
                self.error_message = None;
            }
            UsageAction::ClearFilters => {
                self.filters = UsageFilters::default();
                self.status = UsageViewStatus::Loading;
                self.error_message = None;
            }
        }
    }
}

fn normalized(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

pub fn classify_usage_result(
    usage: &UsageQueryResult,
    capacity: &CapacityQueryResult,
    now_ms: u64,
) -> UsageViewStatus {
    let has_data = usage.aggregate.observation_count > 0 || !usage.items.is_empty();
    let has_quarantine = usage.coverage.quarantined_count > 0
        || usage
            .coverage
            .scanner_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.quarantined_count > 0);
    let has_malformed = usage.coverage.scanner_diagnostics.iter().any(|diagnostic| {
        diagnostic
            .last_safe_error_code
            .as_deref()
            .is_some_and(is_malformed_code)
    });
    let has_offline = !capacity.offline_environments.is_empty()
        || usage
            .coverage
            .environment_sync
            .iter()
            .any(|coverage| matches!(coverage.state, UsageEnvironmentSyncState::Unreachable));
    let has_partial = has_quarantine
        || has_malformed
        || usage.coverage.environment_sync.iter().any(|coverage| {
            matches!(
                coverage.state,
                UsageEnvironmentSyncState::Reconciling
                    | UsageEnvironmentSyncState::CoverageGap
                    | UsageEnvironmentSyncState::Unreachable
            )
        })
        || usage.coverage.scanner_diagnostics.iter().any(|diagnostic| {
            !matches!(
                diagnostic.coverage_state,
                UsageSourceCoverageState::Complete
            )
        });
    let has_stale = usage
        .coverage
        .environment_sync
        .iter()
        .any(|coverage| matches!(coverage.state, UsageEnvironmentSyncState::Stale))
        || usage
            .coverage
            .latest_source_at_ms
            .is_some_and(|latest| now_ms.saturating_sub(latest) > DEFAULT_USAGE_STALE_AFTER_MS);

    if !has_data {
        if has_malformed {
            UsageViewStatus::MalformedSource
        } else if has_quarantine {
            UsageViewStatus::MigrationQuarantine
        } else if has_offline {
            UsageViewStatus::Offline
        } else {
            UsageViewStatus::Empty
        }
    } else if has_partial {
        UsageViewStatus::Partial
    } else if has_stale {
        UsageViewStatus::Stale
    } else {
        UsageViewStatus::Ready
    }
}

fn is_malformed_code(code: &str) -> bool {
    let code = code.to_ascii_lowercase();
    code.contains("malformed") || code.contains("parse") || code.contains("invalid_source")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapacityCard {
    pub subscription_name: String,
    pub provider: String,
    pub masked_account_label: Option<String>,
    pub window_name: String,
    pub used: String,
    pub remaining: String,
    pub confidence: String,
    pub freshness: String,
    pub source: String,
    pub as_of: String,
    pub reset: String,
    pub hard_limit: String,
    pub reasons: Vec<String>,
}

/// Convert capacity evidence into display-safe cards. Opaque subscription keys are
/// used only for joining and are never returned as presentation text.
pub fn capacity_cards(
    result: &CapacityQueryResult,
    subscriptions: &[UsageSubscription],
    now_ms: u64,
) -> Vec<CapacityCard> {
    result
        .effective
        .iter()
        .map(|projection| {
            let subscription = subscriptions
                .iter()
                .find(|subscription| subscription.key == projection.subscription_key);
            let matching_snapshots = result
                .snapshots
                .iter()
                .filter(|snapshot| {
                    snapshot.subscription_key == projection.subscription_key
                        && snapshot.window_name == projection.window_name
                })
                .collect::<Vec<_>>();
            let mut sources = matching_snapshots
                .iter()
                .map(|snapshot| snapshot.source.clone())
                .collect::<Vec<_>>();
            sources.sort();
            sources.dedup();
            CapacityCard {
                subscription_name: subscription
                    .map(|subscription| subscription.display_name.clone())
                    .unwrap_or_else(|| "Unnamed subscription".to_string()),
                provider: subscription
                    .map(|subscription| title_case_provider(&subscription.provider))
                    .unwrap_or_else(|| "Unknown provider".to_string()),
                masked_account_label: subscription
                    .and_then(|subscription| subscription.masked_account_label.clone()),
                window_name: projection
                    .window_name
                    .clone()
                    .unwrap_or_else(|| "Unnamed quota window".to_string()),
                used: format_millionths(projection.used_millionths),
                remaining: format_millionths(projection.remaining_millionths),
                confidence: confidence_label(projection.confidence).to_string(),
                freshness: freshness_label(projection.freshness).to_string(),
                source: if sources.is_empty() {
                    "Not reported".to_string()
                } else {
                    sources.join(", ")
                },
                as_of: format_age(now_ms, projection.as_of_ms),
                reset: projection
                    .reset_at_ms
                    .map(|reset| format_relative_time(now_ms, reset))
                    .unwrap_or_else(|| "Not reported".to_string()),
                hard_limit: if projection.hard_limited {
                    "Hard limit active".to_string()
                } else {
                    "No verified hard limit".to_string()
                },
                reasons: projection.reasons.clone(),
            }
        })
        .collect()
}

pub fn format_tokens(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(character);
    }
    result
}

pub fn format_micro_usd(value: Option<u64>) -> String {
    let Some(value) = value else {
        return "Unknown".to_string();
    };
    let whole = value / 1_000_000;
    let fraction = value % 1_000_000;
    if fraction == 0 {
        return format!("${whole}.00");
    }
    let mut fraction = format!("{fraction:06}");
    while fraction.len() > 2 && fraction.ends_with('0') {
        fraction.pop();
    }
    format!("${whole}.{fraction}")
}

pub fn format_millionths(value: Option<u32>) -> String {
    value
        .map(|value| {
            let whole = value / 10_000;
            let hundredths = (value % 10_000) / 100;
            format!("{whole}.{hundredths:02}%")
        })
        .unwrap_or_else(|| "Unknown".to_string())
}

pub fn format_age(now_ms: u64, timestamp_ms: Option<u64>) -> String {
    let Some(timestamp_ms) = timestamp_ms else {
        return "Not reported".to_string();
    };
    if timestamp_ms > now_ms {
        return "Clock skew".to_string();
    }
    let seconds = (now_ms - timestamp_ms) / 1_000;
    match seconds {
        0..=59 => format!("{seconds}s ago"),
        60..=3_599 => format!("{}m ago", seconds / 60),
        3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

pub fn format_relative_time(now_ms: u64, timestamp_ms: u64) -> String {
    if timestamp_ms >= now_ms {
        let minutes = (timestamp_ms - now_ms) / 60_000;
        if minutes < 60 {
            format!("in {minutes}m")
        } else {
            format!("in {}h", minutes / 60)
        }
    } else {
        format_age(now_ms, Some(timestamp_ms))
    }
}

pub fn confidence_label(value: CapacityConfidence) -> &'static str {
    match value {
        CapacityConfidence::Reported => "Reported",
        CapacityConfidence::Estimated => "Estimated",
        CapacityConfidence::Unknown => "Unknown",
    }
}

pub fn freshness_label(value: CapacityFreshness) -> &'static str {
    match value {
        CapacityFreshness::Fresh => "Fresh",
        CapacityFreshness::Aging => "Aging",
        CapacityFreshness::Stale => "Stale",
        CapacityFreshness::Reset => "Reset",
        CapacityFreshness::ClockSkew => "Clock skew",
        CapacityFreshness::HardLimited => "Hard limited",
    }
}

pub fn title_case_provider(provider: &UsageProvider) -> String {
    let value = provider.as_str();
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use pandamux_core::{
        CapacityProjection, CapacitySnapshot, TokenCounts, UsageObservation, UsageObservationId,
        UsageSourceKind,
    };
    use pandamux_protocol::{
        UsageAggregate, UsageCoverage, UsageEnvironmentSyncCoverage, UsageScannerDiagnostic,
    };

    use super::*;

    fn query_with(count: u64) -> UsageQueryResult {
        let items = if count == 0 {
            Vec::new()
        } else {
            vec![pandamux_protocol::UsageQueryItem {
                environment_id: EnvironmentId::new("env-local"),
                observation: UsageObservation {
                    id: UsageObservationId::new("observation-1"),
                    source_kind: UsageSourceKind::ManagedTurn,
                    parser_version: 1,
                    provider_native_session_id: None,
                    provider_native_turn_id: None,
                    occurred_at_ms: Some(10_000),
                    provider: UsageProvider::Codex,
                    provider_instance_id: ProviderInstanceId::new("codex-work"),
                    subscription_key: SubscriptionKey::new("opaque-secret-key"),
                    model: Some("gpt-test".to_string()),
                    tokens: TokenCounts {
                        input: 10,
                        ..TokenCounts::default()
                    },
                    unknown_fields: Default::default(),
                    provider_reported_micro_usd: None,
                    source_record_count: 1,
                    thread_id: None,
                    turn_id: None,
                    agent_id: None,
                    schedule_id: None,
                    fact_kind: Default::default(),
                },
            }]
        };
        UsageQueryResult {
            items,
            next: None,
            has_more: false,
            aggregate: UsageAggregate {
                observation_count: count,
                tokens: TokenCounts {
                    input: count.saturating_mul(10),
                    ..TokenCounts::default()
                },
                ..UsageAggregate::default()
            },
            coverage: UsageCoverage {
                quarantined_count: 0,
                included_environments: vec![EnvironmentId::new("env-local")],
                observation_sources: vec![UsageSourceKind::ManagedTurn],
                latest_source_at_ms: Some(10_000),
                unknown_price_tokens: 0,
                priceable_tokens: 10,
                unknown_price_token_millionths: None,
                environment_sync: Vec::new(),
                scanner_diagnostics: Vec::new(),
            },
        }
    }

    fn capacity() -> CapacityQueryResult {
        CapacityQueryResult {
            snapshots: Vec::new(),
            next: None,
            has_more: false,
            effective: Vec::new(),
            offline_environments: Vec::new(),
        }
    }

    fn sync(state: UsageEnvironmentSyncState) -> UsageEnvironmentSyncCoverage {
        UsageEnvironmentSyncCoverage {
            environment_id: EnvironmentId::new("env-local"),
            state,
            last_success_at_ms: None,
            last_safe_error_code: None,
            coverage_gap: None,
        }
    }

    fn diagnostic(code: Option<&str>, quarantined_count: u64) -> UsageScannerDiagnostic {
        UsageScannerDiagnostic {
            environment_id: EnvironmentId::new("env-local"),
            provider_instance_id: ProviderInstanceId::new("codex-work"),
            source_kind: UsageSourceKind::CodexTranscript,
            coverage_state: UsageSourceCoverageState::Unavailable,
            parser_version: 1,
            last_scan_started_at_ms: 10_000,
            last_success_at_ms: None,
            latest_source_at_ms: None,
            source_lag_ms: None,
            files_scanned: 0,
            bytes_scanned: 0,
            quarantined_count,
            last_safe_error_code: code.map(str::to_string),
        }
    }

    #[test]
    fn reducer_covers_every_required_data_and_error_state() {
        let mut state = UsageViewState::default();
        assert_eq!(state.status, UsageViewStatus::Loading);

        state.reduce(UsageAction::Loaded {
            usage: Box::new(query_with(0)),
            capacity: Box::new(capacity()),
            now_ms: 10_001,
        });
        assert_eq!(state.status, UsageViewStatus::Empty);

        state.reduce(UsageAction::Loaded {
            usage: Box::new(query_with(1)),
            capacity: Box::new(capacity()),
            now_ms: 10_001,
        });
        assert_eq!(state.status, UsageViewStatus::Ready);

        let mut partial = query_with(1);
        partial.coverage.environment_sync = vec![sync(UsageEnvironmentSyncState::CoverageGap)];
        state.reduce(UsageAction::Loaded {
            usage: Box::new(partial),
            capacity: Box::new(capacity()),
            now_ms: 10_001,
        });
        assert_eq!(state.status, UsageViewStatus::Partial);

        let mut stale = query_with(1);
        stale.coverage.environment_sync = vec![sync(UsageEnvironmentSyncState::Stale)];
        state.reduce(UsageAction::Loaded {
            usage: Box::new(stale),
            capacity: Box::new(capacity()),
            now_ms: 10_001,
        });
        assert_eq!(state.status, UsageViewStatus::Stale);

        let mut offline = query_with(0);
        offline.coverage.environment_sync = vec![sync(UsageEnvironmentSyncState::Unreachable)];
        state.reduce(UsageAction::Loaded {
            usage: Box::new(offline),
            capacity: Box::new(capacity()),
            now_ms: 10_001,
        });
        assert_eq!(state.status, UsageViewStatus::Offline);

        let mut malformed = query_with(0);
        malformed.coverage.scanner_diagnostics = vec![diagnostic(Some("source_parse_failed"), 0)];
        state.reduce(UsageAction::Loaded {
            usage: Box::new(malformed),
            capacity: Box::new(capacity()),
            now_ms: 10_001,
        });
        assert_eq!(state.status, UsageViewStatus::MalformedSource);

        let mut quarantined = query_with(0);
        quarantined.coverage.quarantined_count = 1;
        state.reduce(UsageAction::Loaded {
            usage: Box::new(quarantined),
            capacity: Box::new(capacity()),
            now_ms: 10_001,
        });
        assert_eq!(state.status, UsageViewStatus::MigrationQuarantine);

        state.reduce(UsageAction::LoadFailed {
            kind: UsageLoadFailure::Server,
            message: "request failed".to_string(),
        });
        assert_eq!(state.status, UsageViewStatus::ServerError);
        assert_eq!(state.error_message.as_deref(), Some("request failed"));

        for (kind, expected) in [
            (UsageLoadFailure::Offline, UsageViewStatus::Offline),
            (
                UsageLoadFailure::MalformedSource,
                UsageViewStatus::MalformedSource,
            ),
            (
                UsageLoadFailure::MigrationQuarantine,
                UsageViewStatus::MigrationQuarantine,
            ),
        ] {
            state.reduce(UsageAction::LoadFailed {
                kind,
                message: "bounded failure".to_string(),
            });
            assert_eq!(state.status, expected);
            assert_eq!(state.error_message.as_deref(), Some("bounded failure"));
        }
    }

    #[test]
    fn every_filter_maps_to_the_query_contract() {
        let mut state = UsageViewState::default();
        for change in [
            UsageFilterChange::DateRange {
                from_ms: Some(1),
                to_ms: Some(2),
            },
            UsageFilterChange::Environment(Some(EnvironmentId::new("env-local"))),
            UsageFilterChange::Provider(Some(UsageProvider::Claude)),
            UsageFilterChange::ProviderInstance(Some(ProviderInstanceId::new("claude-work"))),
            UsageFilterChange::Subscription(Some(SubscriptionKey::new("sub-work"))),
            UsageFilterChange::Agent(Some(AgentId::new("reviewer"))),
            UsageFilterChange::Schedule(Some(ScheduleId::new("nightly"))),
            UsageFilterChange::Model(Some(" claude-test ".to_string())),
            UsageFilterChange::Source(Some(UsageSourceKind::ClaudeTranscript)),
        ] {
            state.reduce(UsageAction::SetFilter(change));
        }

        let query = state.filters.to_query_params();
        assert_eq!(query.from_ms, Some(1));
        assert_eq!(query.to_ms, Some(2));
        assert_eq!(query.environment_id.unwrap().as_str(), "env-local");
        assert_eq!(query.provider, Some(UsageProvider::Claude));
        assert_eq!(query.provider_instance_id.unwrap().as_str(), "claude-work");
        assert_eq!(query.subscription_key.unwrap().as_str(), "sub-work");
        assert_eq!(query.agent_id.unwrap().as_str(), "reviewer");
        assert_eq!(query.schedule_id.unwrap().as_str(), "nightly");
        assert_eq!(query.model.as_deref(), Some("claude-test"));
        assert_eq!(query.source_kind, Some(UsageSourceKind::ClaudeTranscript));

        state.reduce(UsageAction::ClearFilters);
        assert!(state.filters.is_empty());
        assert_eq!(state.status, UsageViewStatus::Loading);
    }

    #[test]
    fn filter_options_survive_a_narrower_follow_up_result() {
        let mut first = query_with(1);
        let observation = &mut first.items[0].observation;
        observation.agent_id = Some(AgentId::new("reviewer"));
        observation.schedule_id = Some(ScheduleId::new("nightly"));
        let mut state = UsageViewState::default();
        state.reduce(UsageAction::Loaded {
            usage: Box::new(first),
            capacity: Box::new(capacity()),
            now_ms: 10_001,
        });

        state.reduce(UsageAction::Loaded {
            usage: Box::new(query_with(0)),
            capacity: Box::new(capacity()),
            now_ms: 10_002,
        });

        assert_eq!(state.filter_options.agent_ids[0].as_str(), "reviewer");
        assert_eq!(state.filter_options.schedule_ids[0].as_str(), "nightly");
        assert_eq!(state.filter_options.models[0], "gpt-test");
        assert_eq!(
            state.filter_options.subscription_keys[0].as_str(),
            "opaque-secret-key"
        );
    }

    #[test]
    fn capacity_cards_never_render_opaque_subscription_keys() {
        let key = SubscriptionKey::new("opaque-secret-key");
        let result = CapacityQueryResult {
            snapshots: vec![
                CapacitySnapshot {
                    schema_version: pandamux_core::CAPACITY_SNAPSHOT_SCHEMA_VERSION,
                    subscription_key: key.clone(),
                    environment_id: Some(EnvironmentId::new("env-local")),
                    provider_instance_id: Some(ProviderInstanceId::new("codex-work")),
                    window_name: Some("five-hour".to_string()),
                    window_start_ms: None,
                    window_end_ms: None,
                    used_millionths: Some(250_000),
                    remaining_millionths: Some(750_000),
                    reset_at_ms: Some(80_000),
                    source: "codex-cli".to_string(),
                    confidence: CapacityConfidence::Reported,
                    evidence_scope: Default::default(),
                    hard_limit: false,
                    provider_version: None,
                    as_of_ms: 20_000,
                    max_age_ms: 900_000,
                },
                CapacitySnapshot {
                    schema_version: pandamux_core::CAPACITY_SNAPSHOT_SCHEMA_VERSION,
                    subscription_key: key.clone(),
                    environment_id: Some(EnvironmentId::new("env-remote")),
                    provider_instance_id: Some(ProviderInstanceId::new("codex-work")),
                    window_name: Some("five-hour".to_string()),
                    window_start_ms: None,
                    window_end_ms: None,
                    used_millionths: Some(300_000),
                    remaining_millionths: Some(700_000),
                    reset_at_ms: Some(80_000),
                    source: "hub-snapshot".to_string(),
                    confidence: CapacityConfidence::Reported,
                    evidence_scope: Default::default(),
                    hard_limit: false,
                    provider_version: None,
                    as_of_ms: 19_000,
                    max_age_ms: 900_000,
                },
            ],
            next: None,
            has_more: false,
            effective: vec![CapacityProjection {
                subscription_key: key.clone(),
                window_name: Some("five-hour".to_string()),
                used_millionths: Some(250_000),
                remaining_millionths: Some(750_000),
                confidence: CapacityConfidence::Reported,
                freshness: CapacityFreshness::Fresh,
                as_of_ms: Some(20_000),
                reset_at_ms: Some(80_000),
                hard_limited: false,
                conflicting_evidence: false,
                evidence_count: 1,
                reasons: vec!["fresh reported evidence".to_string()],
            }],
            offline_environments: Vec::new(),
        };
        let subscriptions = vec![UsageSubscription {
            key,
            provider: UsageProvider::Codex,
            display_name: "Work Codex".to_string(),
            masked_account_label: Some("br***@example.com".to_string()),
        }];

        let cards = capacity_cards(&result, &subscriptions, 30_000);
        let rendered = format!("{cards:?}");
        assert!(rendered.contains("Work Codex"));
        assert!(rendered.contains("br***@example.com"));
        assert!(rendered.contains("codex-cli, hub-snapshot"));
        assert!(!rendered.contains("opaque-secret-key"));
    }

    #[test]
    fn exact_formatters_keep_unknown_values_visible() {
        assert_eq!(format_tokens(1_234_567), "1,234,567");
        assert_eq!(format_micro_usd(Some(1_250_000)), "$1.25");
        assert_eq!(format_micro_usd(None), "Unknown");
        assert_eq!(format_millionths(Some(125_000)), "12.50%");
        assert_eq!(format_millionths(None), "Unknown");
        assert_eq!(format_age(10_000, None), "Not reported");
        assert_eq!(format_age(10_000, Some(11_000)), "Clock skew");
    }
}
