//! Status mode — update logic: staging, commit / amend, and the working-tree
//! diff loader. The cross-cutting orchestration (`reload_selected_diff`,
//! cursor clamping) lives in [`crate::update`] and is called back into here.
//!
//! # Selection → file mapping
//!
//! The status view renders the working tree as two groups — *Staged* then
//! *Unstaged* — so a file with both staged and unstaged changes appears once in
//! each group and the number of selectable rows can exceed
//! `app.repo.status.entries.len()`. `list_index` is a *logical* index over those
//! rows. Always resolve it through [`status_view::resolve_entry`] and ask
//! [`status_view::is_selected_staged`] which group the row is in.

use crate::app::{App, Dialog};
use crate::effect::Effect;
use crate::features::status::view as status_view;
use crate::update::reload_selected_diff;

// ─── Staging ───────────────────────────────────────────────────────────────────

pub(crate) fn stage_all(app: &mut App) -> Effect {
    match app.repo.backend.stage_all() {
        Ok(()) => {
            app.status_message = Some("Staged all changes".into());
            crate::update::refresh_silent(app);
            reload_selected_diff(app);
        }
        Err(e) => {
            app.status_message = Some(format!("Stage all failed: {e:#}"));
        }
    }
    Effect::Refresh
}

pub(crate) fn unstage_all(app: &mut App) -> Effect {
    match app.repo.backend.unstage_all() {
        Ok(()) => {
            app.status_message = Some("Unstaged all changes".into());
            crate::update::refresh_silent(app);
            reload_selected_diff(app);
        }
        Err(e) => {
            app.status_message = Some(format!("Unstage all failed: {e:#}"));
        }
    }
    Effect::Refresh
}

/// Stage/unstage toggle for the currently selected status row (Space key).
///
/// The action follows the row's group, so a file with both staged and unstaged
/// changes can be toggled from either side (matches lazygit / magit):
///   - Row in the Unstaged group (incl. untracked) → `git add <path>` (stage)
///   - Row in the Staged group                     → `git restore --staged`  (unstage)
pub(crate) fn toggle_stage_selected(app: &mut App) {
    let Some(entry) = status_view::resolve_entry(app, app.ui.list_index).cloned() else {
        return;
    };
    let on_staged_row = status_view::is_selected_staged(app);
    let paths = vec![entry.path.clone()];

    let result = if on_staged_row {
        app.repo
            .backend
            .unstage(&paths)
            .map(|_| format!("Unstaged {}", entry.path))
    } else {
        app.repo
            .backend
            .stage(&paths)
            .map(|_| format!("Staged {}", entry.path))
    };

    match result {
        Ok(msg) => {
            app.status_message = Some(msg);
            crate::update::refresh_silent(app);
            clamp_list_index(app);
            reload_selected_diff(app);
        }
        Err(e) => {
            app.status_message = Some(format!("Stage toggle failed: {e:#}"));
        }
    }
}

/// Explicitly unstage the currently selected status row (u key).
pub(crate) fn unstage_selected(app: &mut App) {
    let Some(entry) = status_view::resolve_entry(app, app.ui.list_index).cloned() else {
        return;
    };
    if !entry.is_staged() {
        app.status_message = Some(format!("{} is not staged", entry.path));
        return;
    }
    let paths = vec![entry.path.clone()];
    match app.repo.backend.unstage(&paths) {
        Ok(()) => {
            app.status_message = Some(format!("Unstaged {}", entry.path));
            crate::update::refresh_silent(app);
            clamp_list_index(app);
            reload_selected_diff(app);
        }
        Err(e) => {
            app.status_message = Some(format!("Unstage failed: {e:#}"));
        }
    }
}

// ─── Commit / amend ─────────────────────────────────────────────────────────────

pub(crate) fn submit_commit(app: &mut App, msg: String) -> Effect {
    // The update layer reads the actual draft from app.dialog when the action
    // payload is empty (Enter path from the keymap sends "").
    let message = app.dialog.take_text_or(msg);

    app.dialog = Dialog::None;

    if message.trim().is_empty() {
        app.status_message = Some("Commit aborted: empty message".into());
        return Effect::Refresh;
    }

    match app.repo.backend.commit(&message) {
        Ok(()) => {
            app.status_message = Some("Committed successfully".into());
            crate::update::refresh_silent(app);
            // The staged files left the list — keep the cursor in range.
            clamp_list_index(app);
            // After a commit the list is likely empty; clear the diff.
            app.repo.selected_diff = None;
            app.repo.selected_diff_key = None;
        }
        Err(e) => {
            app.status_message = Some(format!("Commit failed: {e:#}"));
        }
    }
    Effect::Refresh
}

pub(crate) fn open_amend(app: &mut App) -> Effect {
    match app.repo.backend.last_commit_message() {
        Ok(msg) => app.dialog = Dialog::Amend(msg),
        Err(e) => app.status_message = Some(format!("Nothing to amend: {e:#}")),
    }
    Effect::Refresh
}

pub(crate) fn submit_amend(app: &mut App, payload: String) -> Effect {
    let message = app.dialog.take_text_or(payload);
    app.dialog = Dialog::None;

    if message.trim().is_empty() {
        app.status_message = Some("Amend aborted: empty message".into());
        return Effect::Refresh;
    }

    match app.repo.backend.commit_amend(&message) {
        Ok(()) => {
            app.status_message = Some("Amended last commit".into());
            crate::update::refresh_silent(app);
            reload_selected_diff(app);
        }
        Err(e) => {
            app.status_message = Some(format!("Amend failed: {e:#}"));
        }
    }
    Effect::Refresh
}

// ─── Diff loader & cursor clamp ─────────────────────────────────────────────────

/// Load the diff for the currently selected status entry.
///
/// Chooses staged vs. unstaged diff based on the entry's state:
/// - If it has unstaged changes, shows the worktree diff.
/// - If it only has staged changes, shows the cached (index) diff.
pub(crate) fn load_status_diff(app: &mut App) {
    let path = status_view::resolve_entry(app, app.ui.list_index).map(|e| e.path.clone());
    // Show the staged (index) diff when the selected row is in the Staged group,
    // otherwise the worktree (unstaged) diff — so a file present in both groups
    // shows the right side depending on which row is highlighted.
    let staged = status_view::is_selected_staged(app);

    if let Some(path) = path {
        match app.repo.backend.file_diff(&path, staged) {
            Ok(diff) => {
                app.repo.selected_diff = Some(diff);
                app.repo.selected_diff_key = None;
            }
            Err(e) => {
                app.status_message = Some(format!("Diff failed: {e:#}"));
                app.repo.selected_diff = None;
                app.repo.selected_diff_key = None;
            }
        }
    } else {
        app.repo.selected_diff = None;
        app.repo.selected_diff_key = None;
    }
}

/// Clamp `list_index` to the current number of selectable status rows so the
/// selection stays valid after the file list changes (e.g. staging a file
/// removes its row, shrinking the list).
fn clamp_list_index(app: &mut App) {
    let count = status_view::status_row_count(app);
    app.ui.list_index = app.ui.list_index.min(count.saturating_sub(1));
}

/// Keep the selected file's row visible in the Changes panel by adjusting
/// `list_offset`. Mirrors [`crate::features::graph::update::clamp_graph_offset`]:
/// the offset only moves when the selection leaves the viewport, so the view
/// scrolls to follow the cursor instead of letting the selection jump
/// off-screen. Works on the *display row* (headers included) so the group
/// headers and empty-group placeholders are accounted for. Called from
/// `reload_selected_diff` after the selection or list changes.
pub(crate) fn clamp_list_offset(app: &mut App) {
    let row = status_view::selected_display_row(app);
    let viewport = app.ui.list_viewport.get().max(1);

    if row < app.ui.list_offset {
        app.ui.list_offset = row;
    } else if row >= app.ui.list_offset + viewport {
        app.ui.list_offset = row + 1 - viewport;
    }
}
