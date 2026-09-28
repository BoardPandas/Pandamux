use gpui_kit::base::StyledExt as _;
use gpui_kit::gpui::*;

use crate::theme::{Radii, Theme, Typography};

/// Represents an entry in the quick-summon scratchpad transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScratchpadMessage {
    pub id: String,
    pub author: String,
    pub text: String,
    pub timestamp_ms: u64,
}

/// State for the floating quick-summon scratchpad window.
///
/// Summonable from anywhere via the system tray or global hotkey
/// (`CmdOrCtrl+Shift+Panda` or `Win+Alt+P`), allowing fast prompts to the
/// Global Orchestrator, scratch notes, or voice input through Hark.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScratchpadState {
    pub is_open: bool,
    pub input_text: String,
    pub messages: Vec<ScratchpadMessage>,
    pub notes: Vec<String>,
    pub is_voice_active: bool,
}

impl ScratchpadState {
    pub fn new() -> Self {
        Self {
            is_open: false,
            input_text: String::new(),
            messages: Vec::new(),
            notes: Vec::new(),
            is_voice_active: false,
        }
    }

    /// Toggles visibility of the floating scratchpad window.
    pub fn toggle(&mut self) {
        self.is_open = !self.is_open;
    }

    pub fn open(&mut self) {
        self.is_open = true;
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn set_input(&mut self, text: impl Into<String>) {
        self.input_text = text.into();
    }

    /// Submits the prompt input, clearing the input field and recording to message history.
    pub fn submit_prompt(&mut self) -> Option<String> {
        let trimmed = self.input_text.trim().to_string();
        if trimmed.is_empty() {
            return None;
        }

        self.messages.push(ScratchpadMessage {
            id: format!("msg-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            author: "You".to_string(),
            text: trimmed.clone(),
            timestamp_ms: 0,
        });

        self.input_text.clear();
        Some(trimmed)
    }

    /// Appends a quick scratch note.
    pub fn add_note(&mut self, note: impl Into<String>) {
        let note = note.into();
        if !note.trim().is_empty() {
            self.notes.push(note);
        }
    }

    /// Toggles Hark native voice transcription input mode.
    pub fn toggle_voice(&mut self) {
        self.is_voice_active = !self.is_voice_active;
    }

    /// Clears notes and message transcript.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.notes.clear();
        self.input_text.clear();
    }
}

/// Renders the floating quick-summon scratchpad window overlay.
pub fn render_scratchpad_window(
    state: &ScratchpadState,
    theme: &Theme,
    hotkey_display: &str,
) -> AnyElement {
    if !state.is_open {
        return div().into_any_element();
    }

    let header = div()
        .h_flex()
        .items_center()
        .justify_between()
        .pb_2()
        .border_b_1()
        .border_color(theme.chrome.inset)
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(div().text_size(px(16.0)).child("🐼"))
                .child(
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t1)
                        .child("Quick Scratchpad"),
                ),
        )
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(Radii::CHIP)
                        .bg(theme.chrome.inset)
                        .text_size(Typography::META_SIZE)
                        .font_family(Typography::MONO_FAMILY)
                        .text_color(theme.chrome.text_t3)
                        .child(hotkey_display.to_string()),
                )
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(Radii::CHIP)
                        .bg(if state.is_voice_active {
                            theme.accent.color()
                        } else {
                            theme.chrome.inset
                        })
                        .text_size(Typography::META_SIZE)
                        .text_color(if state.is_voice_active {
                            theme.chrome.bg_base_start
                        } else {
                            theme.chrome.text_t3
                        })
                        .child(if state.is_voice_active {
                            "🎙️ Hark Active"
                        } else {
                            "🎙️ Voice"
                        }),
                ),
        );

    // Render message history or empty prompt
    let message_scroller = if state.messages.is_empty() && state.notes.is_empty() {
        div()
            .v_flex()
            .items_center()
            .justify_center()
            .py_6()
            .gap_1()
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.chrome.text_t3)
                    .child("Type a quick prompt for the Orchestrator, or jot notes."),
            )
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.chrome.text_t4)
                    .child(format!("Press Esc or {} to dismiss.", hotkey_display)),
            )
            .into_any_element()
    } else {
        let mut items = Vec::new();

        for note in &state.notes {
            items.push(
                div()
                    .p_2()
                    .rounded(Radii::CHIP)
                    .bg(theme.chrome.inset)
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.chrome.text_t2)
                    .child(format!("📝 {note}"))
                    .into_any_element(),
            );
        }

        for msg in &state.messages {
            items.push(
                div()
                    .p_2()
                    .rounded(Radii::CHIP)
                    .bg(theme.chrome.panel2)
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(Typography::META_SIZE)
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.accent.color())
                            .child(msg.author.clone()),
                    )
                    .child(
                        div()
                            .text_size(Typography::BODY_SIZE)
                            .text_color(theme.chrome.text_t1)
                            .child(msg.text.clone()),
                    )
                    .into_any_element(),
            );
        }

        div()
            .v_flex()
            .gap_2()
            .max_h(px(200.0))
            .overflow_hidden()
            .children(items)
            .into_any_element()
    };

    let input_box = div()
        .h_flex()
        .items_center()
        .justify_between()
        .p_2()
        .rounded(Radii::ROW)
        .bg(theme.chrome.panel2)
        .border_1()
        .border_color(theme.accent.color())
        .child(
            div()
                .text_size(Typography::BODY_SIZE)
                .text_color(if state.input_text.is_empty() {
                    theme.chrome.text_t4
                } else {
                    theme.chrome.text_t1
                })
                .child(if state.input_text.is_empty() {
                    "Send instruction to Orchestrator...".to_string()
                } else {
                    state.input_text.clone()
                }),
        )
        .child(
            div()
                .px_2()
                .py_0p5()
                .rounded(Radii::CHIP)
                .bg(theme.accent.color())
                .text_size(Typography::META_SIZE)
                .font_weight(FontWeight::BOLD)
                .text_color(theme.chrome.bg_base_start)
                .child("Send ⏎"),
        );

    div()
        .absolute()
        .top(px(48.0))
        .right(px(24.0))
        .w(px(380.0))
        .p_3()
        .rounded(Radii::PANE)
        .bg(theme.chrome.panel)
        .border_1()
        .border_color(theme.accent.color())
        .shadow_lg()
        .v_flex()
        .gap_2()
        .child(header)
        .child(message_scroller)
        .child(input_box)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn test_scratchpad_state_defaults() {
        let state = ScratchpadState::new();
        assert!(!state.is_open);
        assert!(state.input_text.is_empty());
        assert!(state.messages.is_empty());
        assert!(state.notes.is_empty());
        assert!(!state.is_voice_active);
    }

    #[test]
    fn test_scratchpad_state_toggle_and_notes() {
        let mut state = ScratchpadState::new();
        state.toggle();
        assert!(state.is_open);

        state.add_note("Check pull request review comments");
        assert_eq!(state.notes.len(), 1);
        assert_eq!(state.notes[0], "Check pull request review comments");

        state.toggle_voice();
        assert!(state.is_voice_active);
    }

    #[test]
    fn test_scratchpad_submit_prompt() {
        let mut state = ScratchpadState::new();
        state.set_input("Run fleet audit on all repos");
        let submitted = state.submit_prompt();

        assert_eq!(submitted, Some("Run fleet audit on all repos".to_string()));
        assert!(state.input_text.is_empty());
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].text, "Run fleet audit on all repos");
    }
}
