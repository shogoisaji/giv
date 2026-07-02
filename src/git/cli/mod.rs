use anyhow::Context;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{
    Branch, Commit, Diff, GitBackend, OpInProgress, OpKind, RefKind, ResetMode, Stash, Tag,
    WorkingStatus, Worktree,
};
use crate::git::diff::parse_unified_diff;

mod parse;
use parse::{
    op_subcommand, parse_log_output, parse_porcelain_v2, parse_stash_list, parse_upstream_track,
    parse_worktree_porcelain, shell_escape, tempfile_path,
};

// ─── CliBackend ──────────────────────────────────────────────────────────────

pub struct CliBackend {
    root: PathBuf,
}

impl CliBackend {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    /// Run `git -c color.ui=never --no-pager <args>` in the repo root.
    /// Returns stdout as a UTF-8 string (lossy), or an error containing stderr.
    fn git(&self, args: &[&str]) -> anyhow::Result<String> {
        let output = self.git_raw(args)?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    }

    /// Run git tolerating a non-zero exit status (e.g. `diff --no-index` exits 1
    /// when the two inputs differ). Returns stdout regardless; only errors if git
    /// cannot be spawned at all.
    fn git_lenient(&self, args: &[&str]) -> anyhow::Result<String> {
        let mut cmd = Command::new("git");
        cmd.args([
            "-c",
            "color.ui=never",
            "-c",
            "core.quotepath=false",
            "--no-pager",
        ])
        .args(args)
        .current_dir(&self.root);
        Self::apply_noninteractive_env(&mut cmd);
        let output = cmd
            .output()
            .with_context(|| format!("failed to spawn `git {}`", args.join(" ")))?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Whether `path` is tracked by git. Untracked files are invisible to
    /// `git diff`, which is why a new file would otherwise show an empty diff.
    fn is_tracked(&self, path: &str) -> bool {
        self.git(&["ls-files", "--error-unmatch", "--", path])
            .is_ok()
    }

    /// The configured remote names (one per line from `git remote`). Used to
    /// classify ref decorations — a slashed ref is a remote-tracking branch only
    /// when its prefix is one of these (so `feature/api` stays a local branch).
    /// Best-effort: an error (e.g. not a repo) yields an empty list.
    fn remote_names(&self) -> Vec<String> {
        self.git(&["remote"])
            .map(|out| {
                out.lines()
                    .map(|l| l.trim().to_owned())
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Environment defaults that guarantee a git subprocess never blocks the
    /// (synchronous) UI thread waiting on an interactive editor or a terminal
    /// credential prompt. Without this, `git merge` / `git commit` etc. try to
    /// launch $EDITOR on the raw-mode terminal and hang forever — the cause of
    /// the "all keys dead, can't even quit" freeze. Per-call `envs` are applied
    /// afterwards and may override these (e.g. interactive rebase).
    fn apply_noninteractive_env(cmd: &mut Command) {
        cmd.env("GIT_EDITOR", "true")
            .env("GIT_SEQUENCE_EDITOR", "true")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat");
    }

    /// Like `git()` but returns raw bytes (useful for `-z` / binary output).
    fn git_bytes(&self, args: &[&str]) -> anyhow::Result<Vec<u8>> {
        self.git_raw(args)
    }

    fn git_raw(&self, args: &[&str]) -> anyhow::Result<Vec<u8>> {
        self.git_raw_env(args, &[])
    }

    /// Like `git_raw` but also sets additional environment variables.
    fn git_raw_env(&self, args: &[&str], envs: &[(&str, &str)]) -> anyhow::Result<Vec<u8>> {
        let mut cmd = Command::new("git");
        cmd.args([
            "-c",
            "color.ui=never",
            "-c",
            "core.quotepath=false",
            "--no-pager",
        ])
        .args(args)
        .current_dir(&self.root);
        Self::apply_noninteractive_env(&mut cmd);
        for (k, v) in envs {
            cmd.env(k, v);
        }

        let output = cmd
            .output()
            .with_context(|| format!("failed to spawn `git {}`", args.join(" ")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "`git {}` failed (exit {}): {}",
                args.join(" "),
                output.status,
                stderr.trim()
            );
        }
        Ok(output.stdout)
    }

    /// Like `git()` but also sets additional environment variables.
    fn git_env(&self, args: &[&str], envs: &[(&str, &str)]) -> anyhow::Result<String> {
        let output = self.git_raw_env(args, envs)?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    }

    /// Like `git()` but feeds `stdin_data` to the process's stdin.
    fn git_with_stdin(&self, args: &[&str], stdin_data: &str) -> anyhow::Result<String> {
        use std::io::Write;
        use std::process::Stdio;

        let mut cmd = Command::new("git");
        cmd.args([
            "-c",
            "color.ui=never",
            "-c",
            "core.quotepath=false",
            "--no-pager",
        ])
        .args(args)
        .current_dir(&self.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        Self::apply_noninteractive_env(&mut cmd);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn `git {}`", args.join(" ")))?;

        if let Some(stdin) = child.stdin.take() {
            let mut stdin = stdin;
            stdin
                .write_all(stdin_data.as_bytes())
                .context("failed to write patch to git stdin")?;
        }

        let output = child
            .wait_with_output()
            .context("failed to wait for git process")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "`git {}` failed (exit {}): {}",
                args.join(" "),
                output.status,
                stderr.trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

// ─── GitBackend impl ─────────────────────────────────────────────────────────

impl GitBackend for CliBackend {
    fn root(&self) -> &Path {
        &self.root
    }

    fn status(&self) -> anyhow::Result<WorkingStatus> {
        // `git status --porcelain=v2 --branch -z` outputs NUL-terminated records.
        // Header lines start with `# ` and describe branch state.
        // Entry lines start with `1` (ordinary), `2` (rename/copy), `u` (unmerged), or `?` (untracked).
        //
        // `--untracked-files=all` lists every untracked FILE individually instead
        // of collapsing a wholly-untracked directory into a single `dir/` entry
        // (git's default `normal` mode). Without it a brand-new folder shows up as
        // one row — the user can't see or stage the files inside it. Ignored files
        // (`.gitignore`, e.g. `target/`) are still excluded, so this doesn't recurse
        // into build output.
        let raw = self
            .git_bytes(&[
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=all",
                "-z",
            ])
            .context("git status failed")?;

        parse_porcelain_v2(&raw)
    }

    fn log(&self, limit: usize, all: bool, first_parent: bool) -> anyhow::Result<Vec<Commit>> {
        // A freshly-initialised repo (unborn HEAD) has no commits; `git log` would
        // exit 128 ("does not have any commits yet"). Treat it as an empty history
        // rather than a fatal error so `giv` opens cleanly on brand-new repos.
        if self
            .git(&["rev-parse", "--verify", "--quiet", "HEAD"])
            .is_err()
        {
            return Ok(Vec::new());
        }
        // Use unit separator (0x1f) as field delimiter and record separator (0x1e) to
        // delimit commits. This avoids ambiguity with newlines in commit bodies.
        // %D gives ref decoration names.
        let limit_str = limit.to_string();
        let format = "%H\x1f%h\x1f%P\x1f%an\x1f%ae\x1f%at\x1f%s\x1f%b\x1f%D\x1e";
        // `--all` walks every ref (local + remote branches, tags, HEAD), not just
        // commits reachable from the current HEAD — so the graph shows the full
        // branch topology: where each branch forks, where it merges, and whether
        // an integration branch has advanced past a feature branch's base. Without
        // it, commits living only on other branches are silently missing. When
        // `all` is false the walk is scoped to HEAD's history (the `a` toggle).
        let mut args: Vec<&str> = vec!["log", "--no-color"];
        if all {
            args.push("--all");
        }
        if first_parent {
            args.push("--first-parent");
        }
        args.extend_from_slice(&["-n", &limit_str, "--topo-order"]);
        let pretty = format!("--pretty=format:{}", format);
        args.push(&pretty);
        let output = self.git(&args).context("git log failed")?;

        parse_log_output(&output, &self.remote_names())
    }

    fn log_range(
        &self,
        tip: &str,
        base: Option<&str>,
        limit: usize,
        first_parent: bool,
    ) -> anyhow::Result<Vec<Commit>> {
        let limit_str = limit.to_string();
        let format = "%H\x1f%h\x1f%P\x1f%an\x1f%ae\x1f%at\x1f%s\x1f%b\x1f%D\x1e";
        // `git log <tip> [<base>]` lists commits reachable from EITHER ref — the
        // union of both histories — so the graph shows the branch and its base
        // converging at their fork point, and nothing else.
        let mut args: Vec<&str> = vec!["log", "--no-color"];
        if first_parent {
            args.push("--first-parent");
        }
        args.extend_from_slice(&["-n", &limit_str, "--topo-order"]);
        let pretty = format!("--pretty=format:{}", format);
        args.push(&pretty);
        args.push(tip);
        if let Some(b) = base {
            if b != tip {
                args.push(b);
            }
        }
        let output = self.git(&args).context("git log (range) failed")?;
        parse_log_output(&output, &self.remote_names())
    }

    fn log_between(
        &self,
        base: &str,
        target: &str,
        limit: usize,
        first_parent: bool,
    ) -> anyhow::Result<Vec<Commit>> {
        let limit_str = limit.to_string();
        let format = "%H\x1f%h\x1f%P\x1f%an\x1f%ae\x1f%at\x1f%s\x1f%b\x1f%D\x1e";
        // `git log base..target` lists commits reachable from target but not
        // from base — exactly the commits target has on top of base.
        let mut args: Vec<&str> = vec!["log", "--no-color"];
        if first_parent {
            args.push("--first-parent");
        }
        args.extend_from_slice(&["-n", &limit_str, "--topo-order"]);
        let pretty = format!("--pretty=format:{}", format);
        let range = format!("{base}..{target}");
        args.push(&pretty);
        args.push(&range);
        let output = self.git(&args).context("git log (between) failed")?;
        parse_log_output(&output, &self.remote_names())
    }

    fn diff_between(&self, base: &str, target: &str) -> anyhow::Result<Diff> {
        // `git diff base...target` = changes on target since the merge-base.
        let range = format!("{base}...{target}");
        let text = self
            .git(&["diff", "--no-color", &range])
            .context("git diff (between) failed")?;
        Ok(parse_unified_diff(&text))
    }

    fn commit_info(&self, rev: &str) -> anyhow::Result<Commit> {
        let format = "%H\x1f%h\x1f%P\x1f%an\x1f%ae\x1f%at\x1f%s\x1f%b\x1f%D\x1e";
        let output = self
            .git(&[
                "log",
                "-1",
                "--no-color",
                &format!("--pretty=format:{}", format),
                rev,
            ])
            .with_context(|| format!("could not resolve '{rev}'"))?;
        parse_log_output(&output, &self.remote_names())?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no commit found for '{rev}'"))
    }

    fn commit_diff(&self, oid: &str) -> anyhow::Result<Diff> {
        // `git show --format= --patch <oid>` works for both normal and root commits.
        let text = self
            .git(&["show", "--no-color", "--format=", "--patch", oid])
            .context("git show failed")?;
        Ok(parse_unified_diff(&text))
    }

    fn worktree_diff(&self, staged: bool) -> anyhow::Result<Diff> {
        let text = if staged {
            self.git(&["diff", "--cached", "--no-color"])
                .context("git diff --cached failed")?
        } else {
            self.git(&["diff", "--no-color"])
                .context("git diff failed")?
        };
        Ok(parse_unified_diff(&text))
    }

    fn file_diff(&self, path: &str, staged: bool) -> anyhow::Result<Diff> {
        if staged {
            let text = self
                .git(&["diff", "--cached", "--no-color", "--", path])
                .context("git diff --cached (file) failed")?;
            return Ok(parse_unified_diff(&text));
        }

        // Unstaged: a normal worktree diff for tracked files.
        let text = self
            .git(&["diff", "--no-color", "--", path])
            .context("git diff (file) failed")?;
        if !text.trim().is_empty() {
            return Ok(parse_unified_diff(&text));
        }

        // `git diff` ignores untracked files, so a brand-new file would otherwise
        // show an empty diff panel. Synthesise an all-added diff via
        // `git diff --no-index /dev/null <path>` (which exits 1 when the contents
        // differ — hence git_lenient tolerates the non-zero status).
        if !self.is_tracked(path) {
            let synth =
                self.git_lenient(&["diff", "--no-color", "--no-index", "--", "/dev/null", path])?;
            if !synth.trim().is_empty() {
                return Ok(parse_unified_diff(&synth));
            }
        }

        Ok(parse_unified_diff(&text))
    }

    fn stage(&self, paths: &[String]) -> anyhow::Result<()> {
        let mut args = vec!["add", "--"];
        let path_strs: Vec<&str> = paths.iter().map(String::as_str).collect();
        args.extend_from_slice(&path_strs);
        self.git(&args).context("git add failed")?;
        Ok(())
    }

    fn unstage(&self, paths: &[String]) -> anyhow::Result<()> {
        let mut args = vec!["restore", "--staged", "--"];
        let path_strs: Vec<&str> = paths.iter().map(String::as_str).collect();
        args.extend_from_slice(&path_strs);
        // Try `git restore --staged` first; fall back to `git reset -q HEAD` if it fails.
        if self.git(&args).is_err() {
            let mut fallback_args = vec!["reset", "-q", "HEAD", "--"];
            fallback_args.extend_from_slice(&path_strs);
            self.git(&fallback_args)
                .context("git reset (unstage fallback) failed")?;
        }
        Ok(())
    }

    fn stage_all(&self) -> anyhow::Result<()> {
        self.git(&["add", "-A"]).context("git add -A failed")?;
        Ok(())
    }

    fn unstage_all(&self) -> anyhow::Result<()> {
        // `git restore --staged .` is preferred; fall back to `git reset -q HEAD`.
        if self.git(&["restore", "--staged", "."]).is_err() {
            self.git(&["reset", "-q", "HEAD"])
                .context("git reset (unstage_all fallback) failed")?;
        }
        Ok(())
    }

    fn apply_patch(&self, patch: &str, cached: bool, reverse: bool) -> anyhow::Result<()> {
        let mut args = vec!["apply"];
        if cached {
            args.push("--cached");
        }
        if reverse {
            args.push("--reverse");
        }
        self.git_with_stdin(&args, patch)
            .context("git apply failed")?;
        Ok(())
    }

    fn commit(&self, message: &str) -> anyhow::Result<()> {
        // Use `-F -` (read message from stdin) to safely handle multiline messages
        // and special characters without shell quoting issues.
        self.git_with_stdin(&["commit", "-F", "-"], message)
            .context("git commit failed")?;
        Ok(())
    }

    fn commit_amend(&self, message: &str) -> anyhow::Result<()> {
        // `--amend -F -` rewrites HEAD's message (from stdin) and folds in any
        // currently-staged changes.
        self.git_with_stdin(&["commit", "--amend", "-F", "-"], message)
            .context("git commit --amend failed")?;
        Ok(())
    }

    fn last_commit_message(&self) -> anyhow::Result<String> {
        let msg = self
            .git(&["log", "-1", "--format=%B"])
            .context("git log -1 --format=%B failed")?;
        Ok(msg.trim_end().to_string())
    }

    // ── Phase 2: Branch / Worktree / Network ────────────────────────────────

    fn branches(&self) -> anyhow::Result<Vec<Branch>> {
        // Determine HEAD branch name for is_head marking.
        let head_ref = self
            .git(&["symbolic-ref", "--short", "HEAD"])
            .unwrap_or_default();
        let head_ref = head_ref.trim().to_owned();

        // Use for-each-ref over refs/heads and refs/remotes with a stable
        // delimiter (unit-separator 0x1f) between fields and newline between records.
        // Fields: refname:short | objectname:short | upstream:short | upstream:track
        let format = "%(refname:short)\x1f%(objectname)\x1f%(upstream:short)\x1f%(upstream:track)";

        let local_out = self
            .git(&[
                "for-each-ref",
                &format!("--format={}", format),
                "refs/heads",
            ])
            .context("git for-each-ref refs/heads failed")?;

        let remote_out = self
            .git(&[
                "for-each-ref",
                &format!("--format={}", format),
                "refs/remotes",
            ])
            .context("git for-each-ref refs/remotes failed")?;

        let mut branches = Vec::new();

        for (raw, kind) in [
            (local_out.as_str(), RefKind::LocalBranch),
            (remote_out.as_str(), RefKind::RemoteBranch),
        ] {
            for line in raw.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                // Skip the pseudo-ref "origin/HEAD" etc.
                if line.contains("/HEAD\x1f") || line.ends_with("/HEAD") {
                    continue;
                }
                let fields: Vec<&str> = line.splitn(4, '\x1f').collect();
                let name = fields.first().copied().unwrap_or("").to_owned();
                let target = fields.get(1).copied().unwrap_or("").to_owned();
                let upstream = {
                    let u = fields.get(2).copied().unwrap_or("").trim();
                    if u.is_empty() {
                        None
                    } else {
                        Some(u.to_owned())
                    }
                };
                let track_str = fields.get(3).copied().unwrap_or("").trim();
                let (ahead, behind) = parse_upstream_track(track_str);
                let is_head = matches!(kind, RefKind::LocalBranch) && name == head_ref;

                if name.is_empty() {
                    continue;
                }

                branches.push(Branch {
                    name,
                    kind: kind.clone(),
                    upstream,
                    ahead,
                    behind,
                    is_head,
                    target,
                });
            }
        }

        Ok(branches)
    }

    fn tags(&self) -> anyhow::Result<Vec<Tag>> {
        // Use for-each-ref over refs/tags.
        // For annotated tags, %(objectname) is the tag object; use *objectname for the
        // dereferenced commit. %(subject) gives the tag message subject line.
        let format = "%(refname:short)\x1f%(*objectname)%(objectname)\x1f%(subject)";
        let out = self
            .git(&["for-each-ref", &format!("--format={}", format), "refs/tags"])
            .context("git for-each-ref refs/tags failed")?;

        let mut tags = Vec::new();
        for line in out.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.splitn(3, '\x1f').collect();
            let name = fields.first().copied().unwrap_or("").to_owned();
            let target = fields.get(1).copied().unwrap_or("").to_owned();
            let message = fields.get(2).copied().unwrap_or("").to_owned();
            if name.is_empty() {
                continue;
            }
            tags.push(Tag {
                name,
                target,
                message,
            });
        }
        Ok(tags)
    }

    fn remotes(&self) -> anyhow::Result<Vec<(String, String)>> {
        // `git remote -v` outputs lines like:
        //   origin  https://github.com/... (fetch)
        //   origin  https://github.com/... (push)
        // We keep only the fetch URLs and deduplicate.
        let out = self
            .git(&["remote", "-v"])
            .context("git remote -v failed")?;
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for line in out.lines() {
            if !line.ends_with("(fetch)") {
                continue;
            }
            let line = line.trim_end_matches("(fetch)").trim();
            let mut parts = line.splitn(2, '\t');
            let name = parts.next().unwrap_or("").trim().to_owned();
            let url = parts.next().unwrap_or("").trim().to_owned();
            if !name.is_empty() && !url.is_empty() && seen.insert(name.clone()) {
                result.push((name, url));
            }
        }
        Ok(result)
    }

    fn checkout(&self, name: &str) -> anyhow::Result<()> {
        self.git(&["checkout", name])
            .with_context(|| format!("git checkout {} failed", name))?;
        Ok(())
    }

    fn create_branch(&self, name: &str, from: Option<&str>, checkout: bool) -> anyhow::Result<()> {
        if checkout {
            // `git checkout -b <name> [<from>]`
            if let Some(from_ref) = from {
                self.git(&["checkout", "-b", name, from_ref])
                    .with_context(|| format!("git checkout -b {} {} failed", name, from_ref))?;
            } else {
                self.git(&["checkout", "-b", name])
                    .with_context(|| format!("git checkout -b {} failed", name))?;
            }
        } else {
            // `git branch <name> [<from>]`
            if let Some(from_ref) = from {
                self.git(&["branch", name, from_ref])
                    .with_context(|| format!("git branch {} {} failed", name, from_ref))?;
            } else {
                self.git(&["branch", name])
                    .with_context(|| format!("git branch {} failed", name))?;
            }
        }
        Ok(())
    }

    fn delete_branch(&self, name: &str, force: bool) -> anyhow::Result<()> {
        let flag = if force { "-D" } else { "-d" };
        self.git(&["branch", flag, name])
            .with_context(|| format!("git branch {} {} failed", flag, name))?;
        Ok(())
    }

    fn rename_branch(&self, old: &str, new: &str) -> anyhow::Result<()> {
        self.git(&["branch", "-m", old, new])
            .with_context(|| format!("git branch -m {} {} failed", old, new))?;
        Ok(())
    }

    fn worktrees(&self) -> anyhow::Result<Vec<Worktree>> {
        let out = self
            .git(&["worktree", "list", "--porcelain"])
            .context("git worktree list --porcelain failed")?;
        let worktrees = parse_worktree_porcelain(&out, &self.root);
        Ok(worktrees)
    }

    fn worktree_add(&self, path: &str, branch: &str, new_branch: bool) -> anyhow::Result<()> {
        if new_branch {
            self.git(&["worktree", "add", "-b", branch, path])
                .with_context(|| format!("git worktree add -b {} {} failed", branch, path))?;
        } else {
            self.git(&["worktree", "add", path, branch])
                .with_context(|| format!("git worktree add {} {} failed", path, branch))?;
        }
        Ok(())
    }

    fn worktree_remove(&self, path: &str, force: bool) -> anyhow::Result<()> {
        if force {
            self.git(&["worktree", "remove", "--force", path])
                .with_context(|| format!("git worktree remove --force {} failed", path))?;
        } else {
            self.git(&["worktree", "remove", path])
                .with_context(|| format!("git worktree remove {} failed", path))?;
        }
        Ok(())
    }

    fn worktree_prune(&self) -> anyhow::Result<()> {
        self.git(&["worktree", "prune"])
            .context("git worktree prune failed")?;
        Ok(())
    }

    fn fetch(&self, remote: Option<&str>) -> anyhow::Result<()> {
        if let Some(r) = remote {
            self.git(&["fetch", r])
                .with_context(|| format!("git fetch {} failed", r))?;
        } else {
            self.git(&["fetch", "--all"])
                .context("git fetch --all failed")?;
        }
        Ok(())
    }

    fn pull(&self) -> anyhow::Result<()> {
        self.git(&["pull"]).context("git pull failed")?;
        Ok(())
    }

    fn push(&self, remote: Option<&str>, branch: Option<&str>, force: bool) -> anyhow::Result<()> {
        // Build the arg list as owned Strings first, then pass slices to git().
        let mut owned: Vec<String> = vec!["push".to_owned()];
        if force {
            owned.push("--force-with-lease".to_owned());
        }
        if let Some(r) = remote {
            owned.push(r.to_owned());
        }
        if let Some(b) = branch {
            owned.push(b.to_owned());
        }
        let args: Vec<&str> = owned.iter().map(String::as_str).collect();
        self.git(&args).context("git push failed")?;
        Ok(())
    }

    // ── Phase 3: Stash ──────────────────────────────────────────────────────

    fn stashes(&self) -> anyhow::Result<Vec<Stash>> {
        // Format: index \x1f oid \x1f message (one record per line)
        // We use the reflog format for stash which gives us stash@{i} info.
        // `git stash list --format="%gd\x1f%H\x1f%s"` produces:
        //   stash@{0}\x1f<oid>\x1f<message>
        let format = "%gd\x1f%H\x1f%s";
        let out = self
            .git(&["stash", "list", &format!("--format={}", format)])
            .context("git stash list failed")?;

        parse_stash_list(&out)
    }

    fn stash_save(&self, message: Option<&str>, include_untracked: bool) -> anyhow::Result<()> {
        let mut args = vec!["stash", "push"];
        if include_untracked {
            args.push("-u");
        }
        if let Some(msg) = message {
            args.push("-m");
            args.push(msg);
        }
        self.git(&args).context("git stash push failed")?;
        Ok(())
    }

    fn stash_pop(&self, index: usize) -> anyhow::Result<()> {
        let stash_ref = format!("stash@{{{}}}", index);
        self.git(&["stash", "pop", &stash_ref])
            .with_context(|| format!("git stash pop stash@{{{}}} failed", index))?;
        Ok(())
    }

    fn stash_apply(&self, index: usize) -> anyhow::Result<()> {
        let stash_ref = format!("stash@{{{}}}", index);
        self.git(&["stash", "apply", &stash_ref])
            .with_context(|| format!("git stash apply stash@{{{}}} failed", index))?;
        Ok(())
    }

    fn stash_drop(&self, index: usize) -> anyhow::Result<()> {
        let stash_ref = format!("stash@{{{}}}", index);
        self.git(&["stash", "drop", &stash_ref])
            .with_context(|| format!("git stash drop stash@{{{}}} failed", index))?;
        Ok(())
    }

    fn stash_show(&self, index: usize) -> anyhow::Result<Diff> {
        let stash_ref = format!("stash@{{{}}}", index);
        // We always stash with `-u`, so the entry can hold untracked files.
        // `--include-untracked` makes `stash show` display them too; without it
        // the preview silently omits untracked files (looked like a bug vs other
        // tools). Fall back to the plain form on git versions that lack the flag.
        let text = self
            .git(&[
                "stash",
                "show",
                "--include-untracked",
                "-p",
                "--no-color",
                &stash_ref,
            ])
            .or_else(|_| self.git(&["stash", "show", "-p", "--no-color", &stash_ref]))
            .with_context(|| format!("git stash show stash@{{{}}} failed", index))?;
        Ok(parse_unified_diff(&text))
    }

    // ── Phase 3: History operations ─────────────────────────────────────────

    fn merge(&self, branch: &str, no_ff: bool) -> anyhow::Result<()> {
        let mut args = vec!["merge"];
        if no_ff {
            args.push("--no-ff");
        }
        // Never open an editor for the merge commit message (would hang the TUI).
        args.push("--no-edit");
        args.push(branch);
        // Conflicts are detected separately via .git marker files
        // (operation_in_progress), so don't mangle the error message here — a
        // non-conflict failure (bad ref, etc.) must read as itself.
        self.git(&args)
            .with_context(|| format!("git merge {branch} failed"))?;
        Ok(())
    }

    fn rebase(&self, onto: &str) -> anyhow::Result<()> {
        self.git(&["rebase", onto])
            .with_context(|| format!("git rebase {onto} failed"))?;
        Ok(())
    }

    fn rebase_interactive(&self, base: &str, todo: &[(String, String)]) -> anyhow::Result<()> {
        // Build the desired todo content: "<command> <oid>\n" per entry.
        let mut todo_content = String::new();
        for (command, oid) in todo {
            todo_content.push_str(command);
            todo_content.push(' ');
            todo_content.push_str(oid);
            todo_content.push('\n');
        }

        // Write todo to a temp file.
        let tmp = tempfile_path()?;
        std::fs::write(&tmp, &todo_content)
            .with_context(|| format!("failed to write rebase todo to {:?}", tmp))?;

        // GIT_SEQUENCE_EDITOR="cp <tmp>" so git copies our file over the rebase todo.
        // GIT_EDITOR=true suppresses any interactive editor (reword/squash messages).
        let tmp_str = tmp.to_string_lossy().into_owned();
        let cp_cmd = format!("cp {}", shell_escape(&tmp_str));
        let envs: &[(&str, &str)] = &[
            ("GIT_SEQUENCE_EDITOR", cp_cmd.as_str()),
            ("GIT_EDITOR", "true"),
        ];

        let result = self.git_env(&["rebase", "-i", base], envs);

        // Clean up the temp file regardless of rebase success/failure.
        let _ = std::fs::remove_file(&tmp);

        result.with_context(|| "git rebase -i failed")?;
        Ok(())
    }

    fn cherry_pick(&self, oid: &str) -> anyhow::Result<()> {
        self.git(&["cherry-pick", oid])
            .with_context(|| format!("git cherry-pick {oid} failed"))?;
        Ok(())
    }

    fn revert(&self, oid: &str, no_commit: bool) -> anyhow::Result<()> {
        let mut args = vec!["revert"];
        if no_commit {
            args.push("--no-commit");
        }
        args.push(oid);
        // Suppress interactive editor for the commit message.
        self.git_env(&args, &[("GIT_EDITOR", "true")])
            .with_context(|| format!("git revert {oid} failed"))?;
        Ok(())
    }

    fn reset(&self, mode: ResetMode, target: &str) -> anyhow::Result<()> {
        let flag = match mode {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        };
        self.git(&["reset", flag, target])
            .with_context(|| format!("git reset {} {} failed", flag, target))?;
        Ok(())
    }

    // ── Phase 3: Tag management ─────────────────────────────────────────────

    fn tag_create(
        &self,
        name: &str,
        target: Option<&str>,
        message: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut args = vec!["tag"];
        // If a message is provided, create an annotated tag.
        if let Some(msg) = message {
            args.push("-m");
            args.push(msg);
        }
        args.push(name);
        if let Some(t) = target {
            args.push(t);
        }
        self.git(&args)
            .with_context(|| format!("git tag create {} failed", name))?;
        Ok(())
    }

    fn tag_delete(&self, name: &str) -> anyhow::Result<()> {
        self.git(&["tag", "-d", name])
            .with_context(|| format!("git tag -d {} failed", name))?;
        Ok(())
    }

    // ── Phase 3: Conflict / sequencer state ────────────────────────────────

    fn operation_in_progress(&self) -> anyhow::Result<Option<OpInProgress>> {
        let git_dir = self.git_dir()?;

        // Detect the operation kind from marker files / directories.
        let kind = if git_dir.join("rebase-merge").is_dir() || git_dir.join("rebase-apply").is_dir()
        {
            Some(OpKind::Rebase)
        } else if git_dir.join("MERGE_HEAD").exists() {
            Some(OpKind::Merge)
        } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
            Some(OpKind::CherryPick)
        } else if git_dir.join("REVERT_HEAD").exists() {
            Some(OpKind::Revert)
        } else {
            None
        };

        match kind {
            None => Ok(None),
            Some(k) => {
                let conflicted = self.conflicted_files()?;
                Ok(Some(OpInProgress {
                    kind: k,
                    conflicted,
                }))
            }
        }
    }

    fn op_continue(&self, kind: OpKind) -> anyhow::Result<()> {
        let subcmd = op_subcommand(kind);
        self.git_env(&[subcmd, "--continue"], &[("GIT_EDITOR", "true")])
            .with_context(|| format!("git {} --continue failed", subcmd))?;
        Ok(())
    }

    fn op_abort(&self, kind: OpKind) -> anyhow::Result<()> {
        let subcmd = op_subcommand(kind);
        self.git(&[subcmd, "--abort"])
            .with_context(|| format!("git {} --abort failed", subcmd))?;
        Ok(())
    }

    fn op_skip(&self, kind: OpKind) -> anyhow::Result<()> {
        if matches!(kind, OpKind::Merge) {
            anyhow::bail!("merge has no --skip; use abort or resolve the conflict");
        }
        let subcmd = op_subcommand(kind);
        // `--skip` may itself hit further conflicts; suppress any editor.
        self.git_env(&[subcmd, "--skip"], &[("GIT_EDITOR", "true")])
            .with_context(|| format!("git {} --skip failed", subcmd))?;
        Ok(())
    }

    fn conflicted_files(&self) -> anyhow::Result<Vec<String>> {
        // `git diff --name-only --diff-filter=U` lists files with conflict markers.
        let out = self
            .git(&["diff", "--name-only", "--diff-filter=U"])
            .context("git diff --name-only --diff-filter=U failed")?;
        let files = out
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        Ok(files)
    }

    fn mark_resolved(&self, path: &str) -> anyhow::Result<()> {
        self.git(&["add", "--", path])
            .with_context(|| format!("git add -- {} failed", path))?;
        Ok(())
    }
}

// ─── Phase 3 helpers ─────────────────────────────────────────────────────────

impl CliBackend {
    /// Return the path to the `.git` directory (or the gitdir file target for worktrees).
    fn git_dir(&self) -> anyhow::Result<PathBuf> {
        let out = self
            .git(&["rev-parse", "--git-dir"])
            .context("git rev-parse --git-dir failed")?;
        let p = out.trim();
        // The path may be relative to the repo root.
        let path = if std::path::Path::new(p).is_absolute() {
            PathBuf::from(p)
        } else {
            self.root.join(p)
        };
        Ok(path)
    }
}

// ─── spawn_git: background git task helper ───────────────────────────────────

/// Spawn a background thread that runs `git -c color.ui=never --no-pager <args>`
/// in `root` and sends an `Action::GitTaskDone` when complete.
///
/// The calling UI thread remains unblocked while the operation runs.
pub fn spawn_git(
    root: std::path::PathBuf,
    args: Vec<String>,
    label: String,
    tx: std::sync::mpsc::Sender<crate::action::Action>,
    refresh_after: bool,
    check_op: bool,
) {
    std::thread::spawn(move || {
        let mut cmd = Command::new("git");
        cmd.args([
            "-c",
            "color.ui=never",
            "-c",
            "core.quotepath=false",
            "--no-pager",
        ])
        .args(&args)
        .current_dir(&root)
        // Never block on an editor or a terminal credential prompt — a hung
        // background git would leak a thread and never report completion.
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat");

        let result = cmd.output();
        let (ok, message) = match result {
            Err(e) => (false, format!("failed to spawn git: {}", e)),
            Ok(output) => {
                let ok = output.status.success();
                let message = if ok {
                    String::from_utf8_lossy(&output.stdout).trim().to_owned()
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                    if stderr.is_empty() {
                        stdout
                    } else {
                        stderr
                    }
                };
                (ok, message)
            }
        };

        // Best-effort send; if the receiver is gone (app quit), ignore the error.
        let _ = tx.send(crate::action::Action::GitTaskDone {
            label,
            ok,
            message,
            refresh_after,
            check_op,
        });
    });
}
