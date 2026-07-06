use crate::ids::{PaneId, SurfaceId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceType {
    Terminal,
    Markdown,
    Diff,
    Browser,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceRef {
    pub id: SurfaceId,
    #[serde(rename = "type")]
    pub surface_type: SurfaceType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeafNode {
    pub pane_id: PaneId,
    pub surfaces: Vec<SurfaceRef>,
    pub active_surface_index: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchNode {
    pub direction: SplitDirection,
    pub ratio: f32,
    pub children: Box<[SplitNode; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SplitNode {
    Leaf(LeafNode),
    Branch(BranchNode),
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridLayoutResult {
    pub tree: SplitNode,
    pub new_pane_ids: Vec<PaneId>,
}

pub fn create_leaf(pane_id: Option<PaneId>, surface_type: SurfaceType) -> SplitNode {
    SplitNode::Leaf(LeafNode {
        pane_id: pane_id.unwrap_or_else(PaneId::generate),
        surfaces: vec![SurfaceRef {
            id: SurfaceId::generate(),
            surface_type,
        }],
        active_surface_index: 0,
    })
}

pub fn split_node(
    tree: &SplitNode,
    target_pane_id: &PaneId,
    new_pane_id: PaneId,
    surface_type: SurfaceType,
    direction: SplitDirection,
) -> SplitNode {
    match tree {
        SplitNode::Leaf(leaf) => {
            if &leaf.pane_id != target_pane_id {
                return tree.clone();
            }

            SplitNode::Branch(BranchNode {
                direction,
                ratio: 0.5,
                children: Box::new([tree.clone(), create_leaf(Some(new_pane_id), surface_type)]),
            })
        }
        SplitNode::Branch(branch) => {
            let left = split_node(
                &branch.children[0],
                target_pane_id,
                new_pane_id.clone(),
                surface_type.clone(),
                direction,
            );
            let right = split_node(
                &branch.children[1],
                target_pane_id,
                new_pane_id,
                surface_type,
                direction,
            );

            if left == branch.children[0] && right == branch.children[1] {
                return tree.clone();
            }

            SplitNode::Branch(BranchNode {
                children: Box::new([left, right]),
                ..branch.clone()
            })
        }
    }
}

pub fn remove_leaf(tree: &SplitNode, pane_id: &PaneId) -> Option<SplitNode> {
    match tree {
        SplitNode::Leaf(leaf) => {
            if &leaf.pane_id == pane_id {
                None
            } else {
                Some(tree.clone())
            }
        }
        SplitNode::Branch(branch) => {
            let left = remove_leaf(&branch.children[0], pane_id);
            let right = remove_leaf(&branch.children[1], pane_id);

            match (left, right) {
                (None, None) => None,
                (None, Some(right)) => Some(right),
                (Some(left), None) => Some(left),
                (Some(left), Some(right)) => {
                    if left == branch.children[0] && right == branch.children[1] {
                        Some(tree.clone())
                    } else {
                        Some(SplitNode::Branch(BranchNode {
                            children: Box::new([left, right]),
                            ..branch.clone()
                        }))
                    }
                }
            }
        }
    }
}

pub fn find_leaf<'a>(tree: &'a SplitNode, pane_id: &PaneId) -> Option<&'a LeafNode> {
    match tree {
        SplitNode::Leaf(leaf) => {
            if &leaf.pane_id == pane_id {
                Some(leaf)
            } else {
                None
            }
        }
        SplitNode::Branch(branch) => find_leaf(&branch.children[0], pane_id)
            .or_else(|| find_leaf(&branch.children[1], pane_id)),
    }
}

pub fn replace_leaf(tree: &SplitNode, pane_id: &PaneId, replacement: LeafNode) -> SplitNode {
    match tree {
        SplitNode::Leaf(leaf) => {
            if &leaf.pane_id == pane_id {
                SplitNode::Leaf(replacement)
            } else {
                tree.clone()
            }
        }
        SplitNode::Branch(branch) => {
            let left = replace_leaf(&branch.children[0], pane_id, replacement.clone());
            let right = replace_leaf(&branch.children[1], pane_id, replacement);

            if left == branch.children[0] && right == branch.children[1] {
                tree.clone()
            } else {
                SplitNode::Branch(BranchNode {
                    children: Box::new([left, right]),
                    ..branch.clone()
                })
            }
        }
    }
}

pub fn find_pane_id_for_surface(tree: &SplitNode, surface_id: &SurfaceId) -> Option<PaneId> {
    match tree {
        SplitNode::Leaf(leaf) => {
            if leaf
                .surfaces
                .iter()
                .any(|surface| &surface.id == surface_id)
            {
                Some(leaf.pane_id.clone())
            } else {
                None
            }
        }
        SplitNode::Branch(branch) => find_pane_id_for_surface(&branch.children[0], surface_id)
            .or_else(|| find_pane_id_for_surface(&branch.children[1], surface_id)),
    }
}

pub fn update_ratio(
    tree: &SplitNode,
    left_pane_id: &PaneId,
    right_pane_id: &PaneId,
    new_ratio: f32,
) -> SplitNode {
    match tree {
        SplitNode::Leaf(_) => tree.clone(),
        SplitNode::Branch(branch) => {
            let left = &branch.children[0];
            let right = &branch.children[1];
            let left_has_left = branch_contains_pane_id(left, left_pane_id);
            let left_has_right = branch_contains_pane_id(left, right_pane_id);
            let right_has_left = branch_contains_pane_id(right, left_pane_id);
            let right_has_right = branch_contains_pane_id(right, right_pane_id);

            if (left_has_left && right_has_right) || (left_has_right && right_has_left) {
                return SplitNode::Branch(BranchNode {
                    ratio: clamp_ratio(new_ratio),
                    ..branch.clone()
                });
            }

            let updated_left = update_ratio(left, left_pane_id, right_pane_id, new_ratio);
            let updated_right = update_ratio(right, left_pane_id, right_pane_id, new_ratio);
            if updated_left == *left && updated_right == *right {
                tree.clone()
            } else {
                SplitNode::Branch(BranchNode {
                    children: Box::new([updated_left, updated_right]),
                    ..branch.clone()
                })
            }
        }
    }
}

pub fn get_all_pane_ids(tree: &SplitNode) -> Vec<PaneId> {
    match tree {
        SplitNode::Leaf(leaf) => vec![leaf.pane_id.clone()],
        SplitNode::Branch(branch) => {
            let mut ids = get_all_pane_ids(&branch.children[0]);
            ids.extend(get_all_pane_ids(&branch.children[1]));
            ids
        }
    }
}

pub fn adjust_pane_ratio(
    tree: &SplitNode,
    pane_id: &PaneId,
    orientation: SplitDirection,
    delta: f32,
) -> SplitNode {
    match tree {
        SplitNode::Leaf(_) => tree.clone(),
        SplitNode::Branch(branch) => {
            let left = &branch.children[0];
            let right = &branch.children[1];
            let in_left = branch_contains_pane_id(left, pane_id);
            let in_right = branch_contains_pane_id(right, pane_id);
            if !in_left && !in_right {
                return tree.clone();
            }

            let child_with_pane = if in_left { left } else { right };
            let adjusted_child = adjust_pane_ratio(child_with_pane, pane_id, orientation, delta);
            if adjusted_child != *child_with_pane {
                return if in_left {
                    SplitNode::Branch(BranchNode {
                        children: Box::new([adjusted_child, right.clone()]),
                        ..branch.clone()
                    })
                } else {
                    SplitNode::Branch(BranchNode {
                        children: Box::new([left.clone(), adjusted_child]),
                        ..branch.clone()
                    })
                };
            }

            if branch.direction == orientation {
                SplitNode::Branch(BranchNode {
                    ratio: clamp_ratio(branch.ratio + delta),
                    ..branch.clone()
                })
            } else {
                tree.clone()
            }
        }
    }
}

pub fn collect_active_terminal_surface_ids(tree: &SplitNode) -> Vec<SurfaceId> {
    match tree {
        SplitNode::Leaf(leaf) => {
            let Some(active) = leaf.surfaces.get(leaf.active_surface_index) else {
                return Vec::new();
            };

            if active.surface_type == SurfaceType::Terminal {
                vec![active.id.clone()]
            } else {
                Vec::new()
            }
        }
        SplitNode::Branch(branch) => {
            let mut ids = collect_active_terminal_surface_ids(&branch.children[0]);
            ids.extend(collect_active_terminal_surface_ids(&branch.children[1]));
            ids
        }
    }
}

pub fn build_grid_layout(
    tree: &SplitNode,
    anchor_pane_id: &PaneId,
    count: usize,
    surface_type: SurfaceType,
) -> GridLayoutResult {
    if count < 2 {
        return GridLayoutResult {
            tree: tree.clone(),
            new_pane_ids: Vec::new(),
        };
    }

    let Some(anchor) = find_leaf(tree, anchor_pane_id) else {
        return GridLayoutResult {
            tree: tree.clone(),
            new_pane_ids: Vec::new(),
        };
    };

    let mut absorbed_surfaces = Vec::new();
    for pane_id in get_all_pane_ids(tree) {
        if &pane_id == anchor_pane_id {
            continue;
        }

        if let Some(other_leaf) = find_leaf(tree, &pane_id) {
            absorbed_surfaces.extend(other_leaf.surfaces.iter().cloned());
        }
    }

    let mut merged_anchor = anchor.clone();
    merged_anchor.surfaces.extend(absorbed_surfaces);

    let columns = (count as f64).sqrt().ceil().max(1.0) as usize;
    let rows = count.div_ceil(columns).max(1);
    let mut cells = Vec::with_capacity(count);
    let mut new_pane_ids = Vec::with_capacity(count.saturating_sub(1));

    for index in 0..count {
        if index == 0 {
            cells.push(SplitNode::Leaf(merged_anchor.clone()));
        } else {
            let pane_id = PaneId::generate();
            new_pane_ids.push(pane_id.clone());
            cells.push(create_leaf(Some(pane_id), surface_type.clone()));
        }
    }

    let mut row_trees = Vec::with_capacity(rows);
    for row_index in 0..rows {
        let start = row_index * columns;
        let end = (start + columns).min(count);
        let row_cells = &cells[start..end];
        let mut row_tree = row_cells[row_cells.len() - 1].clone();

        for cell_index in (0..row_cells.len() - 1).rev() {
            row_tree = SplitNode::Branch(BranchNode {
                direction: SplitDirection::Horizontal,
                ratio: 1.0 / (row_cells.len() - cell_index) as f32,
                children: Box::new([row_cells[cell_index].clone(), row_tree]),
            });
        }

        row_trees.push(row_tree);
    }

    let mut grid_tree = row_trees[row_trees.len() - 1].clone();
    for row_index in (0..row_trees.len() - 1).rev() {
        grid_tree = SplitNode::Branch(BranchNode {
            direction: SplitDirection::Vertical,
            ratio: 1.0 / (row_trees.len() - row_index) as f32,
            children: Box::new([row_trees[row_index].clone(), grid_tree]),
        });
    }

    GridLayoutResult {
        tree: grid_tree,
        new_pane_ids,
    }
}

fn clamp_ratio(ratio: f32) -> f32 {
    ratio.clamp(0.1, 0.9)
}

fn branch_contains_pane_id(node: &SplitNode, pane_id: &PaneId) -> bool {
    match node {
        SplitNode::Leaf(leaf) => &leaf.pane_id == pane_id,
        SplitNode::Branch(branch) => {
            branch_contains_pane_id(&branch.children[0], pane_id)
                || branch_contains_pane_id(&branch.children[1], pane_id)
        }
    }
}
