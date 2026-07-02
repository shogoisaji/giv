//! Stashes mode — key resolution.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;

pub(crate) fn resolve(key: KeyEvent) -> Action {
    match key.code {
        // Movement keys (j/k/g/G/PageUp/PageDown) are handled globally in
        // `crate::keymap` before this runs.
        // Stash actions
        // Enter or Space = apply stash (keep it in the list).
        KeyCode::Enter | KeyCode::Char(' ') => Action::StashApply,
        // 'p' = pop (apply + drop).
        KeyCode::Char('p') if key.modifiers == KeyModifiers::NONE => Action::StashPop,
        // 'd' = drop (with confirmation).
        KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => Action::StashDrop,
        // 's' = stash save (open prompt).
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => {
            Action::StashSavePrompt(String::new())
        }
        _ => Action::None,
    }
}
