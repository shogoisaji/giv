/// Inspect mode: enter an arbitrary commit ref (sha / branch / tag / `HEAD~1`)
/// and view that commit's metadata + full diff, scrollable.
///
/// The ref is entered through a centered prompt (`Dialog::InspectRef`), which
/// reuses the dialog input machinery so digits/letters never collide with the
/// global mode-switch keys while typing.
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::App;
use crate::features::graph::view::format_timestamp;
use crate::git::types::{Diff, DiffLineKind};

/// Render the Inspect mode content (the commit detail / hint / error).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;

    let title = if app.inspect.query.is_empty() {
        " Inspect commit ".to_owned()
    } else {
        format!(" Inspect: {} ", app.inspect.query)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.focus_border))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(err) = &app.inspect.error {
        lines.push(Line::from(Span::styled(
            format!("  ✗ {}", err),
            Style::default().fg(theme.removed),
        )));
        lines.push(Line::raw("".to_string()));
        lines.push(Line::from(Span::styled(
            "  Press i (or Enter) to try another ref.",
            Style::default().fg(theme.dim),
        )));
    } else if let Some(commit) = &app.inspect.commit {
        // ── Metadata ──────────────────────────────────────────────────────────
        lines.push(Line::from(vec![
            Span::styled("commit ".to_string(), Style::default().fg(theme.dim)),
            Span::styled(
                commit.id.clone(),
                Style::default()
                    .fg(theme.focus_border)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Author: ".to_string(), Style::default().fg(theme.dim)),
            Span::styled(
                format!("{} <{}>", commit.author_name, commit.author_email),
                Style::default().fg(theme.fg),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Date:   ".to_string(), Style::default().fg(theme.dim)),
            Span::styled(format_timestamp(commit.time), Style::default().fg(theme.fg)),
        ]));
        lines.push(Line::raw("".to_string()));
        lines.push(Line::from(Span::styled(
            format!("    {}", commit.summary),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        )));
        if !commit.body.is_empty() {
            lines.push(Line::raw("".to_string()));
            for body_line in commit.body.lines() {
                lines.push(Line::from(Span::styled(
                    format!("    {}", body_line),
                    Style::default().fg(theme.fg),
                )));
            }
        }

        // ── Diff ──────────────────────────────────────────────────────────────
        if let Some(diff) = &app.inspect.diff {
            lines.extend(diff_lines(diff, app));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  Enter a commit ref to inspect it.",
            Style::default().fg(theme.fg),
        )));
        lines.push(Line::raw("".to_string()));
        lines.push(Line::from(Span::styled(
            "  Accepts a sha, short sha, branch, tag, or HEAD~1.",
            Style::default().fg(theme.dim),
        )));
        lines.push(Line::raw("".to_string()));
        lines.push(Line::from(Span::styled(
            "  Press i or Enter to open the prompt.",
            Style::default().fg(theme.dim),
        )));
    }

    let paragraph = Paragraph::new(lines)
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .scroll((app.ui.diff_scroll, 0));
    frame.render_widget(paragraph, inner);
}

/// Build the colored diff lines for the inspected commit.
fn diff_lines(diff: &Diff, app: &App) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::raw("".to_string()));
    lines.push(Line::from(Span::styled(
        format!("── {} file(s) changed ──", diff.files.len()),
        Style::default().fg(theme.hunk).add_modifier(Modifier::BOLD),
    )));

    for file in &diff.files {
        lines.push(Line::raw("".to_string()));
        lines.push(Line::from(Span::styled(
            format!("─── {} ", file.new_path),
            Style::default().fg(theme.hunk).add_modifier(Modifier::BOLD),
        )));

        if file.is_binary {
            lines.push(Line::from(Span::styled(
                "  (binary file)".to_string(),
                Style::default().fg(theme.dim),
            )));
            continue;
        }

        for hunk in &file.hunks {
            lines.push(Line::from(Span::styled(
                hunk.header.clone(),
                Style::default().fg(theme.hunk),
            )));
            for dl in &hunk.lines {
                let styled = match dl.kind {
                    DiffLineKind::Added => Some((format!("+{}", dl.text), theme.added)),
                    DiffLineKind::Removed => Some((format!("-{}", dl.text), theme.removed)),
                    DiffLineKind::Context => Some((format!(" {}", dl.text), theme.fg)),
                    DiffLineKind::Meta => Some((dl.text.clone(), theme.dim)),
                    // @@ header already rendered via hunk.header.
                    DiffLineKind::Header => None,
                };
                if let Some((text, color)) = styled {
                    lines.push(Line::from(Span::styled(text, Style::default().fg(color))));
                }
            }
        }
    }

    lines
}

/// Render the "enter a commit ref" input prompt (a centered modal). Called from
/// the dialog dispatch when `Dialog::InspectRef` is active.
pub fn render_prompt(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;

    let draft = match &app.dialog {
        crate::app::Dialog::InspectRef(s) => s.clone(),
        _ => String::new(),
    };

    let width = area.width.saturating_mul(6) / 10;
    let width = width.clamp(30, 80).min(area.width);
    let height = 5u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 3;
    let popup = Rect {
        x,
        y,
        width,
        height,
    };

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Inspect commit ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.focus_border))
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines = vec![
        Line::from(vec![
            Span::styled("› ", Style::default().fg(theme.focus_border)),
            Span::styled(draft, Style::default().fg(theme.fg)),
            Span::styled("▌", Style::default().fg(theme.focus_border)),
        ]),
        Line::raw("".to_string()),
        Line::from(Span::styled(
            "sha / branch / tag / HEAD~1   Enter: show   Esc: cancel",
            Style::default().fg(theme.dim),
        )),
    ];

    let p = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .style(Style::default().bg(theme.bg));
    frame.render_widget(p, inner);
}
