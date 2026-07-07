/// Command palette overlay.
///
/// Rendered on top of all other content when `app.palette` is `Some`.
/// Shows a query input line ("> <query>|") followed by a scrollable list of
/// filtered `PaletteItem`s — label on the left, key hint on the right (dimmed).
/// The cursor row is highlighted with the theme's focus colour.
///
/// Renders nothing when `app.palette` is `None`.
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::App;

/// Main entry point — called from `ui::view` when `app.palette.is_some()`.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(ref palette) = app.palette else {
        return;
    };

    let theme = &app.theme;

    // ── Sizing ────────────────────────────────────────────────────────────────
    // Width: 60% of frame, min 50 cols.
    // Height: 1 border + 1 query + 1 separator + items (up to 12) + 1 border.
    let visible_items = palette.items.len().min(12);
    let dialog_height = (3 + 1 + visible_items as u16)
        .max(6)
        .min(area.height.saturating_sub(4));
    let dialog_width = (area.width * 60 / 100).max(50).min(area.width);

    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    // ── Shadow effect: draw a 1-cell offset dark rect first ──────────────────
    let shadow_area = Rect {
        x: dialog_area.x.saturating_add(1),
        y: dialog_area.y.saturating_add(1),
        width: dialog_area
            .width
            .min(area.width.saturating_sub(dialog_area.x.saturating_add(1))),
        height: dialog_area
            .height
            .min(area.height.saturating_sub(dialog_area.y.saturating_add(1))),
    };
    // Render a dark background behind the dialog to simulate a shadow.
    if shadow_area.width > 0 && shadow_area.height > 0 {
        let shadow_block = Block::default().style(Style::default().bg(theme.border));
        frame.render_widget(shadow_block, shadow_area);
    }

    // ── Main dialog block ─────────────────────────────────────────────────────
    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .title(" Command Palette ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.focus_border))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    // ── Inner layout: query line + separator + item list ─────────────────────
    // We need at least 2 rows: query line + items area.
    if inner.height < 2 {
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // query input line
            Constraint::Length(1), // thin separator line
            Constraint::Min(0),    // item list
        ])
        .split(inner);

    // ── Query input line: "> <query>|" ───────────────────────────────────────
    let query_line = Line::from(vec![
        Span::styled(
            "> ",
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(palette.query.clone(), Style::default().fg(theme.fg)),
        Span::styled("\u{2588}", Style::default().fg(theme.focus_border)), // block cursor ▊
    ]);
    frame.render_widget(
        Paragraph::new(query_line).style(Style::default().bg(theme.bg)),
        layout[0],
    );

    // ── Separator line ────────────────────────────────────────────────────────
    if layout[1].height > 0 {
        let sep_width = layout[1].width as usize;
        let sep_line = Line::from(Span::styled(
            "\u{2500}".repeat(sep_width), // ─────
            Style::default().fg(theme.border),
        ));
        frame.render_widget(
            Paragraph::new(sep_line).style(Style::default().bg(theme.bg)),
            layout[1],
        );
    }

    // ── Item list ─────────────────────────────────────────────────────────────
    let list_area = layout[2];
    if list_area.height == 0 {
        // Still record an empty rect so a stale click can't hit a previous frame.
        app.ui.palette_list_rect.set(None);
        return;
    }

    // Scroll so the cursor is always visible.
    let max_visible = list_area.height as usize;
    let scroll_offset = if palette.cursor >= max_visible {
        palette.cursor - max_visible + 1
    } else {
        0
    };

    // Record the list area + scroll offset so the mouse handler can map a
    // click y coordinate to an absolute palette item index.
    app.ui.palette_list_rect.set(Some(list_area));
    app.ui.palette_scroll.set(scroll_offset);

    let hint_col_width: usize = 12; // reserved on the right for key hints
    let label_col_width = (list_area.width as usize).saturating_sub(hint_col_width + 2);

    let lines: Vec<Line<'static>> = palette
        .items
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(max_visible)
        .map(|(i, item)| {
            let is_selected = i == palette.cursor;

            // Truncate label to available width.
            let label = crate::ui::truncate(&item.label, label_col_width);
            // Pad label to fill the column.
            let padded_label = format!("{:<width$}", label, width = label_col_width);
            // Hint right-aligned, truncated.
            let hint = crate::ui::truncate(&item.hint, hint_col_width);
            let padded_hint = format!("{:>width$}", hint, width = hint_col_width);

            if is_selected {
                Line::from(vec![
                    Span::styled(" ", Style::default().bg(theme.focus_border)),
                    Span::styled(
                        padded_label,
                        Style::default()
                            .fg(theme.bg)
                            .bg(theme.focus_border)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" ", Style::default().bg(theme.focus_border)),
                    Span::styled(
                        padded_hint,
                        Style::default().fg(theme.bg).bg(theme.focus_border),
                    ),
                    Span::styled(" ", Style::default().bg(theme.focus_border)),
                ])
            } else {
                Line::from(vec![
                    Span::styled(" ", Style::default().bg(theme.bg)),
                    Span::styled(padded_label, Style::default().fg(theme.fg).bg(theme.bg)),
                    Span::styled(" ", Style::default().bg(theme.bg)),
                    Span::styled(padded_hint, Style::default().fg(theme.dim).bg(theme.bg)),
                    Span::styled(" ", Style::default().bg(theme.bg)),
                ])
            }
        })
        .collect();

    if palette.items.is_empty() {
        let empty_line = Line::from(Span::styled(
            "  No matching commands",
            Style::default().fg(theme.dim),
        ));
        frame.render_widget(
            Paragraph::new(vec![empty_line]).style(Style::default().bg(theme.bg)),
            list_area,
        );
    } else {
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(theme.bg)),
            list_area,
        );
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Return a `Rect` centred in `r` with the given absolute width and height.
fn centered_rect(width: u16, height: u16, r: Rect) -> Rect {
    let x = r.x + r.width.saturating_sub(width) / 2;
    let y = r.y + r.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(r.width),
        height: height.min(r.height),
    }
}
