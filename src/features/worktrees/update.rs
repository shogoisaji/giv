//! Worktrees mode — update logic: add, prune, and switch-to (cd-on-exit).
//!
//! Remove is routed through the cross-cutting confirm flow
//! (`WorktreeRemove` → `DeleteSelected` → `ConfirmDelete`) in [`crate::update`].

use crate::app::{App, Dialog};
use crate::effect::Effect;
use crate::git;

pub(crate) fn submit_add(app: &mut App, path: String) -> Effect {
    // The keymap's Enter sends an empty payload — read the live draft instead.
    let wt_path = app.dialog.take_text_or(path);

    app.dialog = Dialog::None;

    if wt_path.trim().is_empty() {
        app.status_message = Some("Worktree add aborted: empty path".into());
        return Effect::Refresh;
    }

    // Derive a branch name from the last path component.
    let branch = std::path::Path::new(&wt_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&wt_path)
        .to_string();

    // If a local branch of that name already exists, check it OUT into the new
    // worktree (`git worktree add <path> <branch>`) instead of trying to create
    // it (`-b`), which would fail with "already exists".
    let branch_exists = app
        .branches
        .iter()
        .any(|b| b.kind == git::RefKind::LocalBranch && b.name == branch);

    match app
        .repo
        .backend
        .worktree_add(&wt_path, &branch, !branch_exists)
    {
        Ok(()) => {
            app.status_message = Some(format!("Added worktree at '{wt_path}'"));
            crate::update::refresh_silent(app);
        }
        Err(e) => {
            app.status_message = Some(format!("Worktree add failed: {e:#}"));
        }
    }
    Effect::Refresh
}

pub(crate) fn prune(app: &mut App) -> Effect {
    match app.repo.backend.worktree_prune() {
        Ok(()) => {
            app.status_message = Some("Worktree prune completed".into());
            crate::update::refresh_silent(app);
        }
        Err(e) => {
            app.status_message = Some(format!("Worktree prune failed: {e:#}"));
        }
    }
    Effect::Refresh
}

pub(crate) fn switch(app: &mut App) -> Effect {
    let idx = app.ui.worktree_index;
    if let Some(wt) = app.worktrees.get(idx).cloned() {
        app.print_cwd_on_exit = Some(wt.path);
        app.should_quit = true;
        return Effect::Quit;
    }
    Effect::Refresh
}
