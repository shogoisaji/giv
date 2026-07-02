/// Graph mode: left = scrollable commit graph, right = commit detail panel.
///
/// Layout:
///   ┌─────────────────────────────────────────────┐
///   │  Graph (60%)            │  Detail (40%)       │
///   │  ● abc1234 add feature  │  Author: …          │
///   │  │ def5678 fix bug      │  Date:   …          │
///   │  …                      │  Message …          │
///   │                         │  Files changed …    │
///   └─────────────────────────────────────────────┘
///
/// Selection and scrolling are driven by `app.ui.graph_index` and
/// `app.ui.graph_offset`.  The view never panics on an empty commit list.
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
    Frame,
};

use crate::app::{App, Panel};

/// Render Graph mode.  Split `area` into a left graph list and right detail
/// panel; both panels have rounded borders and a focus highlight. The focused
/// pane gets 65% of the width (focus-weighted split).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let focused = *app.ui.panel();
    let ratios = crate::ui::layout::pane_ratios(crate::ui::layout::PaneLayout::TwoPane, focused);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            ratios
                .iter()
                .map(|&p| Constraint::Percentage(p))
                .collect::<Vec<_>>(),
        )
        .split(area);

    render_graph_list(frame, chunks[0], app, focused == Panel::Left);
    render_commit_detail(frame, chunks[1], app);
}

// ─── Left panel: commit graph ─────────────────────────────────────────────────

pub(crate) fn render_graph_list(frame: &mut Frame, area: Rect, app: &App, is_focused: bool) {
    let theme = &app.theme;
    app.ui.record_rect(crate::app::RectSlot::Graph, area);

    let border_color = if is_focused {
        theme.focus_border
    } else {
        theme.border
    };

    let block = Block::default()
        .title(" Graph ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Record the visible height so navigation can auto-scroll to follow the cursor.
    app.ui.graph_viewport.set(inner.height as usize);

    let commits = &app.repo.commits;
    if commits.is_empty() {
        let p = Paragraph::new(" No commits").style(Style::default().fg(theme.dim).bg(theme.bg));
        frame.render_widget(p, inner);
        return;
    }

    // Determine spacious mode from config.
    let spacious = app.config.graph_spacious();
    let first_parent = app.ui.graph_first_parent;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // The main branch's tip OID, but only when it's actually inside the loaded
    // window — that's the gate for reserving column 0 for the main spine. Outside
    // the window there is nothing to anchor, so we fall back to plain layout.
    let main_tip: Option<String> = app
        .detect_main_branch()
        .map(|(_, sha)| sha)
        .filter(|sha| commits.iter().any(|c| &c.id == sha));

    // The current branch's tip = the commit carrying the HEAD decoration. It is
    // pinned to column 1 (right beside main) so the branch you're working on —
    // e.g. an unmerged `dev` where commits accumulate — is always prominent.
    let head_tip: Option<String> = commits
        .iter()
        .find(|c| {
            c.refs
                .iter()
                .any(|r| r.kind == crate::git::types::RefKind::Head)
        })
        .map(|c| c.id.clone());

    // Rebuild the (expensive) lane layout only when the commit set or render
    // mode changes; otherwise reuse the cached rows. The reserved tips are part
    // of the key because they change the column assignment.
    let key = (
        commits.len(),
        commits.first().map(|c| c.id.clone()).unwrap_or_default(),
        spacious,
        first_parent,
        main_tip.clone().unwrap_or_default(),
        head_tip.clone().unwrap_or_default(),
    );
    // Commit-set identity shared by the selection-derived caches (everything
    // that depends on the loaded commits but NOT on the cursor position).
    let set_id = (
        commits.len(),
        commits.first().map(|c| c.id.clone()).unwrap_or_default(),
    );
    let main_tip_str = main_tip.clone().unwrap_or_default();
    let head_tip_str = head_tip.clone().unwrap_or_default();
    let graph_index = app.ui.graph_index;
    let graph_focus = app.ui.graph_focus.clone();
    let main_base_sha = app.detect_main_branch().map(|(_, sha)| sha);

    // Fill every cache in one borrow_mut pass so a single frame never runs the
    // O(commits) highlight / ancestry / fork passes more than once, and only
    // re-runs them when their specific key changed (a cursor move invalidates
    // just the highlight, not main-ancestry or fork).
    {
        let mut cache = app.graph_cache.borrow_mut();
        if cache.key.as_ref() != Some(&key) {
            cache.rows = super::layout::build_graph_main(
                commits,
                spacious,
                first_parent,
                main_tip.as_deref(),
                head_tip.as_deref(),
            );
            cache.key = Some(key);
            // The row layout changed → selection-derived caches are stale.
            cache.hl = None;
            cache.main_ancestors = None;
            cache.fork = None;
        }

        // Branch highlight (only when the graph panel has focus).
        if is_focused {
            let hl_key = (
                set_id.0,
                set_id.1.clone(),
                first_parent,
                main_tip_str.clone(),
                head_tip_str,
                graph_index,
            );
            if cache.hl_key.as_ref() != Some(&hl_key) {
                cache.hl = Some(super::layout::branch_highlight_main(
                    commits,
                    graph_index,
                    first_parent,
                    main_tip.as_deref(),
                    head_tip.as_deref(),
                ));
                cache.hl_key = Some(hl_key);
            }
        } else if cache.hl.is_some() {
            cache.hl = None;
            cache.hl_key = None;
        }

        // Main-branch ancestry set.
        let ma_key = (set_id.0, set_id.1.clone(), main_tip_str);
        if cache.main_ancestors_key.as_ref() != Some(&ma_key) {
            cache.main_ancestors = main_tip
                .as_deref()
                .and_then(|sha| commits.iter().position(|c| c.id == sha))
                .map(|mi| super::layout::ancestors(commits, mi));
            cache.main_ancestors_key = Some(ma_key);
        }

        // Branch-lens fork point.
        let fork_key = (
            set_id.0,
            set_id.1,
            graph_focus.clone().unwrap_or_default(),
            main_base_sha.clone().unwrap_or_default(),
        );
        if cache.fork_key.as_ref() != Some(&fork_key) {
            cache.fork = graph_focus.as_deref().and_then(|tip| {
                let base_sha = main_base_sha.clone()?;
                let ti = commits.iter().position(|c| c.id == tip)?;
                let bi = commits.iter().position(|c| c.id == base_sha)?;
                let a = super::layout::ancestors(commits, ti);
                let b = super::layout::ancestors(commits, bi);
                commits
                    .iter()
                    .find(|c| a.contains(&c.id) && b.contains(&c.id))
                    .map(|c| c.id.clone())
            });
            cache.fork_key = Some(fork_key);
        }
    }

    let cache = app.graph_cache.borrow();
    let graph_rows = &cache.rows;
    let total_rows = graph_rows.len();
    let highlight = cache.hl.as_ref();
    let main_ancestors = cache.main_ancestors.as_ref();
    let fork = cache.fork.as_deref();

    // Per-local-branch divergence vs upstream (↑ahead ↓behind) for the tip badges.
    let divergence: super::render::Divergence = app
        .branches
        .iter()
        .filter(|b| b.kind == crate::git::types::RefKind::LocalBranch)
        .map(|b| (b.name.clone(), (b.ahead, b.behind)))
        .collect();

    // Render only the visible viewport instead of the whole history. The
    // selection's background highlight is applied to the in-window row that
    // corresponds to the selected commit.
    let selected_commit = app.ui.graph_index;
    let offset = app.ui.graph_offset;
    let visible = inner.height as usize;
    let end = (offset + visible).min(total_rows);

    let lines = super::render::render_rows_window(
        graph_rows,
        offset,
        end,
        commits,
        theme,
        now,
        &divergence,
        highlight,
        fork,
        main_ancestors,
    );

    let items: Vec<ListItem> = lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let row_idx = offset + i;
            let is_selected = graph_rows
                .get(row_idx)
                .map(|r| r.is_node_row && r.commit_index == selected_commit)
                .unwrap_or(false);

            let item = ListItem::new(line);
            if is_selected {
                // Selection is shown by background color ALONE so box-drawing
                // glyphs stay aligned and lane colors are preserved.
                item.style(Style::default().bg(theme.selection_bg))
            } else {
                item
            }
        })
        .collect();

    let list = List::new(items).style(Style::default().bg(theme.bg));
    frame.render_widget(list, inner);

    // Scrollbar on the right edge — gives a sense of position in long histories.
    if total_rows > inner.height as usize {
        let mut sb_state = ScrollbarState::new(total_rows).position(offset);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█")
            .style(Style::default().fg(theme.dim));
        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut sb_state,
        );
    }
}

// ─── Right panel: commit detail ───────────────────────────────────────────────

fn render_commit_detail(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let is_focused = app.ui.panel() == &Panel::Main;
    // Record the rect so mouse clicks on the commit detail panel can focus it.
    app.ui.record_rect(crate::app::RectSlot::Diff, area);

    let border_color = if is_focused {
        theme.focus_border
    } else {
        theme.border
    };

    let block = Block::default()
        .title(" Commit ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let commits = &app.repo.commits;
    if commits.is_empty() {
        let p = Paragraph::new(" No commit selected")
            .style(Style::default().fg(theme.dim).bg(theme.bg));
        frame.render_widget(p, inner);
        return;
    }

    // Guard against out-of-bounds index.
    let idx = app.ui.graph_index.min(commits.len().saturating_sub(1));
    let commit = &commits[idx];

    let mut lines: Vec<Line<'static>> = Vec::new();

    // ── Commit header ─────────────────────────────────────────────────────────
    lines.push(Line::from(vec![
        Span::styled("commit ".to_string(), Style::default().fg(theme.dim)),
        Span::styled(
            commit.id.clone(),
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Author.
    lines.push(Line::from(vec![
        Span::styled("Author: ".to_string(), Style::default().fg(theme.dim)),
        Span::styled(
            format!("{} <{}>", commit.author_name, commit.author_email),
            Style::default().fg(theme.fg),
        ),
    ]));

    // Date (formatted from Unix timestamp).
    let date_str = format_timestamp(commit.time);
    lines.push(Line::from(vec![
        Span::styled("Date:   ".to_string(), Style::default().fg(theme.dim)),
        Span::styled(date_str, Style::default().fg(theme.fg)),
    ]));

    lines.push(Line::raw("".to_string()));

    // Summary.
    lines.push(Line::from(Span::styled(
        format!("    {}", commit.summary),
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
    )));

    // Body (if present).
    if !commit.body.is_empty() {
        lines.push(Line::raw("".to_string()));
        for body_line in commit.body.lines() {
            lines.push(Line::from(Span::styled(
                format!("    {}", body_line),
                Style::default().fg(theme.fg),
            )));
        }
    }

    // ── Changed files (from selected_diff) ───────────────────────────────────
    if let Some(diff) = &app.repo.selected_diff {
        lines.push(Line::raw("".to_string()));
        lines.push(Line::from(Span::styled(
            format!("── {} file(s) changed ──", diff.files.len()),
            Style::default().fg(theme.hunk).add_modifier(Modifier::BOLD),
        )));

        for file_diff in &diff.files {
            let mut added = 0usize;
            let mut removed = 0usize;
            for hunk in &file_diff.hunks {
                for dl in &hunk.lines {
                    use crate::git::types::DiffLineKind;
                    match dl.kind {
                        DiffLineKind::Added => added += 1,
                        DiffLineKind::Removed => removed += 1,
                        _ => {}
                    }
                }
            }

            let label =
                if file_diff.old_path != file_diff.new_path && !file_diff.old_path.is_empty() {
                    format!("{} → {}", file_diff.old_path, file_diff.new_path)
                } else {
                    file_diff.new_path.clone()
                };

            lines.push(Line::from(vec![
                Span::styled("  ".to_string(), Style::default()),
                Span::styled(label, Style::default().fg(theme.fg)),
                Span::styled(format!("  +{}", added), Style::default().fg(theme.added)),
                Span::styled(format!(" -{}", removed), Style::default().fg(theme.removed)),
            ]));
        }

        // ── Full diff of the commit (scroll with the diff panel focused) ──────
        for file_diff in &diff.files {
            lines.push(Line::raw("".to_string()));
            lines.push(Line::from(Span::styled(
                format!("─── {} ", file_diff.new_path),
                Style::default().fg(theme.hunk).add_modifier(Modifier::BOLD),
            )));

            if file_diff.is_binary {
                lines.push(Line::from(Span::styled(
                    "  (binary file)".to_string(),
                    Style::default().fg(theme.dim),
                )));
                continue;
            }

            for hunk in &file_diff.hunks {
                lines.push(Line::from(Span::styled(
                    hunk.header.clone(),
                    Style::default().fg(theme.hunk),
                )));
                for dl in &hunk.lines {
                    use crate::git::types::DiffLineKind::*;
                    let styled = match dl.kind {
                        Added => Some((format!("+{}", dl.text), theme.added)),
                        Removed => Some((format!("-{}", dl.text), theme.removed)),
                        Context => Some((format!(" {}", dl.text), theme.fg)),
                        Meta => Some((dl.text.clone(), theme.dim)),
                        // The @@ header is already rendered via `hunk.header`.
                        Header => None,
                    };
                    if let Some((text, color)) = styled {
                        lines.push(Line::from(Span::styled(text, Style::default().fg(color))));
                    }
                }
            }
        }
    } else {
        lines.push(Line::raw("".to_string()));
        lines.push(Line::from(Span::styled(
            "  (press Enter to load diff)",
            Style::default().fg(theme.dim),
        )));
    }

    // Scroll with the diff panel focused (Tab → ↑/↓). No wrap so diff lines
    // keep their alignment.
    let paragraph = Paragraph::new(lines)
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .scroll((app.ui.diff_scroll, 0));
    frame.render_widget(paragraph, inner);
}

// ─── Timestamp formatting ─────────────────────────────────────────────────────

/// Format a Unix timestamp as a human-readable string (no external crate).
///
/// Produces output in the form: `2024-01-15 14:32:00 UTC`
pub(crate) fn format_timestamp(unix: i64) -> String {
    // Simple integer arithmetic — no external date library needed.
    // Works correctly for dates from 1970 onwards.
    if unix <= 0 {
        return "1970-01-01 00:00:00 UTC".to_string();
    }

    let secs_per_min = 60u64;
    let _secs_per_hour = 3600u64;
    let _secs_per_day = 86400u64;

    let mut remaining = unix as u64;
    let seconds = remaining % secs_per_min;
    remaining /= secs_per_min;
    let minutes = remaining % 60;
    remaining /= 60;
    let hours = remaining % 24;
    let mut days = remaining / 24; // days since 1970-01-01

    // Compute year, month, day from days-since-epoch.
    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let month_days: &[u64] = if is_leap(year) {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u32;
    for &md in month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    let day = days + 1;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year, month, day, hours, minutes, seconds
    )
}

fn is_leap(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
