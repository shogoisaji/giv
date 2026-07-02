use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;
use crate::app::{Dialog, Mode, PaletteState, SearchState};
use crate::git::ResetMode;

/// The full keymap: translates a raw `KeyEvent` into an `Action` given the
/// current application mode and dialog state.
///
/// `resolve` is the central **dispatcher**: it handles the overlay/dialog/global
/// layers here, then delegates mode-specific bindings to
/// `crate::features::<mode>::keymap`.
// Future: per-mode override maps from config will turn this into a real struct.
#[derive(Debug, Clone, Default)]
pub struct Keymap;

#[derive(Debug, Clone, Copy)]
pub struct KeymapContext<'a> {
    pub mode: Mode,
    pub dialog: &'a Dialog,
    pub palette: Option<&'a PaletteState>,
    pub search: Option<&'a SearchState>,
    pub show_help: bool,
    pub op_in_progress: bool,
}

impl Keymap {
    /// Resolve a terminal key event into an application-level `Action`.
    ///
    /// Dispatch order (highest priority first):
    /// 1. Palette overlay (when `palette` is `Some`).
    /// 2. Search bar (when `search` is `Some`).
    /// 3. Help overlay (when `show_help` is true).
    /// 4. Dialog-specific bindings (when a dialog is open).
    /// 5. Global bindings that work in every mode.
    /// 6. Mode-specific bindings (delegated to `crate::features::<mode>::keymap`).
    pub fn resolve(&self, key: KeyEvent, ctx: KeymapContext<'_>) -> Action {
        // ── 1. Command palette ───────────────────────────────────────────────
        if ctx.palette.is_some() {
            return self.resolve_palette(key);
        }

        // ── 2. Search bar ────────────────────────────────────────────────────
        if ctx.search.is_some() {
            return self.resolve_search(key);
        }

        // ── 3. Help overlay — any key closes it ──────────────────────────────
        if ctx.show_help {
            return Action::ToggleHelp;
        }

        // ── 4. Dialog mode ───────────────────────────────────────────────────
        if !matches!(ctx.dialog, Dialog::None) {
            return self.resolve_dialog(key, ctx.dialog);
        }

        // ── 5. Global bindings ───────────────────────────────────────────────
        match key.code {
            // Movement — universal list/scroll navigation, identical in every
            // mode, so it lives here instead of being copy-pasted per feature.
            KeyCode::Char('j') | KeyCode::Down => return Action::Down,
            KeyCode::Char('k') | KeyCode::Up => return Action::Up,
            KeyCode::Char('g') if key.modifiers == KeyModifiers::NONE => return Action::Top,
            KeyCode::Char('G') => return Action::Bottom,
            KeyCode::PageDown => return Action::PageDown,
            KeyCode::PageUp => return Action::PageUp,
            KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => return Action::Quit,
            KeyCode::Char('r') if key.modifiers == KeyModifiers::NONE => return Action::Refresh,
            KeyCode::Char('1') => return Action::SwitchMode(Mode::Status),
            KeyCode::Char('2') => return Action::SwitchMode(Mode::Graph),
            KeyCode::Char('3') => return Action::SwitchMode(Mode::Branches),
            KeyCode::Char('4') => return Action::SwitchMode(Mode::Worktrees),
            KeyCode::Char('5') => return Action::SwitchMode(Mode::Stashes),
            KeyCode::Char('6') => return Action::SwitchMode(Mode::Inspect),
            KeyCode::Tab => return Action::FocusNext,
            // ':' opens the command palette.
            KeyCode::Char(':') if key.modifiers == KeyModifiers::NONE => {
                return Action::OpenPalette;
            }
            // '/' opens the search bar.
            KeyCode::Char('/') if key.modifiers == KeyModifiers::NONE => {
                return Action::OpenSearch;
            }
            // '?' toggles help overlay.
            KeyCode::Char('?') => return Action::ToggleHelp,
            // 'T' (capital) cycles through themes.
            KeyCode::Char('T') => {
                return Action::CycleTheme;
            }
            // 'M' (capital) toggles mouse capture so the terminal's own
            // click-drag text selection can be used to copy on-screen text.
            KeyCode::Char('M') => {
                return Action::ToggleMouseCapture;
            }
            // 'C' (capital) = continue an in-progress sequencer op.
            KeyCode::Char('C') if key.modifiers == KeyModifiers::NONE => {
                return Action::OpContinue;
            }
            // 'S' (capital) = skip the current commit of a sequencer op.
            KeyCode::Char('S') if key.modifiers == KeyModifiers::NONE => {
                return Action::OpSkip;
            }
            // While a git operation is active, abort must win over any
            // mode-local binding using the same key (Status uses A normally).
            KeyCode::Char('A') if key.modifiers == KeyModifiers::NONE && ctx.op_in_progress => {
                return Action::OpAbort;
            }
            _ => {}
        }

        // ── 6. Mode-specific bindings ────────────────────────────────────────
        let mode_action = match ctx.mode {
            Mode::Status => crate::features::status::keymap::resolve(key),
            Mode::Graph => crate::features::graph::keymap::resolve(key),
            Mode::Branches => crate::features::branches::keymap::resolve(key),
            Mode::Worktrees => crate::features::worktrees::keymap::resolve(key),
            Mode::Stashes => crate::features::stashes::keymap::resolve(key),
            Mode::Inspect => crate::features::inspect::keymap::resolve(key),
        };

        // ── Post-mode fallback globals ────────────────────────────────────────
        // 'A' (capital) = abort when no mode-specific binding claimed it.
        if matches!(mode_action, Action::None)
            && key.code == KeyCode::Char('A')
            && key.modifiers == KeyModifiers::NONE
        {
            return Action::OpAbort;
        }

        mode_action
    }

    /// Resolve a key event when the interactive-rebase todo editor is open.
    /// Delegates to the graph feature; kept here as a thin wrapper so the
    /// `event` layer can call it through the `Keymap` it already holds.
    pub fn resolve_rebase_todo(&self, key: KeyEvent) -> Action {
        crate::features::graph::keymap::resolve_rebase_todo(key)
    }

    /// Resolve key events when the command palette is open.
    fn resolve_palette(&self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => Action::ClosePalette,
            KeyCode::Enter => Action::PaletteConfirm,
            KeyCode::Up | KeyCode::Char('k') if key.modifiers == KeyModifiers::NONE => {
                Action::PaletteUp
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers == KeyModifiers::NONE => {
                Action::PaletteDown
            }
            KeyCode::Backspace => Action::PaletteBackspace,
            KeyCode::Char(c) => Action::PaletteChar(c),
            _ => Action::None,
        }
    }

    /// Resolve key events when the search bar is open.
    fn resolve_search(&self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => Action::CloseSearch,
            KeyCode::Enter => Action::SearchNext,
            KeyCode::Backspace => Action::SearchBackspace,
            KeyCode::Char('n') if key.modifiers == KeyModifiers::NONE => Action::SearchNext,
            KeyCode::Char('N') if key.modifiers == KeyModifiers::NONE => Action::SearchPrev,
            KeyCode::Char(c) => Action::SearchChar(c),
            _ => Action::None,
        }
    }

    /// Resolve key events when an input/confirm dialog is open. Shared across
    /// modes because the dialog set is global state ([`Dialog`]).
    fn resolve_dialog(&self, key: KeyEvent, dialog: &Dialog) -> Action {
        match dialog {
            Dialog::Commit(_) => match key.code {
                KeyCode::Esc => Action::CancelDialog,
                // Enter submits (consistent with every other dialog); the update
                // layer reads the draft from app.dialog.
                KeyCode::Enter => Action::SubmitCommit(String::new()),
                KeyCode::Char(c) => Action::DialogChar(c),
                KeyCode::Backspace => Action::DialogBackspace,
                _ => Action::None,
            },
            Dialog::Amend(_) => match key.code {
                KeyCode::Esc => Action::CancelDialog,
                KeyCode::Enter => Action::SubmitAmend(String::new()),
                KeyCode::Char(c) => Action::DialogChar(c),
                KeyCode::Backspace => Action::DialogBackspace,
                _ => Action::None,
            },
            Dialog::NewBranch(_) => match key.code {
                KeyCode::Esc => Action::CancelDialog,
                KeyCode::Enter => Action::SubmitNewBranch(String::new()),
                KeyCode::Char(c) => Action::DialogChar(c),
                KeyCode::Backspace => Action::DialogBackspace,
                _ => Action::None,
            },
            Dialog::RenameBranch { .. } => match key.code {
                KeyCode::Esc => Action::CancelDialog,
                KeyCode::Enter => Action::SubmitRenameBranch(String::new()),
                KeyCode::Char(c) => Action::DialogChar(c),
                KeyCode::Backspace => Action::DialogBackspace,
                _ => Action::None,
            },
            Dialog::WorktreeAdd(_) => match key.code {
                KeyCode::Esc => Action::CancelDialog,
                KeyCode::Enter => Action::SubmitWorktreeAdd(String::new()),
                KeyCode::Char(c) => Action::DialogChar(c),
                KeyCode::Backspace => Action::DialogBackspace,
                _ => Action::None,
            },
            Dialog::StashSave(_) => match key.code {
                KeyCode::Esc => Action::CancelDialog,
                KeyCode::Enter => Action::StashSave(String::new()),
                KeyCode::Char(c) => Action::DialogChar(c),
                KeyCode::Backspace => Action::DialogBackspace,
                _ => Action::None,
            },
            Dialog::TagCreate { .. } => match key.code {
                KeyCode::Esc => Action::CancelDialog,
                // Tab switches between the name and message fields.
                KeyCode::Tab => Action::DialogFocusNext,
                // Enter submits; update layer reads the draft from app.dialog.
                KeyCode::Enter => Action::SubmitTag(String::new()),
                KeyCode::Char(c) => Action::DialogChar(c),
                KeyCode::Backspace => Action::DialogBackspace,
                _ => Action::None,
            },
            Dialog::ResetMenu { .. } => match key.code {
                // s=soft, m=mixed, h=hard, Esc=cancel.
                KeyCode::Char('s') | KeyCode::Char('S') => Action::ResetTo(ResetMode::Soft),
                KeyCode::Char('m') | KeyCode::Char('M') => Action::ResetTo(ResetMode::Mixed),
                KeyCode::Char('h') | KeyCode::Char('H') => Action::ResetTo(ResetMode::Hard),
                KeyCode::Esc => Action::CancelDialog,
                _ => Action::None,
            },
            Dialog::Confirm { .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => Action::ConfirmDelete,
                KeyCode::Char('n') | KeyCode::Esc => Action::CancelDialog,
                _ => Action::None,
            },
            Dialog::InspectRef(_) => match key.code {
                KeyCode::Esc => Action::CancelDialog,
                // Enter submits; the update layer reads the draft from app.dialog.
                KeyCode::Enter => Action::SubmitInspect(String::new()),
                KeyCode::Char(c) => Action::DialogChar(c),
                KeyCode::Backspace => Action::DialogBackspace,
                _ => Action::None,
            },
            Dialog::CompareBranches { .. } => match key.code {
                KeyCode::Esc => Action::CompareCancel,
                // Enter submits the compare with the first matching branches.
                KeyCode::Enter => Action::CompareSubmit,
                // Tab switches between base and target fields.
                KeyCode::Tab => Action::CompareToggleField,
                KeyCode::Char(c) => Action::DialogChar(c),
                KeyCode::Backspace => Action::DialogBackspace,
                _ => Action::None,
            },
            Dialog::None => Action::None,
        }
    }
}
