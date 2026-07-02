/// Integration tests for giv network operations (push/fetch/pull)
/// using a local bare repository as remote — no internet required.
///
/// These tests exercise `CliBackend` directly via the `GitBackend` trait.
use std::path::{Path, PathBuf};
use std::process::Command;

use giv::git::{CliBackend, GitBackend};

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Create a temp directory with a unique suffix. Uses process ID + atomic
/// counter so paths never collide across parallel tests or repeated CI runs
/// (subsec_nanos alone can clash, leaving stale `.git` dirs that cause
/// `git init -b main` to silently keep the old default branch).
fn tempdir(suffix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "giv_net_test_{}_{}_{}",
        suffix,
        std::process::id(),
        n,
    ));
    std::fs::create_dir_all(&base).expect("create temp dir");
    base
}

/// Run a git command inside `dir`, panic with context on failure.
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

/// Init a bare repository at `path`.
fn init_bare(path: &Path) {
    let output = Command::new("git")
        .args(["init", "--bare", "-q", "-b", "main"])
        .arg(path)
        .output()
        .expect("spawn git init --bare");
    assert!(output.status.success(), "git init --bare failed");
}

/// Init a normal repository at `path` with user config, one empty commit on `main`.
/// Uses `git symbolic-ref` to force the branch name regardless of the system's
/// `init.defaultBranch` config — `git init -b main` alone can be silently
/// overridden by a global config on some CI runners.
fn init_repo(path: &Path) {
    git(path, &["init", "-q"]);
    git(path, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(path, &["commit", "--allow-empty", "-m", "initial"]);
}

/// Set up a CliBackend for a path that already has a git repo.
fn backend(path: &Path) -> CliBackend {
    // git rev-parse returns the canonical absolute path.
    let root_str = git(path, &["rev-parse", "--show-toplevel"]);
    let root = PathBuf::from(root_str);
    CliBackend::new(root)
}

/// Write a file inside `dir` and add+commit it with the given message.
fn commit_file(dir: &Path, filename: &str, content: &str, message: &str) {
    let file_path = dir.join(filename);
    std::fs::write(&file_path, content).expect("write file");
    git(dir, &["add", "--", filename]);
    git(dir, &["commit", "-m", message]);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// Test 1: push to a local bare remote and verify the ref was received.
#[test]
fn test_push_to_bare_remote() {
    let work = tempdir("push_work");
    let bare = tempdir("push_bare");

    init_bare(&bare);
    init_repo(&work);

    // Add a file so there's a real commit.
    commit_file(&work, "a.txt", "hello\n", "add a.txt");

    // Configure remote.
    git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);

    // Push via CliBackend.
    let b = backend(&work);
    b.push(Some("origin"), Some("main"), false)
        .expect("push should succeed");

    // Verify the bare repo received the ref.
    let refs = git(&bare, &["show-ref", "--heads"]);
    assert!(
        refs.contains("refs/heads/main"),
        "bare repo should have refs/heads/main after push; got: {refs}"
    );
}

/// Test 2: fetch from a remote that has a new commit, and verify the
/// local remote-tracking ref advances.
#[test]
fn test_fetch_advances_remote_tracking_ref() {
    // --- Setup ---
    let bare = tempdir("fetch_bare");
    let work1 = tempdir("fetch_work1");
    let work2 = tempdir("fetch_work2");

    init_bare(&bare);
    init_repo(&work1);
    commit_file(&work1, "base.txt", "base\n", "base commit");
    git(&work1, &["remote", "add", "origin", bare.to_str().unwrap()]);
    // Push initial state.
    git(&work1, &["push", "-u", "origin", "main"]);

    // Clone into work2 (simulates a second developer).
    git(&work2, &["clone", bare.to_str().unwrap(), "."]);
    git(&work2, &["commit", "--allow-empty", "-m", "extra commit"]);
    git(&work2, &["push", "origin", "main"]);

    // --- Act ---
    // Fetch on work1 via CliBackend — this should advance origin/main.
    let b1 = backend(&work1);
    let before_sha = git(&work1, &["rev-parse", "origin/main"]);
    b1.fetch(Some("origin")).expect("fetch should succeed");
    let after_sha = git(&work1, &["rev-parse", "origin/main"]);

    // --- Assert ---
    assert_ne!(
        before_sha, after_sha,
        "origin/main should have advanced after fetch; before={before_sha} after={after_sha}"
    );
}

/// Test 3: pull on a repo with an upstream, assert the local branch advances.
#[test]
fn test_pull_advances_local_branch() {
    // --- Setup ---
    let bare = tempdir("pull_bare");
    let work1 = tempdir("pull_work1");
    let work2 = tempdir("pull_work2");

    init_bare(&bare);
    init_repo(&work1);
    commit_file(&work1, "base.txt", "base\n", "base commit");
    git(&work1, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&work1, &["push", "-u", "origin", "main"]);

    // Clone work2 and push a new commit.
    git(&work2, &["clone", bare.to_str().unwrap(), "."]);
    git(
        &work2,
        &["commit", "--allow-empty", "-m", "upstream commit"],
    );
    git(&work2, &["push", "origin", "main"]);

    // Record local HEAD on work1 before pull.
    let before_sha = git(&work1, &["rev-parse", "HEAD"]);

    // --- Act ---
    let b1 = backend(&work1);
    b1.pull().expect("pull should succeed");

    // --- Assert ---
    let after_sha = git(&work1, &["rev-parse", "HEAD"]);
    assert_ne!(
        before_sha, after_sha,
        "local HEAD should advance after pull; before={before_sha} after={after_sha}"
    );
}

/// Test 4: branches() returns the correct set of branches including upstream tracking.
#[test]
fn test_branches_upstream_tracking() {
    let bare = tempdir("branches_bare");
    let work = tempdir("branches_work");

    init_bare(&bare);
    init_repo(&work);
    commit_file(&work, "f.txt", "data\n", "first commit");
    git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&work, &["push", "-u", "origin", "main"]);

    // Create an extra local branch with no upstream.
    git(&work, &["branch", "local-only"]);

    let b = backend(&work);
    let branches = b.branches().expect("branches() should succeed");

    // Find main branch — must be head and have upstream.
    let main = branches
        .iter()
        .find(|br| br.name == "main" && matches!(br.kind, giv::git::RefKind::LocalBranch))
        .expect("should find local main branch");
    assert!(main.is_head, "main should be HEAD");
    assert_eq!(main.upstream.as_deref(), Some("origin/main"));

    // Find local-only — no upstream.
    let local_only = branches
        .iter()
        .find(|br| br.name == "local-only")
        .expect("should find local-only branch");
    assert!(
        local_only.upstream.is_none(),
        "local-only should have no upstream"
    );

    // Find remote tracking branch.
    let remote_main = branches
        .iter()
        .find(|br| br.name == "origin/main" && matches!(br.kind, giv::git::RefKind::RemoteBranch));
    assert!(
        remote_main.is_some(),
        "origin/main remote branch should be present"
    );
}

/// Test 5: worktrees() returns correct entries including the linked worktree.
#[test]
fn test_worktrees_linked() {
    let work = tempdir("wt_work");
    let wt2 = tempdir("wt_linked");

    init_repo(&work);
    commit_file(&work, "w.txt", "w\n", "worktree commit");

    // Add a linked worktree on a new branch.
    git(
        &work,
        &["worktree", "add", wt2.to_str().unwrap(), "-b", "wt-branch"],
    );

    let b = backend(&work);
    let worktrees = b.worktrees().expect("worktrees() should succeed");

    assert_eq!(
        worktrees.len(),
        2,
        "should have 2 worktrees (main + linked)"
    );

    // Find the main worktree.
    let main_wt = worktrees.iter().find(|w| w.is_current);
    assert!(
        main_wt.is_some(),
        "one worktree should be marked is_current"
    );
    assert_eq!(
        main_wt.unwrap().branch.as_deref(),
        Some("main"),
        "main worktree branch should be main"
    );

    // Find the linked worktree.
    let linked = worktrees.iter().find(|w| !w.is_current);
    assert!(linked.is_some(), "linked worktree should be present");
    assert_eq!(
        linked.unwrap().branch.as_deref(),
        Some("wt-branch"),
        "linked worktree branch should be wt-branch"
    );
    assert!(
        !linked.unwrap().is_locked,
        "linked worktree should not be locked"
    );
}
