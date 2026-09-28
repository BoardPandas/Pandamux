use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::theme::{Radii, Theme, Typography};

/// Notification severity level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NotificationLevel {
    #[default]
    Info,
    Success,
    Warn,
    Error,
}

impl NotificationLevel {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Info => "ℹ️",
            Self::Success => "✅",
            Self::Warn => "⚠️",
            Self::Error => "🚨",
        }
    }

    pub fn color(&self, theme: &Theme) -> Rgba {
        match self {
            Self::Info => theme.accent_color(),
            Self::Success => rgb(0x7fd88f),
            Self::Warn => rgb(0xd8b45e),
            Self::Error => rgb(0xf87171),
        }
    }
}

/// An individual toast notification displayed in the desktop chrome.
#[derive(Clone, Debug, PartialEq)]
pub struct ToastNotification {
    pub id: String,
    pub title: String,
    pub message: String,
    pub level: NotificationLevel,
    pub timestamp_ms: u64,
}

static TOAST_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl ToastNotification {
    pub fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        level: NotificationLevel,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let counter = TOAST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("toast-{now}-{counter}");
        Self {
            id,
            title: title.into(),
            message: message.into(),
            level,
            timestamp_ms: now,
        }
    }
}

/// Dispatches an OS notification and sound chime.
pub fn dispatch_os_notification(title: &str, message: &str, sound_enabled: bool) {
    if sound_enabled {
        // Trigger audible chime indicator via standard ASCII bell
        eprint!("\x07");
    }

    #[cfg(target_os = "windows")]
    {
        // On Windows, dispatch a native notification via powershell script
        let script = format!(
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
             $template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
             $textNodes = $template.GetElementsByTagName('text'); \
             $textNodes.Item(0).AppendChild($template.CreateTextNode('{}')) > $null; \
             $textNodes.Item(1).AppendChild($template.CreateTextNode('{}')) > $null; \
             $toast = [Windows.UI.Notifications.ToastNotification]::new($template); \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('PandaMUX').Show($toast);",
            title.replace('\'', "''"),
            message.replace('\'', "''")
        );
        let _ = std::process::Command::new("powershell")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(script)
            .spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"PandaMUX\" subtitle \"{}\"",
            message.replace('"', "\\\""),
            title.replace('"', "\\\"")
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .arg(title)
            .arg(message)
            .spawn();
    }
}

/// Renders the floating stack of toast notifications at top-right.
pub fn render_toast_overlay<V: 'static>(
    notifications: &[ToastNotification],
    theme: &Theme,
    on_dismiss: impl Fn(&mut V, String, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    div()
        .id("toast-overlay-container")
        .absolute()
        .top(px(48.0))
        .right(px(16.0))
        .w(px(320.0))
        .v_flex()
        .gap_2()
        .children(notifications.iter().enumerate().map(|(idx, toast)| {
            let toast_id = toast.id.clone();
            let accent = toast.level.color(theme);

            div()
                .id(ElementId::named_usize("toast", idx))
                .p_3()
                .rounded(Radii::ROW)
                .bg(theme.chrome.panel)
                .border_1()
                .border_color(rgba(0xffffff1a))
                .shadow_lg()
                .v_flex()
                .gap_1()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1p5()
                                .child(div().child(toast.level.icon()))
                                .child(
                                    div()
                                        .text_size(Typography::BODY_SIZE)
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(accent)
                                        .child(toast.title.clone()),
                                ),
                        )
                        .child(
                            Button::new(format!("btn-dismiss-{}", toast.id))
                                .ghost()
                                .label("✕")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_dismiss(this, toast_id.clone(), window, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .text_size(Typography::SECONDARY_SIZE)
                        .text_color(theme.chrome.text_t2)
                        .child(toast.message.clone()),
                )
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn test_toast_notification_creation() {
        let toast = ToastNotification::new(
            "Turn Completed",
            "Thread finished 3 tool calls",
            NotificationLevel::Success,
        );
        assert_eq!(toast.title, "Turn Completed");
        assert_eq!(toast.level, NotificationLevel::Success);
        assert!(toast.timestamp_ms > 0);
    }
}
