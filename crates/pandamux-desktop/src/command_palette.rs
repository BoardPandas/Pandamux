use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_core::ThreadId;

use crate::sidebar::RailTab;
use crate::theme::{AccentColor, Radii, Theme, Typography};

/// Available executable actions from the Command Palette.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandAction {
    SelectRail(RailTab),
    ToggleTheme,
    SetAccent(AccentColor),
    NewThread,
    SelectThread(ThreadId),
    GitCommit,
    GitPush,
    GitPr,
    GitRefresh,
    CheckHealth,
    OpenSettings,
}

/// An individual command entry listed in the palette.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandItem {
    pub id: String,
    pub icon: &'static str,
    pub title: String,
    pub category: &'static str,
    pub shortcut: Option<&'static str>,
    pub action: CommandAction,
}

/// State for the Command Palette modal and search filtering.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CommandPaletteState {
    pub is_open: bool,
    pub query: String,
    pub selected_index: usize,
}

impl CommandPaletteState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.query.clear();
        self.selected_index = 0;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.query.clear();
        self.selected_index = 0;
    }

    pub fn toggle(&mut self) {
        if self.is_open {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.selected_index = 0;
    }

    pub fn select_next(&mut self, max: usize) {
        if max > 0 {
            self.selected_index = (self.selected_index + 1) % max;
        }
    }

    pub fn select_prev(&mut self, max: usize) {
        if max > 0 {
            if self.selected_index == 0 {
                self.selected_index = max - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }
}

/// Builds the default catalog of commands available in PandaMUX.
#[allow(clippy::vec_init_then_push)]
pub fn default_commands(
    threads: &[(ThreadId, String)],
    active_thread_id: Option<&ThreadId>,
) -> Vec<CommandItem> {
    let mut items = Vec::new();

    // 1. General & Navigation
    items.push(CommandItem {
        id: "new-thread".to_string(),
        icon: "➕",
        title: "Create New Thread".to_string(),
        category: "Navigation",
        shortcut: Some("Ctrl+T"),
        action: CommandAction::NewThread,
    });
    items.push(CommandItem {
        id: "nav-threads".to_string(),
        icon: "💬",
        title: "View Threads & Chats".to_string(),
        category: "Navigation",
        shortcut: Some("Ctrl+1"),
        action: CommandAction::SelectRail(RailTab::Threads),
    });
    items.push(CommandItem {
        id: "nav-agents".to_string(),
        icon: "🤖",
        title: "View AI Agent Roster".to_string(),
        category: "Navigation",
        shortcut: Some("Ctrl+2"),
        action: CommandAction::SelectRail(RailTab::Agents),
    });
    items.push(CommandItem {
        id: "nav-terminals".to_string(),
        icon: "📟",
        title: "View Terminal Multiplexer".to_string(),
        category: "Navigation",
        shortcut: Some("Ctrl+3"),
        action: CommandAction::SelectRail(RailTab::Terminal),
    });
    items.push(CommandItem {
        id: "nav-settings".to_string(),
        icon: "⚙️",
        title: "Open Application Settings".to_string(),
        category: "Navigation",
        shortcut: Some("Ctrl+,"),
        action: CommandAction::OpenSettings,
    });

    // 2. Active Threads Switcher
    for (tid, title) in threads {
        let is_active = active_thread_id.map(|a| a == tid).unwrap_or(false);
        items.push(CommandItem {
            id: format!("switch-thread-{}", tid.as_str()),
            icon: if is_active { "🟢" } else { "⚪" },
            title: format!("Switch to Thread: {title}"),
            category: "Threads",
            shortcut: None,
            action: CommandAction::SelectThread(tid.clone()),
        });
    }

    // 3. Theme & Appearance
    items.push(CommandItem {
        id: "toggle-theme".to_string(),
        icon: "🌓",
        title: "Toggle Light / Dark Theme".to_string(),
        category: "Appearance",
        shortcut: Some("Ctrl+Shift+T"),
        action: CommandAction::ToggleTheme,
    });
    items.push(CommandItem {
        id: "accent-teal".to_string(),
        icon: "💎",
        title: "Set Accent Color: Mint Teal".to_string(),
        category: "Appearance",
        shortcut: None,
        action: CommandAction::SetAccent(AccentColor::Teal),
    });
    items.push(CommandItem {
        id: "accent-gold".to_string(),
        icon: "🌟",
        title: "Set Accent Color: Gold Amber".to_string(),
        category: "Appearance",
        shortcut: None,
        action: CommandAction::SetAccent(AccentColor::Gold),
    });
    items.push(CommandItem {
        id: "accent-blue".to_string(),
        icon: "🔷",
        title: "Set Accent Color: Sky Blue".to_string(),
        category: "Appearance",
        shortcut: None,
        action: CommandAction::SetAccent(AccentColor::Blue),
    });
    items.push(CommandItem {
        id: "accent-purple".to_string(),
        icon: "🟣",
        title: "Set Accent Color: Royal Purple".to_string(),
        category: "Appearance",
        shortcut: None,
        action: CommandAction::SetAccent(AccentColor::Purple),
    });

    // 4. Git Actions
    items.push(CommandItem {
        id: "git-commit".to_string(),
        icon: "📦",
        title: "Git: Commit Changes (with drafted message)".to_string(),
        category: "Git",
        shortcut: Some("Ctrl+G"),
        action: CommandAction::GitCommit,
    });
    items.push(CommandItem {
        id: "git-push".to_string(),
        icon: "🚀",
        title: "Git: Push to Remote Origin".to_string(),
        category: "Git",
        shortcut: None,
        action: CommandAction::GitPush,
    });
    items.push(CommandItem {
        id: "git-pr".to_string(),
        icon: "🔀",
        title: "Git: Create Pull Request (gh / compare URL)".to_string(),
        category: "Git",
        shortcut: None,
        action: CommandAction::GitPr,
    });
    items.push(CommandItem {
        id: "git-refresh".to_string(),
        icon: "🔄",
        title: "Git: Refresh Worktree Status".to_string(),
        category: "Git",
        shortcut: None,
        action: CommandAction::GitRefresh,
    });

    // 5. Providers & Diagnostics
    items.push(CommandItem {
        id: "check-health".to_string(),
        icon: "🩺",
        title: "Providers: Run Offline Health Checks (Rule 1: zero-spawn)".to_string(),
        category: "Providers",
        shortcut: None,
        action: CommandAction::CheckHealth,
    });

    items
}

/// Filters command items against search query.
pub fn filter_commands<'a>(commands: &'a [CommandItem], query: &str) -> Vec<&'a CommandItem> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return commands.iter().collect();
    }

    commands
        .iter()
        .filter(|cmd| {
            cmd.title.to_lowercase().contains(&q) || cmd.category.to_lowercase().contains(&q)
        })
        .collect()
}

/// Renders the 560px centered Command Palette overlay.
pub fn render_command_palette<V: 'static>(
    state: &CommandPaletteState,
    commands: &[CommandItem],
    theme: &Theme,
    on_select_action: impl Fn(&mut V, CommandAction, &mut Window, &mut Context<V>) + 'static + Copy,
    on_filter: impl Fn(&mut V, String, &mut Window, &mut Context<V>) + 'static + Copy,
    on_close: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let filtered = filter_commands(commands, &state.query);
    let selected_idx = state.selected_index.min(filtered.len().saturating_sub(1));

    // Fullscreen Scrim Backdrop
    div()
        .id("command-palette-backdrop")
        .size_full()
        .absolute()
        .top_0()
        .left_0()
        .bg(theme.chrome.scrim)
        .h_flex()
        .items_start()
        .justify_center()
        .pt(px(80.0))
        .on_click(cx.listener(move |this, _event, window, cx| {
            on_close(this, window, cx);
        }))
        // 560px Centered Modal Card
        .child(
            div()
                .id("command-palette-modal")
                .w(px(560.0))
                .rounded(Radii::OVERLAY)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff1a))
                .shadow_lg()
                .v_flex()
                .overflow_hidden()
                .on_click(|_event, _window, cx| {
                    cx.stop_propagation();
                })
                // Top Search Bar
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .px_4()
                        .py_3()
                        .gap_3()
                        .border_b_1()
                        .border_color(rgba(0xffffff0d))
                        .child(div().text_size(px(16.0)).child("🔍"))
                        .child(
                            div()
                                .flex_1()
                                .text_size(Typography::BODY_SIZE)
                                .text_color(theme.chrome.text_t1)
                                .child(if state.query.is_empty() {
                                    "Type a command, thread title, or shortcut...".to_string()
                                } else {
                                    state.query.clone()
                                }),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff0a))
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child("ESC"),
                        ),
                )
                // Category Quick Filter Bar
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .py_1p5()
                        .bg(theme.chrome.bgc_knockout)
                        .border_b_1()
                        .border_color(rgba(0xffffff0d))
                        .child(
                            Button::new("filter-cat-all")
                                .ghost()
                                .label(if state.query.is_empty() {
                                    "● All"
                                } else {
                                    "All"
                                })
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_filter(this, String::new(), window, cx);
                                })),
                        )
                        .child(
                            Button::new("filter-cat-nav")
                                .ghost()
                                .label("Navigation")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_filter(this, "Navigation".to_string(), window, cx);
                                })),
                        )
                        .child(
                            Button::new("filter-cat-appearance")
                                .ghost()
                                .label("Appearance")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_filter(this, "Appearance".to_string(), window, cx);
                                })),
                        )
                        .child(Button::new("filter-cat-git").ghost().label("Git").on_click(
                            cx.listener(move |this, _event, window, cx| {
                                on_filter(this, "Git".to_string(), window, cx);
                            }),
                        ))
                        .child(
                            Button::new("filter-cat-providers")
                                .ghost()
                                .label("Providers")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_filter(this, "Providers".to_string(), window, cx);
                                })),
                        )
                        .child(
                            Button::new("filter-cat-threads")
                                .ghost()
                                .label("Threads")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_filter(this, "Threads".to_string(), window, cx);
                                })),
                        ),
                )
                // Filtered Command List
                .child(
                    div()
                        .max_h(px(360.0))
                        .v_flex()
                        .p_2()
                        .gap_1()
                        .overflow_hidden()
                        .children(filtered.into_iter().enumerate().map(|(idx, cmd)| {
                            let is_selected = idx == selected_idx;
                            let action = cmd.action.clone();
                            let bg_color = if is_selected {
                                rgba(0xffffff14)
                            } else {
                                rgba(0x00000000)
                            };
                            let border_color = if is_selected {
                                theme.accent_color()
                            } else {
                                rgba(0x00000000)
                            };

                            div()
                                .id(ElementId::named_usize("cmd-item", idx))
                                .h_flex()
                                .items_center()
                                .justify_between()
                                .px_3()
                                .py_2()
                                .rounded(Radii::ROW)
                                .bg(bg_color)
                                .border_1()
                                .border_color(border_color)
                                .child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_2p5()
                                        .child(div().text_size(px(14.0)).child(cmd.icon))
                                        .child(
                                            div()
                                                .px_1p5()
                                                .py_0p5()
                                                .rounded(Radii::CHIP)
                                                .bg(rgba(0xffffff0a))
                                                .text_size(Typography::META_SIZE)
                                                .text_color(theme.chrome.text_t3)
                                                .child(cmd.category),
                                        )
                                        .child(
                                            div()
                                                .text_size(Typography::BODY_SIZE)
                                                .text_color(if is_selected {
                                                    theme.chrome.text_t1
                                                } else {
                                                    theme.chrome.text_t2
                                                })
                                                .child(cmd.title.clone()),
                                        ),
                                )
                                .when_some(cmd.shortcut, |this, shortcut| {
                                    this.child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded(Radii::CHIP)
                                            .bg(rgba(0xffffff08))
                                            .font_family(Typography::MONO_FAMILY)
                                            .text_size(Typography::META_SIZE)
                                            .text_color(theme.chrome.text_t3)
                                            .child(shortcut),
                                    )
                                })
                                .child(
                                    Button::new(format!("btn-run-{}", cmd.id))
                                        .ghost()
                                        .label("↵ Run")
                                        .on_click(cx.listener(move |this, _event, window, cx| {
                                            on_select_action(this, action.clone(), window, cx);
                                        })),
                                )
                        })),
                )
                // Bottom Help Footer
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .px_4()
                        .py_2()
                        .bg(theme.chrome.bgc_knockout)
                        .border_t_1()
                        .border_color(rgba(0xffffff0d))
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child(div().child("↑↓ Navigate · ↵ Select · Esc Close"))
                        .child(div().child("PandaMUX Command Engine")),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn test_filter_commands() {
        let threads = vec![(ThreadId::from("thread-1"), "My Working Thread".to_string())];
        let commands = default_commands(&threads, None);

        // Filter by title
        let results = filter_commands(&commands, "theme");
        assert!(!results.is_empty());
        assert!(results.iter().any(|c| c.id == "toggle-theme"));

        // Filter by category
        let git_results = filter_commands(&commands, "git");
        assert!(git_results.len() >= 4);

        // Filter by thread title
        let thread_results = filter_commands(&commands, "working thread");
        assert_eq!(thread_results.len(), 1);
    }

    #[test]
    fn test_palette_state_cycle() {
        let mut state = CommandPaletteState::new();
        assert!(!state.is_open);

        state.open();
        assert!(state.is_open);

        state.select_next(5);
        assert_eq!(state.selected_index, 1);

        state.select_prev(5);
        assert_eq!(state.selected_index, 0);

        state.select_prev(5);
        assert_eq!(state.selected_index, 4);

        state.close();
        assert!(!state.is_open);
        assert_eq!(state.selected_index, 0);
    }
}
