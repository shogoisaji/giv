/// Side-effects produced by `update()`.
///
/// For Phase 0/1 all effects are handled synchronously in the event loop.
/// Background-thread variants (network ops, etc.) are introduced in a later phase.
#[derive(Default)]
pub enum Effect {
    /// No side-effect.
    #[default]
    None,
    /// Ask the event loop to exit.
    Quit,
    /// Ask the event loop to re-draw / reload data.
    Refresh,
    /// Enable (`true`) or disable (`false`) terminal mouse capture. Handled by
    /// the event loop because it must issue a crossterm command on the terminal.
    SetMouseCapture(bool),
    /// Execute multiple effects in sequence.
    Batch(Vec<Effect>),
}
