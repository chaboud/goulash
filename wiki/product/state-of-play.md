# State of Play — handoff notes

Written to survive a context compaction. Where the code stands, what to
do next, and the field findings that are expensive to rediscover.

## Branches

| Ref | Meaning |
|---|---|
| `main` | last human-validated promotion; also what `v0.3.0` shipped from |
| `dev` | working line — everything below marked *(dev)* lives here only |
| `rest/v0.1.0`, tags `v0.2.0`, `v0.3.0` | resting points |

Promotion is a fast-forward (`git push origin dev:main`) once a build
has been driven on the mac for a while. The repo's **default branch on
GitHub is still the old `claude/…` branch** — a settings flip only the
owner can do.

## On `dev`, not yet promoted

- **Resize hygiene**: rows never fill the terminal's last cell (that
  flagged them soft-wrapped, and width changes reflowed them into
  fragments); vacated band rows are handed back by repainting from the
  vt100 mirror, never blank-erased; drag-resizes debounce 60 ms and
  **all painting is suspended while one is in flight**.
- **CMD-first** (`[engine] command_first`, default on): the directive
  asks for `CMD:` before the prose, and the engine emits
  `Event::Command` **mid-stream** the moment that line completes, so
  the chip is pullable while the explanation is still arriving. The
  session log mirrors the shape (mimicry teaches small models).
- **Budgets**: `max_tokens` is the *response* budget (512);
  `thinking_tokens` is a separate allowance so reasoning cannot eat the
  answer. `[engine] thinking = off|low|medium|high`, `#/thinking`.
- **`#/memory` is a browser**: filter, scroll, `+ new memory …` compose
  row, arm-then-confirm delete. Default slots 25 → 50.
- **Menus grow**: `[status] menu_rows` (default 8) with a 10-row floor
  for the shell; rows returned on close.
- **`#/settings`** (alias `#/config`): Enter cycles a value, applies it
  live, persists it via the generalized `Config::persist_key`.
- **`#/help`**: a browsable menu of current commands.
- **Per-model capabilities** ([model-capabilities](../architecture/model-capabilities.md)):
  `src/models.rs` resolves what the bound model does with `thinking`
  from a family table (longest-prefix match), ollama's `/api/show`
  capability list, and a `[models."name"]` override, in that order of
  authority. Drives the wire format (a non-reasoner is sent no `think`
  field at all), the reasoning allowance (the model's number, not one
  global), the settings/`#/thinking`/`#/status` annotations, and a
  single-cause empty-answer diagnosis.
- **Deferred-wrap fix**: the cursor is restored with DECSC/DECRC
  (`ESC 7`/`ESC 8`) instead of an absolute CUP rebuilt from the mirror,
  because CUP cannot carry the wrap flag and silently cancelled it —
  the tab-completion listing that cleared one row short. Full mechanism
  in [status-rows](../architecture/status-rows.md).
- **`#/debug`**: `[debug]` config + menu for terminal hackery —
  `cursor_save` (A/B the fix above), `idle_repaint` (is the unprovoked
  paint buying anything?), `wrap_guard`. Defaults are the shipped
  behaviour.
- **`#@` working context v1** (`src/context.rs`): `#@/path <p>` pins
  deterministically, `#@/unset` / `#@/drop` / `#@/list`, bare `#@`
  lists, and `#@ <words>` goes to the model, which answers in
  `PIN:`/`PINCLEAR` verbs. Verbatim or deterministic outline against a
  shared budget, `*` dirty marker set by a stat at prompt turns, chrome
  shows the active `@`. Session-scoped — persisting a pin would force
  the per-cwd-vs-global call that is still open.
- **`#@` digest tier**: over-budget pins get an LLM compression behind
  the deterministic outline, which keeps serving until it lands. Fed
  the *outline* rather than the raw file (bounded by construction — no
  local context window would take 512 KB). Second-class work in the
  worker: one per loop pass, after every interactive job, so an ask
  waits behind at most one compression. Attempts capped at 2, chrome
  shows a percent while cooking, `#@/cancel` abandons the batch.

## Next, in order

1. **Text-entry settings.** `#/settings` cycles presets, which is
   wrong for numbers. The memory browser's compose field is the
   pattern — generalize it so a setting can declare *entry* over
   *cycle*.
2. **Two-lane engagement** — designed, not built:
   [two-lane-engagement](two-lane-engagement.md). Fast stays the sole
   voice; slow researches with tools and contributes suggestions fast
   relays. Start with **the card** (a few lines emitted near the
   question, testing the SWA thesis) — if that does not beat a
   prefix-only digest, the rest is built on sand. Do not build a lane
   scheduler before measuring: ollama slots or two separately-cached
   models may simply do the job.
3. **`#/study`** — background worker mining transcripts into memories
   tuned to coach *this* user. Prerequisites: transcript retention
   (`~/.goulash/history/*.jsonl` grows unbounded today) and
   review/approve for machine-written slots.

## Field findings worth keeping

- **Arrows arrive as SS3** (`ESC O A/B`) in live zsh sessions, not CSI
  (`ESC [ A/B`) — application-cursor mode. Missing this made the modal
  menu look frozen (it read the Esc and typed the rest into the
  filter). Tests that only send CSI will not catch it.
- **A blocking prewarm pins the whole worker** — that was the "frozen
  after `#/model`" report. Warms now run after the job drain, and
  model loads announce themselves (`loading … / ready`).
- **Resize residue is not fully fixed** and cannot be with this
  approach — we paint absolute rows into a screen the emulator is
  concurrently reflowing. The escape hatches are documented in
  [status-rows](../architecture/status-rows.md); full alt-screen
  containment would cost the native scrollback that makes goulash feel
  like a normal terminal.
- **Cargo can silently skip rebuilds** in this container (the clock
  jumps days, confusing mtime freshness). If a change appears to have
  no effect, `cargo clean -p goulash` before concluding anything.
- **`#`, `#/`, `#@` are safe shell syntax by specification** — see
  [shell-integration](../architecture/shell-integration.md). The one
  dependency is zsh's `interactive_comments`, which the adapter sets
  *after* sourcing the user's rc.
- **Never blank-erase inner rows.** The shell only rebuilds its screen
  at a prompt turn, so anything erased between turns leaves a hole
  nothing repairs. Restore from the vt100 mirror instead.
- **Rows that fill the last cell are a hazard family, not one bug.**
  Two distinct failures so far: soft-wrap reflow on resize, and
  deferred-wrap cancellation by an absolute cursor restore. Suspect the
  final column whenever the artifact is off by one row.
- **Relative paths belong to the SHELL, not to goulash.** goulash's own
  cwd is wherever it was launched; the shell has been `cd`-ing ever
  since. Anything path-shaped resolves against the cwd from the OSC
  wire. (Caught by `#@/path` silently failing in a `cd`-ed directory.)

## Test sweep before promoting `dev`

Mid-stream vending (chip before prose, no double-vend); menu open/close
on a short window and inside tmux (it SIGWINCHes the shell twice per
visit); `#/settings` cycling, especially `max_tokens` at 2048; thinking
on a model that supports it; memory at volume (browse/filter/delete/
compose); a drag-resize; and the regression sweep — vim/less
(alt-screen suspends chat), Ctrl-C mid-generation, long output with
commentary firing, `##` chat with CMD-first.

Added by this round, and needing a **real emulator** (e2e drives a PTY
with nothing interpreting the other end, so it cannot see these):

- **Tab-complete, then keep typing.** The original report. Listing
  should clear completely. A/B it with `#/debug` →
  `cursor_save = absolute`, which should bring the orphan row back.
- **`idle_repaint = off`** for a while: does anything actually go
  stale, or was the unprovoked paint buying nothing?
- **`#@/path` on a real command reference**, then ask something only
  that file could answer. Also: pin a directory, watch the outline
  tier, edit a pinned file and confirm the `*`.
- **A big pin on a real model**: watch the chrome percent while it
  cooks, confirm asks stay answerable during it, and compare the
  digest's quality against the outline it replaced (`#@/list` names the
  tier). `#@/cancel` mid-cook. This is the one where a small local model
  might simply do a bad job — if the digest is worse than the outline,
  the tier order is wrong and the fix is a quality gate, not a bigger
  budget.

## Backlog note: virtual terminal, and scrollback

Full alt-screen containment (tmux-style virtual terminal operation) is
the end state that makes every artifact in the hazard family impossible
— we would own every cell. It also makes **scrollback our problem**,
since the native buffer that makes goulash feel like a normal terminal
would no longer exist. Not now; noted so the trade is remembered.
