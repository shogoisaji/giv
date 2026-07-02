//! A mock `GitBackend` for unit tests — returns canned data without touching
//! the filesystem or spawning git processes.  Used by tests in `core/app.rs`,
//! `core/update.rs`, and the feature update modules to exercise the pure
//! state-transition logic without a real repository.

use std::path::Path;

use crate::git::{
    Branch, Commit, Diff, GitBackend, OpInProgress, OpKind, RefKind, RefName, ResetMode, Stash,
    Tag, WorkingStatus, Worktree,
};

/// A mock backend whose data can be set up field-by-field in tests.
#[derive(Debug, Default)]
pub struct MockBackend {
    pub root: std::path::PathBuf,
    pub status: WorkingStatus,
    pub commits: Vec<Commit>,
    pub branches: Vec<Branch>,
    pub tags: Vec<Tag>,
    pub worktrees: Vec<Worktree>,
    pub stashes: Vec<Stash>,
    pub op_in_progress: Option<OpInProgress>,
    pub last_commit_message: String,
    /// Recorded calls for assertion in tests.
    pub calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            root: std::path::PathBuf::from("/tmp/mock"),
            ..Default::default()
        }
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl GitBackend for MockBackend {
    fn root(&self) -> &Path {
        &self.root
    }

    fn status(&self) -> anyhow::Result<WorkingStatus> {
        Ok(self.status.clone())
    }

    fn log(&self, _limit: usize, _all: bool, _first_parent: bool) -> anyhow::Result<Vec<Commit>> {
        Ok(self.commits.clone())
    }

    fn log_range(
        &self,
        _tip: &str,
        _base: Option<&str>,
        _limit: usize,
        _first_parent: bool,
    ) -> anyhow::Result<Vec<Commit>> {
        Ok(self.commits.clone())
    }

    fn log_between(
        &self,
        _base: &str,
        _target: &str,
        _limit: usize,
        _first_parent: bool,
    ) -> anyhow::Result<Vec<Commit>> {
        Ok(self.commits.clone())
    }

    fn diff_between(&self, base: &str, target: &str) -> anyhow::Result<Diff> {
        self.record(format!("diff_between {base} {target}"));
        Ok(Diff::default())
    }

    fn commit_info(&self, rev: &str) -> anyhow::Result<Commit> {
        self.commits
            .iter()
            .find(|c| c.id == rev)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("mock: commit {rev} not found"))
    }

    fn commit_diff(&self, oid: &str) -> anyhow::Result<Diff> {
        self.record(format!("commit_diff {oid}"));
        Ok(Diff::default())
    }

    fn worktree_diff(&self, _staged: bool) -> anyhow::Result<Diff> {
        Ok(Diff::default())
    }

    fn file_diff(&self, _path: &str, _staged: bool) -> anyhow::Result<Diff> {
        Ok(Diff::default())
    }

    fn stage(&self, paths: &[String]) -> anyhow::Result<()> {
        self.record(format!("stage {paths:?}"));
        Ok(())
    }

    fn unstage(&self, paths: &[String]) -> anyhow::Result<()> {
        self.record(format!("unstage {paths:?}"));
        Ok(())
    }

    fn stage_all(&self) -> anyhow::Result<()> {
        self.record("stage_all".into());
        Ok(())
    }

    fn unstage_all(&self) -> anyhow::Result<()> {
        self.record("unstage_all".into());
        Ok(())
    }

    fn apply_patch(&self, _patch: &str, _cached: bool, _reverse: bool) -> anyhow::Result<()> {
        Ok(())
    }

    fn commit(&self, message: &str) -> anyhow::Result<()> {
        self.record(format!("commit {message}"));
        Ok(())
    }

    fn commit_amend(&self, message: &str) -> anyhow::Result<()> {
        self.record(format!("commit_amend {message}"));
        Ok(())
    }

    fn last_commit_message(&self) -> anyhow::Result<String> {
        Ok(self.last_commit_message.clone())
    }

    fn branches(&self) -> anyhow::Result<Vec<Branch>> {
        Ok(self.branches.clone())
    }

    fn tags(&self) -> anyhow::Result<Vec<Tag>> {
        Ok(self.tags.clone())
    }

    fn remotes(&self) -> anyhow::Result<Vec<(String, String)>> {
        Ok(vec![])
    }

    fn checkout(&self, name: &str) -> anyhow::Result<()> {
        self.record(format!("checkout {name}"));
        Ok(())
    }

    fn create_branch(&self, name: &str, _from: Option<&str>, checkout: bool) -> anyhow::Result<()> {
        self.record(format!("create_branch {name} checkout={checkout}"));
        Ok(())
    }

    fn delete_branch(&self, name: &str, force: bool) -> anyhow::Result<()> {
        self.record(format!("delete_branch {name} force={force}"));
        Ok(())
    }

    fn rename_branch(&self, old: &str, new: &str) -> anyhow::Result<()> {
        self.record(format!("rename_branch {old} {new}"));
        Ok(())
    }

    fn worktrees(&self) -> anyhow::Result<Vec<Worktree>> {
        Ok(self.worktrees.clone())
    }

    fn worktree_add(&self, path: &str, branch: &str, new_branch: bool) -> anyhow::Result<()> {
        self.record(format!("worktree_add {path} {branch} new={new_branch}"));
        Ok(())
    }

    fn worktree_remove(&self, path: &str, force: bool) -> anyhow::Result<()> {
        self.record(format!("worktree_remove {path} force={force}"));
        Ok(())
    }

    fn worktree_prune(&self) -> anyhow::Result<()> {
        self.record("worktree_prune".into());
        Ok(())
    }

    fn fetch(&self, remote: Option<&str>) -> anyhow::Result<()> {
        self.record(format!("fetch {:?}", remote));
        Ok(())
    }

    fn pull(&self) -> anyhow::Result<()> {
        self.record("pull".into());
        Ok(())
    }

    fn push(&self, remote: Option<&str>, branch: Option<&str>, force: bool) -> anyhow::Result<()> {
        self.record(format!("push {:?} {:?} force={force}", remote, branch));
        Ok(())
    }

    fn stashes(&self) -> anyhow::Result<Vec<Stash>> {
        Ok(self.stashes.clone())
    }

    fn stash_save(&self, message: Option<&str>, include_untracked: bool) -> anyhow::Result<()> {
        self.record(format!(
            "stash_save {:?} untracked={include_untracked}",
            message
        ));
        Ok(())
    }

    fn stash_pop(&self, index: usize) -> anyhow::Result<()> {
        self.record(format!("stash_pop {index}"));
        Ok(())
    }

    fn stash_apply(&self, index: usize) -> anyhow::Result<()> {
        self.record(format!("stash_apply {index}"));
        Ok(())
    }

    fn stash_drop(&self, index: usize) -> anyhow::Result<()> {
        self.record(format!("stash_drop {index}"));
        Ok(())
    }

    fn stash_show(&self, _index: usize) -> anyhow::Result<Diff> {
        Ok(Diff::default())
    }

    fn merge(&self, branch: &str, no_ff: bool) -> anyhow::Result<()> {
        self.record(format!("merge {branch} no_ff={no_ff}"));
        Ok(())
    }

    fn rebase(&self, onto: &str) -> anyhow::Result<()> {
        self.record(format!("rebase {onto}"));
        Ok(())
    }

    fn rebase_interactive(&self, base: &str, todo: &[(String, String)]) -> anyhow::Result<()> {
        self.record(format!("rebase_interactive {base} {} entries", todo.len()));
        Ok(())
    }

    fn cherry_pick(&self, oid: &str) -> anyhow::Result<()> {
        self.record(format!("cherry_pick {oid}"));
        Ok(())
    }

    fn revert(&self, oid: &str, no_commit: bool) -> anyhow::Result<()> {
        self.record(format!("revert {oid} no_commit={no_commit}"));
        Ok(())
    }

    fn reset(&self, mode: ResetMode, target: &str) -> anyhow::Result<()> {
        self.record(format!("reset {mode:?} {target}"));
        Ok(())
    }

    fn tag_create(
        &self,
        name: &str,
        _target: Option<&str>,
        _message: Option<&str>,
    ) -> anyhow::Result<()> {
        self.record(format!("tag_create {name}"));
        Ok(())
    }

    fn tag_delete(&self, name: &str) -> anyhow::Result<()> {
        self.record(format!("tag_delete {name}"));
        Ok(())
    }

    fn operation_in_progress(&self) -> anyhow::Result<Option<OpInProgress>> {
        Ok(self.op_in_progress.clone())
    }

    fn op_continue(&self, kind: OpKind) -> anyhow::Result<()> {
        self.record(format!("op_continue {kind:?}"));
        Ok(())
    }

    fn op_abort(&self, kind: OpKind) -> anyhow::Result<()> {
        self.record(format!("op_abort {kind:?}"));
        Ok(())
    }

    fn op_skip(&self, kind: OpKind) -> anyhow::Result<()> {
        self.record(format!("op_skip {kind:?}"));
        Ok(())
    }

    fn conflicted_files(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }

    fn mark_resolved(&self, path: &str) -> anyhow::Result<()> {
        self.record(format!("mark_resolved {path}"));
        Ok(())
    }
}

// ── Test helpers for building canned data ────────────────────────────────────

/// Build a `Commit` with the given id, summary, and HEAD ref decoration.
pub fn mk_commit(id: &str, summary: &str, is_head: bool) -> Commit {
    let mut refs = vec![];
    if is_head {
        refs.push(RefName {
            name: "HEAD".into(),
            kind: RefKind::Head,
        });
    }
    Commit {
        id: id.into(),
        short_id: id.into(),
        summary: summary.into(),
        parents: vec![],
        body: String::new(),
        author_name: "T".into(),
        author_email: "t@e".into(),
        time: 0,
        refs,
    }
}

/// Build a local branch.
pub fn mk_branch(name: &str, target: &str) -> Branch {
    Branch {
        name: name.into(),
        kind: RefKind::LocalBranch,
        upstream: None,
        ahead: 0,
        behind: 0,
        is_head: false,
        target: target.into(),
    }
}

/// Build a stash with the given index and message.
pub fn mk_stash(index: usize, message: &str) -> Stash {
    Stash {
        index,
        message: message.into(),
        oid: "deadbeef".into(),
    }
}
