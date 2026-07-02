//! Overlays — transient UI drawn on top of the active mode: the centered modal
//! primitive ([`modal`]) and the dialogs/panels built on it ([`command_palette`],
//! [`confirm_dialog`], [`help`]).

pub mod command_palette;
pub mod confirm_dialog;
pub mod help;
pub mod modal;
