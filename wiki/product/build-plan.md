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

## Milestone 1 — session sensing 🚧 in progress
Poll/track `tcgetpgrp()` to classify PROMPT / COMMAND /
INTERACTIVE_CHILD; track ECHO state; log a raw transcript with state
annotations. No LLM yet. Success = the state log matches reality across
vim, less, fzf, ssh, sudo, pipelines, builtins.
- Spec: [input-ownership](../architecture/input-ownership.md),
  [session-state-machine](../architecture/session-state-machine.md)
- **Done**: gate-1 sensing (`tcgetpgrp` vs shell pgid, 250ms idle tick),
  ECHO/ICANON tracking, alt-screen detection from the vt100 tracker,
  live state label in the status row (`shell`/`run`/`tui`/`secret`),
  JSONL session transcript (`~/.goulash/history/session-*.jsonl`) with
  start/state/resize/out/end events — raw output base64-preserved,
  typed input never recorded (echo-off invariant by omission). CI on
  ubuntu + macos (fmt, clippy -D warnings, build, PTY e2e).
- **Remaining**: PROMPT vs COMMAND needs the M2 shell hooks (today both
  read as `shell`); real-world validation against vim/fzf/ssh/sudo.

## Milestone 2 — shell hooks + block history 🚧 in progress
zsh adapter first (`precmd`/`preexec`), then bash (`PROMPT_COMMAND`).
Commit command blocks and opaque blocks; enforce the echo-off privacy
invariant.
- Spec: [shell-integration](../architecture/shell-integration.md),
  [block-history](../architecture/block-history.md),
  [opaque-blocks](../architecture/opaque-blocks.md)
- **Done**: OSC 7770 wire protocol (prompt / cmd+text / exit code / cwd),
  zsh and bash adapter scripts (`shell/goulash.*`, one source line,
  inert outside goulash), streaming OSC filter with split-sequence
  handling, marks stripped before the terminal and recorded as
  block-boundary events in the transcript, hook-aware status label
  (true `prompt`/`cmd` vs sensed fallback), e2e coverage via
  `bash --rcfile`.
- **Remaining**: assemble events into explicit block structures with
  IDs (currently a flat event stream that implies blocks), opaque-block
  lifecycle summaries for TUI children, zsh adapter field-testing.

## Milestone 3 — suggestion list + deterministic vendors (no LLM yet) 🚧 in progress
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
- **Done**: rules vendor v1 (command-not-found PATH fuzzy with
  subsequence-preferring tie-break, git set-upstream lift, git similar
  command, permission-denied→sudo, cd typo) firing on failed blocks
  with output-tail context; ID'd suggestion list with insert-at-top and
  cwd-change staleness clear; top suggestion shown in the bar with the
  `↓` affordance; **plain Down in zsh** (ZLE binding: history-forward
  always wins, suggestion pull only past end of history); **`#` aside
  interception in zsh** (accept-line wrapper → OSC `Q` mark, nothing
  executes, `\#` escapes, aside recorded — engine hookup is M4);
  Alt-Down + bracketed-paste injection as the generic path (bash
  today); suggest/accept/aside events in the transcript; unit + e2e
  coverage.
- **Remaining**: history/fuzzy and n-gram vendors, scrollable list UI
  (today: top-of-list pull only), bash Readline Down-widget parity,
  aside history-recall in zsh (`print -s` in widget not sticking),
  freeze-on-focus semantics (needs the list UI).

## Milestone 4 — the `#` aside + `#/` commands 🚧 in progress
Intercept `#`-prefixed lines at PROMPT: `#/` opens selector widgets
(`#/status`, `#/provider`, …); plain `#` assembles context from recent
block history (flat recency window is fine here — the full
[memory hierarchy](../architecture/memory-hierarchy.md) comes later) and
answers inline. First LLM integration, provider-pluggable from the
start, probe chain live.
- Spec: [interaction model](../interaction/model.md),
  [settings-and-nav](../interaction/settings-and-nav.md),
  [llm-engine](../architecture/llm-engine.md)
- **Done**: engine worker thread (inference never touches the PTY loop;
  events return via mpsc + self-pipe poll wakeup), ollama provider
  (probe `/api/tags`, auto-pick or configured model, `/api/generate`
  answers), `[engine]` config (provider auto/ollama/none, host, model),
  `#` asides answered with context from the last 8 command blocks
  (cmd/exit/output-tail/cwd), one-line answers in the bar, thinking
  indicator, engine/answer transcript events, dead-worker HUP
  hardening, hermetic fake-ollama e2e. Also **down-arrow context
  shifting**: the zsh pull sends its buffer, so Down on an empty line
  pulls the top suggestion and Down on an injected suggestion cycles to
  the next (kill-line + repaste, wrapping); the user's own typed text is
  never clobbered. Successful commands clear stale suggestions.
  **`#/` commands v1**: `#/model <name>` switches the engine model
  live, `#/model` lists installed models (current starred), `#/status`,
  `#/help`; `[engine] favorites` is a preference-ordered list — the
  first installed favorite wins auto-pick (before smallest-installed;
  explicit `model` beats both; favorites match through the `:tag`).
  **Latency mechanics (knobs on by default)**: `keep_alive` holds the
  model resident; streaming partials fill the bar as tokens arrive
  (150ms throttle); context is an append-only session log with a
  byte-stable preamble — ollama's KV prefix cache re-uses everything
  but the appended tail — epoch-trimming at a block boundary when over
  `context_max_chars`; `tail_chars` bounds per-block output. Staleness
  fix: block headers carry run-time timestamps and the volatile
  current-time line sits after the stable prefix, so "what day is it"
  stops answering from four-minute-old `date` output.
  **GPU sanity + warm starts**: `max_tokens` (96) and a blank-line stop
  hold generations to the one-line contract instead of rambling on the
  GPU; `num_ctx` (8192) stops huge default context windows from eating
  KV memory; `prewarm` loads the model in the background at bind and on
  `#/model` switch; queued asks coalesce so only the newest question
  spends GPU. **Two-part answers**: prose + optional `CMD:` line — the
  command vends into the suggestion list (vendor "engine"), pullable
  with Down like any other suggestion.
  **Heckle band v1 (the 3-4 row layout)**: reserved area grows to
  status + question + explanation rows on a `#` ask and collapses on
  the next command — dynamic winsize, no compositing; engine binding
  is now *late*: an unbound worker re-probes when work arrives (ollama
  started after goulash just works), and unreachable engines error
  visibly instead of parking silently.
  **Bar redesign + chat continuity**: agent content (suggestions,
  notices, band) renders in a contrasting colored block with the static
  chrome right-justified in its own shading; the band holds fixed
  height while open. Asks and answers append to the session log, so
  follow-up questions ("recursively") resolve against the running
  conversation — CI asserts the chat history reaches the prompt.
  **Fixed-height area + proactive commentary**: the goulash area holds
  constant height (rule/question/text/chrome; blank when idle) — no
  terminal resizes mid-session; chrome sits bottom-right in its own
  chip, agent content on plain terminal background with an orange
  suggestion chip cutting into the rule (Claude-Code-style). CMD
  vending hardened: point-of-use directive, history rewritten to teach
  the CMD: shape, and a PATH-executable fallback catches bare-command
  replies. `#/commentary [on|off]` (default on) has the model review
  each command turn and volunteer one tip or stay silent (PASS
  discarded; user asks always win the band; failures silent).
  **Agent memory v1 (flat prime store)**: `#/memory on` (default off)
  bakes a slot-limited pinned store (25 × ≤240 chars,
  `~/.goulash/memory.toml`) into the stable prefix; the model writes
  via `REMEMBER:`/`FORGET:` lines (revision = both in one reply,
  forgets applied first), the user via
  `#/memory add|delete|modify|find|limit` —
  [agent-memory](../architecture/agent-memory.md).
  **Menus, persistence, slot history, `##` chat (v1s)**: bare
  `#/model` opens the modal type-to-filter selector (list scrolls
  under a fixed cursor, `auto` first-class, Enter persists surgically
  to config.toml, Esc/Ctrl-C out); `#/model <name> save` persists from
  the prompt while the bare typed form stays session-only; the **crash
  fuse** (state.toml busy/probation/distrust/last_good marks) keeps a
  model that killed its last run from auto-binding; Down cycles the
  **slot history** of past (suggestion, chat) turns with a frozen band
  and `n/M` indicator; and `##` flips into **chat focus** — grown
  area, transcript, `#`-free follow-ups, Up handing the command to the
  real shell line ([settings-and-nav](../interaction/settings-and-nav.md),
  [down-arrow-protocol](../interaction/down-arrow-protocol.md),
  [chat-mode](../interaction/chat-mode.md)).
- **Remaining**: menus for `#/service`/`#/settings`/`#/memory` (same
  primitive), Up-steps-newer in the slot stack (needs a paste-ack
  channel), richer answer surface, OpenAI-compatible adapter +
  `auto-local` probe chain, Apple FM / cloud providers
  ([distribution](distribution.md)), chat-mode delegated agents (M6).

## Next up — engine & ergonomics backlog

Field-driven, roughly in order of leverage:

1. **Raise the token ceiling.** `max_tokens` (256) was set as a
   runaway backstop *before* streaming existed. With partials flowing
   into the band, the cost of a longer budget is not latency-to-first
   token — it is only the tail nobody reads. Raise it, and consider
   splitting the budget by surface: a one-line bar answer and a `##`
   chat reply do not deserve the same ceiling.
2. **`CMD:` FIRST, prose after.** The command is the payload; the
   explanation is commentary. Emitting the command first means (a)
   truncation eats prose instead of the command, and (b) the
   suggestion chip can populate **mid-stream**, as soon as that line
   completes — a real perceived-latency win, not just a safety one.
   Costs: flip the directive, flip the chat-history shape (mimicry is
   what actually teaches small models the format — see the M4 CMD
   vending fix), and teach `split_answer` both orders so old logs and
   stubborn models still parse. Knob first (`command_first`), default
   once proven.
3. **Thinking level, and a *response-size* budget.** `think:false` is
   currently hardcoded, from the milestone where reasoning models
   spent the whole budget invisibly. Make it a dial — `[engine]
   thinking = off|low|medium|high` plus `#/thinking`. The budget has
   to split with it: what the user actually cares about is **response
   size** (what lands in the band), while thinking is masked spend on
   the same `num_predict` meter. So account them separately —
   `response_tokens` as the visible contract, a thinking allowance on
   top — and make thinking *visible* while it burns time (state label,
   or a dim live line from ollama's separate `thinking` field).
4. **`--help` worth reading, and `--config`.** Sectioned help (the
   `#` / `##` / `#/` surfaces, config path, key table with defaults),
   plus a `git config`-style CLI: `--config print` (effective values
   and where each came from), `--config path`, `--config set k v`
   (reusing the surgical `toml_edit` write-back), and an interactive
   `--config` walkthrough for first run. Hard constraint: goulash is
   zero-setup and stays that way — the walkthrough is offered, never
   required, and never blocks a shell from starting.
5. ~~**Per-model capability schema.**~~ **Built** —
   [model-capabilities](../architecture/model-capabilities.md).
   Field-proven gap: thinking is not one dial across models. gemma3n
   rejects it outright; gpt-oss takes `low|medium|high` but will spend
   an entire budget reasoning and return an empty response; qwen3 uses
   a boolean. `src/models.rs` now resolves each bound model from three
   sources in increasing authority — a longest-prefix family table
   (which alone knows the *dialect* and the realistic reasoning cost),
   ollama's `/api/show` capability list, and a `[models."name"]`
   override. A non-reasoner is sent no `think` field at all (some
   providers reject rather than ignore it), the allowance is sized per
   model, the UI annotates what the dial will actually do, and an empty
   answer now names one cause instead of two.
6. **Text-entry settings.** `#/settings` cycles through preset values,
   which is wrong for numbers — `max_tokens` wants a typed figure, not
   a four-stop carousel. The memory browser's compose field already
   solves this (a modal text input inside a menu row); generalize it
   so a setting can declare *entry* instead of *cycle*.
7. **`#@` working context** — pinned files as near-tool-use, with a
   chrome indicator and an ingest meter:
   [working-context](../architecture/working-context.md). Design
   session before code.
8. **Transcript retention, and `#/study`.**
   `~/.goulash/history/session-*.jsonl` is written forever and pruned
   never — with `record.output` on (default) that is every byte the
   terminal ever printed. It needs a retention policy (age and/or
   total-size cap, pruned at startup) — but the better framing is to
   make those transcripts *earn their keep* first: **`#/study`** kicks
   off a background worker that reads command history and writes
   [memories](../architecture/agent-memory.md) aimed squarely at
   making goulash a better coach for **this** user — their tools,
   their idioms, the fixes they keep re-deriving. Self-directed
   tuning through the memory store, no weights involved.
   Two prerequisites: retention (so study has a bounded corpus rather
   than every byte since install), and **a real memory browser** —
   `[id] text` crammed into a bar row is effectively unusable already
   at 25 slots, and a study run writes far more than a human would
   (which is why the cap goes to 50–100 with study). The
   [menu primitive](../interaction/settings-and-nav.md) already
   exists and works; **`#/memory` becomes an instance of it** —
   scroll, type-to-filter, Enter to act — with review/approve for
   machine-written entries. Cheap, independently useful, and the
   natural next code task.

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
**agent memory tier 2** (prime store shipped in M4 — remaining: the
searchable bank + memory vendor —
[agent-memory](../architecture/agent-memory.md)), fish adapter, remote
markers over ssh
([remote-and-multiplexers](../architecture/remote-and-multiplexers.md)),
`auto-evaluated` steward scopes, bundled llama.cpp option, final name
call — and the road to `brew install goulash`:
[distribution](distribution.md) (license → crates.io name grab →
cargo-dist + tap → provider adapters → notarization → goulash.dev).

## Guiding order
Each milestone is independently usable; trust properties
([positioning](positioning.md)) are built in from milestone 1, not
retrofitted.
