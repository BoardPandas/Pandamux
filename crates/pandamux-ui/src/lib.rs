#[cfg(feature = "iced-runtime")]
pub mod iced_shell;
pub mod shell_projection;

#[cfg(feature = "iced-runtime")]
pub use iced_shell::{
    ShellMessage, ShellViewModel, TerminalSnapshot, shell_view, terminal_viewport,
};
pub use shell_projection::{
    PaneProjection, ShellNodeProjection, ShellProjection, SurfaceProjection,
    project_workspace_shell,
};

pub fn crate_name() -> &'static str {
    "pandamux-ui"
}
