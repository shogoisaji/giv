//! Terminal runtime: setup/teardown, the main event loop, effect handling and
//! logging. Extracted from `main.rs` so the binary entry point stays a thin CLI
//! shell and all TUI-driving logic lives in the library core.

use std::time::{Duration, Instant};

use anyhow::Context;
use std::path::PathBuf;

use crate::action::Action;
use crate::app::App;
use crate::config::{load_config, Config};
use crate::effect::Effect;
use crate::event::next_action;
use crate::git::{self, GitBackend};
use crate::update::update;

// ─── TUI launch ──────────────────────────────────────────────────────────────

/// Open the repository at `path`, run the TUI, and restore the terminal on exit.
pub fn run_tui(path: PathBuf) -> anyhow::Result<()> {
    // Install a panic hook that restores the terminal before printing the panic.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort terminal restore; ignore errors inside a panic handler.
        let _ = crossterm::terminal::disable_raw_mode();
        set_alternate_scroll(&mut std::io::stderr(), true);
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::event::DisableMouseCapture,
            crossterm::terminal::LeaveAlternateScreen,
        );
        original_hook(info);
    }));

    let config = load_config().unwrap_or_default();

    // Set up logging to a file (never to stdout while TUI is active).
    setup_logging()?;

    let backend = git::open(&path).context("opening repository")?;
    let backend_box: Box<dyn GitBackend> = Box::new(backend);

    // Initialise the terminal.
    //
    // Mouse capture is enabled by default so click-to-jump and wheel-scroll
    // work immediately. The `M` key toggles mouse capture off for native
    // text selection (click-drag to copy) when needed.
    crossterm::terminal::enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )
    .context("entering alternate screen")?;
    // Mouse is on → alternate-scroll mode is moot while it is enabled.

    let (result, print_cwd) = run_event_loop(backend_box, config);

    // Always restore terminal before producing any stdout output.
    let _ = crossterm::terminal::disable_raw_mode();
    set_alternate_scroll(&mut std::io::stdout(), true);
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
    );

    // If SwitchWorktree was used, print the path so a shell wrapper can cd.
    // e.g.: `cd "$(giv)"` where the shell function captures stdout.
    if let Some(cwd) = print_cwd {
        println!("{cwd}");
    }

    result
}

/// Run the event loop. Returns `(result, optional_cwd_to_print)`.
///
/// Separating the cwd-print from the event loop lets us restore the terminal
/// **before** writing to stdout (which would otherwise appear in the alt screen).
fn run_event_loop(
    backend: Box<dyn GitBackend>,
    config: Config,
) -> (anyhow::Result<()>, Option<String>) {
    let mut app = match App::new(backend, config).context("initialising app") {
        Ok(a) => a,
        Err(e) => return (Err(e), None),
    };

    // Load the diff for the initially-selected entry so the diff panel shows
    // content immediately on launch instead of being blank until the user moves.
    let effect = update(&mut app, Action::Select);
    handle_effect(&mut app, effect);

    let mut terminal =
        match ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))
            .context("creating terminal")
        {
            Ok(t) => t,
            Err(e) => return (Err(e), None),
        };

    // Short poll timeout keeps the UI responsive while also letting
    // background-task results arrive promptly.
    let poll_timeout = Duration::from_millis(100);

    // Transient status-message auto-expiry: a stale "Committed successfully"
    // lingering next to a later error erodes trust, so we clear messages a few
    // seconds after they were last set. Tracked here (not in `update`) so the
    // ~40 `status_message = Some(..)` call sites stay untouched.
    const STATUS_TTL: Duration = Duration::from_secs(5);
    let mut last_status = app.status_message.clone();
    let mut status_shown_at = Instant::now();

    loop {
        if let Err(e) = terminal
            .draw(|frame| crate::ui::view(frame, &app))
            .context("drawing frame")
        {
            return (Err(e), None);
        }

        // ── Input events ─────────────────────────────────────────────────────
        // Read one event (blocking up to `poll_timeout`).  Then drain any
        // additional events that are already queued (poll(0)) BEFORE the next
        // render.  This prevents a burst of mouse-wheel / key events from
        // causing one full `terminal.draw()` per event — which on a large
        // diff or a fast trackpad momentum-scroll can freeze the UI so hard
        // that even Ctrl-C can't get processed.
        match next_action(&app.keymap, &app, poll_timeout).context("reading input event") {
            Err(e) => return (Err(e), None),
            Ok(Some(action)) => {
                let effect = update(&mut app, action);
                handle_effect(&mut app, effect);

                // Drain the rest of the queued events without rendering.
                // Quit events break immediately so the user gets instant
                // feedback.
                while !app.should_quit
                    && crossterm::event::poll(Duration::ZERO).unwrap_or(false)
                {
                    if let Ok(Some(action)) = next_action(&app.keymap, &app, Duration::ZERO) {
                        let effect = update(&mut app, action);
                        handle_effect(&mut app, effect);
                    } else {
                        break;
                    }
                }
            }
            Ok(None) => {
                let effect = update(&mut app, Action::LoadPendingGraphDiff);
                handle_effect(&mut app, effect);
            }
        }

        // ── Background task results ──────────────────────────────────────────
        // Drain all pending task completions without blocking.
        loop {
            match app.task_rx.try_recv() {
                Ok(task_action) => {
                    let effect = update(&mut app, task_action);
                    handle_effect(&mut app, effect);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        // ── Auto-expire transient status messages ────────────────────────────
        // Reset the timer whenever the message changes; clear it once it has
        // been shown for STATUS_TTL. A running background task owns the status
        // bar (spinner), so don't expire while one is active.
        if app.status_message != last_status {
            last_status = app.status_message.clone();
            status_shown_at = Instant::now();
        } else if app.status_message.is_some()
            && app.running_task.is_none()
            && status_shown_at.elapsed() >= STATUS_TTL
        {
            app.status_message = None;
            last_status = None;
        }

        if app.should_quit {
            break;
        }
    }

    let cwd = app.print_cwd_on_exit.clone();
    (Ok(()), cwd)
}

/// Enable (`true`) or disable (`false`) the terminal's "alternate scroll" mode
/// (DEC private mode 1007).
///
/// When mouse capture is OFF, many terminals translate wheel scroll inside the
/// alternate screen into ↑/↓ arrow keys. Those arrive as ordinary key events and
/// would move the *selection* instead of doing nothing — disruptive while the
/// user is selecting text with the mouse. Turning 1007 off keeps the wheel inert
/// so a stray scroll never disturbs a selection. We restore it on exit.
fn set_alternate_scroll(w: &mut impl std::io::Write, on: bool) {
    let seq: &[u8] = if on { b"\x1b[?1007h" } else { b"\x1b[?1007l" };
    // Best-effort: if the terminal doesn't support this sequence, there's
    // nothing useful to do with the error.
    let _ = w.write_all(seq);
    let _ = w.flush();
}

/// Inline effect handler used inside the event loop.
fn handle_effect(app: &mut App, effect: Effect) {
    match effect {
        Effect::Quit => {
            app.should_quit = true;
        }
        Effect::Refresh => {
            // Refresh already happened inside update() for sync effects.
        }
        Effect::SetMouseCapture(on) => {
            // Issue the crossterm command on the live terminal. Best-effort:
            // a failure just leaves mouse capture in its previous state.
            let mut stdout = std::io::stdout();
            if on {
                // Mouse reporting takes over the wheel (delivered as real scroll
                // events), so alternate-scroll mode is moot while it is enabled.
                let _ = crossterm::execute!(stdout, crossterm::event::EnableMouseCapture);
            } else {
                let _ = crossterm::execute!(stdout, crossterm::event::DisableMouseCapture);
                // Back to selection mode → keep the wheel inert again.
                set_alternate_scroll(&mut stdout, false);
            }
        }
        Effect::Batch(effects) => {
            for e in effects {
                handle_effect(app, e);
            }
        }
        Effect::None => {}
    }
}

// ─── Logging setup ───────────────────────────────────────────────────────────

fn setup_logging() -> anyhow::Result<()> {
    use directories::ProjectDirs;
    use tracing_subscriber::EnvFilter;

    let dirs = match ProjectDirs::from("", "", "giv") {
        Some(d) => d,
        None => {
            // No home directory — skip file logging silently.
            return Ok(());
        }
    };

    let log_dir = dirs.state_dir().unwrap_or_else(|| dirs.data_local_dir());
    std::fs::create_dir_all(log_dir).context("creating log directory")?;

    let log_path = log_dir.join("giv.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening log file {}", log_path.display()))?;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    Ok(())
}
