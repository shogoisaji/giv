//! Legacy three-pane dashboard helper: left column split vertically into
//! Changes (top) + Graph (bottom), right column = Diff (full height).
//!
//! The current `ui::layout::pane_layout` policy keeps every mode in two panes,
//! so this renderer is only used if that policy opts into `ThreePane` again.
//!
//! ```text
//! +----------+---------+
//! | Changes  |         |
//! |  (top)   |  Diff   |
//! +----------+ (right) |
//! | Graph    |         |
//! | (bottom) |         |
//! +----------+---------+
//! ```
//!
//! Pane → panel mapping:
//!   - Left   = Changes  (top-left,  `render_file_list`)
//!   - Middle = Graph    (bottom-left, `render_graph_list`)
//!   - Right  = Diff     (right full-height, `render_diff_panel`)
//!
//! The diff pane reflects whichever list panel currently holds focus: the graph
//! commit diff when the graph is focused, the working-tree file diff when the
//! change list is focused. `reload_selected_diff` (in `core::update`) dispatches
//! by focused semantic target, so the model layer keeps the diff pane in sync
//! without any dashboard-specific wiring.
//!
//! **Focus-weighted split**: the focused pane gets more space — when Changes or
//! Graph is focused the left column widens to 60% and the focused half grows;
//! when Diff is focused the right column widens to 60%. Tabbing shifts the
//! weight smoothly — the pane structure never changes (no reflow).

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

use crate::app::{App, Panel};
use crate::ui::layout::dashboard_splits;

/// Render the legacy three-pane dashboard into `area`.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let focused = *app.ui.panel();
    let splits = dashboard_splits(focused);

    // Split area into left column (Changes+Graph) and right column (Diff).
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(splits.left_col_pct),
            Constraint::Percentage(100 - splits.left_col_pct),
        ])
        .split(area);

    // Split left column vertically into Changes (top) + Graph (bottom).
    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(splits.top_row_pct),
            Constraint::Percentage(100 - splits.top_row_pct),
        ])
        .split(cols[0]);

    crate::features::status::view::render_file_list(
        frame,
        left_rows[0],
        app,
        focused == Panel::Left,
    );
    crate::features::graph::view::render_graph_list(
        frame,
        left_rows[1],
        app,
        focused == Panel::Middle,
    );
    crate::features::status::view::render_diff_panel(frame, cols[1], app, focused == Panel::Right);
}
