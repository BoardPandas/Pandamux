use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi;

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

#[derive(Clone, Copy, Debug)]
struct Listener;

impl EventListener for Listener {
    fn send_event(&self, _event: Event) {}
}

pub struct TerminalGrid {
    parser: ansi::Processor,
    term: Term<Listener>,
}

impl TerminalGrid {
    pub fn new(size: GridSize) -> Self {
        let config = Config {
            scrolling_history: size.rows * 4,
            ..Config::default()
        };

        Self {
            parser: ansi::Processor::new(),
            term: Term::new(config, &size, Listener),
        }
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
}
