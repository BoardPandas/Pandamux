pub mod agent;
pub mod config;
pub mod i18n;
pub mod ids;
pub mod notification;
pub mod project;
pub mod project_registry;
pub mod protocol;
pub mod settings;
pub mod sidebar;
pub mod split_tree;
pub mod ssh;
pub mod state;
pub mod surface_content;

pub use agent::{AgentInfo, AgentRegistry, AgentStatus, SpawnStrategy};
pub use config::{
    Appearance, Theme, ThemeStore, import_windows_terminal, parse_ghostty_theme, parse_hex,
};
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
    ProjectMatcher, ProjectRecord, ProjectResolution, assign_workspace_project,
    ensure_project_registry, normalize_folder_name, normalize_git_remote, parse_git_remote_url,
    record_location, resolve_project_id,
};
pub use protocol::{RpcError, RpcRequest, RpcResponse};
pub use settings::{
    KeyboardSettings, SETTINGS_SCHEMA_VERSION, TerminalSettings, UiSettings, UserSettings,
    settings_get, settings_set,
};
pub use sidebar::{LogEntry, Progress, SidebarState, StatusEntry};
pub use split_tree::{
    BranchNode, DropZone, GridLayoutResult, LeafNode, MoveResult, SessionType, SplitDirection,
    SplitNode, SurfaceRef, SurfaceType, adjust_pane_ratio, build_grid_layout,
    collect_active_terminal_surface_ids, create_leaf, create_leaf_with_ids, find_leaf,
    find_pane_id_for_surface, get_all_pane_ids, move_surface, remove_leaf, replace_leaf,
    split_node, update_ratio,
};
pub use ssh::{ClipboardConfig, SshAuthConfig, SshHostProfile, SshProfiles, parse_ssh_config};
pub use state::{
    APP_STATE_SCHEMA_VERSION, AppDelta, AppIntent, AppState, Capabilities, LayoutGridParams,
    PaneIntent, ProjectIntent, SplitPaneParams, SurfaceIntent, SystemIntent, WorkspaceIntent,
    WorkspaceState, WorkspaceSummary,
};
pub use surface_content::SurfaceContents;
