//! Right-click context menu for terminal panes (spec 1.3).
//!
//! Opened by a right press on the terminal canvas; never auto-pastes. The menu
//! is a positioned card over a transparent backdrop: clicking anywhere else
//! (or right-clicking again) dismisses it, item presses dispatch their action
//! and close it. Modeled on the Windows Terminal pane menu.

use crate::command_palette::overlay_card_style;
use crate::iced_shell::ShellMessage;
use crate::theme::{self, Palette};
use iced::widget::{Space, button, column, container, mouse_area, responsive, text};
use iced::{Element, Length, Padding};
use pandamux_core::{PaneId, SurfaceId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuAction {
    Copy,
    Paste,
    SelectAll,
    ClearBuffer,
    Find,
    SplitRight,
    SplitDown,
    CloseTab,
}

impl ContextMenuAction {
    fn label(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::SelectAll => "Select All",
            Self::ClearBuffer => "Clear Buffer",
            Self::Find => "Find...",
            Self::SplitRight => "Split Right",
            Self::SplitDown => "Split Down",
            Self::CloseTab => "Close Tab",
        }
    }
}

/// What the open context menu targets and where it sits (window coordinates).
#[derive(Clone, Debug, PartialEq)]
pub struct ContextMenuViewState {
    pub surface_id: SurfaceId,
    /// The pane hosting the surface (drives Split/Close items when known).
    pub pane_id: Option<PaneId>,
    pub x: f32,
    pub y: f32,
    pub has_selection: bool,
}

const MENU_WIDTH: f32 = 190.0;
const ROW_HEIGHT: f32 = 28.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
const MENU_PADDING: f32 = 6.0;
const EDGE_GAP: f32 = 8.0;

/// The menu groups, top to bottom. `None` renders a separator.
fn menu_items(state: &ContextMenuViewState) -> Vec<Option<(ContextMenuAction, bool)>> {
    let has_pane = state.pane_id.is_some();
    vec![
        Some((ContextMenuAction::Copy, state.has_selection)),
        Some((ContextMenuAction::Paste, true)),
        Some((ContextMenuAction::SelectAll, true)),
        Some((ContextMenuAction::ClearBuffer, true)),
        None,
        Some((ContextMenuAction::Find, true)),
        None,
        Some((ContextMenuAction::SplitRight, has_pane)),
        Some((ContextMenuAction::SplitDown, has_pane)),
        Some((ContextMenuAction::CloseTab, has_pane)),
    ]
}

/// The full-window context-menu layer: a transparent dismiss backdrop plus the
/// menu card clamped inside the window.
pub fn context_menu_layer<'a>(
    state: &'a ContextMenuViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    responsive(move |size| {
        let items = menu_items(state);
        let menu_height = items
            .iter()
            .map(|item| match item {
                Some(_) => ROW_HEIGHT,
                None => SEPARATOR_HEIGHT,
            })
            .sum::<f32>()
            + MENU_PADDING * 2.0;
        let x = state
            .x
            .min(size.width - MENU_WIDTH - EDGE_GAP)
            .max(EDGE_GAP);
        let y = state
            .y
            .min(size.height - menu_height - EDGE_GAP)
            .max(EDGE_GAP);

        let mut rows = column![].width(Length::Fixed(MENU_WIDTH));
        for item in items {
            rows = rows.push(match item {
                Some((action, enabled)) => menu_row(action, enabled, palette),
                None => separator(palette),
            });
        }
        let card = container(rows)
            .padding(MENU_PADDING)
            .style(move |_theme| overlay_card_style(palette));

        let positioned = container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::Alignment::Start)
            .align_y(iced::Alignment::Start)
            .padding(Padding {
                top: y,
                left: x,
                right: 0.0,
                bottom: 0.0,
            });

        // Transparent backdrop: any press outside the card dismisses. Item
        // buttons capture their own presses first.
        mouse_area(positioned)
            .on_press(ShellMessage::ContextMenuDismissed)
            .on_right_press(ShellMessage::ContextMenuDismissed)
            .into()
    })
    .into()
}

fn menu_row<'a>(
    action: ContextMenuAction,
    enabled: bool,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let color = if enabled { palette.t1 } else { palette.t4 };
    let mut row = button(text(action.label()).size(theme::SIZE_BODY).color(color))
        .width(Length::Fill)
        .padding(Padding::from([6.0, 10.0]))
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: (enabled && hovered).then(|| palette.ov(0.08).into()),
                text_color: color,
                border: theme::border(iced::Color::TRANSPARENT, 0.0, theme::RADIUS_ROW),
                ..button::Style::default()
            }
        });
    if enabled {
        row = row.on_press(ShellMessage::ContextMenuAction(action));
    }
    row.into()
}

fn separator<'a>(palette: Palette) -> Element<'a, ShellMessage> {
    let line = container(Space::new().width(Length::Fill).height(Length::Fixed(1.0))).style(
        move |_theme| container::Style {
            background: Some(palette.ov(0.08).into()),
            ..Default::default()
        },
    );
    container(line)
        .padding(Padding {
            top: 4.0,
            bottom: 4.0,
            left: 6.0,
            right: 6.0,
        })
        .into()
}
