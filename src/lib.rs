pub mod core;
pub mod debug;
pub mod features;
pub mod git;
pub mod ui;

// ─── Facade re-exports ─────────────────────────────────────────────────────────
//
// The application core physically lives under `core/`, but it is re-exported at
// the crate root so the pre-refactor module paths (`crate::app::…`,
// `giv::update::…`, …) keep resolving. This lets the wide web of existing
// `use` statements across the codebase stay untouched by the move.
pub use crate::core::{
    action, app, clipboard, config, effect, event, keymap, palette, search, theme, update,
};

#[cfg(test)]
mod test_backend;

#[cfg(test)]
mod tests;
