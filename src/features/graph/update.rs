//! Graph mode — update logic: history operations (cherry-pick / revert / reset /
//! rebase / tag), the interactive-rebase todo editor, the graph-view toggles
//! (scope / first-parent / branch lens), and the commit diff loader.
//!
//! Operations that mutate history open a `Dialog::Confirm`; the actual git call
//! is executed uniformly by the `ConfirmDelete` handler in [`crate::update`].

use super::rebase_todo::{RebaseTodoEntry, RebaseTodoState};
use crate::app::{App, ConfirmOp, Dialog, DiffKey, Mode};
use crate::effect::Effect;
use crate::git::{self, Diff, ResetMode};
use crate::update::{check_op_in_progress, reload_selected_diff};

const COMMIT_DIFF_CACHE_CAP: usize = 64;

// ─── Confirm-guarded history operations ─────────────────────────────────────────

pub(crate) fn cherry_pick_selected(app: &mut App) -> Effect {
    // Confirm first — an accidental keypress shouldn't run cherry-pick.
    let idx = app.ui.graph_index;
    if let Some(commit) = app.repo.commits.get(idx) {
        let oid = commit.id.clone();
        let short = git::short_oid(&oid).to_string();
        app.dialog = Dialog::Confirm {
            message: format!("Cherry-pick {short} onto HEAD? (y/n)"),
            pending: ConfirmOp::CherryPick { oid },
        };
    }
    Effect::Refresh
}

pub(crate) fn revert_selected(app: &mut App) -> Effect {
    let idx = app.ui.graph_index;
    if let Some(commit) = app.repo.commits.get(idx) {
        let oid = commit.id.clone();
        let short = git::short_oid(&oid).to_string();
        app.dialog = Dialog::Confirm {
            message: format!("Revert {short}? This creates a new commit. (y/n)"),
            pending: ConfirmOp::Revert { oid },
        };
    }
    Effect::Refresh
}

pub(crate) fn open_reset_menu(app: &mut App) -> Effect {
    let idx = app.ui.graph_index;
    if let Some(commit) = app.repo.commits.get(idx).cloned() {
        app.dialog = Dialog::ResetMenu { target: commit.id };
    }
    Effect::Refresh
}

pub(crate) fn reset_to(app: &mut App, mode: ResetMode) -> Effect {
    // The ResetMenu dialog holds the target OID. Every reset mode moves HEAD
    // and can surprise users if triggered accidentally, so all variants go
    // through the same explicit confirmation flow.
    let target = match &app.dialog {
        Dialog::ResetMenu { target } => target.clone(),
        _ => {
            // Fallback: use the selected graph commit.
            let idx = app.ui.graph_index;
            match app.repo.commits.get(idx).cloned() {
                Some(c) => c.id,
                None => return Effect::Refresh,
            }
        }
    };

    let flag = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
        ResetMode::Hard => "--hard",
    };
    let impact = match mode {
        ResetMode::Soft => "This moves HEAD and keeps index/worktree changes.",
        ResetMode::Mixed => "This moves HEAD and resets the index.",
        ResetMode::Hard => "This discards uncommitted changes.",
    };
    app.dialog = Dialog::Confirm {
        message: format!(
            "Reset {flag} to {}? {impact} (y/n)",
            git::short_oid(&target)
        ),
        pending: ConfirmOp::Reset { mode, target },
    };
    Effect::Refresh
}

/// Rebase HEAD onto the selected ref — a commit in Graph mode, a branch in
/// Branches mode. Opens a confirm dialog (rebase rewrites history).
pub(crate) fn rebase_onto_selected(app: &mut App) -> Effect {
    let target_display = match app.mode {
        Mode::Graph => {
            let idx = app.ui.graph_index;
            app.repo
                .commits
                .get(idx)
                .map(|c| (c.id.clone(), git::short_oid(&c.id).to_string()))
        }
        Mode::Branches => {
            let idx = app.ui.branch_index;
            app.branches
                .get(idx)
                .map(|b| (b.name.clone(), b.name.clone()))
        }
        _ => None,
    };
    if let Some((target, display)) = target_display {
        app.dialog = Dialog::Confirm {
            message: format!("Rebase HEAD onto {display}? This rewrites history. (y/n)"),
            pending: ConfirmOp::RebaseOnto { target, display },
        };
    }
    Effect::Refresh
}

// ─── Tags ───────────────────────────────────────────────────────────────────────

pub(crate) fn submit_tag(app: &mut App, payload: String) -> Effect {
    // payload format: "name\tmessage" (message may be empty).
    let (tag_name, tag_message) = if payload.is_empty() {
        // Read from dialog.
        match &app.dialog {
            Dialog::TagCreate { name, message, .. } => (
                name.clone(),
                if message.is_empty() {
                    None
                } else {
                    Some(message.clone())
                },
            ),
            _ => return Effect::Refresh,
        }
    } else {
        let mut parts = payload.splitn(2, '\t');
        let n = parts.next().unwrap_or("").to_string();
        let m = parts.next().unwrap_or("");
        (
            n,
            if m.is_empty() {
                None
            } else {
                Some(m.to_string())
            },
        )
    };

    app.dialog = Dialog::None;

    if tag_name.trim().is_empty() {
        app.status_message = Some("Tag creation aborted: empty name".into());
        return Effect::Refresh;
    }

    // Target: selected commit OID if in Graph mode, otherwise HEAD.
    let target = if app.mode == Mode::Graph {
        app.repo
            .commits
            .get(app.ui.graph_index)
            .map(|c| c.id.clone())
    } else {
        None
    };

    match app
        .repo
        .backend
        .tag_create(&tag_name, target.as_deref(), tag_message.as_deref())
    {
        Ok(()) => {
            app.status_message = Some(format!("Created tag '{tag_name}'"));
            crate::update::refresh_silent(app);
        }
        Err(e) => {
            app.status_message = Some(format!("Tag create failed: {e:#}"));
        }
    }
    Effect::Refresh
}

pub(crate) fn tag_delete(app: &mut App) -> Effect {
    // Delete the tag that matches the selected commit's decoration in Graph.
    let tag_name = if app.mode == Mode::Graph {
        let idx = app.ui.graph_index;
        app.repo.commits.get(idx).and_then(|c| {
            c.refs
                .iter()
                .find(|r| r.kind == git::RefKind::Tag)
                .map(|r| r.name.clone())
        })
    } else {
        None
    };

    if let Some(name) = tag_name {
        app.dialog = Dialog::Confirm {
            message: format!("Delete tag '{name}' ? (y/n)"),
            pending: ConfirmOp::TagDelete { name },
        };
    } else {
        app.status_message = Some("No tag selected".into());
    }
    Effect::Refresh
}

// ─── Interactive rebase ─────────────────────────────────────────────────────────

pub(crate) fn start_interactive_rebase(app: &mut App) -> Effect {
    // Use the selected graph commit as the base.
    // Entries = commits from base..HEAD (oldest first, as git expects).
    let idx = app.ui.graph_index;
    if let Some(base_commit) = app.repo.commits.get(idx).cloned() {
        let base = base_commit.id;

        // Collect all commits from HEAD down to (but not including) the selected
        // one. repo.commits is newest-first, so commits[0] is HEAD. We want
        // commits[0..idx] reversed to get oldest-first.
        let entries: Vec<RebaseTodoEntry> = app.repo.commits[..idx]
            .iter()
            .rev()
            .map(|c| RebaseTodoEntry {
                command: "pick".to_string(),
                oid: c.id.clone(),
                summary: c.summary.clone(),
            })
            .collect();

        if entries.is_empty() {
            app.status_message =
                Some("No commits to rebase (select a commit that is not HEAD)".into());
        } else {
            app.rebase_todo = Some(RebaseTodoState {
                entries,
                cursor: 0,
                base,
            });
        }
    } else {
        app.status_message = Some("No commit selected".into());
    }
    Effect::Refresh
}

pub(crate) fn rebase_todo_up(app: &mut App) -> Effect {
    if let Some(ref mut state) = app.rebase_todo {
        if state.cursor > 0 {
            state.cursor -= 1;
        }
    }
    Effect::Refresh
}

pub(crate) fn rebase_todo_down(app: &mut App) -> Effect {
    if let Some(ref mut state) = app.rebase_todo {
        let max = state.entries.len().saturating_sub(1);
        if state.cursor < max {
            state.cursor += 1;
        }
    }
    Effect::Refresh
}

pub(crate) fn rebase_todo_set_command(app: &mut App, ch: char) -> Effect {
    if let Some(ref mut state) = app.rebase_todo {
        let cmd = match ch {
            'p' => "pick",
            'r' => "reword",
            'e' => "edit",
            's' => "squash",
            'f' => "fixup",
            'd' => "drop",
            _ => return Effect::Refresh,
        };
        if let Some(entry) = state.entries.get_mut(state.cursor) {
            entry.command = cmd.to_string();
        }
    }
    Effect::Refresh
}

pub(crate) fn rebase_todo_move_entry_up(app: &mut App) -> Effect {
    if let Some(ref mut state) = app.rebase_todo {
        let cur = state.cursor;
        if cur > 0 {
            state.entries.swap(cur - 1, cur);
            state.cursor = cur - 1;
        }
    }
    Effect::Refresh
}

pub(crate) fn rebase_todo_move_entry_down(app: &mut App) -> Effect {
    if let Some(ref mut state) = app.rebase_todo {
        let cur = state.cursor;
        if cur + 1 < state.entries.len() {
            state.entries.swap(cur, cur + 1);
            state.cursor = cur + 1;
        }
    }
    Effect::Refresh
}

pub(crate) fn rebase_todo_execute(app: &mut App) -> Effect {
    if let Some(state) = app.rebase_todo.take() {
        let todo: Vec<(String, String)> = state
            .entries
            .iter()
            .map(|e| (e.command.clone(), e.oid.clone()))
            .collect();

        match app.repo.backend.rebase_interactive(&state.base, &todo) {
            Ok(()) => {
                app.status_message = Some("Interactive rebase completed".into());
                crate::update::refresh_silent(app);
                reload_selected_diff(app);
                check_op_in_progress(app);
            }
            Err(e) => {
                app.status_message = Some(format!("Interactive rebase failed: {e:#}"));
                crate::update::refresh_silent(app);
                check_op_in_progress(app);
            }
        }
    }
    Effect::Refresh
}

pub(crate) fn rebase_todo_cancel(app: &mut App) -> Effect {
    app.rebase_todo = None;
    app.status_message = Some("Interactive rebase cancelled".into());
    Effect::Refresh
}

// ─── Graph view toggles ─────────────────────────────────────────────────────────

pub(crate) fn toggle_scope(app: &mut App) -> Effect {
    let prev_all_branches = app.ui.graph_all_branches;
    let prev_index = app.ui.graph_index;
    let prev_offset = app.ui.graph_offset;

    app.ui.graph_all_branches = !app.ui.graph_all_branches;
    // The commit set changes, so move the cursor to the top to stay valid.
    app.ui.graph_index = 0;
    app.ui.graph_offset = 0;
    if let Err(e) = app.refresh() {
        app.ui.graph_all_branches = prev_all_branches;
        app.ui.graph_index = prev_index;
        app.ui.graph_offset = prev_offset;
        app.status_message = Some(format!("scope toggle: refresh failed: {e:#}"));
        return Effect::Refresh;
    }
    app.status_message = Some(if app.ui.graph_all_branches {
        "Graph: all branches".into()
    } else {
        "Graph: current branch only".into()
    });
    reload_selected_diff(app);
    Effect::Refresh
}

pub(crate) fn toggle_first_parent(app: &mut App) -> Effect {
    let prev_first_parent = app.ui.graph_first_parent;
    let prev_index = app.ui.graph_index;
    let prev_offset = app.ui.graph_offset;

    app.ui.graph_first_parent = !app.ui.graph_first_parent;
    // The commit set changes (merges collapse), so reset to the top.
    app.ui.graph_index = 0;
    app.ui.graph_offset = 0;
    if let Err(e) = app.refresh() {
        app.ui.graph_first_parent = prev_first_parent;
        app.ui.graph_index = prev_index;
        app.ui.graph_offset = prev_offset;
        app.status_message = Some(format!("fold toggle: refresh failed: {e:#}"));
        return Effect::Refresh;
    }
    app.status_message = Some(if app.ui.graph_first_parent {
        "Graph: merges folded (first-parent)".into()
    } else {
        "Graph: merges expanded".into()
    });
    reload_selected_diff(app);
    Effect::Refresh
}

pub(crate) fn toggle_branch_focus(app: &mut App) -> Effect {
    let prev_focus = app.ui.graph_focus.clone();
    let prev_index = app.ui.graph_index;
    let prev_offset = app.ui.graph_offset;

    if app.ui.graph_focus.is_some() {
        app.ui.graph_focus = None;
        app.status_message = Some("Branch lens: off".into());
    } else {
        // Anchor on the currently selected commit; show it vs main.
        match app
            .repo
            .commits
            .get(app.ui.graph_index)
            .map(|c| c.id.clone())
        {
            Some(tip) => {
                app.ui.graph_focus = Some(tip);
                let base = app.detect_main_branch().map(|(n, _)| n);
                app.status_message = Some(match base {
                    Some(b) => format!("Branch lens: this branch vs {b}"),
                    None => "Branch lens: this branch (no main found)".into(),
                });
            }
            None => {
                app.status_message = Some("Branch lens: no commit selected".into());
                return Effect::Refresh;
            }
        }
    }
    app.ui.graph_index = 0;
    app.ui.graph_offset = 0;
    if let Err(e) = app.refresh() {
        app.ui.graph_focus = prev_focus;
        app.ui.graph_index = prev_index;
        app.ui.graph_offset = prev_offset;
        app.status_message = Some(format!("branch lens: refresh failed: {e:#}"));
        return Effect::Refresh;
    }
    // Put the cursor back on the focused tip (the lens anchor, held in
    // `graph_focus`) so the lineage highlight keeps the branch vivid and dims
    // main's ahead-of-fork commits (the rebase gap).
    if let Some(tip) = app.ui.graph_focus.clone() {
        if let Some(i) = app.repo.commits.iter().position(|c| c.id == tip) {
            app.ui.graph_index = i;
        }
    }
    reload_selected_diff(app);
    Effect::Refresh
}

// ─── Diff loader & cursor clamp ─────────────────────────────────────────────────

/// Load the commit diff for the currently selected graph entry.
pub(crate) fn load_graph_diff(app: &mut App) {
    // Branch-compare mode: show the cumulative `base...target` diff instead of
    // a single commit's diff. The commit list is still visible for navigation,
    // but the diff panel reflects the entire compared range.
    app.pending_graph_diff = None;

    if let Some((base, target)) = &app.compare {
        let key = DiffKey::Compare(base.clone(), target.clone());
        if app.repo.selected_diff.is_some() && app.repo.selected_diff_key.as_ref() == Some(&key) {
            return;
        }
        match app.repo.backend.diff_between(base, target) {
            Ok(diff) => {
                app.repo.selected_diff = Some(diff);
                app.repo.selected_diff_key = Some(key);
            }
            Err(e) => {
                app.status_message = Some(format!("Compare diff failed: {e:#}"));
                app.repo.selected_diff = None;
                app.repo.selected_diff_key = None;
            }
        }
        return;
    }

    let idx = app.ui.graph_index;
    let oid = app.repo.commits.get(idx).map(|c| c.id.clone());

    if let Some(oid) = oid {
        let key = DiffKey::Commit(oid.clone());
        if let Some(diff) = app.repo.commit_diff_cache.get(&oid).cloned() {
            app.repo.selected_diff = Some(diff);
            app.repo.selected_diff_key = Some(key);
            app.repo.commit_diff_order.retain(|cached| cached != &oid);
            app.repo.commit_diff_order.push_back(oid);
            return;
        }
        match app.repo.backend.commit_diff(&oid) {
            Ok(diff) => {
                app.repo.selected_diff = Some(diff.clone());
                app.repo.selected_diff_key = Some(key);
                insert_commit_diff_cache(app, oid, diff);
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

/// Keep the selected commit's node row visible in the graph panel by adjusting
/// `graph_offset`. Called from `reload_selected_diff` after any graph mutation
/// so the view follows the cursor when the selection moves off-screen.
pub(crate) fn clamp_graph_offset(app: &mut App) {
    // In spacious mode each commit occupies a node row + an edge row, so the
    // selected commit's node sits at row `index * 2`; compact mode is 1:1.
    let sel_row = app.ui.graph_index * app.config.graph_row_step();
    let viewport = app.ui.graph_viewport.get().max(1);

    if sel_row < app.ui.graph_offset {
        app.ui.graph_offset = sel_row;
    } else if sel_row >= app.ui.graph_offset + viewport {
        app.ui.graph_offset = sel_row + 1 - viewport;
    }
}

/// Upper bound on the commit-diff cache's total estimated heap footprint.
/// Picked to comfortably absorb typical diffs (a few hundred KB each) while
/// preventing a handful of huge generated-file / merge diffs from blowing
/// up memory. The entry-count cap (`COMMIT_DIFF_CACHE_CAP`) still applies
/// alongside this.
const COMMIT_DIFF_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;

/// Insert a commit diff into the LRU cache, evicting oldest entries until
/// both the entry-count cap and the byte cap are satisfied. Extracted from
/// `load_graph_diff` so the eviction policy is unit-testable without a git
/// backend.
///
/// A diff whose own `estimated_size` exceeds the byte cap is not cached at
/// all (it would evict everything else and still breach the limit); the
/// caller simply re-fetches it next time. The byte-cap eviction loop keeps
/// the just-inserted entry (the loop guard `len() > 1`), so a single small
/// entry never evicts itself.
pub(crate) fn insert_commit_diff_cache(app: &mut App, oid: String, diff: Diff) {
    if diff.estimated_size() > COMMIT_DIFF_CACHE_MAX_BYTES {
        return;
    }
    app.repo.commit_diff_cache.insert(oid.clone(), diff);
    app.repo.commit_diff_order.retain(|c| c != &oid);
    app.repo.commit_diff_order.push_back(oid);

    // Entry-count cap.
    while app.repo.commit_diff_order.len() > COMMIT_DIFF_CACHE_CAP {
        if let Some(old) = app.repo.commit_diff_order.pop_front() {
            app.repo.commit_diff_cache.remove(&old);
        }
    }
    // Byte cap — evict oldest until the total estimated size fits. Guard on
    // `len() > 1` so we never evict the entry we just inserted.
    while total_cache_bytes(app) > COMMIT_DIFF_CACHE_MAX_BYTES
        && app.repo.commit_diff_order.len() > 1
    {
        if let Some(old) = app.repo.commit_diff_order.pop_front() {
            app.repo.commit_diff_cache.remove(&old);
        }
    }
}

/// Sum of `estimated_size` over every cached diff. O(n) over the cache; called
/// only on insertion so the cost is amortised over a backend `commit_diff`
/// fetch (which dominates).
fn total_cache_bytes(app: &App) -> usize {
    app.repo
        .commit_diff_cache
        .values()
        .map(|d| d.estimated_size())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::git::types::{Diff, DiffLine, DiffLineKind, FileDiff, Hunk};
    use crate::test_backend::{mk_commit, MockBackend};

    fn build_app(n_commits: usize) -> App {
        let mut b = MockBackend::new();
        b.commits = (0..n_commits)
            .map(|i| mk_commit(&format!("c{i}"), &format!("c{i}"), false))
            .collect();
        App::new(Box::new(b), Config::default()).expect("app builds")
    }

    fn mk_diff(text_len: usize) -> Diff {
        Diff {
            files: vec![FileDiff {
                old_path: "a".into(),
                new_path: "b".into(),
                is_binary: false,
                hunks: vec![Hunk {
                    header: "@@@".into(),
                    old_start: 1,
                    old_lines: 1,
                    new_start: 1,
                    new_lines: 1,
                    lines: vec![DiffLine {
                        kind: DiffLineKind::Context,
                        text: "x".repeat(text_len),
                    }],
                }],
            }],
        }
    }

    #[test]
    fn estimated_size_sums_line_text_plus_header() {
        let d = mk_diff(100);
        // header (3) + 1 line of 100 chars + 1 byte per line overhead.
        assert_eq!(d.estimated_size(), 3 + 100 + 1);
    }

    #[test]
    fn insert_keeps_recent_entries_under_byte_limit() {
        let mut app = build_app(4);
        for i in 0..4 {
            insert_commit_diff_cache(&mut app, format!("c{i}"), mk_diff(1024));
        }
        assert_eq!(app.repo.commit_diff_cache.len(), 4);
        assert!(total_cache_bytes(&app) <= COMMIT_DIFF_CACHE_MAX_BYTES);
    }

    #[test]
    fn insert_skips_diff_that_alone_exceeds_byte_limit() {
        // A single diff bigger than the byte cap is not cached at all.
        let mut app = build_app(2);
        let huge = COMMIT_DIFF_CACHE_MAX_BYTES + 1;
        insert_commit_diff_cache(&mut app, "c0".into(), mk_diff(huge));
        assert!(app.repo.commit_diff_cache.is_empty());
        assert!(app.repo.commit_diff_order.is_empty());

        // A subsequent small diff still caches normally.
        insert_commit_diff_cache(&mut app, "c1".into(), mk_diff(1));
        assert_eq!(app.repo.commit_diff_cache.len(), 1);
        assert!(app.repo.commit_diff_cache.contains_key("c1"));
    }

    #[test]
    fn insert_evicts_oldest_when_byte_limit_exceeded() {
        // Three diffs of cap/3+1 each: any two fit, three overflow.
        let each = COMMIT_DIFF_CACHE_MAX_BYTES / 3 + 1;
        let mut app = build_app(3);
        insert_commit_diff_cache(&mut app, "c0".into(), mk_diff(each));
        insert_commit_diff_cache(&mut app, "c1".into(), mk_diff(each));
        // c0 + c1 = 2*(cap/3+1) <= cap (since 2/3*cap + 2 < cap for cap>=6).
        assert_eq!(app.repo.commit_diff_cache.len(), 2);

        insert_commit_diff_cache(&mut app, "c2".into(), mk_diff(each));
        // c0 (oldest) evicted; c1 and c2 remain.
        assert_eq!(app.repo.commit_diff_cache.len(), 2);
        assert!(app.repo.commit_diff_cache.contains_key("c1"));
        assert!(app.repo.commit_diff_cache.contains_key("c2"));
        assert!(!app.repo.commit_diff_cache.contains_key("c0"));
    }

    #[test]
    fn insert_refreshes_lru_position_on_reinsert() {
        // Same per-entry size as the eviction test: three entries overflow.
        let each = COMMIT_DIFF_CACHE_MAX_BYTES / 3 + 1;
        let mut app = build_app(3);
        insert_commit_diff_cache(&mut app, "c0".into(), mk_diff(each));
        insert_commit_diff_cache(&mut app, "c1".into(), mk_diff(each));
        // Re-insert c0 — it becomes the newest, so c1 is now the oldest.
        insert_commit_diff_cache(&mut app, "c0".into(), mk_diff(each));
        insert_commit_diff_cache(&mut app, "c2".into(), mk_diff(each));
        // c1 (oldest after c0's re-insertion) is evicted; c0 and c2 remain.
        assert!(app.repo.commit_diff_cache.contains_key("c0"));
        assert!(app.repo.commit_diff_cache.contains_key("c2"));
        assert!(!app.repo.commit_diff_cache.contains_key("c1"));
    }
}
