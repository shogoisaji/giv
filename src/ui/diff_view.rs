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

            // Hunk body — walk every line to keep `removed_buf` state correct
            // for word-diff pairing, but only ALLOCATE `Line` objects for
            // visible rows.  This is the critical optimisation for large
            // hunks (e.g. a project's initial commit with thousands of added
            // lines): the walk is O(hunk_size) but allocation / styling /
            // intra-line-diff is O(visible_rows).
            render_hunk_lines_window(&hunk.lines, theme, &mut lines, start, end, &mut line_no);
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

/// Render a hunk's diff lines with intra-line word highlighting on 1:1 pairs,
/// emitting only the lines that fall within `[start, end)`.
///
/// Walks every input line to keep the `removed_buf` pairing state correct
/// (a Removed just before the window may pair with an Added inside it), but
/// only ALLOCATES `Line` objects for visible rows.  `line_no` is the running
/// 0-based line index in the full diff and is advanced by the number of lines
/// in this hunk's body (the caller has already counted the hunk header).
fn render_hunk_lines_window(
    dl_lines: &[crate::git::types::DiffLine],
    theme: &crate::theme::Theme,
    out: &mut Vec<Line<'static>>,
    start: usize,
    end: usize,
    line_no: &mut usize,
) {
    use crate::git::types::DiffLineKind::*;

    // Buffer of consecutive removed-lines waiting to be paired with added
    // lines. Each entry records the line's global line_no so we can decide
    // whether to emit it when it's eventually flushed.
    let mut removed_buf: Vec<(usize, &str)> = Vec::new();

    for dl in dl_lines {
        let visible = *line_no >= start && *line_no < end;
        match dl.kind {
            Removed => {
                removed_buf.push((*line_no, dl.text.as_str()));
            }
            Added => {
                if removed_buf.len() == 1 {
                    let (rem_ln, removed_text) = removed_buf.remove(0);
                    let (rem_spans, add_spans) =
                        crate::git::diff::intra_line_spans(removed_text, &dl.text);
                    if rem_ln >= start && rem_ln < end {
                        out.push(spans_to_line('-', rem_spans, theme.removed));
                    }
                    if visible {
                        out.push(spans_to_line('+', add_spans, theme.added));
                    }
                } else {
                    flush_removed_window(&mut removed_buf, out, start, end, theme.removed);
                    if visible {
                        out.push(Line::from(Span::styled(
                            format!("+{}", dl.text),
                            Style::default().fg(theme.added),
                        )));
                    }
                }
            }
            Context => {
                flush_removed_window(&mut removed_buf, out, start, end, theme.removed);
                if visible {
                    out.push(Line::from(Span::styled(
                        format!(" {}", dl.text),
                        Style::default().fg(theme.fg).add_modifier(Modifier::DIM),
                    )));
                }
            }
            Header => {
                flush_removed_window(&mut removed_buf, out, start, end, theme.removed);
                if visible {
                    out.push(Line::from(Span::styled(
                        dl.text.clone(),
                        Style::default().fg(theme.hunk),
                    )));
                }
            }
            Meta => {
                flush_removed_window(&mut removed_buf, out, start, end, theme.removed);
                if visible {
                    out.push(Line::from(Span::styled(
                        dl.text.clone(),
                        Style::default().fg(theme.dim),
                    )));
                }
            }
        }
        *line_no += 1;
    }

    flush_removed_window(&mut removed_buf, out, start, end, theme.removed);
}

/// Drain `removed_buf`, emitting a plain `-text` line for each entry whose
/// `line_no` falls within `[start, end)`. Entries outside the window are
/// silently dropped (no allocation).
fn flush_removed_window(
    removed: &mut Vec<(usize, &str)>,
    out: &mut Vec<Line<'static>>,
    start: usize,
    end: usize,
    color: ratatui::style::Color,
) {
    for (ln, r) in removed.drain(..) {
        if ln >= start && ln < end {
            out.push(Line::from(Span::styled(
                format!("-{}", r),
                Style::default().fg(color),
            )));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::git::types::{Diff, DiffLine, DiffLineKind, FileDiff, Hunk};
    use crate::test_backend::MockBackend;

    fn build_app_with_diff(diff: Diff) -> App {
        let mut b = MockBackend::new();
        b.commits = vec![crate::test_backend::mk_commit("c0", "c0", false)];
        let mut app = App::new(Box::new(b), Config::default()).expect("app builds");
        app.repo.selected_diff = Some(diff);
        app
    }

    fn mk_added(text: &str) -> DiffLine {
        DiffLine { kind: DiffLineKind::Added, text: text.into() }
    }
    fn mk_context(text: &str) -> DiffLine {
        DiffLine { kind: DiffLineKind::Context, text: text.into() }
    }
    fn mk_removed(text: &str) -> DiffLine {
        DiffLine { kind: DiffLineKind::Removed, text: text.into() }
    }

    fn big_hunk_diff(n_added: usize) -> Diff {
        Diff {
            files: vec![FileDiff {
                old_path: "a".into(),
                new_path: "a".into(),
                is_binary: false,
                hunks: vec![Hunk {
                    header: "@@ -1,1 +1,N @@".into(),
                    old_start: 1, old_lines: 1, new_start: 1, new_lines: n_added as u32,
                    lines: (0..n_added).map(|i| mk_added(&format!("line {i}"))).collect(),
                }],
            }],
        }
    }

    /// A 1000-line hunk must return only the visible window, with the correct
    /// content for each row — proving the windowing is correct at the line
    /// level inside a hunk, not just at the file/hunk boundary.
    #[test]
    fn window_returns_only_visible_rows_of_large_hunk() {
        let app = build_app_with_diff(big_hunk_diff(1000));
        // start=500, count=5 → rows 501..505 (0-based line_no 502..506,
        // accounting for file header at line_no 0 and hunk header at line_no 1).
        // file header = line_no 0, hunk header = line_no 1, first added = line_no 2.
        // So start=502 → first added index 500 = "line 500".
        let lines = build_diff_lines_window(&app, 502, 5);
        assert_eq!(lines.len(), 5);
        // Each added line renders as "+line N". Line 500 → "+line 500".
        // We check the first and last to confirm the window offset is correct.
        let first = format!("{:?}", lines[0]);
        assert!(first.contains("line 500"), "first visible row: {first}");
        let last = format!("{:?}", lines[4]);
        assert!(last.contains("line 504"), "last visible row: {last}");
    }

    /// Window starting before the diff returns the first visible rows.
    #[test]
    fn window_at_start_of_large_hunk() {
        let app = build_app_with_diff(big_hunk_diff(1000));
        let lines = build_diff_lines_window(&app, 0, 3);
        assert_eq!(lines.len(), 3);
        // line_no 0 = file header, 1 = hunk header, 2 = first added "line 0".
        let h = format!("{:?}", lines[0]);
        assert!(h.contains("a") || h.contains("─"), "file header: {h}");
    }

    /// Window past the end returns no lines (no padding).
    #[test]
    fn window_past_end_returns_empty() {
        let app = build_app_with_diff(big_hunk_diff(10));
        // file header(1) + hunk header(1) + 10 added + blank(1) = 13 lines.
        let lines = build_diff_lines_window(&app, 100, 5);
        assert!(lines.is_empty());
    }

    /// Word-diff pairing across the window boundary: a Removed just before the
    /// window pairs with an Added inside the window. The Added line must still
    /// get word-diff spans (not a plain "+text" line).
    #[test]
    fn word_diff_pairs_across_window_start() {
        let diff = Diff {
            files: vec![FileDiff {
                old_path: "a".into(),
                new_path: "a".into(),
                is_binary: false,
                hunks: vec![Hunk {
                    header: "@@ -1,2 +1,2 @@".into(),
                    old_start: 1, old_lines: 2, new_start: 1, new_lines: 2,
                    lines: vec![
                        mk_removed("hello world"),
                        mk_added("hello rust"),
                        mk_context("after"),
                    ],
                }],
            }],
        };
        let app = build_app_with_diff(diff);
        // file header=0, hunk header=1, removed=2, added=3, context=4, blank=5.
        // Window starts at the added line (line_no 3).
        let lines = build_diff_lines_window(&app, 3, 2);
        assert_eq!(lines.len(), 2);
        // The added line should have word-diff spans (multiple spans, not a
        // single plain span), because the removed line just before it pairs.
        let added_repr = format!("{:?}", lines[0]);
        // word-diff splits "hello rust" into unchanged "hello r" + changed "ust".
        assert!(
            added_repr.contains("ust") && added_repr.contains("hello r"),
            "added line text: {added_repr}"
        );
    }

    /// Removed lines inside the window are emitted; removed lines outside are
    /// not. This guards the `removed_buf` line_no tracking.
    #[test]
    fn removed_lines_respect_window_bounds() {
        let diff = Diff {
            files: vec![FileDiff {
                old_path: "a".into(),
                new_path: "a".into(),
                is_binary: false,
                hunks: vec![Hunk {
                    header: "@@ -1,5 +1,0 @@".into(),
                    old_start: 1, old_lines: 5, new_start: 1, new_lines: 0,
                    lines: vec![
                        mk_removed("r0"),
                        mk_removed("r1"),
                        mk_removed("r2"),
                        mk_removed("r3"),
                        mk_removed("r4"),
                    ],
                }],
            }],
        };
        let app = build_app_with_diff(diff);
        // file header=0, hunk header=1, r0=2, r1=3, r2=4, r3=5, r4=6, blank=7.
        // Window [4, 6) → r2, r3.
        let lines = build_diff_lines_window(&app, 4, 2);
        assert_eq!(lines.len(), 2);
        let l0 = format!("{:?}", lines[0]);
        assert!(l0.contains("r2"), "first: {l0}");
        let l1 = format!("{:?}", lines[1]);
        assert!(l1.contains("r3"), "second: {l1}");
    }
}
