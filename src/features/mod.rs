//! Feature modules — one directory per top-level [`crate::app::Mode`].
//!
//! Each feature owns the code for a single mode (Status / Graph / Branches /
//! Worktrees / Stashes / Inspect): its `view` (rendering), and — as the refactor
//! progresses — its `update` (action handling), `keymap` (key resolution) and
//! any mode-specific `state`. Cross-cutting infrastructure lives in
//! [`crate::core`]; shared presentation lives in [`crate::ui`].

pub mod branches;
pub mod graph;
pub mod inspect;
pub mod stashes;
pub mod status;
pub mod worktrees;
