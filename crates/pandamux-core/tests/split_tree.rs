use pandamux_core::{
    BranchNode, PaneId, SplitDirection, SplitNode, SurfaceType, build_grid_layout, create_leaf,
    find_leaf, find_pane_id_for_surface, get_all_pane_ids, remove_leaf, replace_leaf, split_node,
    update_ratio,
};

#[test]
fn creates_a_leaf_node() {
    let leaf = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);

    let SplitNode::Leaf(leaf) = leaf else {
        panic!("expected leaf");
    };

    assert_eq!(leaf.pane_id.as_str(), "pane-1");
    assert_eq!(leaf.surfaces.len(), 1);
    assert_eq!(leaf.surfaces[0].surface_type, SurfaceType::Terminal);
}

#[test]
fn splits_a_leaf_horizontally() {
    let leaf = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let result = split_node(
        &leaf,
        &PaneId::from("pane-1"),
        PaneId::from("pane-2"),
        SurfaceType::Terminal,
        SplitDirection::Horizontal,
    );

    let SplitNode::Branch(BranchNode {
        direction,
        ratio,
        children,
    }) = result
    else {
        panic!("expected branch");
    };

    assert_eq!(direction, SplitDirection::Horizontal);
    assert_eq!(ratio, 0.5);
    assert!(matches!(children[0], SplitNode::Leaf(_)));
    assert!(matches!(children[1], SplitNode::Leaf(_)));
}

#[test]
fn splits_a_leaf_vertically() {
    let leaf = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let result = split_node(
        &leaf,
        &PaneId::from("pane-1"),
        PaneId::from("pane-2"),
        SurfaceType::Terminal,
        SplitDirection::Vertical,
    );

    let SplitNode::Branch(branch) = result else {
        panic!("expected branch");
    };

    assert_eq!(branch.direction, SplitDirection::Vertical);
}

#[test]
fn removes_a_leaf_and_collapses_parent() {
    let leaf = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let tree = split_node(
        &leaf,
        &PaneId::from("pane-1"),
        PaneId::from("pane-2"),
        SurfaceType::Terminal,
        SplitDirection::Horizontal,
    );
    let result = remove_leaf(&tree, &PaneId::from("pane-2")).expect("tree should remain");

    let SplitNode::Leaf(leaf) = result else {
        panic!("expected leaf");
    };

    assert_eq!(leaf.pane_id.as_str(), "pane-1");
}

#[test]
fn finds_a_leaf_by_pane_id() {
    let leaf = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let tree = split_node(
        &leaf,
        &PaneId::from("pane-1"),
        PaneId::from("pane-2"),
        SurfaceType::Terminal,
        SplitDirection::Vertical,
    );

    assert!(find_leaf(&tree, &PaneId::from("pane-2")).is_some());
    assert!(find_leaf(&tree, &PaneId::from("pane-999")).is_none());
}

#[test]
fn replaces_a_leaf_and_finds_surface_owner() {
    let tree = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let mut leaf = find_leaf(&tree, &PaneId::from("pane-1"))
        .expect("leaf should exist")
        .clone();
    leaf.active_surface_index = 0;
    let surface_id = leaf.surfaces[0].id.clone();

    let replaced = replace_leaf(&tree, &PaneId::from("pane-1"), leaf);

    assert_eq!(
        find_pane_id_for_surface(&replaced, &surface_id),
        Some(PaneId::from("pane-1"))
    );
}

#[test]
fn updates_ratio_of_a_branch() {
    let leaf = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let tree = split_node(
        &leaf,
        &PaneId::from("pane-1"),
        PaneId::from("pane-2"),
        SurfaceType::Terminal,
        SplitDirection::Horizontal,
    );
    let updated = update_ratio(&tree, &PaneId::from("pane-1"), &PaneId::from("pane-2"), 0.7);

    let SplitNode::Branch(branch) = updated else {
        panic!("expected branch");
    };

    assert_eq!(branch.ratio, 0.7);
}

#[test]
fn clamps_ratio_between_bounds() {
    let leaf = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let tree = split_node(
        &leaf,
        &PaneId::from("pane-1"),
        PaneId::from("pane-2"),
        SurfaceType::Terminal,
        SplitDirection::Horizontal,
    );
    let updated = update_ratio(&tree, &PaneId::from("pane-1"), &PaneId::from("pane-2"), 1.5);

    let SplitNode::Branch(branch) = updated else {
        panic!("expected branch");
    };

    assert_eq!(branch.ratio, 0.9);
}

#[test]
fn grid_layout_returns_empty_result_when_count_is_less_than_two() {
    let tree = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let result = build_grid_layout(&tree, &PaneId::from("pane-1"), 1, SurfaceType::Terminal);

    assert!(result.new_pane_ids.is_empty());
    assert_eq!(result.tree, tree);
}

#[test]
fn grid_layout_builds_two_cells_from_one_pane() {
    let tree = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let result = build_grid_layout(&tree, &PaneId::from("pane-1"), 2, SurfaceType::Terminal);
    let pane_ids = get_all_pane_ids(&result.tree);

    assert_eq!(result.new_pane_ids.len(), 1);
    assert_eq!(pane_ids.len(), 2);
    assert_eq!(pane_ids[0].as_str(), "pane-1");
    assert!(pane_ids.contains(&result.new_pane_ids[0]));
}

#[test]
fn grid_layout_builds_a_four_cell_grid() {
    let tree = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let result = build_grid_layout(&tree, &PaneId::from("pane-1"), 4, SurfaceType::Terminal);

    assert_eq!(result.new_pane_ids.len(), 3);
    assert_eq!(get_all_pane_ids(&result.tree).len(), 4);
}

#[test]
fn grid_layout_uses_the_full_workspace_viewport_with_existing_panes() {
    let mut tree = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    tree = split_node(
        &tree,
        &PaneId::from("pane-1"),
        PaneId::from("pane-2"),
        SurfaceType::Terminal,
        SplitDirection::Horizontal,
    );
    tree = split_node(
        &tree,
        &PaneId::from("pane-2"),
        PaneId::from("pane-3"),
        SurfaceType::Terminal,
        SplitDirection::Vertical,
    );

    let result = build_grid_layout(&tree, &PaneId::from("pane-1"), 3, SurfaceType::Terminal);
    let pane_ids = get_all_pane_ids(&result.tree);
    let anchor = find_leaf(&result.tree, &PaneId::from("pane-1")).expect("anchor should exist");

    assert_eq!(pane_ids.len(), 3);
    assert!(pane_ids.contains(&PaneId::from("pane-1")));
    assert!(!pane_ids.contains(&PaneId::from("pane-2")));
    assert!(!pane_ids.contains(&PaneId::from("pane-3")));
    assert_eq!(result.new_pane_ids.len(), 2);
    assert_eq!(anchor.surfaces.len(), 3);
    assert_eq!(anchor.active_surface_index, 0);
}

#[test]
fn grid_layout_preserves_surfaces_by_absorbing_them_as_tabs() {
    let mut tree = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    tree = split_node(
        &tree,
        &PaneId::from("pane-1"),
        PaneId::from("pane-2"),
        SurfaceType::Terminal,
        SplitDirection::Horizontal,
    );
    tree = split_node(
        &tree,
        &PaneId::from("pane-2"),
        PaneId::from("pane-3"),
        SurfaceType::Terminal,
        SplitDirection::Vertical,
    );
    tree = split_node(
        &tree,
        &PaneId::from("pane-3"),
        PaneId::from("pane-4"),
        SurfaceType::Browser,
        SplitDirection::Horizontal,
    );

    let mut original_surface_ids = Vec::new();
    for pane_id in get_all_pane_ids(&tree) {
        let leaf = find_leaf(&tree, &pane_id).expect("pane should exist");
        original_surface_ids.extend(leaf.surfaces.iter().map(|surface| surface.id.clone()));
    }

    let result = build_grid_layout(&tree, &PaneId::from("pane-1"), 3, SurfaceType::Terminal);
    let mut surviving_surface_ids = Vec::new();
    for pane_id in get_all_pane_ids(&result.tree) {
        let leaf = find_leaf(&result.tree, &pane_id).expect("pane should exist");
        surviving_surface_ids.extend(leaf.surfaces.iter().map(|surface| surface.id.clone()));
    }

    for surface_id in original_surface_ids {
        assert!(surviving_surface_ids.contains(&surface_id));
    }
}

#[test]
fn grid_layout_returns_new_pane_ids_in_row_major_order() {
    let tree = create_leaf(Some(PaneId::from("pane-1")), SurfaceType::Terminal);
    let result = build_grid_layout(&tree, &PaneId::from("pane-1"), 5, SurfaceType::Terminal);

    assert_eq!(result.new_pane_ids.len(), 4);
    for pane_id in &result.new_pane_ids {
        assert!(find_leaf(&result.tree, pane_id).is_some());
    }
}
