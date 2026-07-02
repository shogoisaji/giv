//! Stashes mode — update logic: save / pop / apply / drop and the stash preview
//! diff loader.

use crate::app::{App, ConfirmOp, Dialog, Mode};
use crate::effect::Effect;
use crate::update::reload_selected_diff;

pub(crate) fn save(app: &mut App, msg: String) -> Effect {
    // The keymap's Enter sends an empty payload — read the live draft instead.
    let message = app.dialog.take_text_or(msg);
    app.dialog = Dialog::None;

    let msg_opt = if message.trim().is_empty() {
        None
    } else {
        Some(message.as_str())
    };

    match app.repo.backend.stash_save(msg_opt, true) {
        Ok(()) => {
            app.status_message = Some("Stashed changes".into());
            // The new stash is stash@{0}; move the cursor there so it doesn't
            // silently point at a renumbered older entry.
            app.ui.stash_index = 0;
            crate::update::refresh_silent(app);
            reload_selected_diff(app);
        }
        Err(e) => {
            app.status_message = Some(format!("Stash save failed: {e:#}"));
        }
    }
    Effect::Refresh
}

pub(crate) fn pop(app: &mut App) -> Effect {
    let idx = app.ui.stash_index;
    match app.repo.backend.stash_pop(idx) {
        Ok(()) => {
            app.status_message = Some(format!("Popped stash@{{{idx}}}"));
            crate::update::refresh_silent(app);
            // The list shrank — clamp the cursor and refresh the preview.
            reload_selected_diff(app);
        }
        Err(e) => handle_stash_apply_error(app, "pop", e),
    }
    Effect::Refresh
}

pub(crate) fn apply(app: &mut App) -> Effect {
    let idx = app.ui.stash_index;
    match app.repo.backend.stash_apply(idx) {
        Ok(()) => {
            app.status_message = Some(format!("Applied stash@{{{idx}}}"));
            crate::update::refresh_silent(app);
            reload_selected_diff(app);
        }
        Err(e) => handle_stash_apply_error(app, "apply", e),
    }
    Effect::Refresh
}

pub(crate) fn drop_selected(app: &mut App) -> Effect {
    let idx = app.ui.stash_index;
    if let Some(stash) = app.stashes.get(idx).cloned() {
        app.dialog = Dialog::Confirm {
            message: format!(
                "Drop stash@{{{}}}  \"{}\" ? (y/n)",
                stash.index, stash.message
            ),
            pending: ConfirmOp::StashDrop { index: stash.index },
        };
    }
    Effect::Refresh
}

// ─── Helpers ────────────────────────────────────────────────────────────────────

/// Handle a failed `git stash pop` / `git stash apply`.
///
/// These exit non-zero on conflict but have ALREADY applied the stash (conflict
/// markers are written to the working tree), and git prints its diagnostics to
/// stdout so the captured error body is usually empty. We therefore refresh and,
/// if conflicts now exist, tell the user how to proceed and jump to Status —
/// rather than reporting a bare "failed" with an empty message and a stale view.
fn handle_stash_apply_error(app: &mut App, verb: &str, e: anyhow::Error) {
    crate::update::refresh_silent(app);
    let conflicts = app
        .repo
        .status
        .entries
        .iter()
        .filter(|x| x.is_conflicted())
        .count();
    if conflicts > 0 {
        app.status_message = Some(format!(
            "Stash {verb}: {conflicts} conflict(s) — resolve in Status, R to mark resolved (stash kept)"
        ));
        app.mode = Mode::Status;
        app.ui.list_index = 0;
    } else {
        app.status_message = Some(format!("Stash {verb} failed: {e:#}"));
    }
    reload_selected_diff(app);
}

/// Load the diff for the currently selected stash entry so the Stashes preview
/// pane follows the cursor.
pub(crate) fn load_stash_diff(app: &mut App) {
    let idx = app.ui.stash_index;
    if app.stashes.get(idx).is_some() {
        match app.repo.backend.stash_show(idx) {
            Ok(diff) => {
                app.repo.selected_diff = Some(diff);
                app.repo.selected_diff_key = None;
            }
            Err(e) => {
                app.status_message = Some(format!("Stash diff failed: {e:#}"));
                app.repo.selected_diff = None;
                app.repo.selected_diff_key = None;
            }
        }
    } else {
        app.repo.selected_diff = None;
        app.repo.selected_diff_key = None;
    }
}

// ─── View-scroll clamp ──────────────────────────────────────────────────────────

/// Keep the selected stash row visible in the Stashes panel by adjusting
/// `stash_offset`. The stash list has no group headers, so the display row
/// equals `stash_index`. Mirrors `clamp_graph_offset` / `clamp_branch_offset`.
/// Called from `reload_selected_diff` after the selection or list changes.
pub(crate) fn clamp_stash_offset(app: &mut App) {
    let row = app.ui.stash_index.min(app.stashes.len().saturating_sub(1));
    let viewport = app.ui.stash_viewport.get().max(1);

    if row < app.ui.stash_offset {
        app.ui.stash_offset = row;
    } else if row >= app.ui.stash_offset + viewport {
        app.ui.stash_offset = row + 1 - viewport;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::git::types::Stash;
    use crate::test_backend::MockBackend;

    fn mk_stash(index: usize) -> Stash {
        Stash {
            index,
            message: format!("msg {index}"),
            oid: "deadbeef".into(),
        }
    }

    fn build_app_with_stashes(n: usize) -> App {
        let mut backend = MockBackend::new();
        backend.stashes = (0..n).map(mk_stash).collect();
        App::new(Box::new(backend), Config::default()).expect("app builds")
    }

    #[test]
    fn clamp_stash_offset_pulls_view_down_to_cursor() {
        let mut app = build_app_with_stashes(5);
        app.ui.stash_index = 4;
        app.ui.stash_offset = 0;
        app.ui.stash_viewport.set(3);
        clamp_stash_offset(&mut app);
        assert_eq!(app.ui.stash_offset, 2); // 4 >= 0+3 → 4+1-3 = 2
    }

    #[test]
    fn clamp_stash_offset_pulls_view_up_to_cursor() {
        let mut app = build_app_with_stashes(5);
        app.ui.stash_index = 0;
        app.ui.stash_offset = 4;
        app.ui.stash_viewport.set(3);
        clamp_stash_offset(&mut app);
        assert_eq!(app.ui.stash_offset, 0);
    }

    #[test]
    fn clamp_stash_offset_noop_when_cursor_visible() {
        let mut app = build_app_with_stashes(5);
        app.ui.stash_index = 2;
        app.ui.stash_offset = 1;
        app.ui.stash_viewport.set(3);
        clamp_stash_offset(&mut app);
        assert_eq!(app.ui.stash_offset, 1);
    }
}
