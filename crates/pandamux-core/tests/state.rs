use pandamux_core::{
    AppDelta, AppIntent, AppState, LayoutGridParams, PaneId, PaneIntent, SplitDirection,
    SplitPaneParams, SurfaceIntent, SurfaceType, SystemIntent,
};

#[test]
fn legacy_workspace_json_defaults_to_legacy_project() {
    let value = serde_json::to_value(AppState::default()).unwrap();
    let mut value = value;
    value["workspaces"][0]
        .as_object_mut()
        .unwrap()
        .remove("project");
    let loaded: AppState = serde_json::from_value(value).unwrap();
    assert!(matches!(
        loaded.workspaces[0].project.location,
        pandamux_core::ProjectLocation::Legacy
    ));
}

#[test]
fn default_state_reports_a_tree() {
    let mut state = AppState::default();
    let delta = state
        .apply(AppIntent::System(SystemIntent::Tree { workspace_id: None }))
        .expect("tree should be reported");

    let AppDelta::TreeReported { workspace_id, tree } = delta else {
        panic!("expected tree delta");
    };

    assert_eq!(workspace_id.as_str(), "ws-default");
    assert_eq!(pandamux_core::get_all_pane_ids(&tree).len(), 1);
}

#[test]
fn layout_grid_mutates_the_authoritative_state() {
    let mut state = AppState::default();
    let delta = state
        .apply(AppIntent::Pane(PaneIntent::LayoutGrid(LayoutGridParams {
            workspace_id: None,
            anchor_pane_id: None,
            anchor_surface_id: None,
            count: 4,
            surface_type: SurfaceType::Terminal,
        })))
        .expect("grid should apply");

    let AppDelta::LayoutGridApplied {
        tree, new_pane_ids, ..
    } = delta
    else {
        panic!("expected layout grid delta");
    };

    assert_eq!(new_pane_ids.len(), 3);
    assert_eq!(pandamux_core::get_all_pane_ids(&tree).len(), 4);
    assert_eq!(
        pandamux_core::get_all_pane_ids(&state.active_workspace().unwrap().split_tree).len(),
        4
    );
}

#[test]
fn pane_split_focus_and_close_mutate_authoritative_state() {
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

    let AppDelta::PaneSplit { pane_id, .. } = split else {
        panic!("expected pane split");
    };
    assert_eq!(
        state.active_workspace().unwrap().focused_pane_id.as_ref(),
        Some(&pane_id)
    );

    state
        .apply(AppIntent::Pane(PaneIntent::Focus {
            workspace_id: None,
            pane_id: PaneId::from("pane-default"),
        }))
        .expect("pane focus should apply");
    assert_eq!(
        state.active_workspace().unwrap().focused_pane_id.as_ref(),
        Some(&PaneId::from("pane-default"))
    );

    state
        .apply(AppIntent::Pane(PaneIntent::Close {
            workspace_id: None,
            pane_id,
        }))
        .expect("pane close should apply");
    assert_eq!(
        pandamux_core::get_all_pane_ids(&state.active_workspace().unwrap().split_tree),
        vec![PaneId::from("pane-default")]
    );
}

#[test]
fn pane_zoom_toggles_the_authoritative_state() {
    let mut state = AppState::default();
    let first = state
        .apply(AppIntent::Pane(PaneIntent::Zoom {
            workspace_id: None,
            pane_id: Some(PaneId::from("pane-default")),
        }))
        .expect("zoom should apply");

    let AppDelta::PaneZoomed { pane_id, .. } = first else {
        panic!("expected pane zoomed");
    };
    assert_eq!(pane_id, Some(PaneId::from("pane-default")));
    assert_eq!(
        state.active_workspace().unwrap().zoomed_pane_id.as_ref(),
        Some(&PaneId::from("pane-default"))
    );

    let second = state
        .apply(AppIntent::Pane(PaneIntent::Zoom {
            workspace_id: None,
            pane_id: Some(PaneId::from("pane-default")),
        }))
        .expect("zoom should toggle off");

    let AppDelta::PaneZoomed { pane_id, .. } = second else {
        panic!("expected pane zoomed");
    };
    assert_eq!(pane_id, None);
    assert_eq!(state.active_workspace().unwrap().zoomed_pane_id, None);
}

#[test]
fn surface_create_focus_and_close_mutate_authoritative_state() {
    let mut state = AppState::default();
    let create = state
        .apply(AppIntent::Surface(SurfaceIntent::Create {
            workspace_id: None,
            pane_id: Some(PaneId::from("pane-default")),
            surface_type: SurfaceType::Markdown,
        }))
        .expect("surface should create");

    let AppDelta::SurfaceCreated { surface, .. } = create else {
        panic!("expected surface created");
    };
    let active = pandamux_core::find_leaf(
        &state.active_workspace().unwrap().split_tree,
        &PaneId::from("pane-default"),
    )
    .expect("pane exists")
    .surfaces[1]
        .id
        .clone();
    assert_eq!(active, surface.id);

    let terminal_surface_id = pandamux_core::find_leaf(
        &state.active_workspace().unwrap().split_tree,
        &PaneId::from("pane-default"),
    )
    .expect("pane exists")
    .surfaces[0]
        .id
        .clone();
    state
        .apply(AppIntent::Surface(SurfaceIntent::Focus {
            workspace_id: None,
            surface_id: terminal_surface_id,
        }))
        .expect("surface focus should apply");

    state
        .apply(AppIntent::Surface(SurfaceIntent::Close {
            workspace_id: None,
            surface_id: surface.id,
        }))
        .expect("surface close should apply");
    let leaf = pandamux_core::find_leaf(
        &state.active_workspace().unwrap().split_tree,
        &PaneId::from("pane-default"),
    )
    .expect("pane exists");
    assert_eq!(leaf.surfaces.len(), 1);
    assert_eq!(leaf.active_surface_index, 0);
}

// ---------------------------------------------------------------------------
// July 2026 overhaul: project registry, guard relaxation, rename, close-all
// ---------------------------------------------------------------------------

use pandamux_core::{
    ProjectIntent, ProjectLocation, SessionType, WorkspaceIntent, ensure_project_registry,
};

fn project_workspace(app: &mut AppState, title: &str, cwd: &str) -> pandamux_core::WorkspaceId {
    let workspace_id = pandamux_core::WorkspaceId::generate();
    app.apply(AppIntent::Workspace(WorkspaceIntent::CreateProject {
        workspace_id: workspace_id.clone(),
        pane_id: PaneId::generate(),
        surface_id: pandamux_core::SurfaceId::generate(),
        title: title.to_string(),
        shell: "pwsh".to_string(),
        project: pandamux_core::ProjectSpec {
            location: ProjectLocation::Local {
                cwd: cwd.to_string(),
                shell: "pwsh".to_string(),
            },
        },
    }))
    .expect("create project workspace");
    workspace_id
}

fn ssh_project_workspace(
    app: &mut AppState,
    title: &str,
    profile: &str,
    remote_cwd: &str,
) -> pandamux_core::WorkspaceId {
    let workspace_id = pandamux_core::WorkspaceId::generate();
    app.apply(AppIntent::Workspace(WorkspaceIntent::CreateProject {
        workspace_id: workspace_id.clone(),
        pane_id: PaneId::generate(),
        surface_id: pandamux_core::SurfaceId::generate(),
        title: title.to_string(),
        shell: "ssh".to_string(),
        project: pandamux_core::ProjectSpec {
            location: ProjectLocation::Ssh {
                profile_id: pandamux_core::SshProfileId::from(profile),
                remote_cwd: remote_cwd.to_string(),
            },
        },
    }))
    .expect("create ssh project workspace");
    workspace_id
}

#[test]
fn legacy_app_state_json_defaults_new_fields() {
    // A pre-registry session file: no schemaVersion, no projects, no
    // projectId, a bare active id string, and surfaces without session/name.
    let raw = r#"{
        "workspaces": [{
            "id": "ws-old",
            "title": "Old",
            "shell": "pwsh",
            "splitTree": {
                "type": "leaf",
                "paneId": "pane-old",
                "surfaces": [{"id": "surf-old", "type": "terminal"}],
                "activeSurfaceIndex": 0
            },
            "focusedPaneId": "pane-old",
            "zoomedPaneId": null
        }],
        "activeWorkspaceId": "ws-old"
    }"#;
    let loaded: AppState = serde_json::from_str(raw).expect("legacy file loads");
    assert_eq!(loaded.schema_version, 0);
    assert!(loaded.projects.is_empty());
    assert_eq!(
        loaded.active_workspace_id,
        Some(pandamux_core::WorkspaceId::from("ws-old"))
    );
    assert!(loaded.workspaces[0].project_id.is_none());
    let leaf =
        pandamux_core::find_leaf(&loaded.workspaces[0].split_tree, &PaneId::from("pane-old"))
            .expect("leaf");
    assert!(leaf.surfaces[0].session.is_none());
    assert!(leaf.surfaces[0].name.is_none());
}

#[test]
fn ensure_registry_collapses_per_host_duplicates() {
    let mut app = AppState::default();
    app.workspaces.clear();
    app.active_workspace_id = None;
    project_workspace(&mut app, "SupportForge", "C:\\Dev\\SupportForge");
    ssh_project_workspace(
        &mut app,
        "SupportForge",
        "ssh-pendragon",
        "/home/x/supportforge",
    );
    ssh_project_workspace(&mut app, "Vigilist", "ssh-pendragon", "/home/x/vigilist");

    assert!(ensure_project_registry(&mut app, 1234));

    // The two SupportForge checkouts share one record; Vigilist has its own.
    let forge_id = app.workspaces[0].project_id.clone().expect("assigned");
    assert_eq!(app.workspaces[1].project_id, Some(forge_id.clone()));
    let vigilist_id = app.workspaces[2].project_id.clone().expect("assigned");
    assert_ne!(forge_id, vigilist_id);
    assert_eq!(app.projects.len(), 2);
    // Idempotent on a second run.
    assert!(!ensure_project_registry(&mut app, 5678));
}

#[test]
fn last_workspace_now_closes_into_the_empty_state() {
    let mut app = AppState::default();
    let workspace_id = app.active_workspace_id.clone().expect("default");
    app.apply(AppIntent::Workspace(WorkspaceIntent::Close {
        workspace_id,
    }))
    .expect("closing the last workspace is allowed now");
    assert!(app.workspaces.is_empty());
    assert_eq!(app.active_workspace_id, None);
    // Intents that need a workspace error cleanly on the empty state.
    let error = app
        .apply(AppIntent::Pane(PaneIntent::List { workspace_id: None }))
        .expect_err("no workspace to list");
    assert!(error.contains("no workspace is open"), "error = {error}");
    // Pane and surface guards still hold inside a workspace.
    let mut populated = AppState::default();
    let error = populated
        .apply(AppIntent::Pane(PaneIntent::Close {
            workspace_id: None,
            pane_id: PaneId::from("pane-default"),
        }))
        .expect_err("last pane stays guarded");
    assert!(error.contains("last pane"));
}

#[test]
fn close_all_supports_global_and_per_project() {
    let mut app = AppState::default();
    app.workspaces.clear();
    app.active_workspace_id = None;
    project_workspace(&mut app, "A", "C:\\Dev\\Alpha");
    ssh_project_workspace(&mut app, "A remote", "ssh-x", "/srv/alpha");
    project_workspace(&mut app, "B", "C:\\Dev\\Beta");
    ensure_project_registry(&mut app, 1);
    let alpha_id = app.workspaces[0].project_id.clone().expect("alpha id");

    let delta = app
        .apply(AppIntent::Workspace(WorkspaceIntent::CloseAll {
            project_id: Some(alpha_id),
        }))
        .expect("close alpha");
    let AppDelta::WorkspacesClosed { workspace_ids } = delta else {
        panic!("expected WorkspacesClosed");
    };
    assert_eq!(workspace_ids.len(), 2);
    assert_eq!(app.workspaces.len(), 1);
    assert!(app.active_workspace_id.is_some());

    app.apply(AppIntent::Workspace(WorkspaceIntent::CloseAll {
        project_id: None,
    }))
    .expect("close everything");
    assert!(app.workspaces.is_empty());
    assert_eq!(app.active_workspace_id, None);
}

#[test]
fn surface_rename_and_session_type_round_trip() {
    let mut app = AppState::default();
    let surface_id = app
        .active_workspace()
        .map(|workspace| {
            pandamux_core::find_leaf(&workspace.split_tree, &PaneId::from("pane-default"))
                .expect("leaf")
                .surfaces[0]
                .id
                .clone()
        })
        .expect("default surface");

    app.apply(AppIntent::Surface(SurfaceIntent::Rename {
        workspace_id: None,
        surface_id: surface_id.clone(),
        name: Some("Claude: auth refactor".to_string()),
    }))
    .expect("rename");
    app.apply(AppIntent::Surface(SurfaceIntent::SetSessionType {
        workspace_id: None,
        surface_id: surface_id.clone(),
        session: SessionType::Claude,
    }))
    .expect("set session type");

    let leaf = pandamux_core::find_leaf(
        &app.active_workspace().unwrap().split_tree,
        &PaneId::from("pane-default"),
    )
    .expect("leaf");
    assert_eq!(
        leaf.surfaces[0].name.as_deref(),
        Some("Claude: auth refactor")
    );
    assert_eq!(leaf.surfaces[0].session, Some(SessionType::Claude));

    // Clearing the name and resetting to Terminal store as None (compact JSON).
    app.apply(AppIntent::Surface(SurfaceIntent::Rename {
        workspace_id: None,
        surface_id: surface_id.clone(),
        name: Some("   ".to_string()),
    }))
    .expect("clear rename");
    app.apply(AppIntent::Surface(SurfaceIntent::SetSessionType {
        workspace_id: None,
        surface_id,
        session: SessionType::Terminal,
    }))
    .expect("reset type");
    let leaf = pandamux_core::find_leaf(
        &app.active_workspace().unwrap().split_tree,
        &PaneId::from("pane-default"),
    )
    .expect("leaf");
    assert!(leaf.surfaces[0].name.is_none());
    assert!(leaf.surfaces[0].session.is_none());
}

#[test]
fn manual_merge_split_and_rename_shape_the_registry() {
    let mut app = AppState::default();
    app.workspaces.clear();
    app.active_workspace_id = None;
    let first = project_workspace(&mut app, "One", "C:\\Dev\\One");
    project_workspace(&mut app, "Two", "C:\\Dev\\Two");
    ensure_project_registry(&mut app, 1);
    let one_id = app.workspaces[0].project_id.clone().unwrap();
    let two_id = app.workspaces[1].project_id.clone().unwrap();

    app.apply(AppIntent::Project(ProjectIntent::Rename {
        project_id: one_id.clone(),
        name: "Renamed One".to_string(),
    }))
    .expect("rename project");
    assert!(
        app.projects
            .iter()
            .any(|record| record.name == "Renamed One" && record.manual)
    );

    app.apply(AppIntent::Project(ProjectIntent::Merge {
        source: two_id.clone(),
        target: one_id.clone(),
    }))
    .expect("merge projects");
    assert_eq!(app.projects.len(), 1);
    assert!(
        app.workspaces
            .iter()
            .all(|workspace| workspace.project_id == Some(one_id.clone()))
    );

    let delta = app
        .apply(AppIntent::Project(ProjectIntent::Split {
            workspace_id: first.clone(),
        }))
        .expect("split workspace back out");
    let AppDelta::ProjectSplit { project_id, .. } = delta else {
        panic!("expected ProjectSplit");
    };
    assert_ne!(project_id, one_id);
    assert_eq!(
        app.workspaces[0].project_id,
        Some(project_id.clone()),
        "split workspace repointed"
    );
    // Both records are manual now: a later matching hint must not re-merge.
    assert!(app.projects.iter().all(|record| record.manual));
}

#[test]
fn late_git_hint_auto_merges_machine_created_records_only() {
    let mut app = AppState::default();
    app.workspaces.clear();
    app.active_workspace_id = None;
    project_workspace(&mut app, "Local", "C:\\Dev\\CheckoutA");
    ssh_project_workspace(&mut app, "Remote", "ssh-x", "/srv/checkout-b");
    ensure_project_registry(&mut app, 1);
    let local_id = app.workspaces[0].project_id.clone().unwrap();
    let remote_id = app.workspaces[1].project_id.clone().unwrap();
    assert_ne!(local_id, remote_id, "different folder names stay separate");

    let matcher = pandamux_core::ProjectMatcher::GitRemote {
        url: "github.com/org/repo".to_string(),
    };
    app.apply(AppIntent::Project(ProjectIntent::AttachMatcher {
        project_id: local_id.clone(),
        matcher: matcher.clone(),
    }))
    .expect("attach to local");
    // The remote checkout reports the same git remote: auto-merge into the
    // existing owner because neither record is manual.
    let delta = app
        .apply(AppIntent::Project(ProjectIntent::AttachMatcher {
            project_id: remote_id.clone(),
            matcher,
        }))
        .expect("attach to remote");
    assert!(matches!(delta, AppDelta::ProjectsMerged { .. }));
    assert!(
        app.workspaces
            .iter()
            .all(|workspace| workspace.project_id == Some(local_id.clone()))
    );
}
