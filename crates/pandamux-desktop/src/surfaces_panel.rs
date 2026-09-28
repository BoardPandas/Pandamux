use std::collections::HashMap;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_client::projections::{ThreadProjection, format_duration};
use pandamux_protocol::FsEntry;
use serde::{Deserialize, Serialize};

use crate::terminal_view::{TerminalViewState, render_terminal_view};
use crate::theme::{Radii, Theme, Typography};

/// Available surface tabs in the right-side surfaces panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SurfaceTab {
    Agents,
    Diff,
    Files,
    PullRequest,
    LinkedPullRequests,
    Terminal,
    FloorPlan,
    Artifacts,
}

impl SurfaceTab {
    pub const ALL: [SurfaceTab; 8] = [
        SurfaceTab::Agents,
        SurfaceTab::Diff,
        SurfaceTab::Files,
        SurfaceTab::PullRequest,
        SurfaceTab::LinkedPullRequests,
        SurfaceTab::Terminal,
        SurfaceTab::FloorPlan,
        SurfaceTab::Artifacts,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Agents => "Agents",
            Self::Diff => "Diff",
            Self::Files => "Files",
            Self::PullRequest => "Pull Request",
            Self::LinkedPullRequests => "Linked PRs",
            Self::Terminal => "Terminal",
            Self::FloorPlan => "Floor Plan",
            Self::Artifacts => "Artifacts",
        }
    }

    pub fn shortcut(&self) -> char {
        match self {
            Self::Agents => 'A',
            Self::Diff => 'D',
            Self::Files => 'F',
            Self::PullRequest => 'P',
            Self::LinkedPullRequests => 'L',
            Self::Terminal => 'T',
            Self::FloorPlan => 'O',
            Self::Artifacts => 'R',
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Agents => "🤖",
            Self::Diff => "📝",
            Self::Files => "📁",
            Self::PullRequest => "🔀",
            Self::LinkedPullRequests => "🔗",
            Self::Terminal => "💻",
            Self::FloorPlan => "🏢",
            Self::Artifacts => "📦",
        }
    }
}

/// PandaMUX agent instance dispatched by orchestrator or user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PandamuxAgentInstance {
    pub id: String,
    pub title: String,
    pub agent_role: String,
    pub provider: String,
    pub model: String,
    pub environment: String,
    pub elapsed_ms: u64,
    pub status: String,
    pub latest_message: Option<String>,
    pub tokens: u64,
    pub tool_calls: u32,
    pub effort: Option<String>,
    pub finished: bool,
}

/// Pull request metadata summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestSummary {
    pub id: String,
    pub number: u32,
    pub title: String,
    pub branch: String,
    pub status: String, // "Open", "Merged", "Draft"
    pub url: String,
    pub compare_url: Option<String>,
    pub author: String,
    pub checks_summary: String,
    pub additions: usize,
    pub deletions: usize,
}

/// State for the right-side surfaces panel.
#[derive(Clone, Debug)]
pub struct SurfacesPanelState {
    pub is_open: bool,
    pub active_tab: Option<SurfaceTab>,
    pub width: f32,
    pub open_tabs_by_thread: HashMap<String, Option<SurfaceTab>>,
    pub files: Vec<FsEntry>,
    pub selected_file: Option<String>,
    pub selected_file_content: Option<String>,
    pub selected_file_is_binary: bool,
    pub pandamux_agents: Vec<PandamuxAgentInstance>,
    pub pull_request: Option<PullRequestSummary>,
    pub linked_prs: Vec<PullRequestSummary>,
    pub terminal: Option<TerminalViewState>,
    pub terminals_by_thread: HashMap<String, TerminalViewState>,
}

impl Default for SurfacesPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfacesPanelState {
    pub fn new() -> Self {
        Self {
            is_open: true,
            active_tab: None,
            width: 360.0,
            open_tabs_by_thread: HashMap::new(),
            files: Vec::new(),
            selected_file: None,
            selected_file_content: None,
            selected_file_is_binary: false,
            pandamux_agents: Vec::new(),
            pull_request: None,
            linked_prs: Vec::new(),
            terminal: None,
            terminals_by_thread: HashMap::new(),
        }
    }

    pub fn set_terminal(&mut self, terminal: TerminalViewState) {
        self.terminal = Some(terminal);
    }

    pub fn set_terminal_for_thread(&mut self, thread_id: &str, terminal: TerminalViewState) {
        self.terminals_by_thread
            .insert(thread_id.to_string(), terminal.clone());
        self.terminal = Some(terminal);
    }

    pub fn terminal_for_thread(&self, thread_id: &str) -> Option<&TerminalViewState> {
        self.terminals_by_thread.get(thread_id)
    }

    pub fn terminal_for_thread_mut(&mut self, thread_id: &str) -> Option<&mut TerminalViewState> {
        self.terminals_by_thread.get_mut(thread_id)
    }

    pub fn terminal_mut(&mut self) -> Option<&mut TerminalViewState> {
        self.terminal.as_mut()
    }

    pub fn toggle_open(&mut self) {
        self.is_open = !self.is_open;
    }

    pub fn set_active_tab(&mut self, thread_id: Option<&str>, tab: Option<SurfaceTab>) {
        self.active_tab = tab;
        if let Some(tid) = thread_id {
            self.open_tabs_by_thread.insert(tid.to_string(), tab);
        }
    }

    pub fn restore_for_thread(&mut self, thread_id: &str) {
        if let Some(&tab) = self.open_tabs_by_thread.get(thread_id) {
            self.active_tab = tab;
        }
    }

    pub fn handle_shortcut(&mut self, ch: char, thread_id: Option<&str>) -> bool {
        let upper = ch.to_ascii_uppercase();
        for tab in SurfaceTab::ALL {
            if tab.shortcut() == upper {
                self.set_active_tab(thread_id, Some(tab));
                return true;
            }
        }
        false
    }
}

/// Renders the complete surfaces panel.
#[allow(clippy::too_many_arguments)]
pub fn render_surfaces_panel<V: 'static>(
    state: &SurfacesPanelState,
    thread_proj: Option<&ThreadProjection>,
    theme: &Theme,
    on_toggle_open: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_select_tab: impl Fn(&mut V, Option<SurfaceTab>, &mut Window, &mut Context<V>) + 'static + Copy,
    on_select_file: impl Fn(&mut V, String, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    if !state.is_open {
        return div().into_any_element();
    }

    let running_subagents = thread_proj
        .map(|p| p.sub_agents.values().filter(|a| !a.finished).count())
        .unwrap_or(0);
    let running_pandamux = state.pandamux_agents.iter().filter(|a| !a.finished).count();
    let total_running = running_subagents + running_pandamux;

    div()
        .id("surfaces-panel")
        .w(px(state.width))
        .h_full()
        .bg(theme.chrome.panel)
        .border_l_1()
        .border_color(rgba(0xffffff14))
        .v_flex()
        .overflow_hidden()
        // Top Header
        .child(
            div()
                .h(px(40.0))
                .px_3()
                .border_b_1()
                .border_color(rgba(0xffffff0d))
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
                                .child("Surfaces"),
                        )
                        .when(total_running > 0, |this| {
                            this.child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(Radii::CHIP)
                                    .bg(rgba(0x43d9c922))
                                    .text_size(Typography::META_SIZE)
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.accent.color())
                                    .child(format!("● {total_running} running")),
                            )
                        }),
                )
                .child(
                    div().h_flex().items_center().gap_1().child(
                        Button::new("btn-close-surfaces")
                            .ghost()
                            .label("✕")
                            .on_click(cx.listener(move |this, _ev, win, cx| {
                                on_toggle_open(this, win, cx);
                            })),
                    ),
                ),
        )
        // Tabs Navigation Row
        .child(
            div()
                .px_2()
                .py_1p5()
                .border_b_1()
                .border_color(rgba(0xffffff08))
                .h_flex()
                .gap_1()
                .overflow_hidden()
                .children(SurfaceTab::ALL.iter().map(|&tab| {
                    let is_active = state.active_tab == Some(tab);
                    let (bg_color, text_color) = if is_active {
                        (rgba(0xffffff12), theme.accent.color())
                    } else {
                        (rgba(0x00000000), theme.chrome.text_t3)
                    };

                    div()
                        .id(ElementId::NamedInteger(
                            "tab-btn".into(),
                            tab.shortcut() as u64,
                        ))
                        .px_2()
                        .py_1()
                        .rounded(Radii::CHIP)
                        .bg(bg_color)
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _ev, win, cx| {
                            let next = if is_active { None } else { Some(tab) };
                            on_select_tab(this, next, win, cx);
                        }))
                        .child(div().text_size(px(11.0)).child(tab.icon()))
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .font_weight(if is_active {
                                    FontWeight::BOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(text_color)
                                .child(tab.label()),
                        )
                        .child(
                            div()
                                .px_1()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff08))
                                .text_size(px(9.0))
                                .text_color(theme.chrome.text_t3)
                                .child(tab.shortcut().to_string()),
                        )
                })),
        )
        // Main Surface Body
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .p_3()
                .child(match state.active_tab {
                    None => render_empty_state(theme, on_select_tab, cx),
                    Some(SurfaceTab::Agents) => {
                        render_agents_surface(thread_proj, &state.pandamux_agents, theme)
                    }
                    Some(SurfaceTab::Diff) => render_diff_surface(thread_proj, theme),
                    Some(SurfaceTab::Files) => render_files_surface(
                        &state.files,
                        state.selected_file.as_deref(),
                        state.selected_file_content.as_deref(),
                        state.selected_file_is_binary,
                        theme,
                        on_select_file,
                        cx,
                    ),
                    Some(SurfaceTab::PullRequest) => {
                        render_pull_request_surface(state.pull_request.as_ref(), theme)
                    }
                    Some(SurfaceTab::LinkedPullRequests) => {
                        render_linked_prs_surface(&state.linked_prs, theme)
                    }
                    Some(SurfaceTab::Terminal) => {
                        render_terminal_surface(state.terminal.as_ref(), theme)
                    }
                    Some(SurfaceTab::FloorPlan) => render_floor_plan_surface(theme),
                    Some(SurfaceTab::Artifacts) => render_artifacts_surface(theme),
                }),
        )
        .into_any_element()
}

/// Renders empty surface state with shortcuts list.
fn render_empty_state<V: 'static>(
    theme: &Theme,
    on_select_tab: impl Fn(&mut V, Option<SurfaceTab>, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    div()
        .v_flex()
        .gap_3()
        .items_center()
        .justify_center()
        .py_8()
        .child(div().text_size(px(28.0)).child("📂"))
        .child(
            div()
                .text_size(Typography::TITLE_SIZE)
                .font_weight(FontWeight::BOLD)
                .text_color(theme.chrome.text_t1)
                .child("Open a surface"),
        )
        .child(
            div()
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child("Single-key shortcuts active while panel has focus:"),
        )
        .child(
            div()
                .w_full()
                .v_flex()
                .gap_1p5()
                .mt_2()
                .children(SurfaceTab::ALL.iter().map(|&tab| {
                    div()
                        .id(ElementId::NamedInteger(
                            "empty-tab".into(),
                            tab.shortcut() as u64,
                        ))
                        .p_2()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel2)
                        .border_1()
                        .border_color(rgba(0xffffff0d))
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _ev, win, cx| {
                            on_select_tab(this, Some(tab), win, cx);
                        }))
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_2()
                                .child(div().text_size(px(14.0)).child(tab.icon()))
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .text_color(theme.chrome.text_t1)
                                        .child(tab.label()),
                                ),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff0f))
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.accent.color())
                                .child(tab.shortcut().to_string()),
                        )
                })),
        )
        .child(
            div()
                .mt_4()
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child("Press Esc to return focus to the composer."),
        )
        .into_any_element()
}

/// Renders the Agents surface with two groups: Provider sub-agents and PandaMUX agents.
fn render_agents_surface(
    thread_proj: Option<&ThreadProjection>,
    pandamux_agents: &[PandamuxAgentInstance],
    theme: &Theme,
) -> AnyElement {
    let subagents = thread_proj.map(|p| p.subagent_tree()).unwrap_or_default();

    div()
        .v_flex()
        .gap_4()
        // Group 1: Provider sub-agents
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
                                .text_size(Typography::SECONDARY_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t2)
                                .child("Provider sub-agents"),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff08))
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(format!("{}", subagents.len())),
                        ),
                )
                .child(if subagents.is_empty() {
                    div()
                        .p_3()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel2)
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child("No provider sub-agents spawned for this thread.")
                        .into_any_element()
                } else {
                    div()
                        .v_flex()
                        .gap_2()
                        .children(
                            subagents
                                .iter()
                                .map(|agent| render_subagent_surface_node(agent, 0, theme)),
                        )
                        .into_any_element()
                }),
        )
        // Group 2: PandaMUX agents
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
                                .text_size(Typography::SECONDARY_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t2)
                                .child("PandaMUX agents"),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff08))
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(format!("{}", pandamux_agents.len())),
                        ),
                )
                .child(if pandamux_agents.is_empty() {
                    div()
                        .p_3()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel2)
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child("No orchestrator or specialist agent instances.")
                        .into_any_element()
                } else {
                    div()
                        .v_flex()
                        .gap_2()
                        .children(
                            pandamux_agents
                                .iter()
                                .map(|agent| render_pandamux_agent_node(agent, theme)),
                        )
                        .into_any_element()
                }),
        )
        .into_any_element()
}

/// Recursive helper to render a sub-agent tree node with indentation.
fn render_subagent_surface_node(
    node: &pandamux_client::projections::SubAgentTreeNode,
    depth: usize,
    theme: &Theme,
) -> AnyElement {
    let p = &node.agent;
    let (status_icon, status_text, status_color) = if p.finished {
        (
            "✓",
            p.outcome.as_deref().unwrap_or("Succeeded"),
            theme.terminal.success,
        )
    } else {
        ("●", "Running", theme.accent.color())
    };

    let dur = p.elapsed_ms.map(format_duration);

    div()
        .when(depth > 0, |this| {
            this.ml_3().border_l_2().border_color(theme.accent.color())
        })
        .p_2p5()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .border_1()
        .border_color(rgba(0xffffff0d))
        .v_flex()
        .gap_1p5()
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
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff0d))
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.accent.color())
                                .child(p.agent_type.clone()),
                        )
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.chrome.text_t1)
                                .child(p.title.clone()),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(status_color)
                                .child(format!("{status_icon} {status_text}")),
                        )
                        .when_some(dur, |this, d| {
                            this.child(
                                div()
                                    .text_size(Typography::META_SIZE)
                                    .text_color(theme.chrome.text_t3)
                                    .child(format!("({d})")),
                            )
                        }),
                ),
        )
        .when_some(p.latest_activity.as_ref(), |this, act| {
            this.child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.chrome.text_t2)
                    .child(format!("⚡ {act}")),
            )
        })
        .child(
            div()
                .h_flex()
                .gap_3()
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child(div().child(format!("Model: {}", p.model)))
                .child(div().child(format!("📊 {} tokens", p.tokens)))
                .child(div().child(format!("🛠️ {} tools", p.tool_calls))),
        )
        // Nested children
        .when(!node.children.is_empty(), |this| {
            this.children(
                node.children
                    .iter()
                    .map(|child| render_subagent_surface_node(child, depth + 1, theme)),
            )
        })
        .into_any_element()
}

/// Renders a PandaMUX agent instance node.
fn render_pandamux_agent_node(agent: &PandamuxAgentInstance, theme: &Theme) -> AnyElement {
    let (status_icon, status_color) = if agent.finished {
        ("✓ Succeeded", theme.terminal.success)
    } else {
        ("● Running", theme.accent.color())
    };

    let dur = format_duration(agent.elapsed_ms);

    div()
        .p_2p5()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .border_1()
        .border_color(rgba(0xffffff0d))
        .v_flex()
        .gap_1p5()
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
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff0d))
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.accent.color())
                                .child(agent.agent_role.clone()),
                        )
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.chrome.text_t1)
                                .child(agent.title.clone()),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(status_color)
                                .child(status_icon),
                        )
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(format!("({dur})")),
                        ),
                ),
        )
        .child(
            div()
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child(format!(
                    "{} / {} ({})",
                    agent.provider, agent.model, agent.environment
                )),
        )
        .when_some(agent.latest_message.as_ref(), |this, msg| {
            this.child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.chrome.text_t2)
                    .child(format!("💬 {msg}")),
            )
        })
        .child(
            div()
                .h_flex()
                .gap_3()
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child(div().child(format!("📊 {} tokens", agent.tokens)))
                .child(div().child(format!("🛠️ {} tools", agent.tool_calls))),
        )
        .into_any_element()
}

/// Renders the Diff surface summarizing changes in the thread.
fn render_diff_surface(thread_proj: Option<&ThreadProjection>, theme: &Theme) -> AnyElement {
    let (files, total_additions, total_deletions) = thread_proj
        .and_then(|p| {
            for item in p.grouped_items().iter().rev() {
                if let pandamux_client::projections::TimelineItem::ChangedFiles {
                    files,
                    total_additions,
                    total_deletions,
                } = item
                {
                    return Some((files.clone(), *total_additions, *total_deletions));
                }
            }
            None
        })
        .unwrap_or_default();

    div()
        .v_flex()
        .gap_3()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t1)
                        .child("Thread Diff"),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_1p5()
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0x7fd88f1a))
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.terminal.success)
                                .child(format!("+{total_additions}")),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xf871711a))
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xf87171))
                                .child(format!("-{total_deletions}")),
                        ),
                ),
        )
        .child(if files.is_empty() {
            div()
                .p_4()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel2)
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child("No modified files in this thread.")
                .into_any_element()
        } else {
            div()
                .v_flex()
                .gap_1p5()
                .children(files.iter().map(|f| {
                    div()
                        .p_2()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel2)
                        .border_1()
                        .border_color(rgba(0xffffff0d))
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .font_family(Typography::MONO_FAMILY)
                                .text_color(theme.chrome.text_t1)
                                .child(f.path.clone()),
                        )
                        .child(
                            div()
                                .h_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .text_color(theme.terminal.success)
                                        .child(format!("+{}", f.additions)),
                                )
                                .child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .text_color(rgb(0xf87171))
                                        .child(format!("-{}", f.deletions)),
                                ),
                        )
                }))
                .into_any_element()
        })
        .into_any_element()
}

/// Renders the Files surface showing the confined workspace file tree and preview.
#[allow(clippy::too_many_arguments)]
fn render_files_surface<V: 'static>(
    files: &[FsEntry],
    selected_file: Option<&str>,
    content: Option<&str>,
    is_binary: bool,
    theme: &Theme,
    on_select_file: impl Fn(&mut V, String, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> AnyElement {
    div()
        .v_flex()
        .gap_3()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t1)
                        .child("Workspace Files"),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child(format!("{} entries", files.len())),
                ),
        )
        // File Tree List
        .child(if files.is_empty() {
            div()
                .p_4()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel2)
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child("No files loaded or empty directory.")
                .into_any_element()
        } else {
            div()
                .v_flex()
                .gap_1()
                .max_h(px(240.0))
                .overflow_hidden()
                .children(files.iter().enumerate().map(|(idx, f)| {
                    let is_sel = selected_file == Some(&f.relative_path);
                    let (bg, text) = if is_sel {
                        (rgba(0xffffff14), theme.accent.color())
                    } else {
                        (theme.chrome.panel2, theme.chrome.text_t1)
                    };
                    let icon = if f.is_dir { "📁" } else { "📄" };
                    let path = f.relative_path.clone();

                    div()
                        .id(ElementId::NamedInteger("file-row".into(), idx as u64))
                        .p_1p5()
                        .rounded(Radii::ROW)
                        .bg(bg)
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _ev, win, cx| {
                            on_select_file(this, path.clone(), win, cx);
                        }))
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1p5()
                                .child(div().text_size(px(12.0)).child(icon))
                                .child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .text_color(text)
                                        .child(f.name.clone()),
                                ),
                        )
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(format!("{} B", f.size_bytes)),
                        )
                }))
                .into_any_element()
        })
        // Selected File Content Preview
        .when_some(selected_file, |this, sel| {
            this.child(
                div()
                    .v_flex()
                    .gap_1p5()
                    .p_2()
                    .rounded(Radii::ROW)
                    .bg(theme.terminal.surface)
                    .border_1()
                    .border_color(rgba(0xffffff0d))
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(Typography::META_SIZE)
                                    .font_family(Typography::MONO_FAMILY)
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.accent.color())
                                    .child(sel.to_string()),
                            )
                            .when(is_binary, |this| {
                                this.child(
                                    div()
                                        .px_1()
                                        .rounded(Radii::CHIP)
                                        .bg(rgba(0xffffff0d))
                                        .text_size(px(9.0))
                                        .text_color(theme.chrome.text_t3)
                                        .child("BINARY"),
                                )
                            }),
                    )
                    .child(if is_binary {
                        div()
                            .text_size(Typography::META_SIZE)
                            .text_color(theme.chrome.text_t3)
                            .child("Binary file content not displayed.")
                            .into_any_element()
                    } else if let Some(txt) = content {
                        div()
                            .max_h(px(200.0))
                            .overflow_hidden()
                            .font_family(Typography::MONO_FAMILY)
                            .text_size(Typography::META_SIZE)
                            .text_color(theme.chrome.text_t2)
                            .child(txt.to_string())
                            .into_any_element()
                    } else {
                        div()
                            .text_size(Typography::META_SIZE)
                            .text_color(theme.chrome.text_t3)
                            .child("Loading file content...")
                            .into_any_element()
                    }),
            )
        })
        .into_any_element()
}

/// Renders the Pull Request surface.
fn render_pull_request_surface(pr: Option<&PullRequestSummary>, theme: &Theme) -> AnyElement {
    div()
        .v_flex()
        .gap_3()
        .child(
            div()
                .text_size(Typography::BODY_SIZE)
                .font_weight(FontWeight::BOLD)
                .text_color(theme.chrome.text_t1)
                .child("Pull Request"),
        )
        .child(if let Some(pr) = pr {
            div()
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel2)
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
                                .text_size(Typography::BODY_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t1)
                                .child(format!("#{} {}", pr.number, pr.title)),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0x43d9c922))
                                .text_size(Typography::META_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.accent.color())
                                .child(pr.status.clone()),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child(format!("Branch: {}", pr.branch)),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t2)
                        .child(format!("Checks: {}", pr.checks_summary)),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.accent.color())
                        .child(pr.url.clone()),
                )
                .into_any_element()
        } else {
            div()
                .p_4()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel2)
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child("No pull request open for this branch. Create one from Git Actions.")
                .into_any_element()
        })
        .into_any_element()
}

/// Renders the Linked PRs surface.
fn render_linked_prs_surface(prs: &[PullRequestSummary], theme: &Theme) -> AnyElement {
    div()
        .v_flex()
        .gap_3()
        .child(
            div()
                .text_size(Typography::BODY_SIZE)
                .font_weight(FontWeight::BOLD)
                .text_color(theme.chrome.text_t1)
                .child("Linked Pull Requests"),
        )
        .child(if prs.is_empty() {
            div()
                .p_4()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel2)
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child("No linked pull requests across related worktrees.")
                .into_any_element()
        } else {
            div()
                .v_flex()
                .gap_2()
                .children(prs.iter().map(|pr| {
                    div()
                        .p_2p5()
                        .rounded(Radii::ROW)
                        .bg(theme.chrome.panel2)
                        .border_1()
                        .border_color(rgba(0xffffff0d))
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_size(Typography::BODY_SIZE)
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.chrome.text_t1)
                                .child(format!("#{} {}", pr.number, pr.title)),
                        )
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(format!("Branch: {}", pr.branch)),
                        )
                }))
                .into_any_element()
        })
        .into_any_element()
}

/// Terminal surface (local or remote).
fn render_terminal_surface(terminal: Option<&TerminalViewState>, theme: &Theme) -> AnyElement {
    if let Some(term) = terminal {
        render_terminal_view(term, theme)
    } else {
        div()
            .p_4()
            .rounded(Radii::ROW)
            .bg(theme.chrome.panel2)
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_size(px(18.0)).child("💻"))
                    .child(
                        div()
                            .text_size(Typography::BODY_SIZE)
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.chrome.text_t1)
                            .child("Terminal Surface"),
                    ),
            )
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.chrome.text_t3)
                    .child("No active terminal session attached. Launch or reattach a local or remote terminal session from the surfaces bar."),
            )
            .into_any_element()
    }
}

/// Floor Plan placeholder surface.
fn render_floor_plan_surface(theme: &Theme) -> AnyElement {
    div()
        .p_4()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .v_flex()
        .gap_2()
        .child(
            div()
                .text_size(Typography::BODY_SIZE)
                .font_weight(FontWeight::BOLD)
                .text_color(theme.chrome.text_t1)
                .child("Floor Plan"),
        )
        .child(
            div()
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child("Ambient 2D Virtual Office with active avatars."),
        )
        .into_any_element()
}

/// Artifacts placeholder surface.
fn render_artifacts_surface(theme: &Theme) -> AnyElement {
    div()
        .p_4()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .v_flex()
        .gap_2()
        .child(
            div()
                .text_size(Typography::BODY_SIZE)
                .font_weight(FontWeight::BOLD)
                .text_color(theme.chrome.text_t1)
                .child("Artifacts"),
        )
        .child(
            div()
                .text_size(Typography::META_SIZE)
                .text_color(theme.chrome.text_t3)
                .child("Generated HTML/SVG assets with Broadside preview."),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn test_surfaces_panel_state_defaults() {
        let state = SurfacesPanelState::new();
        assert!(state.is_open);
        assert_eq!(state.active_tab, None);
        assert_eq!(state.width, 360.0);
    }

    #[test]
    fn test_surfaces_panel_shortcuts() {
        let mut state = SurfacesPanelState::new();

        assert!(state.handle_shortcut('A', Some("thread-1")));
        assert_eq!(state.active_tab, Some(SurfaceTab::Agents));

        assert!(state.handle_shortcut('d', Some("thread-1")));
        assert_eq!(state.active_tab, Some(SurfaceTab::Diff));

        assert!(state.handle_shortcut('F', Some("thread-1")));
        assert_eq!(state.active_tab, Some(SurfaceTab::Files));

        assert!(state.handle_shortcut('p', Some("thread-1")));
        assert_eq!(state.active_tab, Some(SurfaceTab::PullRequest));

        assert!(state.handle_shortcut('l', Some("thread-1")));
        assert_eq!(state.active_tab, Some(SurfaceTab::LinkedPullRequests));

        assert!(state.handle_shortcut('t', Some("thread-1")));
        assert_eq!(state.active_tab, Some(SurfaceTab::Terminal));

        assert!(state.handle_shortcut('o', Some("thread-1")));
        assert_eq!(state.active_tab, Some(SurfaceTab::FloorPlan));

        assert!(state.handle_shortcut('r', Some("thread-1")));
        assert_eq!(state.active_tab, Some(SurfaceTab::Artifacts));

        assert!(!state.handle_shortcut('z', Some("thread-1")));
    }

    #[test]
    fn test_surfaces_panel_restore_for_thread() {
        let mut state = SurfacesPanelState::new();
        state.set_active_tab(Some("thread-1"), Some(SurfaceTab::Diff));
        state.set_active_tab(Some("thread-2"), Some(SurfaceTab::Files));

        state.restore_for_thread("thread-1");
        assert_eq!(state.active_tab, Some(SurfaceTab::Diff));

        state.restore_for_thread("thread-2");
        assert_eq!(state.active_tab, Some(SurfaceTab::Files));
    }

    #[test]
    fn test_surfaces_panel_terminal_per_thread() {
        let mut state = SurfacesPanelState::new();
        let term1 = TerminalViewState::new("term-thread-1", "/work/1", 24, 80);
        let term2 = TerminalViewState::new_with_env(
            "term-thread-2",
            "/work/2",
            30,
            120,
            Some(pandamux_core::EnvironmentId::new("env_remote")),
        );

        state.set_terminal_for_thread("t1", term1);
        state.set_terminal_for_thread("t2", term2);

        assert_eq!(
            state.terminal_for_thread("t1").unwrap().terminal_id,
            "term-thread-1"
        );
        assert_eq!(
            state.terminal_for_thread("t2").unwrap().terminal_id,
            "term-thread-2"
        );
        assert!(state.terminal_for_thread("t2").unwrap().is_remote());
        assert_eq!(
            state.terminal.as_ref().unwrap().terminal_id,
            "term-thread-2"
        );
    }
}
