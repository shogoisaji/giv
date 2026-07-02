/// Full SHA-1 / SHA-256 hash as a heap-allocated string.
pub type Oid = String;

// ─── References ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum RefKind {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RefName {
    pub name: String,
    pub kind: RefKind,
}

// ─── Commit ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Commit {
    pub id: Oid,
    pub short_id: String,
    pub parents: Vec<Oid>,
    pub summary: String,
    pub body: String,
    pub author_name: String,
    pub author_email: String,
    /// Unix timestamp (seconds since epoch).
    pub time: i64,
    pub refs: Vec<RefName>,
}

// ─── Working-tree status ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum StatusCode {
    Unmodified,
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Ignored,
    Conflicted,
    TypeChange,
}

#[derive(Debug, Clone)]
pub struct FileStatus {
    pub path: String,
    /// Original path before a rename/copy.
    pub orig_path: Option<String>,
    pub index: StatusCode,
    pub worktree: StatusCode,
}

impl FileStatus {
    pub fn is_staged(&self) -> bool {
        !matches!(
            self.index,
            StatusCode::Unmodified | StatusCode::Untracked | StatusCode::Ignored
        )
    }

    pub fn is_unstaged(&self) -> bool {
        !matches!(
            self.worktree,
            StatusCode::Unmodified | StatusCode::Untracked | StatusCode::Ignored
        )
    }

    pub fn is_conflicted(&self) -> bool {
        self.index == StatusCode::Conflicted || self.worktree == StatusCode::Conflicted
    }

    pub fn is_untracked(&self) -> bool {
        self.index == StatusCode::Untracked
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkingStatus {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub entries: Vec<FileStatus>,
}

// ─── Diff ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
    Header,
    Meta,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub old_path: String,
    pub new_path: String,
    pub is_binary: bool,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Default)]
pub struct Diff {
    pub files: Vec<FileDiff>,
}

impl Diff {
    /// Approximate heap footprint (bytes) of this diff — the sum of every
    /// hunk header and diff-line text length. Used by the commit-diff cache
    /// to bound memory usage; it deliberately ignores small per-`String` /
    /// per-`Vec` allocator overhead so it stays cheap to compute.
    pub fn estimated_size(&self) -> usize {
        self.files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .map(|h| h.header.len() + h.lines.iter().map(|l| l.text.len() + 1).sum::<usize>())
            .sum()
    }

    /// Total number of rendered rows this diff will produce in the diff panel:
    /// per file (1 header + 1 blank separator) + per hunk (1 header + body
    /// lines).  Binary files contribute 1 "cannot display" line instead of
    /// hunk bodies.  Used to clamp `diff_scroll` so wheel-scroll past the end
    /// is a no-op instead of an infinite render loop.
    pub fn total_render_lines(&self) -> usize {
        let mut count = 0usize;
        for file in &self.files {
            count += 1; // file header
            if file.is_binary {
                count += 1; // "(binary file — cannot display)"
            } else {
                for hunk in &file.hunks {
                    count += 1; // hunk header
                    count += hunk.lines.len(); // body
                }
            }
            count += 1; // blank separator
        }
        count
    }
}

// ─── Branches ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Branch {
    pub name: String,
    pub kind: RefKind,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub is_head: bool,
    pub target: Oid,
}

// ─── Worktrees ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub head: Oid,
    pub is_current: bool,
    pub is_bare: bool,
    pub is_locked: bool,
}

// ─── Stash ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Stash {
    pub index: usize,
    pub message: String,
    pub oid: Oid,
}

// ─── Tag ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub target: Oid,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_hunk(n_body: usize) -> Hunk {
        Hunk {
            header: "@@ -1,1 +1,N @@".into(),
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: n_body as u32,
            lines: (0..n_body)
                .map(|i| DiffLine {
                    kind: DiffLineKind::Added,
                    text: format!("line {i}"),
                })
                .collect(),
        }
    }

    #[test]
    fn total_render_lines_empty_diff_is_zero() {
        assert_eq!(Diff::default().total_render_lines(), 0);
    }

    #[test]
    fn total_render_lines_single_file_single_hunk() {
        // 1 file header + 1 hunk header + 10 body + 1 blank = 13
        let diff = Diff {
            files: vec![FileDiff {
                old_path: "a".into(),
                new_path: "a".into(),
                is_binary: false,
                hunks: vec![mk_hunk(10)],
            }],
        };
        assert_eq!(diff.total_render_lines(), 13);
    }

    #[test]
    fn total_render_lines_binary_file() {
        // 1 file header + 1 "(binary)" + 1 blank = 3
        let diff = Diff {
            files: vec![FileDiff {
                old_path: "a".into(),
                new_path: "a".into(),
                is_binary: true,
                hunks: vec![],
            }],
        };
        assert_eq!(diff.total_render_lines(), 3);
    }

    #[test]
    fn total_render_lines_multi_file_multi_hunk() {
        // file A: 1 + (1+5) + 1 = 8 ; file B: 1 + (1+3) + (1+2) + 1 = 9 → 17
        let diff = Diff {
            files: vec![
                FileDiff {
                    old_path: "a".into(),
                    new_path: "a".into(),
                    is_binary: false,
                    hunks: vec![mk_hunk(5)],
                },
                FileDiff {
                    old_path: "b".into(),
                    new_path: "b".into(),
                    is_binary: false,
                    hunks: vec![mk_hunk(3), mk_hunk(2)],
                },
            ],
        };
        assert_eq!(diff.total_render_lines(), 17);
    }
}
