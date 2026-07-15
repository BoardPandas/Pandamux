//! The 264px session panel: the signature left-side navigation.
//!
//! Per the owner-confirmed model (plan Section 12.1 #2), a *session* is a shell
//! context surfaced across workspaces, not a saved layout. Sessions are a
//! projection over canonical state: every terminal surface in every workspace is
//! one session. Selecting a session focuses its pane (and activates its
//! workspace); it never swaps the on-screen layout. The panel groups sessions by
//! Project / Type / Host with a live-regroup segment switcher, and the group
//! containing the active session is pinned first.

use crate::chrome::SessionActivity;
use crate::iced_shell::ShellMessage;
use crate::theme::{self, Palette, ShellKind};
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Color, Element, Length, Padding};
use pandamux_core::{AppState, SplitNode, SurfaceId, SurfaceType, WorkspaceId, WorkspaceState};

/// How the panel groups sessions. The switcher regroups live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SessionGrouping {
    #[default]
    Project,
    Type,
    Host,
}

impl SessionGrouping {
    pub const ALL: [SessionGrouping; 3] = [
        SessionGrouping::Project,
        SessionGrouping::Type,
        SessionGrouping::Host,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SessionGrouping::Project => "Project",
            SessionGrouping::Type => "Type",
            SessionGrouping::Host => "Host",
        }
    }
}

/// One session row: a terminal surface presented as a shell context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEntry {
    pub surface_id: SurfaceId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub meta: String,
    pub kind: ShellKind,
    pub activity: SessionActivity,
    pub is_active: bool,
}

/// A named group of sessions (the group key depends on the active grouping).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionGroup {
    pub key: String,
    pub entries: Vec<SessionEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SessionsViewState {
    pub open: bool,
    pub grouping: SessionGrouping,
    pub groups: Vec<SessionGroup>,
    pub total: usize,
}

/// Project canonical state into the session list. `active_surface_id` marks the
/// session currently focused in the active workspace.
pub fn project_sessions(
    app: &AppState,
    grouping: SessionGrouping,
    open: bool,
    active_surface_id: Option<&SurfaceId>,
) -> SessionsViewState {
    let mut entries: Vec<SessionEntry> = Vec::new();
    for workspace in &app.workspaces {
        let terminals = terminal_surfaces(&workspace.split_tree);
        let multiple = terminals.len() > 1;
        for (index, surface_id) in terminals.into_iter().enumerate() {
            let kind = ShellKind::classify(&workspace.shell);
            let host = host_label(&workspace.shell);
            let name = if multiple {
                format!("{} \u{00b7} {}", workspace.title, index + 1)
            } else {
                workspace.title.clone()
            };
            let meta = format!("{} \u{00b7} {}", workspace.shell, host);
            entries.push(SessionEntry {
                is_active: active_surface_id == Some(&surface_id),
                surface_id,
                workspace_id: workspace.id.clone(),
                name,
                meta,
                kind,
                activity: activity_for(workspace, active_surface_id),
            });
        }
    }

    let total = entries.len();
    let groups = group_entries(entries, grouping, app);
    SessionsViewState {
        open,
        grouping,
        groups,
        total,
    }
}

fn group_entries(
    entries: Vec<SessionEntry>,
    grouping: SessionGrouping,
    app: &AppState,
) -> Vec<SessionGroup> {
    let mut groups: Vec<SessionGroup> = Vec::new();
    for entry in entries {
        let key = match grouping {
            SessionGrouping::Project => app
                .workspace(&entry.workspace_id)
                .map(|workspace| workspace.title.clone())
                .unwrap_or_else(|| "Workspace".to_string()),
            SessionGrouping::Type => entry.kind.abbreviation().to_string(),
            SessionGrouping::Host => host_from_meta(&entry.meta),
        };
        match groups.iter_mut().find(|group| group.key == key) {
            Some(group) => group.entries.push(entry),
            None => groups.push(SessionGroup {
                key,
                entries: vec![entry],
            }),
        }
    }

    // Pinned-first: the group holding the active session floats to the top, the
    // rest stay alphabetical for a stable order.
    groups.sort_by(|a, b| {
        let a_active = a.entries.iter().any(|entry| entry.is_active);
        let b_active = b.entries.iter().any(|entry| entry.is_active);
        b_active.cmp(&a_active).then_with(|| a.key.cmp(&b.key))
    });
    groups
}

/// Running for the active workspace's focused surface, idle otherwise. Busy-agent
/// detection arrives with the agent observer.
fn activity_for(
    workspace: &WorkspaceState,
    active_surface_id: Option<&SurfaceId>,
) -> SessionActivity {
    let is_active_workspace = active_surface_id
        .is_some_and(|surface_id| terminal_surfaces(&workspace.split_tree).contains(surface_id));
    if is_active_workspace {
        SessionActivity::Running
    } else {
        SessionActivity::Idle
    }
}

fn host_label(shell: &str) -> String {
    let lower = shell.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("ssh ") {
        // "ssh user@host ..." -> host
        rest.split_whitespace()
            .next()
            .map(|target| target.rsplit('@').next().unwrap_or(target).to_string())
            .unwrap_or_else(|| "remote".to_string())
    } else {
        "local".to_string()
    }
}

fn host_from_meta(meta: &str) -> String {
    meta.rsplit(" \u{00b7} ")
        .next()
        .unwrap_or("local")
        .to_string()
}

fn terminal_surfaces(tree: &SplitNode) -> Vec<SurfaceId> {
    match tree {
        SplitNode::Leaf(leaf) => leaf
            .surfaces
            .iter()
            .filter(|surface| surface.surface_type == SurfaceType::Terminal)
            .map(|surface| surface.id.clone())
            .collect(),
        SplitNode::Branch(branch) => {
            let mut ids = terminal_surfaces(&branch.children[0]);
            ids.extend(terminal_surfaces(&branch.children[1]));
            ids
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn session_panel<'a>(
    state: &'a SessionsViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let header = row![
        text("Sessions")
            .size(theme::SIZE_TITLE)
            .font(theme::ui(iced::font::Weight::Semibold))
            .color(palette.t1),
        Space::new().width(Length::Fill),
        text(format!("{}", state.total))
            .size(theme::SIZE_METADATA)
            .font(theme::mono(iced::font::Weight::Medium))
            .color(palette.t4),
    ]
    .align_y(Alignment::Center);

    let switcher = grouping_switcher(state.grouping, palette);

    let mut list = column![].spacing(10).width(Length::Fill);
    for group in &state.groups {
        list = list.push(group_header(&group.key, palette));
        for entry in &group.entries {
            list = list.push(session_row(entry, palette));
        }
    }

    let body = column![
        header,
        switcher,
        scrollable(list).height(Length::Fill).width(Length::Fill),
        new_session_footer(palette),
    ]
    .spacing(10)
    .padding(Padding::from([12.0, 10.0]))
    .width(Length::Fill)
    .height(Length::Fill);

    container(body)
        .width(Length::Fixed(theme::SESSION_PANEL_WIDTH))
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(palette.ov(0.02).into()),
            border: theme::border(palette.ov(0.06), 1.0, 0.0),
            ..Default::default()
        })
        .into()
}

fn grouping_switcher<'a>(active: SessionGrouping, palette: Palette) -> Element<'a, ShellMessage> {
    let mut segments = row![].spacing(2).width(Length::Fill);
    for grouping in SessionGrouping::ALL {
        let is_active = grouping == active;
        segments = segments.push(
            button(
                container(
                    text(grouping.label())
                        .size(theme::SIZE_SECONDARY)
                        .color(if is_active { palette.t1 } else { palette.t3 }),
                )
                .width(Length::Fill)
                .align_x(Alignment::Center),
            )
            .padding(Padding::from([4.0, 6.0]))
            .width(Length::Fill)
            .on_press(ShellMessage::SessionGroupingChanged(grouping))
            .style(move |_theme, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(
                        if is_active {
                            palette.ov(0.08)
                        } else if hovered {
                            palette.ov(0.04)
                        } else {
                            Color::TRANSPARENT
                        }
                        .into(),
                    ),
                    text_color: if is_active { palette.t1 } else { palette.t3 },
                    border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_CHIP),
                    ..Default::default()
                }
            }),
        );
    }

    container(segments)
        .padding(2.0)
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(palette.ov(0.03).into()),
            border: theme::border(palette.ov(0.06), 1.0, theme::RADIUS_ROW),
            ..Default::default()
        })
        .into()
}

fn group_header<'a>(key: &str, palette: Palette) -> Element<'a, ShellMessage> {
    text(key.to_uppercase())
        .size(theme::SIZE_GROUP_HEADER)
        .font(theme::ui(iced::font::Weight::Semibold))
        .color(palette.t4)
        .into()
}

fn session_row<'a>(entry: &'a SessionEntry, palette: Palette) -> Element<'a, ShellMessage> {
    let shell_color = palette.shell_color(entry.kind);
    let badge = container(
        text(entry.kind.abbreviation())
            .size(theme::SIZE_METADATA)
            .font(theme::mono(iced::font::Weight::Semibold))
            .color(shell_color),
    )
    .width(Length::Fixed(30.0))
    .height(Length::Fixed(24.0))
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .style(move |_theme| container::Style {
        background: Some(theme::with_alpha(shell_color, 0.1).into()),
        border: theme::border(theme::with_alpha(shell_color, 0.3), 1.0, 7.0),
        ..Default::default()
    });

    let name_and_meta = column![
        text(entry.name.clone())
            .size(theme::SIZE_BODY)
            .color(if entry.is_active {
                palette.t1
            } else {
                palette.t2
            }),
        text(entry.meta.clone())
            .size(theme::SIZE_METADATA)
            .font(theme::mono(iced::font::Weight::Normal))
            .color(palette.t4),
    ]
    .spacing(2)
    .width(Length::Fill);

    let dot_color = match entry.activity {
        SessionActivity::Running => palette.accent,
        SessionActivity::BusyAgent => palette.shell_ssh,
        SessionActivity::Idle => palette.ov(0.16),
    };
    let dot = container(
        Space::new()
            .width(Length::Fixed(6.0))
            .height(Length::Fixed(6.0)),
    )
    .style(move |_theme| container::Style {
        background: Some(dot_color.into()),
        border: theme::border(Color::TRANSPARENT, 0.0, 3.0),
        ..Default::default()
    });

    let content = row![badge, name_and_meta, dot]
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    let is_active = entry.is_active;
    let select = button(content)
        .padding(Padding::from([7.0, 8.0]))
        .width(Length::Fill)
        .on_press(ShellMessage::SessionSelected {
            workspace_id: entry.workspace_id.clone(),
            surface_id: entry.surface_id.clone(),
        })
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(
                    if is_active {
                        palette.accent_alpha(0.1)
                    } else if hovered {
                        palette.ov(0.05)
                    } else {
                        Color::TRANSPARENT
                    }
                    .into(),
                ),
                text_color: palette.t1,
                border: theme::border(
                    if is_active {
                        palette.accent_alpha(0.25)
                    } else {
                        Color::TRANSPARENT
                    },
                    1.0,
                    theme::RADIUS_ROW,
                ),
                ..Default::default()
            }
        });

    // A small X to close the session (removes its surface / pane / workspace via
    // the runtime cascade). Dim by default, brightening on hover so the row stays
    // calm until you reach for it.
    let close = button(text("\u{00d7}").size(theme::SIZE_BODY).color(palette.t4))
        .padding(Padding::from([3.0, 6.0]))
        .on_press(ShellMessage::SessionClosed {
            workspace_id: entry.workspace_id.clone(),
            surface_id: entry.surface_id.clone(),
        })
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: if hovered {
                    Some(palette.ov(0.08).into())
                } else {
                    None
                },
                text_color: if hovered { palette.t1 } else { palette.t4 },
                border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_ROW),
                ..Default::default()
            }
        });

    row![select, close]
        .spacing(2)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}

fn new_session_footer<'a>(palette: Palette) -> Element<'a, ShellMessage> {
    button(
        container(
            text("+ New session")
                .size(theme::SIZE_BODY)
                .color(palette.t3),
        )
        .width(Length::Fill)
        .align_x(Alignment::Center),
    )
    .padding(Padding::from([8.0, 8.0]))
    .width(Length::Fill)
    .on_press(ShellMessage::NewSessionRequested)
    .style(move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if hovered {
                Some(palette.ov(0.04).into())
            } else {
                None
            },
            text_color: palette.t3,
            border: theme::border(palette.ov(0.12), 1.0, theme::RADIUS_ROW),
            ..Default::default()
        }
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Accent, UiTheme};
    use pandamux_core::{AppIntent, PaneIntent, SplitDirection, SplitPaneParams, WorkspaceIntent};

    fn palette() -> Palette {
        Palette::new(UiTheme::Dark, Accent::Teal)
    }

    #[test]
    fn projects_one_session_per_terminal_surface() {
        let mut app = AppState::default();
        app.apply(AppIntent::Workspace(WorkspaceIntent::Create {
            title: Some("Agents".to_string()),
            shell: Some("wsl.exe".to_string()),
        }))
        .expect("create workspace");

        let sessions = project_sessions(&app, SessionGrouping::Project, true, None);
        assert_eq!(sessions.total, 2);
        // Two workspaces -> two Project groups.
        assert_eq!(sessions.groups.len(), 2);
    }

    #[test]
    fn groups_by_type_and_pins_active_first() {
        let mut app = AppState::default();
        app.apply(AppIntent::Workspace(WorkspaceIntent::Create {
            title: Some("Remote".to_string()),
            shell: Some("ssh chaz@galahad".to_string()),
        }))
        .expect("create workspace");
        // Active session is the ssh workspace's surface.
        let active = project_sessions(&app, SessionGrouping::Type, true, None);
        let ssh_surface = active
            .groups
            .iter()
            .flat_map(|group| &group.entries)
            .find(|entry| entry.kind == ShellKind::Ssh)
            .map(|entry| entry.surface_id.clone())
            .expect("ssh session");

        let sessions = project_sessions(&app, SessionGrouping::Type, true, Some(&ssh_surface));
        // Grouped by type: PS + SSH.
        assert_eq!(sessions.groups.len(), 2);
        // The active session's group (SSH) is pinned first.
        assert_eq!(sessions.groups[0].key, "SSH");
        assert!(
            sessions.groups[0]
                .entries
                .iter()
                .any(|entry| entry.is_active)
        );
    }

    #[test]
    fn disambiguates_multiple_terminals_in_one_workspace() {
        let mut app = AppState::default();
        app.apply(AppIntent::Pane(PaneIntent::Split(SplitPaneParams {
            workspace_id: None,
            target_pane_id: Some(pandamux_core::PaneId::from("pane-default")),
            target_surface_id: None,
            direction: SplitDirection::Horizontal,
            surface_type: SurfaceType::Terminal,
        })))
        .expect("split");

        let sessions = project_sessions(&app, SessionGrouping::Project, true, None);
        assert_eq!(sessions.total, 2);
        // Both terminals are in one project group, disambiguated by index.
        assert_eq!(sessions.groups.len(), 1);
        assert!(sessions.groups[0].entries[1].name.contains("\u{00b7} 2"));
    }

    #[test]
    fn builds_panel_view() {
        let app = AppState::default();
        let sessions = project_sessions(&app, SessionGrouping::Project, true, None);
        let _panel = session_panel(&sessions, palette());
    }
}
