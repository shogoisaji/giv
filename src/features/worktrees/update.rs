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

// ─── View-scroll clamp ──────────────────────────────────────────────────────────

/// Keep the selected worktree row visible in the Worktrees panel by adjusting
/// `worktree_offset`. The worktree list has no group headers, so the display
/// row equals `worktree_index`. Mirrors `clamp_graph_offset` /
/// `clamp_branch_offset`. Called from `reload_selected_diff` after the
/// selection or list changes.
pub(crate) fn clamp_worktree_offset(app: &mut App) {
    let row = app
        .ui
        .worktree_index
        .min(app.worktrees.len().saturating_sub(1));
    let viewport = app.ui.worktree_viewport.get().max(1);

    if row < app.ui.worktree_offset {
        app.ui.worktree_offset = row;
    } else if row >= app.ui.worktree_offset + viewport {
        app.ui.worktree_offset = row + 1 - viewport;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::git::types::Worktree;
    use crate::test_backend::MockBackend;

    fn mk_worktree(path: &str) -> Worktree {
        Worktree {
            path: path.into(),
            branch: Some("main".into()),
            head: "deadbeef".into(),
            is_current: false,
            is_bare: false,
            is_locked: false,
        }
    }

    fn build_app_with_worktrees(n: usize) -> App {
        let mut backend = MockBackend::new();
        backend.worktrees = (0..n)
            .map(|i| mk_worktree(&format!("/tmp/wt{i}")))
            .collect();
        App::new(Box::new(backend), Config::default()).expect("app builds")
    }

    #[test]
    fn clamp_worktree_offset_pulls_view_down_to_cursor() {
        let mut app = build_app_with_worktrees(5);
        app.ui.worktree_index = 4;
        app.ui.worktree_offset = 0;
        app.ui.worktree_viewport.set(3);
        clamp_worktree_offset(&mut app);
        assert_eq!(app.ui.worktree_offset, 2); // 4 >= 0+3 → 4+1-3 = 2
    }

    #[test]
    fn clamp_worktree_offset_pulls_view_up_to_cursor() {
        let mut app = build_app_with_worktrees(5);
        app.ui.worktree_index = 0;
        app.ui.worktree_offset = 4;
        app.ui.worktree_viewport.set(3);
        clamp_worktree_offset(&mut app);
        assert_eq!(app.ui.worktree_offset, 0);
    }

    #[test]
    fn clamp_worktree_offset_noop_when_cursor_visible() {
        let mut app = build_app_with_worktrees(5);
        app.ui.worktree_index = 2;
        app.ui.worktree_offset = 1;
        app.ui.worktree_viewport.set(3);
        clamp_worktree_offset(&mut app);
        assert_eq!(app.ui.worktree_offset, 1);
    }
}
