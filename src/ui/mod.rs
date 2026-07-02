pub mod chrome;
pub mod dashboard;
pub mod diff_view;
pub mod layout;
pub mod overlay;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Tabs},
    Frame,
};

use crate::app::{App, Dialog, Mode};

/// Truncate a string to at most `max` chars, appending "…" if clipped. Trailing
/// whitespace is trimmed first. Shared display helper across the views/overlays.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let s = s.trim_end();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}\u{2026}")
    }
}

/// Root view function — renders the entire UI for one frame.
pub fn view(frame: &mut Frame, app: &App) {
    let area = frame.area();
    // Record the terminal width each frame so the model layer can make
    // responsive layout decisions (two-pane vs three-pane dashboard) without
    // re-querying the terminal.
    app.ui.width.set(area.width);
    // Clear stale panel/tab rects from the previous frame so the mouse handler
    // never sees areas from a layout that's no longer active. Must happen BEFORE
    // render_mode_tabs records the tab rect (otherwise render_main would wipe it).
    app.ui.reset_rects();
    // ── Overall layout ───────────────────────────────────────────────────────
    // [ title bar  ]
    // [ mode tabs  ]
    // [ main area  ]
    // [ status bar ]
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title bar
            Constraint::Length(3), // mode tabs (with top/bottom rule)
            Constraint::Min(0),    // main content
            Constraint::Length(1), // status bar
        ])
        .split(area);

    render_title_bar(frame, root[0], app);
    render_mode_tabs(frame, root[1], app);
    render_main(frame, root[2], app);
    chrome::statusbar::render(frame, root[3], app);

    // ── Overlays (dialogs) ───────────────────────────────────────────────────
    // Every input/menu dialog renders as a centered modal so the user always
    // sees what they are typing (these used to be invisible).
    use overlay::modal::{self, Field};
    let theme = &app.theme;
    match &app.dialog {
        Dialog::Commit(draft) => modal::render_text_input(
            frame,
            area,
            theme,
            "Commit message",
            &[Field {
                label: "",
                value: draft.as_str(),
                focused: true,
            }],
            "Enter: commit   Esc: cancel",
        ),
        Dialog::Amend(draft) => modal::render_text_input(
            frame,
            area,
            theme,
            "Amend last commit",
            &[Field {
                label: "",
                value: draft.as_str(),
                focused: true,
            }],
            "Enter: amend   Esc: cancel   (folds in staged changes)",
        ),
        Dialog::NewBranch(draft) => modal::render_text_input(
            frame,
            area,
            theme,
            "New branch",
            &[Field {
                label: "",
                value: draft.as_str(),
                focused: true,
            }],
            "branch name (from current HEAD)   Enter: create   Esc: cancel",
        ),
        Dialog::RenameBranch { new, .. } => modal::render_text_input(
            frame,
            area,
            theme,
            "Rename branch",
            &[Field {
                label: "",
                value: new.as_str(),
                focused: true,
            }],
            "new name   Enter: rename   Esc: cancel",
        ),
        Dialog::WorktreeAdd(draft) => modal::render_text_input(
            frame,
            area,
            theme,
            "Add worktree",
            &[Field {
                label: "",
                value: draft.as_str(),
                focused: true,
            }],
            "path — new branch = last path component   Enter: add   Esc: cancel",
        ),
        Dialog::StashSave(draft) => modal::render_text_input(
            frame,
            area,
            theme,
            "Stash changes",
            &[Field {
                label: "",
                value: draft.as_str(),
                focused: true,
            }],
            "message (optional)   Enter: stash   Esc: cancel",
        ),
        Dialog::TagCreate {
            name,
            message,
            focus_message,
        } => modal::render_text_input(
            frame,
            area,
            theme,
            "Create tag",
            &[
                Field {
                    label: "name",
                    value: name.as_str(),
                    focused: !*focus_message,
                },
                Field {
                    label: "message",
                    value: message.as_str(),
                    focused: *focus_message,
                },
            ],
            "Tab: switch field   Enter: create   Esc: cancel   (empty message = lightweight)",
        ),
        Dialog::ResetMenu { target } => {
            let short = crate::git::short_oid(target);
            let body = vec![Line::from(Span::styled(
                format!("Reset HEAD to {short}"),
                Style::default().fg(theme.fg),
            ))];
            let hint = Line::from(vec![
                Span::styled(
                    "[s]",
                    Style::default()
                        .fg(theme.staged)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("oft  ", Style::default().fg(theme.dim)),
                Span::styled(
                    "[m]",
                    Style::default()
                        .fg(theme.unstaged)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("ixed  ", Style::default().fg(theme.dim)),
                Span::styled(
                    "[h]",
                    Style::default()
                        .fg(theme.removed)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("ard   ", Style::default().fg(theme.dim)),
                Span::styled("Esc: cancel", Style::default().fg(theme.dim)),
            ]);
            modal::render_menu(frame, area, theme, "Reset", body, hint, false);
        }
        Dialog::InspectRef(_) => crate::features::inspect::view::render_prompt(frame, area, app),
        Dialog::Confirm { .. } => overlay::confirm_dialog::render(frame, area, app),
        Dialog::CompareBranches {
            base,
            target,
            focus_target,
        } => {
            // Show both base and target fields in one dialog, with Tab to switch.
            let fields = vec![
                Field {
                    label: "base",
                    value: base.as_str(),
                    focused: !*focus_target,
                },
                Field {
                    label: "target",
                    value: target.as_str(),
                    focused: *focus_target,
                },
            ];
            // Show a preview of matching branches for the focused field.
            let active_query = if *focus_target { target } else { base };
            let filtered: Vec<&str> = app
                .branches
                .iter()
                .filter(|b| b.kind == crate::git::RefKind::LocalBranch)
                .filter(|b| b.name.to_lowercase().contains(&active_query.to_lowercase()))
                .map(|b| b.name.as_str())
                .collect();
            let hint = if filtered.is_empty() {
                "Tab: switch field   Esc: cancel"
            } else {
                "Tab: switch field   Enter: compare   Esc: cancel"
            };
            modal::render_text_input(frame, area, theme, "Compare branches", &fields, hint);
            // Draw the branch list preview below the input fields.
            if !filtered.is_empty() {
                let body: Vec<Line> = filtered
                    .iter()
                    .take(8)
                    .map(|name| {
                        let style = if *name == filtered.first().copied().unwrap_or("") {
                            Style::default()
                                .fg(theme.focus_border)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme.fg)
                        };
                        Line::from(Span::styled(format!("  {name}"), style))
                    })
                    .collect();
                let preview_area = Rect {
                    y: area.y + 5,
                    height: body.len() as u16 + 2,
                    ..area
                };
                modal::render_menu(
                    frame,
                    preview_area,
                    theme,
                    "Matches",
                    body,
                    Line::from(""),
                    false,
                );
            }
        }
        Dialog::None => {}
    }

    // ── Interactive rebase overlay ───────────────────────────────────────────
    if app.rebase_todo.is_some() {
        crate::features::graph::rebase_todo::render(frame, area, app);
    }

    // ── Phase-4 overlays: search bar, command palette, help ──────────────────
    // Priority (topmost wins):  help > command_palette > search_bar
    // Draw in ascending priority order so the highest-priority one is on top.
    // Only one of help / palette is shown at a time; the search bar is a thin
    // bottom bar suppressed when help or palette is active (keeps UI uncluttered).

    // Search bar — one-line overlay at the very bottom of the frame.
    let show_search = app.search.is_some() && !app.show_help && app.palette.is_none();
    if show_search {
        render_search_bar(frame, area, app);
    }

    // Command palette (centered modal).
    if app.palette.is_some() && !app.show_help {
        overlay::command_palette::render(frame, area, app);
    }

    // Help panel — topmost; hides everything else conceptually.
    if app.show_help {
        overlay::help::render(frame, area, app);
    }
}

// ─── Search bar ──────────────────────────────────────────────────────────────

/// Render a one-line search bar at the very bottom of the frame (overlays the
/// status bar row).
///
/// Format:  /‹query›▊  ‹current›/‹total› matches
fn render_search_bar(frame: &mut Frame, area: Rect, app: &App) {
    let Some(ref search) = app.search else {
        return;
    };

    let theme = &app.theme;

    // The bar sits in the last row of the terminal area.
    let bar_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };

    if bar_area.height == 0 || bar_area.width == 0 {
        return;
    }

    // Build match count text.
    let match_text = if search.matches.is_empty() {
        " no matches ".to_string()
    } else {
        format!(
            " {}/{} ",
            search.current.saturating_add(1),
            search.matches.len()
        )
    };

    let match_color = if search.matches.is_empty() {
        theme.removed
    } else {
        theme.staged
    };

    frame.render_widget(Clear, bar_area);

    let bar = Paragraph::new(Line::from(vec![
        Span::styled(
            " /",
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(search.query.clone(), Style::default().fg(theme.fg)),
        Span::styled(
            "\u{2588}", // block cursor ▊
            Style::default().fg(theme.focus_border),
        ),
        Span::styled("  ", Style::default().fg(theme.dim)),
        Span::styled(
            match_text,
            Style::default()
                .fg(match_color)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .style(Style::default().bg(theme.bg));

    frame.render_widget(bar, bar_area);
}

// ─── Title bar ───────────────────────────────────────────────────────────────

fn render_title_bar(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;

    let branch = app.repo.status.branch.as_deref().unwrap_or("(no branch)");

    let root_path = app
        .repo
        .backend
        .root()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?");

    let ahead = app.repo.status.ahead;
    let behind = app.repo.status.behind;

    let mut title_spans = vec![
        Span::styled(
            " giv ",
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ ", Style::default().fg(theme.dim)),
        Span::styled(root_path, Style::default().fg(theme.fg)),
        Span::styled("  ⎇ ", Style::default().fg(theme.dim)),
        Span::styled(
            branch,
            Style::default().fg(theme.head).add_modifier(Modifier::BOLD),
        ),
    ];
    if ahead > 0 {
        title_spans.push(Span::styled(
            format!(" ↑{}", ahead),
            Style::default().fg(theme.staged),
        ));
    }
    if behind > 0 {
        title_spans.push(Span::styled(
            format!(" ↓{}", behind),
            Style::default()
                .fg(theme.removed)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let title = Line::from(title_spans);
    let bar = Paragraph::new(title).style(Style::default().bg(theme.bg));
    frame.render_widget(bar, area);
}

// ─── Mode tabs ───────────────────────────────────────────────────────────────

fn render_mode_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;

    let mode_names = [
        "1:Status",
        "2:Graph",
        "3:Branches",
        "4:Worktrees",
        "5:Stashes",
        "6:Inspect",
    ];
    let selected = match app.mode {
        Mode::Status => 0,
        Mode::Graph => 1,
        Mode::Branches => 2,
        Mode::Worktrees => 3,
        Mode::Stashes => 4,
        Mode::Inspect => 5,
    };

    let titles: Vec<Line> = mode_names
        .iter()
        .map(|&name| Line::from(Span::raw(name)))
        .collect();

    // Wrap the tab strip in a top+bottom rule so it reads as a distinct bar,
    // separated from the title above and the content below.
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Compute each tab's x range for mouse click detection.
    // ratatui's Tabs renders left-to-right: padding(1) + title + padding(1) + divider(3)
    // (no divider after the last tab). We mirror that layout here.
    let divider_w: u16 = 3; // "   "
    let pad_w: u16 = 1; // " "
    let mut ranges = [None; 6];
    let mut x = inner.x;
    for (i, name) in mode_names.iter().enumerate() {
        let title_w = name.chars().count() as u16;
        let tab_w = pad_w + title_w + pad_w;
        let end = x + tab_w;
        ranges[i] = Some((x, end));
        x = end;
        if i < mode_names.len() - 1 {
            x += divider_w;
        }
    }
    app.ui
        .tab_strip
        .set(crate::app::TabStrip { y: inner.y, ranges });

    let tabs = Tabs::new(titles)
        .select(selected)
        .style(Style::default().fg(theme.dim).bg(theme.bg))
        .highlight_style(
            Style::default()
                .fg(theme.focus_border)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled("   ", Style::default().fg(theme.dim)))
        .padding(" ", " ");

    frame.render_widget(tabs, inner);
}

// ─── Main content area ───────────────────────────────────────────────────────

fn render_main(frame: &mut Frame, area: Rect, app: &App) {
    // Wide terminals show the three-pane dashboard for Status and Graph; every
    // other mode (and narrow terminals) keep the mode's own two-pane layout.
    if matches!(
        app.ui.pane_layout(app.mode),
        crate::ui::layout::PaneLayout::ThreePane
    ) {
        crate::ui::dashboard::render(frame, area, app);
        return;
    }
    match app.mode {
        Mode::Status => crate::features::status::view::render(frame, area, app),
        Mode::Graph => crate::features::graph::view::render(frame, area, app),
        Mode::Branches => crate::features::branches::view::render(frame, area, app),
        Mode::Worktrees => crate::features::worktrees::view::render(frame, area, app),
        Mode::Stashes => crate::features::stashes::view::render(frame, area, app),
        Mode::Inspect => crate::features::inspect::view::render(frame, area, app),
    }
}
