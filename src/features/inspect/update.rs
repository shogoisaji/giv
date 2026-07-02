//! Inspect mode — update logic: resolve an entered ref to a commit + its diff.

use crate::app::{App, Dialog};
use crate::effect::Effect;

/// Resolve and display the commit for the ref entered in the Inspect prompt.
///
/// The keymap sends an empty payload; the draft is then read from the dialog.
pub(crate) fn submit(app: &mut App, payload: String) -> Effect {
    let raw = app.dialog.take_text_or(payload);
    app.dialog = Dialog::None;
    let rev = raw.trim().to_string();
    if rev.is_empty() {
        return Effect::Refresh;
    }
    app.inspect.query = rev.clone();
    app.ui.diff_scroll = 0;
    match app.repo.backend.commit_info(&rev) {
        Ok(commit) => {
            let diff = app.repo.backend.commit_diff(&commit.id).ok();
            app.inspect.commit = Some(commit);
            app.inspect.diff = diff;
            app.inspect.error = None;
            app.status_message = Some(format!("Showing {rev}"));
        }
        Err(e) => {
            app.inspect.commit = None;
            app.inspect.diff = None;
            app.inspect.error = Some(format!("{e:#}"));
            app.status_message = Some(format!("Cannot resolve '{rev}'"));
        }
    }
    Effect::Refresh
}
