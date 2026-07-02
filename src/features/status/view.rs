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
use crate::git::types::{FileStatus, StatusCode};

// ─── Flattened index scheme ───────────────────────────────────────────────────
//
// The status list is rendered as two groups:
//
//   [0]         "Staged (n)"   ← group header, not selectable
//   [1..n]      staged files   ← one entry per staged file
//   [n+1]       "Unstaged (m)" ← group header, not selectable
//   [n+2..n+1+m] unstaged / untracked files
//
// `app.ui.list_index` is a *logical* index that counts only the **file entries**
// (not the headers):
//
//   logical 0..staged.len()-1        → staged files
//   logical staged.len()..total-1   → unstaged + untracked files
//
// Rendering converts logical → display row by inserting header offsets.
// Callers (update agent) map `list_index` to a file via `resolve_entry`.

/// Returns (staged_files, unstaged_files) partitioning the working-tree entries.
pub fn partition_entries(app: &App) -> (Vec<&FileStatus>, Vec<&FileStatus>) {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();

    for entry in &app.repo.status.entries {
        // An entry can appear in both panels if it has both staged and unstaged changes.
        if entry.is_staged() {
            staged.push(entry);
        }
        // Show in unstaged if it has worktree changes OR is untracked.
        if entry.is_unstaged() || entry.is_untracked() {
            unstaged.push(entry);
        }
    }
    (staged, unstaged)
}

/// Resolve `list_index` (logical file index) to the concrete `&FileStatus`.
/// Returns `None` when the index is out of range.
pub fn resolve_entry(app: &App, list_index: usize) -> Option<&FileStatus> {
    let (staged, unstaged) = partition_entries(app);
    if list_index < staged.len() {
        Some(staged[list_index])
    } else {
        let u_idx = list_index - staged.len();
        unstaged.get(u_idx).copied()
    }
}

/// Is the entry at `list_index` a staged file? (vs unstaged/untracked)
pub fn is_selected_staged(app: &App) -> bool {
    let (staged, _) = partition_entries(app);
    app.ui.list_index < staged.len()
}

/// Number of selectable rows in the status list: the logical file entries
/// across both the Staged and Unstaged groups. A file with both staged and
/// unstaged changes counts twice (once per group), matching the rendered list,
/// so this can exceed `app.repo.status.entries.len()`.
pub fn status_row_count(app: &App) -> usize {
    let (staged, unstaged) = partition_entries(app);
    staged.len() + unstaged.len()
}

/// The *display row* (rendered list row, with group headers and the empty-group
/// placeholders counted) of the currently selected logical file index.
///
/// The Changes list interleaves non-selectable rows — the "Staged (n)" /
/// "Unstaged (m)" headers, plus a placeholder row when a group is empty — so a
/// file's display row is offset from its logical `list_index`. Auto-scroll
/// ([`crate::features::status::update::clamp_list_offset`]) needs the display
/// row to keep the selection on-screen. Must stay in sync with the row layout
/// built in [`render_file_list`].
pub fn selected_display_row(app: &App) -> usize {
    let (staged, unstaged) = partition_entries(app);
    let total = staged.len() + unstaged.len();
    if total == 0 {
        return 0;
    }
    let sel = app.ui.list_index.min(total - 1);
    if sel < staged.len() {
        // Staged group: header occupies row 0, files start at row 1.
        1 + sel
    } else {
        // Unstaged group. An empty Staged group still renders one placeholder
        // row, so its block is at least one row tall.
        let staged_block = staged.len().max(1);
        let u_idx = sel - staged.len();
        // staged header (1) + staged block + unstaged header (1) + file offset.
        2 + staged_block + u_idx
    }
}

/// Inverse of [`selected_display_row`]: convert a *display row* (the row index
/// in the rendered list, including group headers and placeholders) back to a
/// *logical file index* (0-based, counting only actual file entries). Used by
/// the mouse click handler to map a clicked row to the correct file.
///
/// Non-file rows (headers, placeholders) clamp to the nearest file index so a
/// click on a header selects the first file in that group.
pub fn display_row_to_logical(app: &App, display_row: usize) -> usize {
    let (staged, unstaged) = partition_entries(app);
    let total = staged.len() + unstaged.len();
    if total == 0 {
        return 0;
    }

    // Row 0 = "Staged (n)" header → clamp to first file.
    if display_row == 0 {
        return 0;
    }

    // Staged files occupy rows 1..=staged.len() (when non-empty).
    if !staged.is_empty() && display_row <= staged.len() {
        return display_row - 1;
    }

    // If staged is empty, row 1 = "(nothing staged)" placeholder → clamp to 0.
    if staged.is_empty() && display_row == 1 {
        return 0;
    }

    // Unstaged header is at row 1 + staged_block (staged_block = max(staged.len(), 1)).
    let staged_block = staged.len().max(1);
    let unstaged_header_row = 1 + staged_block;

    // Click on "Unstaged (m)" header → first unstaged file.
    if display_row == unstaged_header_row {
        return staged.len();
    }

    // If unstaged is empty, placeholder row → clamp to last file.
    if unstaged.is_empty() {
        return total.saturating_sub(1);
    }

    // Unstaged files start at unstaged_header_row + 1.
    let u_idx = display_row.saturating_sub(unstaged_header_row + 1);
    let logical = staged.len() + u_idx.min(unstaged.len().saturating_sub(1));
    logical.min(total.saturating_sub(1))
}

// ─── Rendering ───────────────────────────────────────────────────────────────

/// Render the Status mode: left panel (grouped file list) + right panel (diff).
/// The focused pane gets 65% of the width (focus-weighted split).
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

    render_file_list(frame, chunks[0], app, focused == Panel::Left);
    render_diff_panel(frame, chunks[1], app, focused == Panel::Main);
}

/// Wrap a line as a `ListItem`, indicating selection by background color ALONE
/// (no prefix glyph, no inverse, no bold) so rows never shift and stay readable.
fn selectable(
    line: Line<'static>,
    is_selected: bool,
    theme: &crate::theme::Theme,
) -> ListItem<'static> {
    let item = ListItem::new(line);
    if is_selected {
        item.style(Style::default().bg(theme.selection_bg))
    } else {
        item
    }
}

pub(crate) fn render_file_list(frame: &mut Frame, area: Rect, app: &App, is_focused: bool) {
    let theme = &app.theme;
    app.ui.record_rect(crate::app::RectSlot::Changes, area);

    let border_color = if is_focused {
        theme.focus_border
    } else {
        theme.border
    };
    let block = Block::default()
        .title(" Changes ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Record the visible height so navigation can auto-scroll to follow the cursor.
    app.ui.list_viewport.set(inner.height as usize);

    let (staged, unstaged) = partition_entries(app);
    let total_files = staged.len() + unstaged.len();

    if total_files == 0 {
        let p = Paragraph::new(" No changes").style(Style::default().fg(theme.dim).bg(theme.bg));
        frame.render_widget(p, inner);
        return;
    }

    let selected_logical = app.ui.list_index.min(total_files.saturating_sub(1));

    let mut items: Vec<ListItem> = Vec::new();

    // ── Staged group ─────────────────────────────────────────────────────────
    // Header row: "Staged (n)"
    items.push(ListItem::new(Line::from(vec![Span::styled(
        format!("  Staged ({}) ", staged.len()),
        Style::default()
            .fg(theme.staged)
            .add_modifier(Modifier::BOLD),
    )])));

    for (file_i, entry) in staged.iter().enumerate() {
        let is_selected = selected_logical == file_i; // staged files start at logical 0

        let code_char = status_letter_index(&entry.index);
        let display_path = if let Some(orig) = &entry.orig_path {
            format!("{} → {}", orig, entry.path)
        } else {
            entry.path.clone()
        };

        // Staged entries: green ●  (selection shown by background color only)
        let line = Line::from(vec![
            Span::styled("  ● ", Style::default().fg(theme.staged)),
            Span::styled(
                format!("{} {}", code_char, display_path),
                Style::default().fg(theme.staged),
            ),
        ]);
        items.push(selectable(line, is_selected, theme));
    }

    if staged.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "    (nothing staged)",
            Style::default().fg(theme.dim),
        ))));
    }

    // ── Unstaged group ───────────────────────────────────────────────────────
    // Header row: "Unstaged (m)"
    items.push(ListItem::new(Line::from(vec![Span::styled(
        format!("  Unstaged ({}) ", unstaged.len()),
        Style::default()
            .fg(theme.unstaged)
            .add_modifier(Modifier::BOLD),
    )])));

    for (file_i, entry) in unstaged.iter().enumerate() {
        let logical_i = staged.len() + file_i;
        let is_selected = selected_logical == logical_i;

        let is_untracked = entry.is_untracked();

        let display_path = if let Some(orig) = &entry.orig_path {
            format!("{} → {}", orig, entry.path)
        } else {
            entry.path.clone()
        };

        // Untracked files get a single, clear "new file" marker (`+`) with no
        // redundant status letter — previously they showed `? ?` which was
        // confusing. Tracked changes keep their worktree status letter (M/D/…).
        let (symbol, label) = if is_untracked {
            ("  + ".to_owned(), display_path.clone())
        } else {
            (
                "  ○ ".to_owned(),
                format!(
                    "{} {}",
                    status_letter_worktree(&entry.worktree),
                    display_path
                ),
            )
        };

        let entry_color = if is_untracked {
            theme.untracked
        } else {
            theme.unstaged
        };
        let style = Style::default().fg(entry_color);

        // Selection shown by background color only (no inverse, no bold).
        let line = Line::from(vec![
            Span::styled(symbol, style),
            Span::styled(label, style),
        ]);
        items.push(selectable(line, is_selected, theme));
    }

    if unstaged.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "    (nothing to stage)",
            Style::default().fg(theme.dim),
        ))));
    }

    // Scroll the rendered rows so the selection stays on-screen. `list_offset`
    // is maintained by `clamp_list_offset` after every selection change; the
    // `.min` here is a defensive bound in case the list shrank since.
    let total_rows = items.len();
    let offset = app.ui.list_offset.min(total_rows.saturating_sub(1));
    let visible: Vec<ListItem> = items.into_iter().skip(offset).collect();

    let list = List::new(visible).style(Style::default().bg(theme.bg));
    frame.render_widget(list, inner);

    // Scrollbar on the right edge when the list overflows the viewport — gives a
    // sense of position when there are many changes (mirrors the graph panel).
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

pub(crate) fn render_diff_panel(frame: &mut Frame, area: Rect, app: &App, is_focused: bool) {
    let theme = &app.theme;
    app.ui.record_rect(crate::app::RectSlot::Diff, area);

    let border_color = if is_focused {
        theme.focus_border
    } else {
        theme.border
    };
    let title = match &app.repo.selected_diff {
        Some(d) if !d.files.is_empty() => {
            format!(" Diff: {} ", d.files[0].new_path)
        }
        _ => " Diff ".to_owned(),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
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

#[allow(dead_code)]
fn build_diff_lines(app: &App) -> Vec<Line<'static>> {
    build_diff_lines_window(app, 0, usize::MAX)
}

/// Build only the diff lines in the viewport `[start, start+count)`.
///
/// Instead of allocating a `Vec<Line>` for the entire diff (which can be
/// 10 k+ lines for a large changeset), this function walks the diff
/// structure and emits only the lines that fall within the visible window.
/// Lines before `start` are counted but not allocated; iteration stops once
/// `count` lines have been emitted.
fn build_diff_lines_window(app: &App, start: usize, count: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let Some(diff) = &app.repo.selected_diff else {
        if start > 0 {
            return Vec::new();
        }
        return vec![Line::from(Span::styled(
            " Select a file to view its diff   (Space: stage / unstage   u: unstage)",
            Style::default().fg(theme.dim),
        ))];
    };

    if count == 0 {
        return Vec::new();
    }

    let mut lines = Vec::with_capacity(count.min(256));
    let mut line_no = 0usize;
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

            for dl in &hunk.lines {
                if line_no >= start && line_no < end {
                    use crate::git::types::DiffLineKind::*;
                    let line = match dl.kind {
                        Added => Line::from(Span::styled(
                            format!("+{}", dl.text.clone()),
                            Style::default().fg(theme.added),
                        )),
                        Removed => Line::from(Span::styled(
                            format!("-{}", dl.text.clone()),
                            Style::default().fg(theme.removed),
                        )),
                        Context => Line::from(Span::styled(
                            format!(" {}", dl.text.clone()),
                            Style::default().fg(theme.fg).add_modifier(Modifier::DIM),
                        )),
                        Header => Line::from(Span::styled(
                            dl.text.clone(),
                            Style::default().fg(theme.hunk),
                        )),
                        Meta => Line::from(Span::styled(
                            dl.text.clone(),
                            Style::default().fg(theme.dim),
                        )),
                    };
                    lines.push(line);
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

// ─── Status letter helpers ────────────────────────────────────────────────────

fn status_letter_index(code: &StatusCode) -> char {
    match code {
        StatusCode::Modified => 'M',
        StatusCode::Added => 'A',
        StatusCode::Deleted => 'D',
        StatusCode::Renamed => 'R',
        StatusCode::Copied => 'C',
        StatusCode::Conflicted => 'U',
        StatusCode::Untracked => '?',
        StatusCode::Ignored => '!',
        StatusCode::TypeChange => 'T',
        StatusCode::Unmodified => ' ',
    }
}

fn status_letter_worktree(code: &StatusCode) -> char {
    match code {
        StatusCode::Modified => 'M',
        StatusCode::Added => 'A',
        StatusCode::Deleted => 'D',
        StatusCode::Renamed => 'R',
        StatusCode::Copied => 'C',
        StatusCode::Conflicted => 'U',
        StatusCode::Untracked => '?',
        StatusCode::Ignored => '!',
        StatusCode::TypeChange => 'T',
        StatusCode::Unmodified => ' ',
    }
}
