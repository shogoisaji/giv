use crate::app::{Mode, Panel};
use crate::git::ResetMode;

/// Messages produced by the event layer and forwarded to `update()`.
///
/// Each variant maps to a user intention (not a raw key). The keymap
/// layer translates `crossterm::event::KeyEvent` → `Action`.
#[derive(Debug, Clone)]
pub enum Action {
    // ── Application control ─────────────────────────────────────────────────
    Quit,
    Refresh,
    None,

    // ── Navigation ──────────────────────────────────────────────────────────
    SwitchMode(Mode),
    FocusNext,
    /// Focus the main/detail panel (e.g. Enter on a commit → read its diff).
    FocusMain,
    /// Focus the left/list panel (e.g. Esc → back to the commit list).
    FocusLeft,
    /// Mouse click on a panel: focus it and jump the cursor to `row` (the
    /// visible row index within the panel, 0-based). `row` is clamped to the
    /// list length by the update handler.
    ClickPanel {
        panel: Panel,
        row: usize,
    },
    /// Mouse click on the mode-tab strip: switch to the tab at `index` (0-based).
    ClickTab {
        index: usize,
    },
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Select,
    LoadPendingGraphDiff,

    // ── Staging ─────────────────────────────────────────────────────────────
    StageSelected,
    UnstageSelected,
    StageAll,
    UnstageAll,

    // ── Commit dialog ───────────────────────────────────────────────────────
    OpenCommitDialog,
    SubmitCommit(String),
    /// Open the amend dialog (pre-filled with HEAD's message).
    OpenAmendDialog,
    /// Submit the amended commit message.
    SubmitAmend(String),
    CancelDialog,
    DialogChar(char),
    DialogBackspace,
    /// Move focus to the next field within a multi-field dialog (e.g. tag
    /// name ↔ message). Bound to Tab while such a dialog is open.
    DialogFocusNext,

    // ── Diff scrolling ──────────────────────────────────────────────────────
    ScrollDiffUp,
    ScrollDiffDown,

    /// Scroll the graph VIEW (offset) without moving the selection — used by the
    /// mouse wheel so the device scroll feels like scrolling the display.
    ScrollGraphUp,
    ScrollGraphDown,

    /// Scroll the focused non-graph list VIEW (offset) without moving the
    /// selection — used by the mouse wheel in Status / Branches / Worktrees /
    /// Stashes so the device scroll feels like scrolling the display, mirroring
    /// `ScrollGraphUp`/`ScrollGraphDown`.
    ScrollListUp,
    ScrollListDown,

    /// Adjust the Graph-mode left/right split by the given percent delta
    /// (negative = narrower graph panel). Bound to `<` / `>`.
    AdjustGraphSplit(i16),

    // ── Background task completion ───────────────────────────────────────────
    /// Sent by a background thread when a git operation completes.
    GitTaskDone {
        label: String,
        ok: bool,
        message: String,
        refresh_after: bool,
        check_op: bool,
    },

    // ── Branches mode ────────────────────────────────────────────────────────
    /// Checkout the selected branch.
    Checkout,
    /// Open the "new branch" dialog.
    NewBranchDialog,
    /// Submit the new-branch name (from the dialog).
    SubmitNewBranch(String),
    /// Open the rename-branch dialog for the selected branch.
    RenameBranchDialog,
    /// Submit the new name for the branch being renamed.
    SubmitRenameBranch(String),
    /// Prompt to delete the selected item.
    DeleteSelected,
    /// Confirm a pending destructive operation.
    ConfirmDelete,

    // ── Network operations ───────────────────────────────────────────────────
    /// Open a confirm dialog for `git fetch --all`.
    Fetch,
    /// Open a confirm dialog for `git pull --ff-only`.
    Pull,
    /// Open a confirm dialog for `git push`.
    Push,
    /// Open a confirm dialog for `git push --force-with-lease`.
    ForcePush,

    // ── Worktrees mode ───────────────────────────────────────────────────────
    /// Open the "add worktree" dialog.
    WorktreeAddDialog,
    /// Submit the worktree path entered in the dialog.
    SubmitWorktreeAdd(String),
    /// Remove the selected worktree.
    WorktreeRemove,
    /// `git worktree prune`.
    WorktreePrune,
    /// cd into the selected worktree directory (quit + print path).
    SwitchWorktree,

    // ── Stash operations ─────────────────────────────────────────────────────
    /// Open the stash-save prompt (optionally pre-filled message).
    StashSavePrompt(String),
    /// Execute stash save with the given message (empty = no message).
    StashSave(String),
    /// Pop the selected stash entry (apply + drop).
    StashPop,
    /// Apply the selected stash entry (keep in stash list).
    StashApply,
    /// Drop the selected stash entry (with confirmation).
    StashDrop,

    // ── History operations (Graph mode) ──────────────────────────────────────
    /// Cherry-pick the commit currently selected in the graph.
    CherryPickSelected,
    /// Revert the commit currently selected in the graph (creates a new commit).
    RevertSelected,
    /// Open the reset-mode menu (soft / mixed / hard) for the selected commit.
    OpenResetMenu,
    /// Reset HEAD to the selected commit using the given mode.
    ResetTo(ResetMode),
    /// Rebase HEAD onto the selected ref (branch in Branches mode, commit in Graph mode).
    RebaseOntoSelected,
    /// Merge the selected branch into HEAD (Branches mode).
    MergeSelected,

    // ── Tag operations ────────────────────────────────────────────────────────
    /// Open the tag-create dialog (name field).
    TagCreatePrompt,
    /// Submit tag creation: inner String is "name\t<optional message>".
    SubmitTag(String),
    /// Delete the selected tag.
    TagDelete,

    // ── Inspect mode ─────────────────────────────────────────────────────────
    /// Open the "enter a commit ref" prompt in Inspect mode.
    OpenInspectPrompt,
    /// Submit the entered ref; resolve and display that commit.
    SubmitInspect(String),

    // ── Conflict / sequencer ─────────────────────────────────────────────────
    /// Continue the in-progress operation (merge/rebase/cherry-pick/revert).
    OpContinue,
    /// Abort the in-progress operation.
    OpAbort,
    /// Skip the current commit of the in-progress operation (rebase/cherry-pick/revert).
    OpSkip,
    /// Mark the selected conflicted file as resolved (`git add <path>`).
    MarkResolved,

    // ── Interactive rebase ───────────────────────────────────────────────────
    /// Open the interactive-rebase todo editor for the selected commit in Graph
    /// mode. Base = selected commit's first parent; entries = commits from
    /// base..HEAD (oldest first, as git expects).
    StartInteractiveRebase,
    /// Move the todo-list cursor up one row.
    RebaseTodoUp,
    /// Move the todo-list cursor down one row.
    RebaseTodoDown,
    /// Set the command on the cursor entry. The `char` maps:
    ///   'p'=pick, 'r'=reword, 'e'=edit, 's'=squash, 'f'=fixup, 'd'=drop.
    RebaseTodoSetCommand(char),
    /// Move the cursor entry one position earlier (towards the top / oldest).
    RebaseTodoMoveEntryUp,
    /// Move the cursor entry one position later (towards the bottom / newest).
    RebaseTodoMoveEntryDown,
    /// Execute the interactive rebase with the current todo list.
    RebaseTodoExecute,
    /// Discard the todo editor without performing any rebase.
    RebaseTodoCancel,

    // ── Phase 4: Polish ──────────────────────────────────────────────────────
    /// Cycle to the next built-in theme.
    CycleTheme,
    /// Open / toggle the command palette overlay.
    OpenPalette,
    /// Close the command palette without dispatching.
    ClosePalette,
    /// Append a character to the command palette query.
    PaletteChar(char),
    /// Delete the last character of the command palette query.
    PaletteBackspace,
    /// Move the palette cursor up.
    PaletteUp,
    /// Move the palette cursor down.
    PaletteDown,
    /// Confirm the highlighted palette item and dispatch its action.
    PaletteConfirm,
    /// Mouse click on palette item row `row` (0-based within the item list).
    /// Sets the cursor to that item and dispatches it, like `PaletteConfirm`.
    PaletteClick { row: usize },

    /// Open the search bar.
    OpenSearch,
    /// Close the search bar.
    CloseSearch,
    /// Append a character to the search query.
    SearchChar(char),
    /// Delete the last character of the search query.
    SearchBackspace,
    /// Jump to the next search match.
    SearchNext,
    /// Jump to the previous search match.
    SearchPrev,

    /// Toggle the help overlay.
    ToggleHelp,

    /// Copy the selected commit SHA (Graph / Inspect) or branch name (Branches)
    /// to the clipboard via OSC 52.
    YankSha,

    /// Toggle terminal mouse capture. When OFF, the terminal's own click-drag
    /// text selection works again (so the user can select & copy any on-screen
    /// text, e.g. a commit SHA); when ON, the app handles scroll / click-focus.
    ToggleMouseCapture,

    /// Toggle the Graph scope between all branches (`git log --all`) and the
    /// current HEAD's history only. Re-runs the log with the new scope.
    ToggleGraphScope,

    /// Toggle first-parent mode: collapse each merge's side branch so a
    /// merge-heavy trunk reads as one straight line (one row per merge).
    ToggleGraphFirstParent,

    /// Toggle the Branch lens: filter the graph to the selected commit's branch
    /// plus the main branch (a clean two-lane view that converges at the fork).
    ToggleGraphBranchFocus,

    /// Open the branch-compare dialog (Graph mode, `=` key).
    CompareBranches,
    /// Submit the compare dialog: resolve both base/target queries to the first
    /// matching branch and enter compare mode.
    CompareSubmit,
    /// Toggle between the base and target fields in the compare dialog.
    CompareToggleField,
    /// Cancel the compare dialog without entering compare mode.
    CompareCancel,
}
