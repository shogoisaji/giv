/// Reusable centered modal primitives shared by every input/menu dialog.
///
/// Before this existed, several dialogs (`NewBranch`, `WorktreeAdd`,
/// `StashSave`, `TagCreate`, `ResetMenu`) had NO renderer at all — the user
/// typed into an invisible buffer with zero on-screen feedback. Routing them all
/// through these helpers guarantees every dialog is visible, has a block cursor
/// on the focused field, and shows a consistent hint line.
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::theme::Theme;

/// One labeled input field inside a text-input modal.
pub struct Field<'a> {
    /// Field label (e.g. `"name"`). Empty string renders a `›` prompt prefix.
    pub label: &'a str,
    /// Current draft text.
    pub value: &'a str,
    /// Whether this field currently holds the cursor.
    pub focused: bool,
}

/// Compute a centered popup rect of the given height, positioned in the upper
/// third of `r` so it never covers the status bar.
fn popup_rect(r: Rect, height: u16) -> Rect {
    let width = (r.width.saturating_mul(6) / 10).clamp(30, 80).min(r.width);
    let height = height.min(r.height);
    let x = r.x + (r.width.saturating_sub(width)) / 2;
    let y = r.y + (r.height.saturating_sub(height)) / 3;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Render a centered text-input modal with one or more labeled fields, a visible
/// block cursor on the focused field, and a hint line.
pub fn render_text_input(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    fields: &[Field],
    hint: &str,
) {
    // border(2) + fields + blank(1) + hint(1)
    let height = fields.len() as u16 + 4;
    let popup = popup_rect(area, height);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(format!(" {title} "))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.focus_border))
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::new();
    for f in fields {
        let mut spans: Vec<Span> = Vec::new();
        if f.label.is_empty() {
            spans.push(Span::styled("› ", Style::default().fg(theme.focus_border)));
        } else {
            // Dim label for the unfocused field, accent for the focused one.
            let label_color = if f.focused {
                theme.focus_border
            } else {
                theme.dim
            };
            spans.push(Span::styled(
                format!("{}: ", f.label),
                Style::default().fg(label_color),
            ));
        }
        spans.push(Span::styled(
            f.value.to_string(),
            Style::default().fg(theme.fg),
        ));
        if f.focused {
            spans.push(Span::styled(
                "▌",
                Style::default()
                    .fg(theme.focus_border)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        hint.to_string(),
        Style::default().fg(theme.dim),
    )));

    let p = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .style(Style::default().bg(theme.bg));
    frame.render_widget(p, inner);
}

/// Render a centered menu/choice modal: a title, one or more body lines, and a
/// hint line. `danger` tints the border red for destructive choices.
pub fn render_menu(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    body: Vec<Line<'static>>,
    hint: Line<'static>,
    danger: bool,
) {
    let height = body.len() as u16 + 4;
    let popup = popup_rect(area, height);

    let border = if danger {
        theme.removed
    } else {
        theme.focus_border
    };

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(format!(" {title} "))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = body;
    lines.push(Line::raw(""));
    lines.push(hint);

    let p = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .style(Style::default().bg(theme.bg));
    frame.render_widget(p, inner);
}
