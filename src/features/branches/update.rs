//! Branches mode — update logic: checkout, create, rename, and merge-into-HEAD.
//!
//! Delete is routed through the cross-cutting confirm flow
//! (`DeleteSelected` → `ConfirmDelete`) in [`crate::update`].

use crate::app::{App, ConfirmOp, Dialog};
use crate::effect::Effect;
use crate::git;
use crate::update::reload_selected_diff;

pub(crate) fn checkout(app: &mut App) -> Effect {
    let idx = app.ui.branch_index;
    if let Some(branch) = app.branches.get(idx).cloned() {
        // Checking out a remote-tracking ref by its full slashed name
        // ("origin/feature") detaches HEAD. Use the SHORT name so git DWIMs a
        // local branch tracking the remote (or switches to an existing local
        // branch of that name).
        let target = if branch.kind == git::RefKind::RemoteBranch {
            branch
                .name
                .split_once('/')
                .map(|(_, rest)| rest.to_string())
                .unwrap_or_else(|| branch.name.clone())
        } else {
            branch.name
        };
        match app.repo.backend.checkout(&target) {
            Ok(()) => {
                app.status_message = Some(format!("Checked out '{target}'"));
                crate::update::refresh_silent(app);
                reload_selected_diff(app);
            }
            Err(e) => {
                app.status_message = Some(format!("Checkout failed: {e:#}"));
            }
        }
    }
    Effect::Refresh
}

pub(crate) fn submit_new_branch(app: &mut App, name: String) -> Effect {
    // The keymap's Enter sends an empty payload — read the live draft instead.
    let branch_name = app.dialog.take_text_or(name);

    app.dialog = Dialog::None;

    if branch_name.trim().is_empty() {
        app.status_message = Some("Branch creation aborted: empty name".into());
        return Effect::Refresh;
    }

    // Get the current branch as the starting point (optional).
    let from = app.repo.status.branch.clone();
    match app
        .repo
        .backend
        .create_branch(&branch_name, from.as_deref(), true)
    {
        Ok(()) => {
            app.status_message = Some(format!("Created and checked out branch '{branch_name}'"));
            crate::update::refresh_silent(app);
        }
        Err(e) => {
            app.status_message = Some(format!("Create branch failed: {e:#}"));
        }
    }
    Effect::Refresh
}

pub(crate) fn rename_branch_dialog(app: &mut App) -> Effect {
    let idx = app.ui.branch_index;
    if let Some(branch) = app.branches.get(idx).cloned() {
        if branch.kind != git::RefKind::LocalBranch {
            app.status_message = Some("Only local branches can be renamed".into());
            return Effect::Refresh;
        }
        app.dialog = Dialog::RenameBranch {
            old: branch.name.clone(),
            new: branch.name,
        };
    }
    Effect::Refresh
}

pub(crate) fn submit_rename_branch(app: &mut App) -> Effect {
    if let Dialog::RenameBranch { old, new } = app.dialog.clone() {
        app.dialog = Dialog::None;
        let new = new.trim().to_string();
        if new.is_empty() || new == old {
            app.status_message = Some("Rename aborted".into());
            return Effect::Refresh;
        }
        match app.repo.backend.rename_branch(&old, &new) {
            Ok(()) => {
                app.status_message = Some(format!("Renamed '{old}' → '{new}'"));
                crate::update::refresh_silent(app);
            }
            Err(e) => {
                app.status_message = Some(format!("Rename failed: {e:#}"));
            }
        }
    } else {
        app.dialog = Dialog::None;
    }
    Effect::Refresh
}

pub(crate) fn merge_selected(app: &mut App) -> Effect {
    let idx = app.ui.branch_index;
    if let Some(branch) = app.branches.get(idx) {
        if branch.is_head {
            app.status_message = Some("Cannot merge HEAD into itself".into());
            return Effect::Refresh;
        }
        let name = branch.name.clone();
        app.dialog = Dialog::Confirm {
            message: format!("Merge '{name}' into HEAD? (y/n)"),
            pending: ConfirmOp::Merge { branch: name },
        };
    }
    Effect::Refresh
}

// ─── View-scroll clamp ──────────────────────────────────────────────────────────

/// Keep the selected branch/tag's display row visible in the Branches panel by
/// adjusting `branch_offset`. Mirrors `clamp_graph_offset` / `clamp_list_offset`:
/// the offset only moves when the selection leaves the viewport, so the view
/// scrolls to follow the cursor instead of letting the selection jump
/// off-screen. Works on the *display row* (headers / placeholders included) so
/// the group headers are accounted for. Called from `reload_selected_diff`
/// after the selection or list changes.
pub(crate) fn clamp_branch_offset(app: &mut App) {
    let row = crate::features::branches::view::branch_selected_display_row(app);
    let viewport = app.ui.branch_viewport.get().max(1);

    if row < app.ui.branch_offset {
        app.ui.branch_offset = row;
    } else if row >= app.ui.branch_offset + viewport {
        app.ui.branch_offset = row + 1 - viewport;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::test_backend::{mk_branch, MockBackend};

    fn build_app_with_locals(n: usize) -> App {
        let mut backend = MockBackend::new();
        backend.branches = (0..n)
            .map(|i| mk_branch(&format!("local{i}"), "deadbeef"))
            .collect();
        App::new(Box::new(backend), Config::default()).expect("app builds")
    }

    #[test]
    fn clamp_branch_offset_pulls_view_down_to_cursor() {
        // 5 locals → display rows 1..5. Cursor on row 5 (index 4), viewport 3,
        // offset starts at 0: 5 >= 0+3, so offset becomes 5+1-3 = 3.
        let mut app = build_app_with_locals(5);
        app.ui.branch_index = 4;
        app.ui.branch_offset = 0;
        app.ui.branch_viewport.set(3);
        clamp_branch_offset(&mut app);
        assert_eq!(app.ui.branch_offset, 3);
    }

    #[test]
    fn clamp_branch_offset_pulls_view_up_to_cursor() {
        // Cursor on row 1 (index 0), viewport 3, offset starts at 5: 1 < 5,
        // so offset becomes 1.
        let mut app = build_app_with_locals(5);
        app.ui.branch_index = 0;
        app.ui.branch_offset = 5;
        app.ui.branch_viewport.set(3);
        clamp_branch_offset(&mut app);
        assert_eq!(app.ui.branch_offset, 1);
    }

    #[test]
    fn clamp_branch_offset_noop_when_cursor_visible() {
        // Cursor on row 2 (index 1), viewport 3, offset 1: 2 is in [1, 4).
        let mut app = build_app_with_locals(5);
        app.ui.branch_index = 1;
        app.ui.branch_offset = 1;
        app.ui.branch_viewport.set(3);
        clamp_branch_offset(&mut app);
        assert_eq!(app.ui.branch_offset, 1);
    }
}
