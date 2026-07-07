//! Branches mode — key resolution.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;

pub(crate) fn resolve(key: KeyEvent) -> Action {
    match key.code {
        // Movement keys (j/k/g/G/PageUp/PageDown) are handled globally in
        // `crate::keymap` before this runs.
        // Branch actions
        KeyCode::Enter | KeyCode::Char(' ') => Action::Checkout,
        KeyCode::Char('n') if key.modifiers == KeyModifiers::NONE => Action::NewBranchDialog,
        // 'R' = rename the selected (local) branch.
        KeyCode::Char('R') => Action::RenameBranchDialog,
        KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => Action::DeleteSelected,
        // 'm' = merge selected branch into HEAD.
        KeyCode::Char('m') if key.modifiers == KeyModifiers::NONE => Action::MergeSelected,
        // 'r' = rebase HEAD onto selected branch.
        KeyCode::Char('r') if key.modifiers == KeyModifiers::NONE => Action::RebaseOntoSelected,
        // 'y' = yank / copy selected branch name via OSC 52.
        KeyCode::Char('y') if key.modifiers == KeyModifiers::NONE => Action::YankSha,
        // Network
        KeyCode::Char('f') if key.modifiers == KeyModifiers::NONE => Action::Fetch,
        KeyCode::Char('F') => Action::Pull,
        KeyCode::Char('P') => Action::Push,
        KeyCode::Char('X') => Action::ForcePush,
        _ => Action::None,
    }
}
