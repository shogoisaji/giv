//! Overlay dialog state — the input prompts and confirmation guard shared by
//! every mode. Rendered by `crate::ui::overlay`; opened/edited by the feature
//! update modules and the central dispatcher in [`crate::update`].

/// Overlay dialog state.
#[derive(Debug, Clone)]
pub enum Dialog {
    None,
    /// Commit message being typed. Inner `String` is the current draft.
    Commit(String),
    /// Amend the last commit — inner `String` is the (pre-filled) message draft.
    Amend(String),
    /// New branch name being typed.
    NewBranch(String),
    /// Rename a branch: `old` is the existing name, `new` the draft replacement.
    RenameBranch {
        old: String,
        new: String,
    },
    /// Worktree path being typed.
    WorktreeAdd(String),
    /// Stash message being typed (empty is fine — means no message).
    StashSave(String),
    /// Tag name being typed. Second String is the optional annotation message.
    TagCreate {
        name: String,
        message: String,
        /// `true` when cursor is in the "message" field; `false` = name field.
        focus_message: bool,
    },
    /// Reset-mode picker: shows Soft / Mixed / Hard options.
    ResetMenu {
        /// OID of the commit to reset to.
        target: String,
    },
    /// Generic yes/no confirmation with a pending operation label.
    Confirm {
        message: String,
        /// Identifies what action to execute upon confirmation.
        pending: ConfirmOp,
    },
    /// Commit ref (sha / branch / tag / HEAD~1 …) being typed in Inspect mode.
    InspectRef(String),
    /// Branch-compare picker: both base and target are shown in one dialog.
    /// `focus_target` = false → editing base, true → editing target.
    /// `base` / `target` are the current filter queries for each field.
    CompareBranches {
        base: String,
        target: String,
        focus_target: bool,
    },
}

impl Dialog {
    /// The text field currently being edited, if this dialog has one. Centralises
    /// the "which variant/field is the active text buffer" mapping so the input
    /// handlers and submit handlers don't each repeat the same match.
    pub fn active_text_mut(&mut self) -> Option<&mut String> {
        match self {
            Dialog::Commit(d)
            | Dialog::Amend(d)
            | Dialog::NewBranch(d)
            | Dialog::WorktreeAdd(d)
            | Dialog::StashSave(d)
            | Dialog::InspectRef(d) => Some(d),
            Dialog::RenameBranch { new, .. } => Some(new),
            Dialog::TagCreate {
                name,
                message,
                focus_message,
            } => Some(if *focus_message { message } else { name }),
            Dialog::CompareBranches {
                base,
                target,
                focus_target,
            } => Some(if *focus_target { target } else { base }),
            _ => None,
        }
    }

    /// Resolve the submit payload: when the keymap sends an empty payload
    /// (the Enter path), read and drain the live draft text from the active
    /// dialog field instead.  This centralises the
    /// `if payload.is_empty() { active_text_mut().map(take) } else { payload }`
    /// pattern that was duplicated across every feature's submit handler.
    pub fn take_text_or(&mut self, payload: String) -> String {
        if payload.is_empty() {
            self.active_text_mut()
                .map(std::mem::take)
                .unwrap_or(payload)
        } else {
            payload
        }
    }
}

/// Identifies what destructive operation a Confirm dialog is guarding.
#[derive(Debug, Clone)]
pub enum ConfirmOp {
    DeleteBranch {
        name: String,
        force: bool,
    },
    RemoveWorktree {
        path: String,
        force: bool,
    },
    StashDrop {
        index: usize,
    },
    Reset {
        mode: crate::git::ResetMode,
        target: String,
    },
    TagDelete {
        name: String,
    },
    /// Cherry-pick a commit onto HEAD.
    CherryPick {
        oid: String,
    },
    /// Revert a commit (creates a new commit).
    Revert {
        oid: String,
    },
    /// Rebase HEAD onto a target (`display` is shown to the user).
    RebaseOnto {
        target: String,
        display: String,
    },
    /// Merge a branch into HEAD.
    Merge {
        branch: String,
    },
    /// `git fetch --all` — download remote refs without touching the working tree.
    Fetch,
    /// `git pull --ff-only` — fast-forward the current branch into its upstream.
    Pull,
    /// `git push` with pre-computed args (the caller resolves `--set-upstream`
    /// and the remote/branch to use). `args` excludes the leading `git`.
    Push {
        args: Vec<String>,
    },
    /// `git push --force-with-lease` with pre-computed args. `args` excludes
    /// the leading `git`.
    ForcePush {
        args: Vec<String>,
    },
}

impl ConfirmOp {
    /// The git command this operation will run, shown in the confirm dialog so
    /// the user can see exactly what is about to execute before approving it.
    pub fn command_preview(&self) -> String {
        // Abbreviate full OIDs to 7 chars for readability.
        use crate::git::short_oid as short;
        match self {
            ConfirmOp::DeleteBranch { name, force } => {
                format!("git branch -{} {name}", if *force { "D" } else { "d" })
            }
            ConfirmOp::RemoveWorktree { path, force } => {
                if *force {
                    format!("git worktree remove --force {path}")
                } else {
                    format!("git worktree remove {path}")
                }
            }
            ConfirmOp::StashDrop { index } => format!("git stash drop stash@{{{index}}}"),
            ConfirmOp::Reset { mode, target } => {
                let flag = match mode {
                    crate::git::ResetMode::Soft => "--soft",
                    crate::git::ResetMode::Mixed => "--mixed",
                    crate::git::ResetMode::Hard => "--hard",
                };
                format!("git reset {flag} {}", short(target))
            }
            ConfirmOp::TagDelete { name } => format!("git tag -d {name}"),
            ConfirmOp::CherryPick { oid } => format!("git cherry-pick {}", short(oid)),
            ConfirmOp::Revert { oid } => format!("git revert {}", short(oid)),
            ConfirmOp::RebaseOnto { display, .. } => format!("git rebase {display}"),
            // MergeSelected runs `merge(branch, no_ff=false)`, i.e. a normal
            // (fast-forward-allowed) merge — keep the preview accurate.
            ConfirmOp::Merge { branch } => format!("git merge {branch}"),
            ConfirmOp::Fetch => "git fetch --all".to_string(),
            ConfirmOp::Pull => "git pull --ff-only".to_string(),
            ConfirmOp::Push { args } | ConfirmOp::ForcePush { args } => {
                format!("git {}", args.join(" "))
            }
        }
    }
}

#[cfg(test)]
mod confirm_op_tests {
    use super::ConfirmOp;

    #[test]
    fn command_preview_matches_real_commands() {
        assert_eq!(
            ConfirmOp::CherryPick {
                oid: "abcdef1234567".into()
            }
            .command_preview(),
            "git cherry-pick abcdef1"
        );
        assert_eq!(
            ConfirmOp::Revert {
                oid: "abcdef1234567".into()
            }
            .command_preview(),
            "git revert abcdef1"
        );
        assert_eq!(
            ConfirmOp::Reset {
                mode: crate::git::ResetMode::Hard,
                target: "abcdef1234567".into()
            }
            .command_preview(),
            "git reset --hard abcdef1"
        );
        assert_eq!(
            ConfirmOp::Reset {
                mode: crate::git::ResetMode::Soft,
                target: "abcdef1234567".into()
            }
            .command_preview(),
            "git reset --soft abcdef1"
        );
        assert_eq!(
            ConfirmOp::Reset {
                mode: crate::git::ResetMode::Mixed,
                target: "abcdef1234567".into()
            }
            .command_preview(),
            "git reset --mixed abcdef1"
        );
        assert_eq!(
            ConfirmOp::DeleteBranch {
                name: "feat".into(),
                force: true
            }
            .command_preview(),
            "git branch -D feat"
        );
        assert_eq!(
            ConfirmOp::DeleteBranch {
                name: "feat".into(),
                force: false
            }
            .command_preview(),
            "git branch -d feat"
        );
        assert_eq!(
            ConfirmOp::StashDrop { index: 2 }.command_preview(),
            "git stash drop stash@{2}"
        );
        assert_eq!(
            ConfirmOp::Merge {
                branch: "dev".into()
            }
            .command_preview(),
            "git merge dev"
        );
        assert_eq!(
            ConfirmOp::RebaseOnto {
                target: "abcdef1234567".into(),
                display: "main".into()
            }
            .command_preview(),
            "git rebase main"
        );
        // Short oid is not over-truncated.
        assert_eq!(
            ConfirmOp::CherryPick { oid: "abc".into() }.command_preview(),
            "git cherry-pick abc"
        );
    }

    #[test]
    fn command_preview_remove_worktree_force_and_non_force() {
        assert_eq!(
            ConfirmOp::RemoveWorktree {
                path: "/tmp/wt".into(),
                force: true
            }
            .command_preview(),
            "git worktree remove --force /tmp/wt"
        );
        assert_eq!(
            ConfirmOp::RemoveWorktree {
                path: "/tmp/wt".into(),
                force: false
            }
            .command_preview(),
            "git worktree remove /tmp/wt"
        );
    }

    #[test]
    fn command_preview_tag_delete() {
        assert_eq!(
            ConfirmOp::TagDelete {
                name: "v1.0".into()
            }
            .command_preview(),
            "git tag -d v1.0"
        );
    }

    #[test]
    fn command_preview_stash_drop_index_zero() {
        assert_eq!(
            ConfirmOp::StashDrop { index: 0 }.command_preview(),
            "git stash drop stash@{0}"
        );
    }

    #[test]
    fn command_preview_fetch() {
        assert_eq!(ConfirmOp::Fetch.command_preview(), "git fetch --all");
    }

    #[test]
    fn command_preview_pull() {
        assert_eq!(ConfirmOp::Pull.command_preview(), "git pull --ff-only");
    }

    #[test]
    fn command_preview_push_plain() {
        assert_eq!(
            ConfirmOp::Push {
                args: vec!["push".into()]
            }
            .command_preview(),
            "git push"
        );
    }

    #[test]
    fn command_preview_push_set_upstream() {
        assert_eq!(
            ConfirmOp::Push {
                args: vec![
                    "push".into(),
                    "--set-upstream".into(),
                    "origin".into(),
                    "feat".into()
                ]
            }
            .command_preview(),
            "git push --set-upstream origin feat"
        );
    }

    #[test]
    fn command_preview_force_push_with_lease() {
        assert_eq!(
            ConfirmOp::ForcePush {
                args: vec![
                    "push".into(),
                    "--force-with-lease".into(),
                    "origin".into(),
                    "feat".into()
                ]
            }
            .command_preview(),
            "git push --force-with-lease origin feat"
        );
    }
}

#[cfg(test)]
mod dialog_tests {
    use super::Dialog;

    // ── active_text_mut ──────────────────────────────────────────────────────

    #[test]
    fn active_text_mut_commit() {
        let mut d = Dialog::Commit("hello".into());
        assert_eq!(d.active_text_mut().map(|s| s.clone()), Some("hello".into()));
    }

    #[test]
    fn active_text_mut_amend() {
        let mut d = Dialog::Amend("amend msg".into());
        assert_eq!(
            d.active_text_mut().map(|s| s.clone()),
            Some("amend msg".into())
        );
    }

    #[test]
    fn active_text_mut_new_branch() {
        let mut d = Dialog::NewBranch("feat".into());
        assert_eq!(d.active_text_mut().map(|s| s.clone()), Some("feat".into()));
    }

    #[test]
    fn active_text_mut_worktree_add() {
        let mut d = Dialog::WorktreeAdd("/path".into());
        assert_eq!(d.active_text_mut().map(|s| s.clone()), Some("/path".into()));
    }

    #[test]
    fn active_text_mut_stash_save() {
        let mut d = Dialog::StashSave("msg".into());
        assert_eq!(d.active_text_mut().map(|s| s.clone()), Some("msg".into()));
    }

    #[test]
    fn active_text_mut_inspect_ref() {
        let mut d = Dialog::InspectRef("HEAD~1".into());
        assert_eq!(
            d.active_text_mut().map(|s| s.clone()),
            Some("HEAD~1".into())
        );
    }

    #[test]
    fn active_text_mut_rename_branch_returns_new_field() {
        let mut d = Dialog::RenameBranch {
            old: "old".into(),
            new: "new_name".into(),
        };
        assert_eq!(
            d.active_text_mut().map(|s| s.clone()),
            Some("new_name".into())
        );
    }

    #[test]
    fn active_text_mut_tag_create_name_field_when_not_focused() {
        let mut d = Dialog::TagCreate {
            name: "v1.0".into(),
            message: "release".into(),
            focus_message: false,
        };
        assert_eq!(d.active_text_mut().map(|s| s.clone()), Some("v1.0".into()));
    }

    #[test]
    fn active_text_mut_tag_create_message_field_when_focused() {
        let mut d = Dialog::TagCreate {
            name: "v1.0".into(),
            message: "release".into(),
            focus_message: true,
        };
        assert_eq!(
            d.active_text_mut().map(|s| s.clone()),
            Some("release".into())
        );
    }

    #[test]
    fn active_text_mut_compare_branches_base_when_not_focused() {
        let mut d = Dialog::CompareBranches {
            base: "main".into(),
            target: "feat".into(),
            focus_target: false,
        };
        assert_eq!(d.active_text_mut().map(|s| s.clone()), Some("main".into()));
    }

    #[test]
    fn active_text_mut_compare_branches_target_when_focused() {
        let mut d = Dialog::CompareBranches {
            base: "main".into(),
            target: "feat".into(),
            focus_target: true,
        };
        assert_eq!(d.active_text_mut().map(|s| s.clone()), Some("feat".into()));
    }

    #[test]
    fn active_text_mut_none_for_non_text_dialogs() {
        let mut none = Dialog::None;
        assert!(none.active_text_mut().is_none());

        let mut confirm = Dialog::Confirm {
            message: "ok?".into(),
            pending: super::ConfirmOp::TagDelete { name: "v1".into() },
        };
        assert!(confirm.active_text_mut().is_none());

        let mut reset = Dialog::ResetMenu {
            target: "abc".into(),
        };
        assert!(reset.active_text_mut().is_none());
    }

    // ── take_text_or ──────────────────────────────────────────────────────────

    #[test]
    fn take_text_or_returns_payload_when_non_empty() {
        let mut d = Dialog::Commit("draft".into());
        // Non-empty payload wins, draft is NOT drained.
        assert_eq!(d.take_text_or("payload".into()), "payload");
        assert_eq!(d.active_text_mut().map(|s| s.clone()), Some("draft".into()));
    }

    #[test]
    fn take_text_or_drains_draft_when_payload_empty() {
        let mut d = Dialog::Commit("draft".into());
        assert_eq!(d.take_text_or(String::new()), "draft");
        // Draft was drained — active_text_mut is now empty.
        assert_eq!(d.active_text_mut().map(|s| s.clone()), Some(String::new()));
    }

    #[test]
    fn take_text_or_returns_empty_when_no_active_text_and_payload_empty() {
        let mut d = Dialog::None;
        assert_eq!(d.take_text_or(String::new()), "");
    }

    #[test]
    fn take_text_or_drains_tag_name_field() {
        let mut d = Dialog::TagCreate {
            name: "v2.0".into(),
            message: "msg".into(),
            focus_message: false,
        };
        assert_eq!(d.take_text_or(String::new()), "v2.0");
        // Name was drained, message is untouched.
        if let Dialog::TagCreate { name, message, .. } = &d {
            assert_eq!(name, "");
            assert_eq!(message, "msg");
        } else {
            panic!("dialog variant changed");
        }
    }

    #[test]
    fn take_text_or_drains_compare_base_field() {
        let mut d = Dialog::CompareBranches {
            base: "main".into(),
            target: "feat".into(),
            focus_target: false,
        };
        assert_eq!(d.take_text_or(String::new()), "main");
        if let Dialog::CompareBranches { base, target, .. } = &d {
            assert_eq!(base, "");
            assert_eq!(target, "feat");
        } else {
            panic!("dialog variant changed");
        }
    }

    #[test]
    fn take_text_or_drains_compare_target_field() {
        let mut d = Dialog::CompareBranches {
            base: "main".into(),
            target: "feat".into(),
            focus_target: true,
        };
        assert_eq!(d.take_text_or(String::new()), "feat");
        if let Dialog::CompareBranches { base, target, .. } = &d {
            assert_eq!(base, "main");
            assert_eq!(target, "");
        } else {
            panic!("dialog variant changed");
        }
    }

    #[test]
    fn take_text_or_drains_rename_branch_new_field() {
        let mut d = Dialog::RenameBranch {
            old: "old".into(),
            new: "new_name".into(),
        };
        assert_eq!(d.take_text_or(String::new()), "new_name");
        if let Dialog::RenameBranch { old, new } = &d {
            assert_eq!(old, "old");
            assert_eq!(new, "");
        } else {
            panic!("dialog variant changed");
        }
    }
}
