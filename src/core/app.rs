use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::Context;

use crate::action::Action;
use crate::config::Config;
use crate::git::{
    Branch, Commit, Diff, GitBackend, OpInProgress, Stash, Tag, WorkingStatus, Worktree,
};
use crate::keymap::Keymap;
use crate::theme::Theme;

// ─── Mode ────────────────────────────────────────────────────────────────────

/// Top-level application mode (corresponds to a tab in the status bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Status,
    Graph,
    Branches,
    Worktrees,
    Stashes,
    /// Inspect an arbitrary commit by ref (sha / branch / HEAD~1 …).
    Inspect,
}

// ─── Panel ───────────────────────────────────────────────────────────────────

/// Which panel currently holds keyboard focus.
///
/// `Left` and `Main` are the two panes of the selected tab's two-pane layout
/// (Left = list, Main = detail/diff). `Middle` and `Right` are retained for the
/// legacy three-pane layout helpers but are not selected by the current policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Left,
    Main,
    Middle,
    Right,
}

// ─── Dialog ──────────────────────────────────────────────────────────────────

/// `Dialog` and `ConfirmOp` live in [`crate::core::dialog`]; they are re-exported
/// here so the existing `crate::app::{Dialog, ConfirmOp}` paths keep resolving.
pub use crate::core::dialog::{ConfirmOp, Dialog};

/// `InspectState` is owned by the Inspect feature; re-exported here so the
/// existing `crate::app::InspectState` path keeps resolving and the `App` model
/// can compose it.
pub use crate::features::inspect::state::InspectState;

// ─── Overlay state (palette / search) ─────────────────────────────────────────
//
// These overlays live in their own core modules; re-exported here so existing
// `crate::app::{PaletteItem, PaletteState, SearchState}` paths keep resolving.
pub use crate::core::palette::{PaletteItem, PaletteState};
pub use crate::core::search::SearchState;

// ─── Interactive rebase state ─────────────────────────────────────────────────
//
// `RebaseTodoEntry` / `RebaseTodoState` are owned by the Graph feature (next to
// the todo-editor view); re-exported here so the existing `crate::app::…` paths
// keep resolving and the `App` model can compose them.
pub use crate::features::graph::rebase_todo::{RebaseTodoEntry, RebaseTodoState};

// ─── UiState ─────────────────────────────────────────────────────────────────

/// Cursor/scroll positions for every view that needs them.
#[derive(Debug, Clone, Default)]
pub struct UiState {
    /// Focused panel within the current mode.
    pub focus: Option<Panel>,
    /// Selected index in the left-panel list (status / branch / etc.).
    pub list_index: usize,
    /// Scroll offset for the left-panel list.
    pub list_offset: usize,
    /// Number of visible rows in the left-panel list, recorded by the renderer
    /// each frame so navigation can auto-scroll to keep the selection visible
    /// (mirrors `graph_viewport`).
    pub list_viewport: Cell<usize>,
    /// Vertical scroll position of the diff view (in lines).
    pub diff_scroll: u16,
    /// Selected commit index in the graph view.
    pub graph_index: usize,
    /// Scroll offset for the graph list.
    pub graph_offset: usize,
    /// Selected index in the branches list.
    pub branch_index: usize,
    /// Scroll offset for the branches list (mouse-wheel view scroll).
    pub branch_offset: usize,
    /// Number of visible rows in the branches panel, recorded by the renderer
    /// each frame so navigation can auto-scroll to keep the selection visible.
    pub branch_viewport: Cell<usize>,
    /// Selected index in the worktrees list.
    pub worktree_index: usize,
    /// Scroll offset for the worktrees list (mouse-wheel view scroll).
    pub worktree_offset: usize,
    /// Number of visible rows in the worktrees panel, recorded by the renderer
    /// each frame so navigation can auto-scroll to keep the selection visible.
    pub worktree_viewport: Cell<usize>,
    /// Selected index in the stashes list.
    pub stash_index: usize,
    /// Scroll offset for the stashes list (mouse-wheel view scroll).
    pub stash_offset: usize,
    /// Number of visible rows in the stashes panel, recorded by the renderer
    /// each frame so navigation can auto-scroll to keep the selection visible.
    pub stash_viewport: Cell<usize>,
    /// Number of visible rows in the graph panel, recorded by the renderer each
    /// frame so navigation can auto-scroll to keep the selected commit visible.
    pub graph_viewport: Cell<usize>,
    /// Terminal width (columns) recorded by the renderer each frame for layout
    /// policy decisions.
    pub width: Cell<u16>,
    /// Width (percent) of the left graph panel in Graph mode. Adjustable with
    /// `<` / `>`. Clamped to [30, 80].
    pub graph_split: u16,
    /// Graph scope: `true` = all branches (`git log --all`), `false` = only the
    /// current HEAD's history. Toggled with `a` in Graph mode. Defaults to `true`
    /// (set in `App::new`) so the full branch topology is visible by default.
    pub graph_all_branches: bool,
    /// First-parent mode: follow only first parents so a merge-heavy trunk reads
    /// as one straight line (one row per merge). Defaults to `false`.
    pub graph_first_parent: bool,
    /// Branch lens: when `Some(tip_sha)`, the graph is filtered to the union of
    /// that commit's history and the main branch's — a clean two-lane view that
    /// converges at their fork point. Toggled with `f`. `None` = off.
    pub graph_focus: Option<String>,
    /// Last-rendered panel areas, recorded by the renderer each frame so the
    /// mouse handler can map a click coordinate to a panel + row. The rects are
    /// the INNER area (inside the border) of each pane.
    pub panel_rects: Cell<PanelRects>,
    /// Last-rendered mode-tab strip: the y row of the tab strip and the
    /// (start_x, end_x) range of each tab within that row. Clicking inside a
    /// tab's range switches to that tab. Recorded by `render_mode_tabs`.
    pub tab_strip: Cell<TabStrip>,
    /// Last-rendered command-palette item-list area (inner, inside borders).
    /// Used by the mouse handler to map a click row to a palette item index.
    /// `None` when the palette isn't rendered in the last frame.
    pub palette_list_rect: Cell<Option<ratatui::layout::Rect>>,
    /// Scroll offset of the palette item list in the last rendered frame —
    /// the first visible item index. Combined with `palette_list_rect` to map
    /// a click y coordinate to an absolute item index.
    pub palette_scroll: Cell<usize>,
}

/// Recorded tab strip geometry for mouse click detection. The tabs are
/// rendered left-to-right with variable widths (title + padding + divider),
/// so we store each tab's x range rather than assuming equal division.
#[derive(Debug, Default, Clone, Copy)]
pub struct TabStrip {
    /// The y row of the tab strip (inner area, inside borders).
    pub y: u16,
    /// (start_x, end_x) for each of the 6 tabs, in absolute terminal coords.
    /// `end_x` is exclusive. Empty entries mean the tab wasn't rendered.
    pub ranges: [Option<(u16, u16)>; 6],
}

/// Rendered panel areas, used by the mouse click handler to map a click
/// coordinate to a panel and a row within that panel. Only the panels that
/// were actually rendered in the last frame are `Some`.
#[derive(Debug, Default, Clone, Copy)]
pub struct PanelRects {
    /// Changes list in Status mode.
    pub changes: Option<ratatui::layout::Rect>,
    /// Graph list in Graph mode.
    pub graph: Option<ratatui::layout::Rect>,
    /// Diff panel in modes that show one.
    pub diff: Option<ratatui::layout::Rect>,
    /// Generic list panel for Branches / Worktrees / Stashes modes.
    pub other: Option<ratatui::layout::Rect>,
}

impl UiState {
    pub fn panel(&self) -> &Panel {
        self.focus.as_ref().unwrap_or(&Panel::Left)
    }

    /// The pane structure for the currently-recorded terminal width and the
    /// given mode. The width is recorded by the renderer each frame
    /// (`UiState::width`); before the first frame it is `0`.
    pub fn pane_layout(&self, mode: Mode) -> crate::ui::layout::PaneLayout {
        crate::ui::layout::pane_layout(self.width.get(), mode)
    }

    /// Record a rendered panel area so the mouse handler can map clicks to
    /// panels. Called by each panel renderer during `render_main`.
    pub fn record_rect(&self, slot: RectSlot, rect: ratatui::layout::Rect) {
        let mut rects = self.panel_rects.get();
        match slot {
            RectSlot::Changes => rects.changes = Some(rect),
            RectSlot::Graph => rects.graph = Some(rect),
            RectSlot::Diff => rects.diff = Some(rect),
            RectSlot::Other => rects.other = Some(rect),
        }
        self.panel_rects.set(rects);
    }

    /// Clear all recorded panel areas. Called at the start of each frame so
    /// stale rects from a previous layout (e.g. after a resize) don't linger.
    pub fn reset_rects(&self) {
        self.panel_rects.set(PanelRects::default());
        self.tab_strip.set(TabStrip::default());
        self.palette_list_rect.set(None);
        self.palette_scroll.set(0);
    }
}

/// Which panel slot to record a rect for.
#[derive(Debug, Clone, Copy)]
pub enum RectSlot {
    Changes,
    Graph,
    Diff,
    Other,
}

// ─── GraphCache ──────────────────────────────────────────────────────────────

/// Caches the laid-out commit graph so `build_graph` (an O(commits) pass with
/// many allocations) runs only when the commit set or render mode changes —
/// not on every frame / keypress.
///
/// Also caches the selection-derived values the renderer recomputes every frame
/// when the graph is focused: the branch highlight (an O(commits) lane replay),
/// main's ancestry set (an O(commits) BFS), and the branch-lens fork point
/// (two ancestry BFSes). Each is keyed separately so a cursor move only
/// invalidates the highlight (which depends on `graph_index`), not the
/// main-ancestry or fork caches.
#[derive(Default)]
pub struct GraphCache {
    /// Cache key: `(commit_count, newest_commit_id, spacious, first_parent, main_tip, head_tip)`.
    pub key: Option<(usize, String, bool, bool, String, String)>,
    pub rows: Vec<crate::features::graph::layout::GraphRow>,
    pub lane_layout: Option<crate::features::graph::layout::LaneLayout>,

    // ── Selection-derived caches (recomputed only when their key changes) ─────
    /// Branch-highlight cache. Key adds `graph_index` to the row key's
    /// commit-set identity (count + newest id + first_parent + main/head tips).
    pub hl: Option<crate::features::graph::layout::Highlight>,
    pub hl_key: Option<(usize, String, bool, String, String, usize)>,

    /// Main-branch ancestry set cache. Keyed by commit-set identity + main tip.
    pub main_ancestors: Option<std::collections::HashSet<String>>,
    pub main_ancestors_key: Option<(usize, String, String)>,

    /// Branch-lens fork point cache. Keyed by commit-set identity + the focused
    /// branch tip + the main base sha.
    pub fork: Option<String>,
    pub fork_key: Option<(usize, String, String, String)>,
}

// ─── RepoState ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffKey {
    Commit(String),
    Compare(String, String),
}

/// Live data fetched from the git backend.
pub struct RepoState {
    pub backend: Box<dyn GitBackend>,
    pub status: WorkingStatus,
    pub commits: Vec<Commit>,
    pub selected_diff: Option<Diff>,
    pub selected_diff_key: Option<DiffKey>,
    pub commit_diff_cache: HashMap<String, Diff>,
    pub commit_diff_order: VecDeque<String>,
}

// ─── App (Model) ─────────────────────────────────────────────────────────────

/// Central application model — the single source of truth.
pub struct App {
    pub mode: Mode,
    pub repo: RepoState,
    pub ui: UiState,
    pub config: Config,
    pub theme: Theme,
    pub keymap: Keymap,
    pub dialog: Dialog,
    /// Transient status / error message shown in the status bar.
    pub status_message: Option<String>,
    pub should_quit: bool,

    // ── Phase-2 fields ───────────────────────────────────────────────────────
    /// Sender half of the background-task channel; cloned into worker threads.
    pub task_tx: Sender<Action>,
    /// Receiver half of the background-task channel; drained each loop iteration.
    pub task_rx: Receiver<Action>,
    /// Label of the currently running background operation (for spinner display).
    pub running_task: Option<String>,

    /// Cached branch list (local + remote).
    pub branches: Vec<Branch>,
    /// Cached worktree list.
    pub worktrees: Vec<Worktree>,
    /// Cached tag list.
    pub tags: Vec<Tag>,

    /// Cached stash list.
    pub stashes: Vec<Stash>,

    /// Currently running git operation (merge/rebase/cherry-pick/revert), if any.
    pub op_in_progress: Option<OpInProgress>,

    /// If `Some(path)`, print this path to stdout after restoring the terminal
    /// so a shell wrapper can `cd` into it.
    pub print_cwd_on_exit: Option<String>,

    /// Interactive-rebase todo-editor state. `Some` when the overlay is open.
    pub rebase_todo: Option<RebaseTodoState>,

    // ── Phase 4 fields ───────────────────────────────────────────────────────
    /// Command palette overlay state. `Some` when the palette is open.
    pub palette: Option<PaletteState>,

    /// Incremental search state. `Some` when the search bar is open.
    pub search: Option<SearchState>,

    /// Whether the help overlay is currently visible.
    pub show_help: bool,

    /// Name of the currently active theme (kept in sync with `self.theme`).
    pub theme_name: String,

    /// Inspect-mode state: the commit resolved from an entered ref + its diff.
    pub inspect: InspectState,

    /// Cached commit-graph layout (rebuilt only when commits / mode change).
    pub graph_cache: RefCell<GraphCache>,

    /// Whether terminal mouse capture is currently enabled. Defaults to `false`
    /// so the terminal handles click-drag selection itself and the user can
    /// select & copy any on-screen text without holding Shift. Toggled with `M`
    /// (on = wheel-scroll / click-to-focus; off = native text selection).
    pub mouse_capture: bool,

    /// Branch-compare scope: `Some((base, target))` when the user has entered
    /// compare mode (Graph mode, `C` key). The graph shows only `base..target`
    /// commits and the diff panel shows `git diff base...target`. `Esc` clears
    /// this back to the normal scope.
    pub compare: Option<(String, String)>,

    pub pending_graph_diff: Option<String>,
}

impl App {
    /// Construct a new `App` and perform the initial data load.
    ///
    /// Errors from the initial `status()` / `log()` calls are stored as a
    /// `status_message` rather than propagating, so the app can still start
    /// against an empty / partially broken repo.
    pub fn new(backend: Box<dyn GitBackend>, config: Config) -> anyhow::Result<Self> {
        let theme_name = config.theme.clone();
        let theme = Theme::from_name(&theme_name);
        let keymap = Keymap;

        let status = WorkingStatus::default();
        let commits = Vec::new();

        let (task_tx, task_rx) = mpsc::channel::<Action>();

        let mut app = Self {
            mode: Mode::Status,
            repo: RepoState {
                backend,
                status,
                commits,
                selected_diff: None,
                selected_diff_key: None,
                commit_diff_cache: HashMap::new(),
                commit_diff_order: VecDeque::new(),
            },
            ui: UiState {
                focus: Some(Panel::Left),
                graph_split: 60,
                // Default to ALL branches (`git log --all`): unmerged branches —
                // especially a `dev`/integration branch where commits accumulate
                // before merging to main — must be visible without toggling, and
                // the current branch is pinned to a prominent column. `a` narrows
                // to the current branch's history only.
                graph_all_branches: true,
                ..UiState::default()
            },
            config,
            theme,
            keymap,
            dialog: Dialog::None,
            status_message: None,
            should_quit: false,
            task_tx,
            task_rx,
            running_task: None,
            branches: Vec::new(),
            worktrees: Vec::new(),
            tags: Vec::new(),
            stashes: Vec::new(),
            op_in_progress: None,
            print_cwd_on_exit: None,
            rebase_todo: None,
            palette: None,
            search: None,
            show_help: false,
            theme_name,
            graph_cache: RefCell::new(GraphCache::default()),
            inspect: InspectState::default(),
            // Mouse capture is ON on launch so click-to-jump and wheel-scroll
            // work immediately. Press `M` to toggle off for native text
            // selection (click-drag to copy).
            mouse_capture: true,
            compare: None,
            pending_graph_diff: None,
        };

        // Best-effort initial load — failures become status messages.
        if let Err(e) = app.refresh() {
            app.status_message = Some(format!("Warning: initial load failed: {e:#}"));
        }

        // Start the graph cursor on the current branch's tip (the HEAD commit) so
        // the branch you're working on is selected and scrolled on-screen the
        // moment you open the graph — not buried under newer commits from other
        // branches now that the default shows all of them.
        if let Some(i) = app.head_commit_index() {
            app.ui.graph_index = i;
        }

        Ok(app)
    }

    /// Index of the HEAD commit (the current branch's tip) within `repo.commits`,
    /// identified by its `HEAD` ref decoration. `None` when HEAD is outside the
    /// loaded window (e.g. a narrow scope that doesn't include it).
    pub fn head_commit_index(&self) -> Option<usize> {
        self.repo
            .commits
            .iter()
            .position(|c| c.refs.iter().any(|r| r.kind == crate::git::RefKind::Head))
    }

    /// Find the repository's integration branch — the first local `main`,
    /// `master`, or `trunk` — returning its name and target SHA. Used by the
    /// Branch lens as the base to compare a branch against.
    pub fn detect_main_branch(&self) -> Option<(String, String)> {
        for cand in ["main", "master", "trunk"] {
            if let Some(b) = self
                .branches
                .iter()
                .find(|b| b.kind == crate::git::RefKind::LocalBranch && b.name == cand)
            {
                return Some((b.name.clone(), b.target.clone()));
            }
        }
        None
    }

    /// Reload status, recent commits, branches, worktrees, and tags from the backend.
    pub fn refresh(&mut self) -> anyhow::Result<()> {
        self.repo.status = self
            .repo
            .backend
            .status()
            .context("failed to refresh working status")?;

        self.repo.commits = if let Some(tip) = self.ui.graph_focus.clone() {
            // Branch lens: union of the focused commit's history and main's.
            let base = self.detect_main_branch().map(|(name, _)| name);
            self.repo
                .backend
                .log_range(&tip, base.as_deref(), 512, self.ui.graph_first_parent)
                .context("failed to refresh commit log (branch lens)")?
        } else if let Some((base, target)) = self.compare.clone() {
            // Branch compare: only commits in `base..target`.
            self.repo
                .backend
                .log_between(&base, &target, 512, self.ui.graph_first_parent)
                .context("failed to refresh commit log (compare)")?
        } else {
            self.repo
                .backend
                .log(512, self.ui.graph_all_branches, self.ui.graph_first_parent)
                .context("failed to refresh commit log")?
        };

        // Best-effort loads: branches / worktrees / tags failures are non-fatal.
        match self.repo.backend.branches() {
            Ok(b) => self.branches = b,
            Err(e) => {
                tracing::warn!("failed to refresh branches: {e:#}");
            }
        }

        match self.repo.backend.worktrees() {
            Ok(w) => self.worktrees = w,
            Err(e) => {
                tracing::warn!("failed to refresh worktrees: {e:#}");
            }
        }

        match self.repo.backend.tags() {
            Ok(t) => self.tags = t,
            Err(e) => {
                tracing::warn!("failed to refresh tags: {e:#}");
            }
        }

        match self.repo.backend.stashes() {
            Ok(s) => self.stashes = s,
            Err(e) => {
                tracing::warn!("failed to refresh stashes: {e:#}");
            }
        }

        match self.repo.backend.operation_in_progress() {
            Ok(op) => self.op_in_progress = op,
            Err(e) => {
                tracing::warn!("failed to check operation in progress: {e:#}");
                self.op_in_progress = None;
                self.status_message =
                    Some(format!("Failed to check in-progress git operation: {e:#}"));
            }
        }

        // Drop cached commit diffs. SHAs are immutable so cached entries are
        // still *correct* after a refresh, but the commit set may have changed
        // (branch lens / compare / fetch) and we don't want stale entries from
        // a no-longer-visible history to occupy the LRU / byte budget.
        self.repo.commit_diff_cache.clear();
        self.repo.commit_diff_order.clear();

        Ok(())
    }
}

// `ConfirmOp::command_preview` tests live alongside the type in
// `crate::core::dialog`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::RefKind;
    use crate::test_backend::{mk_branch, mk_commit, MockBackend};
    use crate::ui::layout::PaneLayout;

    fn build_app(backend: MockBackend) -> App {
        App::new(Box::new(backend), Config::default()).expect("app builds")
    }

    // ── head_commit_index ────────────────────────────────────────────────────

    #[test]
    fn head_commit_index_finds_head_decoration() {
        let mut b = MockBackend::new();
        b.commits = vec![
            mk_commit("aaa", "newest", true),
            mk_commit("bbb", "older", false),
        ];
        let app = build_app(b);
        assert_eq!(app.head_commit_index(), Some(0));
    }

    #[test]
    fn head_commit_index_none_when_no_head_ref() {
        let mut b = MockBackend::new();
        b.commits = vec![
            mk_commit("aaa", "no head", false),
            mk_commit("bbb", "no head", false),
        ];
        let app = build_app(b);
        assert_eq!(app.head_commit_index(), None);
    }

    #[test]
    fn head_commit_index_none_for_empty_commits() {
        let b = MockBackend::new();
        let app = build_app(b);
        assert_eq!(app.head_commit_index(), None);
    }

    #[test]
    fn head_commit_index_finds_head_at_non_zero_index() {
        let mut b = MockBackend::new();
        b.commits = vec![
            mk_commit("aaa", "other branch tip", false),
            mk_commit("bbb", "head", true),
            mk_commit("ccc", "older", false),
        ];
        let app = build_app(b);
        assert_eq!(app.head_commit_index(), Some(1));
    }

    // ── refresh clears commit-diff cache ──────────────────────────────────────

    #[test]
    fn refresh_clears_commit_diff_cache() {
        use crate::features::graph::update::insert_commit_diff_cache;
        use crate::git::Diff;

        let mut b = MockBackend::new();
        b.commits = vec![mk_commit("aaa", "head", true)];
        let mut app = build_app(b);
        // Seed the cache with a couple of entries.
        insert_commit_diff_cache(&mut app, "aaa".into(), Diff::default());
        insert_commit_diff_cache(&mut app, "bbb".into(), Diff::default());
        assert_eq!(app.repo.commit_diff_cache.len(), 2);
        assert_eq!(app.repo.commit_diff_order.len(), 2);

        app.refresh().expect("refresh succeeds");

        assert!(app.repo.commit_diff_cache.is_empty());
        assert!(app.repo.commit_diff_order.is_empty());
    }

    // ── detect_main_branch ───────────────────────────────────────────────────

    #[test]
    fn detect_main_branch_finds_main() {
        let mut b = MockBackend::new();
        b.branches = vec![
            mk_branch("dev", "abc"),
            mk_branch("main", "def"),
            mk_branch("master", "ghi"),
        ];
        let app = build_app(b);
        assert_eq!(
            app.detect_main_branch(),
            Some(("main".into(), "def".into()))
        );
    }

    #[test]
    fn detect_main_branch_falls_back_to_master() {
        let mut b = MockBackend::new();
        b.branches = vec![mk_branch("dev", "abc"), mk_branch("master", "ghi")];
        let app = build_app(b);
        assert_eq!(
            app.detect_main_branch(),
            Some(("master".into(), "ghi".into()))
        );
    }

    #[test]
    fn detect_main_branch_falls_back_to_trunk() {
        let mut b = MockBackend::new();
        b.branches = vec![mk_branch("trunk", "xyz")];
        let app = build_app(b);
        assert_eq!(
            app.detect_main_branch(),
            Some(("trunk".into(), "xyz".into()))
        );
    }

    #[test]
    fn detect_main_branch_none_when_no_known_names() {
        let mut b = MockBackend::new();
        b.branches = vec![mk_branch("dev", "abc"), mk_branch("feature/x", "def")];
        let app = build_app(b);
        assert_eq!(app.detect_main_branch(), None);
    }

    #[test]
    fn detect_main_branch_none_for_empty_branches() {
        let b = MockBackend::new();
        let app = build_app(b);
        assert_eq!(app.detect_main_branch(), None);
    }

    #[test]
    fn detect_main_branch_ignores_remote_main() {
        // A remote-tracking "origin/main" should NOT be detected — only local.
        let mut b = MockBackend::new();
        b.branches = vec![Branch {
            name: "main".into(),
            kind: RefKind::RemoteBranch,
            upstream: None,
            ahead: 0,
            behind: 0,
            is_head: false,
            target: "def".into(),
        }];
        let app = build_app(b);
        assert_eq!(app.detect_main_branch(), None);
    }

    // ── UiState helpers ──────────────────────────────────────────────────────

    #[test]
    fn uistate_panel_defaults_to_left_when_no_focus() {
        let ui = UiState::default();
        assert_eq!(ui.panel(), &Panel::Left);
    }

    #[test]
    fn uistate_panel_returns_focused_panel() {
        let ui = UiState {
            focus: Some(Panel::Main),
            ..UiState::default()
        };
        assert_eq!(ui.panel(), &Panel::Main);
    }

    #[test]
    fn uistate_pane_layout_for_status_narrow_is_two_pane() {
        let ui = UiState::default();
        // width 0 → narrow → two-pane for Status
        let layout = ui.pane_layout(Mode::Status);
        assert_eq!(layout, PaneLayout::TwoPane);
    }

    #[test]
    fn uistate_pane_layout_for_graph_wide_is_two_pane() {
        let ui = UiState::default();
        ui.width.set(200);
        let layout = ui.pane_layout(Mode::Graph);
        assert_eq!(layout, PaneLayout::TwoPane);
    }

    #[test]
    fn uistate_reset_rects_clears_all_slots() {
        let ui = UiState::default();
        // Record something first.
        ui.record_rect(RectSlot::Changes, ratatui::layout::Rect::new(0, 0, 10, 10));
        assert!(ui.panel_rects.get().changes.is_some());
        ui.reset_rects();
        let rects = ui.panel_rects.get();
        assert!(rects.changes.is_none());
        assert!(rects.graph.is_none());
        assert!(rects.diff.is_none());
        assert!(rects.other.is_none());
    }

    #[test]
    fn uistate_record_rect_stores_each_slot() {
        let ui = UiState::default();
        let r1 = ratatui::layout::Rect::new(1, 2, 3, 4);
        let r2 = ratatui::layout::Rect::new(5, 6, 7, 8);
        ui.record_rect(RectSlot::Graph, r1);
        ui.record_rect(RectSlot::Diff, r2);
        let rects = ui.panel_rects.get();
        assert_eq!(rects.graph, Some(r1));
        assert_eq!(rects.diff, Some(r2));
        assert!(rects.changes.is_none());
        assert!(rects.other.is_none());
    }
}
