//! The native Iced shell view.
//!
//! [`app_view`] composes the full chrome (titlebar, icon rail, styled pane
//! workspace, status bar) from a [`ShellViewModel`]. [`shell_view`] renders just
//! the pane workspace and is used by the headless smoke path and unit tests.
//!
//! This layer is a read-projection plus an intent source: every interaction maps
//! to a [`ShellMessage`] that the runtime routes into core intents or window
//! actions. It never owns canonical state.

use crate::chrome::{self, ChromeState, Overlay, RailItem};
use crate::command_palette::{self, PaletteViewState, QuickLaunchViewState};
use crate::icons::{Icon, icon};
use crate::overlays;
use crate::session_panel::{self, SessionGrouping, SessionsViewState};
use crate::settings::{self, SettingsSection, SettingsViewState};
use crate::shell_projection::{ColumnProjection, PaneProjection, SurfaceProjection};
use crate::theme::{self, Accent, Palette, ShellKind, TermScheme};
use iced::widget::{Space, button, canvas, column, container, mouse_area, row, stack, text};
use iced::{
    Alignment, Color, Element, Length, Padding, Pixels, Point, Rectangle, Renderer, Size, Theme,
    mouse,
};
use pandamux_core::{DropZone, PaneId, SplitDirection, SurfaceId, SurfaceType, WorkspaceId};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellMessage {
    Tick,
    // Pane / surface intents
    PaneFocused(PaneId),
    PaneSplit {
        pane_id: PaneId,
        direction: SplitDirection,
    },
    PaneClosed(PaneId),
    PaneZoomToggled(PaneId),
    /// Keyboard-driven variants that act on the focused pane (the runtime
    /// resolves it, since a keystroke carries no pane id).
    SplitFocused(SplitDirection),
    CloseFocusedPane,
    ZoomFocusedPane,
    TerminalSurfaceCreated(PaneId),
    SurfaceFocused(SurfaceId),
    SurfaceClosed(SurfaceId),
    // Drag-and-drop pane splitting (Section 12.3)
    /// A tab press armed a potential drag of `surface_id` from `pane_id`. A
    /// release without ever entering a drop zone is treated as a plain click
    /// (focus); a release over a zone moves/splits.
    TabDragArmed {
        surface_id: SurfaceId,
        pane_id: PaneId,
    },
    /// The pointer moved while a drag was armed (confirms a real drag vs a click).
    DragMoved,
    /// The drag pointer entered a pane's drop zone.
    DragOverZone {
        pane_id: PaneId,
        zone: DropZone,
    },
    /// The drag pointer was released anywhere (drop if over a zone, else focus).
    DragReleased,
    // Chrome / window
    WindowDragStarted,
    WindowMinimizePressed,
    WindowMaximizeToggled,
    WindowClosePressed,
    RailSelected(RailItem),
    /// An overlay (palette, notifications, settings, quick-launch) was requested.
    /// Overlays land in Phases 4-5; the runtime records the request for now.
    OverlayRequested(RailItem),
    // Session panel
    /// Focus/activate a session (a shell context). Selects its workspace and
    /// focuses its surface; never swaps the layout (plan Section 12.1 #2).
    SessionSelected {
        workspace_id: WorkspaceId,
        surface_id: SurfaceId,
    },
    SessionGroupingChanged(SessionGrouping),
    NewSessionRequested,
    // Overlays (command palette / quick-launch / settings)
    /// Dismiss whatever centered overlay is open (backdrop click / Esc).
    OverlayDismissed,
    PaletteQueryChanged(String),
    /// Move the palette selection by a delta (arrow keys).
    PaletteMoveSelection(i32),
    /// Activate the highlighted palette item (Enter).
    PaletteActivate,
    /// Launch a new session from a quick-launch profile.
    LaunchProfile {
        shell: String,
        title: String,
    },
    SettingsSectionSelected(SettingsSection),
    AccentSelected(Accent),
    // Status-bar pollers (git / ports)
    /// Timer tick asking the runtime to kick off a background poll.
    PollRequested,
    /// A completed poll's results (git branch/ahead + listening ports).
    PollUpdate {
        git_branch: Option<String>,
        git_ahead: u32,
        ports: Vec<u16>,
    },
    // In-app update check (Phase 7)
    /// Timer tick / launch asking the runtime to check GitHub for a newer release.
    UpdateCheckRequested,
    /// A newer release was found (past the quarantine window); the runtime raises
    /// an update toast. The download-and-run-installer step is wired with
    /// packaging.
    UpdateAvailable {
        version: String,
        tag: String,
        url: Option<String>,
        notes: String,
    },
    ToggleStatusBar,
    ToggleTheme,
    CycleAccent,
    // Find-in-terminal
    FindOpened,
    FindClosed,
    FindQueryChanged(String),
    FindNext,
    FindPrev,
    FindCaseToggled,
    // Copy mode
    CopyModeToggled,
    // Notifications
    NotificationsToggled,
    NotificationCleared(String),
    NotificationsClearedAll,
    /// A line arrived on the named pipe (CLI / agents / orchestrator). The
    /// runtime dispatches `payload` against canonical state on the single-writer
    /// path and completes the reply keyed by `id`. This is what makes a
    /// CLI-driven `split`/`notify`/`read-screen` reach the live UI.
    PipeRequest {
        id: u64,
        payload: String,
    },
    /// Raw bytes decoded from a key press, to be written to the focused
    /// terminal's PTY (or SSH channel). Suppressed while a centered overlay is
    /// open, since the overlay's own text inputs consume typing.
    TerminalInput(Vec<u8>),
    /// No-op (e.g. an unmapped key press); ignored by the runtime.
    Noop,
}

/// Live drag-and-drop state (plan Section 12.3). Present only while a tab is
/// being dragged; drives the dimmed source tab and the drop-zone overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragView {
    pub surface_id: SurfaceId,
    pub source_pane_id: PaneId,
    /// The pane + zone the pointer is currently over, if any.
    pub over: Option<(PaneId, DropZone)>,
    /// `true` once the pointer has moved after the press (the drag is a real drag,
    /// not a click). Drop zones only render and register while active, so a
    /// stationary press-release stays a plain click (focus), standing in for the
    /// design's 6px drag threshold.
    pub active: bool,
}

/// A detected-link span on a visible row (character offsets), for underlining.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub surface_id: SurfaceId,
    pub lines: Vec<String>,
    pub columns: usize,
    pub rows: usize,
    /// Detected link spans on the visible screen (underlined by the viewport).
    pub links: Vec<LinkSpan>,
    /// The SSH host this surface is connected to, if it is a remote surface
    /// (plan F2). Drives the SSH context chip on the pane.
    pub remote_host: Option<String>,
}

impl TerminalSnapshot {
    /// Convenience constructor for snapshots without links (tests, placeholders).
    pub fn new(surface_id: SurfaceId, lines: Vec<String>, columns: usize, rows: usize) -> Self {
        Self {
            surface_id,
            lines,
            columns,
            rows,
            links: Vec::new(),
            remote_host: None,
        }
    }

    /// Mark this snapshot as an SSH remote surface connected to `host`.
    pub fn with_remote_host(mut self, host: impl Into<String>) -> Self {
        self.remote_host = Some(host.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShellViewModel {
    pub projection: crate::shell_projection::ShellProjection,
    pub terminals: Vec<TerminalSnapshot>,
    pub chrome: ChromeState,
    /// Blink phase for the focused pane's block cursor (~1.1s cadence).
    pub cursor_on: bool,
    /// Find-in-terminal overlay state.
    pub find: crate::overlays::FindViewState,
    /// Notifications slide-over state.
    pub notifications: crate::overlays::NotificationsViewState,
    /// Whether copy mode is active on the focused pane.
    pub copy_mode: bool,
    /// Session-panel projection (sessions across all workspaces).
    pub sessions: SessionsViewState,
    /// Command-palette state (filtered items live here).
    pub palette: PaletteViewState,
    /// Quick-launch profile list.
    pub quick_launch: QuickLaunchViewState,
    /// Settings modal projection.
    pub settings: SettingsViewState,
    /// Markdown/diff content keyed by surface id, rendered by non-terminal panes.
    pub surface_contents: HashMap<SurfaceId, String>,
    /// Active drag-and-drop, if a tab is being dragged.
    pub drag: Option<DragView>,
    /// Terminal color scheme derived from the selected theme (or default dark).
    pub term_scheme: TermScheme,
    /// Per-surface terminal color-scheme overrides (resolved from `set-color-scheme`).
    pub surface_term_schemes: HashMap<SurfaceId, TermScheme>,
}

// ---------------------------------------------------------------------------
// Terminal viewport (fixed-dark scheme + block cursor)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TerminalViewport {
    lines: Vec<String>,
    columns: usize,
    rows: usize,
    show_cursor: bool,
    links: Vec<LinkSpan>,
    highlight: Option<LinkSpan>,
    scheme: TermScheme,
}

impl TerminalViewport {
    pub fn new(lines: Vec<String>, columns: usize, rows: usize) -> Self {
        Self {
            lines,
            columns,
            rows,
            show_cursor: false,
            links: Vec::new(),
            highlight: None,
            scheme: TermScheme::default(),
        }
    }

    pub fn with_cursor(mut self, show_cursor: bool) -> Self {
        self.show_cursor = show_cursor;
        self
    }

    pub fn with_scheme(mut self, scheme: TermScheme) -> Self {
        self.scheme = scheme;
        self
    }

    pub fn with_links(mut self, links: Vec<LinkSpan>) -> Self {
        self.links = links;
        self
    }

    pub fn with_highlight(mut self, highlight: Option<LinkSpan>) -> Self {
        self.highlight = highlight;
        self
    }

    /// Column of the block cursor: just past the last non-empty line's content.
    fn cursor_cell(&self) -> (usize, usize) {
        let last_row = self
            .lines
            .iter()
            .take(self.rows)
            .rposition(|line| !line.trim_end().is_empty())
            .unwrap_or(0);
        let col = self
            .lines
            .get(last_row)
            .map(|line| line.trim_end().chars().count())
            .unwrap_or(0);
        (last_row, col.min(self.columns.saturating_sub(1)))
    }
}

impl<Message> canvas::Program<Message> for TerminalViewport {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let background = canvas::Path::rectangle(Point::ORIGIN, bounds.size());
        frame.fill(&background, self.scheme.background);

        let pad = theme::TERMINAL_PADDING;
        let cell_h = theme::term::CELL_HEIGHT;
        let cell_w = theme::term::CELL_WIDTH;

        // Current find match highlight (drawn behind the text).
        if let Some(span) = self.highlight
            && span.line < self.rows
            && span.end > span.start
        {
            let x = pad + span.start as f32 * cell_w;
            let width = (span.end - span.start) as f32 * cell_w;
            let y = pad + span.line as f32 * cell_h;
            let rect = canvas::Path::rectangle(Point::new(x, y), Size::new(width.max(0.0), cell_h));
            frame.fill(&rect, theme::with_alpha(theme::Accent::Gold.color(), 0.35));
        }

        for (row_index, line) in self.lines.iter().take(self.rows).enumerate() {
            frame.fill_text(canvas::Text {
                content: line.clone(),
                position: Point::new(pad, pad + row_index as f32 * cell_h),
                max_width: (bounds.width - pad * 2.0).max(0.0),
                color: self.scheme.text,
                size: Pixels(theme::SIZE_TERMINAL),
                line_height: iced::widget::text::LineHeight::Absolute(Pixels(cell_h)),
                font: theme::MONO_FONT,
                shaping: iced::widget::text::Shaping::Advanced,
                ..canvas::Text::default()
            });
        }

        // Underline detected links (accent, 1px) beneath their character span.
        for link in self.links.iter().take(256) {
            if link.line >= self.rows || link.end <= link.start {
                continue;
            }
            let x = pad + link.start as f32 * cell_w;
            let width = (link.end - link.start) as f32 * cell_w;
            let y = pad + link.line as f32 * cell_h + cell_h - 2.0;
            let underline =
                canvas::Path::rectangle(Point::new(x, y), Size::new(width.max(0.0), 1.0));
            frame.fill(&underline, theme::Accent::Teal.color());
        }

        if self.show_cursor {
            let (crow, ccol) = self.cursor_cell();
            let cursor_x = pad + ccol as f32 * cell_w;
            let cursor_y = pad + crow as f32 * cell_h + (cell_h - theme::term::CURSOR_HEIGHT) / 2.0;
            let cursor = canvas::Path::rectangle(
                Point::new(cursor_x, cursor_y),
                Size::new(theme::term::CURSOR_WIDTH, theme::term::CURSOR_HEIGHT),
            );
            // Cursor uses the active scheme's prompt/cursor color.
            frame.fill(&cursor, self.scheme.cursor);
        }

        vec![frame.into_geometry()]
    }
}

pub fn terminal_viewport<'a, Message: 'a>(
    lines: Vec<String>,
    columns: usize,
    rows: usize,
) -> Element<'a, Message> {
    canvas::Canvas::new(TerminalViewport::new(lines, columns, rows))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// ---------------------------------------------------------------------------
// Full app composition
// ---------------------------------------------------------------------------

/// The complete chrome: titlebar, icon rail + pane workspace, optional status bar.
pub fn app_view(model: &ShellViewModel) -> Element<'_, ShellMessage> {
    let palette = model.chrome.palette();

    let mut body = row![chrome::icon_rail(&model.chrome, palette)]
        .width(Length::Fill)
        .height(Length::Fill);
    if model.sessions.open {
        body = body.push(session_panel::session_panel(&model.sessions, palette));
    }
    body = body.push(workspace_view(model, palette));

    let mut root = column![chrome::titlebar(&model.chrome, palette), body]
        .width(Length::Fill)
        .height(Length::Fill);

    if model.chrome.show_status_bar {
        root = root.push(chrome::status_bar(&model.chrome, palette));
    }

    let base = container(root)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(palette.bg_gradient()),
            ..Default::default()
        });

    // Layer overlays on top of the base: the notifications slide-over (a right
    // side panel) and then the active centered overlay (palette / quick-launch /
    // settings), so a modal sits above the notifications panel if both are open.
    let mut layers = stack![base];

    if model.notifications.open {
        let notifications = container(overlays::notifications_panel(&model.notifications, palette))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::End)
            .align_y(Alignment::Start)
            .padding(Padding {
                top: theme::TITLEBAR_HEIGHT + 8.0,
                right: 8.0,
                bottom: 8.0,
                left: 8.0,
            });
        layers = layers.push(notifications);
    }

    match model.chrome.active_overlay {
        Overlay::None => {}
        Overlay::CommandPalette => {
            layers = layers.push(command_palette::command_palette(&model.palette, palette));
        }
        Overlay::QuickLaunch => {
            layers = layers.push(command_palette::quick_launch(&model.quick_launch, palette));
        }
        Overlay::Settings => {
            layers = layers.push(settings::settings_modal(&model.settings, palette));
        }
    }

    // A root release handler completes any in-flight drag (drop or focus). It is
    // a no-op when nothing is being dragged. While a drag is armed, a root
    // `on_move` confirms it (the movement gate) so a stationary click stays a
    // click; the handler is omitted otherwise to avoid a message per mouse move.
    let root = mouse_area(layers).on_release(ShellMessage::DragReleased);
    let root = if model.drag.is_some() {
        root.on_move(|_point| ShellMessage::DragMoved)
    } else {
        root
    };
    root.into()
}

/// The pane workspace only (used by tests and the headless smoke path).
pub fn shell_view(model: &ShellViewModel) -> Element<'_, ShellMessage> {
    workspace_view(model, model.chrome.palette())
}

fn workspace_view<'a>(model: &'a ShellViewModel, palette: Palette) -> Element<'a, ShellMessage> {
    let focused = model.projection.focused_pane_id.as_ref();
    let find_highlight = if model.find.open {
        model.find.current_match
    } else {
        None
    };
    let mut columns = row![].spacing(theme::PANE_GAP);
    for col in &model.projection.columns {
        columns = columns.push(column_view(
            col,
            &model.terminals,
            &model.surface_contents,
            &model.surface_term_schemes,
            model.drag.as_ref(),
            model.term_scheme,
            palette,
            focused,
            model.cursor_on,
            find_highlight,
        ));
    }

    let mut stacked = column![].spacing(theme::PANE_GAP);
    if model.find.open {
        stacked = stacked.push(overlays::find_bar(&model.find, palette));
    }
    if model.copy_mode {
        stacked = stacked.push(overlays::copy_mode_indicator(palette));
    }
    stacked = stacked.push(columns.width(Length::Fill).height(Length::Fill));

    container(stacked.width(Length::Fill).height(Length::Fill))
        .padding(theme::WORKSPACE_PADDING)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[allow(clippy::too_many_arguments)]
fn column_view<'a>(
    col: &'a ColumnProjection,
    terminals: &'a [TerminalSnapshot],
    surface_contents: &'a HashMap<SurfaceId, String>,
    surface_schemes: &'a HashMap<SurfaceId, TermScheme>,
    drag: Option<&'a DragView>,
    term_scheme: TermScheme,
    palette: Palette,
    focused: Option<&PaneId>,
    cursor_on: bool,
    find_highlight: Option<(usize, usize, usize)>,
) -> Element<'a, ShellMessage> {
    let mut stacked = column![].spacing(theme::PANE_GAP);
    for pane in &col.panes {
        stacked = stacked.push(pane_view(
            pane,
            terminals,
            surface_contents,
            surface_schemes,
            drag,
            term_scheme,
            palette,
            focused,
            cursor_on,
            find_highlight,
        ));
    }
    stacked.width(Length::Fill).height(Length::Fill).into()
}

#[allow(clippy::too_many_arguments)]
fn pane_view<'a>(
    pane: &'a PaneProjection,
    terminals: &'a [TerminalSnapshot],
    surface_contents: &'a HashMap<SurfaceId, String>,
    surface_schemes: &'a HashMap<SurfaceId, TermScheme>,
    drag: Option<&'a DragView>,
    term_scheme: TermScheme,
    palette: Palette,
    focused: Option<&PaneId>,
    cursor_on: bool,
    find_highlight: Option<(usize, usize, usize)>,
) -> Element<'a, ShellMessage> {
    let is_focused = focused == Some(&pane.id);
    let highlight = if is_focused {
        find_highlight.map(|(line, start, end)| LinkSpan { line, start, end })
    } else {
        None
    };

    let dragged_surface = drag.map(|drag| &drag.surface_id);
    let tab_bar = tab_bar_view(pane, palette, dragged_surface);

    // Non-terminal surfaces (markdown / diff) render their stored content; a
    // terminal surface (or an unknown/empty pane) renders the canvas viewport.
    let active_surface = pane.surfaces.iter().find(|surface| surface.is_active);
    // Per-surface color-scheme override (set-color-scheme), else the global scheme.
    let scheme = active_surface
        .and_then(|surface| surface_schemes.get(&surface.id))
        .copied()
        .unwrap_or(term_scheme);
    let body: Element<'a, ShellMessage> = match active_surface.map(|surface| &surface.surface_type)
    {
        Some(SurfaceType::Markdown) => {
            let content = active_surface
                .and_then(|surface| surface_contents.get(&surface.id))
                .map(String::as_str)
                .unwrap_or("");
            crate::content_views::markdown_view(content, palette)
        }
        Some(SurfaceType::Diff) => {
            let content = active_surface
                .and_then(|surface| surface_contents.get(&surface.id))
                .map(String::as_str)
                .unwrap_or("");
            crate::content_views::diff_view(content, palette)
        }
        _ => {
            let active_terminal = pane
                .active_surface_id
                .as_ref()
                .and_then(|surface_id| terminal_snapshot(terminals, surface_id));
            match active_terminal {
                Some(snapshot) => canvas::Canvas::new(
                    TerminalViewport::new(snapshot.lines.clone(), snapshot.columns, snapshot.rows)
                        .with_cursor(is_focused && cursor_on)
                        .with_links(snapshot.links.clone())
                        .with_highlight(highlight)
                        .with_scheme(scheme),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
                None => canvas::Canvas::new(
                    TerminalViewport::new(vec![placeholder_line(pane)], 80, 24)
                        .with_cursor(is_focused && cursor_on)
                        .with_scheme(scheme),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            }
        }
    };

    // SSH context chip for a remote surface (plan F2): a slim accent bar naming
    // the host, between the tab bar and the terminal body.
    let remote_host = pane
        .active_surface_id
        .as_ref()
        .and_then(|surface_id| terminal_snapshot(terminals, surface_id))
        .and_then(|snapshot| snapshot.remote_host.clone());
    let contents = match remote_host {
        Some(host) => column![tab_bar, ssh_context_chip(&host, palette), body],
        None => column![tab_bar, body],
    }
    .width(Length::Fill)
    .height(Length::Fill);

    let pane_box = container(contents)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(move |_theme| pane_style(palette, is_focused));

    // While a drag is in flight, overlay the drop zones on top of the pane so the
    // pointer's enter events select a zone; otherwise the pane is a click-to-focus
    // target.
    match drag {
        // Overlay drop zones only once the drag is active (moved), so a plain tab
        // click never registers a zone.
        Some(drag) if drag.active => {
            let active_zone = match &drag.over {
                Some((pane_id, zone)) if pane_id == &pane.id => Some(*zone),
                _ => None,
            };
            stack![pane_box, drop_zone_overlay(&pane.id, active_zone, palette)].into()
        }
        _ => mouse_area(pane_box)
            .on_press(ShellMessage::PaneFocused(pane.id.clone()))
            .into(),
    }
}

/// A slim SSH context chip naming the remote host for a remote surface (plan
/// F2), shown under the tab bar.
fn ssh_context_chip<'a>(host: &str, palette: Palette) -> Element<'a, ShellMessage> {
    let label = text(format!("SSH  {host}"))
        .size(theme::SIZE_METADATA)
        .color(palette.accent);
    container(label)
        .padding(Padding::from([2.0, 8.0]))
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(palette.accent_alpha(0.12).into()),
            border: theme::border(palette.accent_alpha(0.25), 0.0, 0.0),
            ..Default::default()
        })
        .into()
}

fn tab_bar_view<'a>(
    pane: &'a PaneProjection,
    palette: Palette,
    dragged_surface: Option<&SurfaceId>,
) -> Element<'a, ShellMessage> {
    let mut tabs = row![].spacing(4).align_y(Alignment::Center);
    for surface in &pane.surfaces {
        let is_dragging = dragged_surface == Some(&surface.id);
        tabs = tabs.push(tab_view(surface, &pane.id, palette, is_dragging));
    }

    let add_tab = icon_button(
        Icon::Plus,
        palette,
        ShellMessage::TerminalSurfaceCreated(pane.id.clone()),
    );
    let split_right = icon_button(
        Icon::SplitRight,
        palette,
        ShellMessage::PaneSplit {
            pane_id: pane.id.clone(),
            direction: SplitDirection::Horizontal,
        },
    );
    let split_down = icon_button(
        Icon::SplitDown,
        palette,
        ShellMessage::PaneSplit {
            pane_id: pane.id.clone(),
            direction: SplitDirection::Vertical,
        },
    );
    let zoom = icon_button(
        if pane.is_zoomed {
            Icon::ZoomOut
        } else {
            Icon::ZoomIn
        },
        palette,
        ShellMessage::PaneZoomToggled(pane.id.clone()),
    );

    let bar = row![
        tabs,
        add_tab,
        Space::new().width(Length::Fill),
        split_right,
        split_down,
        zoom,
    ]
    .spacing(4)
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .height(Length::Fixed(theme::TAB_BAR_HEIGHT));

    container(bar)
        .padding(Padding::from([0.0, 8.0]))
        .width(Length::Fill)
        .height(Length::Fixed(theme::TAB_BAR_HEIGHT))
        .style(move |_theme| container::Style {
            background: Some(palette.ov(0.02).into()),
            border: theme::border(palette.ov(0.05), 0.0, 0.0),
            ..Default::default()
        })
        .into()
}

fn tab_view<'a>(
    surface: &'a SurfaceProjection,
    pane_id: &PaneId,
    palette: Palette,
    is_dragging: bool,
) -> Element<'a, ShellMessage> {
    let kind = surface_shell_kind(&surface.surface_type);
    // The dragged tab dims to ~35% (plan Section 12.3): a plain click focuses;
    // a press-drag-release over a pane moves. Both start from this same press.
    let shell_color = if is_dragging {
        theme::with_alpha(palette.shell_color(kind), 0.35)
    } else {
        palette.shell_color(kind)
    };
    let is_active = surface.is_active;
    let label_color = if is_dragging {
        palette.t4
    } else if is_active {
        palette.t1
    } else {
        palette.t3
    };

    let label = row![
        text(kind.glyph())
            .size(theme::SIZE_METADATA)
            .font(theme::mono(iced::font::Weight::Semibold))
            .color(shell_color),
        text(surface_type_label(&surface.surface_type))
            .size(theme::SIZE_BODY)
            .color(label_color),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let focus_button = button(label)
        .padding(Padding::from([5.0, 8.0]))
        .on_press(ShellMessage::TabDragArmed {
            surface_id: surface.id.clone(),
            pane_id: pane_id.clone(),
        })
        .style(move |_theme, status| tab_style(palette, is_active, status));

    let close = button(text("\u{00d7}").size(theme::SIZE_BODY).color(palette.t3))
        .padding(Padding::from([3.0, 5.0]))
        .on_press(ShellMessage::SurfaceClosed(surface.id.clone()))
        .style(move |_theme, status| ghost_button_style(palette, status));

    let underline_color = if is_active {
        palette.accent
    } else {
        Color::TRANSPARENT
    };
    let underline = container(Space::new().height(Length::Fixed(2.0)).width(Length::Fill)).style(
        move |_theme| container::Style {
            background: Some(underline_color.into()),
            ..Default::default()
        },
    );

    column![
        row![focus_button, close]
            .spacing(0)
            .align_y(Alignment::Center),
        underline,
    ]
    .spacing(2)
    .into()
}

fn icon_button<'a>(
    kind: Icon,
    palette: Palette,
    message: ShellMessage,
) -> Element<'a, ShellMessage> {
    button(
        container(icon(kind, 14.0, palette.t3))
            .width(Length::Fixed(22.0))
            .height(Length::Fixed(22.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    )
    .padding(0.0)
    .on_press(message)
    .style(move |_theme, status| ghost_button_style(palette, status))
    .into()
}

// ---------------------------------------------------------------------------
// Drag-and-drop drop zones (Section 12.3)
// ---------------------------------------------------------------------------

/// The five drop zones overlaid on a pane during a drag. Left/right are
/// full-height 25% strips (so they win over top/bottom near the corners, per the
/// spec's `x<0.25` / `x>0.75` precedence); the central 50% column carries
/// top (30%) / center (40%) / bottom (30%). Each region reports `on_enter` so the
/// runtime tracks the hovered zone; the active one is highlighted.
fn drop_zone_overlay<'a>(
    pane_id: &PaneId,
    active: Option<DropZone>,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let middle = column![
        zone_region(
            pane_id,
            DropZone::Top,
            active,
            palette,
            Length::FillPortion(30)
        ),
        zone_region(
            pane_id,
            DropZone::Center,
            active,
            palette,
            Length::FillPortion(40)
        ),
        zone_region(
            pane_id,
            DropZone::Bottom,
            active,
            palette,
            Length::FillPortion(30)
        ),
    ]
    .width(Length::FillPortion(50))
    .height(Length::Fill);

    row![
        wrap_width(
            zone_region(pane_id, DropZone::Left, active, palette, Length::Fill),
            Length::FillPortion(25)
        ),
        middle,
        wrap_width(
            zone_region(pane_id, DropZone::Right, active, palette, Length::Fill),
            Length::FillPortion(25)
        ),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn wrap_width<'a>(element: Element<'a, ShellMessage>, width: Length) -> Element<'a, ShellMessage> {
    container(element).width(width).height(Length::Fill).into()
}

fn zone_region<'a>(
    pane_id: &PaneId,
    zone: DropZone,
    active: Option<DropZone>,
    palette: Palette,
    height: Length,
) -> Element<'a, ShellMessage> {
    let is_active = active == Some(zone);
    // A 3px inset so the highlight reads as a card within the pane.
    let inner = container(Space::new().width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| drop_zone_style(palette, is_active));
    let padded = container(inner)
        .padding(3.0)
        .width(Length::Fill)
        .height(height);
    mouse_area(padded)
        .on_enter(ShellMessage::DragOverZone {
            pane_id: pane_id.clone(),
            zone,
        })
        .into()
}

fn drop_zone_style(palette: Palette, is_active: bool) -> container::Style {
    if is_active {
        container::Style {
            background: Some(theme::with_alpha(palette.accent, 0.13).into()),
            border: theme::border(theme::with_alpha(palette.accent, 0.55), 1.5, 10.0),
            ..Default::default()
        }
    } else {
        container::Style::default()
    }
}

// ---------------------------------------------------------------------------
// Style closures
// ---------------------------------------------------------------------------

fn pane_style(palette: Palette, is_focused: bool) -> container::Style {
    let border = if is_focused {
        theme::border(palette.accent_alpha(0.35), 1.0, theme::RADIUS_PANE)
    } else {
        theme::border(palette.ov(0.06), 1.0, theme::RADIUS_PANE)
    };
    container::Style {
        background: Some(theme::term::SURFACE_OPAQUE.into()),
        border,
        shadow: theme::pane_shadow(),
        ..Default::default()
    }
}

fn tab_style(palette: Palette, is_active: bool, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let background = if is_active {
        Some(palette.ov(0.07).into())
    } else if hovered {
        Some(palette.ov(0.04).into())
    } else {
        None
    };
    button::Style {
        background,
        text_color: if is_active { palette.t1 } else { palette.t3 },
        border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_ROW),
        ..Default::default()
    }
}

fn ghost_button_style(palette: Palette, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: if hovered {
            Some(palette.ov(0.08).into())
        } else {
            None
        },
        text_color: palette.t3,
        border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_CHIP),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn terminal_snapshot<'a>(
    terminals: &'a [TerminalSnapshot],
    surface_id: &SurfaceId,
) -> Option<&'a TerminalSnapshot> {
    terminals
        .iter()
        .find(|terminal| &terminal.surface_id == surface_id)
}

fn placeholder_line(pane: &PaneProjection) -> String {
    pane.surfaces
        .iter()
        .find(|surface| surface.is_active)
        .map(|surface| surface_type_label(&surface.surface_type).to_string())
        .unwrap_or_default()
}

fn surface_shell_kind(surface_type: &SurfaceType) -> ShellKind {
    match surface_type {
        SurfaceType::Terminal => ShellKind::PowerShell,
        // Non-terminal surfaces reuse the CMD/neutral tint for their glyph.
        _ => ShellKind::Cmd,
    }
}

fn surface_type_label(surface_type: &SurfaceType) -> &'static str {
    match surface_type {
        SurfaceType::Terminal => "Terminal",
        SurfaceType::Markdown => "Markdown",
        SurfaceType::Diff => "Diff",
        SurfaceType::Browser => "Browser",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_workspace_shell;
    use pandamux_core::{
        AppIntent, AppState, PaneId, PaneIntent, SplitDirection, SplitPaneParams, SurfaceType,
    };

    fn model_from(state: &AppState, terminals: Vec<TerminalSnapshot>) -> ShellViewModel {
        ShellViewModel {
            projection: project_workspace_shell(state.active_workspace().unwrap()),
            terminals,
            chrome: ChromeState::default(),
            cursor_on: true,
            find: crate::overlays::FindViewState::default(),
            notifications: crate::overlays::NotificationsViewState::default(),
            copy_mode: false,
            sessions: SessionsViewState::default(),
            palette: PaletteViewState::default(),
            quick_launch: QuickLaunchViewState::default(),
            settings: SettingsViewState::default(),
            surface_contents: HashMap::new(),
            drag: None,
            term_scheme: TermScheme::default(),
            surface_term_schemes: HashMap::new(),
        }
    }

    #[test]
    fn builds_app_view_for_default_workspace() {
        let state = AppState::default();
        let projection = project_workspace_shell(state.active_workspace().unwrap());
        let active_surface_id = projection.visible_panes[0]
            .active_surface_id
            .clone()
            .expect("active surface id");
        let model = model_from(
            &state,
            vec![TerminalSnapshot::new(
                active_surface_id,
                vec!["PANDAMUX_UI_VIEW_OK".to_string()],
                80,
                24,
            )],
        );

        let _app = app_view(&model);
        let _workspace = shell_view(&model);
    }

    #[test]
    fn builds_app_view_for_split_workspace() {
        let mut state = AppState::default();
        state
            .apply(AppIntent::Pane(PaneIntent::Split(SplitPaneParams {
                workspace_id: None,
                target_pane_id: Some(PaneId::from("pane-default")),
                target_surface_id: None,
                direction: SplitDirection::Vertical,
                surface_type: SurfaceType::Terminal,
            })))
            .expect("split should apply");
        let model = model_from(&state, Vec::new());
        let _app = app_view(&model);
    }

    #[test]
    fn status_bar_is_toggleable() {
        let state = AppState::default();
        let mut model = model_from(&state, Vec::new());
        model.chrome.show_status_bar = false;
        let _app = app_view(&model);
    }

    #[test]
    fn renders_markdown_and_diff_surfaces_from_content_store() {
        // A pane whose active surface is markdown (then diff) renders its stored
        // content rather than the terminal viewport, without panicking.
        let mut state = AppState::default();
        let created = state
            .apply(AppIntent::Surface(pandamux_core::SurfaceIntent::Create {
                workspace_id: None,
                pane_id: Some(PaneId::from("pane-default")),
                surface_type: SurfaceType::Markdown,
            }))
            .expect("markdown surface should create");
        let pandamux_core::AppDelta::SurfaceCreated { surface, .. } = created else {
            panic!("expected surface created");
        };

        let mut model = model_from(&state, Vec::new());
        model.surface_contents.insert(
            surface.id.clone(),
            "# Dashboard\n\n- wave 1\n\n```\ncode\n```\n".to_string(),
        );
        let _app = app_view(&model);

        // Same pane, diff content.
        state
            .apply(AppIntent::Surface(pandamux_core::SurfaceIntent::Create {
                workspace_id: None,
                pane_id: Some(PaneId::from("pane-default")),
                surface_type: SurfaceType::Diff,
            }))
            .expect("diff surface should create");
        let mut model = model_from(&state, Vec::new());
        model
            .surface_contents
            .insert(surface.id, "@@ -1 +1 @@\n-old\n+new\n".to_string());
        let _app = app_view(&model);
    }

    #[test]
    fn renders_drop_zones_during_drag() {
        // With an active drag, panes render the drop-zone overlay (and the source
        // tab dims) without panicking.
        let state = AppState::default();
        let mut model = model_from(&state, Vec::new());
        let pane = &model.projection.visible_panes[0];
        let pane_id = pane.id.clone();
        let surface_id = pane.active_surface_id.clone().expect("active surface");
        model.drag = Some(DragView {
            surface_id,
            source_pane_id: pane_id.clone(),
            over: Some((pane_id, DropZone::Right)),
            active: true,
        });
        let _app = app_view(&model);
        let _workspace = shell_view(&model);
    }
}
