use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, Mode};
use crate::git::OpKind;

/// Render the bottom status bar.
///
/// Layout (priority order — first matching rule wins):
///  1. Operation-in-progress banner (merge / rebase / cherry-pick / revert)
///  2. Background task spinner
///  3. Status / error message
///  4. Normal: [ key hints (mode-specific) ]
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;

    // ── 1. Operation-in-progress banner ────────────────────────────────────────
    if let Some(ref op) = app.op_in_progress {
        let op_label = match op.kind {
            OpKind::Merge => "MERGING",
            OpKind::Rebase => "REBASING",
            OpKind::CherryPick => "CHERRY-PICK",
            OpKind::Revert => "REVERTING",
        };

        let conflict_count = op.conflicted.len();
        let conflict_text = if conflict_count == 1 {
            format!("{conflict_count} conflict")
        } else {
            format!("{conflict_count} conflicts")
        };

        // Warning color: use `removed` (red/pink) for prominence.
        let warn_style = Style::default()
            .fg(theme.removed)
            .add_modifier(Modifier::BOLD);
        let dim_style = Style::default().fg(theme.dim);
        let key_style = Style::default()
            .fg(theme.unstaged)
            .add_modifier(Modifier::BOLD);

        let bar = Paragraph::new(Line::from(vec![
            Span::styled(" \u{26a0} ", warn_style),
            Span::styled(op_label, warn_style),
            Span::styled("  \u{2014} ", dim_style),
            Span::styled(conflict_text, Style::default().fg(theme.unstaged)),
            Span::styled("  ", dim_style),
            Span::styled("[C]", key_style),
            Span::styled("ontinue  ", dim_style),
            Span::styled("[A]", key_style),
            Span::styled("bort  ", dim_style),
            Span::styled("[S]", key_style),
            Span::styled("kip ", dim_style),
        ]))
        .style(Style::default().bg(theme.bg));
        frame.render_widget(bar, area);
        return;
    }

    // ── 2. Background task spinner ──────────────────────────────────────────────
    if let Some(ref task) = app.running_task {
        let bar = Paragraph::new(Line::from(vec![
            Span::styled(
                " \u{27f3} ",
                Style::default().fg(theme.head).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{task}\u{2026}"), Style::default().fg(theme.dim)),
        ]))
        .style(Style::default().bg(theme.bg));
        frame.render_widget(bar, area);
        return;
    }

    // ── 3. Status / error message ────────────────────────────────────────────────
    if let Some(msg) = &app.status_message {
        let bar = Paragraph::new(Line::from(vec![
            Span::styled(
                " \u{26a0} ",
                Style::default()
                    .fg(theme.unstaged)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg.as_str(), Style::default().fg(theme.unstaged)),
        ]))
        .style(Style::default().bg(theme.bg));
        frame.render_widget(bar, area);
        return;
    }

    // ── 4. Normal: key hints ───────────────────────────────────────────────────
    let hints = mode_hints(app);
    let bar = Paragraph::new(hints).style(Style::default().bg(theme.bg).fg(theme.fg));
    frame.render_widget(bar, area);
}

// ─── Mode-specific key hint builders ─────────────────────────────────────────

fn mode_hints(app: &App) -> Line<'static> {
    let theme = &app.theme;

    // Common hints always shown.
    let mut spans: Vec<Span> = vec![
        key(theme, "q"),
        hint(theme, ":quit "),
        key(theme, "r"),
        hint(theme, ":refresh "),
        key(theme, "1-6"),
        hint(theme, ":mode "),
        key(theme, "tab"),
        hint(theme, ":focus "),
        key(theme, "/"),
        hint(theme, ":find "),
        key(theme, ":"),
        hint(theme, ":cmds "),
        key(theme, "?"),
        hint(theme, ":help"),
    ];

    // Whether the diff panel has focus (Status/Graph, any layout) — then
    // movement scrolls the diff, so the hints change accordingly. Delegates to
    // the model's layout-aware check instead of assuming a concrete panel.
    let diff_focus = crate::update::diff_focused(app);

    let op_hints = |spans: &mut Vec<Span<'static>>| {
        if app.op_in_progress.is_some() {
            spans.push(hint(theme, " "));
            spans.push(key(theme, "C"));
            spans.push(hint(theme, ":continue "));
            spans.push(key(theme, "A"));
            spans.push(hint(theme, ":abort "));
            spans.push(key(theme, "S"));
            spans.push(hint(theme, ":skip"));
        }
    };

    // Mode-specific extras appended after common hints.
    match app.mode {
        Mode::Status => {
            spans.push(hint(theme, "  \u{2502} "));
            if diff_focus {
                spans.push(key(theme, "\u{2191}\u{2193}"));
                spans.push(hint(theme, ":scroll diff "));
                spans.push(key(theme, "tab"));
                spans.push(hint(theme, ":back to files"));
            } else {
                spans.push(key(theme, "space"));
                spans.push(hint(theme, ":stage/unstage "));
                spans.push(key(theme, "a"));
                spans.push(hint(theme, ":all "));
                spans.push(key(theme, "c"));
                spans.push(hint(theme, ":commit "));
                spans.push(key(theme, "e"));
                spans.push(hint(theme, ":amend "));
                spans.push(key(theme, "s"));
                spans.push(hint(theme, ":stash"));
            }
            op_hints(&mut spans);
        }
        Mode::Graph => {
            spans.push(hint(theme, "  \u{2502} "));
            if diff_focus {
                spans.push(key(theme, "\u{2191}\u{2193}"));
                spans.push(hint(theme, ":scroll diff "));
                spans.push(key(theme, "tab"));
                spans.push(hint(theme, ":back to commits"));
            } else {
                spans.push(key(theme, "Enter"));
                spans.push(hint(theme, ":diff "));
                spans.push(key(theme, "c"));
                spans.push(hint(theme, ":cherry-pick "));
                spans.push(key(theme, "v"));
                spans.push(hint(theme, ":revert "));
                spans.push(key(theme, "x"));
                spans.push(hint(theme, ":reset "));
                spans.push(key(theme, "b"));
                spans.push(hint(theme, ":rebase "));
                spans.push(key(theme, "y"));
                spans.push(hint(theme, ":copy-sha "));
                spans.push(key(theme, "<>"));
                spans.push(hint(theme, ":resize "));
                spans.push(key(theme, "a"));
                spans.push(hint(theme, ":scope "));
                spans.push(key(theme, "m"));
                spans.push(hint(theme, ":fold "));
                spans.push(key(theme, "l"));
                spans.push(hint(theme, ":lens"));
            }
            op_hints(&mut spans);
        }
        Mode::Branches => {
            spans.push(hint(theme, "  \u{2502} "));
            spans.push(key(theme, "Enter"));
            spans.push(hint(theme, ":checkout "));
            spans.push(key(theme, "n"));
            spans.push(hint(theme, ":new "));
            spans.push(key(theme, "R"));
            spans.push(hint(theme, ":rename "));
            spans.push(key(theme, "d"));
            spans.push(hint(theme, ":delete "));
            spans.push(key(theme, "m"));
            spans.push(hint(theme, ":merge "));
            spans.push(key(theme, "r"));
            spans.push(hint(theme, ":rebase "));
            spans.push(key(theme, "f"));
            spans.push(hint(theme, ":fetch "));
            spans.push(key(theme, "F"));
            spans.push(hint(theme, ":pull "));
            spans.push(key(theme, "P"));
            spans.push(hint(theme, ":push"));
            op_hints(&mut spans);
        }
        Mode::Worktrees => {
            spans.push(hint(theme, "  \u{2502} "));
            spans.push(key(theme, "Enter"));
            spans.push(hint(theme, ":cd "));
            spans.push(key(theme, "a"));
            spans.push(hint(theme, ":add "));
            spans.push(key(theme, "d"));
            spans.push(hint(theme, ":remove "));
            spans.push(key(theme, "p"));
            spans.push(hint(theme, ":prune"));
        }
        Mode::Stashes => {
            spans.push(hint(theme, "  \u{2502} "));
            spans.push(key(theme, "s"));
            spans.push(hint(theme, ":save "));
            spans.push(key(theme, "Enter"));
            spans.push(hint(theme, ":apply "));
            spans.push(key(theme, "p"));
            spans.push(hint(theme, ":pop "));
            spans.push(key(theme, "d"));
            spans.push(hint(theme, ":drop"));
        }
        Mode::Inspect => {
            spans.push(hint(theme, "  \u{2502} "));
            spans.push(key(theme, "i"));
            spans.push(hint(theme, ":enter ref "));
            spans.push(key(theme, "\u{2191}\u{2193}"));
            spans.push(hint(theme, ":scroll "));
            spans.push(key(theme, "y"));
            spans.push(hint(theme, ":copy-sha"));
        }
    }

    Line::from(spans)
}

fn key(theme: &crate::theme::Theme, text: &'static str) -> Span<'static> {
    Span::styled(
        text,
        Style::default()
            .fg(theme.focus_border)
            .add_modifier(Modifier::BOLD),
    )
}

fn hint(theme: &crate::theme::Theme, text: &'static str) -> Span<'static> {
    Span::styled(text, Style::default().fg(theme.dim))
}
