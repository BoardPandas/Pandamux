use crate::ids::{PaneId, ProjectId, SurfaceId, WorkspaceId};
use crate::project::{ProjectKey, ProjectSpec};
use crate::project_registry::{ProjectMatcher, ProjectRecord};
use crate::split_tree::{
    DropZone, GridLayoutResult, MoveResult, SessionType, SplitDirection, SplitNode, SurfaceRef,
    SurfaceType, build_grid_layout, create_leaf, create_leaf_with_ids, find_leaf,
    find_pane_id_for_surface, get_all_pane_ids, move_surface, remove_leaf, replace_leaf,
    split_node,
};
use serde::{Deserialize, Serialize};

/// Bumped when session.json needs an explicit migration (additive fields rely
/// on serde defaults instead and do not bump this).
pub const APP_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    #[serde(default)]
    pub schema_version: u32,
    pub workspaces: Vec<WorkspaceState>,
    /// `None` when every workspace has been closed (the empty state, spec 1.5).
    /// Old session files with a bare id string deserialize as `Some`.
    pub active_workspace_id: Option<WorkspaceId>,
    /// The project registry (spec 1.4): stable identity above ProjectKey.
    #[serde(default)]
    pub projects: Vec<ProjectRecord>,
    /// The Home dashboard layout (spec 2.5), persisted with the session.
    #[serde(default)]
    pub home: crate::home::HomeLayout,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub id: WorkspaceId,
    pub title: String,
    pub shell: String,
    #[serde(default)]
    pub project: ProjectSpec,
    /// Which [`ProjectRecord`] this workspace belongs to. `None` for legacy
    /// workspaces (assigned by `ensure_project_registry` on load otherwise).
    #[serde(default)]
    pub project_id: Option<ProjectId>,
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
    Project(ProjectIntent),
    Home(HomeIntent),
}

/// Home dashboard mutations (spec 2.5). Panes reference live sessions by id;
/// none of these touch the sessions themselves.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HomeIntent {
    Pin {
        surface_id: SurfaceId,
        #[serde(default)]
        pinned: Option<crate::project_registry::LaunchConfig>,
    },
    Assign {
        home_pane_id: PaneId,
        surface_id: SurfaceId,
        #[serde(default)]
        pinned: Option<crate::project_registry::LaunchConfig>,
    },
    Unpin {
        home_pane_id: PaneId,
    },
    MoveBy {
        home_pane_id: PaneId,
        delta: i32,
    },
    Focus {
        home_pane_id: PaneId,
    },
}

/// Project-registry mutations (spec 1.4). Rename/Merge/Split mark records
/// `manual` so heuristics never override a human decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProjectIntent {
    List,
    Rename {
        project_id: ProjectId,
        name: String,
    },
    /// Fold `source` into `target`: workspaces repoint, matchers and known
    /// locations move over, the source record disappears.
    Merge {
        source: ProjectId,
        target: ProjectId,
    },
    /// Detach one workspace into a fresh record (undo for a wrong merge).
    Split {
        workspace_id: WorkspaceId,
    },
    /// Attach an async-discovered matcher (a git remote hint). May auto-merge
    /// this record into an existing owner of the same matcher when neither
    /// record is manual.
    AttachMatcher {
        project_id: ProjectId,
        matcher: ProjectMatcher,
    },
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
    /// Close every workspace, or every workspace of one project (spec 1.5).
    CloseAll {
        #[serde(default)]
        project_id: Option<ProjectId>,
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
    /// User rename of a session entry (spec 2.1). `None` clears back to the
    /// derived name. Searches every workspace when `workspace_id` is `None`.
    Rename {
        workspace_id: Option<WorkspaceId>,
        surface_id: SurfaceId,
        name: Option<String>,
    },
    /// Record what runs in a surface (spec 2.2/2.7 session types).
    SetSessionType {
        workspace_id: Option<WorkspaceId>,
        surface_id: SurfaceId,
        session: SessionType,
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
    WorkspacesClosed {
        workspace_ids: Vec<WorkspaceId>,
    },
    WorkspaceListReported {
        workspaces: Vec<WorkspaceSummary>,
    },
    ProjectListReported {
        projects: Vec<ProjectRecord>,
    },
    ProjectRenamed {
        project_id: ProjectId,
        name: String,
    },
    ProjectsMerged {
        source: ProjectId,
        target: ProjectId,
    },
    ProjectSplit {
        workspace_id: WorkspaceId,
        project_id: ProjectId,
    },
    MatcherAttached {
        project_id: ProjectId,
    },
    HomeChanged {
        home: crate::home::HomeLayout,
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
    SurfaceRenamed {
        workspace_id: WorkspaceId,
        surface_id: SurfaceId,
        name: Option<String>,
    },
    SurfaceSessionTypeSet {
        workspace_id: WorkspaceId,
        surface_id: SurfaceId,
        session: SessionType,
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
            schema_version: APP_STATE_SCHEMA_VERSION,
            workspaces: vec![WorkspaceState {
                id: workspace_id.clone(),
                title: "Workspace".to_string(),
                shell: "pwsh".to_string(),
                project: ProjectSpec::default(),
                project_id: None,
                split_tree: create_leaf(Some(PaneId::from("pane-default")), SurfaceType::Terminal),
                focused_pane_id: Some(PaneId::from("pane-default")),
                zoomed_pane_id: None,
            }],
            active_workspace_id: Some(workspace_id),
            projects: Vec::new(),
            home: crate::home::HomeLayout::default(),
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
            AppIntent::Project(intent) => self.apply_project(intent),
            AppIntent::Home(intent) => self.apply_home(intent),
        }
    }

    fn apply_home(&mut self, intent: HomeIntent) -> Result<AppDelta, String> {
        let applied = match intent {
            HomeIntent::Pin { surface_id, pinned } => {
                if self.workspace_id_for_surface(&surface_id).is_none() {
                    return Err(format!("surface not found: {surface_id}"));
                }
                self.home.pin(surface_id, pinned);
                true
            }
            HomeIntent::Assign {
                home_pane_id,
                surface_id,
                pinned,
            } => {
                if self.workspace_id_for_surface(&surface_id).is_none() {
                    return Err(format!("surface not found: {surface_id}"));
                }
                self.home.assign(&home_pane_id, surface_id, pinned)
            }
            HomeIntent::Unpin { home_pane_id } => self.home.unpin(&home_pane_id),
            HomeIntent::MoveBy {
                home_pane_id,
                delta,
            } => {
                self.home.move_by(&home_pane_id, delta);
                true
            }
            HomeIntent::Focus { home_pane_id } => self.home.focus(&home_pane_id),
        };
        if !applied {
            return Err("home pane not found".to_string());
        }
        // Surfaces may have closed since the layout was saved: release them so
        // their panes render as relaunch placeholders.
        let workspaces = self.workspaces.clone();
        self.home.release_dead_surfaces(&|surface_id| {
            workspaces.iter().any(|workspace| {
                find_pane_id_for_surface(&workspace.split_tree, surface_id).is_some()
            })
        });
        Ok(AppDelta::HomeChanged {
            home: self.home.clone(),
        })
    }

    pub fn active_workspace(&self) -> Option<&WorkspaceState> {
        self.active_workspace_id
            .as_ref()
            .and_then(|workspace_id| self.workspace(workspace_id))
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

    /// The workspace an intent targets: an explicit id, else the active
    /// workspace. Errors cleanly on the empty state so pipe clients never see
    /// a panic when everything is closed.
    fn resolve_workspace_id(
        &self,
        workspace_id: Option<WorkspaceId>,
    ) -> Result<WorkspaceId, String> {
        workspace_id
            .or_else(|| self.active_workspace_id.clone())
            .ok_or_else(|| "no workspace is open".to_string())
    }

    /// Find which workspace owns a surface (rename/session-type intents from
    /// the rail may target a non-active workspace).
    fn workspace_id_for_surface(&self, surface_id: &SurfaceId) -> Option<WorkspaceId> {
        self.workspaces
            .iter()
            .find(|workspace| find_pane_id_for_surface(&workspace.split_tree, surface_id).is_some())
            .map(|workspace| workspace.id.clone())
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
                let workspace_id = self.resolve_workspace_id(workspace_id)?;
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
                    project_id: None,
                    split_tree,
                    focused_pane_id,
                    zoomed_pane_id: None,
                };
                let summary = workspace.summary();
                let tree = workspace.split_tree.clone();
                self.workspaces.push(workspace);
                self.active_workspace_id = Some(workspace_id);
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
                    project_id: None,
                    split_tree,
                    focused_pane_id: Some(pane_id),
                    zoomed_pane_id: None,
                };
                let summary = workspace.summary();
                let tree = workspace.split_tree.clone();
                self.workspaces.push(workspace);
                self.active_workspace_id = Some(workspace_id);
                Ok(AppDelta::WorkspaceCreated {
                    workspace: summary,
                    tree,
                })
            }
            WorkspaceIntent::Select { workspace_id } => {
                if self.workspace(&workspace_id).is_none() {
                    return Err(format!("workspace not found: {workspace_id}"));
                }
                self.active_workspace_id = Some(workspace_id.clone());
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
                // Closing the last workspace is allowed: the app lands on the
                // empty state ("All sessions ended", spec 1.5).
                let original_len = self.workspaces.len();
                self.workspaces
                    .retain(|workspace| workspace.id != workspace_id);
                if self.workspaces.len() == original_len {
                    return Err(format!("workspace not found: {workspace_id}"));
                }
                if self.active_workspace_id.as_ref() == Some(&workspace_id) {
                    self.active_workspace_id = self
                        .workspaces
                        .first()
                        .map(|workspace| workspace.id.clone());
                }
                self.prune_project_records();
                Ok(AppDelta::WorkspaceClosed { workspace_id })
            }
            WorkspaceIntent::CloseAll { project_id } => {
                let closing: Vec<WorkspaceId> = self
                    .workspaces
                    .iter()
                    .filter(|workspace| {
                        project_id
                            .as_ref()
                            .is_none_or(|id| workspace.project_id.as_ref() == Some(id))
                    })
                    .map(|workspace| workspace.id.clone())
                    .collect();
                self.workspaces
                    .retain(|workspace| !closing.contains(&workspace.id));
                if self
                    .active_workspace_id
                    .as_ref()
                    .is_some_and(|active| closing.contains(active))
                {
                    self.active_workspace_id = self
                        .workspaces
                        .first()
                        .map(|workspace| workspace.id.clone());
                }
                self.prune_project_records();
                Ok(AppDelta::WorkspacesClosed {
                    workspace_ids: closing,
                })
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
                let workspace_id = self.resolve_workspace_id(params.workspace_id)?;
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
                let workspace_id = self.resolve_workspace_id(workspace_id)?;
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
                let workspace_id = self.resolve_workspace_id(workspace_id)?;
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
                let workspace_id = self.resolve_workspace_id(workspace_id)?;
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
                let workspace_id = self.resolve_workspace_id(params.workspace_id)?;
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
                let workspace_id = self.resolve_workspace_id(workspace_id)?;
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
                let workspace_id = self.resolve_workspace_id(workspace_id)?;
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
                let surface = SurfaceRef::new(SurfaceId::generate(), surface_type);
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
                let surface = SurfaceRef::new(surface_id, surface_type);
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
                let workspace_id = self.resolve_workspace_id(workspace_id)?;
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
                let workspace_id = self.resolve_workspace_id(workspace_id)?;
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
                let workspace_id = self.resolve_workspace_id(workspace_id)?;
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
            SurfaceIntent::Rename {
                workspace_id,
                surface_id,
                name,
            } => {
                let workspace_id = match workspace_id {
                    Some(workspace_id) => workspace_id,
                    None => self
                        .workspace_id_for_surface(&surface_id)
                        .ok_or_else(|| format!("surface not found: {surface_id}"))?,
                };
                let name = name.filter(|name| !name.trim().is_empty());
                self.update_surface(&workspace_id, &surface_id, |surface| {
                    surface.name = name.clone();
                })?;
                Ok(AppDelta::SurfaceRenamed {
                    workspace_id,
                    surface_id,
                    name,
                })
            }
            SurfaceIntent::SetSessionType {
                workspace_id,
                surface_id,
                session,
            } => {
                let workspace_id = match workspace_id {
                    Some(workspace_id) => workspace_id,
                    None => self
                        .workspace_id_for_surface(&surface_id)
                        .ok_or_else(|| format!("surface not found: {surface_id}"))?,
                };
                self.update_surface(&workspace_id, &surface_id, |surface| {
                    surface.session = if session == SessionType::Terminal {
                        None
                    } else {
                        Some(session.clone())
                    };
                })?;
                Ok(AppDelta::SurfaceSessionTypeSet {
                    workspace_id,
                    surface_id,
                    session,
                })
            }
            SurfaceIntent::List {
                workspace_id,
                pane_id,
            } => {
                let workspace_id = self.resolve_workspace_id(workspace_id)?;
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

impl AppState {
    /// Mutate one surface in place (rename / session type), rebuilding the
    /// immutable leaf like the other surface intents do.
    fn update_surface(
        &mut self,
        workspace_id: &WorkspaceId,
        surface_id: &SurfaceId,
        mutate: impl Fn(&mut SurfaceRef),
    ) -> Result<(), String> {
        let workspace = self
            .workspace_mut(workspace_id)
            .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
        let pane_id = find_pane_id_for_surface(&workspace.split_tree, surface_id)
            .ok_or_else(|| format!("surface not found: {surface_id}"))?;
        let leaf = find_leaf(&workspace.split_tree, &pane_id)
            .ok_or_else(|| format!("pane not found: {pane_id}"))?;
        let mut leaf = leaf.clone();
        let surface = leaf
            .surfaces
            .iter_mut()
            .find(|surface| &surface.id == surface_id)
            .ok_or_else(|| format!("surface not found: {surface_id}"))?;
        mutate(surface);
        workspace.split_tree = replace_leaf(&workspace.split_tree, &pane_id, leaf);
        Ok(())
    }

    /// Drop machine-created registry records no workspace references anymore.
    fn prune_project_records(&mut self) {
        let referenced: Vec<ProjectId> = self
            .workspaces
            .iter()
            .filter_map(|workspace| workspace.project_id.clone())
            .collect();
        self.projects
            .retain(|record| record.manual || referenced.contains(&record.id));
    }

    fn apply_project(&mut self, intent: ProjectIntent) -> Result<AppDelta, String> {
        match intent {
            ProjectIntent::List => Ok(AppDelta::ProjectListReported {
                projects: self.projects.clone(),
            }),
            ProjectIntent::Rename { project_id, name } => {
                let record = self
                    .projects
                    .iter_mut()
                    .find(|record| record.id == project_id)
                    .ok_or_else(|| format!("project not found: {project_id}"))?;
                record.name = name.clone();
                record.manual = true;
                Ok(AppDelta::ProjectRenamed { project_id, name })
            }
            ProjectIntent::Merge { source, target } => {
                if source == target {
                    return Err("cannot merge a project into itself".to_string());
                }
                if !self.projects.iter().any(|record| record.id == target) {
                    return Err(format!("project not found: {target}"));
                }
                let source_index = self
                    .projects
                    .iter()
                    .position(|record| record.id == source)
                    .ok_or_else(|| format!("project not found: {source}"))?;
                let source_record = self.projects.remove(source_index);
                let target_record = self
                    .projects
                    .iter_mut()
                    .find(|record| record.id == target)
                    .expect("target checked above");
                for matcher in source_record.matchers {
                    if !target_record.matchers.contains(&matcher) {
                        target_record.matchers.push(matcher);
                    }
                }
                for location in source_record.known_locations {
                    if !target_record.known_locations.contains(&location) {
                        target_record.known_locations.push(location);
                    }
                }
                target_record.manual = true;
                for workspace in &mut self.workspaces {
                    if workspace.project_id.as_ref() == Some(&source) {
                        workspace.project_id = Some(target.clone());
                    }
                }
                Ok(AppDelta::ProjectsMerged { source, target })
            }
            ProjectIntent::Split { workspace_id } => {
                let workspace = self
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == workspace_id)
                    .ok_or_else(|| format!("workspace not found: {workspace_id}"))?;
                let location = workspace.project.location.clone();
                let old_project_id = workspace.project_id.clone();
                let mut matchers = Vec::new();
                if let Ok(Some(key)) = ProjectKey::from_location(&location) {
                    matchers.push(ProjectMatcher::Location {
                        key: key.as_str().to_string(),
                    });
                }
                let record = ProjectRecord {
                    id: crate::ids::ProjectId::generate(),
                    name: crate::project::project_title(&location),
                    matchers,
                    known_locations: vec![location],
                    created_at_ms: 0,
                    manual: true,
                };
                let project_id = record.id.clone();
                // The record left behind was also human-shaped now: stop
                // heuristics from re-merging what the user just separated.
                if let Some(old_id) = old_project_id
                    && let Some(old) = self.projects.iter_mut().find(|record| record.id == old_id)
                {
                    old.manual = true;
                }
                self.projects.push(record);
                if let Some(workspace) = self.workspace_mut(&workspace_id) {
                    workspace.project_id = Some(project_id.clone());
                }
                self.prune_project_records();
                Ok(AppDelta::ProjectSplit {
                    workspace_id,
                    project_id,
                })
            }
            ProjectIntent::AttachMatcher {
                project_id,
                matcher,
            } => {
                let this_manual = self
                    .projects
                    .iter()
                    .find(|record| record.id == project_id)
                    .ok_or_else(|| format!("project not found: {project_id}"))?
                    .manual;
                // An existing machine-created owner of the same matcher absorbs
                // this record (the late git-remote hint proved they are one
                // project), unless a human shaped either record.
                let owner = self
                    .projects
                    .iter()
                    .find(|record| record.id != project_id && record.matchers.contains(&matcher))
                    .map(|record| (record.id.clone(), record.manual));
                if let Some((owner_id, owner_manual)) = owner
                    && !this_manual
                    && !owner_manual
                {
                    return self.apply_project(ProjectIntent::Merge {
                        source: project_id,
                        target: owner_id,
                    });
                }
                let record = self
                    .projects
                    .iter_mut()
                    .find(|record| record.id == project_id)
                    .expect("checked above");
                if !record.matchers.contains(&matcher) {
                    record.matchers.push(matcher);
                }
                Ok(AppDelta::MatcherAttached { project_id })
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
