/// Smoke tests — exercises the view path without a real TTY.
#[cfg(test)]
mod smoke {
    use std::path::PathBuf;
    use std::process::Command;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};

    use crate::action::Action;
    use crate::app::{App, PaletteItem, PaletteState, Panel, SearchState};
    use crate::config::Config;
    use crate::keymap::{Keymap, KeymapContext};
    use crate::theme::Theme;
    use crate::ui::view;

    /// Build a minimal `App` backed by a real, freshly-initialised git repo
    /// and assert that rendering one frame does not panic.
    #[test]
    fn view_does_not_panic() {
        // Create a temp dir with a git repo.
        let tmp = tempdir_with_git_init();

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let config = Config::default();
        // App::new calls refresh(); status/log return errors from the stub
        // impl, which are silently swallowed as status_message.
        let app = App::new(Box::new(backend), config).expect("App::new");

        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");

        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw frame");
    }

    /// (a) show_help=true: render does not panic, buffer contains a known
    /// help string like "Global".
    #[test]
    fn view_help_overlay_renders_known_string() {
        let tmp = tempdir_with_git_init();
        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");

        // Activate the help overlay.
        app.show_help = true;

        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");

        // Must not panic.
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw frame with help");

        // The terminal buffer must contain some known help text.
        let buf = terminal.backend().buffer().clone();
        let rendered: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(
            rendered.contains("Global") || rendered.contains("Quit") || rendered.contains("Help"),
            "help overlay must render known help text; got (truncated): {}",
            &rendered[..rendered.len().min(200)]
        );
    }

    /// (b) app.palette = Some(state with query and items): render does not
    /// panic and the buffer contains the query text.
    #[test]
    fn view_command_palette_renders_query() {
        let tmp = tempdir_with_git_init();
        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");

        // Set up a command palette with a distinctive query.
        let items = vec![PaletteItem {
            label: "Stage All".to_string(),
            hint: "a".to_string(),
            action: crate::action::Action::StageAll,
        }];
        app.palette = Some(PaletteState {
            query: "stageall".to_string(),
            all_items: items.clone(),
            items,
            cursor: 0,
        });

        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");

        // Must not panic.
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw frame with palette");

        // Buffer must contain the query string.
        let buf = terminal.backend().buffer().clone();
        let rendered: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(
            rendered.contains("stageall") || rendered.contains("Stage"),
            "palette overlay must render query/items; got (truncated): {}",
            &rendered[..rendered.len().min(400)]
        );
    }

    /// (c) app.search = Some(...): render does not panic.
    #[test]
    fn view_search_bar_does_not_panic() {
        let tmp = tempdir_with_git_init();
        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");

        // Open the search bar with a query.
        app.search = Some(SearchState {
            query: "feature".to_string(),
            matches: vec![0],
            current: 0,
        });

        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");

        // Must not panic.
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw frame with search bar");
    }

    /// (d) Untracked file: its diff must actually render (regression — `git diff`
    /// ignores untracked files, so the panel used to be blank), and the file list
    /// must use the single "+" marker rather than a confusing double "? ?".
    #[test]
    fn untracked_file_renders_diff_and_plus_marker() {
        let tmp = tempdir_with_git_init();
        // The only change is one untracked file → it is the initially-selected row.
        std::fs::write(
            tmp.join("newfile.txt"),
            "hello_untracked_line\nsecond_line\n",
        )
        .expect("write untracked file");

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        // Load the diff for the selected entry, exactly like launch does.
        let _ = crate::update::update(&mut app, crate::action::Action::Select);

        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw frame");

        let buf = terminal.backend().buffer().clone();
        let rendered: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(
            rendered.contains("newfile.txt"),
            "file list should show the untracked file name"
        );
        assert!(
            rendered.contains("hello_untracked_line"),
            "diff panel must show the untracked file's added content (regression)"
        );
        assert!(
            !rendered.contains("? ?"),
            "untracked files must not render a confusing double '? ?' marker"
        );
    }

    /// (e) Graph selection is shown by background color ONLY — no `>` arrow
    /// (which shifted the row) and no reverse-video/bold.
    #[test]
    fn graph_selection_uses_background_not_arrow_or_reverse() {
        let tmp = tempdir_with_git_init();
        // Add a second commit so the graph has more than one selectable row.
        git_in(&tmp, &["commit", "--allow-empty", "-m", "second"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        app.ui.graph_index = 0;

        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw graph frame");

        let buf = terminal.backend().buffer().clone();
        let sel = app.theme.selection_bg;

        // The selected commit row must paint its background with selection_bg.
        let bg_count = buf.content().iter().filter(|c| c.bg == sel).count();
        assert!(
            bg_count > 0,
            "selected graph row must use selection_bg as its background"
        );

        // No cell may use reverse-video (the old highlight did).
        let reversed = buf
            .content()
            .iter()
            .filter(|c| c.modifier.contains(ratatui::style::Modifier::REVERSED))
            .count();
        assert_eq!(reversed, 0, "selection must not use reverse video");
    }

    /// (f) Graph mode commit detail shows the SELECTED commit's actual diff
    /// content (so you can inspect what a commit changed), and it scrolls.
    #[test]
    fn graph_commit_detail_shows_diff_content() {
        let tmp = tempdir_with_git_init();
        let git = |args: &[&str]| {
            git_in(&tmp, args);
        };
        std::fs::write(tmp.join("f.txt"), "base\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-m", "base"]);
        std::fs::write(tmp.join("f.txt"), "base\nSCROLL_TEST_LINE\n").unwrap();
        git(&["commit", "-am", "add line"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        app.ui.graph_index = 0; // newest commit (the one that added the line)
        let _ = crate::update::update(&mut app, crate::action::Action::Select);

        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw graph frame");

        let buf = terminal.backend().buffer().clone();
        let rendered: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(
            rendered.contains("SCROLL_TEST_LINE"),
            "graph commit detail must show the selected commit's diff content"
        );
    }

    /// (g) Inspect mode: entering a ref resolves the commit and renders its diff.
    #[test]
    fn inspect_mode_shows_commit_diff_for_entered_ref() {
        let tmp = tempdir_with_git_init();
        let git = |args: &[&str]| {
            git_in(&tmp, args);
        };
        std::fs::write(tmp.join("f.txt"), "INSPECT_LINE\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-m", "add f"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Inspect;
        // Simulate typing "HEAD" into the ref prompt and pressing Enter.
        app.dialog = crate::app::Dialog::InspectRef("HEAD".to_string());
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::SubmitInspect(String::new()),
        );

        assert!(
            app.inspect.commit.is_some(),
            "the entered ref must resolve to a commit"
        );

        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw inspect frame");

        let buf = terminal.backend().buffer().clone();
        let rendered: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            rendered.contains("INSPECT_LINE"),
            "inspect view must show the resolved commit's diff content"
        );
    }

    /// (g2) Switching into Inspect mode lands in navigation ("normal") mode, not
    /// input mode: the ref prompt must NOT auto-open, so the global mode-switch
    /// keys keep working. Pressing `i` (OpenInspectPrompt) is what enters input
    /// mode, and Esc (CancelDialog) leaves it again.
    #[test]
    fn inspect_mode_does_not_auto_open_input_prompt() {
        let tmp = tempdir_with_git_init();
        git_in(&tmp, &["commit", "--allow-empty", "-m", "c0"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");

        // Enter Inspect mode the way the keymap does (6 → SwitchMode(Inspect)).
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::SwitchMode(crate::app::Mode::Inspect),
        );
        assert_eq!(app.mode, crate::app::Mode::Inspect);
        assert!(
            matches!(app.dialog, crate::app::Dialog::None),
            "Inspect mode must start in navigation mode, with no input dialog open"
        );

        // A mode-switch key still works because no dialog is swallowing input.
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::SwitchMode(crate::app::Mode::Status),
        );
        assert_eq!(app.mode, crate::app::Mode::Status);

        // `i` (OpenInspectPrompt) is what enters input mode; Esc leaves it.
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::SwitchMode(crate::app::Mode::Inspect),
        );
        let _ = crate::update::update(&mut app, crate::action::Action::OpenInspectPrompt);
        assert!(
            matches!(app.dialog, crate::app::Dialog::InspectRef(_)),
            "pressing i must open the ref input prompt"
        );
        let _ = crate::update::update(&mut app, crate::action::Action::CancelDialog);
        assert!(
            matches!(app.dialog, crate::app::Dialog::None),
            "Esc must leave input mode and return to navigation mode"
        );
    }

    /// (g3) `M` toggles terminal mouse capture so the terminal can do its own
    /// click-drag text selection. The action must flip the flag and emit the
    /// matching `SetMouseCapture` effect.
    #[test]
    fn toggle_mouse_capture_flips_flag_and_effect() {
        let tmp = tempdir_with_git_init();
        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");

        // Default: mouse capture ON so click-to-jump and wheel-scroll work
        // immediately. Press `M` to toggle off for native text selection.
        assert!(app.mouse_capture, "mouse capture must default to on");

        let eff = crate::update::update(&mut app, crate::action::Action::ToggleMouseCapture);
        assert!(
            !app.mouse_capture,
            "first toggle must turn mouse capture off"
        );
        assert!(
            matches!(eff, crate::effect::Effect::SetMouseCapture(false)),
            "toggling off must emit SetMouseCapture(false)"
        );

        let eff = crate::update::update(&mut app, crate::action::Action::ToggleMouseCapture);
        assert!(
            app.mouse_capture,
            "second toggle must turn mouse capture back on"
        );
        assert!(
            matches!(eff, crate::effect::Effect::SetMouseCapture(true)),
            "toggling on must emit SetMouseCapture(true)"
        );
    }

    #[test]
    fn status_a_aborts_only_when_operation_is_active() {
        let keymap = Keymap;
        let key = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE);

        let normal = keymap.resolve(
            key,
            KeymapContext {
                mode: crate::app::Mode::Status,
                dialog: &crate::app::Dialog::None,
                palette: None,
                search: None,
                show_help: false,
                op_in_progress: false,
            },
        );
        assert!(
            matches!(normal, Action::UnstageAll),
            "Status A should remain UnstageAll when no operation is active"
        );

        let in_operation = keymap.resolve(
            key,
            KeymapContext {
                mode: crate::app::Mode::Status,
                dialog: &crate::app::Dialog::None,
                palette: None,
                search: None,
                show_help: false,
                op_in_progress: true,
            },
        );
        assert!(
            matches!(in_operation, Action::OpAbort),
            "Status A must abort when a git operation is active"
        );
    }

    /// (g4) In Inspect mode, `y` copies the resolved commit's full SHA. We can't
    /// read the OS clipboard (OSC 52 writes to the terminal), so we assert the
    /// success status message instead of "Nothing to copy".
    #[test]
    fn yank_sha_copies_inspected_commit() {
        let tmp = tempdir_with_git_init();
        git_in(&tmp, &["commit", "--allow-empty", "-m", "c0"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Inspect;
        app.dialog = crate::app::Dialog::InspectRef("HEAD".to_string());
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::SubmitInspect(String::new()),
        );
        let sha = app
            .inspect
            .commit
            .as_ref()
            .expect("HEAD must resolve")
            .id
            .clone();

        let _ = crate::update::update(&mut app, crate::action::Action::YankSha);
        let msg = app.status_message.clone().unwrap_or_default();
        assert!(
            msg.starts_with("Copied"),
            "yanking in Inspect mode must report a copy, got: {msg:?}"
        );
        let preview = msg.trim_start_matches("Copied ").trim_end_matches('…');
        assert!(
            sha.starts_with(preview),
            "the copied preview {preview:?} must be a prefix of the inspected SHA {sha:?}"
        );
    }

    /// (h) Graph navigation auto-scrolls so the selected commit stays visible
    /// when it would otherwise move off-screen.
    #[test]
    fn graph_navigation_follows_selection_offscreen() {
        let tmp = tempdir_with_git_init();
        let git = |args: &[&str]| {
            git_in(&tmp, args);
        };
        for i in 0..25 {
            git(&["commit", "--allow-empty", "-m", &format!("c{i}")]);
        }

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        app.ui.graph_viewport.set(10); // simulate a 10-row-tall graph panel

        for _ in 0..20 {
            let _ = crate::update::update(&mut app, crate::action::Action::Down);
        }

        let step = app.config.graph_row_step();
        let sel_row = app.ui.graph_index * step;
        let off = app.ui.graph_offset;
        assert!(
            off <= sel_row && sel_row < off + 10,
            "selected row {sel_row} must stay within the viewport [{off}, {off}+10)"
        );
        assert!(
            off > 0,
            "the graph view must have scrolled to follow the off-screen selection"
        );
    }

    /// (i) Status staging acts on the *highlighted* row. The status list is a
    /// two-group (Staged / Unstaged) logical view, so a file with both staged
    /// and unstaged changes appears in both groups and `list_index` is NOT an
    /// index into the raw `entries` vec. Space on the Staged-group row must
    /// unstage that file, and navigation must be able to reach every row.
    #[test]
    fn status_space_toggle_targets_highlighted_row() {
        use crate::features::status::view::{
            is_selected_staged, partition_entries, resolve_entry, status_row_count,
        };

        let tmp = tempdir_with_git_init();
        let git = |args: &[&str]| {
            git_in(&tmp, args);
        };

        // Commit a tracked file, then stage a change to it and modify it again
        // so f.txt has BOTH staged and unstaged changes.
        std::fs::write(tmp.join("f.txt"), "v1\n").unwrap();
        git(&["add", "f.txt"]);
        git(&["commit", "-m", "add f"]);
        std::fs::write(tmp.join("f.txt"), "v2\n").unwrap();
        git(&["add", "f.txt"]);
        std::fs::write(tmp.join("f.txt"), "v3\n").unwrap();
        // Plus an untracked file (unstaged group only).
        std::fs::write(tmp.join("g.txt"), "new\n").unwrap();

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;

        // f.txt is counted in both groups → 3 logical rows from 2 raw entries.
        assert_eq!(
            app.repo.status.entries.len(),
            2,
            "two distinct files changed"
        );
        assert_eq!(status_row_count(&app), 3, "f.txt appears in both groups");

        // Logical row 0 is f.txt in the Staged group.
        assert_eq!(resolve_entry(&app, 0).unwrap().path, "f.txt");
        assert!(is_selected_staged(&app));

        // Navigation must reach the last logical row (the old `entries.len()`
        // cap stopped one row short, making g.txt unreachable).
        for _ in 0..5 {
            let _ = crate::update::update(&mut app, crate::action::Action::Down);
        }
        assert_eq!(app.ui.list_index, 2, "Down must reach the last logical row");
        assert_eq!(
            resolve_entry(&app, app.ui.list_index).unwrap().path,
            "g.txt"
        );

        // Space on the Staged-group row (index 0) must UNSTAGE f.txt — the old
        // code indexed the raw vec and re-staged it instead.
        app.ui.list_index = 0;
        let _ = crate::update::update(&mut app, crate::action::Action::StageSelected);
        let (staged, _unstaged) = partition_entries(&app);
        assert!(
            !staged.iter().any(|e| e.path == "f.txt"),
            "toggling the Staged row of f.txt must unstage it"
        );
    }

    /// Status navigation auto-scrolls so the selected file stays visible when
    /// the Changes list is taller than the panel. Previously the list never
    /// applied an offset, so the selection jumped off-screen without scrolling.
    #[test]
    fn status_navigation_follows_selection_offscreen() {
        use crate::features::status::view::selected_display_row;

        let tmp = tempdir_with_git_init();
        // Many untracked files → a Changes list far taller than the viewport.
        for i in 0..25 {
            std::fs::write(tmp.join(format!("f{i:02}.txt")), "x\n").unwrap();
        }

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;
        app.ui.list_viewport.set(10); // simulate a 10-row-tall Changes panel

        for _ in 0..20 {
            let _ = crate::update::update(&mut app, crate::action::Action::Down);
        }

        let row = selected_display_row(&app);
        let off = app.ui.list_offset;
        assert!(
            off <= row && row < off + 10,
            "selected display row {row} must stay within the viewport [{off}, {off}+10)"
        );
        assert!(
            off > 0,
            "the Changes list must have scrolled to follow the off-screen selection"
        );
    }

    /// Also verify Theme, Config, Keymap construction in isolation.
    #[test]
    fn types_construct() {
        let _ = Theme::tokyonight();
        let _ = Theme::from_name("tokyonight");
        let _ = Theme::default();
        let _ = Config::default();
        let _ = Keymap;
    }

    // ── Audit-fix regressions (mutating-op correctness) ───────────────────────

    use crate::app::Mode;
    use crate::features::status::view as status_view;
    use crate::update::update;

    /// F1: searching in Status mode must land on the correct file even when a
    /// file appears in BOTH the Staged and Unstaged groups (two logical rows).
    #[test]
    fn search_status_lands_on_correct_file_across_groups() {
        let tmp = tempdir_with_git_init();
        for f in ["alpha", "beta", "gamma"] {
            std::fs::write(tmp.join(format!("{f}.txt")), "1\n").unwrap();
        }
        git_in(&tmp, &["add", "."]);
        git_in(&tmp, &["commit", "-m", "base"]);
        // alpha: staged THEN modified again (MM); beta: staged (M.); gamma: unstaged (.M)
        std::fs::write(tmp.join("alpha.txt"), "2\n").unwrap();
        git_in(&tmp, &["add", "alpha.txt"]);
        std::fs::write(tmp.join("alpha.txt"), "3\n").unwrap();
        std::fs::write(tmp.join("beta.txt"), "2\n").unwrap();
        git_in(&tmp, &["add", "beta.txt"]);
        std::fs::write(tmp.join("gamma.txt"), "2\n").unwrap();

        let backend = crate::git::open(&tmp).expect("open");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = Mode::Status;

        update(&mut app, Action::OpenSearch);
        for c in "gamma".chars() {
            update(&mut app, Action::SearchChar(c));
        }

        let entry = status_view::resolve_entry(&app, app.ui.list_index)
            .expect("search cursor must resolve to a file");
        assert_eq!(
            entry.path, "gamma.txt",
            "search 'gamma' must land on gamma, not a dual-group file"
        );
    }

    /// F2: a bulk op that shrinks the list must keep the cursor resolvable.
    #[test]
    fn stage_all_keeps_cursor_resolvable() {
        let tmp = tempdir_with_git_init();
        for f in ["a", "b"] {
            std::fs::write(tmp.join(format!("{f}.txt")), "1\n").unwrap();
        }
        git_in(&tmp, &["add", "."]);
        git_in(&tmp, &["commit", "-m", "base"]);
        // a: MM (staged + unstaged), b: .M (unstaged) → 3 logical rows
        std::fs::write(tmp.join("a.txt"), "2\n").unwrap();
        git_in(&tmp, &["add", "a.txt"]);
        std::fs::write(tmp.join("a.txt"), "3\n").unwrap();
        std::fs::write(tmp.join("b.txt"), "2\n").unwrap();

        let backend = crate::git::open(&tmp).expect("open");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = Mode::Status;
        app.ui.list_index = status_view::status_row_count(&app).saturating_sub(1);

        update(&mut app, Action::StageAll);

        assert!(
            status_view::resolve_entry(&app, app.ui.list_index).is_some(),
            "after StageAll the highlighted row must still resolve (cursor clamped)"
        );
    }

    /// F8: reset soft also requires confirmation before moving HEAD.
    #[test]
    fn reset_soft_opens_confirm_dialog() {
        use crate::git::ResetMode;
        let tmp = tempdir_with_git_init();
        for m in ["c1", "c2", "c3"] {
            std::fs::write(tmp.join("log.txt"), format!("{m}\n")).unwrap();
            git_in(&tmp, &["add", "."]);
            git_in(&tmp, &["commit", "-m", m]);
        }
        let backend = crate::git::open(&tmp).expect("open");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = Mode::Graph;
        app.ui.graph_index = app.repo.commits.len().saturating_sub(1);

        let target = app.repo.commits.last().unwrap().id.clone();
        app.dialog = crate::app::Dialog::ResetMenu { target };
        update(&mut app, Action::ResetTo(ResetMode::Soft));

        match app.dialog {
            crate::app::Dialog::Confirm { ref pending, .. } => {
                assert!(matches!(
                    pending,
                    crate::app::ConfirmOp::Reset {
                        mode: ResetMode::Soft,
                        ..
                    }
                ));
            }
            _ => panic!("reset soft must open a confirmation dialog"),
        }
    }

    /// F15/16/18: a conflicting stash pop must surface the conflict and jump to
    /// Status mode (not report a bare 'failed' with a stale view).
    #[test]
    fn stash_pop_conflict_switches_to_status() {
        let tmp = tempdir_with_git_init();
        std::fs::write(tmp.join("c.txt"), "base\n").unwrap();
        git_in(&tmp, &["add", "."]);
        git_in(&tmp, &["commit", "-m", "base"]);
        std::fs::write(tmp.join("c.txt"), "stash-version\n").unwrap();
        git_in(&tmp, &["stash", "push", "-u", "-m", "wip"]);
        std::fs::write(tmp.join("c.txt"), "main-version\n").unwrap();
        git_in(&tmp, &["add", "."]);
        git_in(&tmp, &["commit", "-m", "main"]);

        let backend = crate::git::open(&tmp).expect("open");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = Mode::Stashes;
        app.ui.stash_index = 0;

        update(&mut app, Action::StashPop);

        assert_eq!(
            app.mode,
            Mode::Status,
            "a stash conflict should switch to Status mode"
        );
        let msg = app
            .status_message
            .clone()
            .unwrap_or_default()
            .to_lowercase();
        assert!(
            msg.contains("conflict"),
            "status should mention conflict: {msg}"
        );
        assert!(
            app.repo.status.entries.iter().any(|e| e.is_conflicted()),
            "working tree must show a conflicted entry after the conflicting pop"
        );
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Run a git command in `dir` with a deterministic identity; panic on failure.
    /// Toggling the graph scope (`a`) must switch the loaded commit set between
    /// all branches (`git log --all`, the default) and the current branch only.
    #[test]
    fn graph_scope_toggle_switches_between_all_and_current() {
        let tmp = tempdir_with_git_init();
        // Add an unmerged feature commit, then return to the base branch and
        // advance it — so each branch has a commit the other lacks.
        git_in(&tmp, &["checkout", "-b", "feature"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "feat-only"]);
        git_in(&tmp, &["checkout", "-"]); // back to the base branch
        git_in(&tmp, &["commit", "--allow-empty", "-m", "main-extra"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;

        let has = |app: &App, s: &str| app.repo.commits.iter().any(|c| c.summary == s);

        // Default scope is ALL branches → the unmerged feature commit IS shown
        // (so a dev/feature branch you're not on is still visible).
        assert!(app.ui.graph_all_branches);
        assert!(
            has(&app, "feat-only"),
            "all-branches default shows other branches"
        );
        assert!(has(&app, "main-extra"), "HEAD's own commit is shown too");

        // Toggle → current branch only → the other branch's commit disappears.
        let _ = crate::update::update(&mut app, crate::action::Action::ToggleGraphScope);
        assert!(!app.ui.graph_all_branches);
        assert!(
            !has(&app, "feat-only"),
            "current-only scope hides other branches"
        );
        assert!(has(&app, "main-extra"), "HEAD's own commit remains");

        // Toggle back → all branches again.
        let _ = crate::update::update(&mut app, crate::action::Action::ToggleGraphScope);
        assert!(app.ui.graph_all_branches);
        assert!(has(&app, "feat-only"));
    }

    /// Folding merges (`m`, first-parent) must drop the side-branch commits a
    /// merge brought in, leaving the straight trunk; unfolding restores them.
    #[test]
    fn graph_fold_merges_toggles_first_parent() {
        let tmp = tempdir_with_git_init();
        // A feature branch with two commits, merged into the base with --no-ff.
        git_in(&tmp, &["checkout", "-b", "feature"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "feat work 1"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "feat work 2"]);
        git_in(&tmp, &["checkout", "-"]);
        git_in(
            &tmp,
            &["merge", "--no-ff", "feature", "-m", "Merge feature"],
        );

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        // Folding is a current-branch-scope behavior; narrow from the all-branches
        // default so the still-existing `feature` ref doesn't keep its commits
        // loaded independently of the merge.
        app.ui.graph_all_branches = false;
        let _ = app.refresh();

        let has = |app: &App, s: &str| app.repo.commits.iter().any(|c| c.summary == s);

        // Expanded (default): the feature's internal commits are visible.
        assert!(!app.ui.graph_first_parent);
        assert!(has(&app, "feat work 1") && has(&app, "feat work 2"));
        assert!(has(&app, "Merge feature"));

        // Fold → first-parent only → the side-branch commits collapse away, but
        // the merge commit (the trunk) stays.
        let _ = crate::update::update(&mut app, crate::action::Action::ToggleGraphFirstParent);
        assert!(app.ui.graph_first_parent);
        assert!(
            has(&app, "Merge feature"),
            "merge commit stays on the trunk"
        );
        assert!(
            !has(&app, "feat work 1"),
            "folded: side-branch commits hidden"
        );
        assert!(
            !has(&app, "feat work 2"),
            "folded: side-branch commits hidden"
        );

        // Unfold → restored.
        let _ = crate::update::update(&mut app, crate::action::Action::ToggleGraphFirstParent);
        assert!(!app.ui.graph_first_parent);
        assert!(has(&app, "feat work 1") && has(&app, "feat work 2"));
    }

    /// The Branch lens (`l`) filters the graph to the selected commit's branch
    /// PLUS main (union), excluding unrelated branches; toggling off restores the
    /// current-branch scope.
    #[test]
    fn graph_branch_lens_unions_branch_and_main() {
        let tmp = tempdir_with_git_init();
        let base_branch = git_out(&tmp, &["branch", "--show-current"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "base"]);
        git_in(&tmp, &["checkout", "-b", "feature"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "feat-1"]);
        git_in(&tmp, &["checkout", &base_branch]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "main-1"]);
        git_in(&tmp, &["checkout", "-b", "other"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "other-1"]);
        git_in(&tmp, &["checkout", "feature"]); // HEAD = feature

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        // This test contrasts the lens against the *current-branch* scope, so
        // narrow from the all-branches default first. Use the real action
        // (not a manual field flip) so `graph_index` is reset to 0 — a stale
        // index from the all-branches view would anchor the lens on the wrong
        // commit.
        let _ = crate::update::update(&mut app, crate::action::Action::ToggleGraphScope);
        assert!(!app.ui.graph_all_branches);

        let has = |app: &App, s: &str| app.repo.commits.iter().any(|c| c.summary == s);

        // Current-branch scope (HEAD = feature): feat-1 yes; main's divergent
        // commit and the unrelated branch are not loaded.
        assert!(has(&app, "feat-1"));
        assert!(!has(&app, "main-1"));
        assert!(!has(&app, "other-1"));

        // Turn on the Branch lens (anchored on the selected feature tip).
        let _ = crate::update::update(&mut app, crate::action::Action::ToggleGraphBranchFocus);
        assert!(app.ui.graph_focus.is_some());
        assert!(
            has(&app, "feat-1") && has(&app, "main-1"),
            "lens unions feature + main"
        );
        assert!(!has(&app, "other-1"), "lens excludes unrelated branches");

        // Turn it off → back to current-branch scope.
        let _ = crate::update::update(&mut app, crate::action::Action::ToggleGraphBranchFocus);
        assert!(app.ui.graph_focus.is_none());
        assert!(!has(&app, "main-1"));
    }

    /// Wide-terminal three-pane dashboard: at >= 150 cols, Status mode renders
    /// the Graph, Changes, and Diff panes side by side. The render must not
    /// panic and the buffer must contain both the Graph and Changes panel
    /// titles — proving all three panes were laid out in one frame.
    #[test]
    fn wide_status_renders_three_pane_dashboard() {
        let tmp = tempdir_with_git_init();
        // A second commit so the graph has content, plus a modified file so the
        // change list has content.
        git_in(&tmp, &["commit", "--allow-empty", "-m", "second"]);
        std::fs::write(tmp.join("file.txt"), "hello\n").expect("write file");

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;
        let _ = app.refresh();
        let _ = crate::update::update(&mut app, crate::action::Action::Select);

        // 200 cols triggers the wide three-pane dashboard for Status mode.
        let backend_tb = TestBackend::new(200, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw frame");

        // The layout decision must be ThreePane for Status at 200 cols.
        assert_eq!(
            app.ui.pane_layout(app.mode),
            crate::ui::layout::PaneLayout::ThreePane
        );

        let buf = terminal.backend().buffer().clone();
        let rendered: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(
            rendered.contains("Graph"),
            "three-pane dashboard must render the Graph pane title"
        );
        assert!(
            rendered.contains("Changes"),
            "three-pane dashboard must render the Changes pane title"
        );
        assert!(
            rendered.contains("Diff"),
            "three-pane dashboard must render the Diff pane title"
        );
    }

    /// Narrow terminal: Status mode keeps the historical two-pane layout (no
    /// Graph pane), proving the responsive switch is width-gated.
    #[test]
    fn narrow_status_keeps_two_pane() {
        let tmp = tempdir_with_git_init();
        let backend = crate::git::open(&tmp).expect("open temp repo");
        let app = App::new(Box::new(backend), Config::default()).expect("App::new");

        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw frame");

        assert_eq!(
            app.ui.pane_layout(app.mode),
            crate::ui::layout::PaneLayout::TwoPane
        );
    }

    /// Focus cycling in the three-pane dashboard: Tab cycles Left → Middle →
    /// Right → Left. Three-pane layout is Left=Changes, Middle=Graph, Right=Diff.
    /// Status primary = Left (Changes), so initial focus after SwitchMode is Left.
    #[test]
    fn focus_next_cycles_three_pane_in_dashboard() {
        use crate::app::Panel;
        let tmp = tempdir_with_git_init();
        git_in(&tmp, &["commit", "--allow-empty", "-m", "second"]);
        std::fs::write(tmp.join("file.txt"), "hello\n").expect("write file");

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;
        let _ = app.refresh();
        // Simulate a wide terminal so the model uses the three-pane layout.
        app.ui.width.set(200);
        // Initial focus after SwitchMode is the Status primary = Left (Changes).
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::SwitchMode(crate::app::Mode::Status),
        );
        assert_eq!(*app.ui.panel(), Panel::Left);

        let _ = crate::update::update(&mut app, crate::action::Action::FocusNext);
        assert_eq!(*app.ui.panel(), Panel::Middle, "Left -> Middle");
        let _ = crate::update::update(&mut app, crate::action::Action::FocusNext);
        assert_eq!(*app.ui.panel(), Panel::Right, "Middle -> Right");
        let _ = crate::update::update(&mut app, crate::action::Action::FocusNext);
        assert_eq!(*app.ui.panel(), Panel::Left, "Right -> Left");
    }

    /// Mouse click on a list row jumps the cursor to that row and focuses the
    /// panel. We simulate a click on the Changes list (Status mode, two-pane)
    /// by first rendering a frame (to record the panel rects), then dispatching
    /// a ClickPanel action and verifying the cursor moved.
    #[test]
    fn click_panel_jumps_cursor_and_focuses() {
        let tmp = tempdir_with_git_init();
        // Create 3 modified files so the change list has multiple rows.
        for i in 0..3 {
            std::fs::write(tmp.join(format!("file{i}.txt")), format!("content {i}\n"))
                .expect("write file");
        }

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;
        let _ = app.refresh();

        // Render a frame so panel rects are recorded.
        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw frame");

        // Display layout (0 staged, 3 unstaged):
        //   row 0: "Staged (0)" header
        //   row 1: "(nothing staged)" placeholder
        //   row 2: "Unstaged (3)" header
        //   row 3: file0 (logical 0)
        //   row 4: file1 (logical 1)
        //   row 5: file2 (logical 2)
        // Click on row 5 = file2 (logical index 2).
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::ClickPanel {
                panel: Panel::Left,
                row: 5,
            },
        );

        // The cursor should have moved to logical index 2 and focus should be Left.
        assert_eq!(
            app.ui.list_index, 2,
            "click should jump cursor to file2 (logical 2)"
        );
        assert_eq!(*app.ui.panel(), Panel::Left, "click should focus the panel");
    }

    /// Click on the Diff panel focuses it without moving any cursor.
    #[test]
    fn click_diff_panel_focuses_without_cursor_move() {
        let tmp = tempdir_with_git_init();
        std::fs::write(tmp.join("file.txt"), "hello\n").expect("write file");

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;
        let _ = app.refresh();

        let _ = crate::update::update(
            &mut app,
            crate::action::Action::ClickPanel {
                panel: Panel::Main,
                row: 5,
            },
        );

        // Focus should be on Main (the diff panel in two-pane mode).
        assert_eq!(
            *app.ui.panel(),
            Panel::Main,
            "click on diff should focus Main"
        );
        // list_index should still be 0 (unchanged).
        assert_eq!(app.ui.list_index, 0, "diff click must not move list cursor");
    }

    /// Click row is clamped to the list length (clicking past the end selects
    /// the last item, not an out-of-bounds index).
    #[test]
    fn click_panel_clamps_row_to_list_length() {
        let tmp = tempdir_with_git_init();
        std::fs::write(tmp.join("file.txt"), "hello\n").expect("write file");

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;
        let _ = app.refresh();

        // The status list has 1 file → max index = 0. Click row 100.
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::ClickPanel {
                panel: Panel::Left,
                row: 100,
            },
        );

        assert_eq!(app.ui.list_index, 0, "row should be clamped to max index 0");
    }

    /// `display_row_to_logical` is the inverse of `selected_display_row`:
    /// it maps a rendered row (including group headers + placeholders) back
    /// to the logical file index. This is what the mouse click handler uses
    /// so that clicking a row selects the file actually at that row, not one
    /// offset by the header rows above it.
    #[test]
    fn display_row_to_logical_maps_correctly() {
        let tmp = tempdir_with_git_init();
        // Create 2 staged + 3 unstaged files.
        for i in 0..5 {
            let name = format!("file{i}.txt");
            std::fs::write(tmp.join(&name), format!("content {i}\n")).expect("write");
        }
        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;
        let _ = app.refresh();
        // Stage the first 2 files.
        git_in(&tmp, &["add", "file0.txt", "file1.txt"]);
        let _ = app.refresh();

        use crate::features::status::view::{display_row_to_logical, selected_display_row};

        // Display layout:
        //   row 0: "Staged (2)" header
        //   row 1: file0.txt  (logical 0)
        //   row 2: file1.txt  (logical 1)
        //   row 3: "Unstaged (3)" header
        //   row 4: file2.txt  (logical 2)
        //   row 5: file3.txt  (logical 3)
        //   row 6: file4.txt  (logical 4)

        // Round-trip: for every logical index, display_row → logical should
        // give back the same index.
        for logical in 0..5 {
            // Temporarily set list_index to check each row.
            app.ui.list_index = logical;
            let disp = selected_display_row(&app);
            let back = display_row_to_logical(&app, disp);
            assert_eq!(
                back, logical,
                "round-trip failed for logical {logical}: display {disp} → logical {back}"
            );
        }

        // Direct mapping checks.
        assert_eq!(display_row_to_logical(&app, 0), 0, "header → first file");
        assert_eq!(
            display_row_to_logical(&app, 1),
            0,
            "row 1 = file0 (logical 0)"
        );
        assert_eq!(
            display_row_to_logical(&app, 2),
            1,
            "row 2 = file1 (logical 1)"
        );
        assert_eq!(
            display_row_to_logical(&app, 3),
            2,
            "unstaged header → first unstaged (logical 2)"
        );
        assert_eq!(
            display_row_to_logical(&app, 4),
            2,
            "row 4 = file2 (logical 2)"
        );
        assert_eq!(
            display_row_to_logical(&app, 5),
            3,
            "row 5 = file3 (logical 3)"
        );
        assert_eq!(
            display_row_to_logical(&app, 6),
            4,
            "row 6 = file4 (logical 4)"
        );
    }

    /// Clicking a Changes row should select the file at that display row,
    /// accounting for group headers. Without the fix, clicking row 4 (file2)
    /// would set list_index=4 which is file3 — one row too far down.
    #[test]
    fn click_changes_row_selects_correct_file() {
        let tmp = tempdir_with_git_init();
        for i in 0..5 {
            let name = format!("file{i}.txt");
            std::fs::write(tmp.join(&name), format!("content {i}\n")).expect("write");
        }
        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;
        let _ = app.refresh();
        git_in(&tmp, &["add", "file0.txt", "file1.txt"]);
        let _ = app.refresh();

        // Click display row 4 = file2.txt (logical index 2).
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::ClickPanel {
                panel: Panel::Left,
                row: 4,
            },
        );
        assert_eq!(
            app.ui.list_index, 2,
            "clicking display row 4 (file2) should set list_index to 2, not 4"
        );

        // Click display row 6 = file4.txt (logical index 4).
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::ClickPanel {
                panel: Panel::Left,
                row: 6,
            },
        );
        assert_eq!(
            app.ui.list_index, 4,
            "clicking display row 6 (file4) should set list_index to 4"
        );
    }

    /// Branch compare: setting `compare` and refreshing filters the commit list
    /// to only `base..target` commits.
    #[test]
    fn compare_mode_filters_commits_to_base_dot_dot_target() {
        let tmp = tempdir_with_git_init();
        // tempdir_with_git_init creates an "initial" commit on main already.
        git_in(&tmp, &["commit", "--allow-empty", "-m", "base commit"]);
        git_in(&tmp, &["checkout", "-b", "feature"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "feat 1"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "feat 2"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        let _ = app.refresh();

        // Without compare: 4 commits (initial + base + 2 feature, all branches).
        assert_eq!(app.repo.commits.len(), 4, "should have 4 commits total");

        // Enter compare mode: main..feature.
        app.compare = Some(("main".into(), "feature".into()));
        let _ = app.refresh();

        // Only the 2 feature commits (base..target excludes base).
        assert_eq!(
            app.repo.commits.len(),
            2,
            "compare should filter to base..target commits only"
        );
    }

    /// Compare mode: `Esc` (FocusLeft) exits compare mode and restores the
    /// full commit list.
    #[test]
    fn compare_mode_esc_exits_and_restores_commits() {
        let tmp = tempdir_with_git_init();
        // tempdir_with_git_init creates an "initial" commit on main already.
        git_in(&tmp, &["commit", "--allow-empty", "-m", "base commit"]);
        git_in(&tmp, &["checkout", "-b", "feature"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "feat 1"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        let _ = app.refresh();

        // Enter compare mode.
        app.compare = Some(("main".into(), "feature".into()));
        let _ = app.refresh();
        assert_eq!(app.repo.commits.len(), 1, "compare shows 1 commit");

        // Esc exits compare mode.
        let _ = crate::update::update(&mut app, crate::action::Action::FocusLeft);
        assert!(app.compare.is_none(), "Esc should exit compare mode");
        assert_eq!(app.repo.commits.len(), 3, "full commit list restored");
    }

    /// Compare mode: `=` key opens the CompareBranches dialog with both fields
    /// empty and base field focused.
    #[test]
    fn compare_key_opens_dialog_with_both_fields() {
        let tmp = tempdir_with_git_init();
        git_in(&tmp, &["commit", "--allow-empty", "-m", "init"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        let _ = app.refresh();

        let _ = crate::update::update(&mut app, crate::action::Action::CompareBranches);
        match &app.dialog {
            crate::app::Dialog::CompareBranches {
                base,
                target,
                focus_target,
            } => {
                assert!(base.is_empty(), "base field should start empty");
                assert!(target.is_empty(), "target field should start empty");
                assert!(!*focus_target, "base field should be focused first");
            }
            _ => panic!("expected CompareBranches dialog"),
        }
    }

    /// Compare dialog: Tab toggles between base and target fields.
    #[test]
    fn compare_tab_toggles_between_fields() {
        let tmp = tempdir_with_git_init();
        git_in(&tmp, &["commit", "--allow-empty", "-m", "init"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        let _ = app.refresh();

        // Open dialog.
        let _ = crate::update::update(&mut app, crate::action::Action::CompareBranches);
        // Tab to target.
        let _ = crate::update::update(&mut app, crate::action::Action::CompareToggleField);
        match &app.dialog {
            crate::app::Dialog::CompareBranches { focus_target, .. } => {
                assert!(*focus_target, "Tab should focus target field");
            }
            _ => panic!(),
        }
        // Tab back to base.
        let _ = crate::update::update(&mut app, crate::action::Action::CompareToggleField);
        match &app.dialog {
            crate::app::Dialog::CompareBranches { focus_target, .. } => {
                assert!(!*focus_target, "Tab again should focus base field");
            }
            _ => panic!(),
        }
    }

    /// Compare dialog: typing fills the focused field, Enter resolves both
    /// queries and enters compare mode.
    #[test]
    fn compare_submit_enters_compare_mode() {
        let tmp = tempdir_with_git_init();
        git_in(&tmp, &["commit", "--allow-empty", "-m", "init"]);
        git_in(&tmp, &["checkout", "-b", "feature"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "feat 1"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        let _ = app.refresh();

        // Open dialog.
        let _ = crate::update::update(&mut app, crate::action::Action::CompareBranches);
        // Type "main" in base field.
        for c in "main".chars() {
            let _ = crate::update::update(&mut app, crate::action::Action::DialogChar(c));
        }
        // Tab to target.
        let _ = crate::update::update(&mut app, crate::action::Action::CompareToggleField);
        // Type "feature" in target field.
        for c in "feature".chars() {
            let _ = crate::update::update(&mut app, crate::action::Action::DialogChar(c));
        }
        // Enter to submit.
        let _ = crate::update::update(&mut app, crate::action::Action::CompareSubmit);

        assert_eq!(
            app.compare,
            Some(("main".into(), "feature".into())),
            "compare mode should be active with main..feature"
        );
        assert!(matches!(app.dialog, crate::app::Dialog::None));
    }

    /// Compare mode: pressing `C` again exits compare mode.
    #[test]
    fn compare_key_toggles_off_when_already_in_compare_mode() {
        let tmp = tempdir_with_git_init();
        git_in(&tmp, &["commit", "--allow-empty", "-m", "init"]);
        git_in(&tmp, &["checkout", "-b", "feature"]);
        git_in(&tmp, &["commit", "--allow-empty", "-m", "feat 1"]);

        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Graph;
        let _ = app.refresh();

        // Enter compare mode directly.
        app.compare = Some(("main".into(), "feature".into()));
        let _ = app.refresh();
        assert!(app.compare.is_some());

        // Press C again to exit.
        let _ = crate::update::update(&mut app, crate::action::Action::CompareBranches);
        assert!(app.compare.is_none(), "C should toggle compare off");
    }

    /// Clicking a mode tab switches to that mode.
    #[test]
    fn click_tab_switches_mode() {
        let tmp = tempdir_with_git_init();
        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;

        // Click tab index 1 = Graph.
        let _ = crate::update::update(&mut app, crate::action::Action::ClickTab { index: 1 });
        assert_eq!(
            app.mode,
            crate::app::Mode::Graph,
            "clicking tab 1 should switch to Graph"
        );

        // Click tab index 3 = Worktrees.
        let _ = crate::update::update(&mut app, crate::action::Action::ClickTab { index: 3 });
        assert_eq!(
            app.mode,
            crate::app::Mode::Worktrees,
            "clicking tab 3 should switch to Worktrees"
        );

        // Click tab index 5 = Inspect.
        let _ = crate::update::update(&mut app, crate::action::Action::ClickTab { index: 5 });
        assert_eq!(
            app.mode,
            crate::app::Mode::Inspect,
            "clicking tab 5 should switch to Inspect"
        );
    }

    /// The tab strip records each tab's actual x range (ratatui renders tabs
    /// with variable widths: title + padding + divider, NOT equal division).
    /// This test renders a frame, reads the recorded ranges, and verifies
    /// that clicking the Worktrees tab (index 3) at any column within its
    /// range switches to Worktrees mode — not Stashes.
    #[test]
    fn tab_click_hits_correct_tab_after_render() {
        let tmp = tempdir_with_git_init();
        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;
        let _ = app.refresh();

        // Render a frame so tab ranges are recorded.
        let backend_tb = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw frame");

        let tab_strip = app.ui.tab_strip.get();
        // Tab 3 = Worktrees. Get its range and click at its midpoint.
        let (start, end) = tab_strip.ranges[3].expect("tab 3 range should be recorded");
        let mid = (start + end) / 2;
        assert_eq!(
            tab_strip.y, 2,
            "tab strip should be at row 2 (inside top/bottom border)"
        );

        // Simulate a mouse click at (mid, tab_strip.y).
        let _ = crate::update::update(
            &mut app,
            crate::action::Action::ClickTab {
                index: crate::core::event::tab_index_from_strip(tab_strip, mid, tab_strip.y)
                    .expect("should find a tab"),
            },
        );
        assert_eq!(
            app.mode,
            crate::app::Mode::Worktrees,
            "clicking mid of tab 3 should switch to Worktrees, not Stashes"
        );

        // Also verify the right edge of tab 3 is NOT tab 4.
        let right_edge = end - 1;
        let idx = crate::core::event::tab_index_from_strip(tab_strip, right_edge, tab_strip.y)
            .expect("should find a tab");
        assert_eq!(idx, 3, "right edge of tab 3 should be index 3, not 4");
    }

    /// On a narrow terminal (e.g. 80 columns), the tab strip still records
    /// correct ranges and clicks land on the right tab.
    #[test]
    fn tab_click_narrow_terminal() {
        let tmp = tempdir_with_git_init();
        let backend = crate::git::open(&tmp).expect("open temp repo");
        let mut app = App::new(Box::new(backend), Config::default()).expect("App::new");
        app.mode = crate::app::Mode::Status;
        let _ = app.refresh();

        let backend_tb = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend_tb).expect("TestBackend terminal");
        terminal
            .draw(|frame| view(frame, &app))
            .expect("draw frame");

        let tab_strip = app.ui.tab_strip.get();
        // Verify all 6 tabs have ranges.
        for i in 0..6 {
            assert!(
                tab_strip.ranges[i].is_some(),
                "tab {i} should have a range on narrow terminal"
            );
        }

        // Click tab 4 (Stashes) at its midpoint.
        let (start, end) = tab_strip.ranges[4].expect("tab 4 range");
        let mid = (start + end) / 2;
        let idx = crate::core::event::tab_index_from_strip(tab_strip, mid, tab_strip.y)
            .expect("should find a tab");
        assert_eq!(idx, 4, "mid of tab 4 should be index 4");
    }

    fn git_in(dir: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@t.com")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@t.com")
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    fn git_out(dir: &std::path::Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn tempdir_with_git_init() -> PathBuf {
        // Unique per call AND per process so parallel tests never collide on the
        // same path (a sub-second-nanos-only name used to clash, making `git
        // init` flakily fail).
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("giv_test_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let output = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .output()
            .expect("git init");
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );

        // Create an initial commit so HEAD exists.
        git_in(&dir, &["commit", "--allow-empty", "-m", "initial"]);

        dir
    }
}
