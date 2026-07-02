//! Responsive layout decisions.
//!
//! The terminal width determines whether the main area renders the historical
//! two-pane layout (left list + right detail) or, on wide terminals, a
//! three-pane dashboard (`Graph | Changes | Diff`) that shows the repository
//! overview without switching modes.
//!
//! All functions here are pure and side-effect-free so the breakpoint logic is
//! unit-testable without a TTY.

use crate::app::{Mode, Panel};

/// Coarse terminal-width bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutKind {
    /// Default width — two-pane layouts.
    Narrow,
    /// Wide terminal — three-pane dashboard where applicable.
    Wide,
}

impl LayoutKind {
    /// Minimum terminal width (in columns) for the wide layout to activate.
    pub const WIDE_MIN_WIDTH: u16 = 150;

    /// Classify a terminal width into a layout bucket.
    pub fn for_width(width: u16) -> Self {
        if width >= Self::WIDE_MIN_WIDTH {
            LayoutKind::Wide
        } else {
            LayoutKind::Narrow
        }
    }
}

/// The pane structure the main area should render for a given width + mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneLayout {
    /// Two panes: left list + right detail (the historical layout).
    TwoPane,
    /// Three panes: `Graph | Changes | Diff` (the wide Status/Graph dashboard).
    ThreePane,
}

/// Decide the main-area pane structure for `width` and the active `mode`.
///
/// Only Status and Graph modes grow a third pane on wide terminals — they are
/// the two modes that naturally combine a commit graph, a working-tree change
/// list, and a diff. Branches / Worktrees / Stashes / Inspect keep the
/// two-pane layout even when wide, since they have no natural third pane.
pub fn pane_layout(width: u16, mode: Mode) -> PaneLayout {
    if LayoutKind::for_width(width) == LayoutKind::Wide
        && matches!(mode, Mode::Status | Mode::Graph)
    {
        PaneLayout::ThreePane
    } else {
        PaneLayout::TwoPane
    }
}

/// Panels that exist in a given pane layout.
pub fn valid_panels(layout: PaneLayout) -> &'static [Panel] {
    match layout {
        PaneLayout::TwoPane => &[Panel::Left, Panel::Main],
        PaneLayout::ThreePane => &[Panel::Left, Panel::Middle, Panel::Right],
    }
}

/// Map `current` to a panel that actually exists in `layout`. Out-of-layout
/// panels fall back to `Left`. Used when a resize changes the layout so a
/// previously-valid focus never points at a non-existent pane.
pub fn coerce_panel(current: Panel, layout: PaneLayout) -> Panel {
    if valid_panels(layout).contains(&current) {
        current
    } else {
        Panel::Left
    }
}

/// Next panel in focus order for `layout`:
///   - TwoPane:   `Left → Main → Left`
///   - ThreePane: `Left → Middle → Right → Left`
pub fn next_panel(current: Panel, layout: PaneLayout) -> Panel {
    let panels = valid_panels(layout);
    let idx = panels.iter().position(|&p| p == current).unwrap_or(0);
    let next = (idx + 1) % panels.len();
    panels[next]
}

/// Semantic target the focused panel controls. Drives navigation and diff
/// loading so the same key (`j`/`k`) does the right thing regardless of which
/// pane holds focus in the active layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    /// The commit graph (moves `graph_index`).
    Graph,
    /// The working-tree change list (moves `list_index`).
    Changes,
    /// The diff panel (movement scrolls the diff).
    Diff,
    /// A mode-specific list with no graph/changes/diff semantics
    /// (Branches / Worktrees / Stashes).
    Other,
}

/// Resolve which semantic target `panel` controls under `layout` and `mode`.
pub fn focus_target(layout: PaneLayout, mode: Mode, panel: Panel) -> FocusTarget {
    match (layout, mode, panel) {
        // Three-pane dashboard: Left=Changes, Middle=Graph, Right=Diff.
        (PaneLayout::ThreePane, _, Panel::Left) => FocusTarget::Changes,
        (PaneLayout::ThreePane, _, Panel::Middle) => FocusTarget::Graph,
        (PaneLayout::ThreePane, _, Panel::Right) => FocusTarget::Diff,
        (PaneLayout::ThreePane, _, _) => FocusTarget::Other,

        // Two-pane Status: Left=Changes, Main=Diff.
        (PaneLayout::TwoPane, Mode::Status, Panel::Left) => FocusTarget::Changes,
        (PaneLayout::TwoPane, Mode::Status, Panel::Main) => FocusTarget::Diff,
        // Two-pane Graph: Left=Graph, Main=Diff.
        (PaneLayout::TwoPane, Mode::Graph, Panel::Left) => FocusTarget::Graph,
        (PaneLayout::TwoPane, Mode::Graph, Panel::Main) => FocusTarget::Diff,
        // Everything else (Branches/Worktrees/Stashes/Inspect) is mode-specific.
        _ => FocusTarget::Other,
    }
}

/// The panel that should receive initial focus when entering `mode` under
/// `layout` — the mode's primary list. `Esc` (FocusLeft) returns here.
pub fn primary_panel(layout: PaneLayout, mode: Mode) -> Panel {
    match (layout, mode) {
        // Three-pane: Left=Changes, Middle=Graph, Right=Diff.
        // Status primary = Changes (Left). Graph primary = Graph (Middle).
        (PaneLayout::ThreePane, Mode::Status) => Panel::Left,
        (PaneLayout::ThreePane, Mode::Graph) => Panel::Middle,
        _ => Panel::Left,
    }
}

/// The panel that renders the diff under `layout` (focus target for `Enter`).
pub fn diff_panel(layout: PaneLayout) -> Panel {
    match layout {
        PaneLayout::TwoPane => Panel::Main,
        PaneLayout::ThreePane => Panel::Right,
    }
}

/// Pane-width percentages for a focus-weighted two-pane split: the focused
/// pane gets 65%, the other 35%.
pub fn pane_ratios(layout: PaneLayout, focused: Panel) -> Vec<u16> {
    match layout {
        PaneLayout::TwoPane => match focused {
            Panel::Left => vec![65, 35],
            _ => vec![35, 65],
        },
        // Three-pane uses `dashboard_splits` instead (nested vertical+horizontal).
        PaneLayout::ThreePane => vec![40, 30, 30],
    }
}

/// Focus-weighted split dimensions for the three-pane dashboard.
///
/// Layout: left column split vertically into Changes (top) + Graph (bottom),
/// right column = Diff (full height).
///
/// ```text
/// +----------+---------+
/// | Changes  |         |
/// |  (top)   |  Diff   |
/// +----------+ (right) |
/// | Graph    |         |
/// | (bottom) |         |
/// +----------+---------+
/// ```
///
/// - `left_col_pct`: left column width %. 60% when a left-column pane is
///   focused, 40% when the diff is focused.
/// - `top_row_pct`: Changes height % within the left column. 60% when Changes
///   is focused, 40% when Graph is focused, 50% when Diff is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DashboardSplits {
    pub left_col_pct: u16,
    pub top_row_pct: u16,
}

pub fn dashboard_splits(focused: Panel) -> DashboardSplits {
    match focused {
        Panel::Left => DashboardSplits {
            left_col_pct: 60,
            top_row_pct: 60,
        }, // Changes
        Panel::Middle => DashboardSplits {
            left_col_pct: 60,
            top_row_pct: 40,
        }, // Graph
        Panel::Right => DashboardSplits {
            left_col_pct: 40,
            top_row_pct: 50,
        }, // Diff
        _ => DashboardSplits {
            left_col_pct: 50,
            top_row_pct: 50,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_below_threshold() {
        assert_eq!(LayoutKind::for_width(0), LayoutKind::Narrow);
        assert_eq!(LayoutKind::for_width(80), LayoutKind::Narrow);
        assert_eq!(LayoutKind::for_width(149), LayoutKind::Narrow);
    }

    #[test]
    fn wide_at_and_above_threshold() {
        assert_eq!(LayoutKind::for_width(150), LayoutKind::Wide);
        assert_eq!(LayoutKind::for_width(200), LayoutKind::Wide);
        assert_eq!(LayoutKind::for_width(u16::MAX), LayoutKind::Wide);
    }

    #[test]
    fn two_pane_for_non_dashboard_modes_even_when_wide() {
        let wide = 200;
        assert_eq!(pane_layout(wide, Mode::Branches), PaneLayout::TwoPane);
        assert_eq!(pane_layout(wide, Mode::Worktrees), PaneLayout::TwoPane);
        assert_eq!(pane_layout(wide, Mode::Stashes), PaneLayout::TwoPane);
        assert_eq!(pane_layout(wide, Mode::Inspect), PaneLayout::TwoPane);
    }

    #[test]
    fn three_pane_for_status_and_graph_when_wide() {
        let wide = 200;
        assert_eq!(pane_layout(wide, Mode::Status), PaneLayout::ThreePane);
        assert_eq!(pane_layout(wide, Mode::Graph), PaneLayout::ThreePane);
    }

    #[test]
    fn two_pane_for_status_and_graph_when_narrow() {
        assert_eq!(pane_layout(80, Mode::Status), PaneLayout::TwoPane);
        assert_eq!(pane_layout(149, Mode::Graph), PaneLayout::TwoPane);
    }

    #[test]
    fn threshold_is_exactly_150() {
        // Boundary: 149 is narrow, 150 is wide.
        assert_eq!(LayoutKind::for_width(149), LayoutKind::Narrow);
        assert_eq!(LayoutKind::for_width(150), LayoutKind::Wide);
    }

    // ── Focus cycling ───────────────────────────────────────────────────────

    #[test]
    fn next_panel_two_pane_cycles_left_main() {
        assert_eq!(next_panel(Panel::Left, PaneLayout::TwoPane), Panel::Main);
        assert_eq!(next_panel(Panel::Main, PaneLayout::TwoPane), Panel::Left);
    }

    #[test]
    fn next_panel_three_pane_cycles_left_middle_right() {
        assert_eq!(
            next_panel(Panel::Left, PaneLayout::ThreePane),
            Panel::Middle
        );
        assert_eq!(
            next_panel(Panel::Middle, PaneLayout::ThreePane),
            Panel::Right
        );
        assert_eq!(next_panel(Panel::Right, PaneLayout::ThreePane), Panel::Left);
    }

    #[test]
    fn coerce_panel_keeps_valid_falls_back_to_left() {
        // Valid panels are kept.
        assert_eq!(coerce_panel(Panel::Left, PaneLayout::TwoPane), Panel::Left);
        assert_eq!(coerce_panel(Panel::Main, PaneLayout::TwoPane), Panel::Main);
        assert_eq!(
            coerce_panel(Panel::Right, PaneLayout::ThreePane),
            Panel::Right
        );
        // Out-of-layout panels fall back to Left.
        assert_eq!(
            coerce_panel(Panel::Middle, PaneLayout::TwoPane),
            Panel::Left
        );
        assert_eq!(coerce_panel(Panel::Right, PaneLayout::TwoPane), Panel::Left);
        assert_eq!(
            coerce_panel(Panel::Main, PaneLayout::ThreePane),
            Panel::Left
        );
    }

    // ── FocusTarget mapping ─────────────────────────────────────────────────

    #[test]
    fn focus_target_three_pane_maps_left_changes_middle_graph_right_diff() {
        let l = PaneLayout::ThreePane;
        // Left=Changes, Middle=Graph, Right=Diff — same for both Status and Graph.
        assert_eq!(
            focus_target(l, Mode::Status, Panel::Left),
            FocusTarget::Changes
        );
        assert_eq!(
            focus_target(l, Mode::Status, Panel::Middle),
            FocusTarget::Graph
        );
        assert_eq!(
            focus_target(l, Mode::Status, Panel::Right),
            FocusTarget::Diff
        );
        assert_eq!(
            focus_target(l, Mode::Graph, Panel::Left),
            FocusTarget::Changes
        );
        assert_eq!(
            focus_target(l, Mode::Graph, Panel::Middle),
            FocusTarget::Graph
        );
        assert_eq!(
            focus_target(l, Mode::Graph, Panel::Right),
            FocusTarget::Diff
        );
    }

    #[test]
    fn focus_target_two_pane_status_left_changes_main_diff() {
        let l = PaneLayout::TwoPane;
        assert_eq!(
            focus_target(l, Mode::Status, Panel::Left),
            FocusTarget::Changes
        );
        assert_eq!(
            focus_target(l, Mode::Status, Panel::Main),
            FocusTarget::Diff
        );
    }

    #[test]
    fn focus_target_two_pane_graph_left_graph_main_diff() {
        let l = PaneLayout::TwoPane;
        assert_eq!(
            focus_target(l, Mode::Graph, Panel::Left),
            FocusTarget::Graph
        );
        assert_eq!(focus_target(l, Mode::Graph, Panel::Main), FocusTarget::Diff);
    }

    #[test]
    fn focus_target_other_modes_are_other() {
        let l = PaneLayout::TwoPane;
        assert_eq!(
            focus_target(l, Mode::Branches, Panel::Left),
            FocusTarget::Other
        );
        assert_eq!(
            focus_target(l, Mode::Worktrees, Panel::Main),
            FocusTarget::Other
        );
        assert_eq!(
            focus_target(l, Mode::Stashes, Panel::Left),
            FocusTarget::Other
        );
    }

    #[test]
    fn primary_panel_three_pane_status_left_graph_middle() {
        // Three-pane: Left=Changes, Middle=Graph. Status primary = Changes (Left).
        assert_eq!(
            primary_panel(PaneLayout::ThreePane, Mode::Status),
            Panel::Left
        );
        // Graph primary = Graph (Middle).
        assert_eq!(
            primary_panel(PaneLayout::ThreePane, Mode::Graph),
            Panel::Middle
        );
        // Two-pane always defaults to Left.
        assert_eq!(
            primary_panel(PaneLayout::TwoPane, Mode::Status),
            Panel::Left
        );
        assert_eq!(primary_panel(PaneLayout::TwoPane, Mode::Graph), Panel::Left);
    }

    #[test]
    fn diff_panel_main_in_two_pane_right_in_three() {
        assert_eq!(diff_panel(PaneLayout::TwoPane), Panel::Main);
        assert_eq!(diff_panel(PaneLayout::ThreePane), Panel::Right);
    }

    // ── Focus-weighted pane ratios ───────────────────────────────────────────

    #[test]
    fn pane_ratios_two_pane_focused_gets_65() {
        assert_eq!(pane_ratios(PaneLayout::TwoPane, Panel::Left), vec![65, 35]);
        assert_eq!(pane_ratios(PaneLayout::TwoPane, Panel::Main), vec![35, 65]);
    }

    // ── Dashboard splits (three-pane: Changes-top | Graph-bottom | Diff-right) ─

    #[test]
    fn dashboard_splits_changes_focused_gets_wide_left_tall_top() {
        let s = dashboard_splits(Panel::Left);
        assert_eq!(
            s,
            DashboardSplits {
                left_col_pct: 60,
                top_row_pct: 60
            }
        );
    }

    #[test]
    fn dashboard_splits_graph_focused_gets_wide_left_tall_bottom() {
        let s = dashboard_splits(Panel::Middle);
        assert_eq!(
            s,
            DashboardSplits {
                left_col_pct: 60,
                top_row_pct: 40
            }
        );
    }

    #[test]
    fn dashboard_splits_diff_focused_gets_wide_right_even_split() {
        let s = dashboard_splits(Panel::Right);
        assert_eq!(
            s,
            DashboardSplits {
                left_col_pct: 40,
                top_row_pct: 50
            }
        );
    }

    #[test]
    fn dashboard_splits_percentages_are_valid() {
        for &panel in &[Panel::Left, Panel::Middle, Panel::Right] {
            let s = dashboard_splits(panel);
            assert!(s.left_col_pct <= 100 && s.left_col_pct > 0);
            assert!(s.top_row_pct <= 100 && s.top_row_pct > 0);
        }
    }
}
