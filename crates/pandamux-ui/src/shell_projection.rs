use pandamux_core::{
    PaneId, SplitDirection, SplitNode, SurfaceId, SurfaceType, WorkspaceId, WorkspaceState,
    find_leaf,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellProjection {
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub focused_pane_id: Option<PaneId>,
    pub zoomed_pane_id: Option<PaneId>,
    pub root: ShellNodeProjection,
    pub visible_panes: Vec<PaneProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellNodeProjection {
    Pane(PaneProjection),
    Split {
        direction: SplitDirection,
        ratio_percent: u8,
        first: Box<ShellNodeProjection>,
        second: Box<ShellNodeProjection>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneProjection {
    pub id: PaneId,
    pub is_focused: bool,
    pub is_zoomed: bool,
    pub surfaces: Vec<SurfaceProjection>,
    pub active_surface_id: Option<SurfaceId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceProjection {
    pub id: SurfaceId,
    pub surface_type: SurfaceType,
    pub is_active: bool,
}

pub fn project_workspace_shell(workspace: &WorkspaceState) -> ShellProjection {
    let root = match workspace.zoomed_pane_id.as_ref() {
        Some(pane_id) => find_leaf(&workspace.split_tree, pane_id)
            .map(|leaf| {
                ShellNodeProjection::Pane(project_pane(
                    leaf,
                    workspace.focused_pane_id.as_ref(),
                    workspace.zoomed_pane_id.as_ref(),
                ))
            })
            .unwrap_or_else(|| {
                project_node(
                    &workspace.split_tree,
                    workspace.focused_pane_id.as_ref(),
                    workspace.zoomed_pane_id.as_ref(),
                )
            }),
        None => project_node(
            &workspace.split_tree,
            workspace.focused_pane_id.as_ref(),
            workspace.zoomed_pane_id.as_ref(),
        ),
    };
    let mut visible_panes = Vec::new();
    collect_visible_panes(&root, &mut visible_panes);

    ShellProjection {
        workspace_id: workspace.id.clone(),
        title: workspace.title.clone(),
        focused_pane_id: workspace.focused_pane_id.clone(),
        zoomed_pane_id: workspace.zoomed_pane_id.clone(),
        root,
        visible_panes,
    }
}

fn project_node(
    node: &SplitNode,
    focused_pane_id: Option<&PaneId>,
    zoomed_pane_id: Option<&PaneId>,
) -> ShellNodeProjection {
    match node {
        SplitNode::Leaf(leaf) => {
            ShellNodeProjection::Pane(project_pane(leaf, focused_pane_id, zoomed_pane_id))
        }
        SplitNode::Branch(branch) => ShellNodeProjection::Split {
            direction: branch.direction,
            ratio_percent: ratio_percent(branch.ratio),
            first: Box::new(project_node(
                &branch.children[0],
                focused_pane_id,
                zoomed_pane_id,
            )),
            second: Box::new(project_node(
                &branch.children[1],
                focused_pane_id,
                zoomed_pane_id,
            )),
        },
    }
}

fn project_pane(
    leaf: &pandamux_core::LeafNode,
    focused_pane_id: Option<&PaneId>,
    zoomed_pane_id: Option<&PaneId>,
) -> PaneProjection {
    let active_surface_id = leaf
        .surfaces
        .get(leaf.active_surface_index)
        .map(|surface| surface.id.clone());
    let surfaces = leaf
        .surfaces
        .iter()
        .map(|surface| SurfaceProjection {
            id: surface.id.clone(),
            surface_type: surface.surface_type.clone(),
            is_active: active_surface_id.as_ref() == Some(&surface.id),
        })
        .collect();

    PaneProjection {
        id: leaf.pane_id.clone(),
        is_focused: focused_pane_id == Some(&leaf.pane_id),
        is_zoomed: zoomed_pane_id == Some(&leaf.pane_id),
        surfaces,
        active_surface_id,
    }
}

fn collect_visible_panes(node: &ShellNodeProjection, panes: &mut Vec<PaneProjection>) {
    match node {
        ShellNodeProjection::Pane(pane) => panes.push(pane.clone()),
        ShellNodeProjection::Split { first, second, .. } => {
            collect_visible_panes(first, panes);
            collect_visible_panes(second, panes);
        }
    }
}

fn ratio_percent(ratio: f32) -> u8 {
    (ratio.clamp(0.0, 1.0) * 100.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_core::{
        AppIntent, AppState, PaneId, PaneIntent, SplitDirection, SplitPaneParams, SurfaceType,
    };

    #[test]
    fn projects_default_workspace_shell() {
        let state = AppState::default();
        let projection = project_workspace_shell(state.active_workspace().unwrap());

        assert_eq!(projection.title, "Workspace");
        assert_eq!(projection.visible_panes.len(), 1);
        assert_eq!(projection.visible_panes[0].id, PaneId::from("pane-default"));
        assert!(projection.visible_panes[0].is_focused);
        assert_eq!(projection.visible_panes[0].surfaces.len(), 1);
    }

    #[test]
    fn projects_split_tree_and_zoomed_view() {
        let mut state = AppState::default();
        let split = state
            .apply(AppIntent::Pane(PaneIntent::Split(SplitPaneParams {
                workspace_id: None,
                target_pane_id: Some(PaneId::from("pane-default")),
                target_surface_id: None,
                direction: SplitDirection::Horizontal,
                surface_type: SurfaceType::Terminal,
            })))
            .expect("split should apply");
        let pandamux_core::AppDelta::PaneSplit { pane_id, .. } = split else {
            panic!("expected pane split");
        };

        let projection = project_workspace_shell(state.active_workspace().unwrap());
        assert_eq!(projection.visible_panes.len(), 2);
        assert!(matches!(
            projection.root,
            ShellNodeProjection::Split {
                direction: SplitDirection::Horizontal,
                ratio_percent: 50,
                ..
            }
        ));

        state
            .apply(AppIntent::Pane(PaneIntent::Zoom {
                workspace_id: None,
                pane_id: Some(pane_id.clone()),
            }))
            .expect("zoom should apply");
        let projection = project_workspace_shell(state.active_workspace().unwrap());

        assert_eq!(projection.visible_panes.len(), 1);
        assert_eq!(projection.visible_panes[0].id, pane_id);
        assert!(projection.visible_panes[0].is_zoomed);
        assert!(matches!(projection.root, ShellNodeProjection::Pane(_)));
    }
}
