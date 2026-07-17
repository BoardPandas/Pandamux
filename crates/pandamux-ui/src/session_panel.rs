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
use iced::widget::{Space, button, column, container, mouse_area, row, scrollable, text};
use iced::{Alignment, Color, Element, Length, Padding};
use pandamux_core::{
    AppState, ProjectId, ProjectLocation, SessionType, SplitNode, SshProfiles, SurfaceId,
    SurfaceRef, SurfaceType, WorkspaceId, WorkspaceState,
};
use std::collections::HashSet;

/// How the panel groups sessions. The Host group-by is gone (spec 2.4): hosts
/// stay visible as per-session badges but never affect grouping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SessionGrouping {
    #[default]
    Project,
    Type,
}

impl SessionGrouping {
    pub const ALL: [SessionGrouping; 2] = [SessionGrouping::Project, SessionGrouping::Type];

    pub fn label(self) -> &'static str {
        match self {
            SessionGrouping::Project => "Projects",
            SessionGrouping::Type => "Type",
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
    pub host: String,
    pub kind: ShellKind,
    /// What runs in the session (drives the Type grouping and the badge).
    pub session: SessionType,
    pub activity: SessionActivity,
    pub is_active: bool,
}

impl SessionEntry {
    /// The Type-grouping key and badge label: the tool for agent sessions,
    /// the shell flavor for plain terminals.
    pub fn type_label(&self) -> String {
        match &self.session {
            SessionType::Terminal => self.kind.abbreviation().to_string(),
            other => other.label().to_string(),
        }
    }

    fn badge(&self) -> String {
        match &self.session {
            SessionType::Terminal => self.kind.abbreviation().to_string(),
            SessionType::PowerShell { .. } => "PS".to_string(),
            SessionType::Claude => "CL".to_string(),
            SessionType::Codex => "CX".to_string(),
            SessionType::Gemini => "GM".to_string(),
            SessionType::Custom { .. } => "RUN".to_string(),
        }
    }
}

/// A named group of sessions (the group key depends on the active grouping).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionGroup {
    pub key: String,
    pub workspace_id: Option<WorkspaceId>,
    /// The registry identity behind a Projects-grouping header (context menu:
    /// rename / merge / close all).
    pub project_id: Option<ProjectId>,
    pub add_pending: bool,
    pub entries: Vec<SessionEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SessionsViewState {
    pub open: bool,
    pub grouping: SessionGrouping,
    /// Whether the Home dashboard view is active (highlights the switcher).
    pub home_active: bool,
    pub groups: Vec<SessionGroup>,
    pub total: usize,
    /// In-flight session rename: (surface, current input text).
    pub rename: Option<(SurfaceId, String)>,
    /// In-flight project rename: (project, current input text).
    pub project_rename: Option<(ProjectId, String)>,
}

/// Project canonical state into the session list. `active_surface_id` marks the
/// session currently focused in the active workspace.
pub fn project_sessions(
    app: &AppState,
    grouping: SessionGrouping,
    open: bool,
    active_surface_id: Option<&SurfaceId>,
) -> SessionsViewState {
    project_sessions_with_profiles(
        app,
        &SshProfiles::new(),
        grouping,
        open,
        active_surface_id,
        &HashSet::new(),
    )
}

pub fn project_sessions_with_profiles(
    app: &AppState,
    profiles: &SshProfiles,
    grouping: SessionGrouping,
    open: bool,
    active_surface_id: Option<&SurfaceId>,
    pending_projects: &HashSet<WorkspaceId>,
) -> SessionsViewState {
    let mut entries: Vec<SessionEntry> = Vec::new();
    for workspace in &app.workspaces {
        let terminals = terminal_surfaces(&workspace.split_tree);
        let multiple = terminals.len() > 1;
        for (index, surface) in terminals.into_iter().enumerate() {
            let surface_id = surface.id.clone();
            let session = surface.session.clone().unwrap_or_default();
            let (kind, host, meta) = match &workspace.project.location {
                ProjectLocation::Local { cwd, .. } => (
                    ShellKind::PowerShell,
                    "Local".to_string(),
                    format!("PowerShell \u{00b7} {cwd}"),
                ),
                ProjectLocation::Ssh {
                    profile_id,
                    remote_cwd,
                } => {
                    let profile = profiles.get(profile_id);
                    let name = profile
                        .map(|profile| profile.name.clone())
                        .unwrap_or_else(|| "Missing connection".to_string());
                    let host = profile
                        .map(|profile| profile.host.clone())
                        .unwrap_or_else(|| "Missing connection".to_string());
                    (
                        ShellKind::Ssh,
                        host,
                        format!("SSH \u{00b7} {name} \u{00b7} {remote_cwd}"),
                    )
                }
                ProjectLocation::Legacy => {
                    let host = host_label(&workspace.shell);
                    (
                        ShellKind::classify(&workspace.shell),
                        host.clone(),
                        format!("{} \u{00b7} {host}", workspace.shell),
                    )
                }
            };
            // Naming (spec 2.1): a user-set name always wins; otherwise
            // "<Type> · <project>" for agent sessions, "<project>" for plain
            // terminals, with an index when the project has several.
            let base = match &session {
                SessionType::Terminal => workspace.title.clone(),
                other => format!("{} \u{00b7} {}", other.label(), workspace.title),
            };
            let derived = if multiple {
                format!("{base} \u{00b7} {}", index + 1)
            } else {
                base
            };
            let name = surface.name.clone().unwrap_or(derived);
            entries.push(SessionEntry {
                is_active: active_surface_id == Some(&surface_id),
                surface_id,
                workspace_id: workspace.id.clone(),
                name,
                meta,
                host,
                kind,
                session,
                activity: activity_for(workspace, active_surface_id),
            });
        }
    }

    let total = entries.len();
    let groups = group_entries(entries, grouping, app, pending_projects);
    SessionsViewState {
        open,
        grouping,
        home_active: false,
        groups,
        total,
        rename: None,
        project_rename: None,
    }
}

fn group_entries(
    entries: Vec<SessionEntry>,
    grouping: SessionGrouping,
    app: &AppState,
    pending_projects: &HashSet<WorkspaceId>,
) -> Vec<SessionGroup> {
    let mut groups: Vec<SessionGroup> = Vec::new();
    for entry in entries {
        // Projects grouping keys on the registry identity (spec 1.4): the same
        // project reached from any host or transport lands in ONE group. A
        // legacy workspace without an identity groups by itself.
        let workspace = app.workspace(&entry.workspace_id);
        let project_id = workspace.and_then(|workspace| workspace.project_id.clone());
        let (key, group_workspace_id, group_project_id) = match grouping {
            SessionGrouping::Project => {
                let record = project_id
                    .as_ref()
                    .and_then(|id| app.projects.iter().find(|record| &record.id == id));
                let key = record
                    .map(|record| record.name.clone())
                    .or_else(|| workspace.map(|workspace| workspace.title.clone()))
                    .unwrap_or_else(|| "Workspace".to_string());
                (key, Some(entry.workspace_id.clone()), project_id.clone())
            }
            SessionGrouping::Type => (entry.type_label(), None, None),
        };
        let same_group = |group: &&mut SessionGroup| match grouping {
            SessionGrouping::Project => {
                if group.project_id.is_some() || group_project_id.is_some() {
                    group.project_id == group_project_id
                } else {
                    group.workspace_id == group_workspace_id
                }
            }
            SessionGrouping::Type => group.key == key,
        };
        match groups.iter_mut().find(same_group) {
            Some(group) => group.entries.push(entry),
            None => groups.push(SessionGroup {
                key,
                add_pending: group_workspace_id
                    .as_ref()
                    .is_some_and(|id| pending_projects.contains(id)),
                workspace_id: group_workspace_id,
                project_id: group_project_id,
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
    let is_active_workspace = active_surface_id.is_some_and(|surface_id| {
        terminal_surfaces(&workspace.split_tree)
            .iter()
            .any(|surface| &surface.id == surface_id)
    });
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

/// Every terminal surface in the tree, depth-first. Exported so keyboard
/// cycling (spec 2.6) uses exactly the visible panel order.
pub fn terminal_surfaces(tree: &SplitNode) -> Vec<SurfaceRef> {
    match tree {
        SplitNode::Leaf(leaf) => leaf
            .surfaces
            .iter()
            .filter(|surface| surface.surface_type == SurfaceType::Terminal)
            .cloned()
            .collect(),
        SplitNode::Branch(branch) => {
            let mut surfaces = terminal_surfaces(&branch.children[0]);
            surfaces.extend(terminal_surfaces(&branch.children[1]));
            surfaces
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

    let switcher = grouping_switcher(state.grouping, state.home_active, palette);

    let mut list = column![].spacing(10).width(Length::Fill);
    for group in &state.groups {
        list = list.push(group_header(group, state, palette));
        for entry in &group.entries {
            list = list.push(session_row(entry, state, palette));
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

/// The Home | Projects | Type view switcher (spec 2.4). Home switches the
/// main area to the dashboard; Projects/Type regroup the session list (and
/// return to the workspace view).
fn grouping_switcher<'a>(
    active: SessionGrouping,
    home_active: bool,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let mut segments = row![].spacing(2).width(Length::Fill);
    let segment = |label: &'static str, is_active: bool, message: ShellMessage| {
        button(
            container(text(label).size(theme::SIZE_SECONDARY).color(if is_active {
                palette.t1
            } else {
                palette.t3
            }))
            .width(Length::Fill)
            .align_x(Alignment::Center),
        )
        .padding(Padding::from([4.0, 6.0]))
        .width(Length::Fill)
        .on_press(message)
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
        })
    };
    segments = segments.push(segment("Home", home_active, ShellMessage::HomeRequested));
    for grouping in SessionGrouping::ALL {
        segments = segments.push(segment(
            grouping.label(),
            !home_active && grouping == active,
            ShellMessage::SessionGroupingChanged(grouping),
        ));
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

fn group_header<'a>(
    group: &'a SessionGroup,
    state: &'a SessionsViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    // Inline project rename (spec 1.4 manual path).
    let renaming = group
        .project_id
        .as_ref()
        .zip(state.project_rename.as_ref())
        .is_some_and(|(id, (renaming_id, _))| id == renaming_id);
    let label: Element<'a, ShellMessage> = if renaming {
        let value = state
            .project_rename
            .as_ref()
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        iced::widget::text_input("Project name", value)
            .on_input(ShellMessage::ProjectRenameEdited)
            .on_submit(ShellMessage::ProjectRenameCommitted)
            .size(theme::SIZE_SECONDARY)
            .width(Length::Fill)
            .into()
    } else {
        text(group.key.to_uppercase())
            .size(theme::SIZE_GROUP_HEADER)
            .font(theme::ui(iced::font::Weight::Semibold))
            .color(palette.t4)
            .into()
    };
    let mut header = row![label, Space::new().width(Length::Fill)].align_y(Alignment::Center);
    if let Some(workspace_id) = &group.workspace_id {
        let mut add = button(
            text(if group.add_pending { "..." } else { "+" })
                .size(theme::SIZE_BODY)
                .color(palette.t3),
        )
        .padding(Padding::from([1.0, 6.0]))
        .style(move |_theme, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then(|| palette.ov(0.08).into()),
            text_color: palette.t3,
            border: theme::border(palette.ov(0.1), 1.0, theme::RADIUS_CHIP),
            ..Default::default()
        });
        if !group.add_pending {
            add = add.on_press(ShellMessage::ProjectSessionRequested(workspace_id.clone()));
        }
        header = header.push(add);
    }
    // Right-click opens the project actions menu (rename / merge / close all).
    if group.project_id.is_some() {
        let project_id = group.project_id.clone();
        return mouse_area(header)
            .on_right_press(ShellMessage::ProjectContextRequested(
                project_id.expect("checked above"),
            ))
            .into();
    }
    header.into()
}

fn session_row<'a>(
    entry: &'a SessionEntry,
    state: &'a SessionsViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let shell_color = palette.shell_color(entry.kind);
    let badge = container(
        text(entry.badge())
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

    // Inline rename (spec 2.1): the row being renamed swaps its name for an
    // input; Enter commits, Esc (OverlayDismissed) cancels.
    let renaming = state
        .rename
        .as_ref()
        .is_some_and(|(surface_id, _)| surface_id == &entry.surface_id);
    let name_element: Element<'a, ShellMessage> = if renaming {
        let value = state
            .rename
            .as_ref()
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        iced::widget::text_input("Session name", value)
            .on_input(ShellMessage::SessionRenameEdited)
            .on_submit(ShellMessage::SessionRenameCommitted)
            .size(theme::SIZE_BODY)
            .width(Length::Fill)
            .into()
    } else {
        text(entry.name.clone())
            .size(theme::SIZE_BODY)
            .color(if entry.is_active {
                palette.t1
            } else {
                palette.t2
            })
            .into()
    };
    let name_and_meta = column![
        name_element,
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

    // Right-click opens the session actions menu (rename / detach / close).
    mouse_area(
        row![select, close]
            .spacing(2)
            .align_y(Alignment::Center)
            .width(Length::Fill),
    )
    .on_right_press(ShellMessage::SessionContextRequested {
        workspace_id: entry.workspace_id.clone(),
        surface_id: entry.surface_id.clone(),
    })
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
