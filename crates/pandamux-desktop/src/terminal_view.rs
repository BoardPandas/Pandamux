use std::sync::{Arc, Mutex};

use gpui_kit::base::StyledExt as _;
use gpui_kit::gpui::*;
use gpui_kit::prelude::FluentBuilder as _;
use pandamux_term::grid::{
    CellColor, GridSize, ScreenCells, ScrollAmount, StyledCell, TerminalGrid,
};

use crate::theme::{Radii, Theme, Typography};

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
        let tid = terminal_id.into();
        let r = rows.max(1);
        let c = cols.max(1);
        let grid = TerminalGrid::new(GridSize::new(c as usize, r as usize));
        let cached_cells = grid.visible_cells();

        Self {
            terminal_id: tid.clone(),
            cwd: cwd.into(),
            title: format!("Terminal ({tid})"),
            rows: r,
            cols: c,
            attached_offset: 0,
            is_attached: true,
            is_running: true,
            exit_code: None,
            cached_cells,
            grid: Arc::new(Mutex::new(grid)),
        }
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
                .child(div().text_size(Typography::BODY_SIZE).child("💻"))
                .child(
                    div()
                        .text_size(Typography::BODY_SIZE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.chrome.text_t1)
                        .child(state.title.clone()),
                )
                .child(status_badge),
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
        assert_eq!(state.cached_cells.rows.len(), 24);
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
}
