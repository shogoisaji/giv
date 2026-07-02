//! Status mode — key resolution. Dispatched from [`crate::keymap`] after the
//! global bindings have had their chance.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;

pub(crate) fn resolve(key: KeyEvent) -> Action {
    match key.code {
        // Movement keys (j/k/g/G/PageUp/PageDown) are handled globally in
        // `crate::keymap` before this runs.
        // Enter focuses the diff panel (scroll with ↑/↓); Esc returns to the
        // file list.
        KeyCode::Enter => Action::FocusMain,
        KeyCode::Esc => Action::FocusLeft,
        // Staging
        KeyCode::Char(' ') => Action::StageSelected,
        // Ctrl-u scrolls the diff up; plain u unstages the selection.
        KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => Action::ScrollDiffUp,
        KeyCode::Char('u') => Action::UnstageSelected,
        KeyCode::Char('a') if key.modifiers == KeyModifiers::NONE => Action::StageAll,
        KeyCode::Char('A') => Action::UnstageAll,
        // Commit
        KeyCode::Char('c') if key.modifiers == KeyModifiers::NONE => Action::OpenCommitDialog,
        // 'e' = amEnd the last commit (reword + fold in staged changes).
        KeyCode::Char('e') if key.modifiers == KeyModifiers::NONE => Action::OpenAmendDialog,
        // Diff scroll
        KeyCode::Char('d') => Action::ScrollDiffDown,
        // Conflict resolution: 'R' marks the selected conflicted file as resolved.
        KeyCode::Char('R') => Action::MarkResolved,
        // Stash save shortcut available from status view.
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => {
            Action::StashSavePrompt(String::new())
        }
        // Tag create shortcut.
        KeyCode::Char('t') if key.modifiers == KeyModifiers::NONE => Action::TagCreatePrompt,
        // Network
        KeyCode::Char('f') if key.modifiers == KeyModifiers::NONE => Action::Fetch,
        KeyCode::Char('F') => Action::Pull,
        KeyCode::Char('P') => Action::Push,
        _ => Action::None,
    }
}
