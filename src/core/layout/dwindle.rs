use crate::models::WindowEntry;

use super::{
    GAP_ROUNDING_BUFFER, IndexedWindow, Rect, bounding_rect, extract_indexed,
    infer_gap_from_geometry, split_bounds,
};

/// Binary split direction in the Dwindle BSP tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    /// Left / Right
    Horizontal,
    /// Top / Bottom
    Vertical,
}

/// A node in the inferred BSP tree, storing window indices.
#[derive(Debug)]
#[allow(dead_code)]
pub enum BspNode {
    Leaf {
        idx: usize,
    },
    Split {
        dir: SplitDir,
        ratio: f64,
        first: Box<Self>,
        second: Box<Self>,
    },
}

/// A single step in the restore plan, executed in order.
#[derive(Debug, Clone)]
pub struct RestoreStep {
    /// Index into the original window list for the window to open.
    pub window_idx: usize,
    /// If Some, this window should be placed adjacent to this previously-
    /// opened window index once it exists.
    pub focus_idx: Option<usize>,
    /// If Some, the direction to preselect this window relative to
    /// `focus_idx`. Consumed via `layoutmsg preselect <dir>` immediately
    /// before the window's placement settles — not before it opens, since a
    /// preselect binding armed earlier doesn't reliably survive an
    /// indeterminate wait for the window to actually appear.
    pub preselect: Option<PreselDir>,
}

/// Post-placement correction: focus this window and apply
/// `layoutmsg splitratio <delta>` to set its parent split precisely.
#[derive(Debug, Clone)]
pub struct SplitRatioStep {
    pub focus_window_idx: usize,
    pub ratio: f64,
}

/// Post-placement correction for a split node whose two children are both
/// internal sub-trees rather than a direct leaf window. No window is ever
/// directly parented by such a split, so `layoutmsg splitratio` (which only
/// ever adjusts the currently focused window's own immediate parent split)
/// can never reach it. It's reachable only through a raw pixel resize:
/// Hyprland's resize dispatcher walks from the resized window up to the
/// nearest ancestor split matching the resize axis, so resizing the
/// leftmost leaf of one side lands on exactly this split.
#[derive(Debug, Clone)]
pub struct PixelRatioStep {
    pub dir: SplitDir,
    pub ratio: f64,
    pub first_leaf_idx: usize,
    pub second_leaf_idx: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum PreselDir {
    Right,
    Bottom,
}

impl std::fmt::Display for PreselDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Right => write!(f, "r"),
            // Hyprland's dwindle `layoutmsg preselect <dir>` only recognizes
            // l/r/u/d (left/right/up/down) — "b" is not a valid direction.
            // Live-tested: "b" produced inconsistent placement (looked correct
            // in simple 2-window cases but landed windows on the wrong side
            // of the tree entirely once more than ~2 splits were involved).
            Self::Bottom => write!(f, "d"),
        }
    }
}

/// Complete restore plan for one workspace under the dwindle layout.
pub struct DwindlePlan {
    pub steps: Vec<RestoreStep>,
    pub ratio_steps: Vec<SplitRatioStep>,
    pub pixel_ratio_steps: Vec<PixelRatioStep>,
}

/// Infer a BSP tree from a set of tiled windows based on their geometry.
///
/// The gap tolerance is measured from the actual window positions rather than
/// from a config value, so it adapts to any `gaps_in` / `gaps_out` setting.
///
/// Returns `None` if the windows can't form a valid BSP partition
/// (e.g. overlapping, gaps, or missing geometry).
fn infer_bsp(indexed: &[IndexedWindow], bounds: Rect) -> Option<BspNode> {
    if indexed.is_empty() {
        return None;
    }
    let tolerance = infer_gap_from_geometry(indexed) + GAP_ROUNDING_BUFFER;
    infer_bsp_inner(indexed, bounds, tolerance)
}

fn infer_bsp_inner(indexed: &[IndexedWindow], bounds: Rect, tolerance: i32) -> Option<BspNode> {
    if indexed.is_empty() {
        return None;
    }
    if indexed.len() == 1 {
        return Some(BspNode::Leaf {
            idx: indexed[0].idx,
        });
    }

    try_split(indexed, bounds, tolerance, SplitDir::Horizontal)
        .or_else(|| try_split(indexed, bounds, tolerance, SplitDir::Vertical))
}

fn try_split(
    indexed: &[IndexedWindow],
    bounds: Rect,
    tolerance: i32,
    dir: SplitDir,
) -> Option<BspNode> {
    let horizontal = dir == SplitDir::Horizontal;
    let candidates = super::split_candidates(indexed, horizontal, tolerance);

    for &split_at in &candidates {
        if let Some(node) = try_split_at(indexed, bounds, tolerance, dir, split_at) {
            return Some(node);
        }
    }
    None
}

fn try_split_at(
    indexed: &[IndexedWindow],
    bounds: Rect,
    tolerance: i32,
    dir: SplitDir,
    split_at: i32,
) -> Option<BspNode> {
    let horizontal = dir == SplitDir::Horizontal;
    let (range_start, range_end) = if horizontal {
        (bounds.x, bounds.x + bounds.w)
    } else {
        (bounds.y, bounds.y + bounds.h)
    };

    if split_at <= range_start || split_at >= range_end {
        return None;
    }

    let mut first_group: Vec<&IndexedWindow> = Vec::new();
    let mut second_group: Vec<&IndexedWindow> = Vec::new();

    for iw in indexed {
        let (start, end) = if horizontal {
            (iw.x, iw.x + iw.w)
        } else {
            (iw.y, iw.y + iw.h)
        };

        if end <= split_at + tolerance {
            first_group.push(iw);
        } else if start >= split_at - tolerance {
            second_group.push(iw);
        } else {
            return None;
        }
    }

    if first_group.is_empty() || second_group.is_empty() {
        return None;
    }

    let (first_bounds, second_bounds) = split_bounds(bounds, horizontal, split_at);

    let first_owned: Vec<IndexedWindow> = first_group.into_iter().cloned().collect();
    let second_owned: Vec<IndexedWindow> = second_group.into_iter().cloned().collect();

    let first_node = infer_bsp_inner(&first_owned, first_bounds, tolerance)?;
    let second_node = infer_bsp_inner(&second_owned, second_bounds, tolerance)?;

    let total = if horizontal {
        f64::from(bounds.w)
    } else {
        f64::from(bounds.h)
    };
    let first_size = if horizontal {
        f64::from(first_bounds.w)
    } else {
        f64::from(first_bounds.h)
    };

    Some(BspNode::Split {
        dir,
        ratio: first_size / total,
        first: Box::new(first_node),
        second: Box::new(second_node),
    })
}

fn plan_from_bsp(tree: &BspNode) -> Vec<RestoreStep> {
    let mut steps = Vec::new();
    walk_bsp(tree, None, None, &mut steps);
    steps
}

/// Walk the BSP tree and emit restore steps in an order compatible with
/// Hyprland's dwindle layout. In dwindle, a new window always splits the
/// *focused window's cell*, so each level's split must be created before
/// any deeper splits within its children.
///
/// For each Split node:
/// 1. Open first child's leftmost leaf (inherits parent's focus/preselect)
/// 2. Open second child's leftmost leaf (creates THIS level's split)
/// 3. Fill remaining leaves of first child (deeper splits)
/// 4. Fill remaining leaves of second child (deeper splits)
fn walk_bsp(
    node: &BspNode,
    focus_idx: Option<usize>,
    preselect: Option<PreselDir>,
    steps: &mut Vec<RestoreStep>,
) {
    match node {
        BspNode::Leaf { idx } => {
            steps.push(RestoreStep {
                window_idx: *idx,
                focus_idx,
                preselect,
            });
        }
        BspNode::Split {
            dir, first, second, ..
        } => {
            let first_leaf = leftmost_leaf_idx(first);
            let second_leaf = leftmost_leaf_idx(second);
            let presel = match dir {
                SplitDir::Horizontal => PreselDir::Right,
                SplitDir::Vertical => PreselDir::Bottom,
            };

            steps.push(RestoreStep {
                window_idx: first_leaf,
                focus_idx,
                preselect,
            });
            steps.push(RestoreStep {
                window_idx: second_leaf,
                focus_idx: Some(first_leaf),
                preselect: Some(presel),
            });

            walk_remaining(first, steps);
            walk_remaining(second, steps);
        }
    }
}

/// Emit steps for all leaves of a subtree except its leftmost leaf (which
/// was already opened by the parent's `walk_bsp`). Each split in the subtree
/// is created by opening the second child's leftmost leaf while focusing the
/// first child's leftmost leaf.
fn walk_remaining(node: &BspNode, steps: &mut Vec<RestoreStep>) {
    if let BspNode::Split {
        dir, first, second, ..
    } = node
    {
        let first_leaf = leftmost_leaf_idx(first);
        let second_leaf = leftmost_leaf_idx(second);
        let presel = match dir {
            SplitDir::Horizontal => PreselDir::Right,
            SplitDir::Vertical => PreselDir::Bottom,
        };

        steps.push(RestoreStep {
            window_idx: second_leaf,
            focus_idx: Some(first_leaf),
            preselect: Some(presel),
        });

        walk_remaining(first, steps);
        walk_remaining(second, steps);
    }
}

fn leftmost_leaf_idx(node: &BspNode) -> usize {
    match node {
        BspNode::Leaf { idx } => *idx,
        BspNode::Split { first, .. } => leftmost_leaf_idx(first),
    }
}

/// Collect `SplitRatioStep`s for every split node that has at least one
/// direct Leaf child.
fn collect_splitratio_steps(tree: &BspNode) -> Vec<SplitRatioStep> {
    let mut steps = Vec::new();
    collect_ratios_inner(tree, &mut steps);
    steps
}

fn collect_ratios_inner(node: &BspNode, steps: &mut Vec<SplitRatioStep>) {
    if let BspNode::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        if let BspNode::Leaf { idx } = first.as_ref() {
            steps.push(SplitRatioStep {
                focus_window_idx: *idx,
                ratio: *ratio,
            });
        } else if let BspNode::Leaf { idx } = second.as_ref() {
            steps.push(SplitRatioStep {
                focus_window_idx: *idx,
                ratio: *ratio,
            });
        }

        collect_ratios_inner(first, steps);
        collect_ratios_inner(second, steps);
    }
}

/// Collect `PixelRatioStep`s for every split node whose two children are
/// both internal sub-trees — the splits `collect_splitratio_steps` cannot
/// reach because neither child is a direct leaf window.
fn collect_pixel_ratio_steps(tree: &BspNode) -> Vec<PixelRatioStep> {
    let mut steps = Vec::new();
    collect_pixel_ratios_inner(tree, &mut steps);
    steps
}

fn collect_pixel_ratios_inner(node: &BspNode, steps: &mut Vec<PixelRatioStep>) {
    if let BspNode::Split {
        dir,
        ratio,
        first,
        second,
    } = node
    {
        if !matches!(first.as_ref(), BspNode::Leaf { .. })
            && !matches!(second.as_ref(), BspNode::Leaf { .. })
        {
            steps.push(PixelRatioStep {
                dir: *dir,
                ratio: *ratio,
                first_leaf_idx: leftmost_leaf_idx(first),
                second_leaf_idx: leftmost_leaf_idx(second),
            });
        }

        collect_pixel_ratios_inner(first, steps);
        collect_pixel_ratios_inner(second, steps);
    }
}

/// Build a dwindle restore plan for tiled windows on a single workspace.
///
/// `global_indices` maps each window in `windows` to its index in the full
/// session window list.
///
/// Returns `None` if BSP inference fails.
pub fn build_workspace_plan(
    windows: &[&WindowEntry],
    global_indices: &[usize],
) -> Option<DwindlePlan> {
    let bounds = bounding_rect(windows)?;
    let indexed = extract_indexed(windows, global_indices)?;
    let tree = infer_bsp(&indexed, bounds)?;
    let steps = plan_from_bsp(&tree);
    let ratio_steps = collect_splitratio_steps(&tree);
    let pixel_ratio_steps = collect_pixel_ratio_steps(&tree);
    Some(DwindlePlan {
        steps,
        ratio_steps,
        pixel_ratio_steps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(app_id: &str, ws: &str, x: i32, y: i32, w: i32, h: i32) -> WindowEntry {
        WindowEntry {
            app_id: app_id.to_string(),
            launch_cmd: app_id.to_string(),
            workspace: ws.to_string(),
            monitor: None,
            floating: false,
            fullscreen: false,
            position: Some((x, y)),
            size: Some((w, h)),
            cwd: None,
            profile: None,
        }
    }

    #[test]
    fn single_window_is_leaf() {
        let w = make_entry("firefox", "1", 0, 0, 1920, 1080);
        let refs = vec![&w];
        let bounds = bounding_rect(&refs).unwrap();
        let indexed = extract_indexed(&refs, &[0]).unwrap();
        let tree = infer_bsp(&indexed, bounds).unwrap();
        assert!(matches!(tree, BspNode::Leaf { .. }));
    }

    #[test]
    fn two_windows_horizontal_split() {
        let a = make_entry("firefox", "1", 0, 0, 960, 1080);
        let b = make_entry("code", "1", 960, 0, 960, 1080);
        let refs = vec![&a, &b];
        let bounds = bounding_rect(&refs).unwrap();
        let indexed = extract_indexed(&refs, &[0, 1]).unwrap();
        let tree = infer_bsp(&indexed, bounds).unwrap();
        match &tree {
            BspNode::Split { dir, ratio, .. } => {
                assert_eq!(*dir, SplitDir::Horizontal);
                assert!((ratio - 0.5).abs() < 0.01);
            }
            BspNode::Leaf { .. } => panic!("expected split"),
        }
    }

    #[test]
    fn two_windows_vertical_split() {
        let a = make_entry("firefox", "1", 0, 0, 1920, 540);
        let b = make_entry("code", "1", 0, 540, 1920, 540);
        let refs = vec![&a, &b];
        let bounds = bounding_rect(&refs).unwrap();
        let indexed = extract_indexed(&refs, &[0, 1]).unwrap();
        let tree = infer_bsp(&indexed, bounds).unwrap();
        match &tree {
            BspNode::Split { dir, ratio, .. } => {
                assert_eq!(*dir, SplitDir::Vertical);
                assert!((ratio - 0.5).abs() < 0.01);
            }
            BspNode::Leaf { .. } => panic!("expected split"),
        }
    }

    #[test]
    fn three_windows_nested() {
        //  +-------+-------+
        //  |       |   B   |
        //  |   A   +-------+
        //  |       |   C   |
        //  +-------+-------+
        let a = make_entry("a", "1", 0, 0, 960, 1080);
        let b = make_entry("b", "1", 960, 0, 960, 540);
        let c = make_entry("c", "1", 960, 540, 960, 540);
        let refs = vec![&a, &b, &c];
        let bounds = bounding_rect(&refs).unwrap();
        let indexed = extract_indexed(&refs, &[0, 1, 2]).unwrap();
        let tree = infer_bsp(&indexed, bounds).unwrap();
        match &tree {
            BspNode::Split {
                dir: SplitDir::Horizontal,
                first,
                second,
                ..
            } => {
                assert!(matches!(first.as_ref(), BspNode::Leaf { .. }));
                assert!(matches!(
                    second.as_ref(),
                    BspNode::Split {
                        dir: SplitDir::Vertical,
                        ..
                    }
                ));
            }
            other => panic!("expected horizontal split at root, got {other:?}"),
        }
    }

    #[test]
    fn plan_right_heavy_tree() {
        // A (left) | B (top-right) / C (bottom-right)
        let a = make_entry("a", "1", 0, 0, 960, 1080);
        let b = make_entry("b", "1", 960, 0, 960, 540);
        let c = make_entry("c", "1", 960, 540, 960, 540);
        let refs: Vec<&WindowEntry> = vec![&a, &b, &c];
        let wp = build_workspace_plan(&refs, &[0, 1, 2]).unwrap();
        assert_eq!(wp.steps.len(), 3);

        // Order: A (full), B (right of A → root H split), C (below B → nested V split)
        assert_eq!(wp.steps[0].window_idx, 0);
        assert!(wp.steps[0].focus_idx.is_none());
        assert_eq!(wp.steps[1].window_idx, 1);
        assert_eq!(wp.steps[1].focus_idx, Some(0));
        assert!(matches!(wp.steps[1].preselect, Some(PreselDir::Right)));
        assert_eq!(wp.steps[2].window_idx, 2);
        assert_eq!(wp.steps[2].focus_idx, Some(1));
        assert!(matches!(wp.steps[2].preselect, Some(PreselDir::Bottom)));
    }

    #[test]
    fn plan_left_heavy_tree() {
        // A (top-left) / B (bottom-left) | C (right, full height)
        let a = make_entry("a", "1", 0, 0, 400, 540);
        let b = make_entry("b", "1", 0, 540, 400, 540);
        let c = make_entry("c", "1", 400, 0, 560, 1080);
        let refs: Vec<&WindowEntry> = vec![&a, &b, &c];
        let wp = build_workspace_plan(&refs, &[0, 1, 2]).unwrap();
        assert_eq!(wp.steps.len(), 3);

        // Order: A (full), C (right of A → root H split), B (below A → nested V split)
        assert_eq!(wp.steps[0].window_idx, 0);
        assert_eq!(wp.steps[1].window_idx, 2);
        assert!(matches!(wp.steps[1].preselect, Some(PreselDir::Right)));
        assert_eq!(wp.steps[2].window_idx, 1);
        assert!(matches!(wp.steps[2].preselect, Some(PreselDir::Bottom)));
    }

    #[test]
    fn two_windows_with_gap_horizontal() {
        // 10px gap between windows (simulating gaps_in = 5)
        let a = make_entry("a", "1", 0, 0, 955, 1080);
        let b = make_entry("b", "1", 965, 0, 955, 1080);
        let refs = vec![&a, &b];
        let plan = build_workspace_plan(&refs, &[0, 1]);
        assert!(plan.is_some(), "should handle gaps between windows");
        assert_eq!(plan.unwrap().steps.len(), 2);
    }

    #[test]
    fn three_windows_with_gaps() {
        //  +-------+  gap  +-------+
        //  |       |       |   B   |
        //  |   A   |  gap  +--gap--+
        //  |       |       |   C   |
        //  +-------+       +-------+
        let a = make_entry("a", "1", 5, 5, 950, 1070);
        let b = make_entry("b", "1", 965, 5, 950, 530);
        let c = make_entry("c", "1", 965, 545, 950, 530);
        let refs = vec![&a, &b, &c];
        let plan = build_workspace_plan(&refs, &[0, 1, 2]);
        assert!(plan.is_some(), "should handle gaps in nested splits");
        assert_eq!(plan.unwrap().steps.len(), 3);
    }

    #[test]
    fn multi_monitor_offset_handled() {
        // Windows on a second monitor at x=1920 with gaps
        let a = make_entry("a", "2", 1925, 5, 950, 1070);
        let b = make_entry("b", "2", 2885, 5, 950, 1070);
        let refs = vec![&a, &b];
        let plan = build_workspace_plan(&refs, &[0, 1]);
        assert!(plan.is_some(), "should handle windows on offset monitor");
        assert_eq!(plan.unwrap().steps.len(), 2);
    }

    #[test]
    fn uneven_ratio() {
        let a = make_entry("a", "1", 0, 0, 1152, 1080);
        let b = make_entry("b", "1", 1152, 0, 768, 1080);
        let refs = vec![&a, &b];
        let bounds = bounding_rect(&refs).unwrap();
        let indexed = extract_indexed(&refs, &[0, 1]).unwrap();
        let tree = infer_bsp(&indexed, bounds).unwrap();
        match &tree {
            BspNode::Split { ratio, .. } => assert!((ratio - 0.6).abs() < 0.01),
            BspNode::Leaf { .. } => panic!("expected split"),
        }
    }

    #[test]
    fn ratio_steps_two_windows() {
        let a = make_entry("a", "1", 0, 0, 960, 1080);
        let b = make_entry("b", "1", 960, 0, 960, 1080);
        let refs = vec![&a, &b];
        let wp = build_workspace_plan(&refs, &[0, 1]).unwrap();
        assert_eq!(wp.ratio_steps.len(), 1);
        assert_eq!(wp.ratio_steps[0].focus_window_idx, 0);
        assert!((wp.ratio_steps[0].ratio - 0.5).abs() < 0.01);
    }

    #[test]
    fn ratio_steps_three_windows_right_heavy() {
        let a = make_entry("a", "1", 0, 0, 960, 1080);
        let b = make_entry("b", "1", 960, 0, 960, 540);
        let c = make_entry("c", "1", 960, 540, 960, 540);
        let refs = vec![&a, &b, &c];
        let wp = build_workspace_plan(&refs, &[0, 1, 2]).unwrap();
        assert_eq!(wp.ratio_steps.len(), 2);
    }

    #[test]
    fn ratio_steps_uneven_ratio_preserved() {
        let a = make_entry("a", "1", 0, 0, 1152, 1080);
        let b = make_entry("b", "1", 1152, 0, 768, 1080);
        let refs = vec![&a, &b];
        let wp = build_workspace_plan(&refs, &[0, 1]).unwrap();
        assert_eq!(wp.ratio_steps.len(), 1);
        assert!((wp.ratio_steps[0].ratio - 0.6).abs() < 0.01);
    }

    #[test]
    fn four_windows_nested_with_large_gap_between_non_adjacent() {
        //  +--------+----------+------+
        //  |        |          |  c   |
        //  |   a    |    d     |      |
        //  |        |          +------+
        //  |        |          |  b   |
        //  +--------+----------+------+
        let a = make_entry("a", "2", 21, 69, 1112, 1350);
        let b = make_entry("b", "2", 2062, 1124, 477, 295);
        let c = make_entry("c", "2", 2062, 69, 477, 1033);
        let d = make_entry("d", "2", 1155, 69, 885, 1350);
        let refs: Vec<&WindowEntry> = vec![&a, &b, &c, &d];
        let plan = build_workspace_plan(&refs, &[0, 1, 2, 3]);
        assert!(
            plan.is_some(),
            "BSP inference should succeed for 4-window dwindle layout with gaps"
        );
        assert_eq!(plan.unwrap().steps.len(), 4);
    }

    /// Real-world 2x2 grid geometry (a live workspace: two terminals
    /// stacked in a left column, two other apps stacked in a right
    /// column), array order [a0(top), a1(bottom), discord, slack] with
    /// global indices [0, 1, 2, 3].
    ///
    ///  +--------------+------------+
    ///  |    a0(top)   |  discord   |
    ///  +--------------+------------+
    ///  |  a1(bottom)  |   slack    |
    ///  +--------------+------------+
    #[test]
    fn real_world_four_window_2x2_grid() {
        let a0 = make_entry("Alacritty", "4", 2570, 45, 1464, 602);
        let a1 = make_entry("Alacritty", "4", 2570, 657, 1464, 773);
        let discord = make_entry("discord", "4", 4044, 45, 1066, 688);
        let slack = make_entry("slack", "4", 4044, 743, 1066, 687);
        let refs: Vec<&WindowEntry> = vec![&a0, &a1, &discord, &slack];

        let plan = build_workspace_plan(&refs, &[0, 1, 2, 3])
            .expect("BSP inference should succeed for a clean 2x2 grid");
        assert_eq!(plan.steps.len(), 4);

        // Root split is horizontal (left column vs. right column): a0 opens
        // first with no anchor, then discord splits off to its right.
        assert_eq!(plan.steps[0].window_idx, 0, "a0 opens first, no anchor");
        assert_eq!(plan.steps[0].focus_idx, None);
        assert!(plan.steps[0].preselect.is_none());

        assert_eq!(plan.steps[1].window_idx, 2, "discord splits off a0");
        assert_eq!(plan.steps[1].focus_idx, Some(0));
        assert!(matches!(plan.steps[1].preselect, Some(PreselDir::Right)));

        // Left column fills in: a1 splits below a0.
        assert_eq!(plan.steps[2].window_idx, 1, "a1 splits below a0");
        assert_eq!(plan.steps[2].focus_idx, Some(0));
        assert!(matches!(plan.steps[2].preselect, Some(PreselDir::Bottom)));

        // Right column fills in: slack splits below discord.
        assert_eq!(plan.steps[3].window_idx, 3, "slack splits below discord");
        assert_eq!(plan.steps[3].focus_idx, Some(2));
        assert!(matches!(plan.steps[3].preselect, Some(PreselDir::Bottom)));
    }

    /// Same 2x2 grid geometry as `real_world_four_window_2x2_grid`, but
    /// using the array order actually found in a live `last.toml` session
    /// file (slack, discord, then the two Alacrittys) with global indices
    /// [0, 1, 2, 3] mapping to [slack, discord, a1, a0]. BSP inference is
    /// purely geometric (`split_candidates` sorts/dedups edge coordinates
    /// independent of input order), so the tree *shape* should be identical
    /// to the other ordering — only the global-index numbers attached to
    /// each leaf differ, following the remapping.
    #[test]
    fn real_world_four_window_2x2_grid_matches_live_session_order() {
        let slack = make_entry("slack", "4", 4044, 743, 1066, 687);
        let discord = make_entry("discord", "4", 4044, 45, 1066, 688);
        let a1 = make_entry("Alacritty", "4", 2570, 657, 1464, 773);
        let a0 = make_entry("Alacritty", "4", 2570, 45, 1464, 602);
        let refs: Vec<&WindowEntry> = vec![&slack, &discord, &a1, &a0];
        // global indices: slack=0, discord=1, a1=2, a0=3
        let plan = build_workspace_plan(&refs, &[0, 1, 2, 3])
            .expect("BSP inference should succeed regardless of array order");
        assert_eq!(plan.steps.len(), 4);

        assert_eq!(plan.steps[0].window_idx, 3, "a0 opens first, no anchor");
        assert_eq!(plan.steps[0].focus_idx, None);
        assert!(plan.steps[0].preselect.is_none());

        assert_eq!(plan.steps[1].window_idx, 1, "discord splits off a0");
        assert_eq!(plan.steps[1].focus_idx, Some(3));
        assert!(matches!(plan.steps[1].preselect, Some(PreselDir::Right)));

        assert_eq!(plan.steps[2].window_idx, 2, "a1 splits below a0");
        assert_eq!(plan.steps[2].focus_idx, Some(3));
        assert!(matches!(plan.steps[2].preselect, Some(PreselDir::Bottom)));

        assert_eq!(plan.steps[3].window_idx, 0, "slack splits below discord");
        assert_eq!(plan.steps[3].focus_idx, Some(1));
        assert!(matches!(plan.steps[3].preselect, Some(PreselDir::Bottom)));
    }

    #[test]
    fn pixel_ratio_steps_two_windows_has_none() {
        // The lone split has two direct leaf children, so it's fully
        // reachable via splitratio: no pixel_ratio_steps expected.
        let a = make_entry("a", "1", 0, 0, 960, 1080);
        let b = make_entry("b", "1", 960, 0, 960, 1080);
        let refs = vec![&a, &b];
        let wp = build_workspace_plan(&refs, &[0, 1]).unwrap();
        assert!(wp.pixel_ratio_steps.is_empty());
    }

    #[test]
    fn pixel_ratio_steps_three_windows_right_heavy_has_none() {
        // Root split has one direct leaf child (a); the other (b/c) is a
        // Split, but that's still reachable by focusing a or b/c's leaf.
        let a = make_entry("a", "1", 0, 0, 960, 1080);
        let b = make_entry("b", "1", 960, 0, 960, 540);
        let c = make_entry("c", "1", 960, 540, 960, 540);
        let refs = vec![&a, &b, &c];
        let wp = build_workspace_plan(&refs, &[0, 1, 2]).unwrap();
        assert!(wp.pixel_ratio_steps.is_empty());
    }

    /// The 2x2 grid's root split has two Split children (each column is
    /// itself a nested vertical split of two leaves) — neither is a direct
    /// leaf, so this is exactly the split `collect_splitratio_steps` cannot
    /// reach and that `apply_split_ratios` silently drops. This is the real-
    /// reboot failure shape: 4 windows, 2 ratio_steps (the two column
    /// splits), and one root split ratio previously left entirely
    /// uncorrected — the flat per-leaf pixel-delta convergence that used to
    /// run afterward had no way to express "the root split governing your
    /// whole column is wrong," only "you personally are the wrong size."
    #[test]
    fn pixel_ratio_steps_capture_2x2_grid_root_split() {
        let a0 = make_entry("Alacritty", "4", 2570, 45, 1464, 602);
        let a1 = make_entry("Alacritty", "4", 2570, 657, 1464, 773);
        let discord = make_entry("discord", "4", 4044, 45, 1066, 688);
        let slack = make_entry("slack", "4", 4044, 743, 1066, 687);
        let refs: Vec<&WindowEntry> = vec![&a0, &a1, &discord, &slack];

        let wp = build_workspace_plan(&refs, &[0, 1, 2, 3]).unwrap();

        assert_eq!(
            wp.ratio_steps.len(),
            2,
            "only the two column splits are directly reachable via splitratio"
        );
        assert_eq!(
            wp.pixel_ratio_steps.len(),
            1,
            "the root split (left column vs. right column) has no direct \
             leaf child on either side and must surface as a \
             pixel_ratio_steps entry"
        );

        let step = &wp.pixel_ratio_steps[0];
        assert_eq!(step.dir, SplitDir::Horizontal, "columns split left/right");
        assert!(
            (step.ratio - 0.578).abs() < 0.01,
            "left column (1464px) vs. right column (1066px) of a 2540px \
             total span, got ratio {}",
            step.ratio
        );
        assert_eq!(
            step.first_leaf_idx, 0,
            "a0, leftmost leaf of the left column"
        );
        assert_eq!(
            step.second_leaf_idx, 2,
            "discord, leftmost leaf of the right column"
        );
    }
}
