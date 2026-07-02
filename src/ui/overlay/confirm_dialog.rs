use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::{App, Dialog};

/// Render a yes/no confirmation dialog.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Dialog::Confirm {
        ref message,
        ref pending,
    } = app.dialog
    else {
        return;
    };

    let theme = &app.theme;
    let command = pending.command_preview();
    let dialog_area = centered_rect(64, 9, area);

    let block = Block::default()
        .title(" Confirm ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.focus_border))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(dialog_area);

    frame.render_widget(Clear, dialog_area);
    frame.render_widget(block, dialog_area);

    let p = Paragraph::new(vec![
        Line::from(Span::styled(
            message.as_str(),
            Style::default().fg(theme.fg).bg(theme.bg),
        )),
        Line::from(""),
        // The exact git command that will run — shown before execution.
        Line::from(Span::styled(
            format!("$ {command}"),
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [y]",
                Style::default()
                    .fg(theme.added)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("es  ", Style::default().fg(theme.fg)),
            Span::styled(
                "[n]",
                Style::default()
                    .fg(theme.removed)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("o / Esc", Style::default().fg(theme.fg)),
        ]),
    ])
    .style(Style::default().bg(theme.bg))
    .alignment(Alignment::Center);

    frame.render_widget(p, inner);
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let top = r.height.saturating_sub(height) / 2;
    let left = r.width * (100 - percent_x) / 200;
    let width = r.width * percent_x / 100;

    Rect {
        x: r.x + left,
        y: r.y + top,
        width: width.min(r.width),
        height: height.min(r.height),
    }
}
