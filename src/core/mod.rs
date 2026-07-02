//! Application core — the shared foundation every feature is built on.
//!
//! This module holds the cross-cutting pieces of the Elm-style architecture:
//!
//! - [`app`] — the central model (`App`) and the data it owns.
//! - [`action`] / [`effect`] — the message and side-effect types.
//! - [`update`] — the central dispatcher; per-mode logic lives in
//!   `crate::features::<mode>::update` and is delegated to from here.
//! - [`keymap`] / [`event`] — input → `Action` translation.
//! - [`runtime`] — terminal setup and the main event loop.
//! - [`config`] / [`theme`] / [`clipboard`] — shared services.
//!
//! Mode-specific behaviour lives under `crate::features`; shared presentation
//! lives under `crate::ui`; the git backend lives under `crate::git`.

pub mod action;
pub mod app;
pub mod clipboard;
pub mod config;
pub mod dialog;
pub mod effect;
pub mod event;
pub mod keymap;
pub mod palette;
pub mod runtime;
pub mod search;
pub mod theme;
pub mod update;
