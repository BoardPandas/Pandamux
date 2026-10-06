use gpui_kit::base::{Button as AccessibleButton, Disableable as _, StyledExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, ThemeStyled as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_client::{
    UsageViewState, UsageViewStatus, capacity_cards, format_age, format_micro_usd,
    format_millionths, format_tokens, title_case_provider,
};
use pandamux_core::{
    EnvironmentId, ProviderInstanceId, SubscriptionKey, UsageProvider, UserSettings,
};
use pandamux_protocol::UsageCatalogResult;

use crate::theme::{Radii, Theme, Typography};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageFilterIntent {
    Date,
    Environment,
    Provider,
    Subscription,
    Agent,
    Schedule,
    Model,
    Source,
}

impl UsageFilterIntent {
    pub const ALL: [Self; 8] = [
        Self::Date,
        Self::Environment,
        Self::Provider,
        Self::Subscription,
        Self::Agent,
        Self::Schedule,
        Self::Model,
        Self::Source,
    ];

    pub fn accessible_name(self) -> &'static str {
        match self {
            Self::Date => "Filter usage by date range",
            Self::Environment => "Filter usage by environment",
            Self::Provider => "Filter usage by provider",
            Self::Subscription => "Filter usage by named subscription",
            Self::Agent => "Filter usage by agent",
            Self::Schedule => "Filter usage by schedule",
            Self::Model => "Filter usage by model",
            Self::Source => "Filter usage by source",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageEditorTarget {
    pub subscription_key: Option<SubscriptionKey>,
    pub environment_id: EnvironmentId,
    pub provider_instance_id: ProviderInstanceId,
    pub provider: UsageProvider,
    pub provider_display_name: String,
}

#[derive(Clone, Copy)]
enum EditorButtonStyle {
    Primary,
    Ghost,
}

#[allow(clippy::too_many_arguments)]
pub fn render_usage_view<V: 'static>(
    state: &UsageViewState,
    catalog: &UsageCatalogResult,
    settings: &UserSettings,
    editor_target: Option<&UsageEditorTarget>,
    name_input: &Entity<InputState>,
    account_input: &Entity<InputState>,
    theme: &Theme,
    now_ms: u64,
    on_retry: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_filter: impl Fn(&mut V, UsageFilterIntent, &mut Window, &mut Context<V>) + 'static + Copy,
    on_clear_filters: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_edit: impl Fn(&mut V, UsageEditorTarget, &mut Window, &mut Context<V>) + 'static + Copy,
    on_save_editor: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_cancel_editor: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    window: &mut Window,
    cx: &mut Context<V>,
) -> AnyElement {
    div()
        .flex_1()
        .h_full()
        .v_flex()
        .bg(theme.chrome.bg_base_start)
        .overflow_hidden()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .px_4()
                .py_3()
                .bg(theme.chrome.panel)
                .border_b_1()
                .border_color(rgba(0xffffff0d))
                .child(
                    div()
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_size(Typography::TITLE_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t1)
                                .child("Usage & Capacity"),
                        )
                        .child(
                            div()
                                .text_size(Typography::SECONDARY_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child("Token totals, defensible spend evidence, quota windows, and source coverage."),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(status_badge(state.status, theme))
                        .child(
                            Button::new("usage-retry")
                                .ghost()
                                .label("Retry")
                                .accessibility_label("Retry loading usage and capacity")
                                .tooltip("Retry usage and capacity queries")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_retry(this, window, cx);
                                })),
                        ),
                ),
        )
        .child(
            div()
                .flex_1()
                .v_flex()
                .gap_4()
                .p_4()
                .overflow_y_scrollbar()
                .child(render_filters(
                    state,
                    catalog,
                    settings,
                    theme,
                    on_filter,
                    on_clear_filters,
                    cx,
                ))
                .when_some(state.error_message.as_ref(), |this, message| {
                    this.child(
                        div()
                            .p_3()
                            .rounded(Radii::ROW)
                            .border_1()
                            .border_color(rgba(0xf8717166))
                            .bg(rgba(0xf8717122))
                            .text_color(rgb(0xf87171))
                            .child(format!("Error: {message}")),
                    )
                })
                .child(render_state_explanation(state, theme))
                .when_some(state.usage.as_ref(), |this, result| {
                    this.child(render_totals(result, theme, now_ms))
                        .child(render_coverage_details(result, settings, theme, now_ms))
                        .child(render_usage_table(result, catalog, settings, theme, now_ms))
                })
                .when_some(state.capacity.as_ref(), |this, result| {
                    this.child(render_capacity_cards(
                        result, catalog, theme, now_ms,
                    ))
                })
                .child(render_subscription_editor(
                    catalog,
                    settings,
                    editor_target,
                    name_input,
                    account_input,
                    theme,
                    on_edit,
                    on_save_editor,
                    on_cancel_editor,
                    window,
                    cx,
                )),
        )
        .into_any_element()
}

fn status_badge(status: UsageViewStatus, theme: &Theme) -> AnyElement {
    let (marker, color, background) = match status {
        UsageViewStatus::Ready => ("●", theme.terminal.success, rgba(0x22c55e22)),
        UsageViewStatus::Loading => ("◌", theme.accent.color(), rgba(0x38bdf822)),
        UsageViewStatus::Empty => ("○", theme.chrome.text_t3, rgba(0xffffff0d)),
        UsageViewStatus::Partial
        | UsageViewStatus::Stale
        | UsageViewStatus::MigrationQuarantine => ("▲", theme.terminal.warn, rgba(0xfbbf2422)),
        UsageViewStatus::Offline
        | UsageViewStatus::MalformedSource
        | UsageViewStatus::ServerError => ("✕", rgb(0xf87171), rgba(0xf8717122)),
    };
    div()
        .px_2()
        .py_1()
        .rounded(Radii::CHIP)
        .bg(background)
        .text_size(Typography::META_SIZE)
        .font_weight(FontWeight::BOLD)
        .text_color(color)
        .child(format!("{marker} {}", status.label()))
        .into_any_element()
}

fn render_state_explanation(state: &UsageViewState, theme: &Theme) -> AnyElement {
    let text = match state.status {
        UsageViewStatus::Loading => "Loading usage, capacity, and source coverage…",
        UsageViewStatus::Empty => {
            "No observations match these filters. Unknown values remain Unknown rather than being inferred."
        }
        UsageViewStatus::Ready => "All reported sources in this result are current and complete.",
        UsageViewStatus::Partial => {
            "Totals are usable but at least one source is incomplete, reconciling, offline, or quarantined."
        }
        UsageViewStatus::Stale => {
            "The latest complete source evidence is older than the freshness window."
        }
        UsageViewStatus::Offline => {
            "No usable observations are available while one or more environments are unreachable."
        }
        UsageViewStatus::MalformedSource => {
            "A source could not be parsed. Its records are excluded until the parser succeeds."
        }
        UsageViewStatus::MigrationQuarantine => {
            "Only quarantined legacy or migration records match this view; they are not included in totals."
        }
        UsageViewStatus::ServerError => {
            "The server rejected or could not complete the request. The error remains visible until retry."
        }
    };
    div()
        .p_3()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .text_size(Typography::SECONDARY_SIZE)
        .text_color(theme.chrome.text_t2)
        .child(text)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_filters<V: 'static>(
    state: &UsageViewState,
    catalog: &UsageCatalogResult,
    settings: &UserSettings,
    theme: &Theme,
    on_filter: impl Fn(&mut V, UsageFilterIntent, &mut Window, &mut Context<V>) + 'static + Copy,
    on_clear_filters: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    div()
        .v_flex()
        .gap_2()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(Typography::GROUP_HEADER_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t2)
                        .child("FILTERS"),
                )
                .child(
                    Button::new("usage-clear-filters")
                        .ghost()
                        .label("Clear filters")
                        .disabled(state.filters.is_empty())
                        .accessibility_label("Clear all usage filters")
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_clear_filters(this, window, cx);
                        })),
                ),
        )
        .child(
            div()
                .h_flex()
                .flex_wrap()
                .gap_2()
                .children(UsageFilterIntent::ALL.into_iter().map(|intent| {
                    let label = filter_label(intent, state, catalog, settings);
                    Button::new(format!("usage-filter-{intent:?}"))
                        .ghost()
                        .label(label)
                        .accessibility_label(intent.accessible_name())
                        .tooltip(intent.accessible_name())
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_filter(this, intent, window, cx);
                        }))
                })),
        )
        .into_any_element()
}

fn filter_label(
    intent: UsageFilterIntent,
    state: &UsageViewState,
    catalog: &UsageCatalogResult,
    settings: &UserSettings,
) -> String {
    match intent {
        UsageFilterIntent::Date => match (state.filters.from_ms, state.filters.to_ms) {
            (Some(_), Some(_)) => "Date: custom range".to_string(),
            (Some(_), None) => "Date: last 7 days".to_string(),
            _ => "Date: all".to_string(),
        },
        UsageFilterIntent::Environment => match state.filters.environment_id.as_ref() {
            None => "Environment: all".to_string(),
            Some(id) => settings
                .environments
                .iter()
                .find(|environment| &environment.id == id)
                .map(|environment| format!("Environment: {}", environment.display_name))
                .unwrap_or_else(|| "Environment: unavailable name".to_string()),
        },
        UsageFilterIntent::Provider => state
            .filters
            .provider
            .as_ref()
            .map(|provider| format!("Provider: {}", title_case_provider(provider)))
            .unwrap_or_else(|| "Provider: all".to_string()),
        UsageFilterIntent::Subscription => match state.filters.subscription_key.as_ref() {
            None => "Subscription: all".to_string(),
            Some(key) => catalog
                .subscriptions
                .iter()
                .find(|subscription| &subscription.key == key)
                .map(|subscription| format!("Subscription: {}", subscription.display_name))
                .unwrap_or_else(|| "Subscription: unavailable name".to_string()),
        },
        UsageFilterIntent::Agent => state
            .filters
            .agent_id
            .as_ref()
            .map(|agent| format!("Agent: {}", agent.as_str()))
            .unwrap_or_else(|| "Agent: all".to_string()),
        UsageFilterIntent::Schedule => state
            .filters
            .schedule_id
            .as_ref()
            .map(|schedule| format!("Schedule: {}", schedule.as_str()))
            .unwrap_or_else(|| "Schedule: all".to_string()),
        UsageFilterIntent::Model => state
            .filters
            .model
            .as_ref()
            .map(|model| format!("Model: {model}"))
            .unwrap_or_else(|| "Model: all".to_string()),
        UsageFilterIntent::Source => state
            .filters
            .source_kind
            .as_ref()
            .map(|source| format!("Source: {}", source.as_str().replace('_', " ")))
            .unwrap_or_else(|| "Source: all".to_string()),
    }
}

fn render_totals(
    result: &pandamux_protocol::UsageQueryResult,
    theme: &Theme,
    now_ms: u64,
) -> AnyElement {
    let total_tokens = result
        .aggregate
        .tokens
        .checked_total()
        .map(format_tokens)
        .unwrap_or_else(|_| "Unknown".to_string());
    let freshness = format_age(now_ms, result.coverage.latest_source_at_ms);
    div()
        .v_flex()
        .gap_2()
        .child(section_heading("TOTALS & COVERAGE", theme))
        .child(
            div()
                .h_flex()
                .flex_wrap()
                .gap_3()
                .child(metric_card("Tokens", total_tokens, "Primary total", theme))
                .child(metric_card(
                    "API-equivalent estimate",
                    format_micro_usd(result.aggregate.api_equivalent_estimated_micro_usd),
                    if result.coverage.unknown_price_tokens > 0 {
                        "Partial: unpriced tokens excluded"
                    } else {
                        "Pricing-model estimate"
                    },
                    theme,
                ))
                .child(metric_card(
                    "Provider-reported spend",
                    format_micro_usd(result.aggregate.provider_reported_micro_usd),
                    "Reported only; never inferred",
                    theme,
                ))
                .child(metric_card(
                    "Source coverage",
                    format!(
                        "{} source{}",
                        result.coverage.observation_sources.len(),
                        if result.coverage.observation_sources.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ),
                    &format!("Latest evidence: {freshness}"),
                    theme,
                )),
        )
        .into_any_element()
}

fn metric_card(title: &str, value: String, note: &str, theme: &Theme) -> AnyElement {
    div()
        .min_w(px(190.0))
        .p_3()
        .v_flex()
        .gap_1()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel)
        .border_1()
        .border_color(rgba(0xffffff0d))
        .child(
            div()
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_size(px(20.0))
                .font_weight(FontWeight::BOLD)
                .text_color(theme.chrome.text_t1)
                .child(value),
        )
        .child(
            div()
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child(note.to_string()),
        )
        .into_any_element()
}

fn render_coverage_details(
    result: &pandamux_protocol::UsageQueryResult,
    settings: &UserSettings,
    theme: &Theme,
    now_ms: u64,
) -> AnyElement {
    let coverage = &result.coverage;
    div()
        .v_flex()
        .gap_2()
        .child(section_heading("SOURCE COVERAGE DETAILS", theme))
        .child(
            div()
                .p_3()
                .v_flex()
                .gap_1()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .child(format!(
                    "Included environments: {} · Quarantined records: {}",
                    coverage.included_environments.len(),
                    coverage.quarantined_count
                ))
                .child(format!(
                    "Unknown-price token share: {}",
                    format_millionths(coverage.unknown_price_token_millionths)
                )),
        )
        .when(coverage.environment_sync.is_empty(), |this| {
            this.child(
                div()
                    .p_3()
                    .rounded(Radii::ROW)
                    .bg(theme.chrome.panel2)
                    .text_color(theme.chrome.text_t3)
                    .child("Environment synchronization: Not reported"),
            )
        })
        .children(coverage.environment_sync.iter().map(|environment| {
            let environment_name = settings
                .environments
                .iter()
                .find(|candidate| candidate.id == environment.environment_id)
                .map(|candidate| candidate.display_name.clone())
                .unwrap_or_else(|| "Unnamed environment".to_string());
            let gap = environment.coverage_gap.as_ref().map(|gap| {
                format!(
                    " · coverage gap: requested {}, retained from {}, latest {}",
                    gap.requested_after_seq, gap.retained_from_seq, gap.latest_seq
                )
            });
            let error = environment
                .last_safe_error_code
                .as_deref()
                .map(|code| format!(" · error: {code}"))
                .unwrap_or_default();
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel2)
                .text_size(Typography::SECONDARY_SIZE)
                .text_color(theme.chrome.text_t2)
                .child(format!(
                    "{environment_name}: {} · last success {}{}{error}",
                    environment_sync_label(environment.state),
                    format_age(now_ms, environment.last_success_at_ms),
                    gap.unwrap_or_default(),
                ))
        }))
        .when(coverage.scanner_diagnostics.is_empty(), |this| {
            this.child(
                div()
                    .p_3()
                    .rounded(Radii::ROW)
                    .bg(theme.chrome.panel2)
                    .text_color(theme.chrome.text_t3)
                    .child("Scanner diagnostics: Not reported"),
            )
        })
        .children(coverage.scanner_diagnostics.iter().map(|diagnostic| {
            let provider_name = settings
                .providers
                .iter()
                .find(|provider| provider.id == diagnostic.provider_instance_id)
                .map(|provider| provider.display_name.clone())
                .unwrap_or_else(|| "Unnamed provider instance".to_string());
            let safe_error = diagnostic
                .last_safe_error_code
                .as_deref()
                .unwrap_or("Not reported");
            let lag = diagnostic
                .source_lag_ms
                .map(|lag| format!("{}s", lag / 1_000))
                .unwrap_or_else(|| "Not reported".to_string());
            div()
                .p_3()
                .v_flex()
                .gap_1()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel2)
                .text_size(Typography::SECONDARY_SIZE)
                .text_color(theme.chrome.text_t2)
                .child(format!(
                    "{provider_name} · {} · {}",
                    diagnostic.source_kind.as_str().replace('_', " "),
                    source_coverage_label(diagnostic.coverage_state)
                ))
                .child(format!(
                    "Last success: {} · source lag: {lag} · parser v{} · quarantined: {} · error: {safe_error}",
                    format_age(now_ms, diagnostic.last_success_at_ms),
                    diagnostic.parser_version,
                    diagnostic.quarantined_count
                ))
        }))
        .into_any_element()
}

fn environment_sync_label(state: pandamux_protocol::UsageEnvironmentSyncState) -> &'static str {
    match state {
        pandamux_protocol::UsageEnvironmentSyncState::Ready => "Ready",
        pandamux_protocol::UsageEnvironmentSyncState::Reconciling => "Reconciling",
        pandamux_protocol::UsageEnvironmentSyncState::CoverageGap => "Coverage gap",
        pandamux_protocol::UsageEnvironmentSyncState::Stale => "Stale",
        pandamux_protocol::UsageEnvironmentSyncState::Unreachable => "Unreachable",
    }
}

fn source_coverage_label(state: pandamux_protocol::UsageSourceCoverageState) -> &'static str {
    match state {
        pandamux_protocol::UsageSourceCoverageState::Complete => "Complete",
        pandamux_protocol::UsageSourceCoverageState::Partial => "Partial",
        pandamux_protocol::UsageSourceCoverageState::Unavailable => "Unavailable",
    }
}

fn render_usage_table(
    result: &pandamux_protocol::UsageQueryResult,
    catalog: &UsageCatalogResult,
    settings: &UserSettings,
    theme: &Theme,
    now_ms: u64,
) -> AnyElement {
    div()
        .v_flex()
        .gap_2()
        .child(section_heading("USAGE DATA TABLE", theme))
        .child(
            div()
                .v_flex()
                .rounded(Radii::ROW)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .child(table_row(
                    [
                        "Provider instance",
                        "Subscription",
                        "Model",
                        "Tokens",
                        "Source",
                        "Observed",
                    ],
                    true,
                    theme,
                ))
                .children(result.items.iter().map(|item| {
                    let observation = &item.observation;
                    let instance_name = settings
                        .providers
                        .iter()
                        .find(|provider| provider.id == observation.provider_instance_id)
                        .map(|provider| provider.display_name.clone())
                        .unwrap_or_else(|| {
                            format!("{} instance", title_case_provider(&observation.provider))
                        });
                    let subscription_name = catalog
                        .subscriptions
                        .iter()
                        .find(|subscription| subscription.key == observation.subscription_key)
                        .map(|subscription| subscription.display_name.clone())
                        .unwrap_or_else(|| "Unnamed subscription".to_string());
                    let tokens = observation
                        .tokens
                        .checked_total()
                        .map(format_tokens)
                        .unwrap_or_else(|_| "Unknown".to_string());
                    table_row(
                        [
                            instance_name,
                            subscription_name,
                            observation
                                .model
                                .clone()
                                .unwrap_or_else(|| "Unknown".to_string()),
                            tokens,
                            observation.source_kind.as_str().replace('_', " "),
                            format_age(now_ms, observation.occurred_at_ms),
                        ],
                        false,
                        theme,
                    )
                })),
        )
        .into_any_element()
}

fn table_row(values: [impl Into<SharedString>; 6], header: bool, theme: &Theme) -> AnyElement {
    div()
        .h_flex()
        .min_h(px(36.0))
        .px_2()
        .border_b_1()
        .border_color(rgba(0xffffff08))
        .bg(if header {
            theme.chrome.panel2
        } else {
            theme.chrome.panel
        })
        .children(values.into_iter().map(|value| {
            div()
                .flex_1()
                .min_w(px(100.0))
                .text_size(if header {
                    Typography::META_SIZE
                } else {
                    Typography::SECONDARY_SIZE
                })
                .font_weight(if header {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if header {
                    theme.chrome.text_t2
                } else {
                    theme.chrome.text_t1
                })
                .child(value.into())
        }))
        .into_any_element()
}

fn render_capacity_cards(
    result: &pandamux_protocol::CapacityQueryResult,
    catalog: &UsageCatalogResult,
    theme: &Theme,
    now_ms: u64,
) -> AnyElement {
    let cards = capacity_cards(result, &catalog.subscriptions, now_ms);
    div()
        .v_flex()
        .gap_2()
        .child(section_heading("CAPACITY", theme))
        .when(cards.is_empty(), |this| {
            this.child(
                div()
                    .p_3()
                    .rounded(Radii::ROW)
                    .bg(theme.chrome.panel)
                    .text_color(theme.chrome.text_t3)
                    .child("Capacity: Not reported"),
            )
        })
        .children(cards.into_iter().map(|card| {
            div()
                .p_3()
                .v_flex()
                .gap_2()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t1)
                                .child(format!(
                                    "{} · {}",
                                    card.subscription_name, card.window_name
                                )),
                        )
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t2)
                                .child(format!("{} · {}", card.confidence, card.freshness)),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child(card.masked_account_label.map_or_else(
                            || card.provider.clone(),
                            |label| format!("{} · {label}", card.provider),
                        )),
                )
                .child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .gap_3()
                        .child(format!("Used: {}", card.used))
                        .child(format!("Remaining: {}", card.remaining))
                        .child(format!("Source: {}", card.source))
                        .child(format!("As of: {}", card.as_of))
                        .child(format!("Reset: {}", card.reset))
                        .child(card.hard_limit),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child(if card.reasons.is_empty() {
                            "Evidence notes: no discount or exclusion reported".to_string()
                        } else {
                            format!("Evidence notes: {}", card.reasons.join("; "))
                        }),
                )
        }))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_subscription_editor<V: 'static>(
    catalog: &UsageCatalogResult,
    settings: &UserSettings,
    editor_target: Option<&UsageEditorTarget>,
    name_input: &Entity<InputState>,
    account_input: &Entity<InputState>,
    theme: &Theme,
    on_edit: impl Fn(&mut V, UsageEditorTarget, &mut Window, &mut Context<V>) + 'static + Copy,
    on_save_editor: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_cancel_editor: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    window: &mut Window,
    cx: &mut Context<V>,
) -> AnyElement {
    div()
        .v_flex()
        .gap_2()
        .child(section_heading("SUBSCRIPTIONS & INSTANCES", theme))
        .child(
            div()
                .text_size(Typography::SECONDARY_SIZE)
                .text_color(theme.chrome.text_t3)
                .child("Names are user-defined. Account labels are optional and must stay masked."),
        )
        .children(settings.providers.iter().map(|provider| {
            let environment_id = provider
                .environment_id
                .clone()
                .unwrap_or_else(|| EnvironmentId::new("env-local"));
            let mapping = catalog.mappings.iter().find(|mapping| {
                mapping.environment_id == environment_id
                    && mapping.provider_instance_id == provider.id
            });
            let subscription = mapping.and_then(|mapping| {
                catalog
                    .subscriptions
                    .iter()
                    .find(|subscription| subscription.key == mapping.subscription_key)
            });
            let target = UsageEditorTarget {
                subscription_key: subscription.map(|subscription| subscription.key.clone()),
                environment_id,
                provider_instance_id: provider.id.clone(),
                provider: UsageProvider::from(provider.provider),
                provider_display_name: provider.display_name.clone(),
            };
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .child(
                    div()
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t1)
                                .child(provider.display_name.clone()),
                        )
                        .child(
                            div()
                                .text_size(Typography::SECONDARY_SIZE)
                                .text_color(theme.chrome.text_t2)
                                .child(subscription.map_or_else(
                                    || "Subscription: Not named".to_string(),
                                    |subscription| {
                                        format!(
                                            "Subscription: {} · {}",
                                            subscription.display_name,
                                            subscription
                                                .masked_account_label
                                                .as_deref()
                                                .unwrap_or("No account label")
                                        )
                                    },
                                )),
                        ),
                )
                .child(
                    Button::new(format!("usage-edit-instance-{}", provider.id.as_str()))
                        .ghost()
                        .label(if subscription.is_some() {
                            "Edit"
                        } else {
                            "Name subscription"
                        })
                        .accessibility_label(format!(
                            "Edit subscription for {}",
                            provider.display_name
                        ))
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_edit(this, target.clone(), window, cx);
                        })),
                )
        }))
        .when_some(editor_target, |this, target| {
            this.child(
                div()
                    .p_3()
                    .v_flex()
                    .gap_2()
                    .rounded(Radii::ROW)
                    .bg(theme.chrome.panel2)
                    .border_1()
                    .border_color(theme.accent.color())
                    .child(div().font_weight(FontWeight::BOLD).child(format!(
                        "Edit {} subscription",
                        target.provider_display_name
                    )))
                    .child(
                        div().v_flex().gap_1().child("Subscription name").child(
                            Input::new(name_input)
                                .accessibility_id("usage.subscription.name")
                                .aria_label("Subscription name"),
                        ),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child("Masked account label (optional)")
                            .child(
                                Input::new(account_input)
                                    .accessibility_id("usage.subscription.masked-account")
                                    .aria_label("Masked account label"),
                            ),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(editor_action_button(
                                "usage-save-subscription",
                                "Save subscription",
                                EditorButtonStyle::Primary,
                                on_save_editor,
                                window,
                                cx,
                            ))
                            .child(editor_action_button(
                                "usage-cancel-subscription",
                                "Cancel",
                                EditorButtonStyle::Ghost,
                                on_cancel_editor,
                                window,
                                cx,
                            )),
                    ),
            )
        })
        .into_any_element()
}

fn editor_action_button<V: 'static>(
    id: &'static str,
    label: &'static str,
    style: EditorButtonStyle,
    on_activate: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    window: &mut Window,
    cx: &mut Context<V>,
) -> AnyElement {
    // gpui-component's styled Button does not expose StatefulInteractiveElement,
    // so UI Automation Click falls back to pointer synthesis. Keep the component's
    // presentation here while giving these editor actions a direct Click handler.
    let component_theme = cx.theme();
    let (background, hover_background, active_background, foreground, hover_foreground): (
        Background,
        Background,
        Background,
        Hsla,
        Hsla,
    ) = match style {
        EditorButtonStyle::Primary => (
            component_theme.tokens.button_primary.into(),
            component_theme.tokens.button_primary_hover.into(),
            component_theme.tokens.button_primary_active.into(),
            component_theme.button_primary_foreground,
            component_theme.button_primary_foreground,
        ),
        EditorButtonStyle::Ghost => {
            let accent: Background = component_theme.tokens.accent.into();
            let hover_background = if component_theme.mode.is_dark() {
                accent.opacity(0.5)
            } else {
                accent
            };
            (
                component_theme.transparent.into(),
                hover_background,
                component_theme.tokens.button_active.into(),
                component_theme.secondary_foreground,
                component_theme.accent_foreground,
            )
        }
    };
    let active_foreground = foreground;
    let radius = component_theme.radius;
    let focus_handle = window
        .use_keyed_state(id, cx, |_, cx| cx.focus_handle())
        .read(cx)
        .clone();
    let is_focused = focus_handle.is_focused(window);

    AccessibleButton::new(id)
        .accessibility_label(label)
        .flex_shrink_0()
        .cursor_default()
        .h_8()
        .px_2p5()
        .rounded(radius)
        .bg(background)
        .text_color(foreground)
        .text_base()
        .line_height(relative(1.25))
        .hover(move |style| style.bg(hover_background).text_color(hover_foreground))
        .active(move |style| style.bg(active_background).text_color(active_foreground))
        .track_focus(&focus_handle)
        .on_click(cx.listener(move |this, _event, window, cx| {
            on_activate(this, window, cx);
        }))
        .on_a11y_action(AccessibleAction::Click, {
            let activate = cx.listener(move |this, _event: &(), window, cx| {
                on_activate(this, window, cx);
            });
            move |_data, window, cx| activate(&(), window, cx)
        })
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(label),
        )
        .when(is_focused, |button| button.focus_ring_style(window, cx))
        .into_any_element()
}

fn section_heading(label: &str, theme: &Theme) -> AnyElement {
    div()
        .text_size(Typography::GROUP_HEADER_SIZE)
        .font_weight(FontWeight::BOLD)
        .text_color(theme.chrome.text_t2)
        .child(label.to_string())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn usage_controls_have_unique_accessible_names() {
        let names = UsageFilterIntent::ALL.map(UsageFilterIntent::accessible_name);
        assert!(names.iter().all(|name| !name.is_empty()));
        let unique = names.into_iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), UsageFilterIntent::ALL.len());
    }

    #[test]
    fn active_filters_never_masquerade_as_all() {
        let mut state = UsageViewState::default();
        state.filters.environment_id = Some(EnvironmentId::new("env-missing"));
        state.filters.subscription_key = Some(SubscriptionKey::new("subscription-hidden"));
        state.filters.agent_id = Some(pandamux_core::AgentId::new("reviewer"));
        state.filters.schedule_id = Some(pandamux_core::ScheduleId::new("nightly"));
        let catalog = UsageCatalogResult::default();
        let settings = UserSettings::default();

        assert_eq!(
            filter_label(UsageFilterIntent::Environment, &state, &catalog, &settings),
            "Environment: unavailable name"
        );
        assert_eq!(
            filter_label(UsageFilterIntent::Subscription, &state, &catalog, &settings),
            "Subscription: unavailable name"
        );
        assert_eq!(
            filter_label(UsageFilterIntent::Agent, &state, &catalog, &settings),
            "Agent: reviewer"
        );
        assert_eq!(
            filter_label(UsageFilterIntent::Schedule, &state, &catalog, &settings),
            "Schedule: nightly"
        );
    }
}
