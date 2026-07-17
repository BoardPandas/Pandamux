pub mod clipboard;
pub mod cwd;
pub mod grid;
pub mod links;
pub mod pty;
pub mod search;
pub mod session;
pub mod shell;
pub mod ssh;

pub use clipboard::{ClipboardKind, ClipboardPolicy, ClipboardStore, wrap_paste};
pub use cwd::CwdScanner;
pub use grid::{
    CellColor, DEFAULT_GRID_SIZE, DEFAULT_SCROLLBACK_LINES, GridSize, ScreenCells, ScrollAmount,
    SelectionMode, SelectionSpan, StyledCell, TermModes, TerminalGrid, render_bytes_to_text,
};
pub use links::{DetectedLink, detect_links};
pub use pty::{PtyCapture, PtyCommand, capture_pty_command, shell_marker_command};
pub use search::{SearchMatch, SearchOptions, search_lines};
pub use session::PtySessionManager;
pub use shell::{ShellType, chunk_write, resolve_powershell, resolve_shell, shell_type};
pub use ssh::{
    RemoteFolderEntry, RemoteFolderListing, RemoteSessionManager, RemoteStatus, SshAuth, SshConfig,
    SshConnectionPool, SshErrorCategory, SshFailure, browse_remote_folders, read_remote_file,
};
