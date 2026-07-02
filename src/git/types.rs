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
