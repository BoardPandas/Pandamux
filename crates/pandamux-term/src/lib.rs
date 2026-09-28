pub mod clipboard;
pub mod cwd;
#[cfg(feature = "grid")]
pub mod grid;
pub mod links;
#[cfg(feature = "pty")]
pub mod pty;
pub mod ring_buffer;
pub mod search;
#[cfg(all(feature = "pty", feature = "grid"))]
pub mod session;
pub mod shell;

pub use clipboard::{ClipboardKind, ClipboardPolicy, ClipboardStore, wrap_paste};
pub use cwd::CwdScanner;
#[cfg(feature = "grid")]
pub use grid::{
    CellColor, DEFAULT_GRID_SIZE, DEFAULT_SCROLLBACK_LINES, GridSize, ScreenCells, ScrollAmount,
    SelectionMode, SelectionSpan, StyledCell, TermModes, TerminalGrid, render_bytes_to_text,
};
pub use links::{DetectedLink, detect_links};
#[cfg(feature = "pty")]
pub use pty::{PtyCapture, PtyCommand, capture_pty_command, shell_marker_command};
pub use ring_buffer::{DEFAULT_RING_BUFFER_CAPACITY, TerminalRingBuffer};
pub use search::{SearchMatch, SearchOptions, search_lines};
#[cfg(all(feature = "pty", feature = "grid"))]
pub use session::PtySessionManager;
pub use shell::{ShellType, chunk_write, resolve_powershell, resolve_shell, shell_type};
