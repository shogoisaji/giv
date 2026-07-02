/// Branches mode view.
///
/// Layout: a single bordered panel listing branches in three groups:
///   Local   — local branches (● marks HEAD, * in compact form)
///   Remotes — remote-tracking branches
///   Tags    — annotated / lightweight tags
///
/// Selection is driven by `app.ui.branch_index` (the dedicated cursor for the
/// Branches mode).  Each branch row that is selectable receives a logical index;
/// the cursor highlights the matching row.
///
/// Color scheme (Tokyo Night defaults):
///   HEAD branch    → theme.head   + BOLD
///   Local branch   → theme.lane[1] (green)
///   Remote branch  → theme.dim   (blue-ish)
///   Selected row   → REVERSED + BOLD highlight
///
/// Footer shows key hints.  The view never panics on empty data.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::app::App;
use crate::git::types::RefKind;

/// Render the Branches mode.
///
/// Reads:
///   `app.branches`         — Vec<Branch> (local + remote), populated by App::refresh()
///   `app.tags`             — Vec<Tag>, populated by App::refresh()
///   `app.ui.branch_index`  — selection cursor for the branches list
///   `app.theme`            — color palette
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    // ── Layout: main panel + footer hint line ────────────────────────────────
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    render_branch_panel(frame, chunks[0], app);
    render_hint_bar(frame, chunks[1], app);
}

// ─── Main branch list panel ───────────────────────────────────────────────────

fn render_branch_panel(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    app.ui.record_rect(crate::app::RectSlot::Other, area);

    // Border color: always show focused style (Branches mode owns the whole area).
    let block = Block::default()
        .title(" Branches ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.focus_border))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // ── Partition into local / remote / tags ─────────────────────────────────
    let branches = app.branches.as_slice();
    let tags = app.tags.as_slice();

    let locals: Vec<_> = branches
        .iter()
        .filter(|b| matches!(b.kind, RefKind::LocalBranch | RefKind::Head))
        .collect();

    let remotes: Vec<_> = branches
        .iter()
        .filter(|b| matches!(b.kind, RefKind::RemoteBranch))
        .collect();

    // Build the flat list of selectable items (headers are not selectable).
    // logical_index increments only for actual branch/tag rows.
    let total_selectable = locals.len() + remotes.len() + tags.len();

    if total_selectable == 0 {
        // Truly empty — show placeholder.
        let p = Paragraph::new(Line::from(Span::styled(
            " No branches found",
            Style::default().fg(theme.dim),
        )))
        .style(Style::default().bg(theme.bg));
        frame.render_widget(p, inner);
        return;
    }

    let selected = app.ui.branch_index.min(total_selectable.saturating_sub(1));

    let mut items: Vec<ListItem> = Vec::new();
    let mut logical_idx: usize = 0;

    // ── Local branches ────────────────────────────────────────────────────────
    items.push(section_header(
        format!(" Local ({}) ", locals.len()),
        theme.staged,
        theme,
    ));

    if locals.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "    (none)",
            Style::default().fg(theme.dim),
        ))));
    } else {
        for branch in &locals {
            let is_selected = selected == logical_idx;
            items.push(branch_row(branch, is_selected, theme));
            logical_idx += 1;
        }
    }

    // ── Remote branches ───────────────────────────────────────────────────────
    items.push(section_header(
        format!(" Remotes ({}) ", remotes.len()),
        // Use lane[0] (blue) for remotes header
        theme.lane.first().copied().unwrap_or(theme.dim),
        theme,
    ));

    if remotes.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "    (none)",
            Style::default().fg(theme.dim),
        ))));
    } else {
        for branch in &remotes {
            let is_selected = selected == logical_idx;
            items.push(remote_row(branch, is_selected, theme));
            logical_idx += 1;
        }
    }

    // ── Tags ──────────────────────────────────────────────────────────────────
    if !tags.is_empty() {
        items.push(section_header(
            format!(" Tags ({}) ", tags.len()),
            // Use lane[4] (orange) for tags
            theme.lane.get(4).copied().unwrap_or(theme.hunk),
            theme,
        ));

        for tag in tags {
            let is_selected = selected == logical_idx;
            let color = theme.lane.get(4).copied().unwrap_or(theme.hunk);
            let short_id = crate::git::short_oid(&tag.target);
            let line = if is_selected {
                Line::from(vec![
                    Span::styled(
                        "  ◆ ",
                        Style::default()
                            .fg(theme.bg)
                            .bg(color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{:<32}", tag.name),
                        Style::default()
                            .fg(theme.bg)
                            .bg(color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {}", short_id),
                        Style::default()
                            .fg(theme.bg)
                            .bg(color)
                            .add_modifier(Modifier::DIM | Modifier::BOLD),
                    ),
                ])
            } else {
                Line::from(vec![
                    Span::styled("  ◆ ", Style::default().fg(color)),
                    Span::styled(format!("{:<32}", tag.name), Style::default().fg(color)),
                    Span::styled(format!(" {}", short_id), Style::default().fg(theme.dim)),
                ])
            };
            items.push(ListItem::new(line));
            logical_idx += 1;
        }
    }

    let list = List::new(items).style(Style::default().bg(theme.bg));
    frame.render_widget(list, inner);
}

// ─── Row builders ─────────────────────────────────────────────────────────────

/// Build a `ListItem` for a local branch row.
fn branch_row(
    branch: &crate::git::types::Branch,
    is_selected: bool,
    theme: &crate::theme::Theme,
) -> ListItem<'static> {
    // HEAD marker: ● for head, · for others.
    let marker = if branch.is_head { " ● " } else { " · " };

    // Base color: head = theme.head (gold), local = lane[1] (green).
    let base_color = if branch.is_head {
        theme.head
    } else {
        theme.lane.get(1).copied().unwrap_or(theme.fg)
    };

    let name_col = format!("{:<36}", branch.name);

    // Upstream + ahead/behind annotation.
    let upstream_text = build_upstream_annotation(branch);

    // Short target SHA.
    let short = crate::git::short_oid(&branch.target);

    let line = if is_selected {
        let mut spans = vec![
            Span::styled(
                marker.to_string(),
                Style::default()
                    .fg(theme.bg)
                    .bg(base_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                name_col,
                Style::default()
                    .fg(theme.bg)
                    .bg(base_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        if !upstream_text.is_empty() {
            spans.push(Span::styled(
                upstream_text,
                Style::default()
                    .fg(theme.bg)
                    .bg(base_color)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled(
            format!(" {}", short),
            Style::default()
                .fg(theme.bg)
                .bg(theme.dim)
                .add_modifier(Modifier::DIM | Modifier::BOLD),
        ));
        Line::from(spans)
    } else {
        let name_style = if branch.is_head {
            Style::default().fg(base_color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(base_color)
        };

        let mut spans = vec![
            Span::styled(marker.to_string(), Style::default().fg(base_color)),
            Span::styled(name_col, name_style),
        ];
        if !upstream_text.is_empty() {
            spans.push(Span::styled(upstream_text, Style::default().fg(theme.dim)));
        }
        spans.push(Span::styled(
            format!(" {}", short),
            Style::default().fg(theme.dim),
        ));
        Line::from(spans)
    };

    ListItem::new(line)
}

/// Build a `ListItem` for a remote-tracking branch row.
fn remote_row(
    branch: &crate::git::types::Branch,
    is_selected: bool,
    theme: &crate::theme::Theme,
) -> ListItem<'static> {
    // Remote branches: dim blue (lane[0]).
    let base_color = theme.lane.first().copied().unwrap_or(theme.dim);
    let name_col = format!("{:<36}", branch.name);
    let short = crate::git::short_oid(&branch.target);

    let line = if is_selected {
        Line::from(vec![
            Span::styled(
                "  · ".to_string(),
                Style::default()
                    .fg(theme.bg)
                    .bg(base_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                name_col,
                Style::default()
                    .fg(theme.bg)
                    .bg(base_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", short),
                Style::default()
                    .fg(theme.bg)
                    .bg(base_color)
                    .add_modifier(Modifier::DIM | Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                "  · ".to_string(),
                Style::default().fg(base_color).add_modifier(Modifier::DIM),
            ),
            Span::styled(
                name_col,
                Style::default().fg(base_color).add_modifier(Modifier::DIM),
            ),
            Span::styled(format!(" {}", short), Style::default().fg(theme.dim)),
        ])
    };

    ListItem::new(line)
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Build a styled section header `ListItem` (not selectable).
fn section_header(
    label: String,
    color: ratatui::style::Color,
    theme: &crate::theme::Theme,
) -> ListItem<'static> {
    ListItem::new(Line::from(vec![Span::styled(
        label,
        Style::default()
            .fg(color)
            .bg(theme.bg)
            .add_modifier(Modifier::BOLD),
    )]))
}

/// Format upstream + ahead/behind divergence as a compact string.
///
/// Examples:
///   ""                       → no upstream
///   " [origin/main]"         → upstream, 0/0
///   " [origin/main ↑2]"      → 2 ahead
///   " [origin/main ↓3]"      → 3 behind
///   " [origin/main ↑1 ↓2]"   → both
fn build_upstream_annotation(branch: &crate::git::types::Branch) -> String {
    let Some(upstream) = &branch.upstream else {
        return String::new();
    };

    let mut inner = upstream.clone();
    if branch.ahead > 0 {
        inner.push_str(&format!(" \u{2191}{}", branch.ahead)); // ↑
    }
    if branch.behind > 0 {
        inner.push_str(&format!(" \u{2193}{}", branch.behind)); // ↓
    }
    format!(" [{}]", inner)
}

// ─── Footer hint bar ─────────────────────────────────────────────────────────

fn render_hint_bar(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;

    let hints = vec![
        Span::styled(" ↑↓ navigate", Style::default().fg(theme.dim)),
        Span::styled("  Enter checkout", Style::default().fg(theme.dim)),
        Span::styled("  n new branch", Style::default().fg(theme.dim)),
        Span::styled("  D delete", Style::default().fg(theme.dim)),
        Span::styled("  f fetch", Style::default().fg(theme.dim)),
        Span::styled("  p pull", Style::default().fg(theme.dim)),
        Span::styled("  P push", Style::default().fg(theme.dim)),
    ];

    let line = Line::from(hints);
    let bar = Paragraph::new(line).style(Style::default().bg(theme.bg));
    frame.render_widget(bar, area);
}
