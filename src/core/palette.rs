//! Command palette — overlay state plus the registry of every command (label,
//! key hint, and the [`Action`] it dispatches). The palette is a discoverable,
//! searchable index of the keymap; rendered by `crate::ui::overlay`.

use crate::action::Action;
use crate::app::Mode;

/// A single item shown in the command palette.
#[derive(Debug, Clone)]
pub struct PaletteItem {
    /// Human-readable command label (e.g. "Stage All").
    pub label: String,
    /// Short key-hint string shown on the right (e.g. "a").
    pub hint: String,
    /// The action to dispatch when this item is confirmed.
    pub action: Action,
}

/// State for the command palette overlay.
#[derive(Debug, Clone)]
pub struct PaletteState {
    /// Current filter query typed by the user.
    pub query: String,
    /// The full registry, built once when the palette opens. `items` is always a
    /// filtered view of this — we never rebuild the registry on each keystroke.
    pub all_items: Vec<PaletteItem>,
    /// Items currently visible after filtering (subset of `all_items`).
    pub items: Vec<PaletteItem>,
    /// Index of the highlighted item within `items`.
    pub cursor: usize,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self::new()
    }
}

impl PaletteState {
    /// Open a fresh palette: build the registry once and show all of it.
    pub fn new() -> Self {
        let all_items = build_palette_items();
        Self {
            query: String::new(),
            items: all_items.clone(),
            all_items,
            cursor: 0,
        }
    }

    /// Recompute `items` from `all_items` against the current `query`, keeping the
    /// cursor in bounds. Called after every edit to the query.
    pub fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.items = self
            .all_items
            .iter()
            .filter(|item| item.label.to_lowercase().contains(&q))
            .cloned()
            .collect();
        if self.cursor >= self.items.len() {
            self.cursor = self.items.len().saturating_sub(1);
        }
    }
}

/// Build the full list of palette items (all commands + key hints).
pub(crate) fn build_palette_items() -> Vec<PaletteItem> {
    let mut items = Vec::new();

    macro_rules! item {
        ($label:expr, $hint:expr, $action:expr) => {
            items.push(PaletteItem {
                label: $label.to_string(),
                hint: $hint.to_string(),
                action: $action,
            });
        };
    }

    // ── Mode switching ───────────────────────────────────────────────────────
    item!(
        "Switch to Status mode",
        "1",
        Action::SwitchMode(Mode::Status)
    );
    item!("Switch to Graph mode", "2", Action::SwitchMode(Mode::Graph));
    item!(
        "Switch to Branches mode",
        "3",
        Action::SwitchMode(Mode::Branches)
    );
    item!(
        "Switch to Worktrees mode",
        "4",
        Action::SwitchMode(Mode::Worktrees)
    );
    item!(
        "Switch to Stashes mode",
        "5",
        Action::SwitchMode(Mode::Stashes)
    );
    item!(
        "Inspect a commit (enter ref)",
        "6",
        Action::SwitchMode(Mode::Inspect)
    );

    // ── Global ───────────────────────────────────────────────────────────────
    item!("Refresh", "r", Action::Refresh);
    item!("Quit", "q", Action::Quit);
    item!("Toggle help", "?", Action::ToggleHelp);
    item!("Cycle theme", "T", Action::CycleTheme);
    item!("Search / filter", "/", Action::OpenSearch);
    item!("Copy SHA / name", "y", Action::YankSha);
    item!(
        "Toggle mouse (off = select text)",
        "M",
        Action::ToggleMouseCapture
    );
    item!(
        "Graph scope: all branches / current",
        "a",
        Action::ToggleGraphScope
    );
    item!(
        "Graph: fold/expand merges (first-parent)",
        "m",
        Action::ToggleGraphFirstParent
    );
    item!(
        "Graph: Branch lens (this branch vs main)",
        "l",
        Action::ToggleGraphBranchFocus
    );

    // ── Status mode ──────────────────────────────────────────────────────────
    item!("Stage selected file", "Space", Action::StageSelected);
    item!("Unstage selected file", "u", Action::UnstageSelected);
    item!("Stage all changes", "a", Action::StageAll);
    item!("Unstage all changes", "A", Action::UnstageAll);
    item!("Open commit dialog", "c", Action::OpenCommitDialog);
    item!("Amend last commit", "e", Action::OpenAmendDialog);

    // ── Network ──────────────────────────────────────────────────────────────
    item!("Fetch (all remotes)", "f", Action::Fetch);
    item!("Pull (ff-only)", "F", Action::Pull);
    item!("Push", "P", Action::Push);
    item!("Force push (with lease)", "X", Action::ForcePush);

    // ── Branches ─────────────────────────────────────────────────────────────
    item!("Checkout branch", "Enter", Action::Checkout);
    item!("Create new branch", "n", Action::NewBranchDialog);
    item!("Rename selected branch", "R", Action::RenameBranchDialog);
    item!("Delete selected branch", "d", Action::DeleteSelected);
    item!("Merge selected branch", "m", Action::MergeSelected);
    item!("Rebase onto branch", "r", Action::RebaseOntoSelected);

    // ── Worktrees ────────────────────────────────────────────────────────────
    item!("Add worktree", "a", Action::WorktreeAddDialog);
    item!("Remove worktree", "d", Action::WorktreeRemove);
    item!("Prune worktrees", "p", Action::WorktreePrune);
    item!("Switch to worktree (cd)", "Enter", Action::SwitchWorktree);

    // ── Stash ────────────────────────────────────────────────────────────────
    item!("Stash save", "s", Action::StashSavePrompt(String::new()));
    item!("Stash pop", "p", Action::StashPop);
    item!("Stash apply", "Enter", Action::StashApply);
    item!("Stash drop", "d", Action::StashDrop);

    // ── Graph / History ──────────────────────────────────────────────────────
    item!(
        "Cherry-pick selected commit",
        "c",
        Action::CherryPickSelected
    );
    item!("Revert selected commit", "v", Action::RevertSelected);
    item!(
        "Reset to selected commit (menu)",
        "x",
        Action::OpenResetMenu
    );
    item!(
        "Rebase onto selected commit",
        "b",
        Action::RebaseOntoSelected
    );
    item!(
        "Interactive rebase from commit",
        "i",
        Action::StartInteractiveRebase
    );
    item!("Create tag on commit", "t", Action::TagCreatePrompt);
    item!("Delete tag on commit", "D", Action::TagDelete);

    // ── Sequencer ────────────────────────────────────────────────────────────
    item!("Continue in-progress operation", "C", Action::OpContinue);
    item!("Abort in-progress operation", "A", Action::OpAbort);
    item!(
        "Skip current commit (rebase/cherry-pick)",
        "S",
        Action::OpSkip
    );

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_palette_has_all_items_visible() {
        let p = PaletteState::new();
        assert_eq!(p.items.len(), p.all_items.len());
        assert_eq!(p.cursor, 0);
        assert!(p.query.is_empty());
    }

    #[test]
    fn refilter_with_empty_query_shows_all() {
        let mut p = PaletteState::new();
        let total = p.all_items.len();
        p.query.clear();
        p.refilter();
        assert_eq!(p.items.len(), total);
    }

    #[test]
    fn refilter_with_matching_query_filters_items() {
        let mut p = PaletteState::new();
        p.query = "stage".into();
        p.refilter();
        // Should include "Stage selected file", "Stage all changes",
        // "Unstage selected file", "Unstage all changes".
        assert!(!p.items.is_empty());
        assert!(p
            .items
            .iter()
            .all(|i| i.label.to_lowercase().contains("stage")));
    }

    #[test]
    fn refilter_is_case_insensitive() {
        let mut p = PaletteState::new();
        p.query = "STAGE".into();
        p.refilter();
        assert!(!p.items.is_empty());
        assert!(p
            .items
            .iter()
            .all(|i| i.label.to_lowercase().contains("stage")));
    }

    #[test]
    fn refilter_with_no_match_returns_empty() {
        let mut p = PaletteState::new();
        p.query = "zzzzzzzznomatch".into();
        p.refilter();
        assert!(p.items.is_empty());
    }

    #[test]
    fn refilter_clamps_cursor_into_bounds() {
        let mut p = PaletteState::new();
        // Set cursor to a large value, then filter to a small set.
        p.cursor = 999;
        p.query = "stage".into();
        p.refilter();
        assert!(p.cursor < p.items.len());
    }

    #[test]
    fn refilter_cursor_zero_when_no_matches() {
        let mut p = PaletteState::new();
        p.cursor = 5;
        p.query = "zzzzzzzznomatch".into();
        p.refilter();
        // saturating_sub(0) = 0
        assert_eq!(p.cursor, 0);
    }

    #[test]
    fn build_palette_items_contains_mode_switches() {
        let items = build_palette_items();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.iter().any(|l| l.contains("Status mode")));
        assert!(labels.iter().any(|l| l.contains("Graph mode")));
        assert!(labels.iter().any(|l| l.contains("Branches mode")));
        assert!(labels.iter().any(|l| l.contains("Worktrees mode")));
        assert!(labels.iter().any(|l| l.contains("Stashes mode")));
        assert!(labels.iter().any(|l| l.contains("Inspect")));
    }

    #[test]
    fn build_palette_items_contains_quit_and_help() {
        let items = build_palette_items();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"Quit"));
        assert!(labels.contains(&"Toggle help"));
        assert!(labels.contains(&"Refresh"));
    }

    #[test]
    fn build_palette_items_non_empty() {
        let items = build_palette_items();
        assert!(
            !items.is_empty(),
            "palette should have at least some commands"
        );
    }

    #[test]
    fn build_palette_items_have_non_empty_labels_and_hints() {
        let items = build_palette_items();
        for item in &items {
            assert!(!item.label.is_empty(), "label should not be empty");
            assert!(
                !item.hint.is_empty(),
                "hint for '{}' should not be empty",
                item.label
            );
        }
    }

    #[test]
    fn refilter_preserves_all_items_for_repeated_calls() {
        let mut p = PaletteState::new();
        let total = p.all_items.len();
        p.query = "stage".into();
        p.refilter();
        let filtered = p.items.len();
        assert!(filtered < total, "filter should narrow the list");

        // Clear query → all items should reappear.
        p.query.clear();
        p.refilter();
        assert_eq!(p.items.len(), total);
    }
}
