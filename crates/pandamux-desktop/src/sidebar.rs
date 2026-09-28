use std::collections::HashMap;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use pandamux_client::projections::ThreadProjection;
use pandamux_core::{ThreadId, ThreadOrigin, ThreadStatus, TurnStatus};

use crate::settings_view::SettingsCategory;
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
#[allow(clippy::too_many_arguments)]
pub fn render_sidebar<V: 'static>(
    active_tab: RailTab,
    thread_order: &[ThreadId],
    thread_projections: &HashMap<ThreadId, ThreadProjection>,
    active_thread_id: Option<&ThreadId>,
    theme: &Theme,
    on_new_thread: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_select_thread: impl Fn(&mut V, ThreadId, &mut Window, &mut Context<V>) + 'static + Copy,
    on_assign_agent: impl Fn(&mut V, &'static str, &mut Window, &mut Context<V>) + 'static + Copy,
    active_settings_category: SettingsCategory,
    on_select_settings_category: impl Fn(&mut V, SettingsCategory, &mut Window, &mut Context<V>)
    + 'static
    + Copy,
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
            RailTab::Settings => render_settings_sidebar(
                active_settings_category,
                theme,
                on_select_settings_category,
                cx,
            )
            .into_any_element(),
        })
}

/// Formats working duration for a thread currently in Working status.
/// Format follows "Working Nm" (for example: "Working 7m", "Working <1m", "Working 1h 5m").
pub fn format_working_duration(proj: &ThreadProjection, now_ms: u64) -> String {
    let start_ms = proj
        .turns
        .iter()
        .rev()
        .find(|t| t.status == TurnStatus::Running || t.status == TurnStatus::Requested)
        .map(|t| t.started_at_ms)
        .filter(|&ms| ms > 0)
        .or_else(|| {
            proj.turns
                .last()
                .map(|t| t.started_at_ms)
                .filter(|&ms| ms > 0)
        })
        .unwrap_or(proj.thread.updated_at_ms);

    let elapsed_ms = if now_ms > start_ms && start_ms > 0 {
        now_ms - start_ms
    } else {
        0
    };

    let total_mins = elapsed_ms / 60_000;
    if total_mins >= 60 {
        let hours = total_mins / 60;
        let mins = total_mins % 60;
        format!("Working {}h {}m", hours, mins)
    } else if total_mins > 0 {
        format!("Working {}m", total_mins)
    } else {
        "Working <1m".to_string()
    }
}

/// Convenience helper using system time.
pub fn working_duration_str(proj: &ThreadProjection) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    format_working_duration(proj, now_ms)
}

/// Represents a project or repository grouping of threads with settled counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectThreadGroup {
    pub name: String,
    pub thread_ids: Vec<ThreadId>,
    pub settled_count: usize,
    pub active_count: usize,
}

/// Groups thread IDs by project or workspace repository and computes settled counts.
pub fn group_threads_by_project(
    thread_order: &[ThreadId],
    thread_projections: &HashMap<ThreadId, ThreadProjection>,
) -> Vec<ProjectThreadGroup> {
    let mut groups: Vec<ProjectThreadGroup> = Vec::new();
    let mut group_map: HashMap<String, usize> = HashMap::new();

    for id in thread_order {
        if let Some(proj) = thread_projections.get(id) {
            let group_name = if let Some(ref pid) = proj.thread.project_id {
                pid.as_str().to_string()
            } else if let Some(ref wt) = proj.thread.workspace.worktree {
                if let Some(name) = std::path::Path::new(&wt.path)
                    .file_name()
                    .and_then(|n| n.to_str())
                {
                    name.to_string()
                } else {
                    wt.branch.clone()
                }
            } else if !proj.thread.workspace.cwd.is_empty() {
                if let Some(name) = std::path::Path::new(&proj.thread.workspace.cwd)
                    .file_name()
                    .and_then(|n| n.to_str())
                {
                    name.to_string()
                } else {
                    "Workspace".to_string()
                }
            } else {
                "General".to_string()
            };

            let is_settled = matches!(
                proj.thread.status,
                ThreadStatus::Idle | ThreadStatus::Archived
            );

            if let Some(&idx) = group_map.get(&group_name) {
                groups[idx].thread_ids.push(id.clone());
                if is_settled {
                    groups[idx].settled_count += 1;
                } else {
                    groups[idx].active_count += 1;
                }
            } else {
                let idx = groups.len();
                group_map.insert(group_name.clone(), idx);
                groups.push(ProjectThreadGroup {
                    name: group_name,
                    thread_ids: vec![id.clone()],
                    settled_count: if is_settled { 1 } else { 0 },
                    active_count: if is_settled { 0 } else { 1 },
                });
            }
        }
    }

    groups
}

fn render_status_marker(proj: &ThreadProjection, now_ms: u64, theme: &Theme) -> AnyElement {
    match proj.thread.status {
        ThreadStatus::Working => div()
            .h_flex()
            .items_center()
            .gap_1()
            .px_1p5()
            .py_0p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0x38bdf825))
            .text_size(Typography::META_SIZE)
            .font_weight(FontWeight::BOLD)
            .text_color(theme.accent.color())
            .child(div().child("⟳"))
            .child(div().child(format_working_duration(proj, now_ms)))
            .into_any_element(),
        ThreadStatus::AwaitingApproval => div()
            .px_1p5()
            .py_0p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0xfbbf2425))
            .text_size(Typography::META_SIZE)
            .font_weight(FontWeight::BOLD)
            .text_color(theme.terminal.warn)
            .child("⚠️ Approval")
            .into_any_element(),
        ThreadStatus::Paused => div()
            .px_1p5()
            .py_0p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0x94a3b825))
            .text_size(Typography::META_SIZE)
            .text_color(theme.terminal.dim)
            .child("⏸️ Paused")
            .into_any_element(),
        ThreadStatus::Errored => div()
            .px_1p5()
            .py_0p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0xf8717125))
            .text_size(Typography::META_SIZE)
            .text_color(rgb(0xf87171))
            .child("❌ Error")
            .into_any_element(),
        ThreadStatus::Idle => div()
            .px_1p5()
            .py_0p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0x22c55e18))
            .text_size(Typography::META_SIZE)
            .text_color(theme.terminal.success)
            .child(if !proj.turns.is_empty() {
                "✓ Settled"
            } else {
                "Idle"
            })
            .into_any_element(),
        ThreadStatus::Archived => div()
            .px_1p5()
            .py_0p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0xffffff10))
            .text_size(Typography::META_SIZE)
            .text_color(theme.chrome.text_t3)
            .child("Archived")
            .into_any_element(),
    }
}

fn render_agent_chip(agent_id: &str) -> AnyElement {
    div()
        .px_1p5()
        .py_0p5()
        .rounded(Radii::CHIP)
        .bg(rgba(0xa855f720))
        .text_size(Typography::META_SIZE)
        .text_color(rgb(0xc084fc))
        .child(format!("🤖 {agent_id}"))
        .into_any_element()
}

fn render_origin_chip(origin: &ThreadOrigin, theme: &Theme) -> Option<AnyElement> {
    match origin {
        ThreadOrigin::Orchestrator { run_id, .. } => Some(
            div()
                .px_1()
                .py_0p5()
                .rounded(Radii::CHIP)
                .bg(rgba(0xffffff0d))
                .text_size(Typography::META_SIZE)
                .text_color(theme.accent.color())
                .child(format!("🧭 {}", run_id.as_str()))
                .into_any_element(),
        ),
        ThreadOrigin::Schedule { schedule_id, .. } => Some(
            div()
                .px_1()
                .py_0p5()
                .rounded(Radii::CHIP)
                .bg(rgba(0xffffff0d))
                .text_size(Typography::META_SIZE)
                .text_color(theme.terminal.warn)
                .child(format!("⏰ {}", schedule_id.as_str()))
                .into_any_element(),
        ),
        ThreadOrigin::Manual => None,
    }
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
    let settled_count = thread_order
        .iter()
        .filter_map(|id| thread_projections.get(id))
        .filter(|proj| {
            matches!(
                proj.thread.status,
                ThreadStatus::Idle | ThreadStatus::Archived
            )
        })
        .count();

    let groups = group_threads_by_project(thread_order, thread_projections);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

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
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0x22c55e18))
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.terminal.success)
                                .child(format!("{settled_count} settled")),
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
        // Thread list grouped by project
        .child(
            div()
                .flex_1()
                .v_flex()
                .p_2()
                .gap_2()
                .overflow_hidden()
                .child(if groups.is_empty() {
                    div()
                        .p_4()
                        .v_flex()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(div().text_size(Typography::TITLE_SIZE).child("💬"))
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child("No threads yet. Click + New to begin."),
                        )
                        .into_any_element()
                } else {
                    let mut group_elements: Vec<AnyElement> = Vec::new();
                    let mut row_idx: u64 = 0;

                    for group in groups {
                        let mut thread_elements: Vec<AnyElement> = Vec::new();

                        for tid in &group.thread_ids {
                            let Some(proj) = thread_projections.get(tid) else {
                                continue;
                            };
                            let is_active = Some(tid) == active_thread_id;
                            let current_tid = tid.clone();
                            let current_idx = row_idx;
                            row_idx += 1;

                            let status_dot_color = match proj.thread.status {
                                ThreadStatus::Working => theme.accent.color(),
                                ThreadStatus::AwaitingApproval => theme.terminal.warn,
                                ThreadStatus::Idle => theme.terminal.success,
                                ThreadStatus::Errored => rgb(0xf87171),
                                ThreadStatus::Paused => theme.terminal.dim,
                                ThreadStatus::Archived => rgba(0xffffff20),
                            };

                            let status_marker = render_status_marker(proj, now_ms, theme);
                            let agent_chip =
                                proj.thread.agent.as_ref().map(|agent_ref| {
                                    render_agent_chip(agent_ref.agent_id.as_str())
                                });
                            let origin_chip = render_origin_chip(&proj.thread.origin, theme);

                            thread_elements.push(
                                div()
                                    .id(ElementId::NamedInteger(
                                        "sb-thread-row".into(),
                                        current_idx,
                                    ))
                                    .v_flex()
                                    .gap_1()
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
                                            on_select_thread(this, current_tid.clone(), window, cx);
                                        }),
                                    )
                                    .child(
                                        div()
                                            .h_flex()
                                            .items_center()
                                            .justify_between()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .text_size(Typography::BODY_SIZE)
                                                    .font_weight(if is_active {
                                                        FontWeight::BOLD
                                                    } else {
                                                        FontWeight::NORMAL
                                                    })
                                                    .overflow_hidden()
                                                    .child(proj.thread.title.clone()),
                                            )
                                            .child(
                                                div()
                                                    .h_flex()
                                                    .items_center()
                                                    .gap_1()
                                                    .children(agent_chip)
                                                    .child(status_marker),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .h_flex()
                                            .items_center()
                                            .justify_between()
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
                                                                proj.thread
                                                                    .provider_instance_id
                                                                    .as_str(),
                                                                proj.turns.len()
                                                            )),
                                                    ),
                                            )
                                            .children(origin_chip),
                                    )
                                    .into_any_element(),
                            );
                        }

                        let group_header = div()
                            .h_flex()
                            .items_center()
                            .justify_between()
                            .pt_2()
                            .pb_1()
                            .px_1p5()
                            .child(
                                div()
                                    .h_flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(div().text_size(Typography::META_SIZE).child("📁"))
                                    .child(
                                        div()
                                            .text_size(Typography::META_SIZE)
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.chrome.text_t2)
                                            .child(group.name.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(Radii::CHIP)
                                    .bg(rgba(0xffffff0d))
                                    .text_size(Typography::META_SIZE)
                                    .text_color(theme.chrome.text_t3)
                                    .child(if group.active_count > 0 {
                                        format!(
                                            "{} settled · {} active",
                                            group.settled_count, group.active_count
                                        )
                                    } else {
                                        format!("{} settled", group.settled_count)
                                    }),
                            );

                        group_elements.push(
                            div()
                                .v_flex()
                                .gap_1()
                                .child(group_header)
                                .children(thread_elements)
                                .into_any_element(),
                        );
                    }

                    div()
                        .v_flex()
                        .gap_2()
                        .children(group_elements)
                        .into_any_element()
                }),
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
                        .id(ElementId::NamedInteger(
                            "sb-agent".into(),
                            agent.id.len() as u64,
                        ))
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
                .child(Button::new("btn-new-term").primary().label("+ Shell")),
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

/// Renders the Settings navigation sidebar.
fn render_settings_sidebar<V: 'static>(
    active_category: SettingsCategory,
    theme: &Theme,
    on_select_category: impl Fn(&mut V, SettingsCategory, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let sections = [
        (SettingsCategory::Providers, "🔑", "Providers & Tiers"),
        (SettingsCategory::Environments, "🖥️", "Environments"),
        (SettingsCategory::Terminal, "📟", "Terminal"),
        (SettingsCategory::General, "🎨", "General & UI"),
        (SettingsCategory::Advanced, "⚙️", "Advanced Engine"),
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
                .children(sections.iter().map(|&(cat, icon, title)| {
                    let is_active = active_category == cat;
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px_2p5()
                        .py_2()
                        .rounded(Radii::CHIP)
                        .bg(if is_active {
                            rgba(0xffffff1a)
                        } else {
                            theme.chrome.panel2
                        })
                        .border_1()
                        .border_color(if is_active {
                            theme.accent_color()
                        } else {
                            rgba(0xffffff08)
                        })
                        .child(div().child(icon))
                        .child(
                            Button::new(format!("settings-tab-{}", cat.title()))
                                .ghost()
                                .label(title)
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_select_category(this, cat, window, cx);
                                })),
                        )
                })),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use pandamux_core::{
        AgentId, AgentInstanceRef, EnvironmentId, ProjectId, ProviderInstanceId, Thread,
        ThreadOrigin, ThreadWorkspace, Turn, TurnId, TurnInput, TurnStatus,
    };

    fn make_test_thread(
        id: &str,
        project_id: Option<&str>,
        status: ThreadStatus,
        agent: Option<&str>,
    ) -> Thread {
        Thread {
            id: ThreadId::new(id),
            project_id: project_id.map(ProjectId::new),
            environment_id: EnvironmentId::new("env-local"),
            parent_thread_id: None,
            title: format!("Thread {id}"),
            provider_instance_id: ProviderInstanceId::new("claude-main"),
            model: "claude-3-7-sonnet".to_string(),
            effort: None,
            access_mode: pandamux_core::AccessMode::Ask,
            workspace: ThreadWorkspace {
                cwd: "/test/project".to_string(),
                worktree: None,
            },
            status,
            agent: agent.map(|a| AgentInstanceRef {
                agent_id: AgentId::new(a),
                bundle_sha: "test-sha".to_string(),
            }),
            origin: ThreadOrigin::Manual,
            created_at_ms: 1000,
            updated_at_ms: 2000,
        }
    }

    #[test]
    fn test_format_working_duration_minutes() {
        let thread = make_test_thread("t1", Some("p1"), ThreadStatus::Working, None);
        let mut proj = ThreadProjection::new(thread);

        // Turn started 7 minutes ago
        let started_ms = 1_000_000;
        let now_ms = started_ms + 7 * 60 * 1000;
        proj.turns.push(Turn {
            id: TurnId::new("turn-1"),
            thread_id: ThreadId::new("t1"),
            seq: 1,
            input: TurnInput {
                text: "test".to_string(),
                ..Default::default()
            },
            status: TurnStatus::Running,
            started_at_ms: started_ms,
            ended_at_ms: None,
            usage: None,
            checkpoint_before: None,
            checkpoint_after: None,
        });

        assert_eq!(format_working_duration(&proj, now_ms), "Working 7m");

        // Turn started 11 minutes ago
        let now_ms = started_ms + 11 * 60 * 1000;
        assert_eq!(format_working_duration(&proj, now_ms), "Working 11m");

        // Turn started 65 minutes ago (1h 5m)
        let now_ms = started_ms + 65 * 60 * 1000;
        assert_eq!(format_working_duration(&proj, now_ms), "Working 1h 5m");

        // Turn started 20 seconds ago (<1m)
        let now_ms = started_ms + 20 * 1000;
        assert_eq!(format_working_duration(&proj, now_ms), "Working <1m");
    }

    #[test]
    fn test_group_threads_by_project_and_settled_counts() {
        let t1 = make_test_thread("t1", Some("pandamux"), ThreadStatus::Idle, None);
        let t2 = make_test_thread(
            "t2",
            Some("pandamux"),
            ThreadStatus::Working,
            Some("builder"),
        );
        let t3 = make_test_thread("t3", Some("pandamux"), ThreadStatus::Archived, None);
        let t4 = make_test_thread(
            "t4",
            Some("other-repo"),
            ThreadStatus::AwaitingApproval,
            None,
        );

        let mut projections = HashMap::new();
        projections.insert(ThreadId::new("t1"), ThreadProjection::new(t1));
        projections.insert(ThreadId::new("t2"), ThreadProjection::new(t2));
        projections.insert(ThreadId::new("t3"), ThreadProjection::new(t3));
        projections.insert(ThreadId::new("t4"), ThreadProjection::new(t4));

        let order = vec![
            ThreadId::new("t1"),
            ThreadId::new("t2"),
            ThreadId::new("t3"),
            ThreadId::new("t4"),
        ];

        let groups = group_threads_by_project(&order, &projections);
        assert_eq!(groups.len(), 2);

        let pandamux_group = groups.iter().find(|g| g.name == "pandamux").unwrap();
        assert_eq!(pandamux_group.thread_ids.len(), 3);
        assert_eq!(pandamux_group.settled_count, 2); // t1 (Idle) + t3 (Archived)
        assert_eq!(pandamux_group.active_count, 1); // t2 (Working)

        let other_group = groups.iter().find(|g| g.name == "other-repo").unwrap();
        assert_eq!(other_group.thread_ids.len(), 1);
        assert_eq!(other_group.settled_count, 0);
        assert_eq!(other_group.active_count, 1); // t4 (AwaitingApproval)
    }

    #[test]
    fn test_group_threads_fallback_to_workspace_path() {
        let mut t1 = make_test_thread("t1", None, ThreadStatus::Idle, None);
        t1.workspace.cwd = "/home/brandon/Projects/MyWorkspace".to_string();

        let mut projections = HashMap::new();
        projections.insert(ThreadId::new("t1"), ThreadProjection::new(t1));

        let order = vec![ThreadId::new("t1")];
        let groups = group_threads_by_project(&order, &projections);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "MyWorkspace");
        assert_eq!(groups[0].settled_count, 1);
    }
}
