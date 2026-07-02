/// Commit-graph lane-assignment & compaction engine.
///
/// The renderer walks commits newest-first and maintains a *lane table*
/// `active: Vec<Option<Lane>>` where each slot holds the commit OID currently
/// flowing down that column (the next commit expected in that lane).
///
/// Design choices that give the graph its readability:
///
///   1. **Stable columns.** A lane never changes column once placed, so a branch
///      reads as a straight vertical line from tip to merge. Freed columns are
///      reclaimed by trimming trailing gaps and by letting future branch-outs
///      reuse interior gaps — so the graph stays compact WITHOUT sliding lanes
///      sideways (which produced a distracting left/right "zigzag").
///   2. **Diagonals only at real branch/merge points.** Rounded corners
///      (`╭ ╮ ╰ ╯`) and junctions appear only where a commit actually branches
///      out or merges in — not on inter-commit connector rows, which stay
///      straight `│`.
///   3. **Crossings duck under.** Where an unrelated merge connector must cross
///      a vertical lane, the lane stays continuous and the horizontal passes
///      behind it (`─│─`) instead of an ambiguous `┼`.
///
/// Every cell's glyph is derived from its up/down/left/right connections, so
/// merges, crossings and junctions all fall out of the same small rule table.
/// Node glyphs are differentiated: `◉` merge, `◆` tagged, `●` ordinary.
use crate::git::types::{Commit, Oid, RefKind};
use smallvec::SmallVec;

/// Inline-storage Vec for edge OID lists.  The vast majority of graph cells
/// carry 0–2 edges, so `SmallVec<[Oid; 2]>` keeps them on the stack without a
/// heap allocation — a significant saving in the row-builder hot path where
/// cells are cloned by the hundreds per frame.
type EdgeOids = SmallVec<[Oid; 2]>;

// ─── GraphCell / GraphRow ──────────────────────────────────────────────────────

/// A single cell in the graph grid.
///
/// `symbol` is one of: `●  ◉  ◆  │  ─  ╭  ╮  ╰  ╯  ├  ┤  ┬  ┴  ┼  ' '`
/// `lane` is a stable color index into `Theme::lane` (wraps cyclically).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCell {
    /// Default glyph (used when no selection highlight is active).
    pub symbol: char,
    /// Index into `Theme::lane` (wraps cyclically).
    pub lane: usize,
    /// Direction bits present at this cell: `1`=up, `2`=down, `4`=left, `8`=right.
    /// Lets the renderer recompute the glyph giving the selected branch priority
    /// (e.g. drop an unrelated crossing so `┴` becomes a clean `╯`).
    pub dirs: u8,
    /// OID of the vertical lane through this cell (the lane's head commit).
    pub vertical_oid: Option<Oid>,
    /// OIDs of horizontal edges touching this cell's LEFT side. A single
    /// connector cell can be shared by several edges (e.g. one branch's merge-in
    /// AND another branch's branch-out both traverse the same gap), so a
    /// selection lights the cell if ANY of its edges belongs to the branch.
    pub left_edge_oids: EdgeOids,
    /// OIDs of horizontal edges touching this cell's RIGHT side.
    pub right_edge_oids: EdgeOids,
}

pub(crate) const UP: u8 = 1;
pub(crate) const DOWN: u8 = 2;
pub(crate) const LEFT: u8 = 4;
pub(crate) const RIGHT: u8 = 8;

impl GraphCell {
    fn empty() -> Self {
        Self {
            symbol: ' ',
            lane: 0,
            dirs: 0,
            vertical_oid: None,
            left_edge_oids: EdgeOids::new(),
            right_edge_oids: EdgeOids::new(),
        }
    }

    /// Whether this cell carries any segment of the given lineage set — used to
    /// decide whether it should stay vivid (vs dimmed) under a selection.
    pub fn in_lineage(&self, set: &std::collections::HashSet<Oid>) -> bool {
        self.vertical_oid
            .as_deref()
            .map(|x| set.contains(x))
            .unwrap_or(false)
            || self.left_edge_oids.iter().any(|x| set.contains(x))
            || self.right_edge_oids.iter().any(|x| set.contains(x))
    }
}

/// One rendered row of the commit graph.
///
/// For node rows `commit_index` is a valid index into `commits`.
/// For edge-only rows (inter-commit connectors) `commit_index == usize::MAX`.
#[derive(Debug, Clone)]
pub struct GraphRow {
    pub commit_index: usize,
    pub cells: Vec<GraphCell>,
    pub is_node_row: bool,
}

// ─── Lane ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Lane {
    /// OID this lane is currently flowing toward (the next commit in the lane).
    oid: Oid,
    /// Stable color index for the lane's whole life.
    color: usize,
    /// OID of the most recent commit *node* in this lane (the lane's "head").
    /// Cells are tagged with this — not the target — so a first-parent highlight
    /// can tell whose branch a vertical segment belongs to even where two lanes
    /// both flow toward the same shared ancestor.
    head: Oid,
}

const NUM_COLORS: usize = 5;

/// Pick a color index for a new lane at `col`, avoiding the colors of the
/// immediate left and right neighbors so adjacent lanes stay distinguishable.
///
/// When `reserve0` is set, color index 0 is held back for the main spine, so a
/// feature lane never accidentally shares main's anchor color — main stays a
/// single recognizable backbone hue and side branches rotate through the rest.
fn pick_color(active: &[Option<Lane>], col: usize, seed: usize, reserve0: bool) -> usize {
    let left = col
        .checked_sub(1)
        .and_then(|i| active.get(i))
        .and_then(|o| o.as_ref())
        .map(|l| l.color);
    let right = active
        .get(col + 1)
        .and_then(|o| o.as_ref())
        .map(|l| l.color);

    let start = if reserve0 { 1 } else { 0 };
    let span = NUM_COLORS - start;
    for k in 0..span {
        let cand = start + (seed + k) % span;
        if Some(cand) != left && Some(cand) != right {
            return cand;
        }
    }
    start + seed % span
}

/// Find the leftmost free (`None`) slot at index `>= min`, growing the table if
/// necessary. Returns the chosen index (left as `None` for the caller to fill).
fn alloc<T>(active: &mut Vec<Option<T>>, min: usize) -> usize {
    let mut i = min;
    while i < active.len() {
        if active[i].is_none() {
            return i;
        }
        i += 1;
    }
    active.push(None);
    active.len() - 1
}

/// Like [`alloc`] but the returned index is GUARANTEED `>= min` (it grows the
/// table with empty slots when it must), so a reserved low column can never be
/// handed out even when the table is currently shorter than `min`.
fn alloc_at_least<T>(active: &mut Vec<Option<T>>, min: usize) -> usize {
    let mut i = min;
    loop {
        if i >= active.len() {
            active.resize_with(i + 1, || None);
        }
        if active[i].is_none() {
            return i;
        }
        i += 1;
    }
}

/// Choose the column for a NEW branch-tip lane, honoring RESERVED columns.
///
/// `reserved` is a priority-ordered list of tip OIDs that own fixed leftmost
/// columns — by convention `[main_tip, head_tip]`, so main is the stable col-0
/// backbone and the current branch (e.g. an unmerged `dev` integration branch) is
/// pinned to col-1 right beside it. A reserved tip claims its exact rank column;
/// every other tip opens to the RIGHT of the whole reserved block. This keeps the
/// branches that matter (main + what you're working on) in stable, prominent
/// columns no matter where their tips fall in the walk, and stops an ephemeral
/// feature from displacing them.
fn alloc_reserved_tip<T>(
    active: &mut Vec<Option<T>>,
    rank: Option<usize>,
    reserved_count: usize,
) -> usize {
    if let Some(r) = rank {
        if r >= active.len() {
            active.resize_with(r + 1, || None);
        }
        if active[r].is_none() {
            return r;
        }
        // Already taken (shouldn't happen for a unique tip) — fall through and
        // open to the right of the reserved block instead.
    }
    alloc_at_least(active, reserved_count)
}

/// Build the priority-ordered reservation list from the main and current-branch
/// tips: `[main, head]` with `head` dropped when it equals `main`. The first
/// entry owns column 0 (the backbone), the second column 1.
fn reserved_tips<'a>(main_tip: Option<&'a str>, head_tip: Option<&'a str>) -> Vec<&'a str> {
    let mut v = Vec::new();
    if let Some(m) = main_tip {
        v.push(m);
    }
    if let Some(h) = head_tip {
        if Some(h) != main_tip {
            v.push(h);
        }
    }
    v
}

// ─── Glyph rule table ──────────────────────────────────────────────────────────

/// Resolve a box-drawing glyph from its four connection directions.
fn glyph(up: bool, down: bool, left: bool, right: bool) -> char {
    match (up, down, left, right) {
        (true, true, false, false) => '│',
        (true, false, false, false) => '│',
        (false, true, false, false) => '│',
        (false, false, true, true) => '─',
        (false, false, true, false) => '─',
        (false, false, false, true) => '─',
        (true, true, true, true) => '┼',
        (true, true, false, true) => '├',
        (true, true, true, false) => '┤',
        (false, true, true, true) => '┬',
        (true, false, true, true) => '┴',
        (false, true, false, true) => '╭',
        (false, true, true, false) => '╮',
        (true, false, false, true) => '╰',
        (true, false, true, false) => '╯',
        _ => ' ',
    }
}

// ─── RowBuilder ────────────────────────────────────────────────────────────────

/// Accumulates per-lane connections for one row, then resolves them into cells.
///
/// Lane `c` lives at cell column `2*c`; the connector between lane `c` and
/// `c+1` lives at cell column `2*c+1` (so lanes get one column of horizontal
/// breathing room without a separate gap-fill pass).
struct RowBuilder {
    w: usize,
    up: Vec<bool>,
    down: Vec<bool>,
    /// `hor[c]` = a horizontal segment crosses between lane `c` and lane `c+1`.
    hor: Vec<bool>,
    /// Per-lane vertical color.
    col: Vec<Option<usize>>,
    /// Per-connector horizontal color.
    hcol: Vec<Option<usize>>,
    /// Node glyph + color override at a lane column.
    node: Vec<Option<(char, usize)>>,
    /// Per-lane vertical OID (the commit the lane flows toward).
    vertical_oid: Vec<Option<Oid>>,
    /// Per-connector horizontal OIDs (a connector can carry several edges).
    horizontal_edge_oids: Vec<EdgeOids>,
    /// Per-node OID.
    node_oids: Vec<Option<Oid>>,
}

impl RowBuilder {
    fn new(w: usize) -> Self {
        let conn = w.saturating_sub(1);
        Self {
            w,
            up: vec![false; w],
            down: vec![false; w],
            hor: vec![false; conn],
            col: vec![None; w],
            hcol: vec![None; conn],
            node: vec![None; w],
            vertical_oid: vec![None; w],
            horizontal_edge_oids: vec![EdgeOids::new(); conn],
            node_oids: vec![None; w],
        }
    }

    fn vert(&mut self, c: usize, color: usize, up: bool, down: bool, oid: &Oid) {
        if up {
            self.up[c] = true;
        }
        if down {
            self.down[c] = true;
        }
        self.col[c] = Some(color);
        self.vertical_oid[c] = Some(oid.clone());
    }

    fn horiz(&mut self, a: usize, b: usize, color: usize, oid: &Oid) {
        let (lo, hi) = (a.min(b), a.max(b));
        for k in lo..hi {
            self.hor[k] = true;
            if self.hcol[k].is_none() {
                self.hcol[k] = Some(color);
            }
            if !self.horizontal_edge_oids[k].contains(oid) {
                self.horizontal_edge_oids[k].push(oid.clone());
            }
        }
    }

    fn node(&mut self, c: usize, g: char, color: usize, up: bool, down: bool, oid: &Oid) {
        self.node[c] = Some((g, color));
        self.node_oids[c] = Some(oid.clone());
        if up {
            self.up[c] = true;
        }
        if down {
            self.down[c] = true;
        }
    }

    fn finish(self) -> Vec<GraphCell> {
        if self.w == 0 {
            return Vec::new();
        }
        let width_cells = 2 * self.w - 1;
        let mut cells = vec![GraphCell::empty(); width_cells];

        for c in 0..self.w {
            let idx = 2 * c;
            let u = self.up[c];
            let d = self.down[c];
            let l = c > 0 && self.hor.get(c - 1).copied().unwrap_or(false);
            let r = self.hor.get(c).copied().unwrap_or(false);
            let dirs =
                ((u as u8) * UP) | ((d as u8) * DOWN) | ((l as u8) * LEFT) | ((r as u8) * RIGHT);
            let left_edge_oids = if l {
                c.checked_sub(1)
                    .and_then(|i| self.horizontal_edge_oids.get(i).cloned())
                    .unwrap_or_default()
            } else {
                EdgeOids::new()
            };
            let right_edge_oids = if r {
                self.horizontal_edge_oids
                    .get(c)
                    .cloned()
                    .unwrap_or_default()
            } else {
                EdgeOids::new()
            };

            if let Some((g, color)) = self.node[c] {
                cells[idx] = GraphCell {
                    symbol: g,
                    lane: color,
                    dirs,
                    vertical_oid: self.node_oids[c].clone(),
                    left_edge_oids,
                    right_edge_oids,
                };
                continue;
            }

            // Default glyph. True crossing: a continuous vertical lane (up+down)
            // overlaid by an UNRELATED horizontal edge (different color) renders
            // the vertical dominant ("─│─") instead of an ambiguous `┼`. The
            // selection-aware renderer can still flip this per the highlight.
            let mut symbol = ' ';
            let mut lane = 0usize;
            if u && d && l && r {
                let vcol = self.col[c];
                let hcol = self.hcol.get(c).copied().flatten().or_else(|| {
                    c.checked_sub(1)
                        .and_then(|i| self.hcol.get(i).copied().flatten())
                });
                if let (Some(v), Some(h)) = (vcol, hcol) {
                    if v != h {
                        symbol = '│';
                        lane = v;
                    }
                }
            }
            if symbol == ' ' {
                let sym = glyph(u, d, l, r);
                if sym != ' ' {
                    let color = self.col[c]
                        .or_else(|| {
                            if r {
                                self.hcol.get(c).copied().flatten()
                            } else {
                                None
                            }
                        })
                        .or_else(|| {
                            if l {
                                self.hcol.get(c - 1).copied().flatten()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    symbol = sym;
                    lane = color;
                }
            }
            if symbol != ' ' {
                cells[idx] = GraphCell {
                    symbol,
                    lane,
                    dirs,
                    vertical_oid: self.vertical_oid[c].clone(),
                    left_edge_oids,
                    right_edge_oids,
                };
            }
        }

        for c in 0..self.w.saturating_sub(1) {
            if self.hor[c] {
                cells[2 * c + 1] = GraphCell {
                    symbol: '─',
                    lane: self.hcol[c].unwrap_or(0),
                    dirs: LEFT | RIGHT,
                    vertical_oid: None,
                    left_edge_oids: self.horizontal_edge_oids[c].clone(),
                    right_edge_oids: self.horizontal_edge_oids[c].clone(),
                };
            }
        }

        cells
    }
}

/// Resolve a cell's glyph and whether it is dimmed under a selection lineage.
///
/// The GLYPH is never changed — a through-lane stays continuous and an unrelated
/// branch is never broken to favour the selection (that produced messy, mis-
/// coloured lines that cut across other branches). Only the COLOUR changes: a
/// cell is vivid iff its dominant lane belongs to the selected branch, otherwise
/// it is dimmed. The selected branch's own connectors simply duck under the
/// branches they cross — standard, unambiguous graph rendering. Returns
/// `(glyph, dim)`.
pub fn cell_glyph(cell: &GraphCell, hl: Option<&Highlight>) -> (char, bool) {
    let Some(hl) = hl else {
        return (cell.symbol, false);
    };
    // A NODE keeps its glyph; it glows if it is in `nodes` (interior + fork/merge
    // boundaries).
    if matches!(cell.symbol, '●' | '◉' | '◆') {
        let on = cell
            .vertical_oid
            .as_deref()
            .map(|o| hl.nodes.contains(o))
            .unwrap_or(false);
        return (cell.symbol, !on);
    }
    // Line cells: which of the selected branch's directions pass through here?
    // (`lanes` = interior commits, so the boundary trunk stays dim.)
    let vsel = cell
        .vertical_oid
        .as_deref()
        .map(|o| hl.lanes.contains(o))
        .unwrap_or(false);
    let lsel = cell.left_edge_oids.iter().any(|o| hl.lanes.contains(o));
    let rsel = cell.right_edge_oids.iter().any(|o| hl.lanes.contains(o));
    if !(vsel || lsel || rsel) {
        return (cell.symbol, true); // unrelated lane → default glyph, dimmed
    }
    // The selected branch passes through this cell → draw it with TOP PRIORITY:
    // rebuild the glyph from ONLY its directions, so it stays a straight line and
    // the unrelated lane it crosses is segmented (`─│─` becomes `───`, the
    // crossed vertical ducks under). This is what the user asked for.
    let u = cell.dirs & UP != 0 && vsel;
    let d = cell.dirs & DOWN != 0 && vsel;
    let l = cell.dirs & LEFT != 0 && lsel;
    let r = cell.dirs & RIGHT != 0 && rsel;
    let g = glyph(u, d, l, r);
    (if g == ' ' { cell.symbol } else { g }, false)
}

// ─── Main algorithm ───────────────────────────────────────────────────────────

/// Select the node glyph for a commit: `◉` merge, `◆` tagged, `●` ordinary.
fn node_glyph(c: &Commit) -> char {
    if c.parents.len() >= 2 {
        '◉'
    } else if c.refs.iter().any(|r| r.kind == RefKind::Tag) {
        '◆'
    } else {
        '●'
    }
}

/// Compute the lineage of the commit at `selected`: that commit plus every
/// ancestor reachable by following parent links within `commits`. Used to
/// highlight a selected commit's branch history (and dim everything else).
/// Parent OIDs that fall outside the loaded window are simply not traversed.
pub fn ancestors(commits: &[Commit], selected: usize) -> std::collections::HashSet<Oid> {
    use std::collections::{HashMap, HashSet, VecDeque};
    let mut set: HashSet<Oid> = HashSet::new();
    let Some(start) = commits.get(selected) else {
        return set;
    };
    let by_id: HashMap<&str, &Commit> = commits.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut queue: VecDeque<Oid> = VecDeque::new();
    set.insert(start.id.clone());
    queue.push_back(start.id.clone());
    while let Some(id) = queue.pop_front() {
        if let Some(c) = by_id.get(id.as_str()) {
            for p in &c.parents {
                if set.insert(p.clone()) {
                    queue.push_back(p.clone());
                }
            }
        }
    }
    set
}

/// Compute the *first-parent* lineage of the commit at `selected`: that commit
/// and the chain reached by following only first parents to the root. Unlike
/// [`ancestors`], this does NOT balloon into merged-in side branches — so it is
/// the right set for highlighting "the branch line the selected commit sits on"
/// (selecting a merge commit or a trunk tip highlights the trunk spine, not
/// every branch ever merged into it).
pub fn first_parent_lineage(commits: &[Commit], selected: usize) -> std::collections::HashSet<Oid> {
    use std::collections::{HashMap, HashSet};
    let mut set: HashSet<Oid> = HashSet::new();
    let Some(start) = commits.get(selected) else {
        return set;
    };
    let by_id: HashMap<&str, &Commit> = commits.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut id = start.id.clone();
    loop {
        if !set.insert(id.clone()) {
            break; // cycle guard
        }
        match by_id.get(id.as_str()).and_then(|c| c.parents.first()) {
            Some(p) => id = p.clone(),
            None => break,
        }
    }
    set
}

/// The lane each commit sits in, plus each lane's birth/death, computed by
/// replaying the EXACT column bookkeeping of [`build_graph_opts`]. A "lane" is
/// one continuous column-life: it is born where a branch first appears (a tip,
/// or a merge's side parent) and dies where it rejoins another lane (its fork
/// into the trunk) or hits the root. Because it mirrors the renderer 1:1, the
/// set of commits sharing a lane id is EXACTLY the straight vertical line the
/// graph draws — which is what "the branch a commit is on" means visually.
#[derive(Debug, Clone, Default)]
pub struct LaneLayout {
    /// Lane id for each commit (parallel to `commits`).
    pub lane_of: Vec<usize>,
    /// Per lane id: `Some(merge_idx)` if a merge opened it for a side parent
    /// (that merge is the branch's END / absorbing merge); `None` if it began as
    /// a branch tip / root (no absorbing merge — unmerged, or the trunk).
    creator: Vec<Option<usize>>,
    /// Per lane id: `Some(commit_idx)` where the lane rejoined another lane (its
    /// FORK back into the trunk); `None` if still open at the window edge.
    terminator: Vec<Option<usize>>,
}

/// Replay [`build_graph_opts`]'s lane allocation to label every commit with the
/// continuous column (lane) it occupies. See [`LaneLayout`]. `first_parent` must
/// match the flag the graph was built with so the lanes line up with the render.
pub fn compute_lanes(commits: &[Commit], first_parent: bool) -> LaneLayout {
    compute_lanes_main(commits, first_parent, None, None)
}

/// Like [`compute_lanes`] but with the same column reservation as
/// [`build_graph_main`]: `main_tip` owns column 0 and `head_tip` (the current
/// branch) owns column 1. These MUST match what the graph was rendered with, or
/// a selection highlight will light the wrong column.
pub fn compute_lanes_main(
    commits: &[Commit],
    first_parent: bool,
    main_tip: Option<&str>,
    head_tip: Option<&str>,
) -> LaneLayout {
    let reserved = reserved_tips(main_tip, head_tip);
    let mut active: Vec<Option<(Oid, usize)>> = Vec::new();
    let mut lane_of = vec![0usize; commits.len()];
    let mut creator: Vec<Option<usize>> = Vec::new();
    let mut terminator: Vec<Option<usize>> = Vec::new();

    // Reusable OID → lane-columns index (see build_graph_main for rationale).
    // Keys are owned (cloned) so the map does NOT borrow `active`, allowing
    // `active` to be mutated freely after the lookups are extracted.
    let mut oid_to_lanes: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();

    for (idx, commit) in commits.iter().enumerate() {
        let cid = &commit.id;

        // ── locate the commit's lane (leftmost arriving, else open a tip lane) ──
        oid_to_lanes.clear();
        for (i, slot) in active.iter().enumerate() {
            if let Some((o, _)) = slot {
                oid_to_lanes.entry(o.clone()).or_default().push(i);
            }
        }
        let arriving: Vec<usize> = oid_to_lanes.get(cid).cloned().unwrap_or_default();

        // Pre-compute which extra parents already have a lane.
        let extra: &[Oid] = if first_parent {
            &[]
        } else {
            &commit.parents[1.min(commit.parents.len())..]
        };
        let parent_has_lane: Vec<bool> =
            extra.iter().map(|p| oid_to_lanes.contains_key(p)).collect();

        let commit_pos = if let Some(&first) = arriving.first() {
            first
        } else {
            let rank = reserved.iter().position(|&x| x == cid.as_str());
            let pos = alloc_reserved_tip(&mut active, rank, reserved.len());
            let lid = creator.len();
            creator.push(None);
            terminator.push(None);
            active[pos] = Some((cid.clone(), lid));
            pos
        };
        // `commit_pos` was either an existing arriving lane (which is `Some`)
        // or a freshly allocated slot set to `Some` above, so this is always
        // `Some` in practice — but use `unwrap_or` for defensive safety.
        let lid = active[commit_pos].as_ref().map(|(_, l)| *l).unwrap_or(0);
        lane_of[idx] = lid;

        // ── open columns for extra (merge) parents ──────────────────────────────
        for (parent, &already) in extra.iter().zip(parent_has_lane.iter()) {
            if !already {
                let col = alloc(&mut active, commit_pos + 1);
                let nlid = creator.len();
                creator.push(Some(idx)); // this merge is the side branch's END
                terminator.push(None);
                active[col] = Some((parent.clone(), nlid));
            }
        }

        // ── advance: terminate merged-in lanes, continue this one to its 1st parent ──
        for &col in &arriving {
            if col != commit_pos {
                if let Some((_, l)) = active[col].take() {
                    terminator[l] = Some(idx); // rejoined here → this commit is its fork
                }
            }
        }
        match commit.parents.first() {
            Some(p) => active[commit_pos] = Some((p.clone(), lid)),
            None => {
                terminator[lid] = Some(idx);
                active[commit_pos] = None;
            }
        }

        while matches!(active.last(), Some(None)) {
            active.pop();
        }
    }

    LaneLayout {
        lane_of,
        creator,
        terminator,
    }
}

/// The set of commits to highlight for a selection, split so the renderer can
/// treat boundaries correctly: `lanes` are the branch's INTERIOR commits (their
/// vertical lane segments + connectors should glow); `nodes` is `lanes` PLUS the
/// fork and merge boundary commits (their NODES glow as the branch's start/end,
/// but their mainline lane stays dim so the trunk between fork and merge isn't
/// lit up).
#[derive(Debug, Clone, Default)]
pub struct Highlight {
    pub nodes: std::collections::HashSet<Oid>,
    pub lanes: std::collections::HashSet<Oid>,
}

/// Highlight the BRANCH the selected commit is on — defined as the continuous
/// rendered lane it sits in (see [`compute_lanes`]). This is the straight
/// vertical line the graph already draws through the commit: from where the
/// branch forked off the trunk (START) up to where it was absorbed by a merge
/// (END) — spanning any number of intermediate merges of the same line — or to
/// its tip if unmerged. Sibling branches that forked off the same commit, and
/// unrelated merges, keep their own lanes and stay dim.
///
/// `first_parent` must match the flag the graph was built with.
pub fn branch_highlight(commits: &[Commit], selected: usize, first_parent: bool) -> Highlight {
    branch_highlight_main(commits, selected, first_parent, None, None)
}

/// Like [`branch_highlight`] but using the same column reservation the graph was
/// rendered with (see [`compute_lanes_main`]). The view passes the main and
/// current-branch tips so the highlighted lane lines up with the drawn column.
pub fn branch_highlight_main(
    commits: &[Commit],
    selected: usize,
    first_parent: bool,
    main_tip: Option<&str>,
    head_tip: Option<&str>,
) -> Highlight {
    let layout = compute_lanes_main(commits, first_parent, main_tip, head_tip);
    branch_highlight_from_lanes(commits, &layout, selected)
}

pub fn branch_highlight_from_lanes(
    commits: &[Commit],
    layout: &LaneLayout,
    selected: usize,
) -> Highlight {
    use std::collections::HashSet;
    if commits.get(selected).is_none() || selected >= layout.lane_of.len() {
        return Highlight::default();
    }
    let lid = layout.lane_of[selected];

    let interior: HashSet<Oid> = commits
        .iter()
        .enumerate()
        .filter(|(i, _)| layout.lane_of.get(*i) == Some(&lid))
        .map(|(_, c)| c.id.clone())
        .collect();

    let mut nodes = interior.clone();
    // END boundary: the merge that absorbed this branch (if any).
    if let Some(&Some(m)) = layout.creator.get(lid) {
        nodes.insert(commits[m].id.clone());
    }
    // START boundary: the trunk commit the branch forked from (if any).
    if let Some(&Some(f)) = layout.terminator.get(lid) {
        nodes.insert(commits[f].id.clone());
    }

    Highlight {
        nodes,
        lanes: interior,
    }
}

/// The tip (newest commit) of the selected lane, as an index into `commits`.
///
/// `commits` are newest-first, so the tip is the first commit in iteration
/// order whose id is in `hl.lanes`. Returns `None` when the highlight is empty
/// (no selection / empty graph).
pub fn selected_lane_tip(hl: &Highlight, commits: &[Commit]) -> Option<usize> {
    commits
        .iter()
        .enumerate()
        .find(|(_, c)| hl.lanes.contains(&c.id))
        .map(|(i, _)| i)
}

/// Build the commit graph for `commits` (newest-first topological order).
///
/// In *spacious* mode an edge row is emitted between consecutive commits and
/// lanes are compacted leftward with diagonal connectors. In *compact* mode
/// only node rows are produced and lanes keep fixed columns.
pub fn build_graph(commits: &[Commit], spacious: bool) -> Vec<GraphRow> {
    build_graph_opts(commits, spacious, false)
}

/// Like [`build_graph`] but with extra options.
///
/// When `first_parent` is true, side-branch (2nd+) parents do NOT open lanes —
/// pair this with a first-parent commit list (`git log --first-parent`) to draw
/// a merge-heavy trunk as one straight line. Merge commits still render `◉`.
pub fn build_graph_opts(commits: &[Commit], spacious: bool, first_parent: bool) -> Vec<GraphRow> {
    build_graph_main(commits, spacious, first_parent, None, None)
}

/// Like [`build_graph_opts`] but with reserved backbone columns.
///
/// `main_tip` (the `main`/`master`/`trunk` tip) is pinned to column 0 and
/// `head_tip` (the current branch's tip) to column 1 — so main is the stable
/// backbone and the branch you're working on (e.g. an unmerged `dev` integration
/// branch where commits pile up) is always pinned right beside it, prominent and
/// stable. Every other branch opens to their right. This stops an unmerged
/// feature — even one whose commits are newer than main's tip — from stealing the
/// leftmost column and reading as if it were the mainline. Pass `None`/`None` to
/// disable (the layout is then identical to the historical behavior).
pub fn build_graph_main(
    commits: &[Commit],
    spacious: bool,
    first_parent: bool,
    main_tip: Option<&str>,
    head_tip: Option<&str>,
) -> Vec<GraphRow> {
    if commits.is_empty() {
        return Vec::new();
    }

    let reserved = reserved_tips(main_tip, head_tip);
    let mut active: Vec<Option<Lane>> = Vec::new();
    let mut rows: Vec<GraphRow> = Vec::new();
    let mut color_seed = 0usize;

    // Reusable index: OID → lane columns currently flowing toward that OID.
    // Rebuilt at the top of each commit iteration (O(k)) so the arriving-lane
    // and merge-parent lookups below are O(1) instead of O(k) linear scans —
    // turning the overall layout from O(n²·k) to O(n·k) on merge-heavy repos.
    // Keys are owned (cloned) so the map does NOT borrow `active`, allowing
    // `active` to be mutated freely after the lookups are extracted.
    let mut oid_to_lanes: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();

    for (idx, commit) in commits.iter().enumerate() {
        let cid = &commit.id;

        // ── Step 1: locate the commit's lane ────────────────────────────────
        oid_to_lanes.clear();
        for (i, slot) in active.iter().enumerate() {
            if let Some(lane) = slot {
                oid_to_lanes.entry(lane.oid.clone()).or_default().push(i);
            }
        }
        let arriving: Vec<usize> = oid_to_lanes.get(cid).cloned().unwrap_or_default();

        // Pre-compute which extra parents already have a lane.
        let extra_parents: &[Oid] = if first_parent {
            &[]
        } else {
            &commit.parents[1.min(commit.parents.len())..]
        };
        let parent_existing: Vec<Option<usize>> = extra_parents
            .iter()
            .map(|p| oid_to_lanes.get(p).and_then(|v| v.first().copied()))
            .collect();

        let commit_pos = if let Some(&first) = arriving.first() {
            first
        } else {
            // A branch tip / root with no lane reserved — open one. A reserved tip
            // (main → col0, current branch → col1) claims its column; others open
            // to the right of the reserved block.
            let rank = reserved.iter().position(|&x| x == cid.as_str());
            let pos = alloc_reserved_tip(&mut active, rank, reserved.len());
            // The col-0 backbone always uses the anchor color (index 0); side
            // branches rotate through the rest, keeping it one recognizable hue.
            let color = if rank == Some(0) {
                0
            } else {
                pick_color(&active, pos, color_seed, !reserved.is_empty())
            };
            color_seed += 1;
            active[pos] = Some(Lane {
                oid: cid.clone(),
                color,
                head: cid.clone(),
            });
            pos
        };

        let node_color = active[commit_pos].as_ref().map(|l| l.color).unwrap_or(0);

        // ── Step 2: choose columns for extra (merge) parents ────────────────
        // (parent_oid, col, color, reused). The first parent reuses commit_pos.
        // In first-parent mode the side branches are collapsed, so we skip this
        // entirely — the node still shows `◉` (it keeps both parents in its data)
        // but only the first-parent lane continues, giving a straight trunk.
        let mut branch: Vec<(Oid, usize, usize, bool)> = Vec::new();
        for (parent, existing) in extra_parents.iter().zip(parent_existing) {
            if let Some(existing) = existing {
                // Another lane already heads to this parent — merge into it
                // rather than open a duplicate column.
                let color = active[existing].as_ref().map(|l| l.color).unwrap_or(0);
                branch.push((parent.clone(), existing, color, true));
            } else {
                let col = alloc(&mut active, commit_pos + 1);
                let color = pick_color(&active, col, color_seed, !reserved.is_empty());
                color_seed += 1;
                // Reserve so the next parent doesn't pick the same slot.
                active[col] = Some(Lane {
                    oid: parent.clone(),
                    color,
                    head: parent.clone(),
                });
                branch.push((parent.clone(), col, color, false));
            }
        }

        let branch_cols: Vec<usize> = branch.iter().map(|(_, c, _, _)| *c).collect();

        // ── Step 3: build the NODE row ──────────────────────────────────────
        let width = active.len();
        let mut rb = RowBuilder::new(width);

        for (col, slot) in active.iter().enumerate() {
            let Some(lane) = slot else { continue };
            if col == commit_pos || branch_cols.contains(&col) {
                continue; // handled as node / branch-out below
            }
            if &lane.oid == cid {
                // Extra lane arriving at this commit → merges in. Tag with the
                // lane's head so the convergence dims with the branch it carries.
                rb.vert(col, lane.color, true, false, &lane.head);
                rb.horiz(col, commit_pos, lane.color, &lane.head);
            } else {
                // Unrelated pass-through lane — tagged with its head (the branch
                // it belongs to), so a first-parent highlight only lights the
                // selected branch's own column, not every lane heading to a
                // shared ancestor.
                rb.vert(col, lane.color, true, true, &lane.head);
            }
        }

        for (poid, col, color, reused) in &branch {
            // Branch-out to an extra (merge) parent: horizontal from the node,
            // then down. Tag BOTH the horizontal and the vertical with the side
            // parent's OID so they track that side branch — a first-parent
            // highlight then dims the whole reach-to-side-branch, keeping only
            // the trunk spine vivid.
            rb.horiz(commit_pos, *col, *color, poid);
            rb.vert(*col, *color, *reused, true, poid);
        }

        let has_up = !arriving.is_empty();
        let has_down = !commit.parents.is_empty();
        rb.node(
            commit_pos,
            node_glyph(commit),
            node_color,
            has_up,
            has_down,
            cid,
        );

        rows.push(GraphRow {
            commit_index: idx,
            cells: rb.finish(),
            is_node_row: true,
        });

        // ── Step 4: advance the lane table ──────────────────────────────────
        // Terminate extra lanes that merged into this commit.
        for &col in &arriving {
            if col != commit_pos {
                active[col] = None;
            }
        }
        // The commit's lane becomes its first parent (or closes). Its head is now
        // THIS commit, so the segment below (down to the first parent) is tagged
        // with `cid` — the trunk spine stays one continuous highlighted column.
        active[commit_pos] = commit.parents.first().map(|p| Lane {
            oid: p.clone(),
            color: node_color,
            head: cid.clone(),
        });
        // New branch lanes adopt their parent oid (reused ones already do); their
        // head is the side parent so the side branch is tagged as its own.
        for (poid, col, color, _) in &branch {
            active[*col] = Some(Lane {
                oid: poid.clone(),
                color: *color,
                head: poid.clone(),
            });
        }

        // ── Step 5: reclaim columns + edge row ──────────────────────────────
        // Stable-column layout: a lane NEVER changes column once placed, so the
        // eye can follow a straight vertical from tip to merge. Freed columns
        // are reclaimed two ways: trailing gaps are trimmed, and interior gaps
        // are reused by future branch-outs (`alloc` picks the leftmost free
        // slot). The only diagonals are at genuine branch (`╮`) / merge (`╯`)
        // points — there are no lane-sliding "zigzag" rows.
        while matches!(active.last(), Some(None)) {
            active.pop();
        }
        if spacious && idx + 1 < commits.len() {
            // Edge row: every active lane continues straight down.
            let mut eb = RowBuilder::new(active.len());
            for (col, slot) in active.iter().enumerate() {
                if let Some(lane) = slot {
                    eb.vert(col, lane.color, true, true, &lane.head);
                }
            }
            rows.push(GraphRow {
                commit_index: usize::MAX,
                cells: eb.finish(),
                is_node_row: false,
            });
        }
    }

    rows
}

// ─── render_ascii ─────────────────────────────────────────────────────────────

/// Render the commit graph as plain ASCII text for `giv debug graph`.
pub fn render_ascii(commits: &[Commit], spacious: bool) -> String {
    render_ascii_main(commits, spacious, None, None)
}

/// Like [`render_ascii`] but with the reserved backbone columns (col0 = main,
/// col1 = current branch), so the ASCII debug output matches what the TUI draws.
pub fn render_ascii_main(
    commits: &[Commit],
    spacious: bool,
    main_tip: Option<&str>,
    head_tip: Option<&str>,
) -> String {
    let rows = build_graph_main(commits, spacious, false, main_tip, head_tip);
    let mut out = String::new();

    for row in &rows {
        let graph_str: String = row.cells.iter().map(|c| c.symbol).collect();

        if row.is_node_row && row.commit_index < commits.len() {
            let commit = &commits[row.commit_index];
            let ref_str = if commit.refs.is_empty() {
                String::new()
            } else {
                let parts: Vec<String> = commit
                    .refs
                    .iter()
                    .map(|r| match r.kind {
                        RefKind::Head => format!("HEAD -> {}", r.name),
                        RefKind::LocalBranch => r.name.clone(),
                        RefKind::RemoteBranch => r.name.clone(),
                        RefKind::Tag => format!("tag: {}", r.name),
                    })
                    .collect();
                format!(" ({})", parts.join(", "))
            };
            out.push_str(&format!(
                "{}  {} {}{}",
                graph_str, commit.short_id, commit.summary, ref_str
            ));
        } else {
            out.push_str(&graph_str);
        }
        out.push('\n');
    }

    out
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::types::{Commit, RefKind, RefName};

    fn mk(id: &str, parents: Vec<&str>, summary: &str) -> Commit {
        Commit {
            id: id.to_string(),
            short_id: id.to_string(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            summary: summary.to_string(),
            body: String::new(),
            author_name: "T".to_string(),
            author_email: "t@e".to_string(),
            time: 0,
            refs: Vec::new(),
        }
    }

    fn mk_refs(id: &str, parents: Vec<&str>, summary: &str, refs: Vec<RefName>) -> Commit {
        let mut c = mk(id, parents, summary);
        c.refs = refs;
        c
    }

    /// Concatenate just the graph-cell symbols of every row.
    fn graph_lines(commits: &[Commit], spacious: bool) -> Vec<String> {
        build_graph(commits, spacious)
            .iter()
            .map(|r| r.cells.iter().map(|c| c.symbol).collect::<String>())
            .collect()
    }

    fn node_row<'a>(rows: &'a [GraphRow], commits: &[Commit], sid: &str) -> &'a GraphRow {
        rows.iter()
            .find(|r| r.is_node_row && commits[r.commit_index].short_id == sid)
            .unwrap_or_else(|| panic!("no node row for {sid}"))
    }

    // ── Glyph rule table ─────────────────────────────────────────────────────

    #[test]
    fn test_glyph_rules() {
        assert_eq!(glyph(true, true, false, false), '│');
        assert_eq!(glyph(false, false, true, true), '─');
        assert_eq!(glyph(false, true, false, true), '╭');
        assert_eq!(glyph(false, true, true, false), '╮');
        assert_eq!(glyph(true, false, false, true), '╰');
        assert_eq!(glyph(true, false, true, false), '╯');
        assert_eq!(glyph(true, true, true, true), '┼');
        assert_eq!(glyph(true, true, true, false), '┤');
        assert_eq!(glyph(false, false, false, false), ' ');
    }

    // ── Linear history ───────────────────────────────────────────────────────

    #[test]
    fn test_linear_single() {
        let commits = vec![mk("a", vec![], "init")];
        let rows = build_graph(&commits, false);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_node_row);
        assert_eq!(rows[0].cells[0].symbol, '●');
    }

    #[test]
    fn test_linear_compact() {
        let commits = vec![
            mk("c", vec!["b"], "third"),
            mk("b", vec!["a"], "second"),
            mk("a", vec![], "first"),
        ];
        assert_eq!(graph_lines(&commits, false), vec!["●", "●", "●"]);
    }

    #[test]
    fn test_linear_spacious() {
        let commits = vec![
            mk("c", vec!["b"], "third"),
            mk("b", vec!["a"], "second"),
            mk("a", vec![], "first"),
        ];
        // node, edge, node, edge, node
        assert_eq!(graph_lines(&commits, true), vec!["●", "│", "●", "│", "●"]);
    }

    #[test]
    fn test_linear_width_stays_one() {
        let commits: Vec<Commit> = (0..8)
            .rev()
            .map(|i| {
                let id = format!("{i:04}");
                let parents = if i == 0 {
                    vec![]
                } else {
                    vec![format!("{:04}", i - 1)]
                };
                let mut c = mk(&id, parents.iter().map(|s| s.as_str()).collect(), "x");
                c.short_id = id.clone();
                c
            })
            .collect();
        for line in graph_lines(&commits, true) {
            assert!(
                line.chars().count() <= 1,
                "linear graph must stay 1 wide: {line:?}"
            );
        }
    }

    // ── Node glyph differentiation ─────────────────────────────────────────────

    #[test]
    fn test_merge_glyph_is_filled_circle() {
        let commits = vec![
            mk("M", vec!["A", "B"], "merge"),
            mk("B", vec!["A"], "feat"),
            mk("A", vec![], "root"),
        ];
        let rows = build_graph(&commits, true);
        let m = node_row(&rows, &commits, "M");
        assert!(
            m.cells.iter().any(|c| c.symbol == '◉'),
            "merge node should be ◉"
        );
        // Non-merge commits use ●.
        let b = node_row(&rows, &commits, "B");
        assert!(b.cells.iter().any(|c| c.symbol == '●'));
    }

    #[test]
    fn test_tag_glyph_is_diamond() {
        let commits = vec![
            mk_refs(
                "a",
                vec!["b"],
                "tagged",
                vec![RefName {
                    name: "v1".into(),
                    kind: RefKind::Tag,
                }],
            ),
            mk("b", vec![], "root"),
        ];
        let rows = build_graph(&commits, false);
        let a = node_row(&rows, &commits, "a");
        assert!(
            a.cells.iter().any(|c| c.symbol == '◆'),
            "tagged node should be ◆"
        );
    }

    // ── Branch + merge ─────────────────────────────────────────────────────────

    #[test]
    fn test_branch_out_has_corner() {
        // M merges A and B. Branch-out to the feature lane uses a corner.
        let commits = vec![
            mk("M", vec!["A", "B"], "merge"),
            mk("B", vec!["A"], "feat"),
            mk("A", vec![], "root"),
        ];
        let ascii = render_ascii(&commits, false);
        assert!(ascii.contains('◉'), "merge glyph present:\n{ascii}");
        assert!(
            ascii.contains('╮') || ascii.contains('┬'),
            "branch-out corner present:\n{ascii}"
        );
        assert!(
            ascii.contains('╯') || ascii.contains('╰') || ascii.contains('┴'),
            "convergence present:\n{ascii}"
        );
    }

    #[test]
    fn test_quick_branch_merge_all_render() {
        let commits = vec![
            mk("M", vec!["A", "F"], "merge"),
            mk("F", vec!["A"], "feature"),
            mk("A", vec![], "root"),
        ];
        let rows = build_graph(&commits, true);
        let node_rows = rows.iter().filter(|r| r.is_node_row).count();
        assert_eq!(node_rows, 3);
        // The feature commit shares a row with the still-open main lane.
        let f = node_row(&rows, &commits, "F");
        assert!(f.cells.iter().any(|c| c.symbol == '●'));
        assert!(f.cells.iter().any(|c| c.symbol == '│'));
    }

    // ── Compaction: a closed branch lets the graph shrink back to width 1 ──────

    #[test]
    fn test_compaction_collapses_after_merge() {
        // A feature branch opens, then merges back mid-history. Commits BELOW the
        // point where the feature lane closes must collapse back to width 1 —
        // proving the freed lane is reclaimed (no permanent gap held open).
        //
        //   T  parents=[M]        post-merge, on main
        //   M  parents=[X, F]     merge feature
        //   F  parents=[X]        feature commit
        //   X  parents=[W]        main commit (feature lane converges here)
        //   W  parents=[]         root
        let commits = vec![
            mk("T", vec!["M"], "post-merge"),
            mk("M", vec!["X", "F"], "merge feature"),
            mk("F", vec!["X"], "feat"),
            mk("X", vec!["W"], "main"),
            mk("W", vec![], "root"),
        ];
        let rows = build_graph(&commits, true);

        // The merge must widen the graph somewhere (>1 lane column).
        let max_w = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
        assert!(max_w >= 3, "merge should widen the graph: {max_w}");

        // The root row (W) is below the feature convergence at X, so it must be
        // fully compacted back to a single node.
        let w = node_row(&rows, &commits, "W");
        let w_str: String = w.cells.iter().map(|c| c.symbol).collect();
        assert_eq!(
            w_str.trim_end(),
            "●",
            "graph must compact to width 1 below the merge"
        );

        // X is where the feature lane converges → it carries a convergence corner.
        let x = node_row(&rows, &commits, "X");
        let x_str: String = x.cells.iter().map(|c| c.symbol).collect();
        assert!(
            x_str.contains('╯') || x_str.contains('┴'),
            "feature lane should converge at X: {x_str:?}"
        );
    }

    // ── Lane continuity: no active lane vanishes for a row ─────────────────────

    #[test]
    fn test_crossing_renders_as_duck_under_not_plus() {
        // A merge whose branch-out must cross an UNRELATED open lane should draw
        // the crossed lane as a continuous '│' (horizontal ducks under), never a
        // '┼' that falsely implies a connection.
        //
        //   C1 parents=[M]      keeps M in col0
        //   B1 parents=[P]      an unrelated open branch tip (col1)
        //   M  parents=[P, F]   merges F (col2) → connector crosses col1
        //   F  parents=[P]
        //   P  parents=[]
        let commits = vec![
            mk("C1", vec!["M"], "child"),
            mk("B1", vec!["P"], "other branch"),
            mk("M", vec!["P", "F"], "merge F"),
            mk("F", vec!["P"], "feature"),
            mk("P", vec![], "base"),
        ];
        let rows = build_graph(&commits, true);
        let m = node_row(&rows, &commits, "M");
        let m_str: String = m.cells.iter().map(|c| c.symbol).collect();
        assert!(
            !m_str.contains('┼'),
            "unrelated crossing must not render as ┼: {m_str:?}"
        );
        // The crossed lane stays a continuous vertical between the node and the
        // branch-out corner.
        assert!(
            m_str.contains('│'),
            "crossed lane must stay continuous │: {m_str:?}"
        );
        assert!(
            m_str.contains('╮') || m_str.contains('┬'),
            "merge still branches out: {m_str:?}"
        );
    }

    #[test]
    fn test_stable_columns_edge_rows_are_straight() {
        // Inter-commit edge rows must be pure verticals (│ / space) — lanes never
        // slide sideways, so no diagonal connectors appear off the node rows.
        let commits = vec![
            mk("M", vec!["A", "B"], "merge"),
            mk("B", vec!["A"], "feat"),
            mk("A", vec!["R"], "main"),
            mk("R", vec![], "root"),
        ];
        let rows = build_graph(&commits, true);
        for row in rows.iter().filter(|r| !r.is_node_row) {
            for cell in &row.cells {
                assert!(
                    matches!(cell.symbol, '│' | ' '),
                    "edge rows must contain only │/space (no slide diagonals), got {:?}",
                    row.cells.iter().map(|c| c.symbol).collect::<String>()
                );
            }
        }
    }

    #[test]
    fn test_lane_continuity_no_gaps() {
        let commits = vec![
            mk("PM", vec!["MC"], "post-merge"),
            mk("MC", vec!["TH", "FC2"], "merge feature"),
            mk("FC2", vec!["FC1"], "feat 2"),
            mk("FC1", vec!["SE"], "feat 1"),
            mk("TH", vec!["SE"], "third"),
            mk("SE", vec!["IN"], "second"),
            mk("IN", vec![], "initial"),
        ];
        let rows = build_graph(&commits, true);

        let num_cols = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
        let grid: Vec<Vec<char>> = rows
            .iter()
            .map(|r| {
                let mut v: Vec<char> = r.cells.iter().map(|c| c.symbol).collect();
                v.resize(num_cols, ' ');
                v
            })
            .collect();

        let is_lane = |c: char| {
            matches!(
                c,
                '●' | '◉'
                    | '◆'
                    | '│'
                    | '╭'
                    | '╮'
                    | '╰'
                    | '╯'
                    | '├'
                    | '┤'
                    | '┬'
                    | '┴'
                    | '┼'
            )
        };
        for col in 0..num_cols {
            let rows_active: Vec<usize> =
                (0..grid.len()).filter(|&r| is_lane(grid[r][col])).collect();
            if let (Some(&first), Some(&last)) = (rows_active.first(), rows_active.last()) {
                for (r, row) in grid.iter().enumerate().take(last + 1).skip(first) {
                    assert!(
                        is_lane(row[col]),
                        "lane gap at row {r} col {col}\n{}",
                        rows.iter()
                            .map(|x| x.cells.iter().map(|c| c.symbol).collect::<String>())
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                }
            }
        }
    }

    // ── Merge into an existing lane reuses it (no phantom duplicate) ───────────

    #[test]
    fn test_merge_into_existing_lane_reuses_it() {
        let commits = vec![
            mk("M1", vec!["M2", "q1"], "merge A"),
            mk("q1", vec!["P"], "featA tip"),
            mk("M2", vec!["C", "P"], "merge B"),
            mk("P", vec!["C"], "shared P"),
            mk("C", vec![], "root C"),
        ];
        let rows = build_graph(&commits, true);
        // P must appear in exactly one lane (single node).
        let p = node_row(&rows, &commits, "P");
        let p_nodes = p
            .cells
            .iter()
            .filter(|c| matches!(c.symbol, '●' | '◉' | '◆'))
            .count();
        assert_eq!(p_nodes, 1, "P must have a single node lane");
    }

    // ── render_ascii smoke ─────────────────────────────────────────────────────

    #[test]
    fn test_render_ascii_contains_metadata() {
        let commits = vec![
            mk_refs(
                "aaa1234",
                vec!["bbb5678"],
                "second commit",
                vec![RefName {
                    name: "main".into(),
                    kind: RefKind::LocalBranch,
                }],
            ),
            mk("bbb5678", vec![], "initial commit"),
        ];
        let ascii = render_ascii(&commits, false);
        assert!(ascii.contains("aaa1234"));
        assert!(ascii.contains("second commit"));
        assert!(ascii.contains("(main)"));
        assert!(ascii.contains('●'));
        assert!(ascii.contains("initial commit"));
    }

    // ── Lineage (ancestors) ────────────────────────────────────────────────────

    #[test]
    fn test_ancestors_linear() {
        // C → B → A. Selecting C yields the whole chain; selecting B drops C.
        let commits = vec![
            mk("C", vec!["B"], "c"),
            mk("B", vec!["A"], "b"),
            mk("A", vec![], "a"),
        ];
        let from_c = ancestors(&commits, 0);
        assert!(from_c.contains("C") && from_c.contains("B") && from_c.contains("A"));
        let from_b = ancestors(&commits, 1);
        assert!(from_b.contains("B") && from_b.contains("A"));
        assert!(
            !from_b.contains("C"),
            "B's lineage must not include its child C"
        );
    }

    #[test]
    fn test_ancestors_follows_both_merge_parents() {
        // M merges A and F. M's lineage includes BOTH parents' histories.
        let commits = vec![
            mk("M", vec!["A", "F"], "merge"),
            mk("F", vec!["A"], "feat"),
            mk("A", vec![], "root"),
        ];
        let from_m = ancestors(&commits, 0);
        assert!(from_m.contains("M") && from_m.contains("A") && from_m.contains("F"));
        // F alone does not include the merge commit M.
        let from_f = ancestors(&commits, 1);
        assert!(from_f.contains("F") && from_f.contains("A"));
        assert!(!from_f.contains("M"));
    }

    #[test]
    fn test_pass_through_cell_tracks_lane_head() {
        // featB→featA→main1→root, plus a separate main2→main1 tip. At main2's
        // node row the feature lane merely passes through; that '│' cell must
        // carry the lane's HEAD (featA, the last feature node above) — NOT its
        // target main1. Both the feature lane and main2's lane flow toward main1,
        // so tagging by head is what lets a first-parent highlight light only the
        // selected branch's own column.
        let commits = vec![
            mk("fb", vec!["fa"], "featB"),
            mk("fa", vec!["m1"], "featA"),
            mk("m2", vec!["m1"], "main2"),
            mk("m1", vec!["r"], "main1"),
            mk("r", vec![], "root"),
        ];
        let rows = build_graph(&commits, false);
        let m2 = node_row(&rows, &commits, "m2");

        // The node cell carries m2's own oid.
        let node = m2
            .cells
            .iter()
            .find(|c| matches!(c.symbol, '●' | '◉' | '◆'))
            .unwrap();
        assert_eq!(node.vertical_oid.as_deref(), Some("m2"));

        // The pass-through feature lane carries its head (featA), which IS in
        // featB's first-parent lineage → it stays highlighted (line unbroken).
        let lineage = first_parent_lineage(&commits, 0); // select featB
        let pass = m2
            .cells
            .iter()
            .find(|c| c.symbol == '│' && c.vertical_oid.as_deref() == Some("fa"));
        assert!(
            pass.is_some(),
            "pass-through lane must carry its head oid 'fa': {:?}",
            m2.cells
                .iter()
                .map(|c| (c.symbol, c.vertical_oid.clone()))
                .collect::<Vec<_>>()
        );
        assert!(
            lineage.contains("fa"),
            "featA is part of featB's first-parent lineage"
        );
        assert!(
            !lineage.contains("m2"),
            "main2 is NOT part of featB's lineage"
        );
    }

    #[test]
    fn test_branch_highlight_fork_to_merge() {
        // M merges feature (f1,f2,f3) into main. Selecting a MID-feature commit
        // highlights the whole feature LANE (f1,f2,f3); the fork (A) and merge (M)
        // glow only as boundary NODES (not lanes), and the mainline above the
        // merge / below the fork stays dark.
        let commits = vec![
            mk("C", vec!["M"], "main after"),
            mk("M", vec!["B", "f3"], "merge"),
            mk("f3", vec!["f2"], "f3"),
            mk("f2", vec!["f1"], "f2"),
            mk("f1", vec!["A"], "f1"),
            mk("B", vec!["A"], "B main"),
            mk("A", vec!["root"], "A fork"),
            mk("root", vec![], "root"),
        ];
        let hl = branch_highlight(&commits, 3, false); // select f2
        for id in ["f3", "f2", "f1"] {
            assert!(
                hl.lanes.contains(id),
                "{id} must be branch lane: {:?}",
                hl.lanes
            );
        }
        // Boundaries glow as nodes, NOT as lanes.
        assert!(
            hl.nodes.contains("M") && hl.nodes.contains("A"),
            "fork/merge are nodes"
        );
        assert!(
            !hl.lanes.contains("M") && !hl.lanes.contains("A"),
            "boundaries not lanes"
        );
        for id in ["C", "B", "root"] {
            assert!(
                !hl.lanes.contains(id) && !hl.nodes.contains(id),
                "{id} stays dark"
            );
        }

        // A mainline commit lights the whole trunk lane, no merge boundary.
        let main = branch_highlight(&commits, 0, false); // select C
        assert!(main.lanes.contains("C") && main.lanes.contains("root"));
    }

    #[test]
    fn test_branch_highlight_separates_true_siblings() {
        // featA and featB BOTH fork from `base` (a1.p0 == b1.p0 == base) and merge
        // separately. Their lanes genuinely diverge at base, so selecting a1 must
        // light featA alone — never bleed into the sibling featB.
        let commits = vec![
            mk("top", vec!["MB"], "top"),
            mk("MB", vec!["MA", "b1"], "merge featB"),
            mk("b1", vec!["base"], "b1"),
            mk("MA", vec!["base", "a1"], "merge featA"),
            mk("a1", vec!["base"], "a1"),
            mk("base", vec!["root"], "base"),
            mk("root", vec![], "root"),
        ];
        let ha = branch_highlight(&commits, 4, false); // select a1
        assert!(ha.lanes.contains("a1"));
        assert!(ha.nodes.contains("MA") && ha.nodes.contains("base"));
        for id in ["b1", "MB", "top"] {
            assert!(
                !ha.lanes.contains(id) && !ha.nodes.contains(id),
                "{id} (sibling) dark"
            );
        }
    }

    #[test]
    fn test_branch_highlight_spans_across_intervening_merge() {
        // Reproduction of the reported bug. A design line `u2→u1→sel→d` forks off
        // the trunk at `base` and is absorbed by the LATER merge ML. A feature `x`
        // forks off the SAME line at `u1` and is absorbed by the EARLIER merge MX.
        // Selecting `sel` must light the WHOLE design line up to ML — spanning past
        // MX — and must NOT stop at MX or pull in the fork-off feature `x`.
        let commits = vec![
            mk("top", vec!["ML"], "top"),
            mk("ML", vec!["MX", "u2"], "merge design (END)"),
            mk("u2", vec!["u1"], "design u2"),
            mk("MX", vec!["base", "x"], "merge feature x"),
            mk("x", vec!["u1"], "feature x (forks at u1)"),
            mk("u1", vec!["sel"], "design u1"),
            mk("sel", vec!["d"], "SELECTED design"),
            mk("d", vec!["base"], "design d"),
            mk("base", vec!["root"], "base"),
            mk("root", vec![], "root"),
        ];
        let hl = branch_highlight(&commits, 6, false); // select sel
        for id in ["u2", "u1", "sel", "d"] {
            assert!(
                hl.lanes.contains(id),
                "{id} must be on the design lane: {:?}",
                hl.lanes
            );
        }
        // END is the LATER merge ML (not the intervening MX); START is `base`.
        assert!(
            hl.nodes.contains("ML"),
            "later merge ML is the END boundary"
        );
        assert!(hl.nodes.contains("base"), "base is the fork");
        // The fork-off feature and the intervening merge stay dark.
        for id in ["x", "MX", "top", "root"] {
            assert!(
                !hl.lanes.contains(id) && !hl.nodes.contains(id),
                "{id} must stay dark"
            );
        }
    }

    #[test]
    fn test_selected_line_wins_crossing() {
        // At a crossing (`─│─`), the SELECTED branch's line is drawn with top
        // priority: if the selected branch owns the horizontal it renders `─`
        // (straight line, the crossed vertical segmented); if it owns the
        // vertical it stays `│`.
        let commits = vec![
            mk("C1", vec!["M"], "child"),
            mk("B1", vec!["P"], "other"),
            mk("M", vec!["P", "F"], "merge"),
            mk("F", vec!["P"], "feature"),
            mk("P", vec![], "base"),
        ];
        let rows = build_graph(&commits, false);
        let m = node_row(&rows, &commits, "M");
        let cross = m
            .cells
            .iter()
            .find(|c| {
                c.symbol == '│'
                    && c.vertical_oid.is_some()
                    && (!c.left_edge_oids.is_empty() || !c.right_edge_oids.is_empty())
            })
            .expect("a duck-under crossing cell");

        let hl = |ids: &[&str]| Highlight {
            lanes: ids.iter().map(|s| s.to_string()).collect(),
            nodes: ids.iter().map(|s| s.to_string()).collect(),
        };

        // No selection → default duck-under stays `│`.
        assert_eq!(cell_glyph(cross, None).0, '│');

        // Selecting the crossing HORIZONTAL's branch → it wins → `─` (straight).
        let h = cross
            .left_edge_oids
            .first()
            .or_else(|| cross.right_edge_oids.first())
            .cloned()
            .unwrap();
        assert_eq!(
            cell_glyph(cross, Some(&hl(&[&h]))).0,
            '─',
            "selected horizontal must win the crossing (line stays straight)"
        );

        // Selecting the vertical lane → it stays continuous `│`.
        let v = cross.vertical_oid.clone().unwrap();
        assert_eq!(cell_glyph(cross, Some(&hl(&[&v]))).0, '│');
    }

    // ── Main-spine reservation (column 0 = main) ──────────────────────────────

    /// A feature branch with commits NEWER than main's tip must not steal the
    /// backbone column: with `main_tip` set, main owns column 0 and the unmerged
    /// feature is pushed to the right.
    #[test]
    fn test_main_spine_reserved_to_column_zero() {
        let commits = vec![
            mk("feat3", vec!["feat2"], "f3"),
            mk("feat2", vec!["feat1"], "f2"),
            mk("feat1", vec!["main1"], "f1"),
            mk("main3", vec!["main2"], "m3"), // main tip
            mk("main2", vec!["main1"], "m2"),
            mk("main1", vec!["root"], "m1"),
            mk("root", vec![], "root"),
        ];

        let is_node = |c: char| matches!(c, '●' | '◉' | '◆');

        // Baseline: WITHOUT reservation the newest tip (feature) grabs column 0.
        let plain = build_graph_main(&commits, false, false, None, None);
        let f3_plain = node_row(&plain, &commits, "feat3");
        assert!(
            is_node(f3_plain.cells[0].symbol),
            "without reservation the newest feature tip sits in column 0"
        );

        // WITH reservation: main is the column-0 backbone, feature pushed right.
        let rows = build_graph_main(&commits, false, false, Some("main3"), None);
        let m3 = node_row(&rows, &commits, "main3");
        assert!(
            is_node(m3.cells[0].symbol),
            "main tip must be the column-0 backbone"
        );

        let f3 = node_row(&rows, &commits, "feat3");
        assert_eq!(
            f3.cells[0].symbol, ' ',
            "feature column 0 stays empty (reserved for main)"
        );
        assert!(
            f3.cells.iter().any(|c| is_node(c.symbol)),
            "feature node still rendered to the right of main"
        );

        for sid in ["main2", "main1", "root"] {
            let r = node_row(&rows, &commits, sid);
            assert!(
                is_node(r.cells[0].symbol),
                "{sid} must stay on the column-0 main spine"
            );
        }
    }

    /// The highlight (which replays `compute_lanes_main`) must stay in lockstep
    /// with the reserved layout: selecting a feature commit lights only the
    /// feature lane; selecting main lights the spine.
    #[test]
    fn test_main_reservation_highlight_consistent() {
        let commits = vec![
            mk("feat3", vec!["feat2"], "f3"),
            mk("feat2", vec!["feat1"], "f2"),
            mk("feat1", vec!["main1"], "f1"),
            mk("main3", vec!["main2"], "m3"),
            mk("main2", vec!["main1"], "m2"),
            mk("main1", vec!["root"], "m1"),
            mk("root", vec![], "root"),
        ];
        let main = Some("main3");

        let hf = branch_highlight_main(&commits, 1, false, main, None); // select feat2
        for id in ["feat1", "feat2", "feat3"] {
            assert!(hf.lanes.contains(id), "{id} must be on the feature lane");
        }
        assert!(!hf.lanes.contains("main3") && !hf.lanes.contains("main2"));
        assert!(
            hf.nodes.contains("main1"),
            "main1 is the feature's fork boundary"
        );

        let hm = branch_highlight_main(&commits, 3, false, main, None); // select main3
        for id in ["main3", "main2", "main1", "root"] {
            assert!(hm.lanes.contains(id), "{id} must be on the main spine");
        }
        assert!(
            !hm.lanes.contains("feat1"),
            "feature must stay off the main spine"
        );
    }

    #[test]
    fn branch_highlight_from_cached_lane_layout_matches_main_highlight() {
        let commits = vec![
            mk("feat3", vec!["feat2"], "f3"),
            mk("feat2", vec!["feat1"], "f2"),
            mk("feat1", vec!["main1"], "f1"),
            mk("main3", vec!["main2"], "m3"),
            mk("main2", vec!["main1"], "m2"),
            mk("main1", vec!["root"], "m1"),
            mk("root", vec![], "root"),
        ];
        let lanes = compute_lanes_main(&commits, false, Some("main3"), None);
        let expected = branch_highlight_main(&commits, 1, false, Some("main3"), None);
        let actual = branch_highlight_from_lanes(&commits, &lanes, 1);

        assert_eq!(actual.nodes, expected.nodes);
        assert_eq!(actual.lanes, expected.lanes);
    }

    /// The current branch (`head_tip`) is pinned to column 1, right beside main —
    /// even when an ephemeral feature branch has a newer tip — so the branch you
    /// are working on (e.g. an unmerged `dev`) is always prominent and stable.
    #[test]
    fn test_current_branch_reserved_to_column_one() {
        let commits = vec![
            mk("feat1", vec!["dev1"], "feat1"), // newest, forks off dev1 (not dev's tip)
            mk("dev2", vec!["dev1"], "dev2"),   // current branch (head) TIP
            mk("dev1", vec!["main1"], "dev1"),
            mk("main2", vec!["main1"], "main2"), // main tip
            mk("main1", vec!["root"], "main1"),
            mk("root", vec![], "root"),
        ];
        let is_node = |c: char| matches!(c, '●' | '◉' | '◆');
        let rows = build_graph_main(&commits, false, false, Some("main2"), Some("dev2"));

        let node_cell = |sid: &str| {
            let r = node_row(&rows, &commits, sid);
            r.cells
                .iter()
                .position(|c| is_node(c.symbol))
                .expect("a node cell")
        };
        // Lane column N lives at cell 2*N: main→col0(cell0), dev→col1(cell2).
        assert_eq!(node_cell("main2"), 0, "main is the column-0 backbone");
        assert_eq!(
            node_cell("dev2"),
            2,
            "current branch dev is pinned to column 1"
        );
        assert_eq!(node_cell("dev1"), 2, "dev's spine stays in column 1");
        assert!(
            node_cell("feat1") >= 4,
            "an ephemeral feature opens to the right of the reserved main+dev block"
        );
    }

    #[test]
    fn test_edge_rows_have_max_commit_index() {
        let commits = vec![mk("b", vec!["a"], "second"), mk("a", vec![], "first")];
        let rows = build_graph(&commits, true);
        for row in rows.iter().filter(|r| !r.is_node_row) {
            assert_eq!(row.commit_index, usize::MAX);
        }
    }

    // ── selected_lane_tip ───────────────────────────────────────────────────

    #[test]
    fn selected_lane_tip_returns_newest_commit_on_the_lane() {
        let commits = vec![
            mk("feat3", vec!["feat2"], "f3"),
            mk("feat2", vec!["feat1"], "f2"),
            mk("feat1", vec!["main1"], "f1"),
            mk("main3", vec!["main2"], "m3"),
            mk("main2", vec!["main1"], "m2"),
            mk("main1", vec!["root"], "m1"),
            mk("root", vec![], "root"),
        ];
        let main = Some("main3");
        // Select feat2 → lane = {feat1, feat2, feat3}. Tip (newest) = feat3 at index 0.
        let hl = branch_highlight_main(&commits, 1, false, main, None);
        assert_eq!(selected_lane_tip(&hl, &commits), Some(0));

        // Select main3 → lane = {main1, main2, main3, root}. Tip = main3 at index 3.
        let hl = branch_highlight_main(&commits, 3, false, main, None);
        assert_eq!(selected_lane_tip(&hl, &commits), Some(3));
    }

    #[test]
    fn selected_lane_tip_none_for_empty_highlight() {
        let commits = vec![mk("a", vec![], "first")];
        let hl = Highlight::default();
        assert_eq!(selected_lane_tip(&hl, &commits), None);
    }

    // ── ancestors ────────────────────────────────────────────────────────────

    #[test]
    fn ancestors_empty_for_out_of_range_index() {
        let commits = vec![mk("a", vec![], "first")];
        assert!(ancestors(&commits, 5).is_empty());
    }

    #[test]
    fn ancestors_empty_for_empty_commits() {
        let commits: Vec<Commit> = vec![];
        assert!(ancestors(&commits, 0).is_empty());
    }

    #[test]
    fn ancestors_single_root_commit() {
        let commits = vec![mk("a", vec![], "root")];
        let set = ancestors(&commits, 0);
        assert_eq!(set.len(), 1);
        assert!(set.contains("a"));
    }

    #[test]
    fn ancestors_linear_chain() {
        let commits = vec![
            mk("c", vec!["b"], "third"),
            mk("b", vec!["a"], "second"),
            mk("a", vec![], "first"),
        ];
        let set = ancestors(&commits, 0);
        assert!(set.contains("c"));
        assert!(set.contains("b"));
        assert!(set.contains("a"));
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn ancestors_follows_both_merge_parents() {
        // Merge commit M has parents A and B.
        let commits = vec![
            mk("M", vec!["A", "B"], "merge"),
            mk("A", vec!["root"], "a"),
            mk("B", vec!["root"], "b"),
            mk("root", vec![], "root"),
        ];
        let set = ancestors(&commits, 0);
        assert!(set.contains("M"));
        assert!(set.contains("A"));
        assert!(set.contains("B"));
        assert!(set.contains("root"));
        assert_eq!(set.len(), 4);
    }

    #[test]
    fn ancestors_handles_missing_parent_gracefully() {
        // A commit whose parent is not in the commits list — the BFS simply
        // skips it (it's added to the set but not traversed further).
        let commits = vec![mk("a", vec!["ghost"], "a")];
        let set = ancestors(&commits, 0);
        assert!(set.contains("a"));
        assert!(set.contains("ghost"));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn ancestors_cycle_guard() {
        // A pathological cycle: a → b → a. The BFS should not loop forever.
        let commits = vec![mk("a", vec!["b"], "a"), mk("b", vec!["a"], "b")];
        let set = ancestors(&commits, 0);
        assert!(set.contains("a"));
        assert!(set.contains("b"));
        assert_eq!(set.len(), 2);
    }

    // ── first_parent_lineage ─────────────────────────────────────────────────

    #[test]
    fn first_parent_lineage_empty_for_out_of_range() {
        let commits = vec![mk("a", vec![], "root")];
        assert!(first_parent_lineage(&commits, 5).is_empty());
    }

    #[test]
    fn first_parent_lineage_root_only() {
        let commits = vec![mk("a", vec![], "root")];
        let set = first_parent_lineage(&commits, 0);
        assert_eq!(set.len(), 1);
        assert!(set.contains("a"));
    }

    #[test]
    fn first_parent_lineage_follows_only_first_parent() {
        // Merge commit M (parents A, B). first_parent_lineage should follow
        // only A, NOT B.
        let commits = vec![
            mk("M", vec!["A", "B"], "merge"),
            mk("A", vec!["root"], "a"),
            mk("B", vec!["root"], "b"),
            mk("root", vec![], "root"),
        ];
        let set = first_parent_lineage(&commits, 0);
        // M → A → root. B is NOT in the first-parent lineage.
        assert!(set.contains("M"));
        assert!(set.contains("A"));
        assert!(set.contains("root"));
        assert!(!set.contains("B"));
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn first_parent_lineage_cycle_guard() {
        // a → b → a (cycle). Should terminate.
        let commits = vec![mk("a", vec!["b"], "a"), mk("b", vec!["a"], "b")];
        let set = first_parent_lineage(&commits, 0);
        assert!(set.contains("a"));
        assert!(set.contains("b"));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn first_parent_lineage_handles_missing_first_parent() {
        // Commit with no parents → lineage is just itself.
        let commits = vec![mk("a", vec![], "root")];
        let set = first_parent_lineage(&commits, 0);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn first_parent_lineage_missing_parent_in_list() {
        // Commit references a parent not in the list — the loop adds the
        // current id then breaks when the parent can't be found.
        let commits = vec![mk("a", vec!["ghost"], "a")];
        let set = first_parent_lineage(&commits, 0);
        assert!(set.contains("a"));
        assert!(set.contains("ghost"));
        assert_eq!(set.len(), 2);
    }

    // ── compute_lanes ────────────────────────────────────────────────────────

    #[test]
    fn compute_lanes_linear_assigns_single_lane() {
        let commits = vec![
            mk("c", vec!["b"], "third"),
            mk("b", vec!["a"], "second"),
            mk("a", vec![], "first"),
        ];
        let lanes = compute_lanes(&commits, false);
        assert_eq!(lanes.lane_of, vec![0, 0, 0]);
    }

    #[test]
    fn compute_lanes_branch_gets_new_lane() {
        // Two branches: main (a→b→c) and feature (d→b).
        // c is HEAD, d is a branch tip merging back.
        let commits = vec![
            mk("c", vec!["b"], "c"),
            mk("d", vec!["b"], "feature tip"),
            mk("b", vec!["a"], "b"),
            mk("a", vec![], "a"),
        ];
        let lanes = compute_lanes(&commits, false);
        // c and d are both tips → they get different lanes initially.
        assert_ne!(lanes.lane_of[0], lanes.lane_of[1]);
        // b and a are on the trunk lane (whichever c ended up on).
        assert_eq!(lanes.lane_of[2], lanes.lane_of[3]);
    }

    #[test]
    fn compute_lanes_empty_commits() {
        let commits: Vec<Commit> = vec![];
        let lanes = compute_lanes(&commits, false);
        assert!(lanes.lane_of.is_empty());
    }

    #[test]
    fn compute_lanes_single_root() {
        let commits = vec![mk("a", vec![], "root")];
        let lanes = compute_lanes(&commits, false);
        assert_eq!(lanes.lane_of, vec![0]);
    }

    // ── build_graph edge cases ───────────────────────────────────────────────

    #[test]
    fn build_graph_empty_commits_returns_empty() {
        let commits: Vec<Commit> = vec![];
        let rows = build_graph(&commits, false);
        assert!(rows.is_empty());
    }

    #[test]
    fn build_graph_single_root_has_one_row_compact() {
        let commits = vec![mk("a", vec![], "root")];
        let rows = build_graph(&commits, false);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_node_row);
    }

    #[test]
    fn build_graph_single_root_spacious_has_node_only() {
        // A single root commit in spacious mode: just the node row, no edge row
        // (no parent to draw an edge to).
        let commits = vec![mk("a", vec![], "root")];
        let rows = build_graph(&commits, true);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_node_row);
    }

    #[test]
    fn build_graph_two_commits_spacious_has_three_rows() {
        let commits = vec![mk("b", vec!["a"], "second"), mk("a", vec![], "first")];
        let rows = build_graph(&commits, true);
        // node(b), edge, node(a)
        assert_eq!(rows.len(), 3);
        assert!(rows[0].is_node_row);
        assert!(!rows[1].is_node_row);
        assert!(rows[2].is_node_row);
    }

    #[test]
    fn build_graph_first_parent_collapses_merge_side() {
        // A merge: M (parents A, B). With first_parent=true, the side branch B
        // should be collapsed — the graph should be shorter than without
        // first_parent.
        let commits = vec![
            mk("M", vec!["A", "B"], "merge"),
            mk("A", vec!["root"], "a"),
            mk("B", vec!["root"], "b"),
            mk("root", vec![], "root"),
        ];
        let normal = build_graph_opts(&commits, false, false);
        let first_parent = build_graph_opts(&commits, false, true);
        // first_parent should have fewer or equal rows (it collapses the side).
        assert!(
            first_parent.len() <= normal.len(),
            "first_parent ({}) should be <= normal ({})",
            first_parent.len(),
            normal.len()
        );
    }

    // ── render_ascii ─────────────────────────────────────────────────────────

    #[test]
    fn render_ascii_empty_commits() {
        let commits: Vec<Commit> = vec![];
        let s = render_ascii(&commits, false);
        assert!(s.is_empty());
    }

    #[test]
    fn render_ascii_single_commit_contains_node_glyph() {
        let commits = vec![mk("a", vec![], "root")];
        let s = render_ascii(&commits, false);
        assert!(s.contains('●'));
    }

    #[test]
    fn render_ascii_linear_contains_vertical_bar() {
        let commits = vec![
            mk("c", vec!["b"], "third"),
            mk("b", vec!["a"], "second"),
            mk("a", vec![], "first"),
        ];
        let s = render_ascii(&commits, true);
        assert!(s.contains('│'));
    }

    // ── cell_glyph ───────────────────────────────────────────────────────────

    #[test]
    fn cell_glyph_node_cell_returns_filled_circle() {
        let cell = GraphCell {
            symbol: '●',
            lane: 0,
            dirs: 0,
            vertical_oid: None,
            left_edge_oids: SmallVec::new(),
            right_edge_oids: SmallVec::new(),
        };
        let (ch, _) = cell_glyph(&cell, None);
        assert_eq!(ch, '●');
    }

    #[test]
    fn cell_glyph_empty_cell_returns_space() {
        let cell = GraphCell {
            symbol: ' ',
            lane: 0,
            dirs: 0,
            vertical_oid: None,
            left_edge_oids: SmallVec::new(),
            right_edge_oids: SmallVec::new(),
        };
        let (ch, _) = cell_glyph(&cell, None);
        assert_eq!(ch, ' ');
    }

    // ── GraphCell::in_lineage ────────────────────────────────────────────────

    #[test]
    fn in_lineage_matches_vertical_oid() {
        let mut cell = GraphCell::empty();
        cell.vertical_oid = Some("abc".into());
        let mut set = std::collections::HashSet::new();
        set.insert("abc".to_string());
        assert!(cell.in_lineage(&set));
    }

    #[test]
    fn in_lineage_matches_left_edge_oid() {
        let mut cell = GraphCell::empty();
        cell.left_edge_oids = SmallVec::from_vec(vec!["xyz".to_string()]);
        let mut set = std::collections::HashSet::new();
        set.insert("xyz".to_string());
        assert!(cell.in_lineage(&set));
    }

    #[test]
    fn in_lineage_matches_right_edge_oid() {
        let mut cell = GraphCell::empty();
        cell.right_edge_oids = SmallVec::from_vec(vec!["def".to_string()]);
        let mut set = std::collections::HashSet::new();
        set.insert("def".to_string());
        assert!(cell.in_lineage(&set));
    }

    #[test]
    fn in_lineage_false_when_no_oid_matches() {
        let mut cell = GraphCell::empty();
        cell.vertical_oid = Some("abc".into());
        let set = std::collections::HashSet::new();
        assert!(!cell.in_lineage(&set));
    }

    #[test]
    fn in_lineage_false_for_empty_cell() {
        let cell = GraphCell::empty();
        let mut set = std::collections::HashSet::new();
        set.insert("anything".to_string());
        assert!(!cell.in_lineage(&set));
    }
}
