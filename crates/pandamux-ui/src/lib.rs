#[cfg(feature = "iced-runtime")]
pub mod chrome;
#[cfg(feature = "iced-runtime")]
pub mod command_palette;
#[cfg(feature = "iced-runtime")]
pub mod content_views;
#[cfg(feature = "iced-runtime")]
pub mod context_menu;
#[cfg(feature = "iced-runtime")]
pub mod iced_shell;
#[cfg(feature = "iced-runtime")]
pub mod icons;
#[cfg(feature = "iced-runtime")]
pub mod metrics;
#[cfg(feature = "iced-runtime")]
pub mod overlays;
#[cfg(feature = "iced-runtime")]
pub mod session_launcher;
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
pub use context_menu::{ContextMenuAction, ContextMenuViewState};
#[cfg(feature = "iced-runtime")]
pub use iced_shell::{
    DragView, LinkSpan, ShellMessage, ShellViewModel, TerminalSnapshot, app_view, shell_view,
    terminal_viewport,
};
#[cfg(feature = "iced-runtime")]
pub use icons::{Icon, icon};
#[cfg(feature = "iced-runtime")]
pub use metrics::CellMetrics;
#[cfg(feature = "iced-runtime")]
pub use overlays::{FindViewState, NotificationCard, NotificationsViewState};
#[cfg(feature = "iced-runtime")]
pub use session_launcher::{
    LauncherStep, SessionLauncherViewState, SshProfileForm, session_launcher,
};
#[cfg(feature = "iced-runtime")]
pub use session_panel::{
    SessionEntry, SessionGroup, SessionGrouping, SessionsViewState, project_sessions,
    project_sessions_with_profiles, session_panel,
};
#[cfg(feature = "iced-runtime")]
pub use settings::{SettingsSection, SettingsViewState, TerminalToggle};
pub use shell_projection::{
    ColumnProjection, PaneProjection, ShellNodeProjection, ShellProjection, SurfaceProjection,
    project_workspace_shell,
};
#[cfg(feature = "iced-runtime")]
pub use theme::{Accent, Palette, ShellKind, TermScheme, UiTheme};

pub fn crate_name() -> &'static str {
    "pandamux-ui"
}
