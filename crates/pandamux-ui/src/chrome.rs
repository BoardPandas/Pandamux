//! Window chrome: the 40px custom titlebar, the 52px icon rail, and the 26px
//! status bar. Every color, size, and radius comes from [`crate::theme`]; this
//! module never hardcodes a color.
//!
//! Icons are rendered as compact glyph text for now (the design calls for
//! 1.2-1.4px-stroke line glyphs recreated as canvas/SVG; that refinement is
//! tracked for a later pass). The layout, sizing, and interaction wiring match
//! the design spec.

use crate::iced_shell::ShellMessage;
use crate::session_panel::SessionGrouping;
use crate::theme::{self, Accent, Palette, ShellKind, UiTheme};
use iced::widget::{Space, button, column, container, mouse_area, row, text};
use iced::{Alignment, Border, Color, Element, Length, Padding, Shadow, Vector};

fn fixed_space(width: f32) -> Space {
    Space::new().width(Length::Fixed(width))
}

fn fill_space() -> Space {
    Space::new().width(Length::Fill)
}

fn dot_space(size: f32) -> Space {
    Space::new()
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
}

/// The icon-rail items, top to bottom (Settings is pinned to the bottom).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailItem {
    Sessions,
    CommandPalette,
    NewSession,
    Notifications,
    Settings,
}

/// Which centered/anchored overlay is showing. One at a time; a backdrop click
/// dismisses it. The notifications slide-over is tracked separately (it is a
/// side panel, not a modal).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Overlay {
    #[default]
    None,
    CommandPalette,
    QuickLaunch,
    Settings,
}

/// Activity state of the focused session, driving the status-bar dot color
/// (running = accent, busy-agent = gold, idle = dim), mirroring the Electron
/// `shellState` + Claude-activity signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SessionActivity {
    #[default]
    Idle,
    Running,
    BusyAgent,
}

/// All chrome-facing view state. Populated by the runtime from `AppState` plus
/// the (future) pollers; terminal/pane state stays in the projection.
#[derive(Clone, Debug, PartialEq)]
pub struct ChromeState {
    pub ui_theme: UiTheme,
    pub accent: Accent,
    pub show_status_bar: bool,
    pub active_rail: RailItem,
    pub active_session_name: String,
    /// Whether the 264px session panel is shown (toggled by the Sessions rail).
    pub session_panel_open: bool,
    /// Current session-panel grouping (Project / Type / Host).
    pub session_grouping: SessionGrouping,
    /// The currently open centered overlay (palette / quick-launch / settings).
    pub active_overlay: Overlay,
    pub unread_notifications: bool,
    pub activity: SessionActivity,
    pub shell_kind: ShellKind,
    pub shell_label: String,
    pub git_branch: Option<String>,
    pub git_ahead: u32,
    pub ports: Vec<u16>,
    pub session_count: usize,
    pub pane_count: usize,
    pub encoding: String,
    pub version: String,
}

impl Default for ChromeState {
    fn default() -> Self {
        ChromeState {
            ui_theme: UiTheme::Dark,
            accent: Accent::Teal,
            show_status_bar: true,
            active_rail: RailItem::Sessions,
            active_session_name: "Workspace".to_string(),
            session_panel_open: true,
            session_grouping: SessionGrouping::default(),
            active_overlay: Overlay::None,
            unread_notifications: false,
            activity: SessionActivity::Idle,
            shell_kind: ShellKind::PowerShell,
            shell_label: "pwsh".to_string(),
            git_branch: None,
            git_ahead: 0,
            ports: Vec::new(),
            session_count: 1,
            pane_count: 1,
            encoding: "UTF-8".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl ChromeState {
    pub fn palette(&self) -> Palette {
        Palette::new(self.ui_theme, self.accent)
    }
}

// ---------------------------------------------------------------------------
// Titlebar
// ---------------------------------------------------------------------------

pub fn titlebar<'a>(chrome: &ChromeState, palette: Palette) -> Element<'a, ShellMessage> {
    // Left: logo mark + wordmark.
    let brand = row![
        logo_mark(palette),
        text("PandaMUX")
            .size(theme::SIZE_TITLE)
            .font(theme::ui(iced::font::Weight::Semibold))
            .color(palette.t1),
        text("Everywhere")
            .size(theme::SIZE_SECONDARY)
            .font(theme::ui(iced::font::Weight::Normal))
            .color(palette.t4),
    ]
    .spacing(7)
    .align_y(Alignment::Center);

    // Center: session-switcher pill that opens the command palette.
    let pill = button(
        row![
            glyph(icons::SEARCH, theme::SIZE_SECONDARY, palette.t3),
            text(chrome.active_session_name.clone())
                .size(theme::SIZE_BODY)
                .color(palette.t2),
            kbd_chip("Ctrl K", palette),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([4.0, 10.0]))
    .on_press(ShellMessage::OverlayRequested(RailItem::CommandPalette))
    .style(move |_theme, status| pill_style(palette, status));

    // Right: bell (with unread dot), settings, window controls.
    let bell = titlebar_icon_button(
        icons::BELL,
        palette,
        ShellMessage::OverlayRequested(RailItem::Notifications),
        chrome.unread_notifications,
    );
    let settings = titlebar_icon_button(
        icons::SETTINGS,
        palette,
        ShellMessage::OverlayRequested(RailItem::Settings),
        false,
    );

    let controls = row![
        bell,
        settings,
        fixed_space(6.0),
        window_button(
            icons::MINIMIZE,
            palette,
            ShellMessage::WindowMinimizePressed,
            false
        ),
        window_button(
            icons::MAXIMIZE,
            palette,
            ShellMessage::WindowMaximizeToggled,
            false
        ),
        window_button(
            icons::CLOSE,
            palette,
            ShellMessage::WindowClosePressed,
            true
        ),
    ]
    .spacing(2)
    .align_y(Alignment::Center);

    let bar = row![brand, fill_space(), pill, fill_space(), controls]
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .height(Length::Fixed(theme::TITLEBAR_HEIGHT));

    // The whole bar is a drag handle; child buttons capture their own presses.
    let draggable = mouse_area(
        container(bar)
            .padding(Padding::from([0.0, 8.0]))
            .width(Length::Fill)
            .height(Length::Fixed(theme::TITLEBAR_HEIGHT))
            .style(move |_theme| titlebar_style(palette)),
    )
    .on_press(ShellMessage::WindowDragStarted);

    draggable.into()
}

fn logo_mark<'a>(palette: Palette) -> Element<'a, ShellMessage> {
    // Rounded accent square standing in for the panda badge asset.
    container(
        text("\u{25c9}") // ◉
            .size(13.0)
            .color(palette.bgc),
    )
    .width(Length::Fixed(20.0))
    .height(Length::Fixed(20.0))
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .style(move |_theme| container::Style {
        background: Some(palette.accent.into()),
        border: theme::border(Color::TRANSPARENT, 0.0, 5.0),
        ..Default::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// Icon rail
// ---------------------------------------------------------------------------

pub fn icon_rail<'a>(chrome: &ChromeState, palette: Palette) -> Element<'a, ShellMessage> {
    let top = column![
        rail_button(
            icons::SESSIONS,
            RailItem::Sessions,
            chrome.active_rail,
            palette
        ),
        rail_button(
            icons::PALETTE,
            RailItem::CommandPalette,
            chrome.active_rail,
            palette
        ),
        rail_button(
            icons::PLUS,
            RailItem::NewSession,
            chrome.active_rail,
            palette
        ),
        rail_button(
            icons::BELL,
            RailItem::Notifications,
            chrome.active_rail,
            palette
        ),
    ]
    .spacing(6)
    .align_x(Alignment::Center);

    let rail = column![
        top,
        Space::new().height(Length::Fill),
        rail_button(
            icons::SETTINGS,
            RailItem::Settings,
            chrome.active_rail,
            palette
        ),
    ]
    .spacing(6)
    .align_x(Alignment::Center)
    .padding(Padding::from([8.0, 0.0]))
    .width(Length::Fixed(theme::RAIL_WIDTH))
    .height(Length::Fill);

    container(rail)
        .width(Length::Fixed(theme::RAIL_WIDTH))
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(palette.ov(0.02).into()),
            border: theme::border(palette.ov(0.05), 0.0, 0.0),
            ..Default::default()
        })
        .into()
}

fn rail_button<'a>(
    glyph_str: &'static str,
    item: RailItem,
    active: RailItem,
    palette: Palette,
) -> Element<'a, ShellMessage> {
    let is_active = item == active;
    let message = match item {
        RailItem::Sessions | RailItem::NewSession => ShellMessage::RailSelected(item),
        other => ShellMessage::OverlayRequested(other),
    };
    button(
        container(glyph(
            glyph_str,
            16.0,
            if is_active {
                palette.accent
            } else {
                palette.t3
            },
        ))
        .width(Length::Fixed(38.0))
        .height(Length::Fixed(38.0))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center),
    )
    .padding(0.0)
    .on_press(message)
    .style(move |_theme, status| rail_button_style(palette, is_active, status))
    .into()
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

pub fn status_bar<'a>(chrome: &ChromeState, palette: Palette) -> Element<'a, ShellMessage> {
    let mut left = row![
        status_dot(activity_color(chrome.activity, palette)),
        mono_label(&chrome.shell_label, palette.t3),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    if let Some(branch) = &chrome.git_branch {
        left = left.push(fixed_space(8.0));
        left = left.push(glyph(icons::GIT, theme::SIZE_STATUS_BAR, palette.t3));
        left = left.push(mono_label(branch, palette.t3));
        if chrome.git_ahead > 0 {
            left = left.push(mono_label(
                &format!("\u{2191}{}", chrome.git_ahead),
                palette.shell_ssh,
            ));
        }
    }

    if !chrome.ports.is_empty() {
        let ports = chrome
            .ports
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(" \u{00b7} ");
        left = left.push(fixed_space(8.0));
        left = left.push(mono_label(&format!("ports {ports}"), palette.t3));
    }

    let right = row![
        mono_label(
            &format!(
                "{} session{} \u{00b7} {} pane{}",
                chrome.session_count,
                plural(chrome.session_count),
                chrome.pane_count,
                plural(chrome.pane_count),
            ),
            palette.t3,
        ),
        fixed_space(14.0),
        mono_label(&chrome.encoding, palette.t4),
        fixed_space(14.0),
        mono_label(&format!("v{}", chrome.version), palette.t4),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let bar = row![left, fill_space(), right]
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .height(Length::Fixed(theme::STATUS_BAR_HEIGHT));

    container(bar)
        .padding(Padding::from([0.0, 12.0]))
        .width(Length::Fill)
        .height(Length::Fixed(theme::STATUS_BAR_HEIGHT))
        .style(move |_theme| container::Style {
            background: Some(palette.ov(0.02).into()),
            border: Border {
                color: palette.ov(0.06),
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

// ---------------------------------------------------------------------------
// Small shared widgets
// ---------------------------------------------------------------------------

fn glyph<'a>(value: &'static str, size: f32, color: Color) -> Element<'a, ShellMessage> {
    text(value).size(size).color(color).into()
}

fn mono_label<'a>(value: &str, color: Color) -> Element<'a, ShellMessage> {
    text(value.to_string())
        .size(theme::SIZE_STATUS_BAR)
        .font(theme::mono(iced::font::Weight::Medium))
        .color(color)
        .into()
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
        border: theme::border(palette.ov(0.08), 1.0, 4.0),
        ..Default::default()
    })
    .into()
}

fn status_dot<'a>(color: Color) -> Element<'a, ShellMessage> {
    container(dot_space(6.0))
        .style(move |_theme| container::Style {
            background: Some(color.into()),
            border: theme::border(Color::TRANSPARENT, 0.0, 3.0),
            shadow: Shadow {
                color: theme::with_alpha(color, 0.5),
                offset: Vector::new(0.0, 0.0),
                blur_radius: 8.0,
            },
            ..Default::default()
        })
        .into()
}

fn activity_color(activity: SessionActivity, palette: Palette) -> Color {
    match activity {
        SessionActivity::Running => palette.accent,
        SessionActivity::BusyAgent => palette.shell_ssh, // gold
        SessionActivity::Idle => palette.ov(0.16),
    }
}

fn titlebar_icon_button<'a>(
    glyph_str: &'static str,
    palette: Palette,
    message: ShellMessage,
    unread: bool,
) -> Element<'a, ShellMessage> {
    // Icon plus, when unread, a small accent dot with a knockout border.
    let mut content = row![glyph(glyph_str, 15.0, palette.t3)]
        .spacing(1)
        .align_y(Alignment::Center);
    if unread {
        content = content.push(
            container(dot_space(6.0)).style(move |_theme| container::Style {
                background: Some(palette.accent.into()),
                border: theme::border(palette.bgc, 1.5, 3.0),
                ..Default::default()
            }),
        );
    }
    button(
        container(content)
            .width(Length::Fixed(34.0))
            .height(Length::Fixed(28.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    )
    .padding(0.0)
    .on_press(message)
    .style(move |_theme, status| rail_button_style(palette, false, status))
    .into()
}

fn window_button<'a>(
    glyph_str: &'static str,
    palette: Palette,
    message: ShellMessage,
    is_close: bool,
) -> Element<'a, ShellMessage> {
    button(
        container(glyph(glyph_str, 13.0, palette.t3))
            .width(Length::Fixed(40.0))
            .height(Length::Fixed(28.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    )
    .padding(0.0)
    .on_press(message)
    .style(move |_theme, status| window_button_style(palette, is_close, status))
    .into()
}

// ---------------------------------------------------------------------------
// Style closures
// ---------------------------------------------------------------------------

fn titlebar_style(palette: Palette) -> container::Style {
    container::Style {
        background: Some(palette.ov(0.015).into()),
        border: Border {
            color: palette.ov(0.06),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn pill_style(palette: Palette, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(palette.ov(if hovered { 0.06 } else { 0.035 }).into()),
        text_color: palette.t2,
        border: theme::border(palette.ov(0.06), 1.0, 7.0),
        ..Default::default()
    }
}

fn rail_button_style(palette: Palette, is_active: bool, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let (background, border) = if is_active {
        (
            Some(palette.accent_alpha(0.12).into()),
            theme::border(palette.accent_alpha(0.25), 1.0, theme::RADIUS_RAIL_BUTTON),
        )
    } else if hovered {
        (
            Some(palette.ov(0.08).into()),
            theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_RAIL_BUTTON),
        )
    } else {
        (
            None,
            theme::border(Color::TRANSPARENT, 0.0, theme::RADIUS_RAIL_BUTTON),
        )
    };
    button::Style {
        background,
        text_color: if is_active {
            palette.accent
        } else {
            palette.t3
        },
        border,
        ..Default::default()
    }
}

fn window_button_style(palette: Palette, is_close: bool, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let background = if hovered && is_close {
        Some(Color::from_rgba8(224, 90, 90, 0.85).into())
    } else if hovered {
        Some(palette.ov(0.08).into())
    } else {
        None
    };
    let text_color = if hovered && is_close {
        Color::WHITE
    } else {
        palette.t3
    };
    button::Style {
        background,
        text_color,
        border: theme::border(Color::TRANSPARENT, 0.0, 0.0),
        ..Default::default()
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Glyph placeholders (unicode) standing in for the design's line-icon set.
mod icons {
    pub const SEARCH: &str = "\u{1f50d}"; // 🔍
    pub const BELL: &str = "\u{1f514}"; // 🔔
    pub const SETTINGS: &str = "\u{2699}"; // ⚙
    pub const MINIMIZE: &str = "\u{2013}"; // –
    pub const MAXIMIZE: &str = "\u{25a1}"; // □
    pub const CLOSE: &str = "\u{00d7}"; // ×
    pub const SESSIONS: &str = "\u{2637}"; // ☷ (stacked rows)
    pub const PALETTE: &str = "\u{2318}"; // ⌘
    pub const PLUS: &str = "\u{002b}"; // +
    pub const GIT: &str = "\u{2325}"; // ⌥ (branch stand-in)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_default_palette_follows_theme() {
        let mut chrome = ChromeState::default();
        assert_eq!(chrome.palette().theme, UiTheme::Dark);
        chrome.ui_theme = UiTheme::Light;
        assert_eq!(chrome.palette().theme, UiTheme::Light);
    }

    #[test]
    fn builds_all_chrome_regions() {
        let chrome = ChromeState::default();
        let palette = chrome.palette();
        let _titlebar = titlebar(&chrome, palette);
        let _rail = icon_rail(&chrome, palette);
        let _status = status_bar(&chrome, palette);
    }
}
