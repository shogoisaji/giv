//! Graph mode — key resolution, including the interactive-rebase todo editor
//! (which takes full priority over all other keymaps; see [`crate::event`]).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;

pub(crate) fn resolve(key: KeyEvent) -> Action {
    match key.code {
        // Movement keys (j/k/g/G/PageUp/PageDown) are handled globally in
        // `crate::keymap` before this runs.
        // Enter focuses the detail panel so its diff can be scrolled; Esc
        // returns to the commit list.
        KeyCode::Enter => Action::FocusMain,
        KeyCode::Esc => Action::FocusLeft,
        KeyCode::Char('d') => Action::ScrollDiffDown,
        // Resize the graph / detail split.
        KeyCode::Char('<') => Action::AdjustGraphSplit(-5),
        KeyCode::Char('>') => Action::AdjustGraphSplit(5),
        // 'a' toggles scope: all branches ⇄ current branch only.
        KeyCode::Char('a') if key.modifiers == KeyModifiers::NONE => Action::ToggleGraphScope,
        // 'm' folds/expands merges (first-parent straight line ⇄ full).
        KeyCode::Char('m') if key.modifiers == KeyModifiers::NONE => Action::ToggleGraphFirstParent,
        // 'l' = Branch lens: focus the selected commit's branch vs main.
        KeyCode::Char('l') if key.modifiers == KeyModifiers::NONE => Action::ToggleGraphBranchFocus,
        // History operations on the selected commit.
        // 'c' = cherry-pick (note: lowercase, distinct from OpenCommitDialog which
        //        is only in Status mode).
        KeyCode::Char('c') if key.modifiers == KeyModifiers::NONE => Action::CherryPickSelected,
        // 'v' = reVert (mnemonic: inVerse of cherry-pick, or lazygit uses 'v').
        KeyCode::Char('v') if key.modifiers == KeyModifiers::NONE => Action::RevertSelected,
        // 'x' = reset (lazygit uses 'x' for custom patch; we use it for reset menu).
        KeyCode::Char('x') if key.modifiers == KeyModifiers::NONE => Action::OpenResetMenu,
        // 'b' = reBase onto selected commit.
        KeyCode::Char('b') if key.modifiers == KeyModifiers::NONE => Action::RebaseOntoSelected,
        // 'i' = interactive rebase from selected commit.
        KeyCode::Char('i') if key.modifiers == KeyModifiers::NONE => Action::StartInteractiveRebase,
        // 't' = tag create on selected commit.
        KeyCode::Char('t') if key.modifiers == KeyModifiers::NONE => Action::TagCreatePrompt,
        // 'D' = tag delete (capital to avoid accidental deletion).
        KeyCode::Char('D') => Action::TagDelete,
        // '=' = Compare branches (base..target picker).
        KeyCode::Char('=') if key.modifiers == KeyModifiers::NONE => Action::CompareBranches,
        // 'y' = yank / copy selected commit SHA via OSC 52.
        KeyCode::Char('y') if key.modifiers == KeyModifiers::NONE => Action::YankSha,
        // 'n' = next search match when not in search mode.
        KeyCode::Char('n') if key.modifiers == KeyModifiers::NONE => Action::SearchNext,
        // Network
        KeyCode::Char('f') if key.modifiers == KeyModifiers::NONE => Action::Fetch,
        KeyCode::Char('F') => Action::Pull,
        KeyCode::Char('P') => Action::Push,
        _ => Action::None,
    }
}

/// Resolve a key event when the interactive-rebase todo editor is open.
///
/// This takes full priority over all other keymaps (see [`crate::event`]).
pub(crate) fn resolve_rebase_todo(key: KeyEvent) -> Action {
    match key.code {
        // Cursor movement
        KeyCode::Char('j') | KeyCode::Down => Action::RebaseTodoDown,
        KeyCode::Char('k') | KeyCode::Up => Action::RebaseTodoUp,
        // Command assignment (lowercase = set command on current entry)
        KeyCode::Char('p') if key.modifiers == KeyModifiers::NONE => {
            Action::RebaseTodoSetCommand('p')
        }
        KeyCode::Char('r') if key.modifiers == KeyModifiers::NONE => {
            Action::RebaseTodoSetCommand('r')
        }
        KeyCode::Char('e') if key.modifiers == KeyModifiers::NONE => {
            Action::RebaseTodoSetCommand('e')
        }
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => {
            Action::RebaseTodoSetCommand('s')
        }
        KeyCode::Char('f') if key.modifiers == KeyModifiers::NONE => {
            Action::RebaseTodoSetCommand('f')
        }
        KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => {
            Action::RebaseTodoSetCommand('d')
        }
        // Entry reordering (Shift+J/K)
        KeyCode::Char('J') => Action::RebaseTodoMoveEntryDown,
        KeyCode::Char('K') => Action::RebaseTodoMoveEntryUp,
        // Execute / cancel
        KeyCode::Enter => Action::RebaseTodoExecute,
        KeyCode::Esc | KeyCode::Char('q') => Action::RebaseTodoCancel,
        _ => Action::None,
    }
}
