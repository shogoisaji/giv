//! Inspect mode — key resolution. Movement keys scroll the commit detail;
//! `i`/Enter (re)open the ref prompt.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;

pub(crate) fn resolve(key: KeyEvent) -> Action {
    match key.code {
        // Movement keys (j/k/g/G/PageUp/PageDown) are handled globally in
        // `crate::keymap` before this runs.
        KeyCode::Char('i') | KeyCode::Enter => Action::OpenInspectPrompt,
        // 'y' = yank / copy the inspected commit's full SHA via OSC 52.
        KeyCode::Char('y') if key.modifiers == KeyModifiers::NONE => Action::YankSha,
        _ => Action::None,
    }
}
