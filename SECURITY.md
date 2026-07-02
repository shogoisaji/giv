# Security Policy

giv shells out to the local `git` binary and can run mutating git operations
such as stage, commit, stash, merge, rebase, reset, and worktree changes.

## Supported versions

Security fixes are applied to the latest released version.

## Reporting a vulnerability

Until a public security advisory channel is configured, report suspected
vulnerabilities privately to the repository owner.

Please include:

- A concise description of the issue.
- Exact reproduction steps.
- The affected platform and terminal.
- Whether the issue can mutate repository data, leak data, or execute commands.

Do not open a public issue for a vulnerability before the maintainer has had a
chance to investigate.

## Security expectations

- Git command arguments must be passed as argv entries, not interpolated into a shell command.
- Operations that can discard or rewrite user work need an explicit confirmation flow.
- Subprocesses must not block on editors, pagers, or credential prompts while the TUI is active.
- Tests should not touch real user repositories or network remotes.
