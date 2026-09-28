#[cfg(feature = "pty")]
pub mod byte_session;
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
pub mod size;

#[cfg(feature = "pty")]
pub use byte_session::PtyByteSessionManager;
pub use clipboard::{ClipboardKind, ClipboardPolicy, ClipboardStore, wrap_paste};
pub use cwd::CwdScanner;
#[cfg(feature = "grid")]
pub use grid::{
    CellColor, DEFAULT_GRID_SIZE, DEFAULT_SCROLLBACK_LINES, ScreenCells, ScrollAmount,
    SelectionMode, SelectionSpan, StyledCell, TermModes, TerminalGrid, render_bytes_to_text,
};
pub use links::{DetectedLink, detect_links};
#[cfg(all(feature = "pty", feature = "grid"))]
pub use pty::{PtyCapture, capture_pty_command};
#[cfg(feature = "pty")]
pub use pty::{PtyCommand, shell_marker_command};
pub use ring_buffer::{DEFAULT_RING_BUFFER_CAPACITY, TerminalRingBuffer};
pub use search::{SearchMatch, SearchOptions, search_lines};
#[cfg(all(feature = "pty", feature = "grid"))]
pub use session::PtySessionManager;
pub use shell::{ShellType, chunk_write, resolve_powershell, resolve_shell, shell_type};
pub use size::GridSize;
