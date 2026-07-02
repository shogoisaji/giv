//! Incremental search — overlay state plus the match-computation and
//! cursor-jump helpers shared by the search actions in [`crate::update`].

use crate::app::{App, Mode};
use crate::git::{Branch, Commit, WorkingStatus};

/// Maximum number of search matches retained.  The search bar rebuilds the
/// match list on every keystroke; capping it prevents unbounded `Vec` growth
/// on very large repositories (10 k+ commits / branches) and keeps the
/// incremental search responsive.  The user can narrow the query to see more
/// specific results beyond the cap.
pub(crate) const SEARCH_MAX_MATCHES: usize = 1000;

/// State for the incremental search bar.
#[derive(Debug, Clone)]
pub struct SearchState {
    /// The user's current query string.
    pub query: String,
    /// Indices into the current mode's list that match the query.
    pub matches: Vec<usize>,
    /// Which match we are currently highlighting (wraps).
    pub current: usize,
}

/// Compute which indices in the current mode's list match `query`.
///
/// Results are capped at [`SEARCH_MAX_MATCHES`] to keep the per-keystroke
/// rebuild cheap on very large repositories.
pub(crate) fn compute_search_matches(
    mode: Mode,
    commits: &[Commit],
    branches: &[Branch],
    status: &WorkingStatus,
    query: &str,
) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let q = query.to_lowercase();

    match mode {
        Mode::Graph => commits
            .iter()
            .enumerate()
            .filter(|(_, c)| c.summary.to_lowercase().contains(&q) || c.id.starts_with(query))
            .map(|(i, _)| i)
            .take(SEARCH_MAX_MATCHES)
            .collect(),
        Mode::Branches => branches
            .iter()
            .enumerate()
            .filter(|(_, b)| b.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .take(SEARCH_MAX_MATCHES)
            .collect(),
        Mode::Status => {
            // The Status list is a flattened Staged-then-Unstaged view in which a
            // file with both staged and unstaged changes appears TWICE. Search
            // must return *logical* indices over that flattened view (the same
            // space resolve_entry uses), not raw status.entries indices — else
            // any dual-group file shifts the mapping and the cursor lands wrong.
            let mut matches = Vec::new();
            let mut li = 0usize;
            for e in status.entries.iter().filter(|e| e.is_staged()) {
                if e.path.to_lowercase().contains(&q) {
                    matches.push(li);
                    if matches.len() >= SEARCH_MAX_MATCHES {
                        return matches;
                    }
                }
                li += 1;
            }
            for e in status
                .entries
                .iter()
                .filter(|e| e.is_unstaged() || e.is_untracked())
            {
                if e.path.to_lowercase().contains(&q) {
                    matches.push(li);
                    if matches.len() >= SEARCH_MAX_MATCHES {
                        return matches;
                    }
                }
                li += 1;
            }
            matches
        }
        _ => Vec::new(),
    }
}

/// Move the active list cursor to the current search match index.
pub(crate) fn jump_to_match(app: &mut App) {
    if let Some(ref state) = app.search {
        if let Some(&idx) = state.matches.get(state.current) {
            match app.mode {
                Mode::Graph => {
                    app.ui.graph_index = idx;
                    // Adjust offset so the selection is visible.
                    if idx < app.ui.graph_offset {
                        app.ui.graph_offset = idx;
                    }
                }
                Mode::Branches => {
                    app.ui.branch_index = idx;
                }
                Mode::Status => {
                    app.ui.list_index = idx;
                    // Scroll up to reveal a match above the viewport. Scroll-down
                    // is handled by `clamp_list_offset` when the diff reloads
                    // (SearchNext / SearchPrev). Use the display row so group
                    // headers are accounted for.
                    let row = crate::features::status::view::selected_display_row(app);
                    if row < app.ui.list_offset {
                        app.ui.list_offset = row;
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{compute_search_matches, SEARCH_MAX_MATCHES};
    use crate::app::Mode;
    use crate::git::{Branch, Commit, RefKind, WorkingStatus};

    fn mk_commit(id: &str, summary: &str) -> Commit {
        Commit {
            id: id.into(),
            short_id: id.into(),
            summary: summary.into(),
            parents: vec![],
            body: String::new(),
            author_name: String::new(),
            author_email: String::new(),
            time: 0,
            refs: vec![],
        }
    }

    #[test]
    fn search_results_capped_at_max_matches() {
        // Create more commits than SEARCH_MAX_MATCHES, all matching "fix".
        let commits: Vec<Commit> = (0..SEARCH_MAX_MATCHES + 500)
            .map(|i| mk_commit(&format!("a{i}"), "fix something"))
            .collect();
        let branches = vec![];
        let status = WorkingStatus::default();
        let matches = compute_search_matches(Mode::Graph, &commits, &branches, &status, "fix");
        assert_eq!(matches.len(), SEARCH_MAX_MATCHES);
    }

    #[test]
    fn search_results_under_cap_returned_in_full() {
        let commits: Vec<Commit> = (0..10)
            .map(|i| mk_commit(&format!("a{i}"), "fix"))
            .collect();
        let branches = vec![];
        let status = WorkingStatus::default();
        let matches = compute_search_matches(Mode::Graph, &commits, &branches, &status, "fix");
        assert_eq!(matches.len(), 10);
    }

    #[test]
    fn search_branch_results_capped() {
        let branches: Vec<Branch> = (0..SEARCH_MAX_MATCHES + 100)
            .map(|i| Branch {
                name: format!("fix-{i}"),
                target: String::new(),
                kind: RefKind::LocalBranch,
                upstream: None,
                ahead: 0,
                behind: 0,
                is_head: false,
            })
            .collect();
        let commits = vec![];
        let status = WorkingStatus::default();
        let matches = compute_search_matches(Mode::Branches, &commits, &branches, &status, "fix");
        assert_eq!(matches.len(), SEARCH_MAX_MATCHES);
    }

    // Suppress unused-import warnings for types only used in tests above.
    #[allow(dead_code)]
    fn _silence() {
        let _ = RefKind::LocalBranch;
    }

    // ── Edge cases ───────────────────────────────────────────────────────────

    #[test]
    fn empty_query_returns_no_matches() {
        let commits = vec![mk_commit("a1", "fix bug")];
        let matches =
            compute_search_matches(Mode::Graph, &commits, &[], &WorkingStatus::default(), "");
        assert!(matches.is_empty());
    }

    #[test]
    fn whitespace_only_query_still_filters() {
        // A query of " " is non-empty, so it filters; only commits with a space
        // in the summary match.
        let commits = vec![
            mk_commit("a1", "no space here"),
            mk_commit("a2", "has space here"),
        ];
        let matches =
            compute_search_matches(Mode::Graph, &commits, &[], &WorkingStatus::default(), " ");
        // "no space here" contains a space, "has space here" too — both match.
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn graph_search_matches_by_summary_case_insensitive() {
        let commits = vec![
            mk_commit("a1", "Fix BUG"),
            mk_commit("a2", "unrelated"),
            mk_commit("a3", "another FIX"),
        ];
        let matches =
            compute_search_matches(Mode::Graph, &commits, &[], &WorkingStatus::default(), "fix");
        assert_eq!(matches, vec![0, 2]);
    }

    #[test]
    fn graph_search_matches_by_oid_prefix() {
        let commits = vec![
            mk_commit("abc123", "first"),
            mk_commit("def456", "second"),
            mk_commit("abc789", "third"),
        ];
        // OID prefix match is case-sensitive (uses `starts_with(query)`).
        let matches =
            compute_search_matches(Mode::Graph, &commits, &[], &WorkingStatus::default(), "abc");
        assert_eq!(matches, vec![0, 2]);
    }

    #[test]
    fn graph_search_oid_prefix_case_sensitive() {
        let commits = vec![mk_commit("ABC123", "first")];
        // Lowercase "abc" should NOT match "ABC123" for the OID check.
        let matches =
            compute_search_matches(Mode::Graph, &commits, &[], &WorkingStatus::default(), "abc");
        assert!(matches.is_empty());
    }

    #[test]
    fn graph_search_no_matches_returns_empty() {
        let commits = vec![mk_commit("a1", "nothing here")];
        let matches =
            compute_search_matches(Mode::Graph, &commits, &[], &WorkingStatus::default(), "zzz");
        assert!(matches.is_empty());
    }

    #[test]
    fn branches_search_case_insensitive() {
        let branches = vec![
            Branch {
                name: "Feature/X".into(),
                target: String::new(),
                kind: RefKind::LocalBranch,
                upstream: None,
                ahead: 0,
                behind: 0,
                is_head: false,
            },
            Branch {
                name: "main".into(),
                target: String::new(),
                kind: RefKind::LocalBranch,
                upstream: None,
                ahead: 0,
                behind: 0,
                is_head: false,
            },
        ];
        let matches = compute_search_matches(
            Mode::Branches,
            &[],
            &branches,
            &WorkingStatus::default(),
            "feature",
        );
        assert_eq!(matches, vec![0]);
    }

    #[test]
    fn unsupported_modes_return_empty() {
        // Worktrees, Stashes, Inspect modes are not searchable.
        let commits = vec![mk_commit("a1", "fix")];
        for mode in [Mode::Worktrees, Mode::Stashes, Mode::Inspect] {
            let matches =
                compute_search_matches(mode, &commits, &[], &WorkingStatus::default(), "fix");
            assert!(matches.is_empty(), "mode {mode:?} should have no matches");
        }
    }

    #[test]
    fn status_search_matches_staged_and_unstaged_paths() {
        use crate::git::{FileStatus, StatusCode};
        let status = WorkingStatus {
            entries: vec![
                FileStatus {
                    path: "staged_file.rs".into(),
                    orig_path: None,
                    index: StatusCode::Modified,
                    worktree: StatusCode::Unmodified,
                },
                FileStatus {
                    path: "unstaged_file.rs".into(),
                    orig_path: None,
                    index: StatusCode::Unmodified,
                    worktree: StatusCode::Modified,
                },
                FileStatus {
                    path: "untracked.txt".into(),
                    orig_path: None,
                    index: StatusCode::Untracked,
                    worktree: StatusCode::Untracked,
                },
            ],
            ..WorkingStatus::default()
        };
        // "file" matches both staged and unstaged entries.
        let matches = compute_search_matches(Mode::Status, &[], &[], &status, "file");
        // Staged group has 1 row (staged_file.rs), unstaged group has 2 rows
        // (unstaged_file.rs, untracked.txt). "file" matches the first two.
        assert_eq!(matches, vec![0, 1]);
    }

    #[test]
    fn status_search_untracked_included() {
        use crate::git::{FileStatus, StatusCode};
        let status = WorkingStatus {
            entries: vec![FileStatus {
                path: "new_untracked.txt".into(),
                orig_path: None,
                index: StatusCode::Untracked,
                worktree: StatusCode::Untracked,
            }],
            ..WorkingStatus::default()
        };
        let matches = compute_search_matches(Mode::Status, &[], &[], &status, "untracked");
        assert_eq!(matches, vec![0]);
    }

    #[test]
    fn status_search_empty_when_no_path_matches() {
        use crate::git::{FileStatus, StatusCode};
        let status = WorkingStatus {
            entries: vec![FileStatus {
                path: "foo.rs".into(),
                orig_path: None,
                index: StatusCode::Modified,
                worktree: StatusCode::Modified,
            }],
            ..WorkingStatus::default()
        };
        let matches = compute_search_matches(Mode::Status, &[], &[], &status, "bar");
        assert!(matches.is_empty());
    }

    #[test]
    fn status_search_caps_at_max_matches() {
        use crate::git::{FileStatus, StatusCode};
        // All entries staged and matching "file".
        let status = WorkingStatus {
            entries: (0..SEARCH_MAX_MATCHES + 50)
                .map(|i| FileStatus {
                    path: format!("file_{i}.rs"),
                    orig_path: None,
                    index: StatusCode::Modified,
                    worktree: StatusCode::Unmodified,
                })
                .collect(),
            ..WorkingStatus::default()
        };
        let matches = compute_search_matches(Mode::Status, &[], &[], &status, "file");
        assert_eq!(matches.len(), SEARCH_MAX_MATCHES);
    }

    #[test]
    fn graph_search_returns_indices_in_order() {
        // Matches should be returned in list order (not sorted by relevance).
        let commits = vec![
            mk_commit("a1", "first fix"),
            mk_commit("a2", "no match"),
            mk_commit("a3", "second fix"),
            mk_commit("a4", "third fix"),
        ];
        let matches =
            compute_search_matches(Mode::Graph, &commits, &[], &WorkingStatus::default(), "fix");
        assert_eq!(matches, vec![0, 2, 3]);
    }

    #[test]
    fn graph_search_summary_match_takes_precedence_over_no_oid_match() {
        // A commit whose summary matches but whose OID doesn't start with the
        // query should still be included.
        let commits = vec![mk_commit("xyz123", "fix the bug")];
        let matches =
            compute_search_matches(Mode::Graph, &commits, &[], &WorkingStatus::default(), "fix");
        assert_eq!(matches, vec![0]);
    }
}
