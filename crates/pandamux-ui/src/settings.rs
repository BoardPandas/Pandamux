//! The settings modal: a 640x440 card on a scrim with a left nav
//! (General / Terminal / Keyboard / Notifications / Quick launch) and control
//! rows. The controls that map to live chrome state (UI theme, accent, show
//! status bar) mutate it immediately; sections without backing state yet present
//! their settings read-only and are labeled as such.

use crate::command_palette::{modal, overlay_card_style};
use crate::iced_shell::ShellMessage;
use crate::theme::{self, Accent, Palette, UiTheme};
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Color, Element, Length, Padding};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsSection {
    #[default]
    General,
    Terminal,
    Keyboard,
    Notifications,
    QuickLaunch,
}

impl SettingsSection {
    pub const ALL: [SettingsSection; 5] = [
        SettingsSection::General,
        SettingsSection::Terminal,
        SettingsSection::Keyboard,
        SettingsSection::Notifications,
        SettingsSection::QuickLaunch,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsSection::General => "General",
            SettingsSection::Terminal => "Terminal",
            SettingsSection::Keyboard => "Keyboard",
            SettingsSection::Notifications => "Notifications",
            SettingsSection::QuickLaunch => "Quick launch",
        }
    }
}

/// Which Terminal-tab toggle was pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalToggle {
    WelcomePrompt,
    RightClickPaste,
    ConfirmClose,
}

/// View state for the settings modal, projected from live chrome state and
/// the persistent user settings.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsViewState {
    pub section: SettingsSection,
    pub ui_theme: UiTheme,
    pub accent: Accent,
    pub show_status_bar: bool,
    /// The bound keyboard shortcuts (label, chord), shown on the Keyboard
    /// tab. Filled by the runtime FROM the live keymap (spec 2.6): the render
    /// source is the decode source, so this list can never drift again.
    pub shortcuts: Vec<(String, String)>,
    /// Persistent terminal settings (spec 1.2 / 1.3 / 2.6 / 2.7 toggles).
    pub terminal: pandamux_core::TerminalSettings,
    /// In-progress text of the scrollback-lines input.
    pub scrollback_input: String,
}

impl Default for SettingsViewState {
    fn default() -> Self {
        let terminal = pandamux_core::TerminalSettings::default();
        Self {
            section: SettingsSection::default(),
            ui_theme: UiTheme::default(),
            accent: Accent::default(),
            show_status_bar: true,
            shortcuts: Vec::new(),
            scrollback_input: terminal.scrollback_lines.to_string(),
            terminal,
        }
    }
}

pub fn settings_modal<'a>(
    state: &'a SettingsViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let nav = settings_nav(state.section, palette);
    let content = scrollable(section_content(state, palette))
        .height(Length::Fill)
        .width(Length::Fill);

    let header = row![
        text("Settings")
            .size(theme::SIZE_TITLE)
            .font(theme::ui(iced::font::Weight::Semibold))
            .color(palette.t1),
        Space::new().width(Length::Fill),
        button(text("\u{00d7}").size(theme::SIZE_TITLE).color(palette.t3))
            .padding(Padding::from([2.0, 8.0]))
            .on_press(ShellMessage::OverlayDismissed)
            .style(move |_theme, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: hovered.then(|| palette.ov(0.08).into()),
                    text_color: palette.t2,
                    border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_CHIP),
                    ..Default::default()
                }
            }),
    ]
    .align_y(Alignment::Center);

    let body = row![
        nav,
        container(content)
            .padding(Padding::from([4.0, 14.0]))
            .width(Length::Fill)
            .height(Length::Fill),
    ]
    .spacing(0)
    .height(Length::Fill);

    let card = container(
        column![header, body]
            .spacing(12)
            .padding(14)
            .width(Length::Fixed(640.0))
            .height(Length::Fixed(440.0)),
    )
    .width(Length::Fixed(640.0))
    .height(Length::Fixed(440.0))
    .style(move |_theme| overlay_card_style(palette));

    modal(card, palette, Alignment::Center)
}

fn settings_nav<'a>(active: SettingsSection, palette: Palette) -> Element<'a, ShellMessage> {
    let mut nav = column![].spacing(2).width(Length::Fixed(150.0));
    for section in SettingsSection::ALL {
        let is_active = section == active;
        nav = nav.push(
            button(
                text(section.label())
                    .size(theme::SIZE_BODY)
                    .color(if is_active { palette.t1 } else { palette.t3 }),
            )
            .padding(Padding::from([6.0, 10.0]))
            .width(Length::Fill)
            .on_press(ShellMessage::SettingsSectionSelected(section))
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
                    text_color: if is_active { palette.t1 } else { palette.t3 },
                    border: theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_ROW),
                    ..Default::default()
                }
            }),
        );
    }

    container(nav)
        .padding(Padding::from([0.0, 8.0]))
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            border: theme::border(palette.ov(0.06), 0.0, 0.0),
            ..Default::default()
        })
        .into()
}

fn section_content<'a>(
    state: &'a SettingsViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    match state.section {
        SettingsSection::General => general_section(state, palette),
        SettingsSection::Terminal => terminal_section(state, palette),
        SettingsSection::Keyboard => keyboard_section(state, palette),
        SettingsSection::Notifications => note_section(
            "Notification sound, OS toast, and per-source filters arrive with the toast bridge.",
            palette,
        ),
        SettingsSection::QuickLaunch => note_section(
            "Quick-launch profiles are managed from the launcher; SSH host import arrives with the SSH manager.",
            palette,
        ),
    }
}

fn general_section<'a>(
    state: &'a SettingsViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let theme_row = control_row(
        "UI theme",
        toggle_button(
            if state.ui_theme == UiTheme::Dark {
                "Dark"
            } else {
                "Light"
            },
            palette,
            ShellMessage::ToggleTheme,
        ),
        palette,
    );

    let mut accents = row![].spacing(6).align_y(Alignment::Center);
    for accent in [Accent::Teal, Accent::Gold, Accent::Blue, Accent::Mauve] {
        accents = accents.push(accent_swatch(accent, accent == state.accent, palette));
    }
    let accent_row = control_row("Accent", accents.into(), palette);

    let status_row = control_row(
        "Show status bar",
        toggle_button(
            if state.show_status_bar { "On" } else { "Off" },
            palette,
            ShellMessage::ToggleStatusBar,
        ),
        palette,
    );

    column![theme_row, accent_row, status_row]
        .spacing(4)
        .width(Length::Fill)
        .into()
}

fn terminal_section<'a>(
    state: &'a SettingsViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let scrollback = control_row(
        "Scrollback lines",
        iced::widget::text_input("10000", &state.scrollback_input)
            .on_input(ShellMessage::ScrollbackLinesChanged)
            .size(theme::SIZE_SECONDARY)
            .width(Length::Fixed(90.0))
            .into(),
        palette,
    );
    let on_off = |value: bool| if value { "On" } else { "Off" };
    let welcome = control_row(
        "Tool chooser in new terminals",
        toggle_button(
            on_off(state.terminal.welcome_prompt_enabled),
            palette,
            ShellMessage::TerminalSettingToggled(TerminalToggle::WelcomePrompt),
        ),
        palette,
    );
    let right_click = control_row(
        "Right-click pastes (classic conhost)",
        toggle_button(
            on_off(state.terminal.right_click_paste_optin),
            palette,
            ShellMessage::TerminalSettingToggled(TerminalToggle::RightClickPaste),
        ),
        palette,
    );
    let confirm_close = control_row(
        "Confirm closing a running tab",
        toggle_button(
            on_off(state.terminal.confirm_close_on_running),
            palette,
            ShellMessage::TerminalSettingToggled(TerminalToggle::ConfirmClose),
        ),
        palette,
    );
    column![scrollback, welcome, right_click, confirm_close]
        .spacing(4)
        .width(Length::Fill)
        .into()
}

fn keyboard_section<'a>(
    state: &'a SettingsViewState,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let mut list = column![].spacing(4).width(Length::Fill);
    for (label, chord) in &state.shortcuts {
        list = list.push(control_row(label, kbd_chip(chord, palette), palette));
    }
    list.into()
}

fn note_section<'a>(message: &'a str, palette: Palette) -> Element<'a, ShellMessage> {
    container(text(message).size(theme::SIZE_BODY).color(palette.t3))
        .padding(Padding::from([8.0, 0.0]))
        .into()
}

fn control_row<'a>(
    label: &'a str,
    control: Element<'a, ShellMessage>,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    row![
        text(label.to_string())
            .size(theme::SIZE_BODY)
            .color(palette.t2),
        Space::new().width(Length::Fill),
        control,
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .padding(Padding::from([6.0, 0.0]))
    .width(Length::Fill)
    .into()
}

fn toggle_button<'a>(
    label: &'a str,
    palette: Palette,
    message: ShellMessage,
) -> Element<'a, ShellMessage> {
    button(
        text(label.to_string())
            .size(theme::SIZE_SECONDARY)
            .color(palette.t1),
    )
    .padding(Padding::from([4.0, 12.0]))
    .on_press(message)
    .style(move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(palette.ov(if hovered { 0.1 } else { 0.06 }).into()),
            text_color: palette.t1,
            border: theme::border(palette.ov(0.1), 1.0, theme::RADIUS_CHIP),
            ..Default::default()
        }
    })
    .into()
}

fn accent_swatch<'a>(
    accent: Accent,
    selected: bool,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let color = accent.color();
    button(
        Space::new()
            .width(Length::Fixed(18.0))
            .height(Length::Fixed(18.0)),
    )
    .padding(0.0)
    .on_press(ShellMessage::AccentSelected(accent))
    .style(move |_theme, _status| button::Style {
        background: Some(color.into()),
        border: theme::border(
            if selected {
                palette.t1
            } else {
                theme::with_alpha(color, 0.4)
            },
            if selected { 2.0 } else { 1.0 },
            9.0,
        ),
        ..Default::default()
    })
    .into()
}

fn kbd_chip<'a>(label: &str, palette: Palette) -> Element<'a, ShellMessage> {
    container(
        text(label.to_string())
            .size(theme::SIZE_KBD)
            .font(theme::mono(iced::font::Weight::Medium))
            .color(palette.t3),
    )
    .padding(Padding::from([2.0, 6.0]))
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

    #[test]
    fn builds_all_settings_sections() {
        for section in SettingsSection::ALL {
            let state = SettingsViewState {
                section,
                ..SettingsViewState::default()
            };
            let _modal = settings_modal(&state, palette());
        }
    }

    #[test]
    fn keyboard_tab_renders_runtime_supplied_shortcuts() {
        // The list comes from the live keymap (spec 2.6); the tab just
        // renders whatever the runtime projected.
        let state = SettingsViewState {
            section: SettingsSection::Keyboard,
            shortcuts: pandamux_core::Keymap::defaults()
                .sections()
                .into_iter()
                .flat_map(|section| section.entries)
                .collect(),
            ..SettingsViewState::default()
        };
        assert!(
            state
                .shortcuts
                .iter()
                .any(|(label, _)| label == "Command palette")
        );
        let _modal = settings_modal(&state, palette());
    }
}
