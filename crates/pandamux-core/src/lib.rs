pub mod ids;
pub mod protocol;
pub mod split_tree;
pub mod state;

pub use ids::{PaneId, SurfaceId, WindowId, WorkspaceId};
pub use protocol::{RpcError, RpcRequest, RpcResponse};
pub use split_tree::{
    BranchNode, GridLayoutResult, LeafNode, SplitDirection, SplitNode, SurfaceRef, SurfaceType,
    adjust_pane_ratio, build_grid_layout, collect_active_terminal_surface_ids, create_leaf,
    find_leaf, find_pane_id_for_surface, get_all_pane_ids, remove_leaf, replace_leaf, split_node,
    update_ratio,
};
pub use state::{
    AppDelta, AppIntent, AppState, Capabilities, LayoutGridParams, PaneIntent, SplitPaneParams,
    SurfaceIntent, SystemIntent, WorkspaceIntent, WorkspaceState, WorkspaceSummary,
};
