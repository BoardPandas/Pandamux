use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi;

#[derive(Clone, Copy, Debug)]
pub struct SpikeTermSize {
    pub columns: usize,
    pub screen_lines: usize,
}

impl SpikeTermSize {
    pub fn new(columns: usize, screen_lines: usize) -> Self {
        Self {
            columns,
            screen_lines,
        }
    }
}

impl Dimensions for SpikeTermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
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
    pub fn new(columns: usize, rows: usize) -> Self {
        let size = SpikeTermSize::new(columns, rows);
        let config = Config {
            scrolling_history: rows * 4,
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

    pub fn snapshot_text(&self) -> String {
        let grid = self.term.grid();
        let mut lines = Vec::with_capacity(self.term.screen_lines());

        for row in 0..self.term.screen_lines() {
            let line = Line(row as i32);
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

            lines.push(text.trim_end().to_string());
        }

        lines.join("\n")
    }
}

pub fn render_bytes_to_text(bytes: &[u8], columns: usize, rows: usize) -> String {
    let mut grid = TerminalGrid::new(columns, rows);
    grid.advance(bytes);
    grid.snapshot_text()
}
