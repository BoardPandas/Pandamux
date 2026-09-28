use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_core::UserSettings;
use pandamux_core::provider_config::ProviderKind;
use pandamux_protocol::ProviderHealthReport;

use crate::theme::{AccentColor, Radii, Theme, Typography};

/// Top-level categories in the Settings view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsCategory {
    #[default]
    Providers,
    Environments,
    Terminal,
    General,
    Advanced,
}

impl SettingsCategory {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Providers => "Providers & Tier Mapping",
            Self::Environments => "Remote Environments & Nodes",
            Self::Terminal => "Terminal Configuration",
            Self::General => "General & Appearance",
            Self::Advanced => "Advanced Engine & Diagnostics",
        }
    }

    pub fn subtitle(&self) -> &'static str {
        match self {
            Self::Providers => {
                "Configure scoped provider instances, model targets, and run zero-spawn offline health checks."
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

/// Renders the complete Settings view based on active category.
pub fn render_settings_view<V: 'static>(
    state: &SettingsViewState,
    theme: &Theme,
    server_status: String,
    on_save: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_check_health: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
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
            SettingsCategory::Providers => {
                render_providers_tab(state, theme, on_check_health, on_modify, cx)
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
}

/// Renders the Settings category header and save actions.
fn render_settings_header<V: 'static>(
    state: &SettingsViewState,
    theme: &Theme,
    on_save: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_check_health: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
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
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                on_check_health(this, window, cx);
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
}

/// Renders the Providers & Tier Mapping settings panel.
fn render_providers_tab<V: 'static>(
    state: &SettingsViewState,
    theme: &Theme,
    _on_check_health: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
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
                                .child("Zero-Spawn Health Status (Rule 1)"),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0x43d9c922))
                                .text_size(Typography::META_SIZE)
                                .text_color(rgb(0x43d9c9))
                                .child("Never Triggers Auth"),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::SECONDARY_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child("Health checks inspect local disk manifests, CLI binary resolution, and profile credentials. Antigravity bundles are never spawned (saving ~1 GB PyInstaller unpack per check), and no interactive OAuth logins are triggered."),
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
                    let health_report = state.health_reports.iter().find(|r| r.instance_id == instance_id);
                    let (status_text, status_color, bg_pill) = match health_report.map(|r| r.status.as_str()) {
                        Some("healthy") => ("● Healthy", rgb(0x7fd88f), rgba(0x7fd88f22)),
                        Some("unauthenticated") => ("▲ Unauthenticated", rgb(0xd8b45e), rgba(0xd8b45e22)),
                        Some("degraded") => ("▲ Degraded", rgb(0xd8b45e), rgba(0xd8b45e22)),
                        Some("unavailable") => ("✕ Unavailable", rgb(0xf87171), rgba(0xf8717122)),
                        _ => ("○ Not Checked", rgb(0x888899), rgba(0x88889922)),
                    };

                    let badge = health_report.and_then(|r| r.account_badge.as_ref());
                    let is_enabled = inst.enabled;

                    div()
                        .p_3()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel)
                        .border_1()
                        .border_color(if is_enabled { rgba(0xffffff0d) } else { rgba(0xffffff05) })
                        .v_flex()
                        .gap_2()
                        // Row 1: Icon, Name, Provider Kind, Status Badge, Enabled Switch
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
                                                .text_size(Typography::TITLE_SIZE)
                                                .child(match inst.provider {
                                                    ProviderKind::Claude => "🟣",
                                                    ProviderKind::Codex => "🟢",
                                                    ProviderKind::Antigravity => "🔵",
                                                    ProviderKind::Cursor => "⚡",
                                                    ProviderKind::Grok => "🚀",
                                                    ProviderKind::OpenCode => "💻",
                                                    _ => "🤖",
                                                }),
                                        )
                                        .child(
                                            div()
                                                .text_size(Typography::TITLE_SIZE)
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.chrome.text_t1)
                                                .child(inst.display_name.clone()),
                                        )
                                        .child(
                                            div()
                                                .px_2()
                                                .py_0p5()
                                                .rounded(Radii::CHIP)
                                                .bg(theme.chrome.panel2)
                                                .text_size(Typography::META_SIZE)
                                                .text_color(theme.chrome.text_t3)
                                                .child(inst.id.as_str().to_string()),
                                        ),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_2()
                                        // Health pill
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
                                        // Account / subscription badge
                                        .when_some(badge, |this, b| {
                                            this.child(
                                                div()
                                                    .px_2()
                                                    .py_0p5()
                                                    .rounded(Radii::CHIP)
                                                    .bg(theme.chrome.panel2)
                                                    .text_size(Typography::META_SIZE)
                                                    .text_color(theme.chrome.text_t2)
                                                    .child(b.clone()),
                                            )
                                        })
                                        // Enable/Disable toggle
                                        .child(
                                            Button::new(format!("toggle-enable-{}", inst.id.as_str()))
                                                .ghost()
                                                .label(if is_enabled { "Enabled" } else { "Disabled" })
                                                .on_click(cx.listener(move |this, _event, window, cx| {
                                                    on_modify(this, Box::new(move |s| {
                                                        if let Some(p) = s.providers.get_mut(idx) {
                                                            p.enabled = !p.enabled;
                                                        }
                                                    }), window, cx);
                                                })),
                                        ),
                                ),
                        )
                        // Row 2: Profile directory info & Concurrency limit
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .justify_between()
                                .text_size(Typography::SECONDARY_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(
                                    div()
                                        .h_flex()
                                        .gap_2()
                                        .child("Profile Path:")
                                        .child(
                                            div()
                                                .text_color(theme.chrome.text_t2)
                                                .font_family(Typography::MONO_FAMILY)
                                                .child(inst.profile_dir.clone()),
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
                                                .label("▲")
                                                .on_click(cx.listener(move |this, _event, window, cx| {
                                                    on_modify(this, Box::new(move |s| {
                                                        if let Some(p) = s.providers.get_mut(idx) {
                                                            p.concurrency_limit = (p.concurrency_limit + 1).min(8);
                                                        }
                                                    }), window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new(format!("dec-conc-{}", inst.id.as_str()))
                                                .ghost()
                                                .label("▼")
                                                .on_click(cx.listener(move |this, _event, window, cx| {
                                                    on_modify(this, Box::new(move |s| {
                                                        if let Some(p) = s.providers.get_mut(idx) {
                                                            p.concurrency_limit = (p.concurrency_limit.saturating_sub(1)).max(1);
                                                        }
                                                    }), window, cx);
                                                })),
                                        ),
                                ),
                        )
                })),
        )
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
                                                        .child(format!("Instance: {}", target.provider_instance_id.as_str())),
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
}
