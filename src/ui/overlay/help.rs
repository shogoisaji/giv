/// Help overlay.
///
/// Rendered on top of all other content when `app.show_help` is `true`.
/// Displays keybindings grouped by context in a scrollable panel.
/// Renders nothing when `show_help` is `false`.
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::App;

/// Main entry point — called from `ui::view` when `app.show_help`.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if !app.show_help {
        return;
    }

    let theme = &app.theme;

    // ── Panel sizing: 70% wide, 80% tall ─────────────────────────────────────
    let dialog_width = (area.width * 70 / 100).max(60).min(area.width);
    let dialog_height = (area.height * 80 / 100).max(20).min(area.height);
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    // ── Shadow ────────────────────────────────────────────────────────────────
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
    if shadow_area.width > 0 && shadow_area.height > 0 {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.border)),
            shadow_area,
        );
    }

    frame.render_widget(Clear, dialog_area);

    let block = Block::default()
        .title(" Help  (Esc / ? to close) ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.focus_border))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    if inner.height < 2 {
        return;
    }

    // ── Two-column layout ─────────────────────────────────────────────────────
    // Left column: Global + Status + Graph
    // Right column: Branches + Worktrees + Stashes + Dialogs/Rebase
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // ── Helper closures ───────────────────────────────────────────────────────
    let section_style = Style::default()
        .fg(theme.head)
        .add_modifier(Modifier::BOLD)
        .add_modifier(Modifier::UNDERLINED);
    let key_style = Style::default()
        .fg(theme.focus_border)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(theme.fg);
    let dim_style = Style::default().fg(theme.dim);

    let section = |s: &'static str| Line::from(Span::styled(s, section_style));
    let blank = || Line::from("");
    let row = |k: &'static str, d: &'static str| {
        // Key field is left-padded 2 spaces, fixed 14 chars wide; description follows.
        Line::from(vec![
            Span::styled(format!("  {:<14}", k), key_style),
            Span::styled(d, desc_style),
        ])
    };
    let note = |d: &'static str| {
        Line::from(vec![
            Span::styled("  ", dim_style),
            Span::styled(d, dim_style),
        ])
    };

    // ── Left column content ───────────────────────────────────────────────────
    let left_lines: Vec<Line<'static>> = vec![
        // ── Global ──────────────────────────────────────────────────────────
        section("\u{2500} Global "),
        row("q", "Quit"),
        row("r", "Refresh"),
        row("?", "Toggle this help"),
        row("1", "Switch to Status mode"),
        row("2", "Switch to Graph mode"),
        row("3", "Switch to Branches mode"),
        row("4", "Switch to Worktrees mode"),
        row("5", "Switch to Stashes mode"),
        row("6", "Switch to Inspect mode"),
        row("Tab", "Switch focus panel"),
        row(
            "M",
            "Toggle mouse (default off = select text; on = scroll/click)",
        ),
        row("C", "Continue in-progress op"),
        row("A", "Abort in-progress op"),
        row("S", "Skip current commit of op"),
        row("T", "Cycle theme (saved)"),
        row(":", "Command palette"),
        row("/", "Search / filter"),
        blank(),
        // ── Navigation (all modes) ───────────────────────────────────────────
        section("\u{2500} Navigation "),
        row("j / \u{2193}", "Move down"),
        row("k / \u{2191}", "Move up"),
        row("g", "Jump to top"),
        row("G", "Jump to bottom"),
        row("PgDn", "Page down"),
        row("PgUp", "Page up"),
        row("Enter", "Select / show diff"),
        blank(),
        // ── Status mode ──────────────────────────────────────────────────────
        section("\u{2500} Status mode "),
        row("Space", "Stage / unstage selected"),
        row("a", "Stage all changes"),
        row("A", "Unstage all changes"),
        row("u", "Unstage selected"),
        row("c", "Open commit dialog"),
        row("e", "Amend last commit"),
        row("d / Ctrl-u", "Scroll diff down / up"),
        row("R", "Mark conflict as resolved"),
        row("s", "Stash save (prompt)"),
        row("t", "Create tag"),
        row("f / F / P", "Fetch / Pull / Push"),
        row("X", "Force push (with lease)"),
    ];

    // ── Right column content ──────────────────────────────────────────────────
    let right_lines: Vec<Line<'static>> = vec![
        // ── Graph mode ───────────────────────────────────────────────────────
        section("\u{2500} Graph mode "),
        row("Enter", "Show commit diff"),
        row("c", "Cherry-pick selected commit"),
        row("v", "Revert selected commit"),
        row("x", "Reset menu (soft/mixed/hard)"),
        row("b", "Rebase onto selected commit"),
        row("i", "Interactive rebase from here"),
        row("t", "Create tag on selected commit"),
        row("D", "Delete selected tag"),
        row("y", "Copy commit SHA"),
        row("< / >", "Resize graph / detail split"),
        row("f / F / P", "Fetch / Pull / Push"),
        blank(),
        // ── Branches mode ────────────────────────────────────────────────────
        section("\u{2500} Branches mode "),
        row("Enter / Space", "Checkout branch"),
        row("n", "New branch (dialog)"),
        row("R", "Rename branch"),
        row("d", "Delete branch"),
        row("m", "Merge into HEAD"),
        row("r", "Rebase HEAD onto branch"),
        row("f / F / P", "Fetch / Pull / Push"),
        blank(),
        // ── Worktrees mode ───────────────────────────────────────────────────
        section("\u{2500} Worktrees mode "),
        row("Enter", "cd into worktree"),
        row("a", "Add worktree (dialog)"),
        row("d", "Remove worktree"),
        row("p", "Prune stale worktrees"),
        blank(),
        // ── Stashes mode ─────────────────────────────────────────────────────
        section("\u{2500} Stashes mode "),
        row("Enter / Space", "Apply stash (keep)"),
        row("p", "Pop stash (apply + drop)"),
        row("d", "Drop stash"),
        row("s", "Stash save (prompt)"),
        blank(),
        // ── Inspect mode ─────────────────────────────────────────────────────
        section("\u{2500} Inspect mode "),
        row("i / Enter", "Enter ref prompt (input mode)"),
        row("Esc", "Leave input mode"),
        row("j / k / \u{2191}\u{2193}", "Scroll commit detail"),
        row("y", "Copy inspected commit SHA"),
        blank(),
        // ── Interactive rebase overlay ────────────────────────────────────────
        section("\u{2500} Interactive rebase "),
        row("j / k", "Move cursor"),
        row("J / K", "Reorder entries"),
        row("p", "Set command: pick"),
        row("r", "Set command: reword"),
        row("e", "Set command: edit"),
        row("s", "Set command: squash"),
        row("f", "Set command: fixup"),
        row("d", "Set command: drop"),
        row("Enter", "Execute rebase"),
        row("Esc / q", "Cancel rebase"),
        blank(),
        // ── Dialog bindings ───────────────────────────────────────────────────
        section("\u{2500} Dialogs "),
        note("Enter: confirm / submit   Esc: cancel"),
        note("Backspace: delete char"),
        note("Tab: switch field (tag name/msg)"),
    ];

    // Render both columns.
    frame.render_widget(
        Paragraph::new(left_lines).style(Style::default().bg(theme.bg).fg(theme.fg)),
        cols[0],
    );
    frame.render_widget(
        Paragraph::new(right_lines).style(Style::default().bg(theme.bg).fg(theme.fg)),
        cols[1],
    );
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
