pub mod home;
pub mod i18n;
pub mod ids;
pub mod notification;
pub mod project;
pub mod project_registry;
pub mod settings;
pub mod ssh;

pub use home::{HomeLayout, HomePane};
pub use i18n::{Locale, Localizer};
pub use ids::{PaneId, ProjectId, SshProfileId, SurfaceId, WindowId, WorkspaceId};
pub use notification::{NewNotification, NotificationInfo, NotificationSource, Notifications};
pub use project::{
    FolderBreadcrumb, FolderEntry, FolderListing, ProjectError, ProjectErrorCategory, ProjectKey,
    ProjectLocation, ProjectSpec, local_breadcrumbs, local_parent, normalize_posix_path,
    normalize_windows_path, posix_breadcrumbs, posix_parent, project_title, sort_directories,
    strip_windows_verbatim,
};
pub use project_registry::{
    LaunchConfig, ProjectMatcher, ProjectRecord, ProjectResolution, SessionType,
    normalize_folder_name, normalize_git_remote, parse_git_remote_url, record_location,
    resolve_project_id,
};
pub use settings::{
    KeyboardSettings, SETTINGS_SCHEMA_VERSION, TerminalSettings, UiSettings, UserSettings,
    settings_get, settings_set,
};
pub use ssh::{ClipboardConfig, SshAuthConfig, SshHostProfile, SshProfiles, parse_ssh_config};

