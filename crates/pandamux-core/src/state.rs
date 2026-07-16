use crate::ids::{PaneId, SurfaceId, WorkspaceId};
use crate::project::{ProjectKey, ProjectSpec};
use crate::split_tree::{
    DropZone, GridLayoutResult, MoveResult, SplitDirection, SplitNode, SurfaceRef, SurfaceType,
    build_grid_layout, create_leaf, create_leaf_with_ids, find_leaf, find_pane_id_for_surface,
    get_all_pane_ids, move_surface, remove_leaf, replace_leaf, split_node,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub workspaces: Vec<WorkspaceState>,
    pub active_workspace_id: WorkspaceId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub id: WorkspaceId,
    pub title: String,
    pub shell: String,
    #[serde(default)]
    pub project: ProjectSpec,
    pub split_tree: SplitNode,
    pub focused_pane_id: Option<PaneId>,
    pub zoomed_pane_id: Option<PaneId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub id: WorkspaceId,
    pub title: String,
    pub shell: String,
    pub project: ProjectSpec,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub browser: bool,
    pub layout_grid: bool,
    pub native: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "domain", content = "intent", rename_all = "lowercase")]
pub enum AppIntent {
    System(SystemIntent),
    Workspace(WorkspaceIntent),
    Pane(PaneIntent),
    Surface(SurfaceIntent),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SystemIntent {
    Identify,
    Capabilities,
    Tree { workspace_id: Option<WorkspaceId> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceIntent {
    Create {
        title: Option<String>,
        shell: Option<String>,
    },
    /// Transactional Project creation with caller-provided ids. The caller can
    /// prestart a terminal using these ids and commit only after it is ready.
    CreateProject {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
        surface_id: SurfaceId,
        title: String,
        shell: String,
        project: ProjectSpec,
    },
    Select {
        workspace_id: WorkspaceId,
    },
    Rename {
        workspace_id: WorkspaceId,
        title: String,
    },
    Close {
        workspace_id: WorkspaceId,
    },
    List,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneIntent {
    Split(SplitPaneParams),
    Close {
        workspace_id: Option<WorkspaceId>,
        pane_id: PaneId,
    },
    Focus {
        workspace_id: Option<WorkspaceId>,
        pane_id: PaneId,
    },
    Zoom {
        workspace_id: Option<WorkspaceId>,
        pane_id: Option<PaneId>,
    },
    LayoutGrid(LayoutGridParams),
    List {
        workspace_id: Option<WorkspaceId>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SurfaceIntent {
    Create {
        workspace_id: Option<WorkspaceId>,
        pane_id: Option<PaneId>,
        surface_type: SurfaceType,
    },
    /// Add a surface whose id was preallocated by the launch coordinator.
    CreateWithId {
        workspace_id: WorkspaceId,
        pane_id: Option<PaneId>,
        surface_id: SurfaceId,
        surface_type: SurfaceType,
    },
    Focus {
        workspace_id: Option<WorkspaceId>,
        surface_id: SurfaceId,
    },
    Close {
        workspace_id: Option<WorkspaceId>,
        surface_id: SurfaceId,
    },
    /// Drag-drop move of an existing surface to a drop target (plan Section 12.3).
    Move {
        workspace_id: Option<WorkspaceId>,
        surface_id: SurfaceId,
        target_pane_id: PaneId,
        zone: DropZone,
    },
    List {
        workspace_id: Option<WorkspaceId>,
        pane_id: Option<PaneId>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutGridParams {
    pub workspace_id: Option<WorkspaceId>,
    pub anchor_pane_id: Option<PaneId>,
    pub anchor_surface_id: Option<SurfaceId>,
    pub count: usize,
    pub surface_type: SurfaceType,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitPaneParams {
    pub workspace_id: Option<WorkspaceId>,
    pub target_pane_id: Option<PaneId>,
    pub target_surface_id: Option<SurfaceId>,
    pub direction: SplitDirection,
    pub surface_type: SurfaceType,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppDelta {
    Identified {
        name: String,
        version: String,
        native: bool,
    },
    CapabilitiesReported {
        capabilities: Capabilities,
    },
    TreeReported {
        workspace_id: WorkspaceId,
        tree: SplitNode,
    },
    WorkspaceCreated {
        workspace: WorkspaceSummary,
        tree: SplitNode,
    },
    WorkspaceSelected {
        workspace_id: WorkspaceId,
    },
    WorkspaceRenamed {
        workspace_id: WorkspaceId,
        title: String,
    },
    WorkspaceClosed {
        workspace_id: WorkspaceId,
    },
    WorkspaceListReported {
        workspaces: Vec<WorkspaceSummary>,
    },
    LayoutGridApplied {
        workspace_id: WorkspaceId,
        tree: SplitNode,
        new_pane_ids: Vec<PaneId>,
    },
    PaneSplit {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
        surface_id: SurfaceId,
        tree: SplitNode,
    },
    PaneClosed {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
        tree: SplitNode,
    },
    PaneFocused {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
    },
    PaneZoomed {
        workspace_id: WorkspaceId,
        pane_id: Option<PaneId>,
    },
    PaneListReported {
        workspace_id: WorkspaceId,
        panes: Vec<PaneSummary>,
    },
    SurfaceCreated {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
        surface: SurfaceRef,
    },
    SurfaceFocused {
        workspace_id: WorkspaceId,
        surface_id: SurfaceId,
    },
    SurfaceClosed {
        workspace_id: WorkspaceId,
        surface_id: SurfaceId,
    },
    SurfaceMoved {
        workspace_id: WorkspaceId,
        tree: SplitNode,
    },
    SurfaceListReported {
        workspace_id: WorkspaceId,
        surfaces: Vec<SurfaceRef>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneSummary {
    pub id: PaneId,
    pub surfaces: Vec<SurfaceRef>,
    pub active_surface_id: Option<SurfaceId>,
}

impl Default for AppState {
    fn default() -> Self {
        let workspace_id = WorkspaceId::from("ws-default");
        Self {
            workspaces: vec![WorkspaceState {
                id: workspace_id.clone(),
                title: "Workspace".to_string(),
                shell: "pwsh".to_string(),
                project: ProjectSpec::default(),
                split_tree: create_leaf(Some(PaneId::from("pane-default")), SurfaceType::Terminal),
                focused_pane_id: Some(PaneId::from("pane-default")),
                zoomed_pane_id: None,
            }],
            active_workspace_id: workspace_id,
        }
    }
}

impl AppState {
    pub fn apply(&mut self, intent: AppIntent) -> Result<AppDelta, String> {
        match intent {
            AppIntent::System(intent) => self.apply_system(intent),
            AppIntent::Workspace(intent) => self.apply_workspace(intent),
            AppIntent::Pane(intent) => self.apply_pane(intent),
            AppIntent::Surface(intent) => self.apply_surface(intent),
        }
    }

    pub fn active_workspace(&self) -> Option<&WorkspaceState> {
        self.workspace(&self.active_workspace_id)
    }

    pub fn workspace(&self, workspace_id: &WorkspaceId) -> Option<&WorkspaceState> {
        self.workspaces
            .iter()
            .find(|workspace| &workspace.id == workspace_id)
    }

    pub fn workspace_by_project_key(&self, key: &ProjectKey) -> Option<&WorkspaceState> {
        self.workspaces.iter().find(|workspace| {
            ProjectKey::from_location(&workspace.project.location)
                .ok()
                .flatten()
                .as_ref()
                == Some(key)
        })
    }

    fn workspace_mut(&mut self, workspace_id: &WorkspaceId) -> Option<&mut WorkspaceState> {
        self.workspaces
            .iter_mut()
            .find(|workspace| &workspace.id == workspace_id)
    }

    fn resolve_workspace_id(&self, workspace_id: Option<WorkspaceId>) -> WorkspaceId {
        workspace_id.unwrap_or_else(|| self.active_workspace_id.clone())
    }

    fn apply_system(&mut self, intent: SystemIntent) -> Result<AppDelta, String> {
        match intent {
            SystemIntent::Identify => Ok(AppDelta::Identified {
                name: "pandamux".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                native: true,
            }),
            SystemIntent::Capabilities => Ok(AppDelta::CapabilitiesReported {
                capabilities: Capabilities {
                    browser: false,
                    layout_grid: true,
                    native: true,
                },
            }),
            SystemIntent::Tree { workspace_id } => {
                let workspace_id = self.resolve_workspace_id(workspace_id);
                let workspace = self
                    .workspace(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                Ok(AppDelta::TreeReported {
                    workspace_id,
                    tree: workspace.split_tree.clone(),
                })
            }
        }
    }

    fn apply_workspace(&mut self, intent: WorkspaceIntent) -> Result<AppDelta, String> {
        match intent {
            WorkspaceIntent::Create { title, shell } => {
                let workspace_id = WorkspaceId::generate();
                let split_tree = create_leaf(None, SurfaceType::Terminal);
                let focused_pane_id = first_pane_id(&split_tree);
                let workspace = WorkspaceState {
                    id: workspace_id.clone(),
                    title: title.unwrap_or_else(|| "Workspace".to_string()),
                    shell: shell.unwrap_or_else(|| "pwsh".to_string()),
                    project: ProjectSpec::default(),
                    split_tree,
                    focused_pane_id,
                    zoomed_pane_id: None,
                };
                let summary = workspace.summary();
                let tree = workspace.split_tree.clone();
                self.workspaces.push(workspace);
                self.active_workspace_id = workspace_id;
                Ok(AppDelta::WorkspaceCreated {
                    workspace: summary,
                    tree,
                })
            }
            WorkspaceIntent::CreateProject {
                workspace_id,
                pane_id,
                surface_id,
                title,
                shell,
                project,
            } => {
                if self.workspace(&workspace_id).is_some() {
                    return Err(format!("workspace already exists: {workspace_id}"));
                }
                if let Some(key) = ProjectKey::from_location(&project.location)?
                    && self.workspace_by_project_key(&key).is_some()
                {
                    return Err(format!("project already exists: {}", key.as_str()));
                }
                let split_tree =
                    create_leaf_with_ids(pane_id.clone(), surface_id, SurfaceType::Terminal);
                let workspace = WorkspaceState {
                    id: workspace_id.clone(),
                    title,
                    shell,
                    project,
                    split_tree,
                    focused_pane_id: Some(pane_id),
                    zoomed_pane_id: None,
                };
                let summary = workspace.summary();
                let tree = workspace.split_tree.clone();
                self.workspaces.push(workspace);
                self.active_workspace_id = workspace_id;
                Ok(AppDelta::WorkspaceCreated {
                    workspace: summary,
                    tree,
                })
            }
            WorkspaceIntent::Select { workspace_id } => {
                if self.workspace(&workspace_id).is_none() {
                    return Err(format!("workspace not found: {workspace_id}"));
                }
                self.active_workspace_id = workspace_id.clone();
                Ok(AppDelta::WorkspaceSelected { workspace_id })
            }
            WorkspaceIntent::Rename {
                workspace_id,
                title,
            } => {
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                workspace.title = title.clone();
                Ok(AppDelta::WorkspaceRenamed {
                    workspace_id,
                    title,
                })
            }
            WorkspaceIntent::Close { workspace_id } => {
                if self.workspaces.len() == 1 {
                    return Err("cannot close the last workspace".to_string());
                }
                let original_len = self.workspaces.len();
                self.workspaces
                    .retain(|workspace| workspace.id != workspace_id);
                if self.workspaces.len() == original_len {
                    return Err(format!("workspace not found: {workspace_id}"));
                }
                if self.active_workspace_id == workspace_id {
                    self.active_workspace_id = self.workspaces[0].id.clone();
                }
                Ok(AppDelta::WorkspaceClosed { workspace_id })
            }
            WorkspaceIntent::List => Ok(AppDelta::WorkspaceListReported {
                workspaces: self
                    .workspaces
                    .iter()
                    .map(WorkspaceState::summary)
                    .collect(),
            }),
        }
    }

    fn apply_pane(&mut self, intent: PaneIntent) -> Result<AppDelta, String> {
        match intent {
            PaneIntent::Split(params) => {
                let workspace_id = self.resolve_workspace_id(params.workspace_id);
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                let target_pane_id = resolve_anchor_pane_id(
                    &workspace.split_tree,
                    params.target_pane_id,
                    params.target_surface_id,
                    workspace.focused_pane_id.clone(),
                )
                .ok_or_else(|| "target pane not found".to_string())?;
                let pane_id = PaneId::generate();
                let tree = split_node(
                    &workspace.split_tree,
                    &target_pane_id,
                    pane_id.clone(),
                    params.surface_type,
                    params.direction,
                );
                let surface_id = find_leaf(&tree, &pane_id)
                    .and_then(|leaf| leaf.surfaces.first())
                    .map(|surface| surface.id.clone())
                    .ok_or_else(|| "split did not create a surface".to_string())?;
                workspace.split_tree = tree.clone();
                workspace.focused_pane_id = Some(pane_id.clone());
                workspace.zoomed_pane_id = None;
                Ok(AppDelta::PaneSplit {
                    workspace_id,
                    pane_id,
                    surface_id,
                    tree,
                })
            }
            PaneIntent::Close {
                workspace_id,
                pane_id,
            } => {
                let workspace_id = self.resolve_workspace_id(workspace_id);
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                let pane_ids = get_all_pane_ids(&workspace.split_tree);
                if pane_ids.len() == 1 {
                    return Err("cannot close the last pane".to_string());
                }
                if !pane_ids.contains(&pane_id) {
                    return Err(format!("pane not found: {pane_id}"));
                }
                let tree = remove_leaf(&workspace.split_tree, &pane_id)
                    .ok_or_else(|| "cannot close the last pane".to_string())?;
                let focused_pane_id = first_pane_id(&tree);
                workspace.split_tree = tree.clone();
                workspace.focused_pane_id = focused_pane_id;
                if workspace.zoomed_pane_id.as_ref() == Some(&pane_id) {
                    workspace.zoomed_pane_id = None;
                }
                Ok(AppDelta::PaneClosed {
                    workspace_id,
                    pane_id,
                    tree,
                })
            }
            PaneIntent::Focus {
                workspace_id,
                pane_id,
            } => {
                let workspace_id = self.resolve_workspace_id(workspace_id);
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                if find_leaf(&workspace.split_tree, &pane_id).is_none() {
                    return Err(format!("pane not found: {pane_id}"));
                }
                workspace.focused_pane_id = Some(pane_id.clone());
                Ok(AppDelta::PaneFocused {
                    workspace_id,
                    pane_id,
                })
            }
            PaneIntent::Zoom {
                workspace_id,
                pane_id,
            } => {
                let workspace_id = self.resolve_workspace_id(workspace_id);
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                let pane_id = pane_id
                    .or_else(|| workspace.focused_pane_id.clone())
                    .ok_or_else(|| "pane not found".to_string())?;
                if find_leaf(&workspace.split_tree, &pane_id).is_none() {
                    return Err(format!("pane not found: {pane_id}"));
                }
                workspace.zoomed_pane_id = if workspace.zoomed_pane_id.as_ref() == Some(&pane_id) {
                    None
                } else {
                    Some(pane_id.clone())
                };
                workspace.focused_pane_id = Some(pane_id);
                Ok(AppDelta::PaneZoomed {
                    workspace_id,
                    pane_id: workspace.zoomed_pane_id.clone(),
                })
            }
            PaneIntent::LayoutGrid(params) => {
                if params.count < 1 {
                    return Err("count must be at least 1".to_string());
                }
                let workspace_id = self.resolve_workspace_id(params.workspace_id);
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                let anchor_pane_id = resolve_anchor_pane_id(
                    &workspace.split_tree,
                    params.anchor_pane_id,
                    params.anchor_surface_id,
                    workspace.focused_pane_id.clone(),
                )
                .ok_or_else(|| "anchor pane not found".to_string())?;
                let GridLayoutResult { tree, new_pane_ids } = build_grid_layout(
                    &workspace.split_tree,
                    &anchor_pane_id,
                    params.count,
                    params.surface_type,
                );
                workspace.focused_pane_id = first_pane_id(&tree);
                workspace.zoomed_pane_id = None;
                workspace.split_tree = tree.clone();
                Ok(AppDelta::LayoutGridApplied {
                    workspace_id,
                    tree,
                    new_pane_ids,
                })
            }
            PaneIntent::List { workspace_id } => {
                let workspace_id = self.resolve_workspace_id(workspace_id);
                let workspace = self
                    .workspace(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                Ok(AppDelta::PaneListReported {
                    workspace_id,
                    panes: pane_summaries(&workspace.split_tree),
                })
            }
        }
    }

    fn apply_surface(&mut self, intent: SurfaceIntent) -> Result<AppDelta, String> {
        match intent {
            SurfaceIntent::Create {
                workspace_id,
                pane_id,
                surface_type,
            } => {
                let workspace_id = self.resolve_workspace_id(workspace_id);
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                let pane_id = pane_id
                    .or_else(|| workspace.focused_pane_id.clone())
                    .or_else(|| first_pane_id(&workspace.split_tree))
                    .ok_or_else(|| "pane not found".to_string())?;
                let leaf = find_leaf(&workspace.split_tree, &pane_id)
                    .ok_or_else(|| format!("pane not found: {pane_id}"))?;
                let mut leaf = leaf.clone();
                let surface = SurfaceRef {
                    id: SurfaceId::generate(),
                    surface_type,
                };
                leaf.surfaces.push(surface.clone());
                leaf.active_surface_index = leaf.surfaces.len() - 1;
                workspace.split_tree = replace_leaf(&workspace.split_tree, &pane_id, leaf);
                workspace.focused_pane_id = Some(pane_id.clone());
                Ok(AppDelta::SurfaceCreated {
                    workspace_id,
                    pane_id,
                    surface,
                })
            }
            SurfaceIntent::CreateWithId {
                workspace_id,
                pane_id,
                surface_id,
                surface_type,
            } => {
                if self.workspaces.iter().any(|workspace| {
                    find_pane_id_for_surface(&workspace.split_tree, &surface_id).is_some()
                }) {
                    return Err(format!("surface already exists: {surface_id}"));
                }
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                let pane_id = pane_id
                    .or_else(|| workspace.focused_pane_id.clone())
                    .or_else(|| first_pane_id(&workspace.split_tree))
                    .ok_or_else(|| "pane not found".to_string())?;
                let leaf = find_leaf(&workspace.split_tree, &pane_id)
                    .ok_or_else(|| format!("pane not found: {pane_id}"))?;
                let mut leaf = leaf.clone();
                let surface = SurfaceRef {
                    id: surface_id,
                    surface_type,
                };
                leaf.surfaces.push(surface.clone());
                leaf.active_surface_index = leaf.surfaces.len() - 1;
                workspace.split_tree = replace_leaf(&workspace.split_tree, &pane_id, leaf);
                workspace.focused_pane_id = Some(pane_id.clone());
                Ok(AppDelta::SurfaceCreated {
                    workspace_id,
                    pane_id,
                    surface,
                })
            }
            SurfaceIntent::Focus {
                workspace_id,
                surface_id,
            } => {
                let workspace_id = self.resolve_workspace_id(workspace_id);
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                let pane_id = find_pane_id_for_surface(&workspace.split_tree, &surface_id)
                    .ok_or_else(|| format!("surface not found: {surface_id}"))?;
                let leaf = find_leaf(&workspace.split_tree, &pane_id)
                    .ok_or_else(|| format!("pane not found: {pane_id}"))?;
                let active_surface_index = leaf
                    .surfaces
                    .iter()
                    .position(|surface| surface.id == surface_id)
                    .ok_or_else(|| format!("surface not found: {surface_id}"))?;
                let mut leaf = leaf.clone();
                leaf.active_surface_index = active_surface_index;
                workspace.split_tree = replace_leaf(&workspace.split_tree, &pane_id, leaf);
                workspace.focused_pane_id = Some(pane_id);
                Ok(AppDelta::SurfaceFocused {
                    workspace_id,
                    surface_id,
                })
            }
            SurfaceIntent::Close {
                workspace_id,
                surface_id,
            } => {
                let workspace_id = self.resolve_workspace_id(workspace_id);
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                let pane_id = find_pane_id_for_surface(&workspace.split_tree, &surface_id)
                    .ok_or_else(|| format!("surface not found: {surface_id}"))?;
                let leaf = find_leaf(&workspace.split_tree, &pane_id)
                    .ok_or_else(|| format!("pane not found: {pane_id}"))?;
                if leaf.surfaces.len() == 1 {
                    return Err("cannot close the last surface in a pane".to_string());
                }
                let mut leaf = leaf.clone();
                let removed_index = leaf
                    .surfaces
                    .iter()
                    .position(|surface| surface.id == surface_id)
                    .ok_or_else(|| format!("surface not found: {surface_id}"))?;
                leaf.surfaces.remove(removed_index);
                leaf.active_surface_index = leaf.active_surface_index.min(leaf.surfaces.len() - 1);
                workspace.split_tree = replace_leaf(&workspace.split_tree, &pane_id, leaf);
                workspace.focused_pane_id = Some(pane_id);
                Ok(AppDelta::SurfaceClosed {
                    workspace_id,
                    surface_id,
                })
            }
            SurfaceIntent::Move {
                workspace_id,
                surface_id,
                target_pane_id,
                zone,
            } => {
                let workspace_id = self.resolve_workspace_id(workspace_id);
                let workspace = self
                    .workspace_mut(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                match move_surface(&workspace.split_tree, &surface_id, &target_pane_id, zone) {
                    Some(MoveResult {
                        tree,
                        focus_pane_id,
                    }) => {
                        workspace.split_tree = tree.clone();
                        workspace.focused_pane_id = Some(focus_pane_id);
                        workspace.zoomed_pane_id = None;
                        Ok(AppDelta::SurfaceMoved { workspace_id, tree })
                    }
                    // No-op drop (e.g. a pane's only tab onto itself): report the
                    // unchanged tree so the UI simply repaints.
                    None => Ok(AppDelta::SurfaceMoved {
                        workspace_id,
                        tree: workspace.split_tree.clone(),
                    }),
                }
            }
            SurfaceIntent::List {
                workspace_id,
                pane_id,
            } => {
                let workspace_id = self.resolve_workspace_id(workspace_id);
                let workspace = self
                    .workspace(&workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                let surfaces = if let Some(pane_id) = pane_id {
                    find_leaf(&workspace.split_tree, &pane_id)
                        .map(|leaf| leaf.surfaces.clone())
                        .unwrap_or_default()
                } else {
                    pane_summaries(&workspace.split_tree)
                        .into_iter()
                        .flat_map(|pane| pane.surfaces)
                        .collect()
                };
                Ok(AppDelta::SurfaceListReported {
                    workspace_id,
                    surfaces,
                })
            }
        }
    }
}

impl WorkspaceState {
    fn summary(&self) -> WorkspaceSummary {
        WorkspaceSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            shell: self.shell.clone(),
            project: self.project.clone(),
        }
    }
}

fn pane_summaries(tree: &SplitNode) -> Vec<PaneSummary> {
    get_all_pane_ids(tree)
        .into_iter()
        .filter_map(|pane_id| {
            let leaf = find_leaf(tree, &pane_id)?;
            Some(PaneSummary {
                id: pane_id,
                surfaces: leaf.surfaces.clone(),
                active_surface_id: leaf
                    .surfaces
                    .get(leaf.active_surface_index)
                    .map(|surface| surface.id.clone()),
            })
        })
        .collect()
}

fn resolve_anchor_pane_id(
    tree: &SplitNode,
    anchor_pane_id: Option<PaneId>,
    anchor_surface_id: Option<SurfaceId>,
    fallback_pane_id: Option<PaneId>,
) -> Option<PaneId> {
    if let Some(anchor_pane_id) = anchor_pane_id {
        return find_leaf(tree, &anchor_pane_id).map(|_| anchor_pane_id);
    }

    if let Some(anchor_surface_id) = anchor_surface_id {
        for pane_id in get_all_pane_ids(tree) {
            let Some(leaf) = find_leaf(tree, &pane_id) else {
                continue;
            };
            if leaf
                .surfaces
                .iter()
                .any(|surface| surface.id == anchor_surface_id)
            {
                return Some(pane_id);
            }
        }
        return None;
    }

    if let Some(fallback_pane_id) = fallback_pane_id
        && find_leaf(tree, &fallback_pane_id).is_some()
    {
        return Some(fallback_pane_id);
    }

    get_all_pane_ids(tree).into_iter().next()
}

fn first_pane_id(tree: &SplitNode) -> Option<PaneId> {
    get_all_pane_ids(tree).into_iter().next()
}
