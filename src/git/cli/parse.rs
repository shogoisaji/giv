//! Pure parsing of git's plumbing output, plus small string/path helpers.
//!
//! Everything here is free of I/O — it turns the text/bytes produced by a git
//! subprocess into the domain types in [`crate::git::types`]. Keeping it separate
//! from the I/O-driving [`super::CliBackend`] makes it independently testable
//! (see the unit tests at the bottom of this file).

use crate::git::{
    Commit, FileStatus, OpKind, RefKind, RefName, Stash, StatusCode, WorkingStatus, Worktree,
};

// ─── Stash list parser ─────────────────────────────────────────────────────────

/// Parse `git stash list --format="%gd\x1f%H\x1f%s"` output.
///
/// Each line has the form: `stash@{N}\x1f<oid>\x1f<message>`
pub(crate) fn parse_stash_list(output: &str) -> anyhow::Result<Vec<Stash>> {
    let mut stashes = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.splitn(3, '\x1f').collect();
        // Extract numeric index from "stash@{N}".
        let stash_ref = fields.first().copied().unwrap_or("");
        let index = parse_stash_index(stash_ref);
        let oid = fields.get(1).copied().unwrap_or("").trim().to_owned();
        let message = fields.get(2).copied().unwrap_or("").trim().to_owned();
        stashes.push(Stash {
            index,
            message,
            oid,
        });
    }
    Ok(stashes)
}

/// Extract the numeric index from a `stash@{N}` reference string.
fn parse_stash_index(s: &str) -> usize {
    // Expected format: "stash@{N}"
    if let Some(inner) = s.strip_prefix("stash@{").and_then(|r| r.strip_suffix('}')) {
        inner.parse().unwrap_or(0)
    } else {
        0
    }
}

// ─── Temp-file / shell helpers (used by interactive rebase & sequencer) ─────────

/// Create a path for a temporary file in the system temp directory.
/// Does not create the file — only produces a unique path.
pub(crate) fn tempfile_path() -> anyhow::Result<std::path::PathBuf> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("giv_rebase_todo_{}.txt", ts));
    Ok(path)
}

/// Minimally shell-escape a string for use in a simple shell command.
/// Wraps the string in single quotes and escapes any embedded single quotes.
pub(crate) fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// Return the git subcommand string for an `OpKind`.
///
/// The caller appends `--continue` or `--abort` directly.
pub(crate) fn op_subcommand(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Merge => "merge",
        OpKind::Rebase => "rebase",
        OpKind::CherryPick => "cherry-pick",
        OpKind::Revert => "revert",
    }
}

// ─── for-each-ref / upstream parsing ───────────────────────────────────────────

/// Parse `%(upstream:track)` output, e.g. `[ahead 3, behind 1]`, `[ahead 2]`,
/// `[behind 4]`, or empty string.
pub(crate) fn parse_upstream_track(track: &str) -> (usize, usize) {
    let inner = track.trim().trim_start_matches('[').trim_end_matches(']');
    let mut ahead = 0usize;
    let mut behind = 0usize;
    for part in inner.split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.trim().parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.trim().parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

/// Parse `git worktree list --porcelain` output into a Vec<Worktree>.
///
/// Each worktree block is separated by a blank line. Fields within a block:
/// ```text
/// worktree /path/to/worktree
/// HEAD <sha>
/// branch refs/heads/<name>   (or "detached")
/// bare                        (optional)
/// locked [reason]             (optional)
/// ```
pub(crate) fn parse_worktree_porcelain(output: &str, main_root: &std::path::Path) -> Vec<Worktree> {
    let mut worktrees = Vec::new();

    // Blocks are separated by blank lines.
    for block in output.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let mut path = String::new();
        let mut head = String::new();
        let mut branch: Option<String> = None;
        let mut is_bare = false;
        let mut is_locked = false;

        for line in block.lines() {
            let line = line.trim_end();
            if let Some(p) = line.strip_prefix("worktree ") {
                path = p.to_owned();
            } else if let Some(h) = line.strip_prefix("HEAD ") {
                head = h.to_owned();
            } else if let Some(b) = line.strip_prefix("branch ") {
                // Strip "refs/heads/" prefix for local branches; keep full for remote.
                let name = b.strip_prefix("refs/heads/").unwrap_or(b).to_owned();
                branch = Some(name);
            } else if line == "bare" {
                is_bare = true;
            } else if line == "detached" {
                branch = None;
            } else if line.starts_with("locked") {
                is_locked = true;
            }
        }

        if path.is_empty() {
            continue;
        }

        let wt_path = std::path::Path::new(&path);
        let is_current = wt_path == main_root;

        worktrees.push(Worktree {
            path,
            branch,
            head,
            is_current,
            is_bare,
            is_locked,
        });
    }

    worktrees
}

// ─── Porcelain v2 status parser ────────────────────────────────────────────────

/// Parse the output of `git status --porcelain=v2 --branch -z`.
///
/// The output is NUL-separated. Header (`# `) records describe branch info;
/// entry records describe individual file states.
pub(crate) fn parse_porcelain_v2(raw: &[u8]) -> anyhow::Result<WorkingStatus> {
    let mut status = WorkingStatus::default();

    // Split on NUL bytes to get individual records/tokens.
    // Note: rename entries (type `2`) have TWO NUL-separated tokens: the entry line
    // and the original filename.
    let records: Vec<&[u8]> = raw.split(|&b| b == 0).collect();
    let mut i = 0;

    while i < records.len() {
        let record = records[i];
        let line = String::from_utf8_lossy(record);
        let line = line.trim_end_matches('\n');

        if line.is_empty() {
            i += 1;
            continue;
        }

        if let Some(rest) = line.strip_prefix("# branch.head ") {
            let head = rest.trim();
            if head != "(detached)" {
                status.branch = Some(head.to_owned());
            } else {
                status.branch = None; // detached HEAD
            }
        } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            status.upstream = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // Format: +A -B
            let parts: Vec<&str> = rest.split_whitespace().collect();
            for part in &parts {
                if let Some(a) = part.strip_prefix('+') {
                    status.ahead = a.parse().unwrap_or(0);
                } else if let Some(b) = part.strip_prefix('-') {
                    status.behind = b.parse().unwrap_or(0);
                }
            }
        } else if line.starts_with("# ") {
            // Other header lines (branch.oid, etc.) — skip.
        } else if line.starts_with("1 ") {
            // Ordinary changed entry.
            // Format: 1 XY sub mH mI mW hH hI path
            if let Some(entry) = parse_ordinary_entry(line) {
                status.entries.push(entry);
            }
        } else if line.starts_with("2 ") {
            // Rename or copy entry.
            // Format: 2 XY sub mH mI mW hH hI X score path\0origPath
            // The original path is the NEXT NUL-separated token.
            let orig_path = if i + 1 < records.len() {
                let s = String::from_utf8_lossy(records[i + 1]).into_owned();
                i += 1; // consume the extra token
                Some(s)
            } else {
                None
            };
            if let Some(mut entry) = parse_rename_entry(line) {
                entry.orig_path = orig_path;
                status.entries.push(entry);
            }
        } else if line.starts_with("u ") {
            // Unmerged entry.
            // Format: u XY sub m1 m2 m3 mW h1 h2 h3 path
            if let Some(entry) = parse_unmerged_entry(line) {
                status.entries.push(entry);
            }
        } else if line.starts_with("? ") {
            // Untracked file.
            let path = line.strip_prefix("? ").unwrap_or(line).trim().to_owned();
            if !path.is_empty() {
                status.entries.push(FileStatus {
                    path,
                    orig_path: None,
                    index: StatusCode::Untracked,
                    worktree: StatusCode::Untracked,
                });
            }
        } else if line.starts_with("! ") {
            // Ignored file — skip (not shown by default, but handle gracefully).
        }

        i += 1;
    }

    Ok(status)
}

/// Parse ordinary changed entry (type `1`).
/// Format: `1 XY sub mH mI mW hH hI path`
fn parse_ordinary_entry(line: &str) -> Option<FileStatus> {
    // Split into at most 9 fields (path may contain spaces, but it's the last field).
    let mut fields = line.splitn(9, ' ');
    let _type = fields.next()?; // "1"
    let xy = fields.next()?; // XY status codes
                             // Skip sub, mH, mI, mW, hH, hI
    for _ in 0..6 {
        fields.next()?;
    }
    let path = fields.next()?.to_owned();

    let (index, worktree) = parse_xy(xy);
    Some(FileStatus {
        path,
        orig_path: None,
        index,
        worktree,
    })
}

/// Parse rename/copy entry (type `2`).
/// Format: `2 XY sub mH mI mW hH hI X score path`
/// The original path comes as the next NUL-separated token (handled by caller).
fn parse_rename_entry(line: &str) -> Option<FileStatus> {
    // Format: 2 XY sub mH mI mW hH hI <X><score> path
    // Note: <X><score> is one space-delimited token (e.g. "R100"), not two.
    // Total: 10 space-separated fields (indices 0..=9), so we need splitn(10, ' ').
    let mut fields = line.splitn(10, ' ');
    let _type = fields.next()?; // "2"
    let xy = fields.next()?; // XY status codes
                             // Skip sub, mH, mI, mW, hH, hI, <X><score> (7 fields)
    for _ in 0..7 {
        fields.next()?;
    }
    let path = fields.next()?.to_owned();

    let (index, worktree) = parse_xy(xy);
    Some(FileStatus {
        path,
        orig_path: None, // caller fills this in
        index,
        worktree,
    })
}

/// Parse unmerged entry (type `u`).
/// Format: `u XY sub m1 m2 m3 mW h1 h2 h3 path`
fn parse_unmerged_entry(line: &str) -> Option<FileStatus> {
    let mut fields = line.splitn(11, ' ');
    let _type = fields.next()?; // "u"
    let _xy = fields.next()?; // XY (we always treat as Conflicted)
                              // Skip sub, m1, m2, m3, mW, h1, h2, h3
    for _ in 0..8 {
        fields.next()?;
    }
    let path = fields.next()?.to_owned();

    Some(FileStatus {
        path,
        orig_path: None,
        index: StatusCode::Conflicted,
        worktree: StatusCode::Conflicted,
    })
}

/// Convert XY status code pair into (index StatusCode, worktree StatusCode).
fn parse_xy(xy: &str) -> (StatusCode, StatusCode) {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    (char_to_status(x), char_to_status(y))
}

/// Map a single porcelain-v2 status character to a `StatusCode`.
fn char_to_status(c: char) -> StatusCode {
    match c {
        '.' | ' ' => StatusCode::Unmodified,
        'M' => StatusCode::Modified,
        'A' => StatusCode::Added,
        'D' => StatusCode::Deleted,
        'R' => StatusCode::Renamed,
        'C' => StatusCode::Copied,
        'T' => StatusCode::TypeChange,
        'U' => StatusCode::Conflicted,
        '?' => StatusCode::Untracked,
        '!' => StatusCode::Ignored,
        _ => StatusCode::Unmodified,
    }
}

// ─── Log parser ──────────────────────────────────────────────────────────────

/// Parse the output of `git log --pretty=format:...` using unit-separator (0x1f)
/// field delimiter and record-separator (0x1e) commit delimiter.
///
/// `remotes` is the list of configured remote names (e.g. `["origin"]`), used to
/// tell a remote-tracking ref (`origin/main`) from a local branch whose name
/// merely contains a slash (`feature/api`).
pub(crate) fn parse_log_output(output: &str, remotes: &[String]) -> anyhow::Result<Vec<Commit>> {
    let mut commits = Vec::new();

    // Records are separated by 0x1e (record separator).
    for record in output.split('\x1e') {
        let record = record.trim_matches('\n').trim();
        if record.is_empty() {
            continue;
        }

        // Fields separated by 0x1f (unit separator):
        // 0: full hash, 1: short hash, 2: parents (space-separated),
        // 3: author name, 4: author email, 5: author timestamp,
        // 6: subject, 7: body, 8: ref decoration (%D)
        let fields: Vec<&str> = record.splitn(9, '\x1f').collect();
        if fields.len() < 8 {
            // Malformed record — skip rather than bail.
            continue;
        }

        let id = fields[0].trim().to_owned();
        let short_id = fields[1].trim().to_owned();
        let parents: Vec<String> = fields[2]
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_owned())
            .collect();
        let author_name = fields[3].trim().to_owned();
        let author_email = fields[4].trim().to_owned();
        let time: i64 = fields[5].trim().parse().unwrap_or(0);
        let summary = fields[6].trim().to_owned();
        let body = fields[7].trim().to_owned();
        let decoration = fields.get(8).copied().unwrap_or("").trim();

        let refs = parse_ref_decoration(decoration, remotes);

        if id.is_empty() {
            continue;
        }

        commits.push(Commit {
            id,
            short_id,
            parents,
            author_name,
            author_email,
            time,
            summary,
            body,
            refs,
        });
    }

    Ok(commits)
}

/// Parse git's `%D` ref decoration string into a `Vec<RefName>`.
///
/// Example input: `HEAD -> main, origin/main, tag: v1.0`
///
/// `remotes` lists configured remote names so a remote-tracking ref can be told
/// apart from a local branch with a slash in its name. A slash alone does NOT
/// mean remote — `feature/api`, `release/1.2` etc. are local branches.
fn parse_ref_decoration(decoration: &str, remotes: &[String]) -> Vec<RefName> {
    let mut refs = Vec::new();
    if decoration.is_empty() {
        return refs;
    }

    for part in decoration.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if part == "HEAD" {
            refs.push(RefName {
                name: "HEAD".to_owned(),
                kind: RefKind::Head,
            });
        } else if let Some(branch) = part.strip_prefix("HEAD -> ") {
            // HEAD pointing to a local branch.
            // Store the branch name in the Head entry so render_ascii can show
            // "HEAD -> <branch>" without a redundant second entry.
            refs.push(RefName {
                name: branch.to_owned(),
                kind: RefKind::Head,
            });
        } else if let Some(tag_name) = part.strip_prefix("tag: ") {
            refs.push(RefName {
                name: tag_name.to_owned(),
                kind: RefKind::Tag,
            });
        } else if remotes.iter().any(|r| part.starts_with(&format!("{r}/"))) {
            // Prefixed by a known remote name (e.g. `origin/main`) → remote branch.
            refs.push(RefName {
                name: part.to_owned(),
                kind: RefKind::RemoteBranch,
            });
        } else {
            // Everything else is a LOCAL branch — including slashed names like
            // `feature/api` or `release/1.2`. A slash does not imply remote.
            refs.push(RefName {
                name: part.to_owned(),
                kind: RefKind::LocalBranch,
            });
        }
    }

    refs
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Porcelain v2 status parser tests ──────────────────────────────────────

    /// Helper: build a NUL-terminated porcelain-v2 status buffer from a list of lines.
    /// Each line is NUL-terminated; rename entries have the orig_path as the next token.
    fn make_status_bytes(lines: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for line in lines {
            out.extend_from_slice(line.as_bytes());
            out.push(0u8); // NUL terminator
        }
        out
    }

    #[test]
    fn test_status_modified_staged_file() {
        // A file modified in index (staged) and unmodified in worktree.
        let raw = make_status_bytes(&[
            "# branch.oid 1234567890abcdef",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +0 -0",
            "1 M. N... 100644 100644 100644 aaa bbb src/foo.rs",
        ]);
        let status = parse_porcelain_v2(&raw).unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.ahead, 0);
        assert_eq!(status.behind, 0);
        assert_eq!(status.entries.len(), 1);
        let entry = &status.entries[0];
        assert_eq!(entry.path, "src/foo.rs");
        assert_eq!(entry.index, StatusCode::Modified);
        assert_eq!(entry.worktree, StatusCode::Unmodified);
        assert!(entry.orig_path.is_none());
    }

    #[test]
    fn test_status_untracked_file() {
        let raw = make_status_bytes(&["# branch.head main", "? untracked_file.txt"]);
        let status = parse_porcelain_v2(&raw).unwrap();
        assert_eq!(status.entries.len(), 1);
        let entry = &status.entries[0];
        assert_eq!(entry.path, "untracked_file.txt");
        assert_eq!(entry.index, StatusCode::Untracked);
        assert_eq!(entry.worktree, StatusCode::Untracked);
    }

    #[test]
    fn test_status_rename_entry() {
        // A renamed file: type 2, followed by original path as next NUL-separated token.
        // Format: 2 XY sub mH mI mW hH hI X score newpath\0origpath
        let mut raw: Vec<u8> = Vec::new();
        // header
        raw.extend_from_slice(b"# branch.head main");
        raw.push(0);
        // rename entry (10 space-separated fields, last is new path)
        raw.extend_from_slice(b"2 R. N... 100644 100644 100644 aaa bbb R100 new_name.rs");
        raw.push(0);
        // original path (next NUL token)
        raw.extend_from_slice(b"old_name.rs");
        raw.push(0);

        let status = parse_porcelain_v2(&raw).unwrap();
        assert_eq!(status.entries.len(), 1);
        let entry = &status.entries[0];
        assert_eq!(entry.path, "new_name.rs");
        assert_eq!(entry.orig_path.as_deref(), Some("old_name.rs"));
        assert_eq!(entry.index, StatusCode::Renamed);
        assert_eq!(entry.worktree, StatusCode::Unmodified);
    }

    #[test]
    fn test_status_ahead_behind() {
        let raw = make_status_bytes(&[
            "# branch.head feature",
            "# branch.upstream origin/feature",
            "# branch.ab +3 -1",
        ]);
        let status = parse_porcelain_v2(&raw).unwrap();
        assert_eq!(status.ahead, 3);
        assert_eq!(status.behind, 1);
    }

    #[test]
    fn test_status_worktree_modified() {
        // A file with worktree modification (unstaged): XY = .M
        let raw = make_status_bytes(&[
            "# branch.head main",
            "1 .M N... 100644 100644 100644 aaa bbb src/bar.rs",
        ]);
        let status = parse_porcelain_v2(&raw).unwrap();
        assert_eq!(status.entries.len(), 1);
        let entry = &status.entries[0];
        assert_eq!(entry.index, StatusCode::Unmodified);
        assert_eq!(entry.worktree, StatusCode::Modified);
    }

    #[test]
    fn test_status_both_modified() {
        // Both staged and worktree modified: XY = MM
        let raw = make_status_bytes(&[
            "# branch.head main",
            "1 MM N... 100644 100644 100644 aaa bbb src/baz.rs",
        ]);
        let status = parse_porcelain_v2(&raw).unwrap();
        let entry = &status.entries[0];
        assert_eq!(entry.index, StatusCode::Modified);
        assert_eq!(entry.worktree, StatusCode::Modified);
    }

    #[test]
    fn test_status_deleted() {
        // Staged deletion: XY = D.
        let raw = make_status_bytes(&[
            "# branch.head main",
            "1 D. N... 100644 000000 000000 aaa 0000000 deleted_file.rs",
        ]);
        let status = parse_porcelain_v2(&raw).unwrap();
        let entry = &status.entries[0];
        assert_eq!(entry.index, StatusCode::Deleted);
        assert_eq!(entry.worktree, StatusCode::Unmodified);
    }

    // ── Log / ref decoration tests ────────────────────────────────────────────

    #[test]
    fn test_parse_ref_decoration_empty() {
        let refs = parse_ref_decoration("", &[]);
        assert!(refs.is_empty());
    }

    #[test]
    fn test_parse_ref_decoration_head_branch() {
        let remotes = ["origin".to_string()];
        let refs = parse_ref_decoration("HEAD -> main, origin/main", &remotes);
        // "HEAD -> main" is stored as a single Head entry with name="main" so
        // render_ascii can produce "HEAD -> main" without a redundant extra entry.
        assert!(refs
            .iter()
            .any(|r| r.kind == RefKind::Head && r.name == "main"));
        assert!(refs
            .iter()
            .any(|r| r.name == "origin/main" && r.kind == RefKind::RemoteBranch));
    }

    #[test]
    fn test_parse_ref_decoration_tag() {
        let refs = parse_ref_decoration("tag: v1.0.0", &[]);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "v1.0.0");
        assert_eq!(refs[0].kind, RefKind::Tag);
    }

    #[test]
    fn test_parse_ref_decoration_detached_head() {
        let refs = parse_ref_decoration("HEAD", &[]);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].kind, RefKind::Head);
    }

    #[test]
    fn test_parse_ref_decoration_local_branch() {
        let refs = parse_ref_decoration("feature-branch", &[]);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "feature-branch");
        assert_eq!(refs[0].kind, RefKind::LocalBranch);
    }

    #[test]
    fn test_parse_ref_decoration_slashed_local_branch() {
        // A slash does NOT make a branch remote: `feature/api` is local, while
        // `origin/feature/api` (known remote prefix) is the remote-tracking ref.
        let remotes = ["origin".to_string()];
        let refs = parse_ref_decoration("feature/api, origin/feature/api", &remotes);
        assert!(
            refs.iter()
                .any(|r| r.name == "feature/api" && r.kind == RefKind::LocalBranch),
            "slashed local branch must stay local: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "origin/feature/api" && r.kind == RefKind::RemoteBranch),
            "known-remote-prefixed ref is remote: {refs:?}"
        );
    }

    // ── Phase 2: worktree --porcelain parser tests ────────────────────────────

    #[test]
    fn test_parse_worktree_main_only() {
        let input = "worktree /home/user/project\nHEAD abc1234\nbranch refs/heads/main\n";
        let root = std::path::Path::new("/home/user/project");
        let wts = parse_worktree_porcelain(input, root);
        assert_eq!(wts.len(), 1);
        let wt = &wts[0];
        assert_eq!(wt.path, "/home/user/project");
        assert_eq!(wt.head, "abc1234");
        assert_eq!(wt.branch.as_deref(), Some("main"));
        assert!(wt.is_current);
        assert!(!wt.is_bare);
        assert!(!wt.is_locked);
    }

    #[test]
    fn test_parse_worktree_two_worktrees() {
        let input = concat!(
            "worktree /home/user/project\n",
            "HEAD aabbccdd\n",
            "branch refs/heads/main\n",
            "\n",
            "worktree /home/user/project-feat\n",
            "HEAD 11223344\n",
            "branch refs/heads/feature/foo\n",
            "locked\n",
        );
        let root = std::path::Path::new("/home/user/project");
        let wts = parse_worktree_porcelain(input, root);
        assert_eq!(wts.len(), 2);

        let main = &wts[0];
        assert_eq!(main.branch.as_deref(), Some("main"));
        assert!(main.is_current);
        assert!(!main.is_locked);

        let feat = &wts[1];
        assert_eq!(feat.path, "/home/user/project-feat");
        assert_eq!(feat.branch.as_deref(), Some("feature/foo"));
        assert!(!feat.is_current);
        assert!(feat.is_locked);
    }

    #[test]
    fn test_parse_worktree_bare() {
        let input = "worktree /home/user/bare.git\nHEAD 0000000\nbranch refs/heads/main\nbare\n";
        let root = std::path::Path::new("/home/user/project");
        let wts = parse_worktree_porcelain(input, root);
        assert_eq!(wts.len(), 1);
        assert!(wts[0].is_bare);
        assert!(!wts[0].is_current);
    }

    #[test]
    fn test_parse_worktree_detached() {
        let input = "worktree /home/user/project\nHEAD deadbeef\ndetached\n";
        let root = std::path::Path::new("/home/user/project");
        let wts = parse_worktree_porcelain(input, root);
        assert_eq!(wts.len(), 1);
        assert!(wts[0].branch.is_none());
        assert!(wts[0].is_current);
    }

    // ── Phase 2: for-each-ref branch parser tests ─────────────────────────────

    #[test]
    fn test_parse_upstream_track_empty() {
        assert_eq!(parse_upstream_track(""), (0, 0));
    }

    #[test]
    fn test_parse_upstream_track_ahead_only() {
        assert_eq!(parse_upstream_track("[ahead 3]"), (3, 0));
    }

    #[test]
    fn test_parse_upstream_track_behind_only() {
        assert_eq!(parse_upstream_track("[behind 2]"), (0, 2));
    }

    #[test]
    fn test_parse_upstream_track_ahead_and_behind() {
        assert_eq!(parse_upstream_track("[ahead 5, behind 1]"), (5, 1));
    }

    #[test]
    fn test_parse_upstream_track_gone() {
        // "[gone]" means upstream was deleted; we return (0,0).
        assert_eq!(parse_upstream_track("[gone]"), (0, 0));
    }

    // ── Phase 3: stash list parser tests ─────────────────────────────────────

    #[test]
    fn test_parse_stash_list_empty() {
        let stashes = parse_stash_list("").unwrap();
        assert!(stashes.is_empty());
    }

    #[test]
    fn test_parse_stash_list_single() {
        // Format: stash@{N}\x1f<oid>\x1f<message>
        let input = "stash@{0}\x1fabc1234def5678\x1fOn main: my stash";
        let stashes = parse_stash_list(input).unwrap();
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].index, 0);
        assert_eq!(stashes[0].oid, "abc1234def5678");
        assert_eq!(stashes[0].message, "On main: my stash");
    }

    #[test]
    fn test_parse_stash_list_multiple() {
        let input = concat!(
            "stash@{0}\x1faaaaaa\x1fOn main: newest\n",
            "stash@{1}\x1fbbbbbb\x1fOn feature: older\n",
            "stash@{2}\x1fcccccc\x1fWIP: oldest\n",
        );
        let stashes = parse_stash_list(input).unwrap();
        assert_eq!(stashes.len(), 3);

        assert_eq!(stashes[0].index, 0);
        assert_eq!(stashes[0].oid, "aaaaaa");
        assert_eq!(stashes[0].message, "On main: newest");

        assert_eq!(stashes[1].index, 1);
        assert_eq!(stashes[1].oid, "bbbbbb");
        assert_eq!(stashes[1].message, "On feature: older");

        assert_eq!(stashes[2].index, 2);
        assert_eq!(stashes[2].oid, "cccccc");
        assert_eq!(stashes[2].message, "WIP: oldest");
    }

    #[test]
    fn test_parse_stash_list_message_with_colon() {
        // Messages often contain colons; make sure we only split on \x1f.
        let input = "stash@{0}\x1fdeadbeef\x1fWIP: feat: some: message";
        let stashes = parse_stash_list(input).unwrap();
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].message, "WIP: feat: some: message");
    }

    #[test]
    fn test_parse_stash_list_blank_lines_ignored() {
        let input = "\nstash@{0}\x1f111111\x1ftest\n\n";
        let stashes = parse_stash_list(input).unwrap();
        assert_eq!(stashes.len(), 1);
    }

    #[test]
    fn test_parse_stash_index_valid() {
        assert_eq!(parse_stash_index("stash@{0}"), 0);
        assert_eq!(parse_stash_index("stash@{5}"), 5);
        assert_eq!(parse_stash_index("stash@{42}"), 42);
    }

    #[test]
    fn test_parse_stash_index_invalid() {
        // Unknown format falls back to 0.
        assert_eq!(parse_stash_index(""), 0);
        assert_eq!(parse_stash_index("not_a_stash"), 0);
    }

    #[test]
    fn test_shell_escape_no_quotes() {
        let escaped = shell_escape("/tmp/some/path.txt");
        assert_eq!(escaped, "'/tmp/some/path.txt'");
    }

    #[test]
    fn test_shell_escape_with_single_quote() {
        let escaped = shell_escape("path with 'quote'");
        assert_eq!(escaped, "'path with '\\''quote'\\'''");
    }
}
