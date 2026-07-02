pub mod cli;
pub mod diff;
pub mod types;

pub use cli::{spawn_git, CliBackend};
pub use types::*;

use anyhow::Context;
use std::path::{Path, PathBuf};

/// Abbreviate an OID to its 7-character git short form (or the whole string if
/// it is shorter). OIDs are ASCII hex so byte slicing never splits a codepoint.
pub fn short_oid(oid: &str) -> &str {
    &oid[..7.min(oid.len())]
}

// ─── Phase 3 enums ──────────────────────────────────────────────────────────

/// Mode for `git reset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

/// Used by `op_continue` / `op_abort` / `op_skip` to select the subcommand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinueAbort {
    Continue,
    Abort,
    Skip,
}

/// The kind of multi-step git operation that may be in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// Describes a sequencer / merge operation that is currently in progress.
#[derive(Debug, Clone)]
pub struct OpInProgress {
    pub kind: OpKind,
    /// Paths that are currently unresolved (conflict markers present).
    pub conflicted: Vec<String>,
}

// ─── GitBackend trait ────────────────────────────────────────────────────────

/// Object-safe abstraction over a git repository.
/// Phase-1 methods + Phase-2 branch/worktree/network extensions.
pub trait GitBackend: Send {
    fn root(&self) -> &Path;

    fn status(&self) -> anyhow::Result<WorkingStatus>;

    /// Returns commits in topological order, newest first, with ref decoration.
    ///
    /// When `all` is true the walk covers every ref (local + remote branches,
    /// tags, HEAD) so the graph shows the full branch topology; when false it is
    /// scoped to the history reachable from the current HEAD only.
    ///
    /// When `first_parent` is true the walk follows only first parents — on a
    /// merge-heavy trunk this collapses each merge's side branch, yielding a
    /// single straight line (one row per merge). Merge commits still report both
    /// parents in their data, so they keep the merge glyph.
    fn log(&self, limit: usize, all: bool, first_parent: bool) -> anyhow::Result<Vec<Commit>>;

    /// Like [`log`](Self::log) but scoped to the union of two histories:
    /// everything reachable from `tip` and (if given) `base`. Used by the Branch
    /// lens to show "this branch + main" as a clean two-lane graph that converges
    /// at their fork point. `base == None` shows only `tip`'s history.
    fn log_range(
        &self,
        tip: &str,
        base: Option<&str>,
        limit: usize,
        first_parent: bool,
    ) -> anyhow::Result<Vec<Commit>>;

    /// Commits reachable from `target` but NOT from `base` — i.e. `git log
    /// base..target`. Used by the branch-compare mode to show only the commits
    /// that `target` has on top of `base`.
    fn log_between(
        &self,
        base: &str,
        target: &str,
        limit: usize,
        first_parent: bool,
    ) -> anyhow::Result<Vec<Commit>>;

    /// Three-dot diff: changes on `target` since its merge-base with `base` —
    /// i.e. `git diff base...target`. Used by the branch-compare mode to show
    /// the cumulative diff of the compared branch range.
    fn diff_between(&self, base: &str, target: &str) -> anyhow::Result<Diff>;

    /// Resolve an arbitrary revision (sha / short sha / branch / tag / `HEAD~1` …)
    /// to a single `Commit`. Errors if the ref cannot be resolved.
    fn commit_info(&self, rev: &str) -> anyhow::Result<Commit>;

    fn commit_diff(&self, oid: &str) -> anyhow::Result<Diff>;

    /// `staged = true`  → index vs HEAD
    /// `staged = false` → worktree vs index
    fn worktree_diff(&self, staged: bool) -> anyhow::Result<Diff>;

    fn file_diff(&self, path: &str, staged: bool) -> anyhow::Result<Diff>;

    fn stage(&self, paths: &[String]) -> anyhow::Result<()>;

    fn unstage(&self, paths: &[String]) -> anyhow::Result<()>;

    fn stage_all(&self) -> anyhow::Result<()>;

    fn unstage_all(&self) -> anyhow::Result<()>;

    /// Apply a patch string. `cached` maps to `--cached`, `reverse` to `--reverse`.
    fn apply_patch(&self, patch: &str, cached: bool, reverse: bool) -> anyhow::Result<()>;

    fn commit(&self, message: &str) -> anyhow::Result<()>;

    /// Amend the last commit: replace its message and fold in any staged changes.
    fn commit_amend(&self, message: &str) -> anyhow::Result<()>;

    /// Full message (subject + body) of HEAD, used to pre-fill the amend dialog.
    fn last_commit_message(&self) -> anyhow::Result<String>;

    // ── Phase 2: Branch / Worktree / Network ────────────────────────────────

    /// Return all local and remote branches with upstream tracking info.
    fn branches(&self) -> anyhow::Result<Vec<Branch>>;

    /// Return all tags.
    fn tags(&self) -> anyhow::Result<Vec<Tag>>;

    /// Return all remotes as (name, url) pairs (fetch URL, deduplicated).
    fn remotes(&self) -> anyhow::Result<Vec<(String, String)>>;

    /// Check out an existing branch or commit by name.
    fn checkout(&self, name: &str) -> anyhow::Result<()>;

    /// Create a branch. If `checkout` is true, switch to it immediately.
    /// If `from` is provided, branch from that ref; otherwise from HEAD.
    fn create_branch(&self, name: &str, from: Option<&str>, checkout: bool) -> anyhow::Result<()>;

    /// Delete a branch. If `force` is true, use `-D` (force-delete unmerged).
    fn delete_branch(&self, name: &str, force: bool) -> anyhow::Result<()>;

    /// Rename a branch (`git branch -m <old> <new>`).
    fn rename_branch(&self, old: &str, new: &str) -> anyhow::Result<()>;

    /// Return all worktrees (`git worktree list --porcelain`).
    fn worktrees(&self) -> anyhow::Result<Vec<Worktree>>;

    /// Add a new worktree at `path`. If `new_branch` is true, create `-b branch`.
    fn worktree_add(&self, path: &str, branch: &str, new_branch: bool) -> anyhow::Result<()>;

    /// Remove a worktree. If `force` is true, pass `--force`.
    fn worktree_remove(&self, path: &str, force: bool) -> anyhow::Result<()>;

    /// Prune stale worktree administrative files.
    fn worktree_prune(&self) -> anyhow::Result<()>;

    /// Fetch from a remote (or all remotes if `remote` is None).
    fn fetch(&self, remote: Option<&str>) -> anyhow::Result<()>;

    /// Pull (fetch + merge/rebase) on the current branch.
    fn pull(&self) -> anyhow::Result<()>;

    /// Push to remote. If `force`, pass `--force-with-lease`.
    fn push(&self, remote: Option<&str>, branch: Option<&str>, force: bool) -> anyhow::Result<()>;

    // ── Phase 3: Stash ──────────────────────────────────────────────────────

    /// List all stash entries.
    fn stashes(&self) -> anyhow::Result<Vec<Stash>>;

    /// Push a new stash. If `include_untracked`, pass `-u`.
    fn stash_save(&self, message: Option<&str>, include_untracked: bool) -> anyhow::Result<()>;

    /// Pop stash entry at `index` (applies and drops it).
    fn stash_pop(&self, index: usize) -> anyhow::Result<()>;

    /// Apply stash entry at `index` without dropping it.
    fn stash_apply(&self, index: usize) -> anyhow::Result<()>;

    /// Drop stash entry at `index`.
    fn stash_drop(&self, index: usize) -> anyhow::Result<()>;

    /// Return the diff of stash entry at `index`.
    fn stash_show(&self, index: usize) -> anyhow::Result<Diff>;

    // ── Phase 3: History operations ─────────────────────────────────────────

    /// Merge `branch` into HEAD. If `no_ff`, pass `--no-ff`.
    /// On conflict, returns `Err` whose message contains "conflict".
    fn merge(&self, branch: &str, no_ff: bool) -> anyhow::Result<()>;

    /// Rebase the current branch onto `onto`.
    /// On conflict, returns `Err` whose message contains "conflict".
    fn rebase(&self, onto: &str) -> anyhow::Result<()>;

    /// Interactive rebase: rewrite history from `base` using the given `todo` list.
    ///
    /// Each entry in `todo` is `(command, oid)` where command ∈ pick|reword|edit|squash|fixup|drop.
    /// Uses a scripted `GIT_SEQUENCE_EDITOR` so it does not open an interactive editor.
    /// `GIT_EDITOR=true` suppresses commit-message editors for reword/squash.
    fn rebase_interactive(&self, base: &str, todo: &[(String, String)]) -> anyhow::Result<()>;

    /// Cherry-pick a single commit.
    fn cherry_pick(&self, oid: &str) -> anyhow::Result<()>;

    /// Revert a commit. If `no_commit`, pass `--no-commit` (stage changes without committing).
    fn revert(&self, oid: &str, no_commit: bool) -> anyhow::Result<()>;

    /// Reset HEAD to `target` with the given mode.
    fn reset(&self, mode: ResetMode, target: &str) -> anyhow::Result<()>;

    // ── Phase 3: Tag management ─────────────────────────────────────────────

    /// Create a tag. If `target` is given, tag that ref; otherwise tag HEAD.
    /// If `message` is given, create an annotated tag (`-m`); otherwise lightweight.
    fn tag_create(
        &self,
        name: &str,
        target: Option<&str>,
        message: Option<&str>,
    ) -> anyhow::Result<()>;

    /// Delete a tag by name.
    fn tag_delete(&self, name: &str) -> anyhow::Result<()>;

    // ── Phase 3: Conflict / sequencer state ────────────────────────────────

    /// Detect whether a multi-step operation (merge, rebase, cherry-pick, revert)
    /// is currently in progress by inspecting `.git/` marker files.
    /// Returns `None` if no such operation is active.
    fn operation_in_progress(&self) -> anyhow::Result<Option<OpInProgress>>;

    /// Continue the in-progress operation (e.g. `git merge --continue`).
    /// `GIT_EDITOR=true` suppresses interactive editor prompts where applicable.
    fn op_continue(&self, kind: OpKind) -> anyhow::Result<()>;

    /// Abort the in-progress operation (e.g. `git rebase --abort`).
    fn op_abort(&self, kind: OpKind) -> anyhow::Result<()>;

    /// Skip the current commit of the in-progress operation (e.g.
    /// `git rebase --skip`). Merge has no `--skip`; callers should not invoke
    /// this for `OpKind::Merge`.
    fn op_skip(&self, kind: OpKind) -> anyhow::Result<()>;

    /// Return the list of paths with unresolved conflicts (`--diff-filter=U`).
    fn conflicted_files(&self) -> anyhow::Result<Vec<String>>;

    /// Mark a conflicted file as resolved by staging it (`git add -- <path>`).
    fn mark_resolved(&self, path: &str) -> anyhow::Result<()>;
}

// ─── Repository discovery ────────────────────────────────────────────────────

/// Discover the repository root by running `git rev-parse --show-toplevel`
/// inside `path`, then return a `CliBackend` rooted there.
pub fn open(path: &Path) -> anyhow::Result<CliBackend> {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "color.ui=never",
            "--no-pager",
            "rev-parse",
            "--show-toplevel",
        ])
        .current_dir(path)
        // Match the non-interactive env every other git call uses, so a
        // repo requiring credentials can't hang the UI during discovery.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .output()
        .context("failed to spawn `git rev-parse --show-toplevel`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("not a git repository (or any parent): {}", stderr.trim());
    }

    let root_str = String::from_utf8_lossy(&output.stdout);
    let root = PathBuf::from(root_str.trim());
    Ok(CliBackend::new(root))
}
