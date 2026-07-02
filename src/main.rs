use clap::{Parser, Subcommand};
use std::path::PathBuf;

use giv::core::runtime;

// ─── CLI definition ──────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "giv", version, about = "Terminal git visualizer")]
struct Cli {
    /// Path to the repository (defaults to the current directory).
    #[arg(global = true, default_value = ".")]
    path: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Non-TTY debug / verification subcommands.
    Debug {
        #[command(subcommand)]
        sub: DebugSub,
    },
}

#[derive(Subcommand, Debug)]
enum DebugSub {
    /// Print name and version, then exit.
    Version,
    /// Print parsed working status.
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print recent commits.
    Log {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Maximum number of commits to show.
        #[arg(short = 'n', default_value = "20")]
        limit: usize,
    },
    /// Print the computed commit graph as ASCII text.
    Graph {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(short = 'n', default_value = "20")]
        limit: usize,
        /// Render in spacious mode (a blank edge row between commits).
        #[arg(long)]
        spacious: bool,
        /// Scope to HEAD's history only (default walks all refs, like `--all`).
        #[arg(long)]
        head: bool,
    },
    /// Render the commit graph as a colored HTML fragment (for visual review).
    GraphHtml {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(short = 'n', default_value = "40")]
        limit: usize,
        /// Render in spacious mode (a blank edge row between commits).
        #[arg(long)]
        spacious: bool,
        /// Focus a commit (sha / short-sha): dim everything outside its lineage.
        #[arg(long)]
        focus: Option<String>,
        /// Scope to HEAD's history only (default walks all refs, like `--all`).
        #[arg(long)]
        head: bool,
        /// Follow only first parents — collapse merges into a straight trunk.
        #[arg(long)]
        first_parent: bool,
        /// Branch lens: focus a commit (sha) vs main — union of both histories.
        #[arg(long)]
        lens: Option<String>,
    },
    /// Print a parsed diff.
    Diff {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Show staged (index vs HEAD) instead of unstaged.
        #[arg(long)]
        staged: bool,
        /// Restrict diff to a specific file.
        file: Option<String>,
    },
    /// Print all branches (local + remote).
    Branches {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print all worktrees.
    Worktrees {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print all tags.
    Tags {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print all stash entries.
    Stashes {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print the currently in-progress git operation (merge / rebase / etc.), if any.
    OpStatus {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Debug { sub }) => run_debug(sub),
        None => runtime::run_tui(cli.path),
    }
}

// ─── Debug dispatch ──────────────────────────────────────────────────────────

fn run_debug(sub: DebugSub) -> anyhow::Result<()> {
    use giv::debug;

    match sub {
        DebugSub::Version => {
            debug::run_version();
            Ok(())
        }
        DebugSub::Status { path } => debug::run_status(&path),
        DebugSub::Log { path, limit } => debug::run_log(&path, limit),
        DebugSub::Graph {
            path,
            limit,
            spacious,
            head,
        } => debug::run_graph(&path, limit, spacious, !head),
        DebugSub::GraphHtml {
            path,
            limit,
            spacious,
            focus,
            head,
            first_parent,
            lens,
        } => debug::run_graph_html(&path, limit, spacious, focus, !head, first_parent, lens),
        DebugSub::Diff { path, staged, file } => debug::run_diff(&path, staged, file.as_deref()),
        DebugSub::Branches { path } => debug::run_branches(&path),
        DebugSub::Worktrees { path } => debug::run_worktrees(&path),
        DebugSub::Tags { path } => debug::run_tags(&path),
        DebugSub::Stashes { path } => debug::run_stashes(&path),
        DebugSub::OpStatus { path } => debug::run_op_status(&path),
    }
}
