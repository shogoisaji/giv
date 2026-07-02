/// Graph lane cell-matrix → ratatui `Line<'static>` renderer.
///
/// `render_rows` converts each `GraphRow` into a ratatui `Line<'static>` with
/// per-lane colours. The graph cells (including the inter-lane connectors) are
/// produced by `build_graph` and rendered 1:1 here. Node rows additionally get
/// metadata: short id, summary, ref badges, and a relative-age column.
use std::collections::HashMap;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::layout::{GraphCell, GraphRow, Highlight};
use crate::git::types::{Commit, RefKind};
use crate::theme::Theme;

/// Per-branch divergence vs its upstream: `name -> (ahead, behind)`.
pub type Divergence = HashMap<String, (usize, usize)>;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Convert graph rows + the originating commits into ratatui lines.
///
/// `now` is the current Unix time (seconds) used to compute each commit's
/// relative age (e.g. `3d`, `2w`).
#[allow(clippy::too_many_arguments)]
pub fn render_rows(
    rows: &[GraphRow],
    commits: &[Commit],
    theme: &Theme,
    now: i64,
    divergence: &Divergence,
    hl: Option<&Highlight>,
    fork: Option<&str>,
    main_ancestors: Option<&std::collections::HashSet<String>>,
) -> Vec<Line<'static>> {
    // The selected branch is drawn as ONE consistent colour: its own lane colour.
    // Without this, boundary nodes (fork/merge) and crossings the branch wins over
    // render in the OTHER lane's colour, so the branch looks multi-coloured.
    let branch_color = hl.and_then(|h| branch_color_index(rows, h));
    let lane_tip = hl.and_then(|h| super::layout::selected_lane_tip(h, commits));
    rows.iter()
        .map(|row| {
            render_one_row(
                row,
                commits,
                theme,
                now,
                divergence,
                hl,
                fork,
                branch_color,
                main_ancestors,
                lane_tip,
            )
        })
        .collect()
}

/// The lane colour index of the highlighted branch: the colour of any of its
/// INTERIOR node cells (they all share one continuous lane, hence one colour).
pub fn branch_color_index(rows: &[GraphRow], hl: &Highlight) -> Option<usize> {
    rows.iter()
        .flat_map(|r| &r.cells)
        .find(|c| {
            matches!(c.symbol, '●' | '◉' | '◆')
                && c.vertical_oid
                    .as_deref()
                    .map(|o| hl.lanes.contains(o))
                    .unwrap_or(false)
        })
        .map(|c| c.lane)
}

/// Render only the rows in `all_rows[start..end]` (clamped to bounds). The
/// branch colour is still resolved over the FULL row set so it stays consistent
/// regardless of which slice is visible — this is the perf-critical path: only
/// the visible viewport is converted to `Line`s instead of the whole history.
#[allow(clippy::too_many_arguments)]
pub fn render_rows_window(
    all_rows: &[GraphRow],
    start: usize,
    end: usize,
    commits: &[Commit],
    theme: &Theme,
    now: i64,
    divergence: &Divergence,
    hl: Option<&Highlight>,
    fork: Option<&str>,
    main_ancestors: Option<&std::collections::HashSet<String>>,
) -> Vec<Line<'static>> {
    let branch_color = hl.and_then(|h| branch_color_index(all_rows, h));
    let lane_tip = hl.and_then(|h| super::layout::selected_lane_tip(h, commits));
    let lo = start.min(all_rows.len());
    let hi = end.min(all_rows.len()).max(lo);
    all_rows[lo..hi]
        .iter()
        .map(|row| {
            render_one_row(
                row,
                commits,
                theme,
                now,
                divergence,
                hl,
                fork,
                branch_color,
                main_ancestors,
                lane_tip,
            )
        })
        .collect()
}

// ─── Per-row rendering ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_one_row(
    row: &GraphRow,
    commits: &[Commit],
    theme: &Theme,
    now: i64,
    divergence: &Divergence,
    hl: Option<&Highlight>,
    fork: Option<&str>,
    branch_color: Option<usize>,
    main_ancestors: Option<&std::collections::HashSet<String>>,
    lane_tip: Option<usize>,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();

    // Left margin so the graph doesn't sit flush against the panel border.
    spans.push(Span::raw("  "));

    // ── Graph cells. With a selection lineage active, each cell's glyph is
    // resolved selection-aware: the selected branch keeps priority (unrelated
    // crossings are dropped) and off-branch cells are dimmed. ─────────────────
    for cell in &row.cells {
        if cell.symbol == ' ' {
            spans.push(Span::raw(" "));
        } else {
            let (glyph, dim) = super::layout::cell_glyph(cell, hl);
            // The selected branch's line is drawn BOLD + ONE colour (the branch's
            // own lane colour) so it reads as a single foreground line; everything
            // else recedes (dim). Using `branch_color` for every lit cell keeps the
            // fork/merge boundary nodes and won crossings the SAME colour as the
            // branch instead of the other lane's colour.
            let color = branch_color
                .map(|i| lane_color_for_index(i, theme))
                .unwrap_or_else(|| lane_color(cell, theme));
            let style = if dim {
                // Stronger dim contrast: the terminal's DIM modifier on top of
                // the dim colour pushes off-branch cells further back so the
                // selected branch's bold line pops more.
                Style::default().fg(theme.dim).add_modifier(Modifier::DIM)
            } else if hl.is_some() {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(lane_color(cell, theme))
            };
            spans.push(Span::styled(glyph.to_string(), style));
        }
    }

    // ── Metadata (node rows only) ─────────────────────────────────────────────
    if row.is_node_row {
        if let Some(commit) = commits.get(row.commit_index) {
            // Dim the metadata of commits outside the selected branch.
            let meta_dim = hl.map(|h| !h.nodes.contains(&commit.id)).unwrap_or(false);
            // Is this row the tip of the selected lane? Used to render a branch
            // label that marks "this is where the selected branch starts".
            let is_lane_tip = lane_tip == Some(row.commit_index);

            let node_lane_color = match branch_color {
                // Lit node (on the selected branch) → the branch's single colour.
                Some(i) if !meta_dim => lane_color_for_index(i, theme),
                _ => row
                    .cells
                    .iter()
                    .find(|c| matches!(c.symbol, '●' | '◉' | '◆'))
                    .map(|c| lane_color(c, theme))
                    .unwrap_or_else(|| lane_color_for_index(0, theme)),
            };

            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                commit.short_id.clone(),
                Style::default().fg(theme.dim),
            ));
            spans.push(Span::raw("  "));

            // Branch label on the selected lane's tip: marks the newest commit of
            // the highlighted branch so the user can see where the selected branch
            // starts. Drawn in the branch's own lane colour + bold so it ties to
            // the lit lane line. Only shown when a highlight is active and this
            // row is the lane tip (not the fork — the fork gets its own ⑂base mark).
            if is_lane_tip && fork != Some(commit.id.as_str()) {
                spans.push(Span::styled(
                    "◀tip ".to_string(),
                    Style::default()
                        .fg(node_lane_color)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            // Branch lens: mark the fork point (where this branch left main).
            if fork == Some(commit.id.as_str()) {
                spans.push(Span::styled(
                    "⑂base ".to_string(),
                    Style::default()
                        .fg(theme.focus_border)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            // A local branch whose tip is NOT in main's ancestry has commits not
            // yet merged into main — work in progress, away from the mainline.
            let unmerged_into_main = main_ancestors
                .map(|a| !a.contains(&commit.id))
                .unwrap_or(false);

            // Ref badges come before the summary so branch/tag context leads.
            for r in &commit.refs {
                spans.push(ref_badge(r, node_lane_color, theme, meta_dim));
                if matches!(r.kind, RefKind::LocalBranch | RefKind::Head) {
                    // `↟` = this branch sits off main with unmerged work. Drawn in
                    // the warm head/amber accent so it reads as "not on main yet".
                    if unmerged_into_main && !meta_dim {
                        spans.push(Span::styled(
                            "↟".to_string(),
                            Style::default().fg(theme.head).add_modifier(Modifier::BOLD),
                        ));
                    }
                    // Append the branch's divergence vs upstream (↑ahead ↓behind).
                    // Includes the checked-out branch, whose ref is `HEAD→<name>`
                    // (RefKind::Head) — that's the branch the user most wants this on.
                    for span in divergence_spans(divergence.get(&r.name), theme, meta_dim) {
                        spans.push(span);
                    }
                }
                spans.push(Span::raw(" "));
            }

            let summary_color = if meta_dim { theme.dim } else { theme.fg };
            spans.push(Span::styled(
                commit.summary.clone(),
                Style::default().fg(summary_color),
            ));

            // Relative age — dim, appended at the end of the row.
            spans.push(Span::styled(
                format!("  {}", relative_age(commit.time, now)),
                Style::default().fg(theme.dim),
            ));
        }
    }

    Line::from(spans)
}

/// Build a styled badge span for a ref decoration. `dim` de-emphasizes the badge
/// when the commit is outside the selected lineage.
fn ref_badge(
    r: &crate::git::types::RefName,
    node_lane_color: Color,
    theme: &Theme,
    dim: bool,
) -> Span<'static> {
    if dim {
        let text = match r.kind {
            RefKind::Head => format!("HEAD→{}", r.name),
            RefKind::LocalBranch => format!("[{}]", r.name),
            RefKind::RemoteBranch => format!("({})", r.name),
            RefKind::Tag => format!("◆{}", r.name),
        };
        return Span::styled(text, Style::default().fg(theme.dim));
    }
    match r.kind {
        RefKind::Head => Span::styled(
            format!("HEAD→{}", r.name),
            Style::default().fg(theme.head).add_modifier(Modifier::BOLD),
        ),
        RefKind::LocalBranch => Span::styled(
            format!("[{}]", r.name),
            Style::default()
                .fg(node_lane_color)
                .add_modifier(Modifier::BOLD),
        ),
        RefKind::RemoteBranch => {
            Span::styled(format!("({})", r.name), Style::default().fg(theme.dim))
        }
        RefKind::Tag => Span::styled(format!("◆{}", r.name), Style::default().fg(theme.unstaged)),
    }
}

/// Build divergence indicator spans for a local branch: `↑{ahead}` (unpushed)
/// and/or `↓{behind}` (upstream is ahead → pull / rebase needed). Returns an
/// empty vec when the branch is in sync or has no upstream.
fn divergence_spans(div: Option<&(usize, usize)>, theme: &Theme, dim: bool) -> Vec<Span<'static>> {
    let Some(&(ahead, behind)) = div else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    if ahead > 0 {
        let c = if dim { theme.dim } else { theme.added };
        spans.push(Span::styled(format!("↑{ahead}"), Style::default().fg(c)));
    }
    if behind > 0 {
        let style = if dim {
            Style::default().fg(theme.dim)
        } else {
            Style::default()
                .fg(theme.removed)
                .add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(format!("↓{behind}"), style));
    }
    spans
}

// ─── Relative age ──────────────────────────────────────────────────────────────

/// Format a commit's age as a short relative string: `now`, `5m`, `3h`, `2d`,
/// `4w`, `6mo`, `2y`. Future / zero timestamps render as `now`.
pub fn relative_age(commit_time: i64, now: i64) -> String {
    let secs = now.saturating_sub(commit_time);
    if secs <= 0 {
        return "now".to_string();
    }
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;
    if secs < 60 {
        "now".to_string()
    } else if mins < 60 {
        format!("{mins}m")
    } else if hours < 24 {
        format!("{hours}h")
    } else if days < 7 {
        format!("{days}d")
    } else if days < 30 {
        format!("{}w", days / 7)
    } else if days < 365 {
        format!("{}mo", days / 30)
    } else {
        format!("{}y", days / 365)
    }
}

// ─── Color helpers ────────────────────────────────────────────────────────────

fn lane_color(cell: &GraphCell, theme: &Theme) -> Color {
    lane_color_for_index(cell.lane, theme)
}

fn lane_color_for_index(lane: usize, theme: &Theme) -> Color {
    if theme.lane.is_empty() {
        Color::White
    } else {
        theme.lane[lane % theme.lane.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_age() {
        let now = 1_000_000_000;
        assert_eq!(relative_age(now, now), "now");
        assert_eq!(relative_age(now - 30, now), "now");
        assert_eq!(relative_age(now - 120, now), "2m");
        assert_eq!(relative_age(now - 3 * 3600, now), "3h");
        assert_eq!(relative_age(now - 2 * 86400, now), "2d");
        assert_eq!(relative_age(now - 14 * 86400, now), "2w");
        assert_eq!(relative_age(now - 60 * 86400, now), "2mo");
        assert_eq!(relative_age(now - 800 * 86400, now), "2y");
        // Future timestamp is clamped to "now".
        assert_eq!(relative_age(now + 500, now), "now");
    }

    // ── Windowed rendering ──────────────────────────────────────────────────

    fn three_linear_commits() -> Vec<Commit> {
        vec![
            Commit {
                id: "c3".into(),
                short_id: "c3".into(),
                parents: vec!["c2".into()],
                summary: "third".into(),
                body: String::new(),
                author_name: "T".into(),
                author_email: "t@e".into(),
                time: 0,
                refs: Vec::new(),
            },
            Commit {
                id: "c2".into(),
                short_id: "c2".into(),
                parents: vec!["c1".into()],
                summary: "second".into(),
                body: String::new(),
                author_name: "T".into(),
                author_email: "t@e".into(),
                time: 0,
                refs: Vec::new(),
            },
            Commit {
                id: "c1".into(),
                short_id: "c1".into(),
                parents: vec![],
                summary: "first".into(),
                body: String::new(),
                author_name: "T".into(),
                author_email: "t@e".into(),
                time: 0,
                refs: Vec::new(),
            },
        ]
    }

    #[test]
    fn render_rows_window_returns_only_the_visible_slice() {
        let commits = three_linear_commits();
        let rows = super::super::layout::build_graph(&commits, false);
        let theme = Theme::tokyonight();
        let div: Divergence = std::collections::HashMap::new();

        // Full render baseline.
        let full = render_rows(&rows, &commits, &theme, 0, &div, None, None, None);
        assert_eq!(full.len(), rows.len());

        // Window covering only the middle row.
        let win = render_rows_window(&rows, 1, 2, &commits, &theme, 0, &div, None, None, None);
        assert_eq!(win.len(), 1);
        // The windowed line must equal the corresponding full line.
        assert_eq!(win[0], full[1]);
    }

    #[test]
    fn render_rows_window_clamps_to_bounds() {
        let commits = three_linear_commits();
        let rows = super::super::layout::build_graph(&commits, false);
        let theme = Theme::tokyonight();
        let div: Divergence = std::collections::HashMap::new();

        // Start past the end → empty.
        let win = render_rows_window(&rows, 100, 200, &commits, &theme, 0, &div, None, None, None);
        assert!(win.is_empty());

        // End past the end → clamped to rows.len().
        let win = render_rows_window(&rows, 0, 100, &commits, &theme, 0, &div, None, None, None);
        assert_eq!(win.len(), rows.len());
    }

    // ── Selection branch emphasis ────────────────────────────────────────────

    /// Off-branch cells get the DIM modifier (stronger dim contrast) when a
    /// highlight is active. We verify by checking that a dimmed cell's style
    /// includes `Modifier::DIM`.
    #[test]
    fn off_branch_cells_get_dim_modifier_when_highlighted() {
        // Two-branch graph: main (c1←c2←c3) and feature (c1←f2←f3).
        let commits = vec![
            Commit {
                id: "f3".into(),
                short_id: "f3".into(),
                parents: vec!["f2".into()],
                summary: "feat3".into(),
                body: String::new(),
                author_name: "T".into(),
                author_email: "t@e".into(),
                time: 0,
                refs: Vec::new(),
            },
            Commit {
                id: "f2".into(),
                short_id: "f2".into(),
                parents: vec!["c1".into()],
                summary: "feat2".into(),
                body: String::new(),
                author_name: "T".into(),
                author_email: "t@e".into(),
                time: 0,
                refs: Vec::new(),
            },
            Commit {
                id: "c3".into(),
                short_id: "c3".into(),
                parents: vec!["c2".into()],
                summary: "main3".into(),
                body: String::new(),
                author_name: "T".into(),
                author_email: "t@e".into(),
                time: 0,
                refs: Vec::new(),
            },
            Commit {
                id: "c2".into(),
                short_id: "c2".into(),
                parents: vec!["c1".into()],
                summary: "main2".into(),
                body: String::new(),
                author_name: "T".into(),
                author_email: "t@e".into(),
                time: 0,
                refs: Vec::new(),
            },
            Commit {
                id: "c1".into(),
                short_id: "c1".into(),
                parents: vec![],
                summary: "root".into(),
                body: String::new(),
                author_name: "T".into(),
                author_email: "t@e".into(),
                time: 0,
                refs: Vec::new(),
            },
        ];
        let rows = super::super::layout::build_graph(&commits, false);
        let theme = Theme::tokyonight();
        let div: Divergence = std::collections::HashMap::new();

        // Select the feature branch tip (index 0 = f3).
        let hl = super::super::layout::branch_highlight(&commits, 0, false);
        let lines = render_rows(&rows, &commits, &theme, 0, &div, Some(&hl), None, None);

        // Find a row that has a dimmed cell (an off-branch cell). The main
        // branch's node row (c3 or c2) should have dimmed connector cells.
        let main_row_line = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("main3")))
            .expect("main3 row must exist");
        let has_dim = main_row_line
            .spans
            .iter()
            .any(|s| s.style.add_modifier == Modifier::DIM);
        assert!(has_dim, "off-branch cells must carry the DIM modifier");
    }

    /// The selected lane's tip row gets a `◀tip` label when a highlight is
    /// active. The label must NOT appear on non-tip rows.
    #[test]
    fn lane_tip_label_appears_only_on_selected_lane_tip() {
        let commits = three_linear_commits();
        let rows = super::super::layout::build_graph(&commits, false);
        let theme = Theme::tokyonight();
        let div: Divergence = std::collections::HashMap::new();

        // Select the newest commit (index 0 = c3) → it is the lane tip.
        let hl = super::super::layout::branch_highlight(&commits, 0, false);
        let lines = render_rows(&rows, &commits, &theme, 0, &div, Some(&hl), None, None);

        // Row 0 (c3, the tip) must contain "◀tip".
        let tip_row = &lines[0];
        let tip_text: String = tip_row.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            tip_text.contains("◀tip"),
            "tip row must contain the ◀tip label: {tip_text:?}"
        );

        // Row 1 (c2, not the tip) must NOT contain "◀tip".
        let non_tip = &lines[1];
        let non_tip_text: String = non_tip.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            !non_tip_text.contains("◀tip"),
            "non-tip row must not contain ◀tip: {non_tip_text:?}"
        );
    }

    /// No highlight → no `◀tip` label anywhere.
    #[test]
    fn no_tip_label_without_highlight() {
        let commits = three_linear_commits();
        let rows = super::super::layout::build_graph(&commits, false);
        let theme = Theme::tokyonight();
        let div: Divergence = std::collections::HashMap::new();

        let lines = render_rows(&rows, &commits, &theme, 0, &div, None, None, None);
        for (i, line) in lines.iter().enumerate() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                !text.contains("◀tip"),
                "row {i} must not have ◀tip without highlight"
            );
        }
    }
}
