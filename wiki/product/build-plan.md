# Build Plan: From Wiki to Working Binary

Naming is [shortlisted, not blocking](../naming/decision.md) — the repo
runs under the working name `goulash`; a rename before 1.0 is cheap.
**The priority now is building it.**

## Milestone 0 — transparent PTY wrapper 🚧 in progress
**In Rust** ([implementation](../architecture/implementation.md)), with
`~/.goulash/config.toml` loading from day one. The null overlay:
`goulash "$SHELL"` allocates a PTY, spawns the shell,
forwards all bytes both ways, handles window resize (SIGWINCH) and exit
codes. Include the **shrunken-winsize reserved rows** from day one (even
if they just show a static status line) — it's foundational geometry and
retrofitting it later touches everything. Success = you can live in it
all day (including inside and around tmux) and forget it's there.
- Spec: [pty-overlay](../architecture/pty-overlay.md),
  [status-rows](../architecture/status-rows.md)
- **Done**: PTY spawn with controlling tty, byte forwarding, shrunken
  winsize + DECSTBM scroll region, vt100 state tracking with
  cursor/SGR-restoring status redraws (debounced, plus immediate fixup
  on RIS/DECSTR/`ESC[r`/2J/3J), no-clear startup via DSR cursor query,
  SIGWINCH propagation, exit-code propagation, config loading,
  PTY-driven e2e test suite (`tests/e2e.py`).
- **Remaining**: daily-driving validation (TUIs, inner/outer tmux,
  ssh), then call it done.

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

## Milestone 3 — suggestion list + deterministic vendors (no LLM yet)
Async vending into the status rows; freeze-on-focus, ID-bound
acceptance, staleness invalidation. ZLE widget for zsh, `bind -x`
prototype for bash, bracketed-paste injection as the generic path.
First vendors are the **free ones** — rules (thefuck-style corrections),
history/fuzzy, n-gram — so this milestone needs no LLM at all and is
already the zero-setup demo (`gti status` → fix waiting in the list).
Record accept/edit/ignore.
- Spec: [suggestion-vendors](../architecture/suggestion-vendors.md),
  [suggestion-list](../interaction/suggestion-list.md),
  [down-arrow-protocol](../interaction/down-arrow-protocol.md)

## Milestone 4 — the `#` aside + `#/` commands
Intercept `#`-prefixed lines at PROMPT: `#/` opens selector widgets
(`#/status`, `#/provider`, …); plain `#` assembles context from recent
block history (flat recency window is fine here — the full
[memory hierarchy](../architecture/memory-hierarchy.md) comes later) and
answers inline. First LLM integration, provider-pluggable from the
start, probe chain live.
- Spec: [interaction model](../interaction/model.md),
  [settings-and-nav](../interaction/settings-and-nav.md),
  [llm-engine](../architecture/llm-engine.md)

## Milestone 5 — memory hierarchy + watcher tier
Rolling cleanup loop (local model if available) setting region markers;
logarithmic ramp-off retrieval; epoch-based prefix caching for API calls.
- Spec: [memory-hierarchy](../architecture/memory-hierarchy.md),
  [llm-engine](../architecture/llm-engine.md)

## Milestone 6 — `##` chat mode + delegated agents
The chat pane by **pushing the splitter** — shrink the inner PTY to the
top third (no compositing; the M0 winsize machinery, bigger number) —
with modal `##` toggle. Exploration tools over the memory tree,
`go find this shit for me` delegation into per-task PTYs with pulse
status. Suggest-only autonomy first; `accept-each` behind a flag.
- Spec: [chat-mode](../interaction/chat-mode.md),
  [delegated-agents](../interaction/delegated-agents.md)

## Later
fish adapter, remote markers over ssh
([remote-and-multiplexers](../architecture/remote-and-multiplexers.md)),
`auto-evaluated` steward scopes, bundled llama.cpp option, final name
call.

## Guiding order
Each milestone is independently usable; trust properties
([positioning](positioning.md)) are built in from milestone 1, not
retrofitted.
