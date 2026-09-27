use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;

use crate::picker::PickerState;
use crate::theme::{Radii, Theme, Typography};

/// State for the text composer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposerState {
    pub text: String,
}

impl ComposerState {
    pub fn new() -> Self {
        Self { text: String::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    pub fn append_char(&mut self, ch: char) {
        self.text.push(ch);
    }
}

/// Renders the Composer component with integrated picker chips and action controls.
pub fn render_composer<V: 'static>(
    composer: &ComposerState,
    picker: &PickerState,
    theme: &Theme,
    on_send: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_clear: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_cycle_provider: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_cycle_model: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    on_cycle_effort: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static + Copy,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let has_text = !composer.is_empty();
    let char_count = composer.text.len();
    let effort_label = picker
        .effort_str()
        .map(|e| format!("Effort: {e}"))
        .unwrap_or_else(|| "Effort: off".to_string());

    div()
        .w_full()
        .v_flex()
        .p_2p5()
        .gap_2()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel)
        .border_1()
        .border_color(if has_text {
            rgba(0x43d9c940)
        } else {
            rgba(0xffffff14)
        })
        // 1. Selector Bar (Provider, Model, Effort, Attachments)
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1p5()
                        // Provider selector chip
                        .child(
                            Button::new("composer-provider-chip")
                                .ghost()
                                .label(format!("{} ▾", picker.current_provider_name()))
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_cycle_provider(this, window, cx);
                                })),
                        )
                        // Model selector chip
                        .child(
                            Button::new("composer-model-chip")
                                .ghost()
                                .label(format!("{} ▾", picker.current_model_name()))
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_cycle_model(this, window, cx);
                                })),
                        )
                        // Effort selector chip
                        .child(
                            Button::new("composer-effort-chip")
                                .ghost()
                                .label(format!("{effort_label} ▾"))
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_cycle_effort(this, window, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Button::new("composer-attach-stub")
                                .ghost()
                                .label("📎 Attach"),
                        ),
                ),
        )
        // 2. Text Input Area
        .child(
            div()
                .min_h(px(64.0))
                .p_2()
                .rounded(Radii::CHIP)
                .bg(theme.chrome.bgc_knockout)
                .border_1()
                .border_color(rgba(0xffffff0d))
                .child(if composer.text.is_empty() {
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child("Type a prompt or task for the AI agent... (Enter to send, Shift+Enter for newline)")
                } else {
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .text_color(theme.chrome.text_t1)
                        .child(composer.text.clone())
                }),
        )
        // 3. Bottom controls (Hints + Clear + Send)
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child("Enter ↵ Send · Shift+Enter ↵ Newline"),
                        )
                        .child(
                            div()
                                .text_size(Typography::META_SIZE)
                                .text_color(theme.chrome.text_t3)
                                .child(format!("{char_count} chars")),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .when(has_text, |this| {
                            this.child(
                                Button::new("composer-clear-btn")
                                    .ghost()
                                    .label("Clear")
                                    .on_click(cx.listener(move |this, _event, window, cx| {
                                        on_clear(this, window, cx);
                                    })),
                            )
                        })
                        .child(
                            Button::new("composer-send-btn")
                                .primary()
                                .label("➤ Send")
                                .on_click(cx.listener(move |this, _event, window, cx| {
                                    on_send(this, window, cx);
                                })),
                        ),
                ),
        )
}
