//! Terminal-adjacent overlay surfaces: the find bar, the copy-mode indicator,
//! and the notifications slide-over. All consume [`crate::theme`] tokens.

use crate::iced_shell::ShellMessage;
use crate::theme::{self, Palette};
use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding};
use pandamux_core::NotificationSource;

// ---------------------------------------------------------------------------
// Find bar
// ---------------------------------------------------------------------------

/// Find-in-terminal overlay state. The runtime owns the authoritative matches;
/// this carries just what the bar renders.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FindViewState {
    pub open: bool,
    pub query: String,
    pub case_sensitive: bool,
    pub match_count: usize,
    /// 1-based index of the current match, or 0 when there are none.
    pub current: usize,
    /// The current match span on the focused terminal's visible screen,
    /// `(line, start, end)`, highlighted by the viewport.
    pub current_match: Option<(usize, usize, usize)>,
}

pub fn find_bar<'a>(state: &FindViewState, palette: Palette) -> Element<'a, ShellMessage> {
    let input = text_input("Find in terminal", &state.query)
        .on_input(ShellMessage::FindQueryChanged)
        .on_submit(ShellMessage::FindNext)
        .size(theme::SIZE_BODY)
        .padding(Padding::from([4.0, 8.0]))
        .width(Length::Fixed(260.0))
        .style(move |_theme, _status| text_input::Style {
            background: palette.ov(0.05).into(),
            border: theme::border(palette.ov(0.1), 1.0, theme::RADIUS_ROW),
            icon: palette.t3,
            placeholder: palette.t4,
            value: palette.t1,
            selection: palette.accent_alpha(0.35),
        });

    let count = if state.match_count == 0 {
        "0/0".to_string()
    } else {
        format!("{}/{}", state.current, state.match_count)
    };

    let bar = row![
        input,
        text(count)
            .size(theme::SIZE_METADATA)
            .font(theme::mono(iced::font::Weight::Medium))
            .color(palette.t3),
        pill_button(
            "Aa",
            state.case_sensitive,
            palette,
            ShellMessage::FindCaseToggled
        ),
        pill_button("\u{2191}", false, palette, ShellMessage::FindPrev),
        pill_button("\u{2193}", false, palette, ShellMessage::FindNext),
        Space::new().width(Length::Fill),
        pill_button("\u{00d7}", false, palette, ShellMessage::FindClosed),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .width(Length::Fill);

    container(bar)
        .padding(Padding::from([6.0, 8.0]))
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(palette.panel.into()),
            border: theme::border(palette.ov(0.08), 1.0, theme::RADIUS_ROW),
            ..Default::default()
        })
        .into()
}

/// A generic destructive-action confirmation (spec 1.5 close-all, spec 2.6
/// close-running-tab). Cancel dismisses; the confirm button fires
/// [`ShellMessage::ConfirmAccepted`] and the runtime runs whatever it parked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmViewState {
    pub title: String,
    pub body: String,
    pub action_label: String,
}

pub fn confirm_modal<'a>(
    state: &'a ConfirmViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let cancel = button(text("Cancel").size(theme::SIZE_BODY).color(palette.t2))
        .padding(Padding::from([6.0, 14.0]))
        .on_press(ShellMessage::OverlayDismissed)
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: hovered.then(|| palette.ov(0.08).into()),
                text_color: palette.t2,
                border: theme::border(palette.ov(0.12), 1.0, theme::RADIUS_ROW),
                ..Default::default()
            }
        });
    let confirm = button(
        text(state.action_label.clone())
            .size(theme::SIZE_BODY)
            .color(palette.bgc),
    )
    .padding(Padding::from([6.0, 14.0]))
    .on_press(ShellMessage::ConfirmAccepted)
    .style(move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(
                theme::with_alpha(palette.accent, if hovered { 1.0 } else { 0.9 }).into(),
            ),
            text_color: palette.bgc,
            border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_ROW),
            ..Default::default()
        }
    });

    let card = container(
        column![
            text(state.title.clone())
                .size(theme::SIZE_TITLE)
                .font(theme::ui(iced::font::Weight::Semibold))
                .color(palette.t1),
            text(state.body.clone())
                .size(theme::SIZE_BODY)
                .color(palette.t3),
            row![Space::new().width(Length::Fill), cancel, confirm]
                .spacing(8)
                .align_y(Alignment::Center),
        ]
        .spacing(12)
        .padding(16)
        .width(Length::Fixed(380.0)),
    )
    .width(Length::Fixed(380.0))
    .style(move |_theme| crate::command_palette::overlay_card_style(palette));

    crate::command_palette::modal(card, palette, Alignment::Center)
}

pub fn copy_mode_indicator<'a>(palette: Palette) -> Element<'a, ShellMessage> {
    container(
        text("COPY MODE  \u{2022}  hjkl / arrows to move  \u{2022}  Esc to exit")
            .size(theme::SIZE_SECONDARY)
            .font(theme::ui(iced::font::Weight::Semibold))
            .color(palette.accent),
    )
    .padding(Padding::from([4.0, 10.0]))
    .width(Length::Fill)
    .align_x(Alignment::Center)
    .style(move |_theme| container::Style {
        background: Some(palette.accent_alpha(0.12).into()),
        border: theme::border(palette.accent_alpha(0.3), 1.0, theme::RADIUS_ROW),
        ..Default::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// Notifications slide-over
// ---------------------------------------------------------------------------

/// One notification card as the panel renders it (a UI projection of a core
/// `NotificationInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationCard {
    pub id: String,
    pub title: String,
    pub body: String,
    pub source: NotificationSource,
    pub read: bool,
    /// Pre-formatted relative age, e.g. "2m ago".
    pub age: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NotificationsViewState {
    pub open: bool,
    pub cards: Vec<NotificationCard>,
}

pub fn notifications_panel<'a>(
    state: &'a NotificationsViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let header = row![
        text("Notifications")
            .size(theme::SIZE_TITLE)
            .font(theme::ui(iced::font::Weight::Semibold))
            .color(palette.t1),
        Space::new().width(Length::Fill),
        button(
            text("Clear all")
                .size(theme::SIZE_SECONDARY)
                .color(palette.accent)
        )
        .padding(Padding::from([2.0, 6.0]))
        .on_press(ShellMessage::NotificationsClearedAll)
        .style(move |_theme, _status| button::Style {
            background: None,
            text_color: palette.accent,
            border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_CHIP),
            ..Default::default()
        }),
    ]
    .align_y(Alignment::Center);

    let mut body = column![header].spacing(8).width(Length::Fill);

    if state.cards.is_empty() {
        body = body.push(
            text("No notifications")
                .size(theme::SIZE_BODY)
                .color(palette.t4),
        );
    } else {
        for card in &state.cards {
            body = body.push(notification_card(card, palette));
        }
    }

    container(body)
        .padding(12)
        .width(Length::Fixed(320.0))
        .style(move |_theme| container::Style {
            background: Some(palette.panel.into()),
            border: theme::border(palette.ov(0.08), 1.0, theme::RADIUS_OVERLAY),
            shadow: theme::overlay_shadow(),
            ..Default::default()
        })
        .into()
}

fn notification_card<'a>(
    card: &'a NotificationCard,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let dot_color = source_color(card.source, palette);
    let head = row![
        source_dot(dot_color),
        text(card.title.clone())
            .size(theme::SIZE_BODY)
            .font(theme::ui(iced::font::Weight::Medium))
            .color(palette.t1),
        Space::new().width(Length::Fill),
        text(card.age.clone())
            .size(theme::SIZE_METADATA)
            .font(theme::mono(iced::font::Weight::Normal))
            .color(palette.t4),
        clear_button(card.id.clone(), palette),
    ]
    .spacing(7)
    .align_y(Alignment::Center);

    let card_body = column![
        head,
        text(card.body.clone())
            .size(theme::SIZE_SECONDARY)
            .color(palette.t3),
    ]
    .spacing(4);

    container(card_body)
        .padding(10)
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(palette.ov(0.03).into()),
            border: theme::border(palette.ov(0.05), 1.0, 10.0),
            ..Default::default()
        })
        .into()
}

fn source_color(source: NotificationSource, palette: Palette) -> Color {
    match source {
        NotificationSource::Build => palette.accent,
        NotificationSource::Agent => palette.shell_ssh, // gold
        NotificationSource::Deploy => palette.shell_wsl, // green
        NotificationSource::Port => palette.shell_cmd,  // gray
        NotificationSource::Generic => palette.t3,
    }
}

fn source_dot<'a>(color: Color) -> Element<'a, ShellMessage> {
    container(
        Space::new()
            .width(Length::Fixed(7.0))
            .height(Length::Fixed(7.0)),
    )
    .style(move |_theme| container::Style {
        background: Some(color.into()),
        border: theme::border(Color::TRANSPARENT, 0.0, 4.0),
        ..Default::default()
    })
    .into()
}

fn clear_button<'a>(id: String, palette: Palette) -> Element<'a, ShellMessage> {
    button(
        text("\u{00d7}")
            .size(theme::SIZE_SECONDARY)
            .color(palette.t4),
    )
    .padding(Padding::from([0.0, 4.0]))
    .on_press(ShellMessage::NotificationCleared(id))
    .style(move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then(|| palette.ov(0.08).into()),
            text_color: palette.t3,
            border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_CHIP),
            ..Default::default()
        }
    })
    .into()
}

fn pill_button<'a>(
    label: &'a str,
    active: bool,
    palette: Palette,
    message: ShellMessage,
) -> Element<'a, ShellMessage> {
    button(text(label.to_string()).size(theme::SIZE_METADATA))
        .padding(Padding::from([3.0, 7.0]))
        .on_press(message)
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let background = if active {
                Some(palette.accent_alpha(0.14).into())
            } else if hovered {
                Some(palette.ov(0.08).into())
            } else {
                None
            };
            button::Style {
                background,
                text_color: if active { palette.accent } else { palette.t3 },
                border: theme::border(
                    if active {
                        palette.accent_alpha(0.3)
                    } else {
                        Color::TRANSPARENT
                    },
                    1.0,
                    theme::RADIUS_CHIP,
                ),
                ..Default::default()
            }
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Accent, UiTheme};

    fn palette() -> Palette {
        Palette::new(UiTheme::Dark, Accent::Teal)
    }

    #[test]
    fn builds_find_bar() {
        let state = FindViewState {
            open: true,
            query: "error".to_string(),
            case_sensitive: true,
            match_count: 12,
            current: 3,
            current_match: Some((2, 0, 5)),
        };
        let _bar = find_bar(&state, palette());
    }

    #[test]
    fn builds_notifications_panel_empty_and_populated() {
        let empty = NotificationsViewState::default();
        let _panel = notifications_panel(&empty, palette());

        let populated = NotificationsViewState {
            open: true,
            cards: vec![NotificationCard {
                id: "notif-1".to_string(),
                title: "Build finished".to_string(),
                body: "cargo build succeeded".to_string(),
                source: NotificationSource::Build,
                read: false,
                age: "2m ago".to_string(),
            }],
        };
        let _panel = notifications_panel(&populated, palette());
    }

    #[test]
    fn builds_copy_mode_indicator() {
        let _indicator = copy_mode_indicator(palette());
    }
}
