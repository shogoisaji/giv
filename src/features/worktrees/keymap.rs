//! Worktrees mode — key resolution.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;

pub(crate) fn resolve(key: KeyEvent) -> Action {
    match key.code {
        // Movement keys (j/k/g/G/PageUp/PageDown) are handled globally in
        // `crate::keymap` before this runs.
        // Worktree actions
        KeyCode::Enter => Action::SwitchWorktree,
        KeyCode::Char('a') if key.modifiers == KeyModifiers::NONE => Action::WorktreeAddDialog,
        KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => Action::WorktreeRemove,
        KeyCode::Char('p') if key.modifiers == KeyModifiers::NONE => Action::WorktreePrune,
        // Network
        KeyCode::Char('f') if key.modifiers == KeyModifiers::NONE => Action::Fetch,
        KeyCode::Char('F') => Action::Pull,
        KeyCode::Char('P') => Action::Push,
        _ => Action::None,
    }
}
