/// crossterm event source.
///
/// Polls the terminal for input events and converts them to `Action`s using
/// the `Keymap`. Dialog-aware: when a dialog is open, text-input events are
/// forwarded as `DialogChar` / `DialogBackspace` / `SubmitCommit` /
/// `CancelDialog`.
use std::time::Duration;

use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};

use crate::action::Action;
use crate::app::App;
use crate::keymap::{Keymap, KeymapContext};

/// Poll for the next action with a `timeout`.
///
/// Returns `Ok(None)` if the timeout elapsed without an event.
/// Returns `Ok(Some(action))` on a key press.
/// Resize events return `Action::Refresh` so the frame is redrawn.
pub fn next_action(
    keymap: &Keymap,
    app: &App,
    timeout: Duration,
) -> anyhow::Result<Option<Action>> {
    if !event::poll(timeout)? {
        return Ok(None);
    }

    match event::read()? {
        Event::Key(key_event) => {
            // Only process key-press events (ignore repeat / release on some platforms).
            if key_event.kind == KeyEventKind::Release {
                return Ok(Some(Action::None));
            }

            // Universal quit: Ctrl-C / Ctrl-Q always quits, from ANY mode, overlay,
            // or dialog. In raw mode Ctrl-C arrives as a key (not a SIGINT), so we
            // must handle it explicitly — it is the guaranteed escape hatch.
            if key_event.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key_event.code, KeyCode::Char('c') | KeyCode::Char('q'))
            {
                return Ok(Some(Action::Quit));
            }

            // Interactive-rebase todo editor takes priority over all other bindings.
            let action = if app.rebase_todo.is_some() {
                keymap.resolve_rebase_todo(key_event)
            } else {
                keymap.resolve(
                    key_event,
                    KeymapContext {
                        mode: app.mode,
                        dialog: &app.dialog,
                        palette: app.palette.as_ref(),
                        search: app.search.as_ref(),
                        show_help: app.show_help,
                        op_in_progress: app.op_in_progress.is_some(),
                    },
                )
            };
            Ok(Some(action))
        }

        Event::Mouse(mouse_event) => {
            let action = handle_mouse(app, mouse_event);
            Ok(Some(action))
        }

        Event::Resize(_, _) => Ok(Some(Action::Refresh)),

        // Ignore focus events, etc.
        _ => Ok(Some(Action::None)),
    }
}

// ─── Mouse handling ───────────────────────────────────────────────────────────

fn handle_mouse(app: &App, event: crossterm::event::MouseEvent) -> Action {
    use crate::app::{Mode, Panel, RectSlot};
    use crate::ui::layout::{focus_target, FocusTarget, PaneLayout};

    // Command palette: a left click on an item row selects + runs it; a left
    // click anywhere else (dialog border, query line, off the palette) closes
    // the palette like Esc. Handle this before the overlay-ignore guard below
    // so palette clicks work even though the palette is an overlay. Wheel and
    // other non-left-click events fall through to None.
    if app.palette.is_some() && app.mouse_capture {
        if let MouseEventKind::Down(MouseButton::Left) = event.kind {
            if let Some(list_rect) = app.ui.palette_list_rect.get() {
                let click = ratatui::layout::Rect {
                    x: event.column,
                    y: event.row,
                    width: 1,
                    height: 1,
                };
                if list_rect.intersects(click) {
                    let row = event.row.saturating_sub(list_rect.y) as usize;
                    return Action::PaletteClick { row };
                }
            }
            // Click outside the palette item list — close the palette.
            return Action::ClosePalette;
        }
        // Any other mouse event while the palette is open is a no-op.
        return Action::None;
    }

    // Ignore mouse while an overlay is open, or when the user has turned mouse
    // capture off to select text (any buffered events must not move the UI).
    if app.search.is_some() || app.show_help || !app.mouse_capture {
        return Action::None;
    }

    let layout = app.ui.pane_layout(app.mode);
    let diff_focused = matches!(app.mode, Mode::Status | Mode::Graph)
        && matches!(
            focus_target(layout, app.mode, *app.ui.panel()),
            FocusTarget::Diff
        );

    match event.kind {
        // Wheel scrolls the focused panel's VIEW (display scroll) without moving
        // the selection — diff / graph / list all behave like a normal
        // scrollable area. Keyboard ↑/↓ still moves the selection (with
        // auto-scroll via `clamp_*_offset`).
        MouseEventKind::ScrollUp => {
            if diff_focused {
                Action::ScrollDiffUp
            } else if matches!(app.mode, Mode::Graph) {
                Action::ScrollGraphUp
            } else {
                Action::ScrollListUp
            }
        }
        MouseEventKind::ScrollDown => {
            if diff_focused {
                Action::ScrollDiffDown
            } else if matches!(app.mode, Mode::Graph) {
                Action::ScrollGraphDown
            } else {
                Action::ScrollListDown
            }
        }
        // Left click: find which panel was clicked from the last-rendered rects,
        // focus it, and jump the cursor to the clicked row.
        MouseEventKind::Down(MouseButton::Left) => {
            // Check the mode-tab strip first (it's at the top, above all panels).
            // The tab strip stores each tab's (start_x, end_x) range because
            // ratatui's Tabs widget renders tabs with variable widths (title
            // + padding + divider), NOT equal division.
            let tab_strip = app.ui.tab_strip.get();
            if let Some(index) = tab_index_from_strip(tab_strip, event.column, event.row) {
                return Action::ClickTab { index };
            }

            let rects = app.ui.panel_rects.get();
            let click = ratatui::layout::Rect {
                x: event.column,
                y: event.row,
                width: 1,
                height: 1,
            };

            // Check each panel rect in order. The rect includes the border, so
            // the row offset inside the list content is `click.y - rect.y - 1`
            // (1 for the top border). Clamp to 0 minimum.
            let try_panel = |rect: Option<ratatui::layout::Rect>,
                             panel: Panel,
                             _slot: RectSlot|
             -> Option<Action> {
                let r = rect?;
                if !r.intersects(click) {
                    return None;
                }
                // Row within the panel's content area (0-based). Subtract the
                // top border (1 row). Clamp to 0 so a click on the border or
                // title bar selects the first row rather than underflowing.
                let row = click.y.saturating_sub(r.y + 1) as usize;
                Some(Action::ClickPanel { panel, row })
            };

            // Three-pane dashboard: Left=Changes, Middle=Graph, Right=Diff.
            // Two-pane: Left=Changes/Graph, Main=Diff.
            // Other modes: Other=list.
            let action = match layout {
                PaneLayout::ThreePane => {
                    // Check Changes (Left), Graph (Middle), Diff (Right) in order.
                    try_panel(rects.changes, Panel::Left, RectSlot::Changes)
                        .or_else(|| try_panel(rects.graph, Panel::Middle, RectSlot::Graph))
                        .or_else(|| try_panel(rects.diff, Panel::Right, RectSlot::Diff))
                }
                PaneLayout::TwoPane => {
                    match app.mode {
                        Mode::Status => {
                            // Left=Changes, Main=Diff.
                            try_panel(rects.changes, Panel::Left, RectSlot::Changes)
                                .or_else(|| try_panel(rects.diff, Panel::Main, RectSlot::Diff))
                        }
                        Mode::Graph => {
                            // Left=Graph, Main=Diff (commit detail).
                            try_panel(rects.graph, Panel::Left, RectSlot::Graph)
                                .or_else(|| try_panel(rects.diff, Panel::Main, RectSlot::Diff))
                        }
                        _ => {
                            // Branches/Worktrees/Stashes: single list panel.
                            try_panel(rects.other, Panel::Left, RectSlot::Other)
                        }
                    }
                }
            };

            // If no panel rect matched (e.g. click on the status bar or a gap),
            // fall back to a simple FocusNext so the click isn't a complete no-op.
            action.unwrap_or(Action::None)
        }
        _ => Action::None,
    }
}

/// Given a recorded `TabStrip` and a click coordinate, return the tab index
/// that was clicked, or `None` if the click is outside all tab ranges.
pub(crate) fn tab_index_from_strip(
    strip: crate::app::TabStrip,
    column: u16,
    row: u16,
) -> Option<usize> {
    if row != strip.y {
        return None;
    }
    strip
        .ranges
        .iter()
        .position(|r| r.map(|(s, e)| column >= s && column < e).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::PaletteState;
    use crate::config::Config;
    use crate::test_backend::MockBackend;
    use crossterm::event::MouseEvent;

    fn build_app() -> App {
        App::new(
            Box::new(MockBackend::new()),
            Config::default(),
        )
        .expect("app builds")
    }

    fn left_click(app: &App, x: u16, y: u16) -> Action {
        handle_mouse(
            app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            },
        )
    }

    fn list_rect() -> ratatui::layout::Rect {
        ratatui::layout::Rect {
            x: 10,
            y: 5,
            width: 40,
            height: 10,
        }
    }

    #[test]
    fn palette_click_inside_list_returns_palette_click_action() {
        let mut app = build_app();
        app.palette = Some(PaletteState::new());
        app.mouse_capture = true;
        app.ui.palette_list_rect.set(Some(list_rect()));
        app.ui.palette_scroll.set(0);
        // Row 3 inside the list (y = 5 + 3 = 8).
        let action = left_click(&app, 15, 8);
        assert!(matches!(action, Action::PaletteClick { row: 3 }));
    }

    #[test]
    fn palette_click_outside_list_closes_palette() {
        let mut app = build_app();
        app.palette = Some(PaletteState::new());
        app.mouse_capture = true;
        app.ui.palette_list_rect.set(Some(list_rect()));
        // Click well outside the palette.
        let action = left_click(&app, 0, 0);
        assert!(matches!(action, Action::ClosePalette));
    }

    #[test]
    fn palette_click_on_dialog_border_closes_palette() {
        let mut app = build_app();
        app.palette = Some(PaletteState::new());
        app.mouse_capture = true;
        app.ui.palette_list_rect.set(Some(list_rect()));
        // Click on the query line (y = 4, just above the list at y = 5).
        let action = left_click(&app, 15, 4);
        assert!(matches!(action, Action::ClosePalette));
    }

    #[test]
    fn palette_click_with_no_recorded_rect_closes_palette() {
        let mut app = build_app();
        app.palette = Some(PaletteState::new());
        app.mouse_capture = true;
        // No rect recorded (e.g. zero-height list frame).
        app.ui.palette_list_rect.set(None);
        let action = left_click(&app, 15, 8);
        assert!(matches!(action, Action::ClosePalette));
    }

    #[test]
    fn palette_wheel_event_is_noop() {
        let mut app = build_app();
        app.palette = Some(PaletteState::new());
        app.mouse_capture = true;
        app.ui.palette_list_rect.set(Some(list_rect()));
        let action = handle_mouse(
            &app,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 15,
                row: 8,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert!(matches!(action, Action::None));
    }

    #[test]
    fn palette_click_ignored_when_mouse_capture_off() {
        let mut app = build_app();
        app.palette = Some(PaletteState::new());
        app.mouse_capture = false;
        app.ui.palette_list_rect.set(Some(list_rect()));
        // mouse_capture off → falls through to the overlay-ignore guard.
        let action = left_click(&app, 15, 8);
        assert!(matches!(action, Action::None));
    }
}
