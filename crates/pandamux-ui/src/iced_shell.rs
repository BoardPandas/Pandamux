use crate::shell_projection::{
    PaneProjection, ShellNodeProjection, ShellProjection, SurfaceProjection,
};
use iced::widget::{button, canvas, column, container, row, text};
use iced::{Color, Element, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme, mouse};
use pandamux_core::{PaneId, SplitDirection, SurfaceId, SurfaceType};

const CELL_WIDTH: f32 = 9.0;
const CELL_HEIGHT: f32 = 20.0;
const PADDING: f32 = 12.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellMessage {
    Tick,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub surface_id: SurfaceId,
    pub lines: Vec<String>,
    pub columns: usize,
    pub rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellViewModel {
    pub projection: ShellProjection,
    pub terminals: Vec<TerminalSnapshot>,
}

#[derive(Debug, Clone)]
pub struct TerminalViewport {
    lines: Vec<String>,
    columns: usize,
    rows: usize,
}

impl TerminalViewport {
    pub fn new(lines: Vec<String>, columns: usize, rows: usize) -> Self {
        Self {
            lines,
            columns,
            rows,
        }
    }

    fn preferred_height(&self) -> f32 {
        PADDING * 2.0 + self.rows as f32 * CELL_HEIGHT
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
        frame.fill(&background, Color::from_rgb8(10, 13, 18));

        let terminal_width = (self.columns as f32 * CELL_WIDTH + PADDING * 2.0).min(bounds.width);
        let terminal_height = self.preferred_height().min(bounds.height);
        let terminal = canvas::Path::rectangle(
            Point::ORIGIN,
            Size::new(terminal_width.max(0.0), terminal_height.max(0.0)),
        );
        frame.fill(&terminal, Color::from_rgb8(16, 22, 30));

        for (row, line) in self.lines.iter().take(self.rows).enumerate() {
            frame.fill_text(canvas::Text {
                content: line.clone(),
                position: Point::new(PADDING, PADDING + row as f32 * CELL_HEIGHT),
                max_width: (terminal_width - PADDING * 2.0).max(0.0),
                color: Color::from_rgb8(229, 236, 244),
                size: Pixels(15.0),
                line_height: iced::widget::text::LineHeight::Absolute(Pixels(CELL_HEIGHT)),
                font: Font::MONOSPACE,
                shaping: iced::widget::text::Shaping::Advanced,
                ..canvas::Text::default()
            });
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

pub fn shell_view(model: &ShellViewModel) -> Element<'_, ShellMessage> {
    view_node(&model.projection.root, &model.terminals)
}

fn view_node<'a>(
    node: &'a ShellNodeProjection,
    terminals: &'a [TerminalSnapshot],
) -> Element<'a, ShellMessage> {
    match node {
        ShellNodeProjection::Pane(pane) => view_pane(pane, terminals),
        ShellNodeProjection::Split {
            direction,
            ratio_percent,
            first,
            second,
        } => {
            let first_portion = (*ratio_percent).max(10) as u16;
            let second_portion = (100_u8.saturating_sub(*ratio_percent)).max(10) as u16;
            let first_view = container(view_node(first, terminals))
                .width(match direction {
                    SplitDirection::Horizontal => Length::FillPortion(first_portion),
                    SplitDirection::Vertical => Length::Fill,
                })
                .height(match direction {
                    SplitDirection::Horizontal => Length::Fill,
                    SplitDirection::Vertical => Length::FillPortion(first_portion),
                });
            let second_view = container(view_node(second, terminals))
                .width(match direction {
                    SplitDirection::Horizontal => Length::FillPortion(second_portion),
                    SplitDirection::Vertical => Length::Fill,
                })
                .height(match direction {
                    SplitDirection::Horizontal => Length::Fill,
                    SplitDirection::Vertical => Length::FillPortion(second_portion),
                });

            match direction {
                SplitDirection::Horizontal => row![first_view, second_view]
                    .spacing(1)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                SplitDirection::Vertical => column![first_view, second_view]
                    .spacing(1)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        }
    }
}

fn view_pane<'a>(
    pane: &'a PaneProjection,
    terminals: &'a [TerminalSnapshot],
) -> Element<'a, ShellMessage> {
    let tabs = pane
        .surfaces
        .iter()
        .fold(row![].spacing(4), |row, surface| {
            row.push(surface_tab(surface))
        });
    let active_terminal = pane
        .active_surface_id
        .as_ref()
        .and_then(|surface_id| terminal_snapshot(terminals, surface_id));
    let terminal = active_terminal
        .map(|snapshot| terminal_viewport(snapshot.lines.clone(), snapshot.columns, snapshot.rows))
        .unwrap_or_else(|| {
            terminal_viewport(
                vec![surface_label(
                    pane.surfaces.iter().find(|surface| surface.is_active),
                )],
                80,
                24,
            )
        });
    let zoom_label = if pane.is_zoomed { "Unzoom" } else { "Zoom" };
    let controls = row![
        button(text("Focus")).on_press(ShellMessage::PaneFocused(pane.id.clone())),
        button(text(zoom_label)).on_press(ShellMessage::PaneZoomToggled(pane.id.clone())),
        button(text("Split H")).on_press(ShellMessage::PaneSplit {
            pane_id: pane.id.clone(),
            direction: SplitDirection::Horizontal,
        }),
        button(text("Split V")).on_press(ShellMessage::PaneSplit {
            pane_id: pane.id.clone(),
            direction: SplitDirection::Vertical,
        }),
        button(text("+ Tab")).on_press(ShellMessage::TerminalSurfaceCreated(pane.id.clone())),
        button(text("Close Pane")).on_press(ShellMessage::PaneClosed(pane.id.clone())),
    ]
    .spacing(6);

    container(column![tabs, terminal, controls].spacing(6))
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn terminal_snapshot<'a>(
    terminals: &'a [TerminalSnapshot],
    surface_id: &SurfaceId,
) -> Option<&'a TerminalSnapshot> {
    terminals
        .iter()
        .find(|terminal| &terminal.surface_id == surface_id)
}

fn surface_tab(surface: &SurfaceProjection) -> Element<'_, ShellMessage> {
    let marker = if surface.is_active { "*" } else { "" };
    row![
        button(text(format!(
            "{}{}",
            marker,
            surface_type_label(&surface.surface_type)
        )))
        .on_press(ShellMessage::SurfaceFocused(surface.id.clone())),
        button(text("x")).on_press(ShellMessage::SurfaceClosed(surface.id.clone())),
    ]
    .spacing(2)
    .into()
}

fn surface_label(surface: Option<&SurfaceProjection>) -> String {
    surface
        .map(|surface| surface_type_label(&surface.surface_type).to_string())
        .unwrap_or_default()
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
    use pandamux_core::{
        AppIntent, AppState, PaneId, PaneIntent, SplitDirection, SplitPaneParams, SurfaceType,
    };

    #[test]
    fn builds_shell_view_for_default_workspace() {
        let state = AppState::default();
        let projection = crate::project_workspace_shell(state.active_workspace().unwrap());
        let active_surface_id = projection.visible_panes[0]
            .active_surface_id
            .clone()
            .expect("active surface id");
        let model = ShellViewModel {
            projection,
            terminals: vec![TerminalSnapshot {
                surface_id: active_surface_id,
                lines: vec!["PANDAMUX_UI_VIEW_OK".to_string()],
                columns: 80,
                rows: 24,
            }],
        };

        let _view = shell_view(&model);
    }

    #[test]
    fn builds_shell_view_for_split_workspace() {
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
        let projection = crate::project_workspace_shell(state.active_workspace().unwrap());
        let model = ShellViewModel {
            projection,
            terminals: Vec::new(),
        };

        let _view = shell_view(&model);
    }
}
