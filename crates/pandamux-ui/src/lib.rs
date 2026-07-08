#[cfg(feature = "iced-runtime")]
pub mod chrome;
#[cfg(feature = "iced-runtime")]
pub mod iced_shell;
#[cfg(feature = "iced-runtime")]
pub mod overlays;
pub mod shell_projection;
#[cfg(feature = "iced-runtime")]
pub mod theme;

#[cfg(feature = "iced-runtime")]
pub use chrome::{ChromeState, RailItem, SessionActivity};
#[cfg(feature = "iced-runtime")]
pub use iced_shell::{
    LinkSpan, ShellMessage, ShellViewModel, TerminalSnapshot, app_view, shell_view,
    terminal_viewport,
};
#[cfg(feature = "iced-runtime")]
pub use overlays::{FindViewState, NotificationCard, NotificationsViewState};
pub use shell_projection::{
    ColumnProjection, PaneProjection, ShellNodeProjection, ShellProjection, SurfaceProjection,
    project_workspace_shell,
};
#[cfg(feature = "iced-runtime")]
pub use theme::{Accent, Palette, ShellKind, UiTheme};

pub fn crate_name() -> &'static str {
    "pandamux-ui"
}
