# Build Plan: From Wiki to Working Binary

Naming is [shortlisted, not blocking](../naming/decision.md) — the repo
runs under the working name `goulash`; a rename before 1.0 is cheap.
**The priority now is building it.**

## Milestone 0 — transparent PTY wrapper
The null overlay: `goulash "$SHELL"` allocates a PTY, spawns the shell,
forwards all bytes both ways, handles window resize (SIGWINCH) and exit
codes. Success = you can live in it all day and forget it's there.
- Spec: [pty-overlay](../architecture/pty-overlay.md)

## Milestone 1 — session sensing
Poll/track `tcgetpgrp()` to classify PROMPT / COMMAND /
INTERACTIVE_CHILD; track ECHO state; log a raw transcript with state
annotations. No LLM yet. Success = the state log matches reality across
vim, less, fzf, ssh, sudo, pipelines, builtins.
- Spec: [input-ownership](../architecture/input-ownership.md),
  [session-state-machine](../architecture/session-state-machine.md)

## Milestone 2 — shell hooks + block history
zsh adapter first (`precmd`/`preexec`), then bash (`PROMPT_COMMAND`).
Commit command blocks and opaque blocks; enforce the echo-off privacy
invariant.
- Spec: [shell-integration](../architecture/shell-integration.md),
  [block-history](../architecture/block-history.md),
  [opaque-blocks](../architecture/opaque-blocks.md)

## Milestone 3 — the `#` aside
Intercept `#`-prefixed lines at PROMPT, assemble context from block
history, answer inline. First LLM integration; renders the aside/answer
as blocks.
- Spec: [interaction model](../interaction/model.md)

## Milestone 4 — Down-arrow suggestion
ZLE widget `goulash-down-or-suggest` for zsh; `bind -x` prototype for
bash; Alt-Down fallback elsewhere. Record accept/edit/ignore.
- Spec: [down-arrow-protocol](../interaction/down-arrow-protocol.md)

## Milestone 5 — delegated agents
`# go …` forks a background task with its own thread, pulse-block status,
and a report block on completion.
- Spec: [delegated-agents](../interaction/delegated-agents.md)

## Later
fish adapter, tmux per-pane story, remote markers over ssh
([remote-and-multiplexers](../architecture/remote-and-multiplexers.md)),
permission scopes for stewards, final name call.

## Guiding order
Each milestone is independently usable; trust properties
([positioning](positioning.md)) are built in from milestone 1, not
retrofitted.
