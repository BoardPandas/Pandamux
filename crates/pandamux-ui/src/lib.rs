#[cfg(feature = "iced-runtime")]
pub mod chrome;
#[cfg(feature = "iced-runtime")]
pub mod command_palette;
#[cfg(feature = "iced-runtime")]
pub mod content_views;
#[cfg(feature = "iced-runtime")]
pub mod iced_shell;
#[cfg(feature = "iced-runtime")]
pub mod overlays;
#[cfg(feature = "iced-runtime")]
pub mod session_panel;
#[cfg(feature = "iced-runtime")]
pub mod settings;
pub mod shell_projection;
#[cfg(feature = "iced-runtime")]
pub mod theme;

#[cfg(feature = "iced-runtime")]
pub use chrome::{ChromeState, Overlay, RailItem, SessionActivity};
#[cfg(feature = "iced-runtime")]
pub use command_palette::{
    PaletteItem, PaletteViewState, QuickLaunchProfile, QuickLaunchViewState, filter_items,
};
#[cfg(feature = "iced-runtime")]
pub use iced_shell::{
    LinkSpan, ShellMessage, ShellViewModel, TerminalSnapshot, app_view, shell_view,
    terminal_viewport,
};
#[cfg(feature = "iced-runtime")]
pub use overlays::{FindViewState, NotificationCard, NotificationsViewState};
#[cfg(feature = "iced-runtime")]
pub use session_panel::{
    SessionEntry, SessionGroup, SessionGrouping, SessionsViewState, project_sessions, session_panel,
};
#[cfg(feature = "iced-runtime")]
pub use settings::{SettingsSection, SettingsViewState};
pub use shell_projection::{
    ColumnProjection, PaneProjection, ShellNodeProjection, ShellProjection, SurfaceProjection,
    project_workspace_shell,
};
#[cfg(feature = "iced-runtime")]
pub use theme::{Accent, Palette, ShellKind, UiTheme};

pub fn crate_name() -> &'static str {
    "pandamux-ui"
}
