//! The command palette (Ctrl+K) and the quick-launch popover.
//!
//! Both are centered/anchored list pickers over a scrim. The palette runs a live
//! substring filter over commands, session switches, and theme switches; each
//! row carries the [`ShellMessage`] it dispatches. The quick-launch popover lists
//! shell profiles that create a new session. One overlay shows at a time and a
//! backdrop click dismisses it (see [`crate::chrome::Overlay`]).

use crate::iced_shell::ShellMessage;
use crate::theme::{self, Palette, ShellKind};
use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding};

/// One palette row: a glyph, a label, an optional shortcut chip, and the action
/// it dispatches when activated.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteItem {
    pub glyph: String,
    pub label: String,
    pub shortcut: Option<String>,
    pub action: ShellMessage,
}

impl PaletteItem {
    pub fn new(
        glyph: impl Into<String>,
        label: impl Into<String>,
        shortcut: Option<&str>,
        action: ShellMessage,
    ) -> Self {
        Self {
            glyph: glyph.into(),
            label: label.into(),
            shortcut: shortcut.map(str::to_string),
            action,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PaletteViewState {
    pub query: String,
    /// The filtered, ready-to-render items (the runtime filters against `query`).
    pub items: Vec<PaletteItem>,
    /// Index of the highlighted item (Enter activates it).
    pub selected: usize,
}

/// A shell profile the quick-launch popover can start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickLaunchProfile {
    pub label: String,
    pub detail: String,
    pub kind: ShellKind,
    pub shell: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickLaunchViewState {
    pub profiles: Vec<QuickLaunchProfile>,
}

impl Default for QuickLaunchViewState {
    fn default() -> Self {
        Self {
            profiles: default_profiles(),
        }
    }
}

/// The standard local shell profiles. SSH hosts (imported from `~/.ssh/config`)
/// join this list when the SSH connection manager lands in Phase 6.
pub fn default_profiles() -> Vec<QuickLaunchProfile> {
    vec![
        QuickLaunchProfile {
            label: "PowerShell 7".to_string(),
            detail: "pwsh".to_string(),
            kind: ShellKind::PowerShell,
            shell: "pwsh".to_string(),
        },
        QuickLaunchProfile {
            label: "Windows PowerShell".to_string(),
            detail: "powershell 5.1".to_string(),
            kind: ShellKind::PowerShell,
            shell: "powershell".to_string(),
        },
        QuickLaunchProfile {
            label: "Command Prompt".to_string(),
            detail: "cmd.exe".to_string(),
            kind: ShellKind::Cmd,
            shell: "cmd".to_string(),
        },
        QuickLaunchProfile {
            label: "WSL".to_string(),
            detail: "default distro".to_string(),
            kind: ShellKind::Wsl,
            shell: "wsl.exe".to_string(),
        },
    ]
}

/// Filter `all` items by a case-insensitive substring of the label.
pub fn filter_items(all: &[PaletteItem], query: &str) -> Vec<PaletteItem> {
    if query.trim().is_empty() {
        return all.to_vec();
    }
    let needle = query.to_ascii_lowercase();
    all.iter()
        .filter(|item| item.label.to_ascii_lowercase().contains(&needle))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

pub fn command_palette<'a>(
    state: &'a PaletteViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let input = text_input("Type a command or session...", &state.query)
        .on_input(ShellMessage::PaletteQueryChanged)
        .on_submit(ShellMessage::PaletteActivate)
        .size(theme::SIZE_BODY)
        .padding(Padding::from([8.0, 10.0]))
        .width(Length::Fill)
        .style(move |_theme, _status| text_input::Style {
            background: palette.ov(0.05).into(),
            border: theme::border(palette.ov(0.1), 1.0, theme::RADIUS_ROW),
            icon: palette.t3,
            placeholder: palette.t4,
            value: palette.t1,
            selection: palette.accent_alpha(0.35),
        });

    let mut list = column![].spacing(2).width(Length::Fill);
    if state.items.is_empty() {
        list = list.push(
            container(
                text("No matching commands")
                    .size(theme::SIZE_BODY)
                    .color(palette.t4),
            )
            .padding(Padding::from([8.0, 8.0])),
        );
    } else {
        for (index, item) in state.items.iter().enumerate() {
            list = list.push(palette_row(item, index == state.selected, palette));
        }
    }

    let card = column![
        input,
        scrollable(list)
            .height(Length::Fixed(320.0))
            .width(Length::Fill),
    ]
    .spacing(10)
    .padding(12)
    .width(Length::Fixed(560.0));

    modal(
        container(card)
            .width(Length::Fixed(560.0))
            .style(move |_theme| overlay_card_style(palette)),
        palette,
        Alignment::Start,
    )
}

fn palette_row<'a>(
    item: &'a PaletteItem,
    selected: bool,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let mut content = row![
        container(
            text(item.glyph.clone())
                .size(theme::SIZE_BODY)
                .color(palette.t3)
        )
        .width(Length::Fixed(22.0))
        .align_x(Alignment::Center),
        text(item.label.clone())
            .size(theme::SIZE_BODY)
            .color(palette.t1),
        Space::new().width(Length::Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    if let Some(shortcut) = &item.shortcut {
        content = content.push(kbd_chip(shortcut, palette));
    }

    button(content)
        .padding(Padding::from([7.0, 8.0]))
        .width(Length::Fill)
        .on_press(item.action.clone())
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(
                    if selected || hovered {
                        palette.accent_alpha(0.1)
                    } else {
                        Color::TRANSPARENT
                    }
                    .into(),
                ),
                text_color: palette.t1,
                border: theme::border(
                    if selected {
                        palette.accent_alpha(0.25)
                    } else {
                        Color::TRANSPARENT
                    },
                    1.0,
                    theme::RADIUS_ROW,
                ),
                ..Default::default()
            }
        })
        .into()
}

pub fn quick_launch<'a>(
    state: &'a QuickLaunchViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let mut list = column![
        text("New session")
            .size(theme::SIZE_GROUP_HEADER)
            .font(theme::ui(iced::font::Weight::Semibold))
            .color(palette.t4),
    ]
    .spacing(4)
    .width(Length::Fill);

    for profile in &state.profiles {
        list = list.push(quick_launch_row(profile, palette));
    }

    let card = container(column![list].padding(10).width(Length::Fixed(300.0)))
        .width(Length::Fixed(300.0))
        .style(move |_theme| overlay_card_style(palette));

    modal(card, palette, Alignment::Start)
}

fn quick_launch_row<'a>(
    profile: &'a QuickLaunchProfile,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let shell_color = palette.shell_color(profile.kind);
    let content = row![
        container(
            text(profile.kind.abbreviation())
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
        }),
        column![
            text(profile.label.clone())
                .size(theme::SIZE_BODY)
                .color(palette.t1),
            text(profile.detail.clone())
                .size(theme::SIZE_METADATA)
                .font(theme::mono(iced::font::Weight::Normal))
                .color(palette.t4),
        ]
        .spacing(2),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    button(content)
        .padding(Padding::from([7.0, 8.0]))
        .width(Length::Fill)
        .on_press(ShellMessage::LaunchProfile {
            shell: profile.shell.clone(),
            title: profile.label.clone(),
        })
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: if hovered {
                    Some(palette.ov(0.05).into())
                } else {
                    None
                },
                text_color: palette.t1,
                border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_ROW),
                ..Default::default()
            }
        })
        .into()
}

// ---------------------------------------------------------------------------
// Shared overlay chrome
// ---------------------------------------------------------------------------

/// Wrap `card` in a full-size scrim that dismisses the overlay on a backdrop
/// click and centers the card horizontally near the top. `align_y` positions the
/// card vertically (Start keeps it near the top like the design).
pub(crate) fn modal<'a>(
    card: impl Into<Element<'a, ShellMessage>>,
    palette: Palette,
    align_y: Alignment,
) -> Element<'a, ShellMessage> {
    use iced::widget::mouse_area;

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(palette.scrim.into()),
                ..Default::default()
            }),
    )
    .on_press(ShellMessage::OverlayDismissed);

    // The card itself absorbs clicks (its own mouse_area) so a click inside does
    // not fall through to the backdrop dismiss.
    let centered = container(mouse_area(card).on_press(ShellMessage::Noop))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(align_y)
        .padding(Padding {
            top: theme::TITLEBAR_HEIGHT + 60.0,
            right: 12.0,
            bottom: 12.0,
            left: 12.0,
        });

    iced::widget::stack![backdrop, centered].into()
}

pub(crate) fn overlay_card_style(palette: Palette) -> container::Style {
    container::Style {
        background: Some(palette.panel.into()),
        border: theme::border(palette.ov(0.1), 1.0, theme::RADIUS_OVERLAY),
        shadow: theme::overlay_shadow(),
        ..Default::default()
    }
}

fn kbd_chip<'a>(label: &str, palette: Palette) -> Element<'a, ShellMessage> {
    container(
        text(label.to_string())
            .size(theme::SIZE_KBD)
            .font(theme::mono(iced::font::Weight::Medium))
            .color(palette.t4),
    )
    .padding(Padding::from([1.0, 5.0]))
    .style(move |_theme| container::Style {
        background: Some(palette.ov(0.05).into()),
        border: theme::border(palette.ov(0.08), 1.0, theme::RADIUS_CHIP),
        ..Default::default()
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

    fn sample_items() -> Vec<PaletteItem> {
        vec![
            PaletteItem::new(
                "\u{2699}",
                "Open Settings",
                Some("Ctrl ,"),
                ShellMessage::Noop,
            ),
            PaletteItem::new(
                "\u{25d1}",
                "Toggle Theme",
                Some("Ctrl Shift T"),
                ShellMessage::ToggleTheme,
            ),
            PaletteItem::new(
                "\u{2637}",
                "Toggle Status Bar",
                Some("Ctrl B"),
                ShellMessage::ToggleStatusBar,
            ),
        ]
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let all = sample_items();
        assert_eq!(filter_items(&all, "").len(), 3);
        assert_eq!(filter_items(&all, "toggle").len(), 2);
        let theme = filter_items(&all, "THEME");
        assert_eq!(theme.len(), 1);
        assert_eq!(theme[0].label, "Toggle Theme");
        assert!(filter_items(&all, "zzz").is_empty());
    }

    #[test]
    fn builds_palette_and_quick_launch() {
        let state = PaletteViewState {
            query: "tog".to_string(),
            items: filter_items(&sample_items(), "tog"),
            selected: 0,
        };
        let _palette_view = command_palette(&state, palette());
        let quick_state = QuickLaunchViewState::default();
        let _quick = quick_launch(&quick_state, palette());
    }

    #[test]
    fn quick_launch_has_local_profiles() {
        let state = QuickLaunchViewState::default();
        assert!(
            state
                .profiles
                .iter()
                .any(|p| p.kind == ShellKind::PowerShell)
        );
        assert!(state.profiles.iter().any(|p| p.kind == ShellKind::Wsl));
        assert!(state.profiles.iter().any(|p| p.kind == ShellKind::Cmd));
    }
}
