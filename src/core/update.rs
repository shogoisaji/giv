//! State-transition logic: `update(app, action) -> Effect`.
//!
//! This is the pure model layer. It mutates `app` and returns an `Effect`
//! that tells the event loop what to do next (quit, refresh, etc.).
//!
//! `update()` is the central **dispatcher**: cross-cutting concerns (navigation,
//! focus, scrolling, dialog text editing, the confirm executor, network,
//! sequencer, palette, search) are handled here, while mode-specific operations
//! are delegated to `crate::features::<mode>::update`. The shared orchestration
//! helpers ([`reload_selected_diff`], [`check_op_in_progress`]) are called back
//! into from the feature modules.

use crate::action::Action;
use crate::app::{App, ConfirmOp, Dialog, Mode, PaletteState, SearchState};
use crate::clipboard::osc52_copy;
use crate::effect::Effect;
use crate::features::branches::update as branches;
use crate::features::graph::update as graph;
use crate::features::inspect::update as inspect;
use crate::features::stashes::update as stashes;
use crate::features::status::update as status;
use crate::features::status::view as status_view;
use crate::features::worktrees::update as worktrees;
use crate::git::{self, ResetMode};
use crate::theme::Theme;

/// Rows moved per mouse-wheel tick when scrolling the diff or graph view.
const WHEEL_SCROLL_LINES: usize = 3;

/// Apply `action` to `app` and return the resulting `Effect`.
pub fn update(app: &mut App, action: Action) -> Effect {
    match action {
        // ── Application control ──────────────────────────────────────────────
        Action::Quit => {
            app.should_quit = true;
            Effect::Quit
        }

        Action::Refresh => {
            let previous_status = app.status_message.clone();
            match app.refresh() {
                Ok(()) => {
                    if app.status_message == previous_status {
                        app.status_message = None;
                    }
                }
                Err(e) => {
                    app.status_message = Some(format!("Refresh failed: {e:#}"));
                }
            }
            // Reload diff for the newly refreshed state.
            reload_selected_diff(app);
            Effect::Refresh
        }

        Action::None => Effect::None,

        // ── Mode switching ───────────────────────────────────────────────────
        Action::SwitchMode(mode) => {
            app.mode = mode;
            // Reset per-mode cursor to avoid stale positions.
            app.ui.list_index = 0;
            app.ui.list_offset = 0;
            app.ui.diff_scroll = 0;
            app.repo.selected_diff = None;
            // Focus the new mode's primary list panel for the active layout
            // (e.g. Changes in 3-pane Status, Graph in 3-pane Graph, Left list
            // in every two-pane mode). Coerce guards against a stale focus from
            // a different layout.
            let layout = app.ui.pane_layout(app.mode);
            app.ui.focus = Some(crate::ui::layout::primary_panel(layout, app.mode));
            // Inspect mode starts in navigation ("normal") mode, NOT input mode:
            // the user presses `i`/Enter to open the ref prompt and Esc to leave
            // it. Auto-opening the prompt here would trap keystrokes in the input
            // field, so the mode-switch number keys (1-6) could not be used.
            if app.mode != Mode::Inspect {
                // Eagerly load diff for the first item in the new mode.
                reload_selected_diff(app);
            }
            Effect::Refresh
        }

        // ── Inspect mode ─────────────────────────────────────────────────────
        Action::OpenInspectPrompt => {
            app.dialog = Dialog::InspectRef(app.inspect.query.clone());
            Effect::Refresh
        }

        Action::SubmitInspect(payload) => inspect::submit(app, payload),

        // ── Focus ────────────────────────────────────────────────────────────
        Action::FocusNext => {
            let layout = app.ui.pane_layout(app.mode);
            let cur = *app.ui.panel();
            // Coerce first so a stale focus (after a resize) never pins us to a
            // panel that does not exist in the current layout.
            let cur = crate::ui::layout::coerce_panel(cur, layout);
            let next = crate::ui::layout::next_panel(cur, layout);
            app.ui.focus = Some(next);
            // Tabbing onto a list panel refreshes the diff to reflect that
            // panel's selection (graph commit diff vs working-tree file diff),
            // so the diff pane always matches what the user is now navigating.
            let target = crate::ui::layout::focus_target(layout, app.mode, next);
            if matches!(
                target,
                crate::ui::layout::FocusTarget::Graph | crate::ui::layout::FocusTarget::Changes
            ) {
                app.ui.diff_scroll = 0;
                reload_selected_diff(app);
            }
            Effect::Refresh
        }

        Action::FocusMain => {
            // Enter on a commit/file → dive into the diff panel so it can be
            // scrolled with ↑/↓. Ensure the diff is loaded for the selection.
            let layout = app.ui.pane_layout(app.mode);
            app.ui.focus = Some(crate::ui::layout::diff_panel(layout));
            app.ui.diff_scroll = 0;
            reload_selected_diff(app);
            Effect::Refresh
        }

        Action::FocusLeft => {
            // Esc → back to the mode's primary list panel.
            // Also exits compare mode if active (like `C`).
            if app.compare.is_some() {
                app.compare = None;
                app.ui.graph_index = 0;
                app.ui.graph_offset = 0;
                refresh_silent(app);
                reload_selected_diff(app);
                app.status_message = Some("Exited compare mode".into());
            }
            let layout = app.ui.pane_layout(app.mode);
            app.ui.focus = Some(crate::ui::layout::primary_panel(layout, app.mode));
            Effect::Refresh
        }

        Action::ClickTab { index } => {
            // Click on a mode tab: switch to the corresponding mode.
            let mode = match index {
                0 => Mode::Status,
                1 => Mode::Graph,
                2 => Mode::Branches,
                3 => Mode::Worktrees,
                4 => Mode::Stashes,
                _ => Mode::Inspect,
            };
            // Reuse the existing SwitchMode logic by dispatching recursively.
            update(app, Action::SwitchMode(mode))
        }

        Action::ClickPanel { panel, row } => {
            // Mouse click on a panel: focus it and jump the cursor to the
            // clicked row (clamped to the list length). The diff is reloaded
            // so the diff pane immediately reflects the new selection.
            let layout = app.ui.pane_layout(app.mode);
            let panel = crate::ui::layout::coerce_panel(panel, layout);
            app.ui.focus = Some(panel);
            let target = crate::ui::layout::focus_target(layout, app.mode, panel);
            match target {
                crate::ui::layout::FocusTarget::Graph => {
                    let max = app.repo.commits.len().saturating_sub(1);
                    app.ui.graph_index = row.min(max);
                    // Keep scroll offset in sync.
                    if app.ui.graph_index < app.ui.graph_offset {
                        app.ui.graph_offset = app.ui.graph_index;
                    }
                }
                crate::ui::layout::FocusTarget::Changes => {
                    // The Changes list interleaves group headers ("Staged (n)",
                    // "Unstaged (m)") and placeholder rows with actual file rows,
                    // so the clicked display row ≠ logical file index. Convert
                    // via `display_row_to_logical`, adding the scroll offset
                    // because `row` is relative to the visible viewport.
                    let offset = app.ui.list_offset;
                    let display_row = row + offset;
                    app.ui.list_index = status_view::display_row_to_logical(app, display_row);
                }
                crate::ui::layout::FocusTarget::Other => match app.mode {
                    Mode::Branches => {
                        let max = app.branches.len().saturating_sub(1);
                        app.ui.branch_index = row.min(max);
                    }
                    Mode::Worktrees => {
                        let max = app.worktrees.len().saturating_sub(1);
                        app.ui.worktree_index = row.min(max);
                    }
                    Mode::Stashes => {
                        let max = app.stashes.len().saturating_sub(1);
                        app.ui.stash_index = row.min(max);
                    }
                    _ => {}
                },
                crate::ui::layout::FocusTarget::Diff => {
                    // Click on the diff panel: just focus it, no cursor to move.
                }
            }
            app.ui.diff_scroll = 0;
            reload_selected_diff(app);
            Effect::Refresh
        }

        // ── Navigation ───────────────────────────────────────────────────────
        Action::Up => {
            if diff_focused(app) {
                // Focus is on the diff panel → scroll the diff, keep selection.
                app.ui.diff_scroll = app.ui.diff_scroll.saturating_sub(1);
            } else {
                move_focused_list(app, -1);
                reload_selected_diff(app);
            }
            Effect::Refresh
        }

        Action::Down => {
            if diff_focused(app) {
                app.ui.diff_scroll = app.ui.diff_scroll.saturating_add(1);
            } else {
                move_focused_list(app, 1);
                reload_selected_diff(app);
            }
            Effect::Refresh
        }

        Action::Top => {
            if diff_focused(app) {
                app.ui.diff_scroll = 0;
            } else {
                jump_focused_list(app, true);
                reload_selected_diff(app);
            }
            Effect::Refresh
        }

        Action::Bottom => {
            if diff_focused(app) {
                // No reliable content height here; leave the diff scroll as-is.
            } else {
                jump_focused_list(app, false);
                reload_selected_diff(app);
            }
            Effect::Refresh
        }

        Action::PageUp => {
            if diff_focused(app) {
                app.ui.diff_scroll = app.ui.diff_scroll.saturating_sub(10);
                return Effect::Refresh;
            }
            move_focused_list(app, -10);
            reload_selected_diff(app);
            Effect::Refresh
        }

        Action::PageDown => {
            if diff_focused(app) {
                app.ui.diff_scroll = app.ui.diff_scroll.saturating_add(10);
                return Effect::Refresh;
            }
            move_focused_list(app, 10);
            reload_selected_diff(app);
            Effect::Refresh
        }

        Action::Select => {
            // Explicit Select always reloads the diff and resets scroll.
            app.ui.diff_scroll = 0;
            reload_selected_diff(app);
            Effect::Refresh
        }

        // ── Diff scroll ──────────────────────────────────────────────────────
        Action::ScrollDiffUp => {
            app.ui.diff_scroll = app.ui.diff_scroll.saturating_sub(WHEEL_SCROLL_LINES as u16);
            Effect::Refresh
        }

        Action::ScrollDiffDown => {
            app.ui.diff_scroll = app.ui.diff_scroll.saturating_add(WHEEL_SCROLL_LINES as u16);
            Effect::Refresh
        }

        Action::AdjustGraphSplit(delta) => {
            let cur = app.ui.graph_split as i16;
            app.ui.graph_split = (cur + delta).clamp(30, 80) as u16;
            Effect::Refresh
        }

        Action::ScrollGraphUp => {
            app.ui.graph_offset = app.ui.graph_offset.saturating_sub(WHEEL_SCROLL_LINES);
            Effect::Refresh
        }

        Action::ScrollGraphDown => {
            // Don't scroll past the last graph row.
            let total_rows = app
                .repo
                .commits
                .len()
                .saturating_mul(app.config.graph_row_step());
            let max_offset = total_rows.saturating_sub(1);
            app.ui.graph_offset = (app.ui.graph_offset + WHEEL_SCROLL_LINES).min(max_offset);
            Effect::Refresh
        }

        // ── Staging (Status mode) ────────────────────────────────────────────
        //
        // Space (StageSelected) acts as a toggle:
        //   • If the selected entry has unstaged changes → stage it.
        //   • If the selected entry is already fully staged → unstage it.
        Action::StageSelected => {
            status::toggle_stage_selected(app);
            Effect::Refresh
        }

        Action::UnstageSelected => {
            status::unstage_selected(app);
            Effect::Refresh
        }

        Action::StageAll => status::stage_all(app),

        Action::UnstageAll => status::unstage_all(app),

        // ── Commit dialog (Status mode) ──────────────────────────────────────
        Action::OpenCommitDialog => {
            app.dialog = Dialog::Commit(String::new());
            Effect::Refresh
        }

        Action::CancelDialog => {
            app.dialog = Dialog::None;
            Effect::Refresh
        }

        Action::DialogChar(c) => {
            if let Some(draft) = app.dialog.active_text_mut() {
                draft.push(c);
            }
            Effect::Refresh
        }

        Action::DialogFocusNext => {
            if let Dialog::TagCreate { focus_message, .. } = &mut app.dialog {
                *focus_message = !*focus_message;
            }
            Effect::Refresh
        }

        Action::DialogBackspace => {
            if let Some(draft) = app.dialog.active_text_mut() {
                draft.pop();
            }
            Effect::Refresh
        }

        Action::SubmitCommit(msg) => status::submit_commit(app, msg),

        Action::OpenAmendDialog => status::open_amend(app),

        Action::SubmitAmend(payload) => status::submit_amend(app, payload),

        // ── Branches mode ────────────────────────────────────────────────────
        Action::Checkout => branches::checkout(app),

        Action::NewBranchDialog => {
            app.dialog = Dialog::NewBranch(String::new());
            Effect::Refresh
        }

        Action::SubmitNewBranch(name) => branches::submit_new_branch(app, name),

        Action::RenameBranchDialog => branches::rename_branch_dialog(app),

        Action::SubmitRenameBranch(_) => branches::submit_rename_branch(app),

        // ── Delete (Branches / Worktrees) → cross-mode confirm flow ──────────
        Action::DeleteSelected => {
            match app.mode {
                Mode::Branches => {
                    let idx = app.ui.branch_index;
                    if let Some(branch) = app.branches.get(idx).cloned() {
                        if branch.is_head {
                            app.status_message =
                                Some("Cannot delete the currently checked-out branch".into());
                            return Effect::Refresh;
                        }
                        app.dialog = Dialog::Confirm {
                            message: format!("Delete branch '{}' ? (y/n)", branch.name),
                            pending: ConfirmOp::DeleteBranch {
                                name: branch.name,
                                force: false,
                            },
                        };
                    }
                }
                Mode::Worktrees => {
                    let idx = app.ui.worktree_index;
                    if let Some(wt) = app.worktrees.get(idx).cloned() {
                        if wt.is_current {
                            app.status_message = Some("Cannot remove the current worktree".into());
                            return Effect::Refresh;
                        }
                        app.dialog = Dialog::Confirm {
                            message: format!("Remove worktree '{}' ? (y/n)", wt.path),
                            pending: ConfirmOp::RemoveWorktree {
                                path: wt.path,
                                force: false,
                            },
                        };
                    }
                }
                _ => {}
            }
            Effect::Refresh
        }

        // ── Confirm executor (cross-mode) ────────────────────────────────────
        //
        // Feature modules decide WHAT to confirm (build a `ConfirmOp` + dialog);
        // this single arm executes the confirmed git operation uniformly by
        // dispatching to per-variant helpers in [`execute_confirm_op`].
        Action::ConfirmDelete => {
            if let Dialog::Confirm { pending, .. } = app.dialog.clone() {
                app.dialog = Dialog::None;
                execute_confirm_op(app, pending);
            } else {
                app.dialog = Dialog::None;
            }
            Effect::Refresh
        }

        // ── Network operations ───────────────────────────────────────────────
        Action::Fetch => {
            let root = app.repo.backend.root().to_path_buf();
            let label = "fetch".to_string();
            app.running_task = Some(label.clone());
            app.status_message = Some("Fetching…".into());
            git::spawn_git(
                root,
                vec!["fetch".to_string(), "--all".to_string()],
                label,
                app.task_tx.clone(),
                true,
                false,
            );
            Effect::Refresh
        }

        Action::Pull => {
            let root = app.repo.backend.root().to_path_buf();
            let label = "pull".to_string();
            app.running_task = Some(label.clone());
            app.status_message = Some("Pulling…".into());
            git::spawn_git(
                root,
                vec!["pull".to_string(), "--ff-only".to_string()],
                label,
                app.task_tx.clone(),
                true,
                false,
            );
            Effect::Refresh
        }

        Action::Push => {
            let root = app.repo.backend.root().to_path_buf();
            let label = "push".to_string();
            app.running_task = Some(label.clone());
            app.status_message = Some("Pushing…".into());

            // A branch with no upstream (never pushed) needs
            // `--set-upstream <remote> <branch>`; bare `git push` would fail with
            // "no upstream branch". Prefer 'origin', else the first remote.
            let args = match (
                app.repo.status.upstream.is_none(),
                app.repo.status.branch.clone(),
            ) {
                (true, Some(branch)) => {
                    let remotes = app.repo.backend.remotes().unwrap_or_default();
                    let remote = remotes
                        .iter()
                        .find(|(n, _)| n == "origin")
                        .or_else(|| remotes.first())
                        .map(|(n, _)| n.clone());
                    match remote {
                        Some(r) => {
                            vec!["push".to_string(), "--set-upstream".to_string(), r, branch]
                        }
                        None => vec!["push".to_string()],
                    }
                }
                _ => vec!["push".to_string()],
            };

            git::spawn_git(root, args, label, app.task_tx.clone(), true, false);
            Effect::Refresh
        }

        Action::GitTaskDone {
            label,
            ok,
            message,
            refresh_after,
            check_op,
        } => {
            // Clear running task spinner.
            if app.running_task.as_deref() == Some(label.as_str()) {
                app.running_task = None;
            }

            let message = message.trim();
            if ok {
                app.status_message = Some(if message.is_empty() {
                    format!("{label} completed")
                } else {
                    format!("{label}: {message}")
                });
            } else if message.is_empty() {
                app.status_message = Some(format!("{label} failed"));
            } else {
                app.status_message = Some(format!("{label} failed: {message}"));
            }

            if refresh_after {
                refresh_silent(app);
                reload_selected_diff(app);
            }
            if check_op {
                check_op_in_progress(app);
            }
            Effect::Refresh
        }

        // ── Worktrees mode ───────────────────────────────────────────────────
        Action::WorktreeAddDialog => {
            app.dialog = Dialog::WorktreeAdd(String::new());
            Effect::Refresh
        }

        Action::SubmitWorktreeAdd(path) => worktrees::submit_add(app, path),

        Action::WorktreeRemove => {
            // Reuse DeleteSelected logic for worktrees.
            update(app, Action::DeleteSelected)
        }

        Action::WorktreePrune => worktrees::prune(app),

        Action::SwitchWorktree => worktrees::switch(app),

        // ── Stash operations ─────────────────────────────────────────────────
        Action::StashSavePrompt(prefill) => {
            app.dialog = Dialog::StashSave(prefill);
            Effect::Refresh
        }

        Action::StashSave(msg) => stashes::save(app, msg),

        Action::StashPop => stashes::pop(app),

        Action::StashApply => stashes::apply(app),

        Action::StashDrop => stashes::drop_selected(app),

        // ── History operations (Graph mode) ──────────────────────────────────
        Action::CherryPickSelected => graph::cherry_pick_selected(app),

        Action::RevertSelected => graph::revert_selected(app),

        Action::OpenResetMenu => graph::open_reset_menu(app),

        Action::ResetTo(mode) => graph::reset_to(app, mode),

        Action::RebaseOntoSelected => graph::rebase_onto_selected(app),

        Action::MergeSelected => branches::merge_selected(app),

        // ── Tag operations ────────────────────────────────────────────────────
        Action::TagCreatePrompt => {
            app.dialog = Dialog::TagCreate {
                name: String::new(),
                message: String::new(),
                focus_message: false,
            };
            Effect::Refresh
        }

        Action::SubmitTag(payload) => graph::submit_tag(app, payload),

        Action::TagDelete => graph::tag_delete(app),

        // ── Conflict / sequencer operations ──────────────────────────────────
        Action::OpContinue => {
            if let Some(op) = app.op_in_progress.clone() {
                // Don't hand git a continue while conflicts remain — it would
                // fail with an opaque "unmerged files" error. Guide the user.
                if !op.conflicted.is_empty() {
                    app.status_message = Some(format!(
                        "{} file(s) still conflicted — resolve, then R to mark resolved before continuing",
                        op.conflicted.len()
                    ));
                    return Effect::Refresh;
                }
                match app.repo.backend.op_continue(op.kind) {
                    Ok(()) => {
                        app.status_message = Some("Operation continued".into());
                        refresh_silent(app);
                        reload_selected_diff(app);
                        check_op_in_progress(app);
                    }
                    Err(e) => {
                        app.status_message = Some(format!("Continue failed: {e:#}"));
                    }
                }
            } else {
                app.status_message = Some("No operation in progress".into());
            }
            Effect::Refresh
        }

        Action::OpAbort => {
            if let Some(op) = app.op_in_progress.clone() {
                match app.repo.backend.op_abort(op.kind) {
                    Ok(()) => {
                        app.status_message = Some("Operation aborted".into());
                        refresh_silent(app);
                        reload_selected_diff(app);
                    }
                    Err(e) => {
                        app.status_message = Some(format!("Abort failed: {e:#}"));
                    }
                }
            } else {
                app.status_message = Some("No operation in progress".into());
            }
            Effect::Refresh
        }

        Action::OpSkip => {
            if let Some(op) = app.op_in_progress.clone() {
                match app.repo.backend.op_skip(op.kind) {
                    Ok(()) => {
                        app.status_message = Some("Skipped current commit".into());
                        refresh_silent(app);
                        reload_selected_diff(app);
                        check_op_in_progress(app);
                    }
                    Err(e) => {
                        app.status_message = Some(format!("Skip failed: {e:#}"));
                        refresh_silent(app);
                        check_op_in_progress(app);
                    }
                }
            } else {
                app.status_message = Some("No operation in progress".into());
            }
            Effect::Refresh
        }

        Action::MarkResolved => {
            // Mark the selected conflicted file as resolved via `git add`.
            if let Some(entry) = status_view::resolve_entry(app, app.ui.list_index).cloned() {
                if entry.is_conflicted() {
                    match app.repo.backend.mark_resolved(&entry.path) {
                        Ok(()) => {
                            app.status_message =
                                Some(format!("Marked '{}' as resolved", entry.path));
                            refresh_silent(app);
                            reload_selected_diff(app);
                        }
                        Err(e) => {
                            app.status_message = Some(format!("Mark resolved failed: {e:#}"));
                        }
                    }
                } else {
                    app.status_message = Some(format!("'{}' has no conflicts", entry.path));
                }
            }
            Effect::Refresh
        }

        // ── Interactive rebase (Graph mode) ──────────────────────────────────
        Action::StartInteractiveRebase => graph::start_interactive_rebase(app),
        Action::RebaseTodoUp => graph::rebase_todo_up(app),
        Action::RebaseTodoDown => graph::rebase_todo_down(app),
        Action::RebaseTodoSetCommand(ch) => graph::rebase_todo_set_command(app, ch),
        Action::RebaseTodoMoveEntryUp => graph::rebase_todo_move_entry_up(app),
        Action::RebaseTodoMoveEntryDown => graph::rebase_todo_move_entry_down(app),
        Action::RebaseTodoExecute => graph::rebase_todo_execute(app),
        Action::RebaseTodoCancel => graph::rebase_todo_cancel(app),

        // ── Phase 4: Theme cycling ────────────────────────────────────────────
        Action::CycleTheme => {
            let names = Theme::theme_names();
            let current = names
                .iter()
                .position(|&n| n == app.theme_name.as_str())
                .unwrap_or(0);
            let next_idx = (current + 1) % names.len();
            let next_name = names[next_idx];
            app.theme_name = next_name.to_string();
            app.theme = Theme::from_name(next_name);
            // Persist the choice so it survives across sessions.
            app.config.theme = next_name.to_string();
            match crate::config::save_config(&app.config) {
                Ok(()) => app.status_message = Some(format!("Theme: {next_name} (saved)")),
                Err(e) => {
                    app.status_message = Some(format!("Theme: {next_name} (save failed: {e:#})"))
                }
            }
            Effect::Refresh
        }

        // ── Phase 4: Command Palette ──────────────────────────────────────────
        Action::OpenPalette => {
            app.palette = Some(PaletteState::new());
            Effect::Refresh
        }

        Action::ClosePalette => {
            app.palette = None;
            Effect::Refresh
        }

        Action::PaletteChar(c) => {
            if let Some(ref mut state) = app.palette {
                state.query.push(c);
                state.refilter();
                state.cursor = 0;
            }
            Effect::Refresh
        }

        Action::PaletteBackspace => {
            if let Some(ref mut state) = app.palette {
                state.query.pop();
                state.refilter();
            }
            Effect::Refresh
        }

        Action::PaletteUp => {
            if let Some(ref mut state) = app.palette {
                if state.cursor > 0 {
                    state.cursor -= 1;
                }
            }
            Effect::Refresh
        }

        Action::PaletteDown => {
            if let Some(ref mut state) = app.palette {
                let max = state.items.len().saturating_sub(1);
                if state.cursor < max {
                    state.cursor += 1;
                }
            }
            Effect::Refresh
        }

        Action::PaletteConfirm => {
            // Extract the action from the selected item, then close the palette
            // and dispatch the action through update() recursively.
            let selected_action = app
                .palette
                .as_ref()
                .and_then(|s| s.items.get(s.cursor))
                .map(|item| item.action.clone());

            app.palette = None;

            if let Some(action) = selected_action {
                update(app, action)
            } else {
                Effect::Refresh
            }
        }

        // ── Phase 4: Search / Filter ──────────────────────────────────────────
        Action::OpenSearch => {
            app.search = Some(SearchState {
                query: String::new(),
                matches: Vec::new(),
                current: 0,
            });
            Effect::Refresh
        }

        Action::CloseSearch => {
            app.search = None;
            Effect::Refresh
        }

        Action::SearchChar(c) => {
            if app.search.is_some() {
                // Step 1: mutate the query.
                if let Some(ref mut state) = app.search {
                    state.query.push(c);
                }
                // Step 2: snapshot what we need (no borrow of `app.search`).
                let query = app
                    .search
                    .as_ref()
                    .map(|s| s.query.clone())
                    .unwrap_or_default();
                let matches = crate::search::compute_search_matches(
                    app.mode,
                    &app.repo.commits,
                    &app.branches,
                    &app.repo.status,
                    &query,
                );
                // Step 3: store matches.
                if let Some(ref mut state) = app.search {
                    state.matches = matches;
                    state.current = 0;
                }
                // Jump to first match.
                crate::search::jump_to_match(app);
            }
            Effect::Refresh
        }

        Action::SearchBackspace => {
            if app.search.is_some() {
                if let Some(ref mut state) = app.search {
                    state.query.pop();
                }
                let query = app
                    .search
                    .as_ref()
                    .map(|s| s.query.clone())
                    .unwrap_or_default();
                let matches = crate::search::compute_search_matches(
                    app.mode,
                    &app.repo.commits,
                    &app.branches,
                    &app.repo.status,
                    &query,
                );
                if let Some(ref mut state) = app.search {
                    let new_current = if matches.is_empty() {
                        0
                    } else if state.current >= matches.len() {
                        matches.len() - 1
                    } else {
                        state.current
                    };
                    state.matches = matches;
                    state.current = new_current;
                }
                crate::search::jump_to_match(app);
            }
            Effect::Refresh
        }

        Action::SearchNext => {
            if let Some(ref mut state) = app.search {
                if !state.matches.is_empty() {
                    state.current = (state.current + 1) % state.matches.len();
                }
            }
            crate::search::jump_to_match(app);
            reload_selected_diff(app);
            Effect::Refresh
        }

        Action::SearchPrev => {
            if let Some(ref mut state) = app.search {
                if !state.matches.is_empty() {
                    state.current = if state.current == 0 {
                        state.matches.len() - 1
                    } else {
                        state.current - 1
                    };
                }
            }
            crate::search::jump_to_match(app);
            reload_selected_diff(app);
            Effect::Refresh
        }

        // ── Phase 4: Help overlay ─────────────────────────────────────────────
        Action::ToggleHelp => {
            app.show_help = !app.show_help;
            Effect::Refresh
        }

        // ── Phase 4: Clipboard copy (OSC 52) ──────────────────────────────────
        Action::YankSha => {
            let text = match app.mode {
                Mode::Graph => {
                    let idx = app.ui.graph_index;
                    app.repo.commits.get(idx).map(|c| c.id.clone())
                }
                Mode::Branches => {
                    let idx = app.ui.branch_index;
                    app.branches.get(idx).map(|b| b.name.clone())
                }
                Mode::Inspect => app.inspect.commit.as_ref().map(|c| c.id.clone()),
                _ => None,
            };

            if let Some(ref t) = text {
                osc52_copy(t);
                // Show a truncated preview in the status message.
                let preview = if t.len() > 12 { &t[..12] } else { t.as_str() };
                app.status_message = Some(format!("Copied {preview}…"));
            } else {
                app.status_message = Some("Nothing to copy".into());
            }
            Effect::Refresh
        }

        Action::ToggleMouseCapture => {
            app.mouse_capture = !app.mouse_capture;
            app.status_message = Some(if app.mouse_capture {
                "Mouse ON — wheel-scroll & click-to-focus (text selection off)".into()
            } else {
                "Mouse OFF — drag to select & copy text (default)".into()
            });
            Effect::SetMouseCapture(app.mouse_capture)
        }

        // ── Graph view toggles (Graph mode) ──────────────────────────────────
        Action::ToggleGraphScope => graph::toggle_scope(app),
        Action::ToggleGraphFirstParent => graph::toggle_first_parent(app),
        Action::ToggleGraphBranchFocus => graph::toggle_branch_focus(app),

        // ── Branch compare ────────────────────────────────────────────────
        Action::CompareBranches => {
            if app.compare.is_some() {
                // Already in compare mode — `=` exits it.
                app.compare = None;
                app.ui.graph_index = 0;
                app.ui.graph_offset = 0;
                refresh_silent(app);
                reload_selected_diff(app);
                app.status_message = Some("Exited compare mode".into());
            } else {
                // Open the compare dialog with both fields empty, base focused.
                app.dialog = Dialog::CompareBranches {
                    base: String::new(),
                    target: String::new(),
                    focus_target: false,
                };
            }
            Effect::Refresh
        }
        Action::CompareToggleField => {
            if let Dialog::CompareBranches { focus_target, .. } = &mut app.dialog {
                *focus_target = !*focus_target;
            }
            Effect::Refresh
        }
        Action::CompareSubmit => {
            // Resolve both base and target queries to the first matching branch.
            let (base_query, target_query) = match &app.dialog {
                Dialog::CompareBranches { base, target, .. } => (base.clone(), target.clone()),
                _ => return Effect::Refresh,
            };
            let resolve = |query: &str| -> Option<String> {
                let q = query.to_lowercase();
                app.branches
                    .iter()
                    .filter(|b| b.kind == crate::git::RefKind::LocalBranch)
                    .find(|b| b.name.to_lowercase().contains(&q))
                    .map(|b| b.name.clone())
            };
            let base = resolve(&base_query);
            let target = resolve(&target_query);
            match (base, target) {
                (Some(b), Some(t)) => {
                    app.dialog = Dialog::None;
                    app.compare = Some((b.clone(), t.clone()));
                    app.ui.graph_index = 0;
                    app.ui.graph_offset = 0;
                    refresh_silent(app);
                    reload_selected_diff(app);
                    app.status_message = Some(format!(
                        "Comparing: {b}..{t} ({} commits)",
                        app.repo.commits.len()
                    ));
                }
                _ => {
                    app.status_message = Some("No matching branch for base or target".into());
                }
            }
            Effect::Refresh
        }
        Action::CompareCancel => {
            app.dialog = Dialog::None;
            Effect::Refresh
        }
    }
}

// ─── Shared orchestration helpers ─────────────────────────────────────────────

/// Best-effort refresh after a git operation. Errors are logged but not shown
/// to the user (the operation itself already set a status message on success
/// or failure). Used in 30+ places where a refresh failure is non-fatal.
pub(crate) fn refresh_silent(app: &mut App) {
    if let Err(e) = app.refresh() {
        tracing::warn!("refresh failed after git op: {e:#}");
    }
}

// ── Confirm executor helpers ──────────────────────────────────────────────────
//
// Each helper runs one `ConfirmOp` variant, sets a status message, then calls
// `refresh_silent` + `reload_selected_diff` (which calls `clamp_cursors`).
// History-rewriting ops also call `check_op_in_progress`.  The manual cursor
// clamping that used to live in the inline match arms is now handled centrally
// by `clamp_cursors` inside `reload_selected_diff`.

fn execute_confirm_op(app: &mut App, pending: ConfirmOp) {
    let task = confirm_task(pending);
    start_git_task(app, task.label, task.args, true, task.check_op);
}

struct GitTaskSpec {
    label: String,
    args: Vec<String>,
    check_op: bool,
}

fn confirm_task(pending: ConfirmOp) -> GitTaskSpec {
    match pending {
        ConfirmOp::DeleteBranch { name, force } => GitTaskSpec {
            label: format!("delete branch {name}"),
            args: vec![
                "branch".into(),
                if force { "-D".into() } else { "-d".into() },
                name,
            ],
            check_op: false,
        },
        ConfirmOp::RemoveWorktree { path, force } => {
            let mut args = vec!["worktree".into(), "remove".into()];
            if force {
                args.push("--force".into());
            }
            args.push(path.clone());
            GitTaskSpec {
                label: format!("remove worktree {path}"),
                args,
                check_op: false,
            }
        }
        ConfirmOp::StashDrop { index } => GitTaskSpec {
            label: format!("drop stash@{{{index}}}"),
            args: vec!["stash".into(), "drop".into(), format!("stash@{{{index}}}")],
            check_op: false,
        },
        ConfirmOp::Reset { mode, target } => {
            let flag = match mode {
                ResetMode::Soft => "--soft",
                ResetMode::Mixed => "--mixed",
                ResetMode::Hard => "--hard",
            };
            GitTaskSpec {
                label: format!("reset {flag} {}", crate::git::short_oid(&target)),
                args: vec!["reset".into(), flag.into(), target],
                check_op: false,
            }
        }
        ConfirmOp::TagDelete { name } => GitTaskSpec {
            label: format!("delete tag {name}"),
            args: vec!["tag".into(), "-d".into(), name],
            check_op: false,
        },
        ConfirmOp::CherryPick { oid } => GitTaskSpec {
            label: format!("cherry-pick {}", crate::git::short_oid(&oid)),
            args: vec!["cherry-pick".into(), oid],
            check_op: true,
        },
        ConfirmOp::Revert { oid } => GitTaskSpec {
            label: format!("revert {}", crate::git::short_oid(&oid)),
            args: vec!["revert".into(), oid],
            check_op: true,
        },
        ConfirmOp::RebaseOnto { target, display } => GitTaskSpec {
            label: format!("rebase onto {display}"),
            args: vec!["rebase".into(), target],
            check_op: true,
        },
        ConfirmOp::Merge { branch } => GitTaskSpec {
            label: format!("merge {branch}"),
            // Keep this in sync with CliBackend::merge(branch, no_ff=false):
            // suppress the editor and allow fast-forward.
            args: vec!["merge".into(), "--no-edit".into(), branch],
            check_op: true,
        },
    }
}

fn start_git_task(
    app: &mut App,
    label: String,
    args: Vec<String>,
    refresh_after: bool,
    check_op: bool,
) {
    let root = app.repo.backend.root().to_path_buf();
    app.running_task = Some(label.clone());
    app.status_message = Some(format!("{label}…"));
    git::spawn_git(
        root,
        args,
        label,
        app.task_tx.clone(),
        refresh_after,
        check_op,
    );
}

/// After a git operation that may produce conflicts, refresh `op_in_progress`
/// and set a helpful status message if a conflict was detected. Called from the
/// feature modules after history-rewriting operations.
pub(crate) fn check_op_in_progress(app: &mut App) {
    if let Some(ref op) = app.op_in_progress {
        if !op.conflicted.is_empty() {
            let kind = format!("{:?}", op.kind).to_lowercase();
            app.status_message = Some(format!(
                "{kind} conflict: {} file(s) — [C]ontinue [A]bort. Switch to Status to resolve.",
                op.conflicted.len()
            ));
            app.mode = Mode::Status;
        }
    }
}

/// The responsive pane layout for the currently-recorded terminal width and
/// the active mode.
fn current_layout(app: &App) -> crate::ui::layout::PaneLayout {
    app.ui.pane_layout(app.mode)
}

/// The semantic target of the currently-focused panel under the active layout.
fn current_focus_target(app: &App) -> crate::ui::layout::FocusTarget {
    crate::ui::layout::focus_target(current_layout(app), app.mode, *app.ui.panel())
}

/// Move the focused list's cursor by `delta` rows (negative = up), clamped to
/// `[0, len-1]`. Dispatches on the focused panel's semantic target so the same
/// key does the right thing in both two-pane and three-pane layouts. The diff
/// target is a no-op here (movement scrolls the diff at the call site).
fn move_focused_list(app: &mut App, delta: i64) {
    match current_focus_target(app) {
        crate::ui::layout::FocusTarget::Graph => {
            let max = app.repo.commits.len().saturating_sub(1);
            let new = (app.ui.graph_index as i64 + delta).clamp(0, max as i64) as usize;
            app.ui.graph_index = new;
            // Keep the scroll offset in sync when the cursor passes the top.
            if app.ui.graph_index < app.ui.graph_offset {
                app.ui.graph_offset = app.ui.graph_index;
            }
        }
        crate::ui::layout::FocusTarget::Changes => {
            let max = status_view::status_row_count(app).saturating_sub(1);
            let new = (app.ui.list_index as i64 + delta).clamp(0, max as i64) as usize;
            app.ui.list_index = new;
        }
        crate::ui::layout::FocusTarget::Other => match app.mode {
            Mode::Branches => {
                let max = app.branches.len().saturating_sub(1);
                app.ui.branch_index =
                    (app.ui.branch_index as i64 + delta).clamp(0, max as i64) as usize;
            }
            Mode::Worktrees => {
                let max = app.worktrees.len().saturating_sub(1);
                app.ui.worktree_index =
                    (app.ui.worktree_index as i64 + delta).clamp(0, max as i64) as usize;
            }
            Mode::Stashes => {
                let max = app.stashes.len().saturating_sub(1);
                app.ui.stash_index =
                    (app.ui.stash_index as i64 + delta).clamp(0, max as i64) as usize;
            }
            _ => {}
        },
        crate::ui::layout::FocusTarget::Diff => {}
    }
}

/// Jump the focused list's cursor to the top (`to_top = true`) or bottom.
fn jump_focused_list(app: &mut App, to_top: bool) {
    match current_focus_target(app) {
        crate::ui::layout::FocusTarget::Graph => {
            if to_top {
                app.ui.graph_index = 0;
                app.ui.graph_offset = 0;
            } else {
                app.ui.graph_index = app.repo.commits.len().saturating_sub(1);
            }
        }
        crate::ui::layout::FocusTarget::Changes => {
            if to_top {
                app.ui.list_index = 0;
                app.ui.list_offset = 0;
            } else {
                let len = status_view::status_row_count(app);
                app.ui.list_index = len.saturating_sub(1);
            }
        }
        crate::ui::layout::FocusTarget::Other => match app.mode {
            Mode::Branches => {
                app.ui.branch_index = if to_top {
                    0
                } else {
                    app.branches.len().saturating_sub(1)
                };
            }
            Mode::Worktrees => {
                app.ui.worktree_index = if to_top {
                    0
                } else {
                    app.worktrees.len().saturating_sub(1)
                };
            }
            Mode::Stashes => {
                app.ui.stash_index = if to_top {
                    0
                } else {
                    app.stashes.len().saturating_sub(1)
                };
            }
            _ => {}
        },
        crate::ui::layout::FocusTarget::Diff => {}
    }
}

/// Whether the diff panel currently has focus (so movement keys scroll the diff
/// instead of changing the selected file/commit).
pub(crate) fn diff_focused(app: &App) -> bool {
    if matches!(app.mode, Mode::Inspect) {
        // Inspect mode is a single scrollable commit view — movement always scrolls.
        return true;
    }
    matches!(
        current_focus_target(app),
        crate::ui::layout::FocusTarget::Diff
    )
}

/// Clamp every list cursor to its current list length so a mutation that shrank
/// a list (commit/reset/rebase/stage-all/stash drop/…) never leaves the cursor
/// pointing past the end. Called from `reload_selected_diff`, which runs after
/// almost every mutating operation.
fn clamp_cursors(app: &mut App) {
    let rows = status_view::status_row_count(app);
    app.ui.list_index = app.ui.list_index.min(rows.saturating_sub(1));
    app.ui.graph_index = app
        .ui
        .graph_index
        .min(app.repo.commits.len().saturating_sub(1));
    app.ui.branch_index = app
        .ui
        .branch_index
        .min(app.branches.len().saturating_sub(1));
    app.ui.worktree_index = app
        .ui
        .worktree_index
        .min(app.worktrees.len().saturating_sub(1));
    app.ui.stash_index = app.ui.stash_index.min(app.stashes.len().saturating_sub(1));
}

/// Reload the diff for whatever is currently selected, dispatching to the
/// focused panel's loader. The diff scroll is reset to the top because the
/// content just changed. Called from almost every mutating operation across the
/// feature modules, so it is `pub(crate)`.
///
/// Dispatch is by focused semantic target (not raw mode) so the three-pane
/// dashboard's diff pane always reflects the panel the user is navigating:
/// the graph commit diff when the graph is focused, the working-tree file diff
/// when the change list is focused. The `Diff` target is a no-op (the diff is
/// being viewed, not driven by a list selection).
pub(crate) fn reload_selected_diff(app: &mut App) {
    clamp_cursors(app);
    app.ui.diff_scroll = 0;
    match current_focus_target(app) {
        crate::ui::layout::FocusTarget::Graph => {
            graph::clamp_graph_offset(app);
            graph::load_graph_diff(app);
        }
        crate::ui::layout::FocusTarget::Changes => {
            status::clamp_list_offset(app);
            status::load_status_diff(app);
        }
        crate::ui::layout::FocusTarget::Diff => {}
        crate::ui::layout::FocusTarget::Other => {
            if matches!(app.mode, Mode::Stashes) {
                stashes::load_stash_diff(app);
            }
        }
    }
}

// Search match-computation and cursor-jump helpers live in `crate::core::search`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Panel;
    use crate::config::Config;
    use crate::test_backend::{mk_branch, mk_commit, MockBackend};

    fn build_app(backend: MockBackend) -> App {
        App::new(Box::new(backend), Config::default()).expect("app builds")
    }

    // ── confirm_task variants ────────────────────────────────────────────────

    #[test]
    fn confirm_task_delete_branch_builds_args() {
        let task = confirm_task(ConfirmOp::DeleteBranch {
            name: "feat".into(),
            force: true,
        });
        assert_eq!(task.label, "delete branch feat");
        assert_eq!(task.args, vec!["branch", "-D", "feat"]);
        assert!(!task.check_op);
    }

    #[test]
    fn confirm_task_remove_worktree_builds_args() {
        let task = confirm_task(ConfirmOp::RemoveWorktree {
            path: "/tmp/wt".into(),
            force: true,
        });
        assert_eq!(task.label, "remove worktree /tmp/wt");
        assert_eq!(task.args, vec!["worktree", "remove", "--force", "/tmp/wt"]);
    }

    #[test]
    fn confirm_task_stash_drop_builds_args() {
        let task = confirm_task(ConfirmOp::StashDrop { index: 2 });
        assert_eq!(task.label, "drop stash@{2}");
        assert_eq!(task.args, vec!["stash", "drop", "stash@{2}"]);
    }

    #[test]
    fn confirm_task_reset_hard_builds_args() {
        let task = confirm_task(ConfirmOp::Reset {
            mode: ResetMode::Hard,
            target: "abcdef1234567".into(),
        });
        assert_eq!(task.label, "reset --hard abcdef1");
        assert_eq!(task.args, vec!["reset", "--hard", "abcdef1234567"]);
    }

    #[test]
    fn confirm_task_reset_soft_builds_args() {
        let task = confirm_task(ConfirmOp::Reset {
            mode: ResetMode::Soft,
            target: "abcdef1234567".into(),
        });
        assert_eq!(task.label, "reset --soft abcdef1");
        assert_eq!(task.args, vec!["reset", "--soft", "abcdef1234567"]);
    }

    #[test]
    fn confirm_task_tag_delete_builds_args() {
        let task = confirm_task(ConfirmOp::TagDelete {
            name: "v1.0".into(),
        });
        assert_eq!(task.label, "delete tag v1.0");
        assert_eq!(task.args, vec!["tag", "-d", "v1.0"]);
    }

    #[test]
    fn confirm_task_cherry_pick_builds_args_and_checks_op() {
        let task = confirm_task(ConfirmOp::CherryPick {
            oid: "abcdef1234567".into(),
        });
        assert_eq!(task.label, "cherry-pick abcdef1");
        assert_eq!(task.args, vec!["cherry-pick", "abcdef1234567"]);
        assert!(task.check_op);
    }

    #[test]
    fn confirm_task_revert_builds_args_and_checks_op() {
        let task = confirm_task(ConfirmOp::Revert {
            oid: "abcdef1234567".into(),
        });
        assert_eq!(task.label, "revert abcdef1");
        assert_eq!(task.args, vec!["revert", "abcdef1234567"]);
        assert!(task.check_op);
    }

    #[test]
    fn confirm_task_rebase_onto_builds_args_and_checks_op() {
        let task = confirm_task(ConfirmOp::RebaseOnto {
            target: "main".into(),
            display: "main".into(),
        });
        assert_eq!(task.label, "rebase onto main");
        assert_eq!(task.args, vec!["rebase", "main"]);
        assert!(task.check_op);
    }

    #[test]
    fn confirm_task_merge_builds_args_and_checks_op() {
        let task = confirm_task(ConfirmOp::Merge {
            branch: "dev".into(),
        });
        assert_eq!(task.label, "merge dev");
        assert_eq!(task.args, vec!["merge", "--no-edit", "dev"]);
        assert!(task.check_op);
    }

    #[test]
    fn git_task_done_with_check_op_refreshes_and_surfaces_conflict() {
        let mut b = MockBackend::new();
        b.op_in_progress = Some(crate::git::OpInProgress {
            kind: crate::git::OpKind::Rebase,
            conflicted: vec!["file.txt".into()],
        });
        let mut app = build_app(b);

        update(
            &mut app,
            Action::GitTaskDone {
                label: "rebase onto main".into(),
                ok: false,
                message: "conflict".into(),
                refresh_after: true,
                check_op: true,
            },
        );

        assert_eq!(app.running_task, None);
        assert_eq!(app.mode, Mode::Status);
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .contains("rebase conflict"));
    }

    // ── diff_focused ─────────────────────────────────────────────────────────

    #[test]
    fn diff_focused_true_for_inspect_mode() {
        let mut b = MockBackend::new();
        b.commits = vec![mk_commit("a", "first", false)];
        let mut app = build_app(b);
        app.mode = Mode::Inspect;
        assert!(diff_focused(&app));
    }

    #[test]
    fn diff_focused_false_for_status_with_left_panel() {
        let mut b = MockBackend::new();
        b.commits = vec![mk_commit("a", "first", false)];
        let mut app = build_app(b);
        app.mode = Mode::Status;
        app.ui.focus = Some(Panel::Left);
        assert!(!diff_focused(&app));
    }

    #[test]
    fn diff_focused_true_when_main_panel_focused_in_two_pane() {
        let mut b = MockBackend::new();
        b.commits = vec![mk_commit("a", "first", false)];
        let mut app = build_app(b);
        app.mode = Mode::Status;
        app.ui.focus = Some(Panel::Main);
        // Two-pane layout (width 0) → Main panel = Diff target.
        assert!(diff_focused(&app));
    }

    // ── current_layout / current_focus_target ────────────────────────────────

    #[test]
    fn current_layout_status_narrow_is_two_pane() {
        let b = MockBackend::new();
        let app = build_app(b);
        assert_eq!(current_layout(&app), crate::ui::layout::PaneLayout::TwoPane);
    }

    #[test]
    fn current_layout_graph_wide_is_three_pane() {
        let b = MockBackend::new();
        let mut app = build_app(b);
        app.mode = Mode::Graph;
        app.ui.width.set(200);
        assert_eq!(
            current_layout(&app),
            crate::ui::layout::PaneLayout::ThreePane
        );
    }

    #[test]
    fn current_focus_target_status_left_is_changes() {
        let b = MockBackend::new();
        let mut app = build_app(b);
        app.mode = Mode::Status;
        app.ui.focus = Some(Panel::Left);
        assert_eq!(
            current_focus_target(&app),
            crate::ui::layout::FocusTarget::Changes
        );
    }

    #[test]
    fn current_focus_target_graph_left_is_graph() {
        let b = MockBackend::new();
        let mut app = build_app(b);
        app.mode = Mode::Graph;
        app.ui.focus = Some(Panel::Left);
        assert_eq!(
            current_focus_target(&app),
            crate::ui::layout::FocusTarget::Graph
        );
    }

    // ── clamp_cursors ────────────────────────────────────────────────────────

    #[test]
    fn clamp_cursors_clamps_graph_index() {
        let mut b = MockBackend::new();
        b.commits = vec![mk_commit("a", "first", false)];
        let mut app = build_app(b);
        app.ui.graph_index = 999;
        clamp_cursors(&mut app);
        assert_eq!(app.ui.graph_index, 0);
    }

    #[test]
    fn clamp_cursors_clamps_branch_index() {
        let mut b = MockBackend::new();
        b.branches = vec![mk_branch("main", "abc"), mk_branch("dev", "def")];
        let mut app = build_app(b);
        app.ui.branch_index = 999;
        clamp_cursors(&mut app);
        assert_eq!(app.ui.branch_index, 1);
    }

    #[test]
    fn clamp_cursors_clamps_worktree_index() {
        let b = MockBackend::new();
        let mut app = build_app(b);
        app.ui.worktree_index = 999;
        clamp_cursors(&mut app);
        // No worktrees → saturating_sub(0) = 0.
        assert_eq!(app.ui.worktree_index, 0);
    }

    #[test]
    fn clamp_cursors_clamps_stash_index() {
        let b = MockBackend::new();
        let mut app = build_app(b);
        app.ui.stash_index = 999;
        clamp_cursors(&mut app);
        assert_eq!(app.ui.stash_index, 0);
    }

    #[test]
    fn clamp_cursors_no_op_when_indices_in_range() {
        let mut b = MockBackend::new();
        b.commits = vec![
            mk_commit("a", "first", false),
            mk_commit("b", "second", false),
            mk_commit("c", "third", false),
        ];
        let mut app = build_app(b);
        app.ui.graph_index = 1;
        clamp_cursors(&mut app);
        assert_eq!(app.ui.graph_index, 1);
    }

    // ── check_op_in_progress ─────────────────────────────────────────────────

    #[test]
    fn check_op_in_progress_switches_to_status_on_conflict() {
        use crate::git::{OpInProgress, OpKind};
        let b = MockBackend::new();
        let mut app = build_app(b);
        app.mode = Mode::Graph;
        app.op_in_progress = Some(OpInProgress {
            kind: OpKind::Rebase,
            conflicted: vec!["src/main.rs".into()],
        });
        check_op_in_progress(&mut app);
        assert_eq!(app.mode, Mode::Status);
        let msg = app.status_message.as_deref().unwrap();
        assert!(msg.contains("rebase conflict"));
        assert!(msg.contains("1 file"));
    }

    #[test]
    fn check_op_in_progress_no_op_when_no_conflicts() {
        use crate::git::{OpInProgress, OpKind};
        let b = MockBackend::new();
        let mut app = build_app(b);
        app.mode = Mode::Graph;
        app.op_in_progress = Some(OpInProgress {
            kind: OpKind::Rebase,
            conflicted: vec![],
        });
        check_op_in_progress(&mut app);
        // Mode unchanged, no status message set by this function.
        assert_eq!(app.mode, Mode::Graph);
    }

    #[test]
    fn check_op_in_progress_no_op_when_none() {
        let b = MockBackend::new();
        let mut app = build_app(b);
        app.mode = Mode::Graph;
        check_op_in_progress(&mut app);
        assert_eq!(app.mode, Mode::Graph);
    }
}
