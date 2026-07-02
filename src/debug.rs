/// Debug subcommand dispatch logic.
///
/// These commands run without a TTY, making them useful for automated
/// verification of git backend parsing (CI, compile-gate, etc.).
///
/// All subcommands open a real `CliBackend` via `git::open(path)`, exercise
/// the backend trait, and print human-readable output to stdout.
use std::path::Path;

use crate::git::{self, GitBackend};

// ─── Debug subcommand actions ────────────────────────────────────────────────

/// Print the application name and version, then exit 0.
pub fn run_version() {
    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
}

/// Print parsed `WorkingStatus` for the repo at `path`.
///
/// Output format (one line per entry):
///   `<index_code> <worktree_code>  <path>[ -> <orig_path>]`
pub fn run_status(path: &Path) -> anyhow::Result<()> {
    let backend = git::open(path)?;
    match backend.status() {
        Ok(ws) => {
            println!("Branch: {}", ws.branch.as_deref().unwrap_or("(none)"));
            if let Some(up) = &ws.upstream {
                println!("Upstream: {} (+{} -{})", up, ws.ahead, ws.behind);
            }
            println!("Entries: {}", ws.entries.len());
            for e in &ws.entries {
                let orig = e
                    .orig_path
                    .as_deref()
                    .map(|p| format!(" -> {p}"))
                    .unwrap_or_default();
                println!("  {:?}/{:?}  {}{}", e.index, e.worktree, e.path, orig);
            }
        }
        Err(e) => {
            eprintln!("status error: {e:#}");
        }
    }
    Ok(())
}

/// Print recent commits for the repo at `path`.
///
/// Output format:
///   `<short_id> <summary> (<refs>)  — <author_name> <<author_email>>`
pub fn run_log(path: &Path, limit: usize) -> anyhow::Result<()> {
    let backend = git::open(path)?;
    match backend.log(limit, true, false) {
        Ok(commits) => {
            println!("{} commit(s):", commits.len());
            for c in &commits {
                let refs: String = if c.refs.is_empty() {
                    String::new()
                } else {
                    format!(
                        " ({})",
                        c.refs
                            .iter()
                            .map(|r| r.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                println!(
                    "  {} {}{}  — {} <{}>",
                    c.short_id, c.summary, refs, c.author_name, c.author_email
                );
            }
        }
        Err(e) => {
            eprintln!("log error: {e:#}");
        }
    }
    Ok(())
}

/// Print the commit graph as ASCII text using the lane engine.
///
/// This calls `git::graph::render_ascii` which is the canonical verification
/// target for the lane-assignment algorithm.  Output includes graph cells,
/// short commit id, summary, and decorated refs.
pub fn run_graph(path: &Path, limit: usize, spacious: bool, all: bool) -> anyhow::Result<()> {
    use crate::features::graph::layout::render_ascii_main;
    use crate::git::types::RefKind;

    let backend = git::open(path)?;
    let branches = backend.branches().unwrap_or_default();
    let main_tip = ["main", "master", "trunk"].iter().find_map(|cand| {
        branches
            .iter()
            .find(|b| b.kind == RefKind::LocalBranch && &b.name == cand)
            .map(|b| b.target.clone())
    });
    match backend.log(limit, all, false) {
        Ok(commits) => {
            let main_tip = main_tip.filter(|sha| commits.iter().any(|c| &c.id == sha));
            let head_tip = commits
                .iter()
                .find(|c| c.refs.iter().any(|r| r.kind == RefKind::Head))
                .map(|c| c.id.clone());
            let ascii =
                render_ascii_main(&commits, spacious, main_tip.as_deref(), head_tip.as_deref());
            print!("{ascii}");
        }
        Err(e) => {
            eprintln!("graph error: {e:#}");
        }
    }
    Ok(())
}

/// Render the commit graph as a colored, self-contained HTML fragment so the
/// rendering can be reviewed visually (the lane colors and box-drawing glyphs
/// are exactly what the TUI draws). Prints a `<section>` block to stdout.
pub fn run_graph_html(
    path: &Path,
    limit: usize,
    spacious: bool,
    focus: Option<String>,
    all: bool,
    first_parent: bool,
    lens: Option<String>,
) -> anyhow::Result<()> {
    use crate::features::graph::layout::{ancestors, branch_highlight_main, build_graph_main};
    use crate::features::graph::render::relative_age;
    use crate::git::types::RefKind;
    use crate::theme::Theme;
    use ratatui::style::Color;

    fn hex(c: Color) -> String {
        match c {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            _ => "#c0caf5".to_string(),
        }
    }
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    let backend = git::open(path)?;
    let branches = backend.branches().unwrap_or_default();
    let main = ["main", "master", "trunk"].iter().find_map(|cand| {
        branches
            .iter()
            .find(|b| b.kind == RefKind::LocalBranch && &b.name == cand)
            .map(|b| (b.name.clone(), b.target.clone()))
    });

    // Branch lens: union of `lens` tip and main; else the normal scoped log.
    let commits = if let Some(tip) = lens.as_deref() {
        backend
            .log_range(
                tip,
                main.as_ref().map(|(n, _)| n.as_str()),
                limit,
                first_parent,
            )
            .unwrap_or_default()
    } else {
        backend.log(limit, all, first_parent).unwrap_or_default()
    };
    let theme = Theme::tokyonight();
    // Reserve column 0 for the main spine when main's tip is in the window.
    let main_tip: Option<String> = main
        .as_ref()
        .map(|(_, sha)| sha.clone())
        .filter(|sha| commits.iter().any(|c| &c.id == sha));
    let head_tip: Option<String> = commits
        .iter()
        .find(|c| c.refs.iter().any(|r| r.kind == RefKind::Head))
        .map(|c| c.id.clone());
    let rows = build_graph_main(
        &commits,
        spacious,
        first_parent,
        main_tip.as_deref(),
        head_tip.as_deref(),
    );
    // Commits merged into main (for the `↟` unmerged-branch marker).
    let main_ancestors: Option<std::collections::HashSet<String>> = main_tip
        .as_deref()
        .and_then(|sha| commits.iter().position(|c| c.id == sha))
        .map(|mi| ancestors(&commits, mi));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let divergence: std::collections::HashMap<String, (usize, usize)> = branches
        .iter()
        .filter(|b| b.kind == RefKind::LocalBranch)
        .map(|b| (b.name.clone(), (b.ahead, b.behind)))
        .collect();

    // Highlight: the selected branch of the --lens / --focus commit.
    let highlight: Option<crate::features::graph::layout::Highlight> =
        lens.as_deref().or(focus.as_deref()).and_then(|f| {
            commits
                .iter()
                .position(|c| c.id == f || c.short_id == f || c.id.starts_with(f))
                .map(|idx| {
                    branch_highlight_main(
                        &commits,
                        idx,
                        first_parent,
                        main_tip.as_deref(),
                        head_tip.as_deref(),
                    )
                })
        });

    // Fork point (lens mode): newest commit shared by the lens tip and main.
    let fork: Option<String> = lens.as_deref().and_then(|tip| {
        let (_, base_sha) = main.as_ref()?;
        let ti = commits
            .iter()
            .position(|c| c.id == tip || c.id.starts_with(tip))?;
        let bi = commits.iter().position(|c| &c.id == base_sha)?;
        let a = ancestors(&commits, ti);
        let b = ancestors(&commits, bi);
        commits
            .iter()
            .find(|c| a.contains(&c.id) && b.contains(&c.id))
            .map(|c| c.id.clone())
    });

    let lane_hex: Vec<String> = theme.lane.iter().map(|c| hex(*c)).collect();
    let dim = hex(theme.dim);
    let fg = hex(theme.fg);
    let head = hex(theme.head);
    let tagc = hex(theme.unstaged);
    let added = hex(theme.added);
    let removed = hex(theme.removed);

    // The selected branch is drawn in ONE colour (its own lane colour) so boundary
    // nodes and won crossings match the branch instead of the crossed lane.
    let branch_hex: Option<String> = highlight
        .as_ref()
        .and_then(|h| crate::features::graph::render::branch_color_index(&rows, h))
        .and_then(|i| lane_hex.get(i % lane_hex.len().max(1)).cloned());

    let mut out = String::new();
    out.push_str(&format!(
        "<pre style=\"background:#1a1b26;color:{fg};padding:16px 20px;border-radius:10px;\
         font:13px/1.5 ui-monospace,Menlo,monospace;overflow:auto;margin:0\">"
    ));

    for row in &rows {
        for cell in &row.cells {
            if cell.symbol == ' ' {
                out.push(' ');
            } else {
                let (g, is_dim) =
                    crate::features::graph::layout::cell_glyph(cell, highlight.as_ref());
                let color = if is_dim {
                    dim.clone()
                } else if let Some(bc) = &branch_hex {
                    bc.clone()
                } else {
                    lane_hex
                        .get(cell.lane % lane_hex.len().max(1))
                        .cloned()
                        .unwrap_or_else(|| fg.clone())
                };
                out.push_str(&format!(
                    "<span style=\"color:{color}\">{}</span>",
                    esc(&g.to_string())
                ));
            }
        }
        if row.is_node_row {
            if let Some(c) = commits.get(row.commit_index) {
                let meta_dim = highlight
                    .as_ref()
                    .map(|h| !h.nodes.contains(&c.id))
                    .unwrap_or(false);
                out.push_str(&format!(
                    "  <span style=\"color:{dim}\">{}</span>  ",
                    esc(&c.short_id)
                ));
                if fork.as_deref() == Some(c.id.as_str()) {
                    out.push_str(&format!(
                        "<span style=\"color:{};font-weight:700\">⑂base</span> ",
                        hex(theme.focus_border)
                    ));
                }
                let unmerged_into_main = main_ancestors
                    .as_ref()
                    .map(|a| !a.contains(&c.id))
                    .unwrap_or(false);
                for r in &c.refs {
                    let (txt, col, bold) = if meta_dim {
                        let txt = match r.kind {
                            RefKind::Head => format!("HEAD→{}", r.name),
                            RefKind::LocalBranch => format!("[{}]", r.name),
                            RefKind::RemoteBranch => format!("({})", r.name),
                            RefKind::Tag => format!("◆{}", r.name),
                        };
                        (txt, dim.clone(), false)
                    } else {
                        match r.kind {
                            RefKind::Head => (format!("HEAD→{}", r.name), head.clone(), true),
                            RefKind::LocalBranch => {
                                let lc = branch_hex.clone().unwrap_or_else(|| {
                                    row.cells
                                        .iter()
                                        .find(|x| matches!(x.symbol, '●' | '◉' | '◆'))
                                        .map(|x| lane_hex[x.lane % lane_hex.len()].clone())
                                        .unwrap_or_else(|| fg.clone())
                                });
                                (format!("[{}]", r.name), lc, true)
                            }
                            RefKind::RemoteBranch => (format!("({})", r.name), dim.clone(), false),
                            RefKind::Tag => (format!("◆{}", r.name), tagc.clone(), false),
                        }
                    };
                    let w = if bold { "font-weight:700" } else { "" };
                    out.push_str(&format!(
                        "<span style=\"color:{col};{w}\">{}</span> ",
                        esc(&txt)
                    ));
                    // `↟` = branch sits off main with unmerged work (WIP).
                    if matches!(r.kind, RefKind::LocalBranch | RefKind::Head)
                        && unmerged_into_main
                        && !meta_dim
                    {
                        out.push_str(&format!(
                            "<span style=\"color:{};font-weight:700\">↟</span> ",
                            head
                        ));
                    }
                    // Divergence vs upstream (incl. the checked-out HEAD→<name>).
                    if matches!(r.kind, RefKind::LocalBranch | RefKind::Head) {
                        if let Some(&(a, b)) = divergence.get(&r.name) {
                            if a > 0 {
                                let c2 = if meta_dim { &dim } else { &added };
                                out.push_str(&format!("<span style=\"color:{c2}\">↑{a}</span> "));
                            }
                            if b > 0 {
                                let c2 = if meta_dim {
                                    dim.clone()
                                } else {
                                    removed.clone()
                                };
                                let w2 = if meta_dim { "" } else { "font-weight:700" };
                                out.push_str(&format!(
                                    "<span style=\"color:{c2};{w2}\">↓{b}</span> "
                                ));
                            }
                        }
                    }
                }
                let sumc = if meta_dim { &dim } else { &fg };
                out.push_str(&format!(
                    "<span style=\"color:{sumc}\">{}</span>",
                    esc(&c.summary)
                ));
                out.push_str(&format!(
                    "  <span style=\"color:{dim}\">{}</span>",
                    relative_age(c.time, now)
                ));
            }
        }
        out.push('\n');
    }
    out.push_str("</pre>");
    print!("{out}");
    Ok(())
}

/// Print a parsed diff for the repo at `path`.
///
/// Output format:
///   `<N> file(s) changed:`
///   `  <old_path> -> <new_path> (<N> hunk(s))[ [binary]]`
///   `    @@ hunk header @@`
///   `    [+/-/ ]<line content>`
pub fn run_diff(path: &Path, staged: bool, file: Option<&str>) -> anyhow::Result<()> {
    let backend = git::open(path)?;

    let diff = if let Some(f) = file {
        backend.file_diff(f, staged)
    } else {
        backend.worktree_diff(staged)
    };

    match diff {
        Ok(d) => {
            println!("{} file(s) changed:", d.files.len());
            for f in &d.files {
                let added: usize = f
                    .hunks
                    .iter()
                    .flat_map(|h| h.lines.iter())
                    .filter(|l| matches!(l.kind, crate::git::types::DiffLineKind::Added))
                    .count();
                let removed: usize = f
                    .hunks
                    .iter()
                    .flat_map(|h| h.lines.iter())
                    .filter(|l| matches!(l.kind, crate::git::types::DiffLineKind::Removed))
                    .count();

                println!(
                    "  {} -> {} ({} hunk(s), +{} -{}){}",
                    f.old_path,
                    f.new_path,
                    f.hunks.len(),
                    added,
                    removed,
                    if f.is_binary { " [binary]" } else { "" }
                );

                for h in &f.hunks {
                    println!("    {}", h.header);
                    for l in &h.lines {
                        use crate::git::types::DiffLineKind::*;
                        // Skip Header lines — the hunk header is already printed
                        // above via h.header to avoid duplicate output.
                        if l.kind == Header {
                            continue;
                        }
                        let prefix = match l.kind {
                            Added => "+",
                            Removed => "-",
                            Context => " ",
                            Header | Meta => "#",
                        };
                        println!("    {}{}", prefix, l.text);
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("diff error: {e:#}");
        }
    }
    Ok(())
}

/// Print all branches (local + remote) for the repo at `path`.
///
/// Output format (one line per branch):
///   `[*] <kind>  <name>  <short_target>[ upstream: <upstream> +<ahead> -<behind>]`
pub fn run_branches(path: &Path) -> anyhow::Result<()> {
    let backend = git::open(path)?;
    match backend.branches() {
        Ok(branches) => {
            println!("{} branch(es):", branches.len());
            for b in &branches {
                let head_marker = if b.is_head { "* " } else { "  " };
                let kind = format!("{:?}", b.kind);
                let target = if b.target.len() >= 7 {
                    &b.target[..7]
                } else {
                    &b.target
                };
                let upstream_info = if let Some(ref up) = b.upstream {
                    format!("  upstream: {} +{} -{}", up, b.ahead, b.behind)
                } else {
                    String::new()
                };
                println!(
                    "  {}{:<15}  {}  {}{}",
                    head_marker, b.name, kind, target, upstream_info
                );
            }
        }
        Err(e) => {
            eprintln!("branches error: {e:#}");
        }
    }
    Ok(())
}

/// Print all worktrees for the repo at `path`.
///
/// Output format (one line per worktree):
///   `[*] <path>  branch: <branch>  head: <short_head>[ locked][ bare]`
pub fn run_worktrees(path: &Path) -> anyhow::Result<()> {
    let backend = git::open(path)?;
    match backend.worktrees() {
        Ok(worktrees) => {
            println!("{} worktree(s):", worktrees.len());
            for wt in &worktrees {
                let current = if wt.is_current { "* " } else { "  " };
                let branch = wt.branch.as_deref().unwrap_or("(detached)");
                let short_head = if wt.head.len() >= 7 {
                    &wt.head[..7]
                } else {
                    &wt.head
                };
                let flags = {
                    let mut f = Vec::new();
                    if wt.is_locked {
                        f.push("locked");
                    }
                    if wt.is_bare {
                        f.push("bare");
                    }
                    if f.is_empty() {
                        String::new()
                    } else {
                        format!("  [{}]", f.join(", "))
                    }
                };
                println!(
                    "  {}{}  branch: {}  head: {}{}",
                    current, wt.path, branch, short_head, flags
                );
            }
        }
        Err(e) => {
            eprintln!("worktrees error: {e:#}");
        }
    }
    Ok(())
}

/// Print all stashes for the repo at `path`.
///
/// Output format (one line per stash):
///   `stash@{<index>}  <short_oid>  <message>`
pub fn run_stashes(path: &Path) -> anyhow::Result<()> {
    let backend = git::open(path)?;
    match backend.stashes() {
        Ok(stashes) => {
            println!("{} stash(es):", stashes.len());
            for s in &stashes {
                let short_oid = if s.oid.len() >= 7 {
                    &s.oid[..7]
                } else {
                    &s.oid
                };
                println!("  stash@{{{}}}  {}  {}", s.index, short_oid, s.message);
            }
        }
        Err(e) => {
            eprintln!("stashes error: {e:#}");
        }
    }
    Ok(())
}

/// Print the current in-progress git operation (merge/rebase/cherry-pick/revert) for the
/// repo at `path`, or "none" if no operation is in progress.
///
/// Output format:
///   `operation: <kind>` / `none`
///   `conflicted files:` (if any)
///     `  <path>`
pub fn run_op_status(path: &Path) -> anyhow::Result<()> {
    let backend = git::open(path)?;
    match backend.operation_in_progress() {
        Ok(Some(op)) => {
            println!("operation: {:?}", op.kind);
            if op.conflicted.is_empty() {
                println!("no conflicted files");
            } else {
                println!("conflicted files ({}):", op.conflicted.len());
                for f in &op.conflicted {
                    println!("  {f}");
                }
            }
        }
        Ok(None) => {
            println!("none");
        }
        Err(e) => {
            eprintln!("op-status error: {e:#}");
        }
    }
    Ok(())
}

/// Print all tags for the repo at `path`.
///
/// Output format (one line per tag):
///   `<name>  <short_target>  <message_preview>`
pub fn run_tags(path: &Path) -> anyhow::Result<()> {
    let backend = git::open(path)?;
    match backend.tags() {
        Ok(tags) => {
            println!("{} tag(s):", tags.len());
            for t in &tags {
                let short_target = if t.target.len() >= 7 {
                    &t.target[..7]
                } else {
                    &t.target
                };
                // Truncate long messages.
                let msg_preview = t.message.lines().next().unwrap_or("").trim();
                let msg_preview = if msg_preview.len() > 60 {
                    format!("{}…", &msg_preview[..60])
                } else {
                    msg_preview.to_string()
                };
                println!("  {:<30}  {}  {}", t.name, short_target, msg_preview);
            }
        }
        Err(e) => {
            eprintln!("tags error: {e:#}");
        }
    }
    Ok(())
}
