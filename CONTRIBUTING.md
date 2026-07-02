# Contributing

Thanks for helping improve giv.

## Development setup

Requirements:

- Git on `PATH`
- Rust 1.85 or newer

Run the full local gate before opening a pull request:

```sh
./scripts/release-check.sh
```

## Pull request expectations

- Keep changes scoped to one behavior or bug.
- Add or update tests for user-visible behavior, git operation semantics, parser changes, and layout/navigation regressions.
- Do not introduce terminal side effects in tests. Capture subprocess output and avoid writing escape sequences to stdout/stderr.
- Avoid destructive git behavior without an explicit confirmation flow.
- Update README or release docs when user-facing commands, keybindings, or distribution steps change.

## Architecture

giv uses an Elm-style model:

- `core::event` converts terminal input to `Action`.
- `core::update` mutates `App` and returns an `Effect`.
- `features/*` own mode-specific update/view/keymap logic.
- `git::cli` shells out to `git`; pure parsers live in `git::cli::parse` and `git::diff`.
- `ui/*` renders from the model and should avoid owning business rules.

Prefer small pure helpers with tests for layout, parsing, and navigation rules.
