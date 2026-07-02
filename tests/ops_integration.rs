/// Integration tests for giv Phase 3 git operations.
///
/// Tests exercise `CliBackend` directly via the `GitBackend` trait against
/// real temporary fixture repositories.
use std::path::{Path, PathBuf};
use std::process::Command;

use giv::git::{CliBackend, GitBackend, OpKind, ResetMode};

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Create a unique temp directory (unique per call AND per process so parallel
/// tests never collide on the same path).
fn tempdir(suffix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "giv_ops_test_{}_{}_{}",
        suffix,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&base).expect("create temp dir");
    base
}

/// Run a git command inside `dir`, panic on failure, return trimmed stdout.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "color.ui=never", "--no-pager"])
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@t.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@t.com")
        .output()
        .unwrap_or_else(|e| panic!("spawn git {}: {}", args.join(" "), e));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "git {} failed ({}): {}",
            args.join(" "),
            output.status,
            stderr.trim()
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Initialise a repo with user config and an initial empty commit on `main`.
/// Uses `git symbolic-ref` to force the branch name regardless of the system's
/// `init.defaultBranch` config. Sets repo-local user.email/user.name so that
/// `CliBackend` operations (merge, cherry-pick, rebase, etc.) have a committer
/// identity even on CI runners without a global git config.
fn init_repo(path: &Path) {
    git(path, &["init", "-q"]);
    git(path, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(path, &["config", "user.email", "t@t.com"]);
    git(path, &["config", "user.name", "T"]);
    git(path, &["commit", "--allow-empty", "-m", "initial"]);
}

/// Write a file and commit it with the given message.
fn commit_file(dir: &Path, filename: &str, content: &str, message: &str) {
    std::fs::write(dir.join(filename), content).expect("write file");
    git(dir, &["add", "--", filename]);
    git(dir, &["commit", "-m", message]);
}

/// Build a `CliBackend` rooted at the canonical toplevel of `path`.
fn backend(path: &Path) -> CliBackend {
    let root = git(path, &["rev-parse", "--show-toplevel"]);
    CliBackend::new(PathBuf::from(root))
}

/// Return the full SHA of HEAD in `dir`.
fn head_sha(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

/// Return the number of commits reachable from HEAD.
fn commit_count(dir: &Path) -> usize {
    let out = git(dir, &["rev-list", "--count", "HEAD"]);
    out.parse().unwrap_or(0)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

// ── merge ─────────────────────────────────────────────────────────────────────

/// Two diverging branches; merge with --no-ff should produce a merge commit
/// with exactly two parents.
#[test]
fn test_merge_produces_merge_commit() {
    let dir = tempdir("merge");
    init_repo(&dir);

    commit_file(&dir, "base.txt", "base\n", "base commit");

    // Create feature branch and add a commit.
    git(&dir, &["checkout", "-b", "feature"]);
    commit_file(&dir, "feature.txt", "feature\n", "feature commit");

    // Switch back to main and add a diverging commit.
    git(&dir, &["checkout", "main"]);
    commit_file(&dir, "main_extra.txt", "extra\n", "main extra");

    let b = backend(&dir);
    b.merge("feature", true).expect("merge should succeed");

    // Verify HEAD is a merge commit with 2 parents.
    let parents_out = git(&dir, &["log", "--format=%P", "-1", "HEAD"]);
    let parents: Vec<&str> = parents_out.split_whitespace().collect();
    assert_eq!(
        parents.len(),
        2,
        "merge commit should have 2 parents; got: {:?}",
        parents
    );
}

// ── cherry-pick ───────────────────────────────────────────────────────────────

/// Cherry-pick a commit from branch B onto A — the file change must be present
/// and a new commit added.
#[test]
fn test_cherry_pick() {
    let dir = tempdir("cherry_pick");
    init_repo(&dir);

    commit_file(&dir, "base.txt", "base\n", "base commit");

    // Create branch B with a unique file.
    git(&dir, &["checkout", "-b", "branch-b"]);
    commit_file(&dir, "cherry.txt", "cherry content\n", "cherry commit");
    let cherry_oid = head_sha(&dir);

    // Switch back to main.
    git(&dir, &["checkout", "main"]);
    let before_count = commit_count(&dir);

    let b = backend(&dir);
    b.cherry_pick(&cherry_oid)
        .expect("cherry-pick should succeed");

    // File should now exist on main.
    assert!(
        dir.join("cherry.txt").exists(),
        "cherry.txt should exist on main after cherry-pick"
    );
    let content = std::fs::read_to_string(dir.join("cherry.txt")).unwrap();
    assert_eq!(content, "cherry content\n");

    // New commit should have been added.
    assert_eq!(
        commit_count(&dir),
        before_count + 1,
        "cherry-pick should add one commit"
    );
}

// ── revert ────────────────────────────────────────────────────────────────────

/// Revert a commit — the change must be undone by a new commit.
#[test]
fn test_revert() {
    let dir = tempdir("revert");
    init_repo(&dir);

    commit_file(&dir, "file.txt", "original\n", "original commit");
    commit_file(&dir, "file.txt", "modified\n", "modification commit");
    let to_revert = head_sha(&dir);
    let before_count = commit_count(&dir);

    let b = backend(&dir);
    b.revert(&to_revert, false).expect("revert should succeed");

    // The file should be back to the original content.
    let content = std::fs::read_to_string(dir.join("file.txt")).unwrap();
    assert_eq!(
        content, "original\n",
        "revert should restore original content"
    );

    // A new commit should have been added.
    assert_eq!(
        commit_count(&dir),
        before_count + 1,
        "revert should add one commit"
    );
}

// ── reset ─────────────────────────────────────────────────────────────────────

/// Hard reset to a previous commit — HEAD must equal prev_sha and worktree must
/// be clean.
#[test]
fn test_reset_hard() {
    let dir = tempdir("reset");
    init_repo(&dir);

    commit_file(&dir, "a.txt", "first\n", "first commit");
    let prev_sha = head_sha(&dir);

    commit_file(&dir, "b.txt", "second\n", "second commit");

    let b = backend(&dir);
    b.reset(ResetMode::Hard, &prev_sha)
        .expect("hard reset should succeed");

    assert_eq!(
        head_sha(&dir),
        prev_sha,
        "HEAD should be at prev_sha after hard reset"
    );

    // Worktree must be clean (no staged or unstaged changes).
    let status_out = git(&dir, &["status", "--porcelain"]);
    assert!(
        status_out.is_empty(),
        "worktree should be clean after hard reset; got: {status_out}"
    );

    // The committed file from the reset-away commit should not exist.
    assert!(
        !dir.join("b.txt").exists(),
        "b.txt should not exist after hard reset to prev commit"
    );
}

// ── stash ─────────────────────────────────────────────────────────────────────

/// Modify a tracked file, stash_save → worktree clean & stashes len=1;
/// stash_pop → change restored & stashes empty.
/// Also verify stash_show returns a non-empty diff.
#[test]
fn test_stash_save_and_pop() {
    let dir = tempdir("stash");
    init_repo(&dir);

    commit_file(&dir, "tracked.txt", "original\n", "tracked file commit");

    // Make an unstaged modification.
    std::fs::write(dir.join("tracked.txt"), "modified\n").expect("write");

    let b = backend(&dir);

    // Save the stash.
    b.stash_save(Some("my stash"), false)
        .expect("stash_save should succeed");

    // Worktree should be clean.
    let status_out = git(&dir, &["status", "--porcelain"]);
    assert!(
        status_out.is_empty(),
        "worktree should be clean after stash_save; got: {status_out}"
    );

    // stashes() should have exactly 1 entry.
    let stashes = b.stashes().expect("stashes() should succeed");
    assert_eq!(
        stashes.len(),
        1,
        "should have 1 stash; got {:?}",
        stashes.len()
    );

    // stash_show should return a non-empty diff.
    let diff = b.stash_show(0).expect("stash_show should succeed");
    assert!(
        !diff.files.is_empty(),
        "stash_show should return a non-empty diff"
    );

    // Pop the stash.
    b.stash_pop(0).expect("stash_pop should succeed");

    // The modification should be restored.
    let content = std::fs::read_to_string(dir.join("tracked.txt")).unwrap();
    assert_eq!(
        content, "modified\n",
        "stash_pop should restore the modification"
    );

    // Stash list should be empty.
    let stashes_after = b.stashes().expect("stashes() should succeed");
    assert!(
        stashes_after.is_empty(),
        "stash list should be empty after pop; got {:?}",
        stashes_after.len()
    );
}

// ── rebase ────────────────────────────────────────────────────────────────────

/// Linear rebase of feature branch onto an advanced main — feature commits
/// must now descend from the main tip.
#[test]
fn test_rebase_linear() {
    let dir = tempdir("rebase");
    init_repo(&dir);

    commit_file(&dir, "base.txt", "base\n", "base commit");

    // Create feature branch.
    git(&dir, &["checkout", "-b", "feature"]);
    commit_file(&dir, "feature.txt", "feat\n", "feature commit");

    // Advance main.
    git(&dir, &["checkout", "main"]);
    commit_file(&dir, "main2.txt", "main2\n", "main advance");
    let main_tip = head_sha(&dir);

    // Rebase feature onto main.
    git(&dir, &["checkout", "feature"]);
    let b = backend(&dir);
    b.rebase("main").expect("rebase should succeed");

    // The parent of HEAD on feature should now be the main tip.
    let parent_out = git(&dir, &["log", "--format=%P", "-1", "HEAD"]);
    let parent = parent_out.trim();
    assert_eq!(
        parent, main_tip,
        "feature HEAD's parent should be main tip after rebase"
    );
}

// ── rebase_interactive ────────────────────────────────────────────────────────

/// A branch with 3 commits; todo = [pick c1, squash c2, pick c3] →
/// commit count drops by 1 and tree content is preserved.
#[test]
fn test_rebase_interactive_squash() {
    let dir = tempdir("rebase_interactive");
    init_repo(&dir);

    // Initial commit on main.
    commit_file(&dir, "base.txt", "base\n", "base");

    // Create feature branch with 3 commits.
    git(&dir, &["checkout", "-b", "feature"]);
    commit_file(&dir, "c1.txt", "c1\n", "commit 1");
    commit_file(&dir, "c2.txt", "c2\n", "commit 2");
    commit_file(&dir, "c3.txt", "c3\n", "commit 3");

    // Collect the 3 feature commit OIDs in order oldest→newest.
    // `git log --reverse` lists oldest first.
    let log_out = git(&dir, &["log", "--format=%H", "--reverse", "main..feature"]);
    let oids: Vec<String> = log_out.lines().map(|s| s.trim().to_owned()).collect();
    assert_eq!(oids.len(), 3, "should have exactly 3 feature commits");

    let c1 = oids[0].clone();
    let c2 = oids[1].clone();
    let c3 = oids[2].clone();

    // todo: pick c1, squash c2 (into c1), pick c3
    let todo = vec![
        ("pick".to_owned(), c1),
        ("squash".to_owned(), c2),
        ("pick".to_owned(), c3),
    ];

    let b = backend(&dir);
    b.rebase_interactive("main", &todo)
        .expect("rebase_interactive should succeed");

    // Commit count on feature relative to main should now be 2 (was 3).
    let count_out = git(&dir, &["rev-list", "--count", "main..HEAD"]);
    let count: usize = count_out.trim().parse().unwrap_or(0);
    assert_eq!(count, 2, "squash should reduce 3 commits to 2; got {count}");

    // All files from the 3 original commits should be present.
    assert!(dir.join("c1.txt").exists(), "c1.txt should exist");
    assert!(dir.join("c2.txt").exists(), "c2.txt should exist");
    assert!(dir.join("c3.txt").exists(), "c3.txt should exist");
}

// ── conflict + abort/continue ─────────────────────────────────────────────────

/// Cause a merge conflict (two branches edit the same line).
/// Verify:
///   - merge returns Err or leaves repo in non-clean state
///   - operation_in_progress() == Some(Merge)
///   - conflicted_files() lists the file
///   - resolve + mark_resolved + op_continue(Merge) completes the merge
///   - operation_in_progress() == None after completion
#[test]
fn test_conflict_continue() {
    let dir = tempdir("conflict_continue");
    init_repo(&dir);

    commit_file(&dir, "conflict.txt", "shared line\n", "initial");

    // Branch A: modify the file.
    git(&dir, &["checkout", "-b", "branch-a"]);
    commit_file(&dir, "conflict.txt", "branch-a change\n", "branch-a commit");

    // Go back to main and modify the same line differently.
    git(&dir, &["checkout", "main"]);
    commit_file(&dir, "conflict.txt", "main change\n", "main commit");

    let b = backend(&dir);

    // Attempt to merge — this should fail due to conflict.
    let merge_result = b.merge("branch-a", true);
    // The merge may return Err OR succeed with conflicts in the index.
    // Either way, operation_in_progress() should detect MERGE_HEAD.
    if let Ok(()) = merge_result {
        // If merge "succeeded" without Err, we check that conflicts are present.
        let conflicted = b.conflicted_files().expect("conflicted_files should work");
        assert!(
            !conflicted.is_empty() || b.operation_in_progress().unwrap().is_some(),
            "merge should have created conflict state"
        );
    }

    // operation_in_progress should detect the merge.
    let op = b
        .operation_in_progress()
        .expect("operation_in_progress should succeed");
    assert!(
        matches!(op, Some(ref o) if o.kind == OpKind::Merge),
        "expected Merge in progress; got {:?}",
        op
    );

    // conflicted_files should list conflict.txt.
    let conflicted = b
        .conflicted_files()
        .expect("conflicted_files should succeed");
    assert!(
        conflicted.iter().any(|f| f == "conflict.txt"),
        "conflict.txt should be in conflicted files; got {:?}",
        conflicted
    );

    // Resolve the conflict by writing the file and marking it resolved.
    std::fs::write(dir.join("conflict.txt"), "resolved\n").expect("write resolved");
    b.mark_resolved("conflict.txt")
        .expect("mark_resolved should succeed");

    // Continue the merge.
    b.op_continue(OpKind::Merge)
        .expect("op_continue(Merge) should succeed");

    // No operation should be in progress anymore.
    let op_after = b
        .operation_in_progress()
        .expect("operation_in_progress should succeed after continue");
    assert!(
        op_after.is_none(),
        "no operation should be in progress after merge --continue; got {:?}",
        op_after
    );

    // The file should contain the resolved content.
    let content = std::fs::read_to_string(dir.join("conflict.txt")).unwrap();
    assert_eq!(content, "resolved\n");
}

/// Same conflict scenario but test op_abort — should return to clean state.
#[test]
fn test_conflict_abort() {
    let dir = tempdir("conflict_abort");
    init_repo(&dir);

    commit_file(&dir, "conflict.txt", "shared line\n", "initial");

    // Branch A modifies.
    git(&dir, &["checkout", "-b", "branch-a"]);
    commit_file(&dir, "conflict.txt", "branch-a change\n", "branch-a");

    // main modifies the same line.
    git(&dir, &["checkout", "main"]);
    commit_file(&dir, "conflict.txt", "main change\n", "main");

    let main_head_before = head_sha(&dir);

    let b = backend(&dir);

    // Attempt the merge (expected to conflict).
    let _ = b.merge("branch-a", true);

    // Abort the merge.
    b.op_abort(OpKind::Merge)
        .expect("op_abort(Merge) should succeed");

    // HEAD should be back to where it was before the merge.
    assert_eq!(
        head_sha(&dir),
        main_head_before,
        "HEAD should be restored after merge abort"
    );

    // No operation in progress.
    let op = b
        .operation_in_progress()
        .expect("operation_in_progress should succeed after abort");
    assert!(op.is_none(), "no op in progress after abort; got {:?}", op);

    // Worktree should be clean.
    let status_out = git(&dir, &["status", "--porcelain"]);
    assert!(
        status_out.is_empty(),
        "worktree should be clean after abort; got: {status_out}"
    );
}

// ── tags ──────────────────────────────────────────────────────────────────────

/// tag_create → tags() contains it; tag_delete removes it.
#[test]
fn test_tag_create_and_delete() {
    let dir = tempdir("tags");
    init_repo(&dir);

    commit_file(&dir, "a.txt", "a\n", "first commit");

    let b = backend(&dir);

    // Create a lightweight tag.
    b.tag_create("v1.0.0", None, None)
        .expect("tag_create should succeed");

    let tags = b.tags().expect("tags() should succeed");
    assert!(
        tags.iter().any(|t| t.name == "v1.0.0"),
        "tags() should contain v1.0.0; got: {:?}",
        tags.iter().map(|t| &t.name).collect::<Vec<_>>()
    );

    // Delete the tag.
    b.tag_delete("v1.0.0").expect("tag_delete should succeed");

    let tags_after = b.tags().expect("tags() should succeed after delete");
    assert!(
        !tags_after.iter().any(|t| t.name == "v1.0.0"),
        "v1.0.0 should not be in tags after delete; got: {:?}",
        tags_after.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
}

/// Also verify annotated tag creation works.
#[test]
fn test_annotated_tag_create() {
    let dir = tempdir("annotated_tag");
    init_repo(&dir);

    commit_file(&dir, "x.txt", "x\n", "commit");

    let b = backend(&dir);
    b.tag_create("v2.0.0", None, Some("Release 2.0.0"))
        .expect("annotated tag_create should succeed");

    let tags = b.tags().expect("tags() should succeed");
    let tag = tags.iter().find(|t| t.name == "v2.0.0");
    assert!(tag.is_some(), "annotated tag v2.0.0 should be in tags()");
}

/// Regression: a freshly-initialised repo with an unborn HEAD (no commits yet)
/// must NOT make status()/log() fail — otherwise App::refresh() shows
/// "Refresh failed" the moment giv opens on a brand-new repo.
#[test]
fn unborn_head_status_and_log_are_empty_not_error() {
    let dir = tempdir("unborn");
    git(&dir, &["init", "-b", "main", "-q"]); // no commit -> unborn HEAD
    let b = backend(&dir);

    let commits = b
        .log(512, true, false)
        .expect("log() must succeed (empty) on an unborn HEAD, not error");
    assert!(commits.is_empty(), "unborn repo should have no commits");

    let status = b.status().expect("status() must succeed on an unborn HEAD");
    assert!(
        status.entries.is_empty(),
        "unborn repo with no files should have no status entries"
    );
}

/// Regression: an untracked (new) file must show its contents as an all-added
/// diff. `git diff` ignores untracked files, so without the `--no-index`
/// fallback the diff panel was blank for new files.
#[test]
fn untracked_file_diff_shows_added_content() {
    let dir = tempdir("untracked_diff");
    init_repo(&dir);
    std::fs::write(dir.join("new.txt"), "line1\nline2\nline3\n").expect("write new file");

    let b = backend(&dir);
    let diff = b
        .file_diff("new.txt", false)
        .expect("file_diff must succeed for an untracked file");

    assert_eq!(diff.files.len(), 1, "expected one file diff");
    let f = &diff.files[0];
    assert!(
        f.new_path.ends_with("new.txt"),
        "new_path should be the file, got {}",
        f.new_path
    );
    let added = f
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| matches!(l.kind, giv::git::types::DiffLineKind::Added))
        .count();
    assert!(
        added >= 3,
        "untracked file should show all its lines as added, got {added}"
    );
}

/// Inspect mode: `commit_info` must resolve arbitrary revisions (HEAD, HEAD~1,
/// short sha, branch) and error on an invalid ref.
#[test]
fn commit_info_resolves_revisions() {
    let dir = tempdir("commit_info");
    init_repo(&dir); // creates the "initial" empty commit
    commit_file(&dir, "a.txt", "hello\n", "add a");

    let b = backend(&dir);

    let head = b.commit_info("HEAD").expect("HEAD must resolve");
    assert_eq!(head.summary, "add a");

    let parent = b.commit_info("HEAD~1").expect("HEAD~1 must resolve");
    assert_eq!(parent.summary, "initial");

    let by_short = b
        .commit_info(&head.short_id)
        .expect("short sha must resolve");
    assert_eq!(by_short.id, head.id);

    let by_branch = b.commit_info("main").expect("branch name must resolve");
    assert_eq!(by_branch.id, head.id);

    assert!(
        b.commit_info("totally-not-a-ref").is_err(),
        "an invalid ref must error"
    );
}

// ── amend / rename / op-skip (added) ───────────────────────────────────────────

/// Set a repo-local identity so backend (no `-c`) commits succeed.
fn set_identity(dir: &Path) {
    git(dir, &["config", "user.email", "t@t.com"]);
    git(dir, &["config", "user.name", "T"]);
}

#[test]
fn test_commit_amend_rewrites_and_folds_staged() {
    let dir = tempdir("amend");
    init_repo(&dir);
    set_identity(&dir);
    commit_file(&dir, "a.txt", "one\n", "first real");

    let before = head_sha(&dir);
    let count_before = commit_count(&dir);

    // Stage an additional file, then amend.
    std::fs::write(dir.join("b.txt"), "two\n").unwrap();
    git(&dir, &["add", "b.txt"]);

    let b = backend(&dir);
    b.commit_amend("amended subject\n\namended body").unwrap();

    assert_ne!(before, head_sha(&dir), "amend rewrites HEAD");
    assert_eq!(
        count_before,
        commit_count(&dir),
        "amend must not add a commit"
    );

    let msg = git(&dir, &["log", "-1", "--format=%B"]);
    assert!(msg.contains("amended subject"), "message rewritten: {msg}");

    let tree = git(&dir, &["ls-tree", "--name-only", "HEAD"]);
    assert!(
        tree.contains("b.txt"),
        "staged file folded into amend: {tree}"
    );
}

#[test]
fn test_last_commit_message_includes_body() {
    let dir = tempdir("lastmsg");
    init_repo(&dir);
    set_identity(&dir);
    // Commit with subject + body via stdin-style message.
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    git(&dir, &["add", "a.txt"]);
    git(
        &dir,
        &["commit", "-m", "subject here", "-m", "body paragraph"],
    );

    let b = backend(&dir);
    let msg = b.last_commit_message().unwrap();
    assert!(msg.starts_with("subject here"), "subject: {msg}");
    assert!(msg.contains("body paragraph"), "body: {msg}");
}

#[test]
fn test_rename_branch_changes_name() {
    let dir = tempdir("rename");
    init_repo(&dir);
    set_identity(&dir);
    git(&dir, &["branch", "oldname"]);

    let b = backend(&dir);
    b.rename_branch("oldname", "newname").unwrap();

    let branches = git(&dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        branches.lines().any(|l| l.trim() == "newname"),
        "renamed: {branches}"
    );
    assert!(
        !branches.lines().any(|l| l.trim() == "oldname"),
        "old gone: {branches}"
    );
}

#[test]
fn test_op_skip_merge_is_rejected() {
    let dir = tempdir("skip_merge");
    init_repo(&dir);
    set_identity(&dir);
    let b = backend(&dir);
    // Merge has no --skip; the backend must reject it explicitly.
    assert!(b.op_skip(OpKind::Merge).is_err(), "merge skip must error");
}

#[test]
fn test_op_skip_rebase_drops_conflicting_commit() {
    let dir = tempdir("skip_rebase");
    init_repo(&dir);
    set_identity(&dir);

    commit_file(&dir, "c.txt", "base\n", "base");

    // feature changes the line one way…
    git(&dir, &["checkout", "-b", "feature"]);
    commit_file(&dir, "c.txt", "feature\n", "feature change");

    // …main changes the same line another way.
    git(&dir, &["checkout", "main"]);
    commit_file(&dir, "c.txt", "mainline\n", "main change");

    // Rebase feature onto main → conflict on the single feature commit.
    git(&dir, &["checkout", "feature"]);
    let b = backend(&dir);
    let _ = b.rebase("main"); // expected to conflict

    let op = b.operation_in_progress().unwrap();
    assert!(
        matches!(op, Some(ref o) if o.kind == OpKind::Rebase),
        "expected rebase in progress; got {op:?}"
    );

    // Skip the only (conflicting) commit → rebase completes with nothing applied.
    b.op_skip(OpKind::Rebase)
        .expect("rebase --skip should succeed");

    let op_after = b.operation_in_progress().unwrap();
    assert!(op_after.is_none(), "no op after skip; got {op_after:?}");

    // feature now matches main's content (its commit was dropped).
    let content = std::fs::read_to_string(dir.join("c.txt")).unwrap();
    assert_eq!(content, "mainline\n", "skipped commit must be dropped");
}

// ── stash preview must include untracked files (we stash with -u) ───────────────

#[test]
fn test_stash_show_includes_untracked() {
    let dir = tempdir("stash_untracked");
    init_repo(&dir);
    set_identity(&dir);
    commit_file(&dir, "tracked.txt", "base\n", "base");

    // A tracked modification + a brand-new untracked file.
    std::fs::write(dir.join("tracked.txt"), "changed\n").unwrap();
    std::fs::write(dir.join("untracked.txt"), "brand new\n").unwrap();

    let b = backend(&dir);
    b.stash_save(Some("test stash"), true).unwrap(); // include_untracked = true (-u)

    let stashes = b.stashes().unwrap();
    assert_eq!(stashes.len(), 1, "one stash expected");

    let diff = b.stash_show(stashes[0].index).unwrap();
    let paths: Vec<&str> = diff.files.iter().map(|f| f.new_path.as_str()).collect();
    assert!(
        paths.iter().any(|p| p.contains("tracked.txt")),
        "tracked change must be in stash preview: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.contains("untracked.txt")),
        "UNTRACKED file must be in the stash preview (was the bug): {paths:?}"
    );
}

// ── audit fixes: backend-level ──────────────────────────────────────────────────

/// Checking out a remote branch by its SHORT name must create a local tracking
/// branch (DWIM), not a detached HEAD. (giv strips the "origin/" prefix.)
#[test]
fn test_checkout_remote_short_name_is_not_detached() {
    let dir = tempdir("checkout_remote");
    init_repo(&dir);
    set_identity(&dir);
    commit_file(&dir, "a.txt", "x\n", "work");
    let head = head_sha(&dir);
    git(&dir, &["remote", "add", "origin", "/tmp/giv_fake_remote"]);
    git(&dir, &["update-ref", "refs/remotes/origin/feature", &head]);

    let b = backend(&dir);
    // giv passes the short name "feature" for a RemoteBranch entry.
    b.checkout("feature")
        .expect("checkout short name should succeed");

    let symref = git(&dir, &["symbolic-ref", "HEAD"]);
    assert_eq!(
        symref, "refs/heads/feature",
        "checkout of a remote branch must land on a local tracking branch, not detached HEAD"
    );
}

/// Adding a worktree for an EXISTING branch must succeed (new_branch=false →
/// `git worktree add <path> <branch>`), not fail with "already exists".
#[test]
fn test_worktree_add_existing_branch_succeeds() {
    let dir = tempdir("wt_existing");
    init_repo(&dir);
    set_identity(&dir);
    git(&dir, &["branch", "feat"]);

    let b = backend(&dir);
    let wt = dir.join("wt_feat");
    let wt_str = wt.to_string_lossy().to_string();
    b.worktree_add(&wt_str, "feat", false)
        .expect("adding a worktree for an existing branch must succeed");

    let wts = b.worktrees().unwrap();
    assert!(
        wts.iter().any(|w| w.branch.as_deref() == Some("feat")),
        "the new worktree should be on branch 'feat'"
    );
}

/// A non-conflict failure (bad ref) of merge/rebase/cherry-pick/revert must NOT
/// contain the word "conflict" in its error message.
#[test]
fn test_non_conflict_failures_dont_say_conflict() {
    let dir = tempdir("noconflict_msg");
    init_repo(&dir);
    set_identity(&dir);
    commit_file(&dir, "a.txt", "x\n", "base");
    let b = backend(&dir);

    let errs = vec![
        b.rebase("deadbeef").err(),
        b.cherry_pick("deadbeef").err(),
        b.revert("deadbeef", false).err(),
        b.merge("deadbeef", false).err(),
    ];
    for e in errs {
        let msg = format!("{:#}", e.expect("bad ref must error")).to_lowercase();
        assert!(
            !msg.contains("conflict"),
            "non-conflict failure must not mention 'conflict': {msg}"
        );
    }
}

// ── log shows ALL branches, not just HEAD's history ─────────────────────────────

/// The graph log must include commits that live only on OTHER branches (the
/// `--all` walk). Regression: previously `log()` walked only HEAD, so commits on
/// an unmerged feature branch were silently missing from the graph even though
/// other git-graph tools showed them.
#[test]
fn test_log_includes_commits_from_other_branches() {
    let dir = tempdir("log_all_branches");
    init_repo(&dir);
    set_identity(&dir);
    commit_file(&dir, "base.txt", "base\n", "base commit");

    // Feature branch with two commits that are NEVER merged into main.
    git(&dir, &["checkout", "-b", "feature"]);
    commit_file(&dir, "f1.txt", "f1\n", "feat-only-A");
    commit_file(&dir, "f2.txt", "f2\n", "feat-only-B");

    // Back on main, advance past the feature's fork point.
    git(&dir, &["checkout", "main"]);
    commit_file(&dir, "m.txt", "m\n", "main-advances");

    let b = backend(&dir);
    let summaries: Vec<String> = b
        .log(512, true, false)
        .expect("log should succeed")
        .into_iter()
        .map(|c| c.summary)
        .collect();

    // Commits reachable only from `feature` (HEAD is `main`) must be present.
    assert!(
        summaries.iter().any(|s| s == "feat-only-A"),
        "feature-only commit A must appear in the graph log: {summaries:?}"
    );
    assert!(
        summaries.iter().any(|s| s == "feat-only-B"),
        "feature-only commit B must appear in the graph log: {summaries:?}"
    );
    // And HEAD's own commit too, of course.
    assert!(
        summaries.iter().any(|s| s == "main-advances"),
        "HEAD commit must still appear: {summaries:?}"
    );
}

// ── log_range: union of a branch and its base (Branch lens) ─────────────────────

/// `log_range(tip, Some(base))` returns the union of both histories and nothing
/// else — the data behind the Branch lens (selected branch vs main).
#[test]
fn test_log_range_is_union_of_branch_and_base() {
    let dir = tempdir("log_range");
    init_repo(&dir); // initial commit on `main`
    set_identity(&dir);
    commit_file(&dir, "base.txt", "b\n", "base");

    // feature branches here and adds a commit.
    git(&dir, &["checkout", "-b", "feature"]);
    commit_file(&dir, "f.txt", "f\n", "feat-1");

    // main advances past the fork.
    git(&dir, &["checkout", "main"]);
    commit_file(&dir, "m.txt", "m\n", "main-1");

    // an UNRELATED branch that must NOT appear in the lens.
    git(&dir, &["checkout", "-b", "other"]);
    commit_file(&dir, "o.txt", "o\n", "other-1");
    git(&dir, &["checkout", "main"]);

    let b = backend(&dir);
    let tip = git(&dir, &["rev-parse", "feature"]);
    let summaries: Vec<String> = b
        .log_range(&tip, Some("main"), 512, false)
        .expect("log_range")
        .into_iter()
        .map(|c| c.summary)
        .collect();

    assert!(
        summaries.iter().any(|s| s == "feat-1"),
        "branch commit: {summaries:?}"
    );
    assert!(
        summaries.iter().any(|s| s == "main-1"),
        "base advanced commit: {summaries:?}"
    );
    assert!(
        summaries.iter().any(|s| s == "base"),
        "shared base: {summaries:?}"
    );
    assert!(
        !summaries.iter().any(|s| s == "other-1"),
        "unrelated branch must be excluded: {summaries:?}"
    );
}
