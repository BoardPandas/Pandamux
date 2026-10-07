use std::sync::{Arc, Mutex};

use gpui_kit::base::StyledExt as _;
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_core::EnvironmentId;
use pandamux_protocol::terminal_rpc::{
    TerminalAttachParams, TerminalAttachResult, TerminalInputParams, TerminalOpenParams,
    TerminalResizeParams,
};
use pandamux_term::GridSize;
use pandamux_term::grid::{CellColor, ScreenCells, ScrollAmount, StyledCell, TerminalGrid};

use crate::theme::{Radii, Theme, Typography};

/// Maximum number of visible terminal characters exposed through accessibility.
///
/// The painted grid remains the visual source of truth. This cap prevents a
/// large viewport from producing an unbounded accessibility node while keeping
/// the most recent visible content, including the active prompt, available to
/// screen readers.
pub const ACCESSIBLE_TERMINAL_OUTPUT_CHAR_LIMIT: usize = 4_096;

/// Builds the accessible text for the currently visible terminal grid.
pub fn accessible_terminal_output(cells: &ScreenCells) -> String {
    let visible = cells
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| if cell.c == '\0' { ' ' } else { cell.c })
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let visible = visible.trim();

    if visible.is_empty() {
        return "Terminal output. No visible output.".to_string();
    }

    let character_count = visible.chars().count();
    if character_count <= ACCESSIBLE_TERMINAL_OUTPUT_CHAR_LIMIT {
        return format!("Terminal output. {visible}");
    }

    let recent = visible
        .chars()
        .skip(character_count - ACCESSIBLE_TERMINAL_OUTPUT_CHAR_LIMIT)
        .collect::<String>();
    format!("Terminal output. Earlier visible text omitted. {recent}")
}

/// Resolves an ANSI / indexed terminal color to an RGBA value.
pub fn resolve_indexed_color(index: u8, _theme: &Theme, _is_bg: bool) -> Rgba {
    match index {
        0 => rgb(0x1a1c1e),  // Black
        1 => rgb(0xe06c75),  // Red
        2 => rgb(0x98c379),  // Green
        3 => rgb(0xe5c07b),  // Yellow
        4 => rgb(0x61afef),  // Blue
        5 => rgb(0xc678dd),  // Magenta
        6 => rgb(0x56b6c2),  // Cyan
        7 => rgb(0xabb2bf),  // Light gray
        8 => rgb(0x5c6370),  // Dark gray
        9 => rgb(0xff7b86),  // Bright red
        10 => rgb(0xb1e38c), // Bright green
        11 => rgb(0xf3d38c), // Bright yellow
        12 => rgb(0x73baff), // Bright blue
        13 => rgb(0xd892eb), // Bright magenta
        14 => rgb(0x6bd7e4), // Bright cyan
        15 => rgb(0xffffff), // Bright white
        16..=231 => {
            // 6x6x6 color cube
            let cube_idx = index - 16;
            let r = (cube_idx / 36) % 6;
            let g = (cube_idx / 6) % 6;
            let b = cube_idx % 6;
            let r_val = if r == 0 { 0 } else { 55 + r * 40 };
            let g_val = if g == 0 { 0 } else { 55 + g * 40 };
            let b_val = if b == 0 { 0 } else { 55 + b * 40 };
            rgb(((r_val as u32) << 16) | ((g_val as u32) << 8) | (b_val as u32))
        }
        232..=255 => {
            // Grayscale ramp
            let gray = 8 + (index - 232) * 10;
            rgb(((gray as u32) << 16) | ((gray as u32) << 8) | (gray as u32))
        }
    }
}

/// Resolves a terminal cell color into a concrete RGBA color.
pub fn resolve_cell_color(color: CellColor, theme: &Theme, is_bg: bool) -> Rgba {
    match color {
        CellColor::Default => {
            if is_bg {
                theme.terminal.surface
            } else {
                theme.terminal.text
            }
        }
        CellColor::Background => theme.terminal.surface,
        CellColor::Rgb(r, g, b) => rgb(((r as u32) << 16) | ((g as u32) << 8) | (b as u32)),
        CellColor::Indexed(idx) => resolve_indexed_color(idx, theme, is_bg),
    }
}

/// A batched horizontal run of characters with identical styling.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalSpan {
    pub text: String,
    pub fg: Rgba,
    pub bg: Rgba,
    pub bold: bool,
    pub is_cursor: bool,
}

/// Converts a row of styled cells into compact, batched spans for high-performance rendering.
pub fn batch_row_cells(
    cells: &[StyledCell],
    row_idx: usize,
    cursor: (usize, usize),
    cursor_visible: bool,
    theme: &Theme,
) -> Vec<TerminalSpan> {
    if cells.is_empty() {
        return Vec::new();
    }

    let mut spans: Vec<TerminalSpan> = Vec::new();
    let mut current_text = String::new();
    let mut current_fg = Rgba::default();
    let mut current_bg = Rgba::default();
    let mut current_bold = false;
    let mut current_is_cursor = false;

    for (col_idx, cell) in cells.iter().enumerate() {
        let is_cursor = cursor_visible && cursor.0 == row_idx && cursor.1 == col_idx;
        let mut fg = resolve_cell_color(cell.fg, theme, false);
        let mut bg = resolve_cell_color(cell.bg, theme, true);

        if is_cursor {
            // Invert colors for cursor
            std::mem::swap(&mut fg, &mut bg);
            if bg == theme.terminal.surface {
                bg = theme.terminal.prompt;
                fg = theme.terminal.surface;
            }
        }

        let char_val = if cell.c == '\0' { ' ' } else { cell.c };

        if spans.is_empty() && current_text.is_empty() {
            current_text.push(char_val);
            current_fg = fg;
            current_bg = bg;
            current_bold = cell.bold;
            current_is_cursor = is_cursor;
        } else if fg == current_fg
            && bg == current_bg
            && cell.bold == current_bold
            && is_cursor == current_is_cursor
        {
            current_text.push(char_val);
        } else {
            spans.push(TerminalSpan {
                text: std::mem::take(&mut current_text),
                fg: current_fg,
                bg: current_bg,
                bold: current_bold,
                is_cursor: current_is_cursor,
            });
            current_text.push(char_val);
            current_fg = fg;
            current_bg = bg;
            current_bold = cell.bold;
            current_is_cursor = is_cursor;
        }
    }

    if !current_text.is_empty() {
        spans.push(TerminalSpan {
            text: current_text,
            fg: current_fg,
            bg: current_bg,
            bold: current_bold,
            is_cursor: current_is_cursor,
        });
    }

    spans
}

/// State for the desktop terminal surface view.
#[derive(Clone, Debug)]
pub struct TerminalViewState {
    pub terminal_id: String,
    pub cwd: String,
    pub title: String,
    pub rows: u16,
    pub cols: u16,
    pub attached_offset: u64,
    pub is_attached: bool,
    pub is_running: bool,
    pub exit_code: Option<u32>,
    pub environment_id: Option<EnvironmentId>,
    pub last_truncated: bool,
    pub error: Option<String>,
    pub cached_cells: ScreenCells,
    grid: Arc<Mutex<TerminalGrid>>,
}

impl TerminalViewState {
    /// Creates a new terminal view state.
    pub fn new(
        terminal_id: impl Into<String>,
        cwd: impl Into<String>,
        rows: u16,
        cols: u16,
    ) -> Self {
        Self::new_with_env(terminal_id, cwd, rows, cols, None)
    }

    /// Creates a new terminal view state with an optional remote environment identifier.
    pub fn new_with_env(
        terminal_id: impl Into<String>,
        cwd: impl Into<String>,
        rows: u16,
        cols: u16,
        environment_id: Option<EnvironmentId>,
    ) -> Self {
        let tid = terminal_id.into();
        let r = rows.max(1);
        let c = cols.max(1);
        let grid = TerminalGrid::new(GridSize::new(c as usize, r as usize));
        let cached_cells = grid.visible_cells();

        let title = if let Some(ref env) = environment_id
            && environment_is_remote(env)
        {
            format!("Terminal ({tid} @ {})", env.as_str())
        } else {
            format!("Terminal ({tid})")
        };

        Self {
            terminal_id: tid,
            cwd: cwd.into(),
            title,
            rows: r,
            cols: c,
            attached_offset: 0,
            is_attached: true,
            is_running: true,
            exit_code: None,
            environment_id,
            last_truncated: false,
            error: None,
            cached_cells,
            grid: Arc::new(Mutex::new(grid)),
        }
    }

    /// Indicates whether this terminal is running on a remote environment.
    pub fn is_remote(&self) -> bool {
        self.environment_id
            .as_ref()
            .is_some_and(environment_is_remote)
    }

    /// Returns the environment identifier if this is a remote terminal session.
    pub fn environment_id(&self) -> Option<&EnvironmentId> {
        self.environment_id.as_ref()
    }

    /// Sets or updates the environment identifier for this session.
    pub fn set_environment_id(&mut self, env: Option<EnvironmentId>) {
        self.environment_id = env;
    }

    /// Feeds incoming terminal byte chunks, honoring monotonic offset tracking and truncation.
    pub fn feed_bytes(&mut self, bytes: &[u8], offset: u64, truncated: bool) {
        if let Ok(mut grid) = self.grid.lock() {
            if truncated {
                // If ring buffer evicted older history, re-create grid to clear stale lines
                *grid = TerminalGrid::new(GridSize::new(self.cols as usize, self.rows as usize));
            }
            grid.advance(bytes);
            self.cached_cells = grid.visible_cells();
        }
        self.attached_offset = offset;
        self.last_truncated = truncated;
    }

    /// Applies the result of a `terminal.attach` call with `since_offset` recovery.
    ///
    /// Returns `true` if the ring buffer was truncated, signalling that the client
    /// must dispatch a resize nudge so full-screen TUIs (e.g. vim, htop) repaint cleanly.
    pub fn apply_attach_result(&mut self, result: &TerminalAttachResult) -> bool {
        self.rows = result.rows.max(1);
        self.cols = result.cols.max(1);
        self.is_attached = true;
        self.error = None;
        let needs_nudge = result.truncated;
        self.feed_bytes(result.data.as_bytes(), result.offset, result.truncated);
        needs_nudge
    }

    /// Marks the terminal unavailable while preserving its last rendered output.
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.is_attached = false;
        self.error = Some(message.into());
    }

    /// Builds a pair of temporary resize parameters to nudge the remote PTY on truncation.
    ///
    /// Nudging the column size down by 1 and immediately restoring it triggers a SIGWINCH
    /// signal on Unix nodes (or ConPTY window buffer size event on Windows), forcing full-screen
    /// terminal applications to repaint completely without user intervention.
    pub fn create_resize_nudge(&self) -> (TerminalResizeParams, TerminalResizeParams) {
        let nudge_cols = self.cols.saturating_sub(1).max(1);
        (
            TerminalResizeParams {
                terminal_id: self.terminal_id.clone(),
                rows: self.rows,
                cols: nudge_cols,
            },
            TerminalResizeParams {
                terminal_id: self.terminal_id.clone(),
                rows: self.rows,
                cols: self.cols,
            },
        )
    }

    /// Constructs `TerminalAttachParams` requesting output since the current `attached_offset`.
    pub fn build_attach_params(&self) -> TerminalAttachParams {
        TerminalAttachParams {
            terminal_id: self.terminal_id.clone(),
            since_offset: Some(self.attached_offset),
        }
    }

    /// Constructs `TerminalInputParams` to send keyboard or paste data to the PTY.
    pub fn build_input_params(&self, data: impl Into<String>) -> TerminalInputParams {
        TerminalInputParams {
            terminal_id: self.terminal_id.clone(),
            data: data.into(),
        }
    }

    /// Constructs `TerminalResizeParams` for explicit window resizing.
    pub fn build_resize_params(&self, rows: u16, cols: u16) -> TerminalResizeParams {
        TerminalResizeParams {
            terminal_id: self.terminal_id.clone(),
            rows,
            cols,
        }
    }

    /// Constructs `TerminalOpenParams` for session creation on the target node.
    pub fn build_open_params(&self) -> TerminalOpenParams {
        TerminalOpenParams {
            terminal_id: Some(self.terminal_id.clone()),
            environment_id: self.environment_id.clone(),
            cwd: Some(self.cwd.clone()),
            shell: None,
            rows: Some(self.rows),
            cols: Some(self.cols),
            env: None,
        }
    }

    /// Scrolls the terminal viewport.
    pub fn scroll(&mut self, amount: ScrollAmount) {
        if let Ok(mut grid) = self.grid.lock() {
            grid.scroll_display(amount);
            self.cached_cells = grid.visible_cells();
        }
    }

    /// Resizes the terminal grid to match new dimensions.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let r = rows.max(1);
        let c = cols.max(1);
        self.rows = r;
        self.cols = c;
        if let Ok(mut grid) = self.grid.lock() {
            grid.resize(GridSize::new(c as usize, r as usize));
            self.cached_cells = grid.visible_cells();
        }
    }

    /// Marks the process running status and exit code.
    pub fn set_running(&mut self, running: bool, exit_code: Option<u32>) {
        self.is_running = running;
        self.exit_code = exit_code;
    }

    /// Returns a bounded accessible description of the painted terminal grid.
    pub fn accessible_output(&self) -> String {
        accessible_terminal_output(&self.cached_cells)
    }

    /// Returns currently selected text in the terminal grid if any.
    pub fn selection_text(&self) -> Option<String> {
        self.grid.lock().ok().and_then(|g| g.selection_text())
    }

    /// Clears any active selection.
    pub fn clear_selection(&mut self) {
        if let Ok(mut grid) = self.grid.lock() {
            grid.clear_selection();
            self.cached_cells = grid.visible_cells();
        }
    }

    /// Clears the terminal screen.
    pub fn clear_screen(&mut self) {
        if let Ok(mut grid) = self.grid.lock() {
            grid.advance(b"\x1b[2J\x1b[H");
            self.cached_cells = grid.visible_cells();
        }
    }
}

fn environment_is_remote(environment_id: &EnvironmentId) -> bool {
    !matches!(environment_id.as_str(), "env-local" | "env_local")
}

/// Converts a GPUI key event into bytes understood by a terminal PTY.
///
/// Platform and function modifiers are left to the application or operating
/// system. AltGr-style events prefer the composed character supplied by GPUI.
pub fn terminal_input_for_keystroke(
    keystroke: &Keystroke,
    prefer_character_input: bool,
) -> Option<String> {
    let modifiers = keystroke.modifiers;
    let key = keystroke.key.to_ascii_lowercase();

    let named = match key.as_str() {
        "enter" | "return" => Some("\r"),
        "backspace" => Some("\x7f"),
        "tab" if modifiers.shift => Some("\x1b[Z"),
        "tab" => Some("\t"),
        "escape" | "esc" => Some("\x1b"),
        "up" | "arrowup" => Some("\x1b[A"),
        "down" | "arrowdown" => Some("\x1b[B"),
        "right" | "arrowright" => Some("\x1b[C"),
        "left" | "arrowleft" => Some("\x1b[D"),
        "home" => Some("\x1b[H"),
        "end" => Some("\x1b[F"),
        "insert" => Some("\x1b[2~"),
        "delete" => Some("\x1b[3~"),
        "pageup" => Some("\x1b[5~"),
        "pagedown" => Some("\x1b[6~"),
        _ => None,
    };
    if let Some(sequence) = named {
        return Some(sequence.to_string());
    }

    if modifiers.platform || modifiers.function {
        return None;
    }

    if modifiers.control && !prefer_character_input && !modifiers.alt {
        return control_character(keystroke.key.as_str())
            .map(char::from)
            .map(String::from);
    }

    let character = keystroke
        .key_char
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            if keystroke.key.chars().count() == 1 {
                Some(keystroke.key.as_str())
            } else {
                None
            }
        })?;

    if modifiers.alt && !prefer_character_input {
        Some(format!("\x1b{character}"))
    } else {
        Some(character.to_string())
    }
}

fn control_character(value: &str) -> Option<u8> {
    let mut characters = value.chars();
    let first = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    match first.to_ascii_lowercase() {
        character @ 'a'..='z' => Some((character as u8 - b'a') + 1),
        '@' | ' ' => Some(0x00),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

/// Renders the complete terminal surface view including header toolbar, screen grid, and scrollbar.
pub fn render_terminal_view(state: &TerminalViewState, theme: &Theme) -> AnyElement {
    let status_badge = if state.is_running {
        div()
            .h_flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0x7fd88f20))
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.terminal.success)
                    .child("● Running"),
            )
    } else {
        let code_str = state
            .exit_code
            .map(|c| format!("Exited ({c})"))
            .unwrap_or_else(|| "Terminated".to_string());
        div()
            .h_flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0x6b7c8020))
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.terminal.dim)
                    .child(code_str),
            )
    };

    let env_badge = if let Some(env) = state.environment_id.as_ref().filter(|_| state.is_remote()) {
        div()
            .h_flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0x38bdf820))
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0x38bdf8))
                    .child(format!("🌐 Remote ({})", env.as_str())),
            )
    } else {
        div()
            .h_flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded(Radii::CHIP)
            .bg(rgba(0x6b7c8020))
            .child(
                div()
                    .text_size(Typography::META_SIZE)
                    .text_color(theme.terminal.dim)
                    .child("💻 Local"),
            )
    };

    let header = div()
        .h_flex()
        .items_center()
        .justify_between()
        .p_2()
        .bg(theme.chrome.panel2)
        .rounded(Radii::ROW)
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .child(if state.is_remote() { "🌐" } else { "💻" }),
                )
                .child(
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t1)
                        .child(state.title.clone()),
                )
                .child(status_badge)
                .child(env_badge)
                .when(state.last_truncated, |this| {
                    this.child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_0p5()
                            .rounded(Radii::CHIP)
                            .bg(rgba(0xe5c07b20))
                            .child(
                                div()
                                    .text_size(Typography::META_SIZE)
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.terminal.warn)
                                    .child("⚠ Grid Re-synced"),
                            ),
                    )
                }),
        )
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t3)
                        .child(format!("📁 {}", state.cwd)),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t4)
                        .child(format!("Offset: {}", state.attached_offset)),
                )
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.chrome.text_t4)
                        .child(format!("{}x{}", state.cols, state.rows)),
                ),
        );

    // Build row elements
    let mut row_elements: Vec<AnyElement> = Vec::with_capacity(state.cached_cells.rows.len());

    for (row_idx, row_cells) in state.cached_cells.rows.iter().enumerate() {
        let spans = batch_row_cells(
            row_cells,
            row_idx,
            state.cached_cells.cursor,
            state.cached_cells.cursor_visible,
            theme,
        );

        let mut span_elements: Vec<AnyElement> = Vec::with_capacity(spans.len());
        for span in spans {
            span_elements.push(
                div()
                    .text_color(span.fg)
                    .bg(span.bg)
                    .font_weight(if span.bold {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .child(span.text)
                    .into_any_element(),
            );
        }

        row_elements.push(
            div()
                .h_flex()
                .line_height(px(16.0))
                .children(span_elements)
                .into_any_element(),
        );
    }

    let history_indicator = if state.cached_cells.display_offset > 0 {
        Some(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .px_2()
                .py_1()
                .bg(theme.chrome.panel2)
                .rounded(Radii::CHIP)
                .child(
                    div()
                        .text_size(Typography::META_SIZE)
                        .text_color(theme.terminal.warn)
                        .child(format!(
                            "Scrolled {} lines into history ({} total history lines)",
                            state.cached_cells.display_offset, state.cached_cells.history_size
                        )),
                )
                .into_any_element(),
        )
    } else {
        None
    };

    let screen_container = div()
        .id("terminal-output-log")
        .role(Role::Log)
        .accessibility_id("terminal-output-log")
        .aria_label(state.accessible_output())
        .v_flex()
        .bg(theme.terminal.surface)
        .p_3()
        .rounded(Radii::ROW)
        .font_family(Typography::MONO_FAMILY)
        .text_size(Typography::BODY_SIZE)
        .overflow_hidden()
        .children(row_elements);

    div()
        .v_flex()
        .gap_2()
        .p_2()
        .when_some(state.error.as_ref(), |this, error| {
            this.child(
                div()
                    .id("terminal-error")
                    .role(Role::Alert)
                    .aria_label(format!("Terminal error: {error}"))
                    .p_2()
                    .rounded(Radii::ROW)
                    .border_1()
                    .border_color(rgb(0xf87171))
                    .text_color(rgb(0xf87171))
                    .child(error.clone()),
            )
        })
        .child(header)
        .when_some(history_indicator, |this, hi| this.child(hi))
        .child(screen_container)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn test_terminal_view_state_creation() {
        let state = TerminalViewState::new("term-test-1", "/test/dir", 24, 80);
        assert_eq!(state.terminal_id, "term-test-1");
        assert_eq!(state.cwd, "/test/dir");
        assert_eq!(state.rows, 24);
        assert_eq!(state.cols, 80);
        assert!(state.is_running);
        assert_eq!(state.attached_offset, 0);
        assert!(state.error.is_none());
        assert_eq!(state.cached_cells.rows.len(), 24);
    }

    #[test]
    fn terminal_keystrokes_map_to_pty_input() {
        let plain = Keystroke::parse("a").expect("plain key");
        assert_eq!(
            terminal_input_for_keystroke(&plain, false).as_deref(),
            Some("a")
        );

        let enter = Keystroke::parse("enter").expect("enter key");
        assert_eq!(
            terminal_input_for_keystroke(&enter, false).as_deref(),
            Some("\r")
        );

        let control_c = Keystroke::parse("ctrl-c").expect("control key");
        assert_eq!(
            terminal_input_for_keystroke(&control_c, false).as_deref(),
            Some("\u{3}")
        );

        let shift_tab = Keystroke::parse("shift-tab").expect("shift tab");
        assert_eq!(
            terminal_input_for_keystroke(&shift_tab, false).as_deref(),
            Some("\u{1b}[Z")
        );

        let platform = Keystroke::parse("cmd-k").expect("platform key");
        assert_eq!(terminal_input_for_keystroke(&platform, false), None);

        let control_question = Keystroke::parse("ctrl-?").expect("control question mark");
        assert_eq!(
            terminal_input_for_keystroke(&control_question, false).as_deref(),
            Some("\u{7f}")
        );
    }

    #[test]
    fn terminal_error_preserves_recovery_state() {
        let mut state = TerminalViewState::new("term-error", ".", 24, 80);
        state.feed_bytes(b"retained output", 15, false);
        state.set_error("attach failed");

        assert!(!state.is_attached);
        assert_eq!(state.error.as_deref(), Some("attach failed"));
        assert_eq!(state.attached_offset, 15);

        let result = TerminalAttachResult {
            terminal_id: "term-error".to_string(),
            rows: 24,
            cols: 80,
            offset: 20,
            data: "again".to_string(),
            truncated: false,
        };
        state.apply_attach_result(&result);
        assert!(state.is_attached);
        assert!(state.error.is_none());
    }

    #[test]
    fn test_terminal_feed_bytes_and_batch_cells() {
        let theme = Theme::dark(crate::theme::AccentColor::Teal);
        let mut state = TerminalViewState::new("term-test-2", "/workspace", 10, 40);

        state.feed_bytes(b"Hello PandaMUX\r\nLine 2\r\n", 24, false);
        assert_eq!(state.attached_offset, 24);

        // Check first row text content
        let spans = batch_row_cells(
            &state.cached_cells.rows[0],
            0,
            state.cached_cells.cursor,
            false,
            &theme,
        );
        let full_text: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert!(full_text.starts_with("Hello PandaMUX"));
    }

    #[test]
    fn accessible_terminal_output_exposes_visible_text_and_keeps_recent_content() {
        let mut state = TerminalViewState::new("term-accessible", ".", 50, 120);
        let old = "old-visible-line ".repeat(400);
        let ready = "PANDAMUX_LOGIN_FIXTURE_READY\r\nPANDAMUX_LOGIN_FIXTURE_PROMPT> ";
        state.feed_bytes(format!("{old}\r\n{ready}").as_bytes(), 7_000, false);

        let output = state.accessible_output();
        assert!(output.starts_with("Terminal output."));
        assert!(output.contains("PANDAMUX_LOGIN_FIXTURE_READY"));
        assert!(output.contains("PANDAMUX_LOGIN_FIXTURE_PROMPT>"));
        assert!(!output.contains('\0'));
        assert!(
            output.chars().count()
                <= ACCESSIBLE_TERMINAL_OUTPUT_CHAR_LIMIT
                    + "Terminal output. Earlier visible text omitted. "
                        .chars()
                        .count()
        );
    }

    #[test]
    fn test_terminal_truncation_resets_grid() {
        let mut state = TerminalViewState::new("term-test-3", "/workspace", 5, 20);
        state.feed_bytes(b"Old output that will be evicted\r\n", 32, false);
        assert_eq!(state.attached_offset, 32);

        // Feed with truncated = true
        state.feed_bytes(b"Fresh start\r\n", 100, true);
        assert_eq!(state.attached_offset, 100);

        let row0: String = state.cached_cells.rows[0].iter().map(|c| c.c).collect();
        assert!(row0.starts_with("Fresh start"));
        assert!(!row0.contains("Old output"));
    }

    #[test]
    fn test_resolve_indexed_color() {
        let theme = Theme::dark(crate::theme::AccentColor::Teal);
        let red = resolve_indexed_color(1, &theme, false);
        assert_eq!(red, rgb(0xe06c75));

        let green = resolve_indexed_color(2, &theme, false);
        assert_eq!(green, rgb(0x98c379));

        let cube_color = resolve_indexed_color(16, &theme, false);
        assert_eq!(cube_color, rgb(0x000000));
    }

    #[test]
    fn test_terminal_view_remote_state() {
        let env = EnvironmentId::new("env_galahad");
        let state =
            TerminalViewState::new_with_env("term-rem-1", "/home/chaz", 30, 100, Some(env.clone()));
        assert!(state.is_remote());
        assert_eq!(state.environment_id(), Some(&env));
        assert!(state.title.contains("env_galahad"));
    }

    #[test]
    fn test_terminal_view_local_environment_is_not_labeled_remote() {
        for local_id in ["env-local", "env_local"] {
            let state = TerminalViewState::new_with_env(
                "term-local",
                "/workspace",
                30,
                100,
                Some(EnvironmentId::new(local_id)),
            );
            assert!(!state.is_remote());
            assert_eq!(state.title, "Terminal (term-local)");
        }
    }

    #[test]
    fn test_terminal_view_apply_attach_result_not_truncated() {
        let mut state = TerminalViewState::new("term-attach-1", "/test", 10, 40);
        let res = TerminalAttachResult {
            terminal_id: "term-attach-1".to_string(),
            rows: 10,
            cols: 40,
            offset: 15,
            data: "first chunk\r\n".to_string(),
            truncated: false,
        };
        let nudge = state.apply_attach_result(&res);
        assert!(!nudge);
        assert!(!state.last_truncated);
        assert_eq!(state.attached_offset, 15);
        let row0: String = state.cached_cells.rows[0].iter().map(|c| c.c).collect();
        assert!(row0.starts_with("first chunk"));
    }

    #[test]
    fn test_terminal_view_apply_attach_result_truncated_and_resize_nudge() {
        let mut state = TerminalViewState::new("term-attach-2", "/test", 5, 20);
        state.feed_bytes(b"stale data\r\n", 12, false);

        let res = TerminalAttachResult {
            terminal_id: "term-attach-2".to_string(),
            rows: 5,
            cols: 20,
            offset: 105,
            data: "new data\r\n".to_string(),
            truncated: true,
        };
        let nudge = state.apply_attach_result(&res);
        assert!(nudge);
        assert!(state.last_truncated);
        assert_eq!(state.attached_offset, 105);

        let row0: String = state.cached_cells.rows[0].iter().map(|c| c.c).collect();
        assert!(row0.starts_with("new data"));
        assert!(!row0.contains("stale data"));

        let (p1, p2) = state.create_resize_nudge();
        assert_eq!(p1.cols, 19);
        assert_eq!(p2.cols, 20);
        assert_eq!(p1.terminal_id, "term-attach-2");
        assert_eq!(p2.terminal_id, "term-attach-2");
    }

    #[test]
    fn test_terminal_view_param_builders() {
        let env = EnvironmentId::new("env_remote");
        let mut state =
            TerminalViewState::new_with_env("term-bld-1", "/work", 24, 80, Some(env.clone()));
        state.attached_offset = 512;

        let attach_p = state.build_attach_params();
        assert_eq!(attach_p.terminal_id, "term-bld-1");
        assert_eq!(attach_p.since_offset, Some(512));

        let input_p = state.build_input_params("clear\n");
        assert_eq!(input_p.terminal_id, "term-bld-1");
        assert_eq!(input_p.data, "clear\n");

        let open_p = state.build_open_params();
        assert_eq!(open_p.terminal_id, Some("term-bld-1".to_string()));
        assert_eq!(open_p.environment_id, Some(env));
        assert_eq!(open_p.cwd, Some("/work".to_string()));
    }
}
