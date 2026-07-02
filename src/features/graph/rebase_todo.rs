/// Interactive-rebase todo-editor overlay.
///
/// Rendered on top of whatever mode is active when `app.rebase_todo.is_some()`.
/// Displays a centered panel listing all todo entries with the cursor row
/// highlighted and commands colour-coded.  A header shows the base commit and
/// a footer explains the key bindings.
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::App;

// ─── State ─────────────────────────────────────────────────────────────────────

/// A single entry in the interactive-rebase todo list.
#[derive(Debug, Clone)]
pub struct RebaseTodoEntry {
    /// The rebase command: pick | reword | edit | squash | fixup | drop.
    pub command: String,
    /// Full commit OID (used when invoking `rebase_interactive`).
    pub oid: String,
    /// One-line commit summary (display only).
    pub summary: String,
}

/// State for the interactive-rebase todo-editor overlay. Held by
/// [`crate::app::App`] in its `rebase_todo` field; `Some` when the overlay is open.
#[derive(Debug, Clone)]
pub struct RebaseTodoState {
    /// The todo list, ordered oldest-first (as git expects).
    pub entries: Vec<RebaseTodoEntry>,
    /// Index of the currently highlighted entry.
    pub cursor: usize,
    /// The base commit OID / ref that was passed to `rebase -i <base>`.
    pub base: String,
}

// ─── View ──────────────────────────────────────────────────────────────────────

/// Main entry point — called from `ui::view` when `app.rebase_todo.is_some()`.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(ref state) = app.rebase_todo else {
        return;
    };

    let theme = &app.theme;

    // ── Panel sizing ─────────────────────────────────────────────────────────
    // Use 80% width, tall enough for all entries + header/footer, capped at the
    // frame height.
    let entry_count = state.entries.len();
    // 2 border rows + 1 title + 1 blank + entries + 1 blank + 1 footer = N+5
    let content_height = (entry_count + 5).min(area.height as usize - 2) as u16;
    let panel_width = (area.width * 85 / 100).max(40).min(area.width);
    let panel_height = content_height.max(8);

    let dialog_area = centered_rect(panel_width, panel_height, area);

    // ── Block / border ───────────────────────────────────────────────────────
    let title = format!(
        " Interactive rebase onto {} ",
        crate::git::short_oid(&state.base)
    );

    let block = Block::default()
        .title(title.as_str())
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.focus_border))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(dialog_area);

    frame.render_widget(Clear, dialog_area);
    frame.render_widget(block, dialog_area);

    // ── Content lines ────────────────────────────────────────────────────────
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Blank padding line.
    lines.push(Line::from(""));

    // Entry rows.
    for (i, entry) in state.entries.iter().enumerate() {
        let is_selected = i == state.cursor;

        let cmd_color = command_color(&entry.command, theme);

        let short = crate::git::short_oid(&entry.oid);
        let summary = crate::ui::truncate(&entry.summary, 55);

        if is_selected {
            // Highlighted row.
            let row_bg = theme.focus_border;
            let text_fg = theme.bg;
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:<7} ", &entry.command),
                    Style::default()
                        .fg(cmd_color)
                        .bg(row_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} ", short),
                    Style::default().fg(text_fg).bg(row_bg),
                ),
                Span::styled(
                    format!("{} ", summary),
                    Style::default()
                        .fg(text_fg)
                        .bg(row_bg)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:<7} ", &entry.command),
                    Style::default().fg(cmd_color).bg(theme.bg),
                ),
                Span::styled(
                    format!("{} ", short),
                    Style::default().fg(theme.dim).bg(theme.bg),
                ),
                Span::styled(
                    format!("{} ", summary),
                    Style::default().fg(theme.fg).bg(theme.bg),
                ),
            ]));
        }
    }

    // Spacer + footer.
    lines.push(Line::from(""));
    lines.push(footer_line(theme));

    let para = Paragraph::new(lines)
        .style(Style::default().bg(theme.bg))
        .alignment(Alignment::Left);

    frame.render_widget(para, inner);
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Return the color for a given rebase command keyword.
fn command_color(cmd: &str, theme: &crate::theme::Theme) -> Color {
    match cmd {
        "drop" => theme.removed,
        "squash" | "fixup" => theme.unstaged,
        "reword" | "edit" => theme.head,
        _ => theme.added, // pick and anything else
    }
}

/// Build the footer line with key hints.
fn footer_line(theme: &crate::theme::Theme) -> Line<'static> {
    let key = |text: &'static str| {
        Span::styled(
            text,
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        )
    };
    let sep = || Span::styled("  ", Style::default().fg(theme.dim));
    let txt = |text: &'static str| Span::styled(text, Style::default().fg(theme.dim));

    Line::from(vec![
        Span::raw(" "),
        key("j/k"),
        txt(":move "),
        sep(),
        key("p"),
        txt(":pick "),
        key("r"),
        txt(":reword "),
        key("s"),
        txt(":squash "),
        key("f"),
        txt(":fixup "),
        key("d"),
        txt(":drop "),
        key("e"),
        txt(":edit"),
        sep(),
        key("J/K"),
        txt(":reorder"),
        sep(),
        key("Enter"),
        txt(":execute "),
        key("Esc"),
        txt(":cancel"),
    ])
}

/// Return a `Rect` centred in `r` with the given width and height.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    // ── command_color ────────────────────────────────────────────────────────

    #[test]
    fn command_color_drop_uses_removed() {
        let theme = Theme::from_name("tokyonight");
        assert_eq!(command_color("drop", &theme), theme.removed);
    }

    #[test]
    fn command_color_squash_uses_unstaged() {
        let theme = Theme::from_name("tokyonight");
        assert_eq!(command_color("squash", &theme), theme.unstaged);
    }

    #[test]
    fn command_color_fixup_uses_unstaged() {
        let theme = Theme::from_name("tokyonight");
        assert_eq!(command_color("fixup", &theme), theme.unstaged);
    }

    #[test]
    fn command_color_reword_uses_head() {
        let theme = Theme::from_name("tokyonight");
        assert_eq!(command_color("reword", &theme), theme.head);
    }

    #[test]
    fn command_color_edit_uses_head() {
        let theme = Theme::from_name("tokyonight");
        assert_eq!(command_color("edit", &theme), theme.head);
    }

    #[test]
    fn command_color_pick_uses_added() {
        let theme = Theme::from_name("tokyonight");
        assert_eq!(command_color("pick", &theme), theme.added);
    }

    #[test]
    fn command_color_unknown_command_uses_added() {
        // Unknown commands fall back to the "pick" color (added).
        let theme = Theme::from_name("tokyonight");
        assert_eq!(command_color("unknown", &theme), theme.added);
        assert_eq!(command_color("", &theme), theme.added);
    }

    // ── centered_rect ────────────────────────────────────────────────────────

    #[test]
    fn centered_rect_centers_in_large_area() {
        let r = Rect::new(0, 0, 100, 40);
        let inner = centered_rect(40, 10, r);
        // x = 0 + (100 - 40) / 2 = 30
        assert_eq!(inner.x, 30);
        // y = 0 + (40 - 10) / 2 = 15
        assert_eq!(inner.y, 15);
        assert_eq!(inner.width, 40);
        assert_eq!(inner.height, 10);
    }

    #[test]
    fn centered_rect_clamps_width_to_area() {
        let r = Rect::new(0, 0, 20, 40);
        let inner = centered_rect(100, 10, r);
        // width is clamped to 20.
        assert_eq!(inner.width, 20);
        assert_eq!(inner.height, 10);
    }

    #[test]
    fn centered_rect_clamps_height_to_area() {
        let r = Rect::new(0, 0, 100, 5);
        let inner = centered_rect(40, 50, r);
        assert_eq!(inner.width, 40);
        assert_eq!(inner.height, 5);
    }

    #[test]
    fn centered_rect_handles_zero_area() {
        let r = Rect::new(0, 0, 0, 0);
        let inner = centered_rect(40, 10, r);
        // saturating_sub prevents underflow; width/height clamped to 0.
        assert_eq!(inner.x, 0);
        assert_eq!(inner.y, 0);
        assert_eq!(inner.width, 0);
        assert_eq!(inner.height, 0);
    }

    #[test]
    fn centered_rect_with_offset_origin() {
        let r = Rect::new(10, 20, 100, 40);
        let inner = centered_rect(40, 10, r);
        // x = 10 + (100 - 40) / 2 = 40
        assert_eq!(inner.x, 40);
        // y = 20 + (40 - 10) / 2 = 35
        assert_eq!(inner.y, 35);
    }

    #[test]
    fn centered_rect_equal_size_fills_area() {
        let r = Rect::new(0, 0, 40, 10);
        let inner = centered_rect(40, 10, r);
        assert_eq!(inner, r);
    }

    // ── RebaseTodoState / RebaseTodoEntry ────────────────────────────────────

    #[test]
    fn rebase_todo_entry_construction() {
        let e = RebaseTodoEntry {
            command: "pick".into(),
            oid: "abc123".into(),
            summary: "fix bug".into(),
        };
        assert_eq!(e.command, "pick");
        assert_eq!(e.oid, "abc123");
        assert_eq!(e.summary, "fix bug");
    }

    #[test]
    fn rebase_todo_state_construction() {
        let s = RebaseTodoState {
            entries: vec![RebaseTodoEntry {
                command: "pick".into(),
                oid: "abc".into(),
                summary: "s".into(),
            }],
            cursor: 0,
            base: "base123".into(),
        };
        assert_eq!(s.entries.len(), 1);
        assert_eq!(s.cursor, 0);
        assert_eq!(s.base, "base123");
    }
}
