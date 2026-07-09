use crate::clipboard::{ClipboardKind, ClipboardStore};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{ClipboardType, Config, Osc52, Term, TermMode};
use alacritty_terminal::vte::ansi;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub columns: usize,
    pub rows: usize,
}

impl GridSize {
    pub fn new(columns: usize, rows: usize) -> Self {
        Self { columns, rows }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// The terminal's event sink. Almost every alacritty event is irrelevant to a
/// headless grid, but OSC 52 clipboard-store events (a remote program copying
/// text, including over SSH) are captured here into a shared buffer the app
/// layer drains and forwards to the OS clipboard (plan F1). The buffer is behind
/// an `Arc<Mutex<_>>` (not `Rc<RefCell<_>>`) so `TerminalGrid` stays `Send`: the
/// backend that owns it lives in an `Arc<Mutex<Backend>>` held across `.await`.
#[derive(Clone, Default)]
struct Listener {
    clipboard: Arc<Mutex<Vec<ClipboardStore>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        if let Event::ClipboardStore(kind, text) = event {
            let kind = match kind {
                ClipboardType::Clipboard => ClipboardKind::Clipboard,
                ClipboardType::Selection => ClipboardKind::Selection,
            };
            if let Ok(mut buffer) = self.clipboard.lock() {
                buffer.push(ClipboardStore { kind, text });
            }
        }
    }
}

pub struct TerminalGrid {
    parser: ansi::Processor,
    term: Term<Listener>,
    clipboard: Arc<Mutex<Vec<ClipboardStore>>>,
}

impl TerminalGrid {
    pub fn new(size: GridSize) -> Self {
        let config = Config {
            scrolling_history: size.rows * 4,
            // OnlyCopy is the secure default and exactly F1's policy: accept a
            // remote's OSC 52 copy, deny its clipboard-read (load) query.
            osc52: Osc52::OnlyCopy,
            ..Config::default()
        };

        let clipboard: Arc<Mutex<Vec<ClipboardStore>>> = Arc::default();
        let listener = Listener {
            clipboard: clipboard.clone(),
        };

        Self {
            parser: ansi::Processor::new(),
            term: Term::new(config, &size, listener),
            clipboard,
        }
    }

    /// Drain OSC 52 clipboard-store requests captured since the last call. The
    /// app layer forwards these to the OS clipboard (arboard) under the size cap
    /// in [`crate::clipboard::ClipboardPolicy`].
    pub fn take_clipboard_stores(&self) -> Vec<ClipboardStore> {
        self.clipboard
            .lock()
            .map(|mut buffer| std::mem::take(&mut *buffer))
            .unwrap_or_default()
    }

    /// Whether the terminal has requested bracketed-paste mode (DECSET 2004).
    /// Callers wrap outgoing pastes with [`crate::clipboard::wrap_paste`].
    pub fn bracketed_paste_active(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// Text of a linear region between two points (inclusive), addressed in
    /// [`Self::serialize_lines`] coordinates (row 0 = top of scrollback, column
    /// = char index into the trimmed row). Points may be given in any order.
    /// Used by copy-mode yank.
    pub fn region_text(&self, a: (usize, usize), b: (usize, usize)) -> String {
        let lines = self.serialize_lines();
        if lines.is_empty() {
            return String::new();
        }
        let last = lines.len() - 1;
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let (start_row, start_col) = (start.0.min(last), start.1);
        let (end_row, end_col) = (end.0.min(last), end.1);

        if start_row == end_row {
            let row: Vec<char> = lines[start_row].chars().collect();
            let from = start_col.min(row.len());
            let to = (end_col + 1).min(row.len()).max(from);
            return row[from..to].iter().collect();
        }

        let mut out = String::new();
        let first: Vec<char> = lines[start_row].chars().collect();
        let from = start_col.min(first.len());
        out.extend(&first[from..]);
        out.push('\n');
        for line in &lines[start_row + 1..end_row] {
            out.push_str(line);
            out.push('\n');
        }
        let last_row: Vec<char> = lines[end_row].chars().collect();
        let to = (end_col + 1).min(last_row.len());
        out.extend(&last_row[..to]);
        out
    }

    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// Text of a single grid line (wide-char spacer cells collapsed), trimmed of
    /// trailing whitespace.
    fn row_text(&self, line: Line) -> String {
        let grid = self.term.grid();
        let mut text = String::with_capacity(self.term.columns());
        for col in 0..self.term.columns() {
            let cell = &grid[line][Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            text.push(cell.c);
        }
        text.trim_end().to_string()
    }

    /// The visible screen as one trimmed line per row.
    pub fn visible_lines(&self) -> Vec<String> {
        (0..self.term.screen_lines())
            .map(|row| self.row_text(Line(row as i32)))
            .collect()
    }

    /// Visible screen as newline-joined text (unchanged behavior).
    pub fn snapshot_text(&self) -> String {
        self.visible_lines().join("\n")
    }

    /// Number of scrollback (history) lines above the visible screen.
    pub fn history_size(&self) -> usize {
        self.term
            .grid()
            .total_lines()
            .saturating_sub(self.term.screen_lines())
    }

    /// Full serialization: every scrollback line above the visible screen,
    /// followed by the visible screen, one trimmed line per row. This is the
    /// native equivalent of the xterm serialize addon used by `read-screen`.
    pub fn serialize(&self) -> String {
        self.serialize_lines().join("\n")
    }

    /// Scrollback + visible as a line vector (top to bottom).
    pub fn serialize_lines(&self) -> Vec<String> {
        let history = self.history_size() as i32;
        let screen = self.term.screen_lines() as i32;
        (-history..screen)
            .map(|row| self.row_text(Line(row)))
            .collect()
    }

    /// The write cursor position as (row, column), clamped to the visible screen.
    pub fn cursor(&self) -> (usize, usize) {
        let point = self.term.grid().cursor.point;
        let rows = self.term.screen_lines().saturating_sub(1);
        let cols = self.term.columns().saturating_sub(1);
        let row = point.line.0.max(0) as usize;
        (row.min(rows), point.column.0.min(cols))
    }

    /// Search the full serialized buffer (scrollback + visible). Match line
    /// indices are relative to [`Self::serialize_lines`].
    pub fn search(
        &self,
        query: &str,
        options: crate::search::SearchOptions,
    ) -> Vec<crate::search::SearchMatch> {
        crate::search::search_lines(&self.serialize_lines(), query, options)
    }

    /// Detect links on the visible screen. Line indices are visible-row indices.
    pub fn links(&self) -> Vec<crate::links::DetectedLink> {
        crate::links::detect_links(&self.visible_lines())
    }
}

pub fn render_bytes_to_text(bytes: &[u8], columns: usize, rows: usize) -> String {
    let mut grid = TerminalGrid::new(GridSize::new(columns, rows));
    grid.advance(bytes);
    grid.snapshot_text()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_ansi_bytes_to_text() {
        let text = render_bytes_to_text(
            b"alpha\r\n\x1b[31mred\x1b[0m\r\nwide:\xE7\x8C\xAB\r\nemoji:\xF0\x9F\x9A\x80\r\n",
            20,
            8,
        );

        assert!(text.contains("alpha"));
        assert!(text.contains("red"));
        assert!(text.contains("wide:"));
        assert!(text.contains("emoji:"));
    }

    #[test]
    fn serialize_includes_scrollback_beyond_the_visible_screen() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 3));
        for row in 0..10 {
            grid.advance(format!("line{row}\r\n").as_bytes());
        }
        // Only 3 rows are visible, but serialize keeps the scrolled-off lines.
        assert!(grid.history_size() >= 7);
        let serialized = grid.serialize();
        assert!(serialized.contains("line0"), "serialized = {serialized:?}");
        assert!(serialized.contains("line9"));
        // The visible snapshot only shows the last rows.
        assert!(!grid.snapshot_text().contains("line0"));
    }

    #[test]
    fn search_finds_matches_across_scrollback() {
        let mut grid = TerminalGrid::new(GridSize::new(30, 3));
        for row in 0..8 {
            let level = if row == 2 { "ERROR" } else { "info" };
            grid.advance(format!("{level} message {row}\r\n").as_bytes());
        }
        let hits = grid.search("error", crate::search::SearchOptions::default());
        assert_eq!(hits.len(), 1, "expected one scrollback match");
    }

    #[test]
    fn links_are_detected_on_the_visible_screen() {
        let mut grid = TerminalGrid::new(GridSize::new(40, 3));
        grid.advance(b"open https://example.com/docs now\r\n");
        let links = grid.links();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://example.com/docs");
    }

    #[test]
    fn cursor_tracks_written_position() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 4));
        grid.advance(b"abc");
        let (row, col) = grid.cursor();
        assert_eq!(row, 0);
        assert_eq!(col, 3);
    }

    #[test]
    fn captures_osc52_clipboard_store() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 4));
        // OSC 52 ; c ; base64("hello") BEL  ->  "aGVsbG8=" is base64 of "hello".
        grid.advance(b"\x1b]52;c;aGVsbG8=\x07");
        let stores = grid.take_clipboard_stores();
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].text, "hello");
        assert_eq!(stores[0].kind, ClipboardKind::Clipboard);
        // Draining is idempotent: a second call sees nothing new.
        assert!(grid.take_clipboard_stores().is_empty());
    }

    #[test]
    fn tracks_bracketed_paste_mode() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 4));
        assert!(!grid.bracketed_paste_active());
        grid.advance(b"\x1b[?2004h");
        assert!(grid.bracketed_paste_active());
        grid.advance(b"\x1b[?2004l");
        assert!(!grid.bracketed_paste_active());
    }

    #[test]
    fn region_text_extracts_single_and_multi_line_spans() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 4));
        grid.advance(b"hello world\r\nsecond line\r\nthird\r\n");
        let lines = grid.serialize_lines();
        let hello_row = lines.iter().position(|l| l.contains("hello")).unwrap();

        // Single-line span "world" (cols 6..=10 of "hello world").
        assert_eq!(grid.region_text((hello_row, 6), (hello_row, 10)), "world");
        // Reversed points give the same result.
        assert_eq!(grid.region_text((hello_row, 10), (hello_row, 6)), "world");

        // Multi-line span from "world" through "second".
        let second_row = hello_row + 1;
        let text = grid.region_text((hello_row, 6), (second_row, 5));
        assert_eq!(text, "world\nsecond");
    }
}
