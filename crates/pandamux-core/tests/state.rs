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
