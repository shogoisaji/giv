//! Graph mode — the commit history visualizer.
//!
//! - [`layout`] computes the lane/glyph layout of the commit DAG.
//! - [`render`] turns that layout into styled ratatui lines.
//! - [`view`] renders the Graph mode panels (graph list + commit detail).
//! - [`rebase_todo`] renders the interactive-rebase todo-editor overlay.

pub mod keymap;
pub mod layout;
pub mod rebase_todo;
pub mod render;
pub mod update;
pub mod view;
