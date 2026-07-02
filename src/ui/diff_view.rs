use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::app::App;

/// Render a standalone diff view (used in Graph mode for commit diffs).
///
/// In Status mode the diff is rendered inline by `status_view`; this entry
/// point is for Graph mode where a full-panel diff is shown on the right.
///
/// Only the visible viewport of lines is built (see [`build_diff_lines_window`])
/// instead of the entire diff — a significant saving on large diffs (10 k+ lines).
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;

    let title = match &app.repo.selected_diff {
        Some(d) if !d.files.is_empty() => {
            format!(" Commit diff: {} ", d.files[0].new_path)
        }
        _ => " Diff ".to_owned(),
    };

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
    let paragraph = Paragraph::new(lines).style(Style::default().bg(theme.bg).fg(theme.fg));

    frame.render_widget(paragraph, inner);
}

/// Build styled diff lines from `app.repo.selected_diff`.
///
/// Where possible, applies intra-line word-diff highlighting on adjacent
/// removed/added pairs using `crate::git::diff::intra_line_spans`.
pub fn build_diff_lines(app: &App) -> Vec<Line<'static>> {
    build_diff_lines_window(app, 0, usize::MAX)
}

/// Build only the diff lines in the viewport `[start, start+count)`.
///
/// Instead of allocating a `Vec<Line>` for the entire diff (which can be
/// 10 k+ lines for a large changeset), this function walks the diff
/// structure and emits only the lines that fall within the visible window.
/// Lines before `start` are counted but not allocated; iteration stops once
/// `count` lines have been emitted.
pub fn build_diff_lines_window(app: &App, start: usize, count: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let Some(diff) = &app.repo.selected_diff else {
        if start > 0 {
            return Vec::new();
        }
        return vec![Line::from(Span::styled(
            " Select a commit to view its diff",
            Style::default().fg(theme.dim),
        ))];
    };

    if count == 0 {
        return Vec::new();
    }

    let mut lines = Vec::with_capacity(count.min(256));
    let mut line_no = 0usize; // 0-based line index in the full diff
    let end = start + count;

    for file in &diff.files {
        // File header (1 line).
        if line_no >= start && line_no < end {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("─── {} ", file.new_path.clone()),
                    Style::default().fg(theme.hunk).add_modifier(Modifier::BOLD),
                ),
                Span::styled("─".repeat(20), Style::default().fg(theme.dim)),
            ]));
        }
        line_no += 1;

        if file.is_binary {
            if line_no >= start && line_no < end {
                lines.push(Line::from(Span::styled(
                    "  (binary file — cannot display)",
                    Style::default().fg(theme.dim),
                )));
            }
            line_no += 1;
            continue;
        }

        for hunk in &file.hunks {
            // Hunk header (1 line).
            if line_no >= start && line_no < end {
                lines.push(Line::from(Span::styled(
                    hunk.header.clone(),
                    Style::default().fg(theme.hunk),
                )));
            }
            line_no += 1;

            // Hunk body — build into a temp buffer, then emit only the
            // visible lines.  Individual hunks are typically small (20–100
            // lines), so building the whole hunk is cheap; the main saving
            // comes from skipping entire files/hunks outside the viewport.
            let mut hunk_lines = Vec::new();
            render_hunk_lines(&hunk.lines, theme, &mut hunk_lines);
            for hl in hunk_lines {
                if line_no >= start && line_no < end {
                    lines.push(hl);
                }
                line_no += 1;
            }
            if line_no >= end {
                return lines;
            }
        }

        // Blank separator between files (1 line).
        if line_no >= start && line_no < end {
            lines.push(Line::from(""));
        }
        line_no += 1;
        if line_no >= end {
            return lines;
        }
    }

    if lines.is_empty() && start == 0 {
        lines.push(Line::from(Span::styled(
            " (empty diff)",
            Style::default().fg(theme.dim),
        )));
    }

    lines
}

/// Render a hunk's diff lines with intra-line word highlighting on 1:1 pairs.
fn render_hunk_lines(
    dl_lines: &[crate::git::types::DiffLine],
    theme: &crate::theme::Theme,
    out: &mut Vec<Line<'static>>,
) {
    use crate::git::types::DiffLineKind::*;

    // Buffer of consecutive removed-lines waiting to be paired with added lines.
    let mut removed_buf: Vec<String> = Vec::new();

    let flush_removed =
        |removed: &mut Vec<String>, out: &mut Vec<Line<'static>>, color: ratatui::style::Color| {
            for r in removed.drain(..) {
                out.push(Line::from(Span::styled(
                    format!("-{}", r),
                    Style::default().fg(color),
                )));
            }
        };

    for dl in dl_lines {
        match dl.kind {
            Removed => {
                // Buffer — might be paired with the next Added line.
                removed_buf.push(dl.text.clone());
            }
            Added => {
                if removed_buf.len() == 1 {
                    // Exactly one removed : one added → word diff.
                    let removed_text = removed_buf.remove(0);
                    let (rem_spans, add_spans) =
                        crate::git::diff::intra_line_spans(&removed_text, &dl.text);
                    out.push(spans_to_line('-', rem_spans, theme.removed));
                    out.push(spans_to_line('+', add_spans, theme.added));
                } else {
                    // Flush buffered removes plain, then emit this added line.
                    flush_removed(&mut removed_buf, out, theme.removed);
                    out.push(Line::from(Span::styled(
                        format!("+{}", dl.text.clone()),
                        Style::default().fg(theme.added),
                    )));
                }
            }
            Context => {
                // Flush pending removes before a context line.
                flush_removed(&mut removed_buf, out, theme.removed);
                out.push(Line::from(Span::styled(
                    format!(" {}", dl.text.clone()),
                    Style::default().fg(theme.fg).add_modifier(Modifier::DIM),
                )));
            }
            Header => {
                flush_removed(&mut removed_buf, out, theme.removed);
                out.push(Line::from(Span::styled(
                    dl.text.clone(),
                    Style::default().fg(theme.hunk),
                )));
            }
            Meta => {
                flush_removed(&mut removed_buf, out, theme.removed);
                out.push(Line::from(Span::styled(
                    dl.text.clone(),
                    Style::default().fg(theme.dim),
                )));
            }
        }
    }

    // Flush any trailing removed lines.
    flush_removed(&mut removed_buf, out, theme.removed);
}

// ─── Intra-line span helpers ──────────────────────────────────────────────────

/// Convert a `(bool, String)` span list (from `intra_line_spans`) to a ratatui
/// `Line`, prepended with `prefix_char`.
///
/// Spans where `changed == true` are rendered bold + underlined; unchanged
/// spans use the base color at normal weight.
fn spans_to_line(
    prefix_char: char,
    spans: Vec<(bool, String)>,
    base_color: ratatui::style::Color,
) -> Line<'static> {
    let mut ratatui_spans = vec![Span::styled(
        prefix_char.to_string(),
        Style::default().fg(base_color),
    )];

    for (changed, text) in spans {
        let style = if changed {
            Style::default()
                .fg(base_color)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(base_color)
        };
        ratatui_spans.push(Span::styled(text, style));
    }

    Line::from(ratatui_spans)
}
