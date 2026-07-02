//! Inspect mode — the state owned by the feature: the ref the user entered and
//! the commit + diff it resolved to. Held by [`crate::app::App`] in its
//! `inspect` field.

/// State for Inspect mode: the resolved commit + its diff for an entered ref.
#[derive(Debug, Clone, Default)]
pub struct InspectState {
    /// The ref that was last submitted (shown in the header).
    pub query: String,
    /// The resolved commit metadata, if resolution succeeded.
    pub commit: Option<crate::git::types::Commit>,
    /// The resolved commit's diff.
    pub diff: Option<crate::git::types::Diff>,
    /// Error message if the ref could not be resolved.
    pub error: Option<String>,
}
