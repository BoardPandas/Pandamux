use gpui_kit::base::StyledExt as _;
use gpui_kit::component::TitleBar as GpuiTitleBar;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;

use crate::server_bridge::ServerStatus;
use crate::theme::{Radii, Spacing, Theme, Typography};

/// Renders the Section 12 40px custom frameless titlebar.
pub struct CustomTitlebar;

impl CustomTitlebar {
    /// Builds the 40px custom titlebar element.
    #[allow(clippy::too_many_arguments)]
    pub fn render<V: 'static>(
        theme: &Theme,
        server_status: &ServerStatus,
        active_title: Option<&str>,
        on_toggle_palette: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
        on_toggle_theme: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
        on_open_settings: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
        on_toggle_surfaces: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
        _window: &mut Window,
        cx: &mut Context<V>,
    ) -> impl IntoElement {
        let status_color = match server_status {
            ServerStatus::Connected { .. } => theme.terminal.success,
            ServerStatus::Connecting => theme.terminal.warn,
            ServerStatus::Disconnected | ServerStatus::Failed(_) => rgb(0xf87171),
        };

        let status_label = match server_status {
            ServerStatus::Connected { pid, .. } => format!("Hub PID {pid}"),
            ServerStatus::Connecting => "Connecting...".to_string(),
            ServerStatus::Disconnected => "Offline".to_string(),
            ServerStatus::Failed(_) => "Error".to_string(),
        };

        let title_text = active_title.unwrap_or("PandaMUX");

        GpuiTitleBar::new().child(
            div()
                .h(Spacing::TITLEBAR_HEIGHT)
                .w_full()
                .h_flex()
                .items_center()
                .justify_between()
                .px_3()
                .bg(theme.chrome.panel)
                .border_b_1()
                .border_color(rgba(0xffffff14))
                // Left: Logo badge and environment indicator
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        // PandaMUX Logo Chip
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1p5()
                                .px_2()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(theme.chrome.bgc_knockout)
                                .border_1()
                                .border_color(theme.accent.color())
                                .child(
                                    div()
                                        .text_size(Typography::TITLE_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.accent.color())
                                        .child("🐼 PandaMUX"),
                                ),
                        )
                        // Environment chip
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1()
                                .px_2()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(rgba(0xffffff0d))
                                .child(div().w_2().h_2().rounded_full().bg(status_color))
                                .child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .text_color(theme.chrome.text_t3)
                                        .child("env-local"),
                                ),
                        ),
                )
                // Center: Active title or breadcrumbs
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .text_size(Typography::BODY_SIZE)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.chrome.text_t1)
                        .child(title_text.to_string()),
                )
                // Right: Status indicator and window controls
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        // Server status pill
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1p5()
                                .px_2()
                                .py_0p5()
                                .rounded(Radii::CHIP)
                                .bg(theme.chrome.bgc_knockout)
                                .child(div().w_2().h_2().rounded_full().bg(status_color))
                                .child(
                                    div()
                                        .text_size(Typography::META_SIZE)
                                        .text_color(theme.chrome.text_t2)
                                        .child(status_label),
                                ),
                        )
                        // Command Palette Button
                        .child(
                            Button::new("btn-titlebar-palette")
                                .ghost()
                                .label("🔍 Ctrl+K")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_toggle_palette(this, window, cx);
                                })),
                        )
                        // Theme Toggle Button
                        .child(
                            Button::new("btn-titlebar-theme")
                                .ghost()
                                .label(match theme.mode {
                                    crate::theme::ThemeMode::Dark => "☀️ Light",
                                    crate::theme::ThemeMode::Light => "🌙 Dark",
                                })
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_toggle_theme(this, window, cx);
                                })),
                        )
                        // Settings Button
                        .child(
                            Button::new("btn-titlebar-settings")
                                .ghost()
                                .label("⚙")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_open_settings(this, window, cx);
                                })),
                        )
                        // Surfaces Panel Button
                        .child(
                            Button::new("btn-titlebar-surfaces")
                                .ghost()
                                .label("📋 Surfaces")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_toggle_surfaces(this, window, cx);
                                })),
                        ),
                ),
        )
    }
}
