#[cfg(feature = "iced-runtime")]
pub mod chrome;
#[cfg(feature = "iced-runtime")]
pub mod iced_shell;
pub mod shell_projection;
#[cfg(feature = "iced-runtime")]
pub mod theme;

#[cfg(feature = "iced-runtime")]
pub use chrome::{ChromeState, RailItem};
#[cfg(feature = "iced-runtime")]
pub use iced_shell::{
    ShellMessage, ShellViewModel, TerminalSnapshot, app_view, shell_view, terminal_viewport,
};
pub use shell_projection::{
    ColumnProjection, PaneProjection, ShellNodeProjection, ShellProjection, SurfaceProjection,
    project_workspace_shell,
};
#[cfg(feature = "iced-runtime")]
pub use theme::{Accent, Palette, ShellKind, UiTheme};

pub fn crate_name() -> &'static str {
    "pandamux-ui"
}
