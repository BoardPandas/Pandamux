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
use crate::session_launcher::{self, SessionLauncherViewState};
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
use pandamux_term::{CellColor, SelectionSpan, StyledCell, TermModes};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
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
    /// Close a session (a shell context) from the session panel. Cascades in the
    /// runtime: drops the surface, its pane, or its whole workspace as needed.
    SessionClosed {
        workspace_id: WorkspaceId,
        surface_id: SurfaceId,
    },
    SessionGroupingChanged(SessionGrouping),
    NewSessionRequested,
    ProjectSessionRequested(WorkspaceId),
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
    LauncherLocalSelected,
    LauncherProfileSelected(pandamux_core::SshProfileId),
    LauncherProfileAdd,
    LauncherProfileEdit(pandamux_core::SshProfileId),
    LauncherProfileDelete(pandamux_core::SshProfileId),
    LauncherProfileImport,
    LauncherProfilesImported(Result<String, pandamux_core::ProjectError>),
    LauncherProfileNameChanged(String),
    LauncherProfileHostChanged(String),
    LauncherProfilePortChanged(String),
    LauncherProfileAuthChanged(pandamux_core::SshAuthConfig),
    LauncherIdentityFileChanged(String),
    LauncherProfileSave,
    LauncherCredentialChanged(String),
    LauncherCredentialSubmit,
    LauncherHostTrustConfirmed,
    LauncherPathChanged(String),
    LauncherFolderGo,
    LauncherFolderHome,
    LauncherFolderNavigate(String),
    LauncherFolderLoaded(Result<pandamux_core::FolderListing, pandamux_core::ProjectError>),
    LauncherFolderSelected,
    LauncherBack,
    SettingsSectionSelected(SettingsSection),
    AccentSelected(Accent),
    /// Terminal-tab settings controls (persisted via the settings store).
    ScrollbackLinesChanged(String),
    TerminalSettingToggled(crate::settings::TerminalToggle),
    /// A debounced async settings save completed (Err surfaces in the status).
    SettingsSaved(Result<(), String>),
    /// The async git-remote probe for a freshly launched project finished
    /// (spec 1.4 identity hint; `None` when no remote could be read).
    GitRemoteDiscovered {
        project_id: pandamux_core::ProjectId,
        url: Option<String>,
    },
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
    /// The terminal canvas measured a pane size whose grid dimensions differ
    /// from the engine grid. The runtime debounces these and resizes the
    /// engine + PTY/SSH channel to match (spec 1.1).
    ViewportResized {
        surface_id: SurfaceId,
        columns: usize,
        rows: usize,
    },
    /// Wheel/trackpad scrolling over a terminal canvas (positive scrolls up
    /// into history). The runtime routes it to the engine scrollback, or
    /// translates it to arrow keys for alternate-screen apps (spec 1.2).
    ViewportScrolled {
        surface_id: SurfaceId,
        lines: i32,
    },
    /// Absolute scroll to a history offset (scrollbar drag; offset 0 is the
    /// jump-to-bottom pill).
    ViewportScrollTo {
        surface_id: SurfaceId,
        offset: usize,
    },
    /// Shift+PageUp (-1) / Shift+PageDown (+1) scrolls the focused surface.
    ScrollPageFocused(i8),
    /// Mouse selection lifecycle on a terminal canvas (spec 1.3). Coordinates
    /// are display cells; `right_half` disambiguates which side of the cell
    /// the pointer grabbed.
    SelectionStarted {
        surface_id: SurfaceId,
        mode: pandamux_term::SelectionMode,
        line: usize,
        col: usize,
        right_half: bool,
    },
    SelectionUpdated {
        surface_id: SurfaceId,
        line: usize,
        col: usize,
        right_half: bool,
    },
    SelectionFinished(SurfaceId),
    /// Right-click on a terminal canvas: open the context menu at window
    /// coordinates. Never auto-pastes (spec 1.3).
    ContextMenuRequested {
        surface_id: SurfaceId,
        x: f32,
        y: f32,
    },
    ContextMenuDismissed,
    ContextMenuAction(crate::context_menu::ContextMenuAction),
    /// Ctrl+C: copy the selection if one exists, else send SIGINT (spec 1.3).
    CopyOrInterrupt,
    /// Ctrl+Shift+C / menu Copy: copy the selection (no SIGINT fallback).
    CopySelectionRequested,
    /// Ctrl+V / Ctrl+Shift+V / menu Paste: paste the OS clipboard.
    PasteRequested,
    SelectAllRequested,
    ClearBufferRequested,
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
    /// Plain-text rows (used for link detection and text consumers). Kept in sync
    /// with `cells` when styled data is available.
    pub lines: Vec<String>,
    /// Styled per-cell rows (color + attributes) from the grid. Empty for
    /// placeholder/text-only snapshots, in which case the viewport falls back to
    /// rendering `lines` in the scheme's default color.
    pub cells: Vec<Vec<StyledCell>>,
    /// The write-cursor position as `(row, column)` in grid coordinates.
    pub cursor: (usize, usize),
    pub columns: usize,
    pub rows: usize,
    /// Detected link spans on the visible screen (underlined by the viewport).
    pub links: Vec<LinkSpan>,
    /// The SSH host this surface is connected to, if it is a remote surface
    /// (plan F2). Drives the SSH context chip on the pane.
    pub remote_host: Option<String>,
    /// Lines above the tail the view is scrolled (0 = following new output).
    pub display_offset: usize,
    /// Scrollback lines above the visible screen (sizes the scrollbar).
    pub history_size: usize,
    /// Selection highlight spans on the visible rows.
    pub selection: Vec<SelectionSpan>,
    /// False while scrolled up into history (the write cursor is off-screen).
    pub cursor_visible: bool,
    /// Terminal mode flags for input routing (alt screen, mouse reporting).
    pub modes: TermModes,
}

impl TerminalSnapshot {
    /// Convenience constructor for text-only snapshots without styled cells or
    /// links (tests, placeholders).
    pub fn new(surface_id: SurfaceId, lines: Vec<String>, columns: usize, rows: usize) -> Self {
        Self {
            surface_id,
            lines,
            cells: Vec::new(),
            cursor: (0, 0),
            columns,
            rows,
            links: Vec::new(),
            remote_host: None,
            display_offset: 0,
            history_size: 0,
            selection: Vec::new(),
            cursor_visible: true,
            modes: TermModes::default(),
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
    pub launcher: SessionLauncherViewState,
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
    /// The open right-click context menu, if any (spec 1.3).
    pub context_menu: Option<crate::context_menu::ContextMenuViewState>,
}

// ---------------------------------------------------------------------------
// Terminal viewport (fixed-dark scheme + block cursor)
// ---------------------------------------------------------------------------

/// Canvas-local interaction state for [`TerminalViewport`].
#[derive(Debug, Clone, Default)]
pub struct ViewportState {
    /// The last grid size published as a resize intent, deduping republication
    /// while the runtime's debounce is still in flight.
    last_published: Option<(usize, usize)>,
    /// Fractional wheel-line accumulator (trackpads scroll in pixels).
    scroll_accum: f32,
    /// When the user last wheel-scrolled or dragged (keeps the scrollbar shown).
    scroll_activity: Option<Instant>,
    /// Active scrollbar-thumb drag: the pointer's grab offset inside the thumb.
    scrollbar_drag: Option<f32>,
    /// A mouse selection drag is in flight.
    selecting: bool,
    /// Last cell a selection update was published for (dedupe to cell moves).
    last_sel_cell: Option<(usize, usize, bool)>,
    /// Multi-click detection: when/where the last press landed and its count.
    last_click: Option<(Instant, Point, u8)>,
    /// Whether Shift is held (forces local selection over app mouse modes).
    shift_down: bool,
}

/// Multi-click window: presses within this interval and radius chain into
/// double/triple clicks.
const MULTI_CLICK_WINDOW: Duration = Duration::from_millis(400);
const MULTI_CLICK_RADIUS: f32 = 4.0;

/// Chain click counts: 1 = simple, 2 = word, 3 = line (wraps back to 1).
fn next_click_count(previous: Option<(Instant, Point, u8)>, position: Point) -> u8 {
    match previous {
        Some((at, last_position, count))
            if at.elapsed() < MULTI_CLICK_WINDOW
                && (position.x - last_position.x).abs() <= MULTI_CLICK_RADIUS
                && (position.y - last_position.y).abs() <= MULTI_CLICK_RADIUS =>
        {
            if count >= 3 {
                1
            } else {
                count + 1
            }
        }
        _ => 1,
    }
}

/// Hit-test a viewport-relative point to a display cell. Returns
/// `(line, col, right_half)`, clamped to the grid.
fn cell_at(
    position: Point,
    metrics: crate::metrics::CellMetrics,
    columns: usize,
    rows: usize,
) -> (usize, usize, bool) {
    let pad = theme::TERMINAL_PADDING;
    let x = ((position.x - pad) / metrics.width).max(0.0);
    let y = ((position.y - pad) / metrics.height).max(0.0);
    let raw_col = x as usize;
    let col = raw_col.min(columns.saturating_sub(1));
    let line = (y as usize).min(rows.saturating_sub(1));
    // A pointer clamped back from beyond the last column grabbed its right side.
    let right_half = raw_col > col || x.fract() > 0.5;
    (line, col, right_half)
}

/// How long the scrollbar stays visible after scroll activity without hover.
const SCROLLBAR_LINGER: Duration = Duration::from_millis(1200);
const SCROLLBAR_TRACK_WIDTH: f32 = 6.0;
const SCROLLBAR_MARGIN: f32 = 3.0;
const SCROLLBAR_MIN_THUMB: f32 = 24.0;

/// Scrollbar track and thumb rectangles in viewport-relative coordinates.
/// `None` when there is no history to scroll. Offset 0 = bottom of history.
fn scrollbar_layout(
    size: Size,
    rows: usize,
    history_size: usize,
    display_offset: usize,
) -> Option<(Rectangle, Rectangle)> {
    if history_size == 0 || rows == 0 {
        return None;
    }
    let track = Rectangle {
        x: size.width - SCROLLBAR_TRACK_WIDTH - SCROLLBAR_MARGIN,
        y: SCROLLBAR_MARGIN,
        width: SCROLLBAR_TRACK_WIDTH,
        height: (size.height - SCROLLBAR_MARGIN * 2.0).max(0.0),
    };
    let total = (rows + history_size) as f32;
    let thumb_height = (track.height * rows as f32 / total).max(SCROLLBAR_MIN_THUMB);
    let travel = (track.height - thumb_height).max(0.0);
    let fraction = display_offset as f32 / history_size as f32;
    let thumb = Rectangle {
        x: track.x,
        y: track.y + travel * (1.0 - fraction),
        width: track.width,
        height: thumb_height.min(track.height),
    };
    Some((track, thumb))
}

/// Map a dragged thumb-top position back to a history offset.
fn offset_for_thumb_top(size: Size, rows: usize, history_size: usize, thumb_top: f32) -> usize {
    let Some((track, thumb)) = scrollbar_layout(size, rows, history_size, 0) else {
        return 0;
    };
    let travel = (track.height - thumb.height).max(1.0);
    let fraction = 1.0 - ((thumb_top - track.y) / travel).clamp(0.0, 1.0);
    (fraction * history_size as f32).round() as usize
}

/// The jump-to-bottom pill, shown while scrolled up (viewport-relative).
fn jump_pill_rect(size: Size) -> Rectangle {
    let width = 118.0_f32.min(size.width * 0.6);
    let height = 24.0;
    Rectangle {
        x: (size.width - width - 18.0).max(0.0),
        y: (size.height - height - 12.0).max(0.0),
        width,
        height,
    }
}

#[derive(Debug, Clone)]
pub struct TerminalViewport {
    /// The surface this viewport renders; resize intents carry it. Placeholder
    /// viewports have none and never publish.
    surface_id: Option<SurfaceId>,
    lines: Vec<String>,
    /// Styled per-cell rows. When non-empty the viewport paints per-cell
    /// backgrounds and colored text; when empty it falls back to `lines`.
    cells: Vec<Vec<StyledCell>>,
    /// Real write-cursor position `(row, column)` from the grid, when available.
    cursor_pos: Option<(usize, usize)>,
    columns: usize,
    rows: usize,
    show_cursor: bool,
    links: Vec<LinkSpan>,
    highlight: Option<LinkSpan>,
    scheme: TermScheme,
    metrics: crate::metrics::CellMetrics,
    /// Lines above the tail the view is scrolled (0 = following).
    display_offset: usize,
    /// Scrollback lines above the visible screen (0 hides the scrollbar).
    history_size: usize,
    /// Selection spans to highlight, in rendered-cell indices.
    selection: Vec<SelectionSpan>,
    /// Terminal mode flags (mouse-owned apps suppress local selection).
    modes: TermModes,
}

impl TerminalViewport {
    pub fn new(lines: Vec<String>, columns: usize, rows: usize) -> Self {
        Self {
            surface_id: None,
            lines,
            cells: Vec::new(),
            cursor_pos: None,
            columns,
            rows,
            show_cursor: false,
            links: Vec::new(),
            highlight: None,
            scheme: TermScheme::default(),
            metrics: crate::metrics::CellMetrics::get(),
            display_offset: 0,
            history_size: 0,
            selection: Vec::new(),
            modes: TermModes::default(),
        }
    }

    /// Attach the surface identity so the canvas can publish resize intents.
    pub fn with_surface(mut self, surface_id: SurfaceId) -> Self {
        self.surface_id = Some(surface_id);
        self
    }

    /// Supply the scroll state, selection spans, and terminal modes.
    pub fn with_view_state(
        mut self,
        display_offset: usize,
        history_size: usize,
        selection: Vec<SelectionSpan>,
        modes: TermModes,
    ) -> Self {
        self.display_offset = display_offset;
        self.history_size = history_size;
        self.selection = selection;
        self.modes = modes;
        self
    }

    pub fn with_cursor(mut self, show_cursor: bool) -> Self {
        self.show_cursor = show_cursor;
        self
    }

    /// Grid dimensions that fit `bounds` at the measured cell metrics, with a
    /// small floor so degenerate layouts never produce a broken PTY size.
    fn grid_size_for(&self, bounds: Rectangle) -> (usize, usize) {
        let pad = theme::TERMINAL_PADDING * 2.0;
        let columns = ((((bounds.width - pad) / self.metrics.width) as usize).max(20)).min(1000);
        let rows = ((((bounds.height - pad) / self.metrics.height) as usize).max(5)).min(500);
        (columns, rows)
    }

    /// Whether the scrollbar renders and hit-tests: history exists and the
    /// pointer is over the pane, scrolling happened recently, or a drag is
    /// live (auto-hide when idle).
    fn scrollbar_visible(
        &self,
        state: &ViewportState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> bool {
        if self.history_size == 0 {
            return false;
        }
        if state.scrollbar_drag.is_some() || cursor.is_over(bounds) {
            return true;
        }
        state
            .scroll_activity
            .is_some_and(|at| at.elapsed() < SCROLLBAR_LINGER)
    }

    /// Supply styled per-cell rows and the real cursor position from the grid.
    pub fn with_cells(mut self, cells: Vec<Vec<StyledCell>>, cursor: (usize, usize)) -> Self {
        if !cells.is_empty() {
            self.cells = cells;
            self.cursor_pos = Some(cursor);
        }
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

    /// Resolve a [`CellColor`] against the active scheme (default fg/bg become the
    /// scheme's text/background colors).
    fn resolve(&self, color: CellColor) -> Color {
        match color {
            CellColor::Default => self.scheme.text,
            CellColor::Background => self.scheme.background,
            CellColor::Indexed(index) => {
                if let Some(color) = self.scheme.ansi.get(index as usize) {
                    *color
                } else if index <= 231 {
                    let index = index - 16;
                    let step = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
                    Color::from_rgb8(step(index / 36), step((index % 36) / 6), step(index % 6))
                } else {
                    let level = 8 + (index - 232) * 10;
                    Color::from_rgb8(level, level, level)
                }
            }
            CellColor::Rgb(r, g, b) => Color::from_rgb8(r, g, b),
        }
    }

    /// Column of the block cursor: the real grid cursor when available, else just
    /// past the last non-empty line's content.
    fn cursor_cell(&self) -> (usize, usize) {
        if let Some((row, col)) = self.cursor_pos {
            return (
                row.min(self.rows.saturating_sub(1)),
                col.min(self.columns.saturating_sub(1)),
            );
        }
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

impl canvas::Program<ShellMessage> for TerminalViewport {
    type State = ViewportState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<ShellMessage>> {
        let surface_id = self.surface_id.as_ref()?;

        // Pointer interactions: wheel scroll, scrollbar drag, jump pill.
        match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta })
                if cursor.is_over(bounds) =>
            {
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * 3.0,
                    mouse::ScrollDelta::Pixels { y, .. } => y / self.metrics.height,
                };
                state.scroll_accum += lines;
                let whole = state.scroll_accum.trunc() as i32;
                if whole != 0 {
                    state.scroll_accum -= whole as f32;
                    state.scroll_activity = Some(Instant::now());
                    return Some(
                        canvas::Action::publish(ShellMessage::ViewportScrolled {
                            surface_id: surface_id.clone(),
                            lines: whole,
                        })
                        .and_capture(),
                    );
                }
                return Some(canvas::Action::capture());
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position_in(bounds) {
                    if self.display_offset > 0 && jump_pill_rect(bounds.size()).contains(position) {
                        state.scroll_activity = Some(Instant::now());
                        return Some(
                            canvas::Action::publish(ShellMessage::ViewportScrollTo {
                                surface_id: surface_id.clone(),
                                offset: 0,
                            })
                            .and_capture(),
                        );
                    }
                    if let Some((track, thumb)) = scrollbar_layout(
                        bounds.size(),
                        self.rows,
                        self.history_size,
                        self.display_offset,
                    ) && self.scrollbar_visible(state, bounds, cursor)
                    {
                        if thumb.contains(position) {
                            state.scrollbar_drag = Some(position.y - thumb.y);
                            state.scroll_activity = Some(Instant::now());
                            return Some(canvas::Action::capture());
                        }
                        if track.contains(position) {
                            // Track click pages toward the pointer.
                            let lines = if position.y < thumb.y {
                                self.rows as i32
                            } else {
                                -(self.rows as i32)
                            };
                            state.scroll_activity = Some(Instant::now());
                            return Some(
                                canvas::Action::publish(ShellMessage::ViewportScrolled {
                                    surface_id: surface_id.clone(),
                                    lines,
                                })
                                .and_capture(),
                            );
                        }
                    }
                    // Mouse-owned apps keep their clicks unless Shift forces a
                    // local selection (Windows Terminal behavior).
                    if self.modes.mouse_reporting && !state.shift_down {
                        return None;
                    }
                    let count = next_click_count(state.last_click.take(), position);
                    state.last_click = Some((Instant::now(), position, count));
                    let mode = match count {
                        2 => pandamux_term::SelectionMode::Word,
                        3 => pandamux_term::SelectionMode::Line,
                        _ => pandamux_term::SelectionMode::Simple,
                    };
                    let (line, col, right_half) =
                        cell_at(position, self.metrics, self.columns, self.rows);
                    state.selecting = true;
                    state.last_sel_cell = Some((line, col, right_half));
                    // Published without capture so the pane's mouse_area still
                    // receives the press and focuses the pane.
                    return Some(canvas::Action::publish(ShellMessage::SelectionStarted {
                        surface_id: surface_id.clone(),
                        mode,
                        line,
                        col,
                        right_half,
                    }));
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if cursor.is_over(bounds)
                    && let Some(absolute) = cursor.position()
                {
                    // Opens the menu; never pastes or writes to the PTY.
                    return Some(
                        canvas::Action::publish(ShellMessage::ContextMenuRequested {
                            surface_id: surface_id.clone(),
                            x: absolute.x,
                            y: absolute.y,
                        })
                        .and_capture(),
                    );
                }
            }
            canvas::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                state.shift_down = modifiers.shift();
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(grab) = state.scrollbar_drag
                    && let Some(position) = cursor.position_in(bounds)
                {
                    state.scroll_activity = Some(Instant::now());
                    let offset = offset_for_thumb_top(
                        bounds.size(),
                        self.rows,
                        self.history_size,
                        position.y - grab,
                    );
                    if offset != self.display_offset {
                        return Some(
                            canvas::Action::publish(ShellMessage::ViewportScrollTo {
                                surface_id: surface_id.clone(),
                                offset,
                            })
                            .and_capture(),
                        );
                    }
                    return Some(canvas::Action::capture());
                }
                if state.selecting {
                    if let Some(position) = cursor.position_in(bounds) {
                        let cell = cell_at(position, self.metrics, self.columns, self.rows);
                        if state.last_sel_cell != Some(cell) {
                            state.last_sel_cell = Some(cell);
                            let (line, col, right_half) = cell;
                            return Some(canvas::Action::publish(ShellMessage::SelectionUpdated {
                                surface_id: surface_id.clone(),
                                line,
                                col,
                                right_half,
                            }));
                        }
                    } else if let Some(absolute) = cursor.position() {
                        // Dragging past the top/bottom edge extends the
                        // selection into scrollback by scrolling the view.
                        let lines = if absolute.y < bounds.y {
                            2
                        } else if absolute.y > bounds.y + bounds.height {
                            -2
                        } else {
                            0
                        };
                        if lines != 0 {
                            return Some(canvas::Action::publish(ShellMessage::ViewportScrolled {
                                surface_id: surface_id.clone(),
                                lines,
                            }));
                        }
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.scrollbar_drag.take().is_some() {
                    return Some(canvas::Action::capture());
                }
                if state.selecting {
                    state.selecting = false;
                    state.last_sel_cell = None;
                    return Some(canvas::Action::publish(ShellMessage::SelectionFinished(
                        surface_id.clone(),
                    )));
                }
            }
            _ => {}
        }

        // The size check runs on redraws (each ~100ms tick repaints) and mouse
        // events; anything else cannot have changed the layout.
        let relevant = matches!(
            event,
            canvas::Event::Window(iced::window::Event::RedrawRequested(_))
                | canvas::Event::Mouse(_)
        );
        if !relevant {
            return None;
        }
        let (columns, rows) = self.grid_size_for(bounds);
        if (columns, rows) == (self.columns, self.rows) {
            // The engine already matches this pane; re-arm publishing.
            state.last_published = None;
            return None;
        }
        if state.last_published == Some((columns, rows)) {
            return None;
        }
        state.last_published = Some((columns, rows));
        Some(canvas::Action::publish(ShellMessage::ViewportResized {
            surface_id: surface_id.clone(),
            columns,
            rows,
        }))
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let background = canvas::Path::rectangle(Point::ORIGIN, bounds.size());
        frame.fill(&background, self.scheme.background);

        let pad = theme::TERMINAL_PADDING;
        let cell_h = self.metrics.height;
        let cell_w = self.metrics.width;

        // Per-cell backgrounds (reverse video, colored prompts, menu highlights),
        // drawn first so everything else composits over them. Contiguous cells
        // with the same background are batched into one rectangle.
        if !self.cells.is_empty() {
            for (row_index, cells) in self.cells.iter().take(self.rows).enumerate() {
                let y = pad + row_index as f32 * cell_h;
                let mut col = 0;
                while col < cells.len() {
                    let bg = self.resolve(cells[col].bg);
                    if bg == self.scheme.background {
                        col += 1;
                        continue;
                    }
                    let start = col;
                    while col < cells.len() && self.resolve(cells[col].bg) == bg {
                        col += 1;
                    }
                    let x = pad + start as f32 * cell_w;
                    let width = (col - start) as f32 * cell_w;
                    let rect = canvas::Path::rectangle(Point::new(x, y), Size::new(width, cell_h));
                    frame.fill(&rect, bg);
                }
            }
        }

        // Selection highlight: over per-cell backgrounds, behind text.
        for span in self.selection.iter().take(1024) {
            if span.line >= self.rows || span.end < span.start {
                continue;
            }
            let x = pad + span.start as f32 * cell_w;
            let width = (span.end - span.start + 1) as f32 * cell_w;
            let y = pad + span.line as f32 * cell_h;
            let rect = canvas::Path::rectangle(Point::new(x, y), Size::new(width, cell_h));
            frame.fill(&rect, theme::with_alpha(self.scheme.text, 0.25));
        }

        // Current find match highlight (drawn over cell backgrounds, behind text).
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

        if self.cells.is_empty() {
            // Text-only fallback (placeholders, tests): one draw call per line.
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
        } else {
            // Styled text: batch contiguous cells sharing (foreground, bold) into
            // one draw call. Runs of blank cells are skipped.
            for (row_index, cells) in self.cells.iter().take(self.rows).enumerate() {
                let y = pad + row_index as f32 * cell_h;
                let mut col = 0;
                while col < cells.len() {
                    let fg = self.resolve(cells[col].fg);
                    let bold = cells[col].bold;
                    let start = col;
                    let mut content = String::new();
                    while col < cells.len()
                        && self.resolve(cells[col].fg) == fg
                        && cells[col].bold == bold
                    {
                        content.push(cells[col].c);
                        col += 1;
                    }
                    if content.trim().is_empty() {
                        continue;
                    }
                    let weight = if bold {
                        iced::font::Weight::Bold
                    } else {
                        iced::font::Weight::Normal
                    };
                    frame.fill_text(canvas::Text {
                        content,
                        position: Point::new(pad + start as f32 * cell_w, y),
                        max_width: (bounds.width - pad * 2.0).max(0.0),
                        color: fg,
                        size: Pixels(theme::SIZE_TERMINAL),
                        line_height: iced::widget::text::LineHeight::Absolute(Pixels(cell_h)),
                        font: theme::mono(weight),
                        shaping: iced::widget::text::Shaping::Advanced,
                        ..canvas::Text::default()
                    });
                }
            }
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
            let cursor_rect = canvas::Path::rectangle(
                Point::new(cursor_x, cursor_y),
                Size::new(theme::term::CURSOR_WIDTH, theme::term::CURSOR_HEIGHT),
            );
            // Cursor uses the active scheme's prompt/cursor color.
            frame.fill(&cursor_rect, self.scheme.cursor);
        }

        // Scrollback affordances: an auto-hide scrollbar at the right edge and
        // a jump-to-bottom pill while scrolled up (spec 1.2).
        if self.scrollbar_visible(state, bounds, cursor)
            && let Some((track, thumb)) = scrollbar_layout(
                bounds.size(),
                self.rows,
                self.history_size,
                self.display_offset,
            )
        {
            let track_path = canvas::Path::rounded_rectangle(
                Point::new(track.x, track.y),
                Size::new(track.width, track.height),
                (track.width / 2.0).into(),
            );
            frame.fill(&track_path, theme::with_alpha(self.scheme.text, 0.08));
            let thumb_path = canvas::Path::rounded_rectangle(
                Point::new(thumb.x, thumb.y),
                Size::new(thumb.width, thumb.height),
                (thumb.width / 2.0).into(),
            );
            let thumb_alpha = if state.scrollbar_drag.is_some() {
                0.55
            } else {
                0.35
            };
            frame.fill(
                &thumb_path,
                theme::with_alpha(self.scheme.text, thumb_alpha),
            );
        }
        if self.display_offset > 0 {
            let pill = jump_pill_rect(bounds.size());
            let pill_path = canvas::Path::rounded_rectangle(
                Point::new(pill.x, pill.y),
                Size::new(pill.width, pill.height),
                (pill.height / 2.0).into(),
            );
            frame.fill(&pill_path, theme::with_alpha(self.scheme.text, 0.18));
            frame.fill_text(canvas::Text {
                content: format!("v {} lines below", self.display_offset),
                position: Point::new(pill.x + 10.0, pill.y + (pill.height - 12.0) / 2.0),
                max_width: pill.width - 16.0,
                color: self.scheme.text,
                size: Pixels(11.0),
                line_height: iced::widget::text::LineHeight::Absolute(Pixels(12.0)),
                font: theme::MONO_FONT,
                shaping: iced::widget::text::Shaping::Advanced,
                ..canvas::Text::default()
            });
        }

        vec![frame.into_geometry()]
    }
}

pub fn terminal_viewport<'a>(
    lines: Vec<String>,
    columns: usize,
    rows: usize,
) -> Element<'a, ShellMessage> {
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
            layers = layers.push(session_launcher::session_launcher(&model.launcher, palette));
        }
        Overlay::Settings => {
            layers = layers.push(settings::settings_modal(&model.settings, palette));
        }
    }

    // The right-click context menu sits above every other overlay.
    if let Some(menu) = &model.context_menu {
        layers = layers.push(crate::context_menu::context_menu_layer(menu, palette));
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
                        .with_surface(snapshot.surface_id.clone())
                        .with_view_state(
                            snapshot.display_offset,
                            snapshot.history_size,
                            snapshot.selection.clone(),
                            snapshot.modes,
                        )
                        .with_cells(snapshot.cells.clone(), snapshot.cursor)
                        .with_cursor(is_focused && cursor_on && snapshot.cursor_visible)
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
            launcher: SessionLauncherViewState::default(),
            settings: SettingsViewState::default(),
            surface_contents: HashMap::new(),
            drag: None,
            term_scheme: TermScheme::default(),
            surface_term_schemes: HashMap::new(),
            context_menu: None,
        }
    }

    #[test]
    fn scrollbar_layout_round_trips_offsets() {
        let size = Size::new(800.0, 600.0);
        // No history: no scrollbar at all.
        assert!(scrollbar_layout(size, 40, 0, 0).is_none());

        let (track, thumb_bottom) = scrollbar_layout(size, 40, 960, 0).expect("scrollbar");
        let (_, thumb_top) = scrollbar_layout(size, 40, 960, 960).expect("scrollbar");
        // Following the tail puts the thumb at the bottom of the track; fully
        // scrolled up puts it at the top.
        assert!(thumb_bottom.y > thumb_top.y);
        assert!((thumb_top.y - track.y).abs() < 0.5);
        assert!(
            (thumb_bottom.y + thumb_bottom.height - (track.y + track.height)).abs() < 0.5,
            "thumb should rest at the track bottom when following"
        );
        // The thumb never collapses below the minimum grab size.
        assert!(thumb_bottom.height >= SCROLLBAR_MIN_THUMB);

        // Dragging the thumb back to where the layout placed it recovers the
        // same offset (round trip within a line).
        for offset in [0_usize, 137, 480, 960] {
            let (_, thumb) = scrollbar_layout(size, 40, 960, offset).expect("scrollbar");
            let recovered = offset_for_thumb_top(size, 40, 960, thumb.y);
            assert!(
                (recovered as i64 - offset as i64).abs() <= 1,
                "offset {offset} round-tripped to {recovered}"
            );
        }
    }

    #[test]
    fn cell_hit_test_accounts_for_padding_and_clamps() {
        let metrics = crate::metrics::CellMetrics {
            width: 8.0,
            height: 20.0,
        };
        let pad = theme::TERMINAL_PADDING;
        // First cell, left half.
        assert_eq!(
            cell_at(Point::new(pad + 1.0, pad + 1.0), metrics, 80, 24),
            (0, 0, false)
        );
        // Third column, right half of the cell.
        assert_eq!(
            cell_at(
                Point::new(pad + 2.0 * 8.0 + 7.0, pad + 1.0),
                metrics,
                80,
                24
            ),
            (0, 2, true)
        );
        // Row math.
        assert_eq!(
            cell_at(
                Point::new(pad + 1.0, pad + 3.0 * 20.0 + 5.0),
                metrics,
                80,
                24
            ),
            (3, 0, false)
        );
        // Clamped to the grid bounds, including negative overshoot.
        assert_eq!(
            cell_at(Point::new(10_000.0, 10_000.0), metrics, 80, 24),
            (23, 79, true)
        );
        let (line, col, _) = cell_at(Point::new(-50.0, -50.0), metrics, 80, 24);
        assert_eq!((line, col), (0, 0));
    }

    #[test]
    fn click_counts_chain_within_the_window_and_wrap() {
        let origin = Point::new(100.0, 100.0);
        assert_eq!(next_click_count(None, origin), 1);
        let now = Instant::now();
        assert_eq!(next_click_count(Some((now, origin, 1)), origin), 2);
        assert_eq!(next_click_count(Some((now, origin, 2)), origin), 3);
        // A fourth quick click starts over.
        assert_eq!(next_click_count(Some((now, origin, 3)), origin), 1);
        // A press far away resets the chain.
        assert_eq!(
            next_click_count(Some((now, origin, 1)), Point::new(200.0, 200.0)),
            1
        );
        // A stale press resets the chain.
        let stale = now - Duration::from_millis(600);
        assert_eq!(next_click_count(Some((stale, origin, 1)), origin), 1);
    }

    #[test]
    fn context_menu_layer_builds_for_both_selection_states() {
        let palette = Palette::new(crate::theme::UiTheme::Dark, crate::theme::Accent::Teal);
        for has_selection in [false, true] {
            let state = crate::context_menu::ContextMenuViewState {
                surface_id: SurfaceId::generate(),
                pane_id: Some(PaneId::generate()),
                x: 200.0,
                y: 150.0,
                has_selection,
            };
            let _menu = crate::context_menu::context_menu_layer(&state, palette);
        }
    }

    #[test]
    fn jump_pill_sits_inside_the_viewport() {
        let size = Size::new(800.0, 600.0);
        let pill = jump_pill_rect(size);
        assert!(pill.x >= 0.0 && pill.y >= 0.0);
        assert!(pill.x + pill.width <= size.width);
        assert!(pill.y + pill.height <= size.height);
    }

    #[test]
    fn styled_viewport_uses_real_cursor_and_resolves_colors() {
        let cell = |c: char, fg: CellColor, bg: CellColor| StyledCell {
            c,
            fg,
            bg,
            bold: false,
        };
        let cells = vec![vec![
            cell('h', CellColor::Default, CellColor::Background),
            cell('i', CellColor::Rgb(0x80, 0x00, 0x00), CellColor::Background),
        ]];
        let scheme = TermScheme::default();
        let viewport = TerminalViewport::new(vec!["hi".to_string()], 80, 24)
            .with_cells(cells, (5, 12))
            .with_scheme(scheme);

        // The real grid cursor position wins over the "past last content" heuristic.
        assert_eq!(viewport.cursor_cell(), (5, 12));
        // Defaults and indexed colors use the scheme; RGB resolves literally.
        assert_eq!(viewport.resolve(CellColor::Default), scheme.text);
        assert_eq!(viewport.resolve(CellColor::Background), scheme.background);
        assert_eq!(viewport.resolve(CellColor::Indexed(3)), scheme.ansi[3]);
        assert_eq!(
            viewport.resolve(CellColor::Rgb(0x80, 0x00, 0x00)),
            Color::from_rgb8(0x80, 0x00, 0x00)
        );
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
