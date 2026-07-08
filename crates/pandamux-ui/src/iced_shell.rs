//! The native Iced shell view.
//!
//! [`app_view`] composes the full chrome (titlebar, icon rail, styled pane
//! workspace, status bar) from a [`ShellViewModel`]. [`shell_view`] renders just
//! the pane workspace and is used by the headless smoke path and unit tests.
//!
//! This layer is a read-projection plus an intent source: every interaction maps
//! to a [`ShellMessage`] that the runtime routes into core intents or window
//! actions. It never owns canonical state.

use crate::chrome::{self, ChromeState, RailItem};
use crate::shell_projection::{ColumnProjection, PaneProjection, SurfaceProjection};
use crate::theme::{self, Palette, ShellKind};
use iced::widget::{Space, button, canvas, column, container, mouse_area, row, text};
use iced::{
    Alignment, Color, Element, Length, Padding, Pixels, Point, Rectangle, Renderer, Size, Theme,
    mouse,
};
use pandamux_core::{PaneId, SplitDirection, SurfaceId, SurfaceType};

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
    TerminalSurfaceCreated(PaneId),
    SurfaceFocused(SurfaceId),
    SurfaceClosed(SurfaceId),
    // Chrome / window
    WindowDragStarted,
    WindowMinimizePressed,
    WindowMaximizeToggled,
    WindowClosePressed,
    RailSelected(RailItem),
    /// An overlay (palette, notifications, settings, quick-launch) was requested.
    /// Overlays land in Phases 4-5; the runtime records the request for now.
    OverlayRequested(RailItem),
    ToggleStatusBar,
    ToggleTheme,
    CycleAccent,
    /// No-op (e.g. an unmapped key press); ignored by the runtime.
    Noop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub surface_id: SurfaceId,
    pub lines: Vec<String>,
    pub columns: usize,
    pub rows: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShellViewModel {
    pub projection: crate::shell_projection::ShellProjection,
    pub terminals: Vec<TerminalSnapshot>,
    pub chrome: ChromeState,
    /// Blink phase for the focused pane's block cursor (~1.1s cadence).
    pub cursor_on: bool,
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
}

impl TerminalViewport {
    pub fn new(lines: Vec<String>, columns: usize, rows: usize) -> Self {
        Self {
            lines,
            columns,
            rows,
            show_cursor: false,
        }
    }

    pub fn with_cursor(mut self, show_cursor: bool) -> Self {
        self.show_cursor = show_cursor;
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
        frame.fill(&background, theme::term::SURFACE_OPAQUE);

        let pad = theme::TERMINAL_PADDING;
        let cell_h = theme::term::CELL_HEIGHT;
        let cell_w = theme::term::CELL_WIDTH;

        for (row_index, line) in self.lines.iter().take(self.rows).enumerate() {
            frame.fill_text(canvas::Text {
                content: line.clone(),
                position: Point::new(pad, pad + row_index as f32 * cell_h),
                max_width: (bounds.width - pad * 2.0).max(0.0),
                color: theme::term::TEXT,
                size: Pixels(theme::SIZE_TERMINAL),
                line_height: iced::widget::text::LineHeight::Absolute(Pixels(cell_h)),
                font: theme::MONO_FONT,
                shaping: iced::widget::text::Shaping::Advanced,
                ..canvas::Text::default()
            });
        }

        if self.show_cursor {
            let (crow, ccol) = self.cursor_cell();
            let cursor_x = pad + ccol as f32 * cell_w;
            let cursor_y = pad + crow as f32 * cell_h + (cell_h - theme::term::CURSOR_HEIGHT) / 2.0;
            let cursor = canvas::Path::rectangle(
                Point::new(cursor_x, cursor_y),
                Size::new(theme::term::CURSOR_WIDTH, theme::term::CURSOR_HEIGHT),
            );
            // Prompt color is the accent; blink handled by the caller's flag.
            frame.fill(&cursor, theme::Accent::Teal.color());
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

    let body = row![
        chrome::icon_rail(&model.chrome, palette),
        workspace_view(model, palette),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    let mut root = column![chrome::titlebar(&model.chrome, palette), body]
        .width(Length::Fill)
        .height(Length::Fill);

    if model.chrome.show_status_bar {
        root = root.push(chrome::status_bar(&model.chrome, palette));
    }

    container(root)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(palette.bg_base.into()),
            ..Default::default()
        })
        .into()
}

/// The pane workspace only (used by tests and the headless smoke path).
pub fn shell_view(model: &ShellViewModel) -> Element<'_, ShellMessage> {
    workspace_view(model, model.chrome.palette())
}

fn workspace_view<'a>(model: &'a ShellViewModel, palette: Palette) -> Element<'a, ShellMessage> {
    let focused = model.projection.focused_pane_id.as_ref();
    let mut columns = row![].spacing(theme::PANE_GAP);
    for col in &model.projection.columns {
        columns = columns.push(column_view(
            col,
            &model.terminals,
            palette,
            focused,
            model.cursor_on,
        ));
    }
    container(columns.width(Length::Fill).height(Length::Fill))
        .padding(theme::WORKSPACE_PADDING)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn column_view<'a>(
    col: &'a ColumnProjection,
    terminals: &'a [TerminalSnapshot],
    palette: Palette,
    focused: Option<&PaneId>,
    cursor_on: bool,
) -> Element<'a, ShellMessage> {
    let mut stacked = column![].spacing(theme::PANE_GAP);
    for pane in &col.panes {
        stacked = stacked.push(pane_view(pane, terminals, palette, focused, cursor_on));
    }
    stacked.width(Length::Fill).height(Length::Fill).into()
}

fn pane_view<'a>(
    pane: &'a PaneProjection,
    terminals: &'a [TerminalSnapshot],
    palette: Palette,
    focused: Option<&PaneId>,
    cursor_on: bool,
) -> Element<'a, ShellMessage> {
    let is_focused = focused == Some(&pane.id);

    let tab_bar = tab_bar_view(pane, palette);

    let active_terminal = pane
        .active_surface_id
        .as_ref()
        .and_then(|surface_id| terminal_snapshot(terminals, surface_id));
    let viewport = match active_terminal {
        Some(snapshot) => canvas::Canvas::new(
            TerminalViewport::new(snapshot.lines.clone(), snapshot.columns, snapshot.rows)
                .with_cursor(is_focused && cursor_on),
        )
        .width(Length::Fill)
        .height(Length::Fill),
        None => canvas::Canvas::new(
            TerminalViewport::new(vec![placeholder_line(pane)], 80, 24)
                .with_cursor(is_focused && cursor_on),
        )
        .width(Length::Fill)
        .height(Length::Fill),
    };

    let contents = column![tab_bar, viewport]
        .width(Length::Fill)
        .height(Length::Fill);

    let pane_box = container(contents)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(move |_theme| pane_style(palette, is_focused));

    mouse_area(pane_box)
        .on_press(ShellMessage::PaneFocused(pane.id.clone()))
        .into()
}

fn tab_bar_view<'a>(pane: &'a PaneProjection, palette: Palette) -> Element<'a, ShellMessage> {
    let mut tabs = row![].spacing(4).align_y(Alignment::Center);
    for surface in &pane.surfaces {
        tabs = tabs.push(tab_view(surface, palette));
    }

    let add_tab = icon_button(
        "+",
        palette,
        ShellMessage::TerminalSurfaceCreated(pane.id.clone()),
    );
    let split_right = icon_button(
        "\u{25eb}", // ◫
        palette,
        ShellMessage::PaneSplit {
            pane_id: pane.id.clone(),
            direction: SplitDirection::Horizontal,
        },
    );
    let split_down = icon_button(
        "\u{2b12}", // ⬒
        palette,
        ShellMessage::PaneSplit {
            pane_id: pane.id.clone(),
            direction: SplitDirection::Vertical,
        },
    );
    let zoom = icon_button(
        if pane.is_zoomed {
            "\u{2921}"
        } else {
            "\u{2922}"
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

fn tab_view<'a>(surface: &'a SurfaceProjection, palette: Palette) -> Element<'a, ShellMessage> {
    let kind = surface_shell_kind(&surface.surface_type);
    let shell_color = palette.shell_color(kind);
    let is_active = surface.is_active;

    let label = row![
        text(kind.glyph())
            .size(theme::SIZE_METADATA)
            .font(theme::mono(iced::font::Weight::Semibold))
            .color(shell_color),
        text(surface_type_label(&surface.surface_type))
            .size(theme::SIZE_BODY)
            .color(if is_active { palette.t1 } else { palette.t3 }),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let focus_button = button(label)
        .padding(Padding::from([5.0, 8.0]))
        .on_press(ShellMessage::SurfaceFocused(surface.id.clone()))
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
    glyph: &'static str,
    palette: Palette,
    message: ShellMessage,
) -> Element<'a, ShellMessage> {
    button(
        container(text(glyph).size(theme::SIZE_BODY).color(palette.t3))
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
            vec![TerminalSnapshot {
                surface_id: active_surface_id,
                lines: vec!["PANDAMUX_UI_VIEW_OK".to_string()],
                columns: 80,
                rows: 24,
            }],
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
}
