use gpui_kit::base::{Disableable as _, StyledExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_client::{format_age, format_micro_usd};
use pandamux_core::{EnvironmentId, ProviderInstanceId, UserSettings};
use pandamux_protocol::{ProviderHealthReport, ProviderLoginPreview, UsageCatalogResult};

use crate::theme::{AccentColor, Radii, Theme, Typography};

/// Top-level categories in the Settings view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsCategory {
    #[default]
    Providers,
    Organizations,
    Environments,
    Terminal,
    General,
    Advanced,
}

impl SettingsCategory {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Providers => "Providers & Tier Mapping",
            Self::Organizations => "Organizations & Policy",
            Self::Environments => "Remote Environments & Nodes",
            Self::Terminal => "Terminal Configuration",
            Self::General => "General & Appearance",
            Self::Advanced => "Advanced Engine & Diagnostics",
        }
    }

    pub fn subtitle(&self) -> &'static str {
        match self {
            Self::Providers => {
                "Configure scoped provider instances, model targets, health evidence, and explicit login actions."
            }
            Self::Organizations => {
                "Edit canonical project, provider, subscription, budget, and Broadside boundaries."
            }
            Self::Environments => {
                "Manage local and remote SSH environments, daemon versions, bootstrap states, and host connectivity."
            }
            Self::Terminal => {
                "Configure scrollback history buffer, default shell family, and interaction options."
            }
            Self::General => {
                "Customize UI chrome theme, accent tint, status bar visibility, audio chimes, and notifications."
            }
            Self::Advanced => {
                "Engine concurrency boundaries, Git checkpoint retention, attachment size caps, and server connection status."
            }
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Providers => "🔑",
            Self::Organizations => "🏢",
            Self::Environments => "🖥️",
            Self::Terminal => "📟",
            Self::General => "🎨",
            Self::Advanced => "⚙️",
        }
    }
}

/// State for the Settings page and active edits.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsViewState {
    pub active_category: SettingsCategory,
    pub settings: UserSettings,
    pub health_reports: Vec<ProviderHealthReport>,
    pub environments: Vec<pandamux_core::Environment>,
    pub is_checking_health: bool,
    pub is_saving: bool,
    pub status_banner: Option<(String, bool)>, // (message, is_error)
    pub login_preview: Option<ProviderLoginPreview>,
    pub is_login_busy: bool,
}

impl Default for SettingsViewState {
    fn default() -> Self {
        Self {
            active_category: SettingsCategory::Providers,
            settings: UserSettings::default(),
            health_reports: Vec::new(),
            environments: vec![pandamux_core::Environment::local_default()],
            is_checking_health: false,
            is_saving: false,
            status_banner: None,
            login_preview: None,
            is_login_busy: false,
        }
    }
}

impl SettingsViewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_category(&mut self, cat: SettingsCategory) {
        self.active_category = cat;
        self.status_banner = None;
    }

    /// Import SSH host configurations into environments.
    pub fn import_ssh_hosts(&mut self, config_text: Option<&str>) -> usize {
        let text = if let Some(t) = config_text {
            t.to_string()
        } else {
            pandamux_core::read_default_ssh_config().unwrap_or_default()
        };

        let (new_envs, _, skipped) =
            pandamux_core::import_ssh_config_into_environments(&text, &self.environments);
        let count = new_envs.len();
        if count > 0 {
            self.environments.extend(new_envs);
            self.status_banner = Some((
                format!(
                    "Successfully imported {count} environment(s) from ~/.ssh/config ({skipped} already existed)"
                ),
                false,
            ));
        } else if skipped > 0 {
            self.status_banner = Some((
                format!("All {skipped} host(s) in ~/.ssh/config already exist in environments"),
                false,
            ));
        } else {
            self.status_banner = Some((
                "No connectable hosts found in ~/.ssh/config".to_string(),
                false,
            ));
        }
        count
    }
}

fn format_number(n: impl Into<u64>) -> String {
    let s = n.into().to_string();
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*c);
    }
    out
}

const ORGANIZATION_BUDGET_STEP_MICRO_USD: u64 = 10_000_000;

fn increase_organization_budget(current: Option<u64>) -> Option<u64> {
    Some(
        current
            .unwrap_or_default()
            .saturating_add(ORGANIZATION_BUDGET_STEP_MICRO_USD),
    )
}

fn decrease_organization_budget(current: Option<u64>) -> Option<u64> {
    current.and_then(|value| {
        value
            .checked_sub(ORGANIZATION_BUDGET_STEP_MICRO_USD)
            .filter(|remaining| *remaining > 0)
    })
}

fn increase_organization_concurrency(current: Option<u32>) -> Option<u32> {
    Some(current.unwrap_or_default().saturating_add(1).max(1))
}

fn decrease_organization_concurrency(current: Option<u32>) -> Option<u32> {
    current.and_then(|value| value.checked_sub(1).filter(|remaining| *remaining > 0))
}

/// Renders the complete Settings view based on active category.
#[allow(clippy::too_many_arguments)]
pub fn render_settings_view<V: 'static>(
    state: &SettingsViewState,
    usage_catalog: &UsageCatalogResult,
    theme: &Theme,
    server_status: String,
    on_save: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_check_health: impl Fn(&mut V, Option<ProviderInstanceId>, &mut Window, &mut Context<V>)
    + 'static
    + Copy,
    on_preview_login: impl Fn(&mut V, ProviderInstanceId, EnvironmentId, &mut Window, &mut Context<V>)
    + 'static
    + Copy,
    on_launch_login: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_cancel_login: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_modify: impl Fn(
        &mut V,
        Box<dyn FnOnce(&mut UserSettings) + Send + 'static>,
        &mut Window,
        &mut Context<V>,
    )
    + 'static
    + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    div()
        .flex_1()
        .h_full()
        .v_flex()
        .bg(theme.chrome.bg_base_start)
        .p_4()
        .gap_4()
        .overflow_hidden()
        // Top Header Bar
        .child(render_settings_header(
            state,
            theme,
            on_save,
            on_check_health,
            cx,
        ))
        // Status Banner if present
        .when_some(state.status_banner.as_ref(), |this, (msg, is_err)| {
            let bg_color = if *is_err {
                rgba(0xf8717122)
            } else {
                rgba(0x43d9c922)
            };
            let text_color = if *is_err {
                rgb(0xf87171)
            } else {
                rgb(0x43d9c9)
            };
            let border_color = if *is_err {
                rgba(0xf8717144)
            } else {
                rgba(0x43d9c944)
            };
            this.child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .rounded(Radii::ROW)
                    .bg(bg_color)
                    .border_1()
                    .border_color(border_color)
                    .text_size(Typography::BODY_SIZE)
                    .text_color(text_color)
                    .child(msg.clone()),
            )
        })
        // Category Content Body
        .child(match state.active_category {
            SettingsCategory::Providers => render_providers_tab(
                state,
                theme,
                on_check_health,
                on_preview_login,
                on_launch_login,
                on_cancel_login,
                on_modify,
                cx,
            )
            .into_any_element(),
            SettingsCategory::Organizations => {
                render_organizations_tab(state, usage_catalog, theme, on_modify, cx)
                    .into_any_element()
            }
            SettingsCategory::Environments => {
                render_environments_tab(state, theme, cx).into_any_element()
            }
            SettingsCategory::Terminal => {
                render_terminal_tab(state, theme, on_modify, cx).into_any_element()
            }
            SettingsCategory::General => {
                render_general_tab(state, theme, on_modify, cx).into_any_element()
            }
            SettingsCategory::Advanced => {
                render_advanced_tab(state, theme, server_status, on_modify, cx).into_any_element()
            }
        })
        .into_any_element()
}

/// Renders the Settings category header and save actions.
fn render_settings_header<V: 'static>(
    state: &SettingsViewState,
    theme: &Theme,
    on_save: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_check_health: impl Fn(&mut V, Option<ProviderInstanceId>, &mut Window, &mut Context<V>)
    + 'static
    + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    let cat = state.active_category;

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
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(div().text_size(Typography::TITLE_SIZE).child(cat.icon()))
                        .child(
                            div()
                                .text_size(Typography::TITLE_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t1)
                                .child(cat.title()),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::SECONDARY_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child(cat.subtitle()),
                ),
        )
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .when(cat == SettingsCategory::Providers, |this| {
                    this.child(
                        Button::new("btn-check-health")
                            .ghost()
                            .label(if state.is_checking_health {
                                "Checking Health..."
                            } else {
                                "⚡ Check Health"
                            })
                            .disabled(state.is_checking_health)
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                on_check_health(this, None, window, cx);
                            })),
                    )
                })
                .child(
                    Button::new("btn-save-settings")
                        .primary()
                        .label(if state.is_saving {
                            "Saving..."
                        } else {
                            "💾 Save Changes"
                        })
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_save(this, window, cx);
                        })),
                ),
        )
        .into_any_element()
}

/// Renders the Providers & Tier Mapping settings panel.
#[allow(clippy::too_many_arguments)]
fn render_providers_tab<V: 'static>(
    state: &SettingsViewState,
    theme: &Theme,
    on_check_health: impl Fn(&mut V, Option<ProviderInstanceId>, &mut Window, &mut Context<V>)
    + 'static
    + Copy,
    on_preview_login: impl Fn(&mut V, ProviderInstanceId, EnvironmentId, &mut Window, &mut Context<V>)
    + 'static
    + Copy,
    on_launch_login: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_cancel_login: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_modify: impl Fn(
        &mut V,
        Box<dyn FnOnce(&mut UserSettings) + Send + 'static>,
        &mut Window,
        &mut Context<V>,
    )
    + 'static
    + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    div()
        .v_flex()
        .gap_4()
        // 1. Health checks notice & summary banner
        .child(
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel2)
                .border_1()
                .border_color(rgba(0xffffff08))
                .v_flex()
                .gap_1p5()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(Typography::TITLE_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t1)
                        .child("Provider Health Evidence"),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0x43d9c922))
                                .text_size(Typography::META_SIZE)
                                .text_color(rgb(0x43d9c9))
                                .child("Login Is Always Separate"),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::SECONDARY_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child("Health probes may run bounded non-interactive status/version commands. They never start an interactive login or provider session; login requires a separate preview and confirmation."),
                ),
        )
        // 2. Provider Instances List
        .child(
            div()
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .text_size(Typography::GROUP_HEADER_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t2)
                        .child("CONFIGURED PROVIDER INSTANCES"),
                )
                .children(state.settings.providers.iter().enumerate().map(|(idx, inst)| {
                    let instance_id = inst.id.clone();
                    let health_instance_id = instance_id.clone();
                    let environment_id = inst
                        .environment_id
                        .clone()
                        .unwrap_or_else(|| EnvironmentId::new("env-local"));
                    let health_report = state
                        .health_reports
                        .iter()
                        .find(|report| report.instance_id == instance_id);
                    let (status_text, status_color, bg_pill) =
                        match health_report.map(|report| report.status.as_str()) {
                            Some("healthy") => ("● Healthy", rgb(0x7fd88f), rgba(0x7fd88f22)),
                            Some("unauthenticated") => {
                                ("▲ Unauthenticated", rgb(0xd8b45e), rgba(0xd8b45e22))
                            }
                            Some("degraded") => {
                                ("▲ Degraded", rgb(0xd8b45e), rgba(0xd8b45e22))
                            }
                            Some("unavailable") => {
                                ("✕ Unavailable", rgb(0xf87171), rgba(0xf8717122))
                            }
                            Some("disabled") => {
                                ("○ Disabled", rgb(0x888899), rgba(0x88889922))
                            }
                            Some("configured") => {
                                ("○ Configured", rgb(0x94a3b8), rgba(0x94a3b822))
                            }
                            Some(_) => ("? Unknown", rgb(0x94a3b8), rgba(0x94a3b822)),
                            None => ("○ Not checked", rgb(0x888899), rgba(0x88889922)),
                        };
                    let version = health_report
                        .and_then(|report| report.version.as_deref())
                        .unwrap_or("Not reported");
                    let reason = health_report
                        .and_then(|report| report.reason.as_deref())
                        .unwrap_or("No reason reported");
                    let authentication = health_report
                        .map(|report| {
                            format!(
                                "{} ({})",
                                report.authentication_state, report.authentication_confidence
                            )
                        })
                        .unwrap_or_else(|| "Unknown (not reported)".to_string());
                    let freshness = health_report.map_or_else(
                        || "Not reported".to_string(),
                        |report| {
                            if now_ms <= report.expires_at_ms {
                                format!("Fresh · checked {}", format_age(now_ms, Some(report.checked_at_ms)))
                            } else {
                                format!("Stale · checked {}", format_age(now_ms, Some(report.checked_at_ms)))
                            }
                        },
                    );
                    let last_success = health_report
                        .and_then(|report| report.last_success_at_ms)
                        .map(|timestamp| format_age(now_ms, Some(timestamp)))
                        .unwrap_or_else(|| "Not reported".to_string());
                    let guidance = health_report.and_then(|report| report.login_guidance.as_ref());
                    let login_available = guidance.is_some_and(|guidance| guidance.available);
                    let login_tooltip = guidance.map_or_else(
                        || "Login guidance is not reported".to_string(),
                        |guidance| {
                            if guidance.available {
                                guidance.summary.clone()
                            } else {
                                guidance
                                    .unavailable_reason
                                    .clone()
                                    .unwrap_or_else(|| guidance.summary.clone())
                            }
                        },
                    );
                    let is_enabled = inst.enabled;

                    div()
                        .p_3()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel)
                        .border_1()
                        .border_color(if is_enabled { rgba(0xffffff0d) } else { rgba(0xffffff05) })
                        .v_flex()
                        .gap_2()
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .v_flex()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(Typography::TITLE_SIZE)
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.chrome.text_t1)
                                                .child(inst.display_name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(Typography::META_SIZE)
                                                .text_color(theme.chrome.text_t3)
                                                .child(format!(
                                                    "{} provider · version {version}",
                                                    inst.provider.as_str()
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .px_2()
                                                .py_0p5()
                                                .rounded(Radii::CHIP)
                                                .bg(bg_pill)
                                                .text_size(Typography::META_SIZE)
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(status_color)
                                                .child(status_text),
                                        )
                                        .child(
                                            Button::new(format!("toggle-enable-{}", inst.id.as_str()))
                                                .ghost()
                                                .label(if is_enabled { "Enabled" } else { "Disabled" })
                                                .accessibility_label(format!(
                                                    "{} {}",
                                                    if is_enabled { "Disable" } else { "Enable" },
                                                    inst.display_name
                                                ))
                                                .on_click(cx.listener(move |this, _event, window, cx| {
                                                    on_modify(this, Box::new(move |settings| {
                                                        if let Some(provider) = settings.providers.get_mut(idx) {
                                                            provider.enabled = !provider.enabled;
                                                        }
                                                    }), window, cx);
                                                })),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .v_flex()
                                .gap_1()
                                .text_size(Typography::SECONDARY_SIZE)
                                .text_color(theme.chrome.text_t2)
                                .child(format!("Authentication: {authentication}"))
                                .child(format!("Freshness: {freshness}"))
                                .child(format!("Last successful snapshot: {last_success}"))
                                .child(format!("Reason: {reason}"))
                                .when_some(health_report.and_then(|report| report.account_badge.as_ref()), |this, badge| {
                                    this.child(format!("Account: {badge}"))
                                }),
                        )
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .h_flex()
                                        .gap_2()
                                        .child(
                                            Button::new(format!("retry-health-{}", inst.id.as_str()))
                                                .ghost()
                                                .label("Retry health")
                                                .disabled(state.is_checking_health)
                                                .accessibility_label(format!("Retry health check for {}", inst.display_name))
                                                .on_click(cx.listener(move |this, _event, window, cx| {
                                                    on_check_health(
                                                        this,
                                                        Some(health_instance_id.clone()),
                                                        window,
                                                        cx,
                                                    );
                                                })),
                                        )
                                        .child(
                                            Button::new(format!("login-provider-{}", inst.id.as_str()))
                                                .ghost()
                                                .label("Prepare login")
                                                .disabled(!login_available || state.is_login_busy)
                                                .accessibility_label(format!("Prepare login for {}", inst.display_name))
                                                .tooltip(login_tooltip)
                                                .on_click(cx.listener(move |this, _event, window, cx| {
                                                    on_preview_login(
                                                        this,
                                                        instance_id.clone(),
                                                        environment_id.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                })),
                                        ),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(format!("Concurrency: {}", inst.concurrency_limit))
                                        .child(
                                            Button::new(format!("inc-conc-{}", inst.id.as_str()))
                                                .ghost()
                                                .label("Increase")
                                                .accessibility_label(format!("Increase concurrency for {}", inst.display_name))
                                                .on_click(cx.listener(move |this, _event, window, cx| {
                                                    on_modify(this, Box::new(move |settings| {
                                                        if let Some(provider) = settings.providers.get_mut(idx) {
                                                            provider.concurrency_limit = (provider.concurrency_limit + 1).min(8);
                                                        }
                                                    }), window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new(format!("dec-conc-{}", inst.id.as_str()))
                                                .ghost()
                                                .label("Decrease")
                                                .accessibility_label(format!("Decrease concurrency for {}", inst.display_name))
                                                .on_click(cx.listener(move |this, _event, window, cx| {
                                                    on_modify(this, Box::new(move |settings| {
                                                        if let Some(provider) = settings.providers.get_mut(idx) {
                                                            provider.concurrency_limit = (provider.concurrency_limit.saturating_sub(1)).max(1);
                                                        }
                                                    }), window, cx);
                                                })),
                                        ),
                                ),
                        )
                })),
        )
        .when_some(state.login_preview.as_ref(), |this, preview| {
            this.child(
                div()
                    .p_3()
                    .v_flex()
                    .gap_2()
                    .rounded(Radii::ROW)
                    .bg(theme.chrome.panel2)
                    .border_1()
                    .border_color(theme.accent.color())
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child("Confirm provider login"),
                    )
                    .child(format!("Profile: {}", preview.profile))
                    .child(format!("Command: {}", preview.display_command))
                    .child("Opening the terminal is the first side effect. Health checks never perform this action.")
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("confirm-provider-login")
                                    .primary()
                                    .label("Open login terminal")
                                    .disabled(state.is_login_busy)
                                    .on_click(cx.listener(move |this, _event, window, cx| {
                                        on_launch_login(this, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("cancel-provider-login")
                                    .ghost()
                                    .label("Cancel")
                                    .on_click(cx.listener(move |this, _event, window, cx| {
                                        on_cancel_login(this, window, cx);
                                    })),
                            ),
                    ),
            )
        })
        // 3. Tier Mapping Card
        .child(
            div()
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .text_size(Typography::GROUP_HEADER_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t2)
                        .child("CAPABILITY TIER MAPPING"),
                )
                .child(
                    div()
                        .p_3()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel)
                        .border_1()
                        .border_color(rgba(0xffffff0d))
                        .v_flex()
                        .gap_3()
                        .child(
                            div()
                                .text_size(Typography::SECONDARY_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child("Maps abstract tiers (fast, smart, reasoning, orchestrator) to concrete models and provider profiles for automated agent routing."),
                        )
                        .children(
                            state.settings.tier_mapping.as_ref().map(|tm| {
                                tm.tiers.iter().map(|(tier, target)| {
                                    let tier_name = tier.clone();
                                    let instance_name = state
                                        .settings
                                        .providers
                                        .iter()
                                        .find(|provider| provider.id == target.provider_instance_id)
                                        .map(|provider| provider.display_name.clone())
                                        .unwrap_or_else(|| "Missing named instance".to_string());
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .justify_between()
                                        .py_1p5()
                                        .px_2p5()
                                        .rounded(Radii::CHIP)
                                        .bg(theme.chrome.panel2)
                                        .child(
                                            div()
                                                .v_flex()
                                                .child(
                                                    div()
                                                        .text_size(Typography::BODY_SIZE)
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(theme.accent_color())
                                                        .child(tier_name.to_uppercase()),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(Typography::META_SIZE)
                                                        .text_color(theme.chrome.text_t3)
                                                        .child(format!("Instance: {instance_name}")),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .h_flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .px_2()
                                                        .py_0p5()
                                                        .rounded(Radii::CHIP)
                                                        .bg(theme.chrome.panel)
                                                        .border_1()
                                                        .border_color(rgba(0xffffff0a))
                                                        .text_size(Typography::SECONDARY_SIZE)
                                                        .font_family(Typography::MONO_FAMILY)
                                                        .text_color(theme.chrome.text_t1)
                                                        .child(target.model.clone()),
                                                )
                                                .when_some(target.effort.as_ref(), |this, eff| {
                                                    this.child(
                                                        div()
                                                            .px_2()
                                                            .py_0p5()
                                                            .rounded(Radii::CHIP)
                                                            .bg(rgba(0xffffff08))
                                                            .text_size(Typography::META_SIZE)
                                                            .text_color(theme.chrome.text_t2)
                                                            .child(format!("Effort: {eff}")),
                                                    )
                                                }),
                                        )
                                }).collect::<Vec<_>>()
                            }).unwrap_or_default()
                        ),
                ),
        )
        .into_any_element()
}

fn render_organizations_tab<V: 'static>(
    state: &SettingsViewState,
    catalog: &UsageCatalogResult,
    theme: &Theme,
    on_modify: impl Fn(
        &mut V,
        Box<dyn FnOnce(&mut UserSettings) + Send + 'static>,
        &mut Window,
        &mut Context<V>,
    )
    + 'static
    + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    div()
        .v_flex()
        .gap_3()
        .when(state.settings.organizations.is_empty(), |this| {
            this.child(
                div()
                    .p_3()
                    .v_flex()
                    .gap_2()
                    .rounded(Radii::ROW)
                    .bg(theme.chrome.panel)
                    .border_1()
                    .border_color(rgba(0xffffff0d))
                    .child("No canonical organization policy is configured.")
                    .child(
                        div()
                            .text_size(Typography::SECONDARY_SIZE)
                            .text_color(theme.chrome.text_t3)
                            .child("Project-scoped routing, schedules, MCP proposals, failover, and Broadside sharing fail closed until a project is assigned by a policy-capable workflow."),
                    )
                    .child(
                        Button::new("create-organization-unavailable")
                            .ghost()
                            .label("Create organization")
                            .disabled(true)
                            .tooltip("Unavailable: the current project catalog does not expose a safe assignment editor"),
                    ),
            )
        })
        .children(
            state
                .settings
                .organizations
                .iter()
                .enumerate()
                .map(|(organization_index, organization)| {
                    let provider_count = organization.allowed_provider_instances.len();
                    let valid_provider_count = organization
                        .allowed_provider_instances
                        .iter()
                        .filter(|provider_id| {
                            let Some(provider) = state
                                .settings
                                .providers
                                .iter()
                                .find(|provider| &provider.id == *provider_id)
                            else {
                                return false;
                            };
                            if !provider.enabled {
                                return false;
                            }
                            let environment_id = provider
                                .environment_id
                                .clone()
                                .unwrap_or_else(|| EnvironmentId::new("env-local"));
                            catalog.mappings.iter().any(|mapping| {
                                mapping.provider_instance_id == provider.id
                                    && mapping.environment_id == environment_id
                                    && organization
                                        .allowed_subscriptions
                                        .contains(&mapping.subscription_key)
                                    && catalog.subscriptions.iter().any(|subscription| {
                                        subscription.key == mapping.subscription_key
                                    })
                            })
                        })
                        .count();
                    div()
                        .p_3()
                        .v_flex()
                        .gap_3()
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
                                        .v_flex()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(Typography::TITLE_SIZE)
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.chrome.text_t1)
                                                .child(organization.display_name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(Typography::META_SIZE)
                                                .text_color(theme.chrome.text_t3)
                                                .child(format!(
                                                    "{} project assignment{} · revision {}",
                                                    organization.project_ids.len(),
                                                    if organization.project_ids.len() == 1 { "" } else { "s" },
                                                    organization.revision
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::SECONDARY_SIZE)
                                        .text_color(theme.chrome.text_t2)
                                        .child(format!(
                                            "Valid named routes: {valid_provider_count}/{provider_count}"
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .v_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(Typography::GROUP_HEADER_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t2)
                                        .child("PROVIDER / SUBSCRIPTION MATRIX"),
                                )
                                .children(
                                    organization
                                        .allowed_provider_instances
                                        .iter()
                                        .map(|provider_id| {
                                            let provider = state
                                                .settings
                                                .providers
                                                .iter()
                                                .find(|provider| &provider.id == provider_id);
                                            let provider_environment_id = provider
                                                .and_then(|provider| provider.environment_id.clone())
                                                .unwrap_or_else(|| EnvironmentId::new("env-local"));
                                            let mapping = catalog
                                                .mappings
                                                .iter()
                                                .find(|mapping| {
                                                    &mapping.provider_instance_id == provider_id
                                                        && mapping.environment_id
                                                            == provider_environment_id
                                                });
                                            let subscription = mapping.and_then(|mapping| {
                                                catalog.subscriptions.iter().find(|subscription| {
                                                    subscription.key == mapping.subscription_key
                                                })
                                            });
                                            let provider_is_valid = provider.is_some_and(|provider| provider.enabled)
                                                && mapping.is_some_and(|mapping| {
                                                    organization
                                                        .allowed_subscriptions
                                                        .contains(&mapping.subscription_key)
                                                })
                                                && subscription.is_some();
                                            let removal_blocked = provider_count <= 1
                                                || (provider_is_valid
                                                    && valid_provider_count <= 1);
                                            let provider_name = provider
                                                .map(|provider| provider.display_name.clone())
                                                .unwrap_or_else(|| "Missing provider instance".to_string());
                                            let subscription_name = subscription
                                                .map(|subscription| subscription.display_name.clone())
                                                .unwrap_or_else(|| "No named subscription".to_string());
                                            let provider_id = provider_id.clone();
                                            let subscription_key = mapping
                                                .map(|mapping| mapping.subscription_key.clone());
                                            let subscription_used_by_another_provider =
                                                subscription_key.as_ref().is_some_and(|key| {
                                                    organization
                                                        .allowed_provider_instances
                                                        .iter()
                                                        .filter(|candidate| {
                                                            candidate.as_str()
                                                                != provider_id.as_str()
                                                        })
                                                        .any(|candidate| {
                                                            catalog.mappings.iter().any(|mapping| {
                                                                mapping.provider_instance_id
                                                                    == *candidate
                                                                    && mapping.subscription_key
                                                                        == *key
                                                            })
                                                        })
                                                });
                                            div()
                                                .h_flex()
                                                .items_center()
                                                .justify_between()
                                                .p_2()
                                                .rounded(Radii::CHIP)
                                                .bg(theme.chrome.panel2)
                                                .child(
                                                    div()
                                                        .v_flex()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .font_weight(FontWeight::BOLD)
                                                                .child(provider_name.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(Typography::META_SIZE)
                                                                .text_color(theme.chrome.text_t3)
                                                                .child(format!("Subscription: {subscription_name}")),
                                                        ),
                                                )
                                                .child(
                                                    Button::new(format!(
                                                        "remove-org-provider-{organization_index}-{}",
                                                        provider_id.as_str()
                                                    ))
                                                    .ghost()
                                                    .label("Remove")
                                                    .disabled(removal_blocked)
                                                    .tooltip(if removal_blocked {
                                                        "Blocked: an organization must keep at least one valid named provider"
                                                    } else {
                                                        "Removal is checked against enabled schedules when saved"
                                                    })
                                                    .accessibility_label(format!(
                                                        "Remove {provider_name} from {}",
                                                        organization.display_name
                                                    ))
                                                    .on_click(cx.listener(move |this, _event, window, cx| {
                                                        let provider_id = provider_id.clone();
                                                        let subscription_key = subscription_key.clone();
                                                        let subscription_used_by_another_provider =
                                                            subscription_used_by_another_provider;
                                                        on_modify(
                                                            this,
                                                            Box::new(move |settings| {
                                                                if let Some(policy) = settings.organizations.get_mut(organization_index) {
                                                                    policy.allowed_provider_instances.retain(|candidate| candidate != &provider_id);
                                                                    if let Some(subscription_key) = subscription_key
                                                                        && !subscription_used_by_another_provider
                                                                        && policy.allowed_subscriptions.len() > 1
                                                                    {
                                                                        policy.allowed_subscriptions.retain(|candidate| candidate != &subscription_key);
                                                                    }
                                                                    policy.revision = policy.revision.saturating_add(1);
                                                                }
                                                            }),
                                                            window,
                                                            cx,
                                                        );
                                                    })),
                                                )
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .v_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(Typography::GROUP_HEADER_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t2)
                                        .child("ADD NAMED INSTANCE"),
                                )
                                .children(state.settings.providers.iter().filter_map(|provider| {
                                    if organization.allowed_provider_instances.contains(&provider.id) {
                                        return None;
                                    }
                                    let provider_environment_id = provider
                                        .environment_id
                                        .clone()
                                        .unwrap_or_else(|| EnvironmentId::new("env-local"));
                                    let mapping = catalog
                                        .mappings
                                        .iter()
                                        .find(|mapping| {
                                            mapping.provider_instance_id == provider.id
                                                && mapping.environment_id
                                                    == provider_environment_id
                                        });
                                    let subscription = mapping.and_then(|mapping| {
                                        catalog.subscriptions.iter().find(|subscription| {
                                            subscription.key == mapping.subscription_key
                                        })
                                    });
                                    let provider_id = provider.id.clone();
                                    let subscription_key = mapping.map(|mapping| mapping.subscription_key.clone());
                                    let provider_name = provider.display_name.clone();
                                    Some(
                                        Button::new(format!(
                                            "add-org-provider-{organization_index}-{}",
                                            provider.id.as_str()
                                        ))
                                        .ghost()
                                        .label(format!("Add {provider_name}"))
                                        .disabled(subscription.is_none() || !provider.enabled)
                                        .tooltip(if !provider.enabled {
                                            "Enable this named provider before adding it"
                                        } else if subscription.is_some() {
                                            "Add the named instance and its subscription boundary"
                                        } else {
                                            "Name this provider's subscription in Usage before adding it"
                                        })
                                        .on_click(cx.listener(move |this, _event, window, cx| {
                                            let provider_id = provider_id.clone();
                                            let subscription_key = subscription_key.clone();
                                            on_modify(
                                                this,
                                                Box::new(move |settings| {
                                                    if let (Some(policy), Some(subscription_key)) = (
                                                        settings.organizations.get_mut(organization_index),
                                                        subscription_key,
                                                    ) {
                                                        policy.allowed_provider_instances.push(provider_id);
                                                        if !policy.allowed_subscriptions.contains(&subscription_key) {
                                                            policy.allowed_subscriptions.push(subscription_key);
                                                        }
                                                        policy.revision = policy.revision.saturating_add(1);
                                                    }
                                                }),
                                                window,
                                                cx,
                                            );
                                        })),
                                    )
                                })),
                        )
                        .child(
                            div()
                                .v_flex()
                                .gap_2()
                                .child({
                                    let current = organization.daily_budget_micro_usd;
                                    let organization_name = organization.display_name.clone();
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .justify_between()
                                        .p_2()
                                        .rounded(Radii::CHIP)
                                        .bg(theme.chrome.panel2)
                                        .child(
                                            div()
                                                .v_flex()
                                                .child(
                                                    div()
                                                        .font_weight(FontWeight::BOLD)
                                                        .child("Daily API-equivalent budget"),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(Typography::META_SIZE)
                                                        .text_color(theme.chrome.text_t3)
                                                        .child(current.map_or_else(
                                                            || "Not set".to_string(),
                                                            |value| format_micro_usd(Some(value)),
                                                        )),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .h_flex()
                                                .gap_2()
                                                .child(
                                                    Button::new(format!(
                                                        "decrease-org-budget-{organization_index}"
                                                    ))
                                                    .ghost()
                                                    .label("Decrease $10")
                                                    .disabled(current.is_none())
                                                    .accessibility_label(format!(
                                                        "Decrease daily budget for {organization_name} by 10 dollars"
                                                    ))
                                                    .on_click(cx.listener(
                                                        move |this, _event, window, cx| {
                                                            on_modify(
                                                                this,
                                                                Box::new(move |settings| {
                                                                    if let Some(policy) = settings
                                                                        .organizations
                                                                        .get_mut(organization_index)
                                                                    {
                                                                        policy.daily_budget_micro_usd =
                                                                            decrease_organization_budget(
                                                                                current,
                                                                            );
                                                                        policy.revision = policy
                                                                            .revision
                                                                            .saturating_add(1);
                                                                    }
                                                                }),
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                                )
                                                .child({
                                                    let organization_name =
                                                        organization.display_name.clone();
                                                    Button::new(format!(
                                                        "increase-org-budget-{organization_index}"
                                                    ))
                                                    .ghost()
                                                    .label("Increase $10")
                                                    .accessibility_label(format!(
                                                        "Increase daily budget for {organization_name} by 10 dollars"
                                                    ))
                                                    .on_click(cx.listener(
                                                        move |this, _event, window, cx| {
                                                            on_modify(
                                                                this,
                                                                Box::new(move |settings| {
                                                                    if let Some(policy) = settings
                                                                        .organizations
                                                                        .get_mut(organization_index)
                                                                    {
                                                                        policy.daily_budget_micro_usd =
                                                                            increase_organization_budget(
                                                                                current,
                                                                            );
                                                                        policy.revision = policy
                                                                            .revision
                                                                            .saturating_add(1);
                                                                    }
                                                                }),
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    ))
                                                }),
                                        )
                                })
                                .child({
                                    let current = organization.max_concurrency;
                                    let organization_name = organization.display_name.clone();
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .justify_between()
                                        .p_2()
                                        .rounded(Radii::CHIP)
                                        .bg(theme.chrome.panel2)
                                        .child(
                                            div()
                                                .v_flex()
                                                .child(
                                                    div()
                                                        .font_weight(FontWeight::BOLD)
                                                        .child("Concurrency ceiling"),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(Typography::META_SIZE)
                                                        .text_color(theme.chrome.text_t3)
                                                        .child(
                                                            current
                                                                .map(|value| value.to_string())
                                                                .unwrap_or_else(|| {
                                                                    "Not set".to_string()
                                                                }),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .h_flex()
                                                .gap_2()
                                                .child(
                                                    Button::new(format!(
                                                        "decrease-org-concurrency-{organization_index}"
                                                    ))
                                                    .ghost()
                                                    .label("Decrease")
                                                    .disabled(current.is_none())
                                                    .accessibility_label(format!(
                                                        "Decrease concurrency ceiling for {organization_name}"
                                                    ))
                                                    .on_click(cx.listener(
                                                        move |this, _event, window, cx| {
                                                            on_modify(
                                                                this,
                                                                Box::new(move |settings| {
                                                                    if let Some(policy) = settings
                                                                        .organizations
                                                                        .get_mut(organization_index)
                                                                    {
                                                                        policy.max_concurrency =
                                                                            decrease_organization_concurrency(
                                                                                current,
                                                                            );
                                                                        policy.revision = policy
                                                                            .revision
                                                                            .saturating_add(1);
                                                                    }
                                                                }),
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                                )
                                                .child({
                                                    let organization_name =
                                                        organization.display_name.clone();
                                                    Button::new(format!(
                                                        "increase-org-concurrency-{organization_index}"
                                                    ))
                                                    .ghost()
                                                    .label("Increase")
                                                    .accessibility_label(format!(
                                                        "Increase concurrency ceiling for {organization_name}"
                                                    ))
                                                    .on_click(cx.listener(
                                                        move |this, _event, window, cx| {
                                                            on_modify(
                                                                this,
                                                                Box::new(move |settings| {
                                                                    if let Some(policy) = settings
                                                                        .organizations
                                                                        .get_mut(organization_index)
                                                                    {
                                                                        policy.max_concurrency =
                                                                            increase_organization_concurrency(
                                                                                current,
                                                                            );
                                                                        policy.revision = policy
                                                                            .revision
                                                                            .saturating_add(1);
                                                                    }
                                                                }),
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    ))
                                                }),
                                        )
                                })
                                .child({
                                    let workspace_mapping = state.settings.tier_mapping.clone();
                                    let workspace_mapping_is_allowed = workspace_mapping
                                        .as_ref()
                                        .is_some_and(|mapping| {
                                            mapping.tiers.values().all(|target| {
                                                organization
                                                    .allowed_provider_instances
                                                    .contains(&target.provider_instance_id)
                                            })
                                        });
                                    let has_override = organization.default_tier_mapping.is_some();
                                    div()
                                        .v_flex()
                                        .gap_2()
                                        .p_2()
                                        .rounded(Radii::CHIP)
                                        .bg(theme.chrome.panel2)
                                        .child(
                                            div()
                                                .h_flex()
                                                .items_center()
                                                .justify_between()
                                                .child(
                                                    div()
                                                        .v_flex()
                                                        .child(
                                                            div()
                                                                .font_weight(FontWeight::BOLD)
                                                                .child("Default capability tiers"),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(Typography::META_SIZE)
                                                                .text_color(theme.chrome.text_t3)
                                                                .child(if has_override {
                                                                    "Organization override"
                                                                } else {
                                                                    "No organization override"
                                                                }),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .h_flex()
                                                        .gap_2()
                                                        .child({
                                                            let mapping = workspace_mapping.clone();
                                                            Button::new(format!(
                                                                "apply-org-tier-map-{organization_index}"
                                                            ))
                                                            .ghost()
                                                            .label("Use workspace mapping")
                                                            .disabled(!workspace_mapping_is_allowed)
                                                            .tooltip(if workspace_mapping_is_allowed {
                                                                "Copy the current workspace tier mapping into this organization"
                                                            } else {
                                                                "Unavailable: the workspace mapping names a provider outside this organization"
                                                            })
                                                            .on_click(cx.listener(
                                                                move |this, _event, window, cx| {
                                                                    let mapping = mapping.clone();
                                                                    on_modify(
                                                                        this,
                                                                        Box::new(move |settings| {
                                                                            if let Some(policy) = settings
                                                                                .organizations
                                                                                .get_mut(organization_index)
                                                                            {
                                                                                policy.default_tier_mapping =
                                                                                    mapping;
                                                                                policy.revision = policy
                                                                                    .revision
                                                                                    .saturating_add(1);
                                                                            }
                                                                        }),
                                                                        window,
                                                                        cx,
                                                                    );
                                                                },
                                                            ))
                                                        })
                                                        .child(
                                                            Button::new(format!(
                                                                "clear-org-tier-map-{organization_index}"
                                                            ))
                                                            .ghost()
                                                            .label("Clear override")
                                                            .disabled(!has_override)
                                                            .on_click(cx.listener(
                                                                move |this, _event, window, cx| {
                                                                    on_modify(
                                                                        this,
                                                                        Box::new(move |settings| {
                                                                            if let Some(policy) = settings
                                                                                .organizations
                                                                                .get_mut(organization_index)
                                                                            {
                                                                                policy.default_tier_mapping =
                                                                                    None;
                                                                                policy.revision = policy
                                                                                    .revision
                                                                                    .saturating_add(1);
                                                                            }
                                                                        }),
                                                                        window,
                                                                        cx,
                                                                    );
                                                                },
                                                            )),
                                                        ),
                                                ),
                                        )
                                        .children(
                                            organization
                                                .default_tier_mapping
                                                .iter()
                                                .flat_map(|mapping| mapping.tiers.iter())
                                                .map(|(tier, target)| {
                                                    let provider_name = state
                                                        .settings
                                                        .providers
                                                        .iter()
                                                        .find(|provider| {
                                                            provider.id
                                                                == target.provider_instance_id
                                                        })
                                                        .map(|provider| {
                                                            provider.display_name.clone()
                                                        })
                                                        .unwrap_or_else(|| {
                                                            "Missing named provider".to_string()
                                                        });
                                                    div()
                                                        .h_flex()
                                                        .items_center()
                                                        .justify_between()
                                                        .child(format!("{}: {provider_name}", tier))
                                                        .child(target.model.clone())
                                                }),
                                        )
                                })
                                .child(
                                    div()
                                        .h_flex()
                                        .flex_wrap()
                                        .gap_2()
                                        .child(
                                            Button::new(format!(
                                                "edit-org-projects-{organization_index}"
                                            ))
                                            .ghost()
                                            .label(format!(
                                                "Project assignments: {}",
                                                organization.project_ids.len()
                                            ))
                                            .disabled(true)
                                            .tooltip("Unavailable: the current project catalog does not expose a safe assignment editor"),
                                        )
                                        .child(
                                            Button::new(format!(
                                                "edit-org-failover-{organization_index}"
                                            ))
                                            .ghost()
                                            .label(format!(
                                                "Failover pools: {}",
                                                organization.failover_pool_ids.len()
                                            ))
                                            .disabled(true)
                                            .tooltip("Unavailable until the capability-aware Phase 4F pool editor is implemented"),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .v_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(Typography::GROUP_HEADER_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t2)
                                        .child("BROADSIDE SHARING"),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .flex_wrap()
                                        .gap_2()
                                        .child({
                                            let enabled = organization.broadside.allow_artifacts;
                                            Button::new(format!(
                                                "broadside-artifacts-{organization_index}"
                                            ))
                                            .ghost()
                                            .label(format!(
                                                "Artifacts: {}",
                                                if enabled { "Allowed" } else { "Blocked" }
                                            ))
                                            .accessibility_label(
                                                "Toggle Broadside artifact sharing",
                                            )
                                            .on_click(cx.listener(
                                                move |this, _event, window, cx| {
                                                    on_modify(
                                                        this,
                                                        Box::new(move |settings| {
                                                            if let Some(policy) = settings
                                                                .organizations
                                                                .get_mut(organization_index)
                                                            {
                                                                policy.broadside.allow_artifacts =
                                                                    !enabled;
                                                                policy.revision = policy
                                                                    .revision
                                                                    .saturating_add(1);
                                                            }
                                                        }),
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            ))
                                        })
                                        .child({
                                            let enabled =
                                                organization.broadside.allow_transcripts;
                                            Button::new(format!(
                                                "broadside-transcripts-{organization_index}"
                                            ))
                                            .ghost()
                                            .label(format!(
                                                "Transcripts: {}",
                                                if enabled { "Allowed" } else { "Blocked" }
                                            ))
                                            .accessibility_label(
                                                "Toggle Broadside transcript sharing",
                                            )
                                            .on_click(cx.listener(
                                                move |this, _event, window, cx| {
                                                    on_modify(
                                                        this,
                                                        Box::new(move |settings| {
                                                            if let Some(policy) = settings
                                                                .organizations
                                                                .get_mut(organization_index)
                                                            {
                                                                policy.broadside.allow_transcripts =
                                                                    !enabled;
                                                                policy.revision = policy
                                                                    .revision
                                                                    .saturating_add(1);
                                                            }
                                                        }),
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            ))
                                        }),
                                ),
                        )
                }),
        )
        .into_any_element()
}

/// Renders the Terminal settings panel.
fn render_terminal_tab<V: 'static>(
    state: &SettingsViewState,
    theme: &Theme,
    on_modify: impl Fn(
        &mut V,
        Box<dyn FnOnce(&mut UserSettings) + Send + 'static>,
        &mut Window,
        &mut Context<V>,
    )
    + 'static
    + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let term = &state.settings.terminal;
    let scrollback = term.scrollback_lines;
    let welcome = term.welcome_prompt_enabled;
    let paste_optin = term.right_click_paste_optin;
    let confirm_close = term.confirm_close_on_running;
    let shell = term.preferred_shell.as_deref().unwrap_or("System Default");

    div()
        .v_flex()
        .gap_4()
        // 1. Scrollback lines
        .child(
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .v_flex()
                                .child(
                                    div()
                                        .text_size(Typography::TITLE_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child("Scrollback Buffer Depth"),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::SECONDARY_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child("Lines of history retained per session (1,000 to 200,000 lines)."),
                                ),
                        )
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .font_family(Typography::MONO_FAMILY)
                                .text_color(theme.accent_color())
                                .child(format!("{} lines", format_number(scrollback))),
                        ),
                )
                // Presets
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .children([5_000, 10_000, 25_000, 50_000, 100_000].iter().map(|&preset| {
                            let is_current = scrollback == preset;
                            let btn = Button::new(format!("preset-{preset}"))
                                .label(format_number(preset));
                            let btn = if is_current { btn.primary() } else { btn.ghost() };
                            btn.on_click(cx.listener(move |this, _event, window, cx| {
                                on_modify(this, Box::new(move |s| {
                                    s.terminal.scrollback_lines = preset;
                                }), window, cx);
                            }))
                        })),
                ),
        )
        // 2. Default Shell
        .child(
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .v_flex()
                        .child(
                            div()
                                .text_size(Typography::TITLE_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t1)
                                .child("Default Shell"),
                        )
                        .child(
                            div()
                                .text_size(Typography::SECONDARY_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(format!("Active shell: {shell}")),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_1p5()
                        .children([("pwsh", "PowerShell 7"), ("bash", "Bash"), ("cmd", "Command Prompt")].iter().map(|&(sh, label)| {
                            Button::new(format!("shell-{sh}"))
                                .ghost()
                                .label(label)
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    let s_name = sh.to_string();
                                    on_modify(this, Box::new(move |s| {
                                        s.terminal.preferred_shell = Some(s_name);
                                    }), window, cx);
                                }))
                        })),
                ),
        )
        // 3. Interaction Toggles
        .child(
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .v_flex()
                .gap_3()
                // Toggle A: Welcome Prompt
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .v_flex()
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child("Tool Chooser Prompt in Fresh Terminals"),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::SECONDARY_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child("Displays quick tool launcher cards when opening an empty terminal session."),
                                ),
                        )
                        .child(
                            Button::new("toggle-welcome")
                                .ghost()
                                .label(if welcome { "Enabled" } else { "Disabled" })
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_modify(this, Box::new(move |s| {
                                        s.terminal.welcome_prompt_enabled = !s.terminal.welcome_prompt_enabled;
                                    }), window, cx);
                                })),
                        ),
                )
                // Toggle B: Right click paste
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .v_flex()
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child("Classic Right-Click Paste"),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::SECONDARY_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child("Right-click pastes clipboard contents immediately instead of opening the context menu."),
                                ),
                        )
                        .child(
                            Button::new("toggle-paste-optin")
                                .ghost()
                                .label(if paste_optin { "Enabled" } else { "Disabled" })
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_modify(this, Box::new(move |s| {
                                        s.terminal.right_click_paste_optin = !s.terminal.right_click_paste_optin;
                                    }), window, cx);
                                })),
                        ),
                )
                // Toggle C: Confirm Close
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .v_flex()
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child("Confirm Closing Running Shells"),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::SECONDARY_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child("Show a confirmation prompt when closing terminal tabs with running background jobs."),
                                ),
                        )
                        .child(
                            Button::new("toggle-confirm-close")
                                .ghost()
                                .label(if confirm_close { "Enabled" } else { "Disabled" })
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_modify(this, Box::new(move |s| {
                                        s.terminal.confirm_close_on_running = !s.terminal.confirm_close_on_running;
                                    }), window, cx);
                                })),
                        ),
                ),
        )
}

/// Renders the General & Appearance settings panel.
fn render_general_tab<V: 'static>(
    state: &SettingsViewState,
    theme: &Theme,
    on_modify: impl Fn(
        &mut V,
        Box<dyn FnOnce(&mut UserSettings) + Send + 'static>,
        &mut Window,
        &mut Context<V>,
    )
    + 'static
    + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let ui = &state.settings.ui;
    let current_theme = ui.theme.as_str();
    let current_accent = ui.accent.as_str();
    let show_statusbar = ui.show_status_bar;
    let sounds = ui.sound_effects_enabled;
    let notifications = ui.notifications_enabled;

    div()
        .v_flex()
        .gap_4()
        // 1. Chrome Theme
        .child(
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .text_size(Typography::TITLE_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t1)
                        .child("Chrome Theme Mode"),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_3()
                        .child(
                            Button::new("theme-dark")
                                .ghost()
                                .label(if current_theme == "dark" { "🌙 Dark Theme (Active)" } else { "🌙 Dark Theme" })
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_modify(this, Box::new(move |s| {
                                        s.ui.theme = "dark".to_string();
                                    }), window, cx);
                                })),
                        )
                        .child(
                            Button::new("theme-light")
                                .ghost()
                                .label(if current_theme == "light" { "☀️ Light Theme (Active)" } else { "☀️ Light Theme" })
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_modify(this, Box::new(move |s| {
                                        s.ui.theme = "light".to_string();
                                    }), window, cx);
                                })),
                        ),
                ),
        )
        // 2. Accent Color Palette
        .child(
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .text_size(Typography::TITLE_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t1)
                        .child("Accent Color Tint"),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .children([
                            ("teal", "Teal", AccentColor::Teal.hex()),
                            ("gold", "Gold", AccentColor::Gold.hex()),
                            ("blue", "Blue", AccentColor::Blue.hex()),
                            ("purple", "Purple", AccentColor::Purple.hex()),
                        ].iter().map(|&(acc_id, label, hex)| {
                            let is_selected = current_accent == acc_id;
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1p5()
                                .px_3()
                                .py_1p5()
                                .rounded(Radii::CHIP)
                                .bg(if is_selected { rgba(0xffffff1a) } else { theme.chrome.panel2 })
                                .border_1()
                                .border_color(if is_selected { rgb(hex) } else { rgba(0xffffff0a) })
                                .child(
                                    div()
                                        .w_3()
                                        .h_3()
                                        .rounded_full()
                                        .bg(rgb(hex)),
                                )
                                .child(
                                    Button::new(format!("accent-{acc_id}"))
                                        .ghost()
                                        .label(label)
                                        .on_click(cx.listener(move |this, _event, window, cx| {
                                            let acc = acc_id.to_string();
                                            on_modify(this, Box::new(move |s| {
                                                s.ui.accent = acc;
                                            }), window, cx);
                                        })),
                                )
                        })),
                ),
        )
        // 3. UI Chrome Options
        .child(
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .v_flex()
                .gap_3()
                // Show Status Bar
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .v_flex()
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child("26px Status Bar"),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::SECONDARY_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child("Display bottom status bar showing server connection and thread metrics."),
                                ),
                        )
                        .child(
                            Button::new("toggle-status-bar")
                                .ghost()
                                .label(if show_statusbar { "Visible" } else { "Hidden" })
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_modify(this, Box::new(move |s| {
                                        s.ui.show_status_bar = !s.ui.show_status_bar;
                                    }), window, cx);
                                })),
                        ),
                )
                // Sound effects
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .v_flex()
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child("Turn Completion Audio"),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::SECONDARY_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child("Play audio chime when agent turn completes."),
                                ),
                        )
                        .child(
                            Button::new("toggle-sounds")
                                .ghost()
                                .label(if sounds { "Enabled" } else { "Disabled" })
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_modify(this, Box::new(move |s| {
                                        s.ui.sound_effects_enabled = !s.ui.sound_effects_enabled;
                                    }), window, cx);
                                })),
                        ),
                )
                // Notifications
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .v_flex()
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child("Desktop Notifications"),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::SECONDARY_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child("Display OS notification toasts when turns require interactive approval."),
                                ),
                        )
                        .child(
                            Button::new("toggle-notifications")
                                .ghost()
                                .label(if notifications { "Enabled" } else { "Disabled" })
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_modify(this, Box::new(move |s| {
                                        s.ui.notifications_enabled = !s.ui.notifications_enabled;
                                    }), window, cx);
                                })),
                        ),
                ),
        )
}

/// Renders the Advanced Engine & Diagnostics settings panel.
fn render_advanced_tab<V: 'static>(
    state: &SettingsViewState,
    theme: &Theme,
    server_status: String,
    on_modify: impl Fn(
        &mut V,
        Box<dyn FnOnce(&mut UserSettings) + Send + 'static>,
        &mut Window,
        &mut Context<V>,
    )
    + 'static
    + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let adv = &state.settings.advanced;
    let concurrency = adv.global_concurrency_limit;
    let max_checkpoints = adv.max_checkpoints_per_thread;

    div()
        .v_flex()
        .gap_4()
        // 1. Engine Boundaries
        .child(
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .v_flex()
                .gap_3()
                // Global Concurrency
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .v_flex()
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child("Global Sub-Process Concurrency Cap"),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::SECONDARY_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child("Maximum concurrent provider and tool processes allowed on this node."),
                                ),
                        )
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .font_family(Typography::MONO_FAMILY)
                                        .text_color(theme.accent_color())
                                        .child(format!("{concurrency} processes")),
                                )
                                .child(
                                    Button::new("inc-global-conc")
                                        .ghost()
                                        .label("▲")
                                        .accessibility_label("Increase global concurrency")
                                        .tooltip("Increase global concurrency")
                                        .on_click(cx.listener(move |this, _event, window, cx| {
                                            on_modify(this, Box::new(move |s| {
                                                s.advanced.global_concurrency_limit = (s.advanced.global_concurrency_limit + 1).min(16);
                                            }), window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("dec-global-conc")
                                        .ghost()
                                        .label("▼")
                                        .accessibility_label("Decrease global concurrency")
                                        .tooltip("Decrease global concurrency")
                                        .on_click(cx.listener(move |this, _event, window, cx| {
                                            on_modify(this, Box::new(move |s| {
                                                s.advanced.global_concurrency_limit = (s.advanced.global_concurrency_limit.saturating_sub(1)).max(1);
                                            }), window, cx);
                                        })),
                                ),
                        ),
                )
                // Max Checkpoints
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .v_flex()
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child("Checkpoint Retention Window"),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::SECONDARY_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child("Maximum hidden-ref Git checkpoints retained per thread before garbage collection."),
                                ),
                        )
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .font_family(Typography::MONO_FAMILY)
                                .text_color(theme.accent_color())
                                .child(format!("{max_checkpoints} checkpoints")),
                        ),
                ),
        )
        // 2. Attachment Size Caps
        .child(
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .text_size(Typography::TITLE_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t1)
                        .child("Attachment Size Limits"),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_3()
                        .child(
                            div()
                                .p_2()
                                .rounded(Radii::CHIP)
                                .bg(theme.chrome.panel2)
                                .v_flex()
                                .child("Image Cap: 10 MiB")
                                .child(div().text_size(Typography::META_SIZE).text_color(theme.chrome.text_t3).child("PNG, JPEG, GIF, WebP")),
                        )
                        .child(
                            div()
                                .p_2()
                                .rounded(Radii::CHIP)
                                .bg(theme.chrome.panel2)
                                .v_flex()
                                .child("File Cap: 25 MiB")
                                .child(div().text_size(Typography::META_SIZE).text_color(theme.chrome.text_t3).child("Text, Code, PDF, Zip")),
                        )
                        .child(
                            div()
                                .p_2()
                                .rounded(Radii::CHIP)
                                .bg(theme.chrome.panel2)
                                .v_flex()
                                .child("Turn Total Cap: 50 MiB")
                                .child(div().text_size(Typography::META_SIZE).text_color(theme.chrome.text_t3).child("Aggregated per send")),
                        ),
                ),
        )
        // 3. Diagnostics & Server Connection
        .child(
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(Typography::TITLE_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t1)
                                .child("Server Daemon Status"),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0x7fd88f22))
                                .text_size(Typography::META_SIZE)
                                .text_color(rgb(0x7fd88f))
                                .child(server_status.to_string()),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::SECONDARY_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child("Protocol Version 3 (JSON-RPC 2.0 over named pipe / Unix domain socket with monotonic event ordering)."),
                ),
        )
}

/// Renders the Environments settings panel for local and remote SSH nodes.
fn render_environments_tab<V: 'static>(
    state: &SettingsViewState,
    theme: &Theme,
    _cx: &mut Context<V>,
) -> impl IntoElement {
    div()
        .v_flex()
        .gap_4()
        .child(
            // Top Overview Card
            div()
                .p_4()
                .rounded(Radii::PANE)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .child(
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
                                        .text_size(Typography::TITLE_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.chrome.text_t1)
                                        .child("Execution Environments & Fleet Nodes"),
                                )
                                .child(
                                    Button::new("btn_import_ssh_config")
                                        .secondary()
                                        .label("Import from ~/.ssh/config"),
                                ),
                        )
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .text_color(theme.chrome.text_t2)
                                .child(
                                    "PandaMUX threads, agent tasks, worktrees, and scheduled jobs run either on your local machine or across remote SSH nodes. The daemon binary is bootstrapped automatically and verified with embedded SHA-256 manifests.",
                                ),
                        ),
                ),
        )
        .when(state.environments.len() <= 1, |this| {
            this.child(
                div()
                    .p_3()
                    .rounded(Radii::ROW)
                    .bg(rgba(0x4d9fff12))
                    .border_1()
                    .border_color(rgba(0x4d9fff25))
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(Typography::BODY_SIZE)
                            .text_color(rgb(0x4d9fff))
                            .child(
                                "💡 Tip: Import hosts configured in your ~/.ssh/config file into PandaMUX to enable remote execution with a single click.",
                            ),
                    )
                    .child(
                        Button::new("btn_import_ssh_config_banner")
                            .primary()
                            .label("Import SSH Hosts"),
                    ),
            )
        })
        .child(
            // Environments List
            div()
                .v_flex()
                .gap_3()
                .children(state.environments.iter().map(|env| {
                    let (status_text, status_icon, status_color) = match &env.status {
                        pandamux_core::EnvironmentStatus::Ready => ("Ready", "●", rgb(0x7fd88f)),
                        pandamux_core::EnvironmentStatus::Connected => ("Connected", "●", rgb(0x4d9fff)),
                        pandamux_core::EnvironmentStatus::Connecting => ("Connecting", "◐", rgb(0xd8b45e)),
                        pandamux_core::EnvironmentStatus::Bootstrapping => ("Bootstrapping", "▲", rgb(0x43d9c9)),
                        pandamux_core::EnvironmentStatus::Degraded => ("Degraded", "⚠", rgb(0xd8b45e)),
                        pandamux_core::EnvironmentStatus::Offline => ("Offline", "○", theme.chrome.text_t3),
                        pandamux_core::EnvironmentStatus::Disconnected => ("Disconnected", "○", theme.chrome.text_t3),
                        pandamux_core::EnvironmentStatus::Unreachable => ("Unreachable", "✕", rgb(0xef4444)),
                        pandamux_core::EnvironmentStatus::Error { .. } => ("Error", "✕", rgb(0xef4444)),
                    };

                    let version_label = env.server_version.as_deref().unwrap_or("v0.53.36");
                    let schedules_label = format!("{} schedules stored", env.schedules_count.unwrap_or(0));
                    let platform_label = env.platform.as_deref().unwrap_or("Linux x86_64");

                    div()
                        .p_3()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel)
                        .border_1()
                        .border_color(rgba(0xffffff0a))
                        .v_flex()
                        .gap_2()
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_size(Typography::BODY_SIZE)
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.chrome.text_t1)
                                                .child(env.display_name.clone()),
                                        )
                                        .child(
                                            div()
                                                .px_2()
                                                .py_0p5()
                                                .rounded(Radii::CHIP)
                                                .bg(rgba(0xffffff08))
                                                .border_1()
                                                .border_color(rgba(0xffffff08))
                                                .text_size(Typography::META_SIZE)
                                                .text_color(status_color)
                                                .child(format!("{status_icon} {status_text}")),
                                        ),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .px_2()
                                                .py_0p5()
                                                .rounded(Radii::CHIP)
                                                .bg(theme.chrome.panel2)
                                                .text_size(Typography::META_SIZE)
                                                .font_family(Typography::MONO_FAMILY)
                                                .text_color(theme.chrome.text_t2)
                                                .child(version_label.to_string()),
                                        )
                                        .child(
                                            div()
                                                .px_2()
                                                .py_0p5()
                                                .rounded(Radii::CHIP)
                                                .bg(theme.chrome.panel2)
                                                .text_size(Typography::META_SIZE)
                                                .text_color(theme.chrome.text_t3)
                                                .child(schedules_label),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .justify_between()
                                .text_size(Typography::SECONDARY_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(format!("Platform: {platform_label} · ID: {}", env.id.as_str()))
                                .child(
                                    div()
                                        .h_flex()
                                        .gap_2()
                                        .child(
                                            Button::new(format!("btn_bootstrap_{}", env.id.as_str()))
                                                .secondary()
                                                .label("Bootstrap & Connect"),
                                        )
                                        .child(
                                            Button::new(format!("btn_teardown_{}", env.id.as_str()))
                                                .ghost()
                                                .label("Teardown"),
                                        ),
                                ),
                        )
                        .when_some(env.last_error.as_ref(), |this, err| {
                            this.child(
                                div()
                                    .p_2()
                                    .rounded(Radii::CHIP)
                                    .bg(rgba(0xef444415))
                                    .border_1()
                                    .border_color(rgba(0xef444430))
                                    .text_size(Typography::SECONDARY_SIZE)
                                    .text_color(rgb(0xef4444))
                                    .child(format!("Last Error: {err}")),
                            )
                        })
                })),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn test_settings_environments_category() {
        let mut state = SettingsViewState::default();
        assert_eq!(state.active_category, SettingsCategory::Providers);
        assert_eq!(state.environments.len(), 1);
        assert_eq!(state.environments[0].display_name, "Local Machine");
        assert_eq!(
            state.environments[0].status,
            pandamux_core::EnvironmentStatus::Ready
        );

        state.set_category(SettingsCategory::Environments);
        assert_eq!(state.active_category, SettingsCategory::Environments);
        assert_eq!(state.active_category.icon(), "🖥️");
        assert_eq!(state.active_category.title(), "Remote Environments & Nodes");
    }

    #[test]
    fn test_settings_view_import_ssh_hosts() {
        let mut state = SettingsViewState::default();
        assert_eq!(state.environments.len(), 1);

        let config = "\
Host galahad
    HostName 10.55.88.48
    User chaz
";
        let imported = state.import_ssh_hosts(Some(config));
        assert_eq!(imported, 1);
        assert_eq!(state.environments.len(), 2);
        assert_eq!(state.environments[1].display_name, "galahad");
        assert!(state.status_banner.is_some());

        // Second import skips
        let reimported = state.import_ssh_hosts(Some(config));
        assert_eq!(reimported, 0);
        assert_eq!(state.environments.len(), 2);
    }

    #[test]
    fn organization_limit_controls_preserve_unset_and_positive_values() {
        assert_eq!(increase_organization_budget(None), Some(10_000_000));
        assert_eq!(
            increase_organization_budget(Some(10_000_000)),
            Some(20_000_000)
        );
        assert_eq!(
            decrease_organization_budget(Some(20_000_000)),
            Some(10_000_000)
        );
        assert_eq!(decrease_organization_budget(Some(10_000_000)), None);
        assert_eq!(decrease_organization_budget(None), None);

        assert_eq!(increase_organization_concurrency(None), Some(1));
        assert_eq!(increase_organization_concurrency(Some(1)), Some(2));
        assert_eq!(decrease_organization_concurrency(Some(2)), Some(1));
        assert_eq!(decrease_organization_concurrency(Some(1)), None);
        assert_eq!(decrease_organization_concurrency(None), None);
    }
}
