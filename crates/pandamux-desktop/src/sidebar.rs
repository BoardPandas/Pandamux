use std::collections::HashMap;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use pandamux_client::projections::ThreadProjection;
use pandamux_core::{ThreadId, ThreadStatus};

use crate::theme::{Radii, Spacing, Theme, Typography};

/// Active navigation rail tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RailTab {
    #[default]
    Threads,
    Agents,
    Terminal,
    Settings,
}

/// Agent item for the Agents roster navigation stub.
#[derive(Clone, Debug)]
pub struct AgentRosterItem {
    pub id: &'static str,
    pub name: &'static str,
    pub role: &'static str,
    pub tier: &'static str,
    pub access: &'static str,
    pub icon: &'static str,
}

pub const DEFAULT_AGENTS: &[AgentRosterItem] = &[
    AgentRosterItem {
        id: "orchestrator",
        name: "Orchestrator",
        role: "Intake, plan DAG, route tasks",
        tier: "Deep",
        access: "Ask",
        icon: "🧭",
    },
    AgentRosterItem {
        id: "architect",
        name: "System Architect",
        role: "Architecture & invariants",
        tier: "Deep",
        access: "ReadOnly",
        icon: "🏛️",
    },
    AgentRosterItem {
        id: "builder",
        name: "Code Builder",
        role: "Implementation & features",
        tier: "Balanced",
        access: "AutoEdit",
        icon: "🔨",
    },
    AgentRosterItem {
        id: "reviewer",
        name: "Code Reviewer",
        role: "Correctness & standards",
        tier: "Deep",
        access: "ReadOnly",
        icon: "🔍",
    },
    AgentRosterItem {
        id: "tester",
        name: "Test Engineer",
        role: "Fixtures & verification",
        tier: "Balanced",
        access: "AutoEdit",
        icon: "🧪",
    },
    AgentRosterItem {
        id: "security",
        name: "Security Auditor",
        role: "Vulnerabilities & OWASP",
        tier: "Deep",
        access: "ReadOnly",
        icon: "🛡️",
    },
    AgentRosterItem {
        id: "performance",
        name: "Performance Reviewer",
        role: "Bottlenecks & latency",
        tier: "Balanced",
        access: "ReadOnly",
        icon: "⚡",
    },
    AgentRosterItem {
        id: "ux-reviewer",
        name: "UX Reviewer",
        role: "Laws of UX & ergonomics",
        tier: "Balanced",
        access: "ReadOnly",
        icon: "🎨",
    },
];

/// Renders the 52px icon rail.
pub fn render_rail<V: 'static>(
    active_tab: RailTab,
    theme: &Theme,
    on_select_tab: impl Fn(&mut V, RailTab, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    div()
        .w(Spacing::RAIL_WIDTH)
        .h_full()
        .v_flex()
        .items_center()
        .py_2()
        .gap_3()
        .bg(theme.chrome.panel2)
        .border_r_1()
        .border_color(rgba(0xffffff0d))
        // Threads tab
        .child(
            div()
                .rounded(Radii::RAIL_BUTTON)
                .bg(if active_tab == RailTab::Threads {
                    rgba(0xffffff1a)
                } else {
                    rgba(0x00000000)
                })
                .child(
                    Button::new("rail-tab-threads")
                        .ghost()
                        .label("💬")
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_select_tab(this, RailTab::Threads, window, cx);
                        })),
                ),
        )
        // Agents tab
        .child(
            div()
                .rounded(Radii::RAIL_BUTTON)
                .bg(if active_tab == RailTab::Agents {
                    rgba(0xffffff1a)
                } else {
                    rgba(0x00000000)
                })
                .child(
                    Button::new("rail-tab-agents")
                        .ghost()
                        .label("🤖")
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_select_tab(this, RailTab::Agents, window, cx);
                        })),
                ),
        )
        // Terminal tab
        .child(
            div()
                .rounded(Radii::RAIL_BUTTON)
                .bg(if active_tab == RailTab::Terminal {
                    rgba(0xffffff1a)
                } else {
                    rgba(0x00000000)
                })
                .child(
                    Button::new("rail-tab-terminal")
                        .ghost()
                        .label("📟")
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_select_tab(this, RailTab::Terminal, window, cx);
                        })),
                ),
        )
        .child(div().flex_1())
        // Settings tab
        .child(
            div()
                .rounded(Radii::RAIL_BUTTON)
                .bg(if active_tab == RailTab::Settings {
                    rgba(0xffffff1a)
                } else {
                    rgba(0x00000000)
                })
                .child(
                    Button::new("rail-tab-settings")
                        .ghost()
                        .label("⚙️")
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_select_tab(this, RailTab::Settings, window, cx);
                        })),
                ),
        )
}

/// Renders the 264px sidebar corresponding to the active navigation rail tab.
pub fn render_sidebar<V: 'static>(
    active_tab: RailTab,
    thread_order: &[ThreadId],
    thread_projections: &HashMap<ThreadId, ThreadProjection>,
    active_thread_id: Option<&ThreadId>,
    theme: &Theme,
    on_new_thread: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_select_thread: impl Fn(&mut V, ThreadId, &mut Window, &mut Context<V>) + 'static + Copy,
    on_assign_agent: impl Fn(&mut V, &'static str, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    div()
        .w(Spacing::SIDEBAR_WIDTH)
        .h_full()
        .v_flex()
        .bg(theme.chrome.panel)
        .border_r_1()
        .border_color(rgba(0xffffff0d))
        .child(match active_tab {
            RailTab::Threads => render_threads_sidebar(
                thread_order,
                thread_projections,
                active_thread_id,
                theme,
                on_new_thread,
                on_select_thread,
                cx,
            )
            .into_any_element(),
            RailTab::Agents => render_agents_sidebar(theme, on_assign_agent, cx).into_any_element(),
            RailTab::Terminal => render_terminal_sidebar(theme).into_any_element(),
            RailTab::Settings => render_settings_sidebar(theme).into_any_element(),
        })
}

/// Renders the Threads navigation sidebar.
fn render_threads_sidebar<V: 'static>(
    thread_order: &[ThreadId],
    thread_projections: &HashMap<ThreadId, ThreadProjection>,
    active_thread_id: Option<&ThreadId>,
    theme: &Theme,
    on_new_thread: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_select_thread: impl Fn(&mut V, ThreadId, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let thread_count = thread_order.len();

    div()
        .size_full()
        .v_flex()
        // Header
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .p_3()
                .border_b_1()
                .border_color(rgba(0xffffff0a))
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(Typography::TITLE_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .child("Threads"),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff14))
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(format!("{thread_count}")),
                        ),
                )
                .child(
                    Button::new("btn-new-thread-sb")
                        .primary()
                        .label("+ New")
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            on_new_thread(this, window, cx);
                        })),
                ),
        )
        // Environment filter pill
        .child(
            div()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(rgba(0xffffff08))
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .rounded(Radii::CHIP)
                        .bg(theme.chrome.bgc_knockout)
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.accent.color())
                                .child("📍 Local Node"),
                        )
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child("env-local"),
                        ),
                ),
        )
        // Thread list
        .child(
            div()
                .flex_1()
                .v_flex()
                .p_2()
                .gap_1()
                .overflow_hidden()
                .children(thread_order.iter().filter_map(|id| {
                    let proj = thread_projections.get(id)?;
                    let is_active = Some(id) == active_thread_id;
                    let tid = id.clone();

                    let status_dot_color = match proj.thread.status {
                        ThreadStatus::Working => theme.accent.color(),
                        ThreadStatus::AwaitingApproval => theme.terminal.warn,
                        ThreadStatus::Idle => theme.terminal.success,
                        ThreadStatus::Errored => rgb(0xf87171),
                        ThreadStatus::Paused => theme.terminal.dim,
                        ThreadStatus::Archived => rgba(0xffffff20),
                    };

                    Some(
                        div()
                            .id(ElementId::NamedInteger("sb-thread-row".into(), tid.as_str().len() as u64))
                            .h_flex()
                            .items_center()
                            .justify_between()
                            .px_2p5()
                            .py_2()
                            .rounded(Radii::ROW)
                            .bg(if is_active {
                                rgba(0xffffff14)
                            } else {
                                rgba(0x00000000)
                            })
                            .border_1()
                            .border_color(if is_active {
                                theme.accent.color()
                            } else {
                                rgba(0x00000000)
                            })
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _event, window, cx| {
                                    on_select_thread(this, tid.clone(), window, cx);
                                }),
                            )
                            .child(
                                div()
                                    .v_flex()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_size(Typography::BODY_SIZE)
                                            .font_weight(if is_active {
                                                FontWeight::BOLD
                                            } else {
                                                FontWeight::NORMAL
                                            })
                                            .child(proj.thread.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .h_flex()
                                            .gap_1p5()
                                            .items_center()
                                            .child(
                                                div()
                                                    .w_1p5()
                                                    .h_1p5()
                                                    .rounded_full()
                                                    .bg(status_dot_color),
                                            )
                                            .child(
                                                div()
                                                    .text_size(Typography::META_SIZE)
                                                    .text_color(theme.chrome.text_t3)
                                                    .child(format!(
                                                        "{} · {} turns",
                                                        proj.thread.provider_instance_id.as_str(),
                                                        proj.turns.len()
                                                    )),
                                            ),
                                    ),
                            ),
                    )
                })),
        )
}

/// Renders the Agents navigation sidebar stub.
fn render_agents_sidebar<V: 'static>(
    theme: &Theme,
    on_assign_agent: impl Fn(&mut V, &'static str, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    div()
        .size_full()
        .v_flex()
        // Header
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .p_3()
                .border_b_1()
                .border_color(rgba(0xffffff0a))
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(Typography::TITLE_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .child("Agents"),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff14))
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(format!("{}", DEFAULT_AGENTS.len())),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.accent.color())
                        .child("Personal"),
                ),
        )
        // Agent list
        .child(
            div()
                .flex_1()
                .v_flex()
                .p_2()
                .gap_1p5()
                .overflow_hidden()
                .children(DEFAULT_AGENTS.iter().map(|agent| {
                    let agent_id = agent.id;
                    div()
                        .id(ElementId::NamedInteger("sb-agent".into(), agent.id.len() as u64))
                        .p_2()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel2)
                        .border_1()
                        .border_color(rgba(0xffffff0d))
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_1p5()
                                        .child(div().child(agent.icon))
                                        .child(
                                            div()
                                                .font_weight(FontWeight::BOLD)
                                                .text_size(Typography::BODY_SIZE)
                                                .child(agent.name),
                                        ),
                                )
                                .child(
                                    Button::new(format!("btn-assign-{}", agent.id))
                                        .ghost()
                                        .label("Assign")
                                        .on_click(cx.listener(move |this, _event, window, cx| {
                                            on_assign_agent(this, agent_id, window, cx);
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(agent.role),
                        )
                        .child(
                            div()
                                .h_flex()
                                .gap_1p5()
                                .child(
                                    div()
                                        .px_1()
                                        .py_0p5()
                                        .rounded(Radii::CHIP)
                                        .bg(rgba(0xffffff0d))
                                        .text_size(Typography::META_SIZE)
                                        .text_color(theme.accent.color())
                                        .child(agent.tier),
                                )
                                .child(
                                    div()
                                        .px_1()
                                        .py_0p5()
                                        .rounded(Radii::CHIP)
                                        .bg(rgba(0xffffff0d))
                                        .text_size(Typography::META_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child(agent.access),
                                ),
                        )
                })),
        )
}

/// Renders the Terminal multiplexer navigation sidebar stub.
fn render_terminal_sidebar(theme: &Theme) -> impl IntoElement {
    div()
        .size_full()
        .v_flex()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .p_3()
                .border_b_1()
                .border_color(rgba(0xffffff0a))
                .child(
                    div()
                        .text_size(Typography::TITLE_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .child("Terminals"),
                )
                .child(
                    Button::new("btn-new-term")
                        .primary()
                        .label("+ Shell"),
                ),
        )
        .child(
            div()
                .flex_1()
                .v_flex()
                .p_3()
                .gap_3()
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t3)
                        .child("ACTIVE LOCAL TERMINALS"),
                )
                .child(
                    div()
                        .p_2()
                        .rounded(Radii::CHIP)
                        .bg(theme.terminal.surface)
                        .border_1()
                        .border_color(rgba(0xffffff14))
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .text_color(theme.terminal.prompt)
                                .child("📟 Terminal 1: default shell"),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t3)
                        .child("SSH HOST PROFILES"),
                )
                .child(
                    div()
                        .p_2()
                        .rounded(Radii::CHIP)
                        .bg(theme.chrome.panel2)
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .child("🌐 galahad (192.168.1.100)"),
                        ),
                ),
        )
}

/// Renders the Settings navigation sidebar stub.
fn render_settings_sidebar(theme: &Theme) -> impl IntoElement {
    let sections = &[
        ("🔑", "Providers & API Keys"),
        ("🧭", "Orchestrator Policies"),
        ("🌐", "Environments & SSH"),
        ("⏰", "Schedules & Cron"),
        ("🎨", "Theme & Appearance"),
        ("⌨️", "Keybindings & Shortcuts"),
        ("ℹ️", "About PandaMUX"),
    ];

    div()
        .size_full()
        .v_flex()
        .child(
            div()
                .p_3()
                .border_b_1()
                .border_color(rgba(0xffffff0a))
                .child(
                    div()
                        .text_size(Typography::TITLE_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .child("Settings"),
                ),
        )
        .child(
            div()
                .flex_1()
                .v_flex()
                .p_2()
                .gap_1()
                .children(sections.iter().map(|(icon, title)| {
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px_2p5()
                        .py_2()
                        .rounded(Radii::CHIP)
                        .bg(theme.chrome.panel2)
                        .border_1()
                        .border_color(rgba(0xffffff08))
                        .child(div().child(*icon))
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .text_color(theme.chrome.text_t1)
                                .child(*title),
                        )
                })),
        )
}
