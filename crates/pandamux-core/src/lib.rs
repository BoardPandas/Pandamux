pub mod agent;
pub mod config;
pub mod i18n;
pub mod ids;
pub mod notification;
pub mod protocol;
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
pub use ids::{PaneId, SurfaceId, WindowId, WorkspaceId};
pub use notification::{NewNotification, NotificationInfo, NotificationSource, Notifications};
pub use protocol::{RpcError, RpcRequest, RpcResponse};
pub use sidebar::{LogEntry, Progress, SidebarState, StatusEntry};
pub use split_tree::{
    BranchNode, DropZone, GridLayoutResult, LeafNode, MoveResult, SplitDirection, SplitNode,
    SurfaceRef, SurfaceType, adjust_pane_ratio, build_grid_layout,
    collect_active_terminal_surface_ids, create_leaf, find_leaf, find_pane_id_for_surface,
    get_all_pane_ids, move_surface, remove_leaf, replace_leaf, split_node, update_ratio,
};
pub use ssh::{ClipboardConfig, SshAuthConfig, SshHostProfile, SshProfiles, parse_ssh_config};
pub use state::{
    AppDelta, AppIntent, AppState, Capabilities, LayoutGridParams, PaneIntent, SplitPaneParams,
    SurfaceIntent, SystemIntent, WorkspaceIntent, WorkspaceState, WorkspaceSummary,
};
pub use surface_content::SurfaceContents;
