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

use crate::app::App;

/// Abbreviate a path: replace the home directory prefix with `~`.
fn abbreviate_home(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if let Some(rest) = path.strip_prefix(&home) {
            return format!("~{}", rest);
        }
    }
    path.to_owned()
}

/// Render the Worktrees mode: a full-width list of worktrees.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    app.ui.record_rect(crate::app::RectSlot::Other, area);

    // ── Outer layout: list area + footer hint ─────────────────────────────────
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let list_area = chunks[0];
    let footer_area = chunks[1];

    // ── Bordered list panel ───────────────────────────────────────────────────
    let block = Block::default()
        .title(" Worktrees ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.focus_border))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(list_area);
    frame.render_widget(block, list_area);

    // Record the visible height so navigation can auto-scroll to follow the
    // cursor (mirrors `list_viewport` / `graph_viewport`).
    app.ui.worktree_viewport.set(inner.height as usize);

    // ── Empty state ───────────────────────────────────────────────────────────
    if app.worktrees.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            " No worktrees found",
            Style::default().fg(theme.dim),
        )))
        .style(Style::default().bg(theme.bg));
        frame.render_widget(p, inner);
        render_footer(frame, footer_area, app);
        return;
    }

    // ── Build list items ──────────────────────────────────────────────────────
    let selected = app
        .ui
        .worktree_index
        .min(app.worktrees.len().saturating_sub(1));

    let items: Vec<ListItem> = app
        .worktrees
        .iter()
        .enumerate()
        .map(|(i, wt)| {
            let is_selected = i == selected;

            // ● for current worktree (the one we launched from), ○ otherwise.
            let current_marker = if wt.is_current { "● " } else { "○ " };

            let path_display = abbreviate_home(&wt.path);

            // Branch name or detached indicator.
            let branch_text = match &wt.branch {
                Some(b) => b.clone(),
                None => "(detached)".to_owned(),
            };

            // Short SHA.
            let sha_text = format!(" {}", crate::git::short_oid(&wt.head));

            // Collect tags: [bare], [locked], [detached]
            let mut tags: Vec<(&str, bool)> = Vec::new(); // (label, is_warning)
            if wt.is_bare {
                tags.push(("[bare]", false));
            }
            if wt.is_locked {
                tags.push(("[locked]", true)); // locked shown in warning/orange color
            }
            if wt.branch.is_none() && !wt.is_bare {
                tags.push(("[detached]", false));
            }

            // ── Colour palette for each row state ─────────────────────────────
            // current_marker color: head (yellow/gold) if current, dim otherwise.
            let marker_color = if wt.is_current { theme.head } else { theme.dim };
            // branch color: focus_border (blue) when selected, staged (green) for
            // current worktree, dim otherwise.
            let branch_color = if is_selected {
                theme.focus_border
            } else if wt.is_current {
                theme.head
            } else {
                theme.staged
            };

            // ── Assemble spans ────────────────────────────────────────────────
            let mut spans: Vec<Span> = Vec::new();

            if is_selected {
                // Full row highlight: dark text on amber focus_border background.
                spans.push(Span::styled(
                    format!(" {} ", current_marker),
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.focus_border)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!("{:<40}", path_display),
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.focus_border)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!(" {:<25}", branch_text),
                    Style::default()
                        .fg(theme.bg)
                        .bg(branch_color)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!("{:<9}", sha_text),
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.focus_border)
                        .add_modifier(Modifier::DIM),
                ));
                for (label, is_warn) in &tags {
                    let tag_color = if *is_warn { theme.removed } else { theme.bg };
                    spans.push(Span::styled(
                        format!(" {}", label),
                        Style::default()
                            .fg(tag_color)
                            .bg(theme.focus_border)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
            } else {
                spans.push(Span::styled(
                    format!(" {} ", current_marker),
                    Style::default().fg(marker_color),
                ));
                spans.push(Span::styled(
                    format!("{:<40}", path_display),
                    Style::default().fg(theme.fg),
                ));
                spans.push(Span::styled(
                    format!(" {:<25}", branch_text),
                    Style::default().fg(branch_color),
                ));
                spans.push(Span::styled(
                    format!("{:<9}", sha_text),
                    Style::default().fg(theme.dim).add_modifier(Modifier::DIM),
                ));
                for (label, is_warn) in &tags {
                    let tag_color = if *is_warn { theme.removed } else { theme.dim };
                    spans.push(Span::styled(
                        format!(" {}", label),
                        Style::default().fg(tag_color),
                    ));
                }
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    // Scroll the rendered rows so the selection stays on-screen. `worktree_offset`
    // is maintained by `clamp_worktree_offset` after every selection change and by
    // the mouse-wheel view-scroll; the `.min` here is a defensive bound in case
    // the list shrank since.
    let total_rows = items.len();
    let offset = app.ui.worktree_offset.min(total_rows.saturating_sub(1));
    let visible: Vec<ListItem> = items.into_iter().skip(offset).collect();

    let list = List::new(visible).style(Style::default().bg(theme.bg));
    frame.render_widget(list, inner);

    // Scrollbar on the right edge when the list overflows the viewport.
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
            list_area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut sb_state,
        );
    }

    // ── Footer hint ───────────────────────────────────────────────────────────
    render_footer(frame, footer_area, app);
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;

    let hint = Line::from(vec![
        Span::styled(
            " a",
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(":add  ", Style::default().fg(theme.dim)),
        Span::styled(
            "d",
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(":remove  ", Style::default().fg(theme.dim)),
        Span::styled(
            "enter",
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(":switch(cd)  ", Style::default().fg(theme.dim)),
        Span::styled(
            "p",
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(":prune", Style::default().fg(theme.dim)),
    ]);

    let footer = Paragraph::new(hint).style(Style::default().bg(theme.bg));
    frame.render_widget(footer, area);
}
