use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::app::{App, Mode};
use crate::ui::diff_view::build_diff_lines_window;

/// Render the Stashes mode: list on the left, optional diff preview on the right.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    // Determine whether to split into list + preview pane.
    let has_preview = app.repo.selected_diff.is_some()
        && app
            .repo
            .selected_diff
            .as_ref()
            .map(|d| !d.files.is_empty())
            .unwrap_or(false);

    if has_preview {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        render_list(frame, chunks[0], app);
        render_preview(frame, chunks[1], app);
    } else {
        render_list(frame, area, app);
    }
}

/// Render the stash list panel.
fn render_list(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    app.ui.record_rect(crate::app::RectSlot::Other, area);

    let focused = app.mode == Mode::Stashes;
    let border_style = if focused {
        Style::default().fg(theme.focus_border)
    } else {
        Style::default().fg(theme.border)
    };

    let block = Block::default()
        .title(" Stashes ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let stash_index = app.ui.stash_index;

    if app.stashes.is_empty() {
        let p = Paragraph::new(vec![Line::from(Span::styled(
            " No stashes",
            Style::default().fg(theme.dim),
        ))])
        .style(Style::default().bg(theme.bg));
        frame.render_widget(p, inner);
    } else {
        // Build visible list rows.
        let list_height = inner.height as usize;
        let selected = stash_index.min(app.stashes.len().saturating_sub(1));

        // Compute scroll offset so selected row is always visible.
        let offset = compute_offset(selected, list_height, app.stashes.len());

        let mut lines: Vec<Line<'static>> = Vec::new();

        for (i, stash) in app
            .stashes
            .iter()
            .enumerate()
            .skip(offset)
            .take(list_height)
        {
            let is_selected = i == selected;

            // Build index label: "stash@{i}" in dim colour.
            let index_label = format!("stash@{{{}}}  ", stash.index);
            let message = stash.message.clone();

            let (index_style, msg_style, row_bg) = if is_selected {
                (
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.focus_border)
                        .add_modifier(Modifier::BOLD | Modifier::DIM),
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.focus_border)
                        .add_modifier(Modifier::BOLD),
                    theme.focus_border,
                )
            } else {
                (
                    Style::default().fg(theme.dim).bg(theme.bg),
                    Style::default().fg(theme.fg).bg(theme.bg),
                    theme.bg,
                )
            };

            let line = Line::from(vec![
                Span::styled(" ", Style::default().bg(row_bg)),
                Span::styled(index_label, index_style),
                Span::styled(message, msg_style),
            ]);
            lines.push(line);
        }

        let p = Paragraph::new(lines).style(Style::default().bg(theme.bg));
        frame.render_widget(p, inner);
    }

    // Footer hint bar — rendered inside the outer block inner area at the bottom.
    // We place it as the last line within inner by computing a sub-rect.
    if inner.height >= 2 {
        let hint_area = Rect {
            y: inner.y + inner.height - 1,
            height: 1,
            ..inner
        };

        let hint_line = Line::from(vec![
            hint_key(theme, "s"),
            hint_text(theme, ":save  "),
            hint_key(theme, "Enter"),
            hint_text(theme, ":apply  "),
            hint_key(theme, "p"),
            hint_text(theme, ":pop  "),
            hint_key(theme, "d"),
            hint_text(theme, ":drop"),
        ]);

        let hint_para = Paragraph::new(hint_line).style(Style::default().bg(theme.bg));
        frame.render_widget(hint_para, hint_area);
    }
}

/// Render the stash diff preview panel.
fn render_preview(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;

    let title = app
        .stashes
        .get(app.ui.stash_index)
        .map(|s| format!(" stash@{{{}}}: {} ", s.index, s.message))
        .unwrap_or_else(|| " Stash diff ".to_owned());

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let scroll = app.ui.diff_scroll as usize;
    let height = inner.height as usize;
    let lines = build_diff_lines_window(app, scroll, height);
    // Scroll is 0 because the window already starts at `scroll`.
    let para = Paragraph::new(lines).style(Style::default().bg(theme.bg).fg(theme.fg));

    frame.render_widget(para, inner);
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Compute a scroll offset so the `selected` item is visible in a viewport of
/// `height` rows from a list of `total` items.
fn compute_offset(selected: usize, height: usize, total: usize) -> usize {
    if height == 0 || total == 0 {
        return 0;
    }
    // Leave at least one row for the footer hint.
    let visible = height.saturating_sub(1).max(1);
    if selected < visible {
        0
    } else {
        selected - visible + 1
    }
}

fn hint_key(theme: &crate::theme::Theme, text: &'static str) -> Span<'static> {
    Span::styled(
        text,
        Style::default()
            .fg(theme.focus_border)
            .add_modifier(Modifier::BOLD),
    )
}

fn hint_text(theme: &crate::theme::Theme, text: &'static str) -> Span<'static> {
    Span::styled(text, Style::default().fg(theme.dim))
}
