use crate::clipboard::{ClipboardKind, ClipboardStore};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{ClipboardType, Config, Osc52, Term, TermMode};
use alacritty_terminal::vte::ansi::{self, Color, NamedColor};
use std::sync::{Arc, Mutex};

/// Scrollback history retained per surface unless a setting overrides it via
/// [`TerminalGrid::with_scrollback`] / [`TerminalGrid::set_scrollback`].
pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

/// Grid dimensions used before the UI has reported a real viewport size.
pub const DEFAULT_GRID_SIZE: GridSize = GridSize {
    columns: 120,
    rows: 30,
};

/// A terminal cell color independent of alacritty's internal types. Default and
/// indexed colors stay symbolic so the UI can apply the active terminal theme;
/// explicit RGB colors pass through unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellColor {
    /// The theme's default foreground (text) color.
    Default,
    /// The theme's default background color.
    Background,
    /// An ANSI/xterm palette entry. The UI resolves the first 16 entries against
    /// the active terminal theme and the remaining entries against xterm-256.
    Indexed(u8),
    /// A concrete color.
    Rgb(u8, u8, u8),
}

/// A single rendered grid cell with resolved styling. Reverse-video (`INVERSE`)
/// is already applied by swapping `fg`/`bg`, so the UI does not special-case it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyledCell {
    pub c: char,
    pub fg: CellColor,
    pub bg: CellColor,
    pub bold: bool,
}

/// Highlighted selection cells on one visible row, in rendered-cell indices
/// (wide-char spacers already collapsed, matching [`ScreenCells::rows`]).
/// `start..=end` is inclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionSpan {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Terminal mode flags the UI needs for input-routing decisions (wheel vs
/// arrow translation, selection vs mouse reporting, paste wrapping).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TermModes {
    pub alt_screen: bool,
    pub mouse_reporting: bool,
    pub alternate_scroll: bool,
    pub app_cursor: bool,
    pub bracketed_paste: bool,
}

/// A viewport scroll request in display terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollAmount {
    /// Positive scrolls up into history, negative scrolls back toward the tail.
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// Mouse selection granularity: click-drag, double-click, triple-click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    Simple,
    Word,
    Line,
}

/// The visible screen as styled cells (one row per screen line, full width, not
/// trimmed so trailing background colors survive), plus the write-cursor position
/// as `(row, column)` and the view state the UI needs for scrollback and
/// selection rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenCells {
    pub rows: Vec<Vec<StyledCell>>,
    pub cursor: (usize, usize),
    /// False while the view is scrolled up into history (the write cursor sits
    /// below the visible region).
    pub cursor_visible: bool,
    /// How many lines above the tail the view is scrolled (0 = following).
    pub display_offset: usize,
    /// Scrollback lines above the visible screen.
    pub history_size: usize,
    /// Selection highlight spans intersecting the visible rows.
    pub selection: Vec<SelectionSpan>,
    pub modes: TermModes,
}

impl Default for ScreenCells {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            cursor: (0, 0),
            cursor_visible: true,
            display_offset: 0,
            history_size: 0,
            selection: Vec::new(),
            modes: TermModes::default(),
        }
    }
}

/// Map an alacritty cell color to PandaMUX's theme-independent representation.
fn resolve_color(color: Color) -> CellColor {
    match color {
        Color::Spec(rgb) => CellColor::Rgb(rgb.r, rgb.g, rgb.b),
        Color::Indexed(index) => CellColor::Indexed(index),
        Color::Named(named) => match named {
            NamedColor::Background => CellColor::Background,
            NamedColor::Foreground
            | NamedColor::BrightForeground
            | NamedColor::DimForeground
            | NamedColor::Cursor => CellColor::Default,
            other => {
                let n = other as usize;
                if n < 16 {
                    CellColor::Indexed(n as u8)
                } else if (259..=266).contains(&n) {
                    // Dim black..dim white -> their normal base color (0..=7).
                    CellColor::Indexed((n - 259) as u8)
                } else {
                    CellColor::Default
                }
            }
        },
    }
}

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

/// The alacritty config PandaMUX runs every grid with. OnlyCopy is the secure
/// default and exactly F1's policy: accept a remote's OSC 52 copy, deny its
/// clipboard-read (load) query.
fn grid_config(scrollback_lines: usize) -> Config {
    Config {
        scrolling_history: scrollback_lines,
        osc52: Osc52::OnlyCopy,
        ..Config::default()
    }
}

impl TerminalGrid {
    pub fn new(size: GridSize) -> Self {
        Self::with_scrollback(size, DEFAULT_SCROLLBACK_LINES)
    }

    pub fn with_scrollback(size: GridSize, scrollback_lines: usize) -> Self {
        let clipboard: Arc<Mutex<Vec<ClipboardStore>>> = Arc::default();
        let listener = Listener {
            clipboard: clipboard.clone(),
        };

        Self {
            parser: ansi::Processor::new(),
            term: Term::new(grid_config(scrollback_lines), &size, listener),
            clipboard,
        }
    }

    /// Change how much history the grid retains, in place (a settings change).
    pub fn set_scrollback(&mut self, scrollback_lines: usize) {
        self.term.set_options(grid_config(scrollback_lines));
    }

    /// Resize preserving contents and history (alacritty reflows), unlike
    /// recreating the grid. Resizing the PTY or SSH channel to match is the
    /// caller's responsibility.
    pub fn resize(&mut self, size: GridSize) {
        self.term.resize(size);
    }

    /// Scroll the viewport within scrollback. New output while scrolled up does
    /// not move the view (alacritty pins the offset as history grows).
    pub fn scroll_display(&mut self, amount: ScrollAmount) {
        let scroll = match amount {
            ScrollAmount::Lines(delta) => Scroll::Delta(delta),
            ScrollAmount::PageUp => Scroll::PageUp,
            ScrollAmount::PageDown => Scroll::PageDown,
            ScrollAmount::Top => Scroll::Top,
            ScrollAmount::Bottom => Scroll::Bottom,
        };
        self.term.scroll_display(scroll);
    }

    /// Lines above the tail the view is currently scrolled (0 = following).
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Convert display coordinates (row 0 = top visible row) to a buffer point,
    /// clamped to the grid.
    fn buffer_point(&self, line: usize, col: usize) -> Point {
        let offset = self.display_offset() as i32;
        let max_line = self.term.screen_lines().saturating_sub(1) as i32;
        let history = self.history_size() as i32;
        let line = ((line as i32).min(max_line) - offset).max(-history);
        let col = col.min(self.term.columns().saturating_sub(1));
        Point::new(Line(line), Column(col))
    }

    /// Begin a mouse selection at a display coordinate. Replaces any existing
    /// selection.
    pub fn start_selection(
        &mut self,
        mode: SelectionMode,
        line: usize,
        col: usize,
        right_half: bool,
    ) {
        let ty = match mode {
            SelectionMode::Simple => SelectionType::Simple,
            SelectionMode::Word => SelectionType::Semantic,
            SelectionMode::Line => SelectionType::Lines,
        };
        let side = if right_half { Side::Right } else { Side::Left };
        self.term.selection = Some(Selection::new(ty, self.buffer_point(line, col), side));
    }

    /// Extend the active selection to a display coordinate (mouse drag).
    pub fn update_selection(&mut self, line: usize, col: usize, right_half: bool) {
        let point = self.buffer_point(line, col);
        let side = if right_half { Side::Right } else { Side::Left };
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn has_selection(&self) -> bool {
        self.term
            .selection
            .as_ref()
            .is_some_and(|selection| !selection.is_empty())
    }

    /// The selected text, if a non-empty selection exists.
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    /// Select the whole buffer (scrollback + visible screen).
    pub fn select_all(&mut self) {
        let history = self.history_size() as i32;
        let last_line = self.term.screen_lines().saturating_sub(1) as i32;
        let last_col = self.term.columns().saturating_sub(1);
        let mut selection = Selection::new(
            SelectionType::Lines,
            Point::new(Line(-history), Column(0)),
            Side::Left,
        );
        selection.update(Point::new(Line(last_line), Column(last_col)), Side::Right);
        self.term.selection = Some(selection);
    }

    /// Drop scrollback history and snap the view back to the tail. The visible
    /// screen is left as-is (callers nudge the shell to repaint the prompt).
    pub fn clear_buffer(&mut self) {
        self.term.selection = None;
        self.term.grid_mut().clear_history();
        self.term.scroll_display(Scroll::Bottom);
    }

    /// Terminal mode flags relevant to UI input routing.
    pub fn modes(&self) -> TermModes {
        let mode = self.term.mode();
        TermModes {
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            mouse_reporting: mode.intersects(TermMode::MOUSE_MODE),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
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

    /// The visible screen as one trimmed line per row, honoring the current
    /// scroll offset (row 0 = top of the viewport, which may be history).
    pub fn visible_lines(&self) -> Vec<String> {
        let offset = self.display_offset() as i32;
        (0..self.term.screen_lines())
            .map(|row| self.row_text(Line(row as i32 - offset)))
            .collect()
    }

    /// The visible screen as styled cells plus the cursor position, rendered at
    /// the current scroll offset. Wide-char spacer cells are collapsed (matching
    /// [`Self::row_text`]); rows keep their full width so a reverse-video
    /// highlight that runs to the line end (e.g. a PSReadLine menu selection)
    /// still paints its background. Selection spans are reported in
    /// rendered-cell indices so the UI can highlight without re-deriving the
    /// spacer collapse.
    pub fn visible_cells(&self) -> ScreenCells {
        let grid = self.term.grid();
        let columns = self.term.columns();
        let offset = self.display_offset();
        let selection_range = self
            .term
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(&self.term));
        let mut selection = Vec::new();
        let rows = (0..self.term.screen_lines())
            .map(|row| {
                let line = Line(row as i32 - offset as i32);
                let mut cells = Vec::with_capacity(columns);
                let mut span: Option<(usize, usize)> = None;
                for col in 0..columns {
                    let cell = &grid[line][Column(col)];
                    if cell
                        .flags
                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                    {
                        continue;
                    }
                    if let Some(range) = &selection_range {
                        if range.contains(Point::new(line, Column(col))) {
                            let index = cells.len();
                            span = Some(match span {
                                Some((start, _)) => (start, index),
                                None => (index, index),
                            });
                        }
                    }
                    let mut fg = resolve_color(cell.fg);
                    let mut bg = resolve_color(cell.bg);
                    // Reverse video: swap fg/bg up front so the UI stays dumb.
                    if cell.flags.contains(Flags::INVERSE) {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    cells.push(StyledCell {
                        c: cell.c,
                        fg,
                        bg,
                        bold: cell.flags.contains(Flags::BOLD),
                    });
                }
                if let Some((start, end)) = span {
                    selection.push(SelectionSpan {
                        line: row,
                        start,
                        end,
                    });
                }
                cells
            })
            .collect();
        ScreenCells {
            rows,
            cursor: self.cursor(),
            cursor_visible: offset == 0,
            display_offset: offset,
            history_size: self.history_size(),
            selection,
            modes: self.modes(),
        }
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
    fn visible_cells_resolve_color_and_reverse_video() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 3));
        // "A" default, "R" red foreground, "V" reverse-video (SGR 7), then reset.
        grid.advance(b"A\x1b[31mR\x1b[0m\x1b[7mV\x1b[0m");
        let screen = grid.visible_cells();
        let row = &screen.rows[0];

        // Plain cell keeps the default (symbolic) foreground.
        assert_eq!(row[0].c, 'A');
        assert_eq!(row[0].fg, CellColor::Default);
        assert_eq!(row[0].bg, CellColor::Background);

        // Named ANSI colors stay indexed for theme-aware resolution in the UI.
        assert_eq!(row[1].c, 'R');
        assert_eq!(row[1].fg, CellColor::Indexed(1));

        // Reverse video swaps default fg/bg so the cell paints an inverted block.
        assert_eq!(row[2].c, 'V');
        assert_eq!(row[2].fg, CellColor::Background);
        assert_eq!(row[2].bg, CellColor::Default);
    }

    #[test]
    fn visible_cells_report_cursor_position() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 3));
        grid.advance(b"hi");
        let screen = grid.visible_cells();
        assert_eq!(screen.cursor, (0, 2));
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
    fn resize_preserves_screen_and_scrollback() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 5));
        for row in 0..50 {
            grid.advance(format!("line{row}\r\n").as_bytes());
        }
        let history_before = grid.history_size();
        assert!(history_before >= 40, "history was {history_before}");

        grid.resize(GridSize::new(30, 8));
        assert!(
            grid.serialize().contains("line0"),
            "scrollback should survive a resize"
        );
        assert!(grid.snapshot_text().contains("line49"));

        grid.resize(GridSize::new(10, 4));
        assert!(grid.serialize().contains("line0"));
    }

    #[test]
    fn default_scrollback_is_generous_and_configurable() {
        let mut grid = TerminalGrid::new(GridSize::new(10, 3));
        for row in 0..200 {
            grid.advance(format!("l{row}\r\n").as_bytes());
        }
        // Well past the old rows * 4 cap.
        assert!(grid.history_size() >= 190);

        let mut small = TerminalGrid::with_scrollback(GridSize::new(10, 3), 20);
        for row in 0..200 {
            small.advance(format!("l{row}\r\n").as_bytes());
        }
        assert!(small.history_size() <= 20);
    }

    #[test]
    fn scroll_display_pins_view_and_reports_offset() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 3));
        for row in 0..10 {
            grid.advance(format!("line{row}\r\n").as_bytes());
        }
        assert_eq!(grid.display_offset(), 0);

        grid.scroll_display(ScrollAmount::Lines(5));
        assert_eq!(grid.display_offset(), 5);
        let scrolled_up = grid.visible_lines();
        assert!(scrolled_up[0].contains("line3"), "lines = {scrolled_up:?}");

        // New output must not yank the view back down.
        grid.advance(b"line10\r\nline11\r\n");
        assert!(grid.display_offset() >= 5);
        let cells = grid.visible_cells();
        assert!(!cells.cursor_visible);
        assert!(cells.display_offset >= 5);
        assert!(cells.history_size >= 7);

        grid.scroll_display(ScrollAmount::Bottom);
        assert_eq!(grid.display_offset(), 0);
        assert!(grid.visible_cells().cursor_visible);

        grid.scroll_display(ScrollAmount::Top);
        assert_eq!(grid.display_offset(), grid.history_size());
    }

    #[test]
    fn simple_selection_yields_text_and_visible_spans() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 4));
        grid.advance(b"hello world\r\nsecond line\r\n");

        assert!(!grid.has_selection());
        grid.start_selection(SelectionMode::Simple, 0, 6, false);
        grid.update_selection(0, 10, true);
        assert!(grid.has_selection());
        assert_eq!(grid.selection_text().as_deref(), Some("world"));

        let cells = grid.visible_cells();
        assert_eq!(cells.selection.len(), 1);
        assert_eq!(
            cells.selection[0],
            SelectionSpan {
                line: 0,
                start: 6,
                end: 10
            }
        );

        grid.clear_selection();
        assert!(!grid.has_selection());
        assert!(grid.visible_cells().selection.is_empty());
    }

    #[test]
    fn word_and_line_selection_modes() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 4));
        grid.advance(b"hello world\r\n");

        grid.start_selection(SelectionMode::Word, 0, 7, false);
        assert_eq!(grid.selection_text().as_deref(), Some("world"));

        grid.start_selection(SelectionMode::Line, 0, 3, false);
        let text = grid.selection_text().unwrap_or_default();
        assert!(text.contains("hello world"), "text = {text:?}");
    }

    #[test]
    fn select_all_covers_scrollback() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 3));
        for row in 0..10 {
            grid.advance(format!("line{row}\r\n").as_bytes());
        }
        grid.select_all();
        let text = grid.selection_text().unwrap_or_default();
        assert!(text.contains("line0"), "text = {text:?}");
        assert!(text.contains("line9"));
    }

    #[test]
    fn selection_survives_scrolling_the_view() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 3));
        for row in 0..10 {
            grid.advance(format!("line{row}\r\n").as_bytes());
        }
        // Select the word on the bottom visible row, then scroll up: the text
        // stays anchored to the content, and the visible spans move off-screen.
        grid.start_selection(SelectionMode::Word, 1, 2, false);
        let selected = grid.selection_text().unwrap_or_default();
        grid.scroll_display(ScrollAmount::Lines(5));
        assert_eq!(grid.selection_text().unwrap_or_default(), selected);
    }

    #[test]
    fn clear_buffer_drops_history_and_snaps_to_tail() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 3));
        for row in 0..10 {
            grid.advance(format!("line{row}\r\n").as_bytes());
        }
        grid.scroll_display(ScrollAmount::Lines(4));
        grid.clear_buffer();
        assert_eq!(grid.history_size(), 0);
        assert_eq!(grid.display_offset(), 0);
        // The visible screen itself is untouched.
        assert!(grid.snapshot_text().contains("line9"));
    }

    #[test]
    fn modes_track_alt_screen_and_mouse_reporting() {
        let mut grid = TerminalGrid::new(GridSize::new(20, 3));
        let modes = grid.modes();
        assert!(!modes.alt_screen);
        assert!(!modes.mouse_reporting);

        // Enter the alternate screen (DECSET 1049) and SGR mouse mode (1002/1006).
        grid.advance(b"\x1b[?1049h\x1b[?1002h\x1b[?1006h");
        let modes = grid.modes();
        assert!(modes.alt_screen);
        assert!(modes.mouse_reporting);
        // History is unavailable on the alt screen.
        assert_eq!(grid.visible_cells().history_size, 0);

        grid.advance(b"\x1b[?1002l\x1b[?1049l");
        let modes = grid.modes();
        assert!(!modes.alt_screen);
        assert!(!modes.mouse_reporting);
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
