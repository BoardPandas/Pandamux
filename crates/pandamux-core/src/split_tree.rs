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

/// What runs inside a terminal surface (spec 2.2 / 2.7). `Terminal` is the
/// project's default shell; the agent variants launch their CLI as the PTY
/// program. Old session files without the field deserialize as `None`, which
/// means `Terminal`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SessionType {
    #[default]
    Terminal,
    /// A specific PowerShell flavor ("pwsh", "powershell") or another shell.
    PowerShell {
        program: String,
    },
    Claude,
    Codex,
    Gemini,
    Custom {
        command: String,
    },
}

impl SessionType {
    /// Short badge label for rail entries and tabs.
    pub fn label(&self) -> &str {
        match self {
            Self::Terminal => "Terminal",
            Self::PowerShell { .. } => "PowerShell",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini",
            Self::Custom { .. } => "Custom",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceRef {
    pub id: SurfaceId,
    #[serde(rename = "type")]
    pub surface_type: SurfaceType,
    /// What runs in this surface. `None` means a plain Terminal (and is what
    /// pre-existing session files deserialize to).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionType>,
    /// User-set display name (spec 2.1 rename). Travels with the surface
    /// through drag-drop moves and persists with the session file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl SurfaceRef {
    pub fn new(id: SurfaceId, surface_type: SurfaceType) -> Self {
        Self {
            id,
            surface_type,
            session: None,
            name: None,
        }
    }
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
    create_leaf_with_ids(
        pane_id.unwrap_or_else(PaneId::generate),
        SurfaceId::generate(),
        surface_type,
    )
}

pub fn create_leaf_with_ids(
    pane_id: PaneId,
    surface_id: SurfaceId,
    surface_type: SurfaceType,
) -> SplitNode {
    SplitNode::Leaf(LeafNode {
        pane_id,
        surfaces: vec![SurfaceRef::new(surface_id, surface_type)],
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

/// Where a dragged surface is dropped relative to a target pane (plan Section
/// 12.3). `Center` appends the surface as a tab; the directional zones create a
/// new pane beside/above/below the target holding the moved surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DropZone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl DropZone {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "center" => Some(DropZone::Center),
            "left" => Some(DropZone::Left),
            "right" => Some(DropZone::Right),
            "top" => Some(DropZone::Top),
            "bottom" => Some(DropZone::Bottom),
            _ => None,
        }
    }

    /// Directional zones map to a split (direction + whether the new leaf goes
    /// first). `Center` has no split.
    fn split(self) -> Option<(SplitDirection, bool)> {
        match self {
            DropZone::Center => None,
            DropZone::Left => Some((SplitDirection::Horizontal, true)),
            DropZone::Right => Some((SplitDirection::Horizontal, false)),
            DropZone::Top => Some((SplitDirection::Vertical, true)),
            DropZone::Bottom => Some((SplitDirection::Vertical, false)),
        }
    }
}

/// The result of moving a surface: the new tree and the pane that should take
/// focus (the destination).
#[derive(Clone, Debug, PartialEq)]
pub struct MoveResult {
    pub tree: SplitNode,
    pub focus_pane_id: PaneId,
}

/// Move an existing surface to a drop target (plan Section 12.3 drop semantics).
///
/// - `Center`: append the surface as a tab in the target pane.
/// - directional: create a new single-surface pane beside/above/below the
///   target, holding the moved surface.
///
/// In all cases the surface is removed from its source pane, and an emptied
/// source pane is pruned. Returns `None` for a no-op (surface/target missing, or
/// dropping a pane's only tab back onto itself), so the caller leaves the tree
/// unchanged.
pub fn move_surface(
    tree: &SplitNode,
    surface_id: &SurfaceId,
    target_pane_id: &PaneId,
    zone: DropZone,
) -> Option<MoveResult> {
    let source_pane_id = find_pane_id_for_surface(tree, surface_id)?;
    let source_leaf = find_leaf(tree, &source_pane_id)?;
    let moved = source_leaf
        .surfaces
        .iter()
        .find(|surface| &surface.id == surface_id)?
        .clone();
    let source_is_single = source_leaf.surfaces.len() == 1;

    // Target must exist.
    find_leaf(tree, target_pane_id)?;

    // No-op: dropping a pane's only tab back onto itself, or a center drop onto
    // its own pane (no structural change).
    if &source_pane_id == target_pane_id && (zone == DropZone::Center || source_is_single) {
        return None;
    }

    match zone.split() {
        None => {
            // Center: append to the target, then remove + prune the source.
            let tree = append_surface_to_leaf(tree, target_pane_id, moved);
            let tree = remove_surface_and_prune(&tree, &source_pane_id, surface_id);
            Some(MoveResult {
                tree,
                focus_pane_id: target_pane_id.clone(),
            })
        }
        Some((direction, new_first)) => {
            // Remove + prune the source first (a same-pane directional drop keeps
            // the source alive because it still has other surfaces), then split
            // the target, placing the moved surface in the new leaf.
            let tree = remove_surface_and_prune(tree, &source_pane_id, surface_id);
            let new_pane_id = PaneId::generate();
            let new_leaf = LeafNode {
                pane_id: new_pane_id.clone(),
                surfaces: vec![moved],
                active_surface_index: 0,
            };
            let tree = split_with_leaf(&tree, target_pane_id, new_leaf, direction, new_first);
            Some(MoveResult {
                tree,
                focus_pane_id: new_pane_id,
            })
        }
    }
}

fn append_surface_to_leaf(tree: &SplitNode, pane_id: &PaneId, surface: SurfaceRef) -> SplitNode {
    let Some(leaf) = find_leaf(tree, pane_id) else {
        return tree.clone();
    };
    let mut leaf = leaf.clone();
    leaf.surfaces.push(surface);
    leaf.active_surface_index = leaf.surfaces.len() - 1;
    replace_leaf(tree, pane_id, leaf)
}

fn remove_surface_and_prune(
    tree: &SplitNode,
    pane_id: &PaneId,
    surface_id: &SurfaceId,
) -> SplitNode {
    let Some(leaf) = find_leaf(tree, pane_id) else {
        return tree.clone();
    };
    let mut leaf = leaf.clone();
    let Some(position) = leaf.surfaces.iter().position(|s| &s.id == surface_id) else {
        return tree.clone();
    };
    leaf.surfaces.remove(position);
    if leaf.surfaces.is_empty() {
        remove_leaf(tree, pane_id).unwrap_or_else(|| tree.clone())
    } else {
        if leaf.active_surface_index >= leaf.surfaces.len() {
            leaf.active_surface_index = leaf.surfaces.len() - 1;
        }
        replace_leaf(tree, pane_id, leaf)
    }
}

/// Like [`split_node`], but inserts a provided (already-populated) leaf instead
/// of creating a fresh single-surface one, and honors `new_first` ordering.
fn split_with_leaf(
    tree: &SplitNode,
    target_pane_id: &PaneId,
    new_leaf: LeafNode,
    direction: SplitDirection,
    new_first: bool,
) -> SplitNode {
    match tree {
        SplitNode::Leaf(leaf) => {
            if &leaf.pane_id != target_pane_id {
                return tree.clone();
            }
            let inserted = SplitNode::Leaf(new_leaf);
            let children = if new_first {
                [inserted, tree.clone()]
            } else {
                [tree.clone(), inserted]
            };
            SplitNode::Branch(BranchNode {
                direction,
                ratio: 0.5,
                children: Box::new(children),
            })
        }
        SplitNode::Branch(branch) => {
            let left = split_with_leaf(
                &branch.children[0],
                target_pane_id,
                new_leaf.clone(),
                direction,
                new_first,
            );
            let right = split_with_leaf(
                &branch.children[1],
                target_pane_id,
                new_leaf,
                direction,
                new_first,
            );
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

#[cfg(test)]
mod move_tests {
    use super::*;

    fn leaf(pane: &str, surfaces: &[&str]) -> SplitNode {
        SplitNode::Leaf(LeafNode {
            pane_id: PaneId::from(pane),
            surfaces: surfaces
                .iter()
                .map(|id| SurfaceRef::new(SurfaceId::from(*id), SurfaceType::Terminal))
                .collect(),
            active_surface_index: 0,
        })
    }

    fn branch(direction: SplitDirection, first: SplitNode, second: SplitNode) -> SplitNode {
        SplitNode::Branch(BranchNode {
            direction,
            ratio: 0.5,
            children: Box::new([first, second]),
        })
    }

    #[test]
    fn center_appends_and_prunes_source() {
        let tree = branch(
            SplitDirection::Horizontal,
            leaf("pane-a", &["s-a"]),
            leaf("pane-b", &["s-b"]),
        );
        let result = move_surface(
            &tree,
            &SurfaceId::from("s-a"),
            &PaneId::from("pane-b"),
            DropZone::Center,
        )
        .expect("move should apply");

        // pane-a emptied and pruned; pane-b now holds both surfaces.
        assert_eq!(get_all_pane_ids(&result.tree), vec![PaneId::from("pane-b")]);
        let target = find_leaf(&result.tree, &PaneId::from("pane-b")).unwrap();
        assert_eq!(target.surfaces.len(), 2);
        assert_eq!(target.active_surface_index, 1);
        assert_eq!(result.focus_pane_id, PaneId::from("pane-b"));
    }

    #[test]
    fn directional_creates_new_pane_beside_target() {
        let tree = branch(
            SplitDirection::Horizontal,
            leaf("pane-a", &["s-a"]),
            leaf("pane-b", &["s-b"]),
        );
        let result = move_surface(
            &tree,
            &SurfaceId::from("s-a"),
            &PaneId::from("pane-b"),
            DropZone::Right,
        )
        .expect("move should apply");

        let panes = get_all_pane_ids(&result.tree);
        assert_eq!(panes.len(), 2);
        assert!(panes.contains(&PaneId::from("pane-b")));
        // Focus is the freshly created pane holding the moved surface.
        assert_ne!(result.focus_pane_id, PaneId::from("pane-b"));
        let moved = find_leaf(&result.tree, &result.focus_pane_id).unwrap();
        assert_eq!(moved.surfaces[0].id, SurfaceId::from("s-a"));
        // Right => target first, new leaf second, horizontal split.
        let SplitNode::Branch(root) = &result.tree else {
            panic!("expected a branch");
        };
        assert_eq!(root.direction, SplitDirection::Horizontal);
        assert!(find_leaf(&root.children[0], &PaneId::from("pane-b")).is_some());
        assert!(find_leaf(&root.children[1], &result.focus_pane_id).is_some());
    }

    #[test]
    fn dropping_only_tab_onto_itself_is_a_no_op() {
        let tree = leaf("pane-a", &["s-a"]);
        assert!(
            move_surface(
                &tree,
                &SurfaceId::from("s-a"),
                &PaneId::from("pane-a"),
                DropZone::Center,
            )
            .is_none()
        );
        assert!(
            move_surface(
                &tree,
                &SurfaceId::from("s-a"),
                &PaneId::from("pane-a"),
                DropZone::Right,
            )
            .is_none()
        );
    }

    #[test]
    fn directional_split_of_own_multi_tab_pane_keeps_source() {
        let tree = leaf("pane-a", &["s-1", "s-2"]);
        let result = move_surface(
            &tree,
            &SurfaceId::from("s-2"),
            &PaneId::from("pane-a"),
            DropZone::Bottom,
        )
        .expect("move should apply");

        assert_eq!(get_all_pane_ids(&result.tree).len(), 2);
        let source = find_leaf(&result.tree, &PaneId::from("pane-a")).unwrap();
        assert_eq!(source.surfaces.len(), 1);
        assert_eq!(source.surfaces[0].id, SurfaceId::from("s-1"));
        let moved = find_leaf(&result.tree, &result.focus_pane_id).unwrap();
        assert_eq!(moved.surfaces[0].id, SurfaceId::from("s-2"));
        let SplitNode::Branch(root) = &result.tree else {
            panic!("expected a branch");
        };
        assert_eq!(root.direction, SplitDirection::Vertical);
    }

    #[test]
    fn missing_surface_or_target_is_a_no_op() {
        let tree = branch(
            SplitDirection::Horizontal,
            leaf("pane-a", &["s-a"]),
            leaf("pane-b", &["s-b"]),
        );
        assert!(
            move_surface(
                &tree,
                &SurfaceId::from("s-missing"),
                &PaneId::from("pane-b"),
                DropZone::Center,
            )
            .is_none()
        );
        assert!(
            move_surface(
                &tree,
                &SurfaceId::from("s-a"),
                &PaneId::from("pane-missing"),
                DropZone::Center,
            )
            .is_none()
        );
    }
}
