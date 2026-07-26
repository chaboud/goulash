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

## Next, in order

1. **Per-model capability schema.** Blocking honest thinking config.
   Thinking is not one dial: gemma3n rejects it, gpt-oss:20b accepts
   `low|medium|high` *and will spend the whole budget reasoning*,
   qwen3 wants a boolean. Needs a small table keyed by model name with
   family fallbacks (`gpt-oss*`, `qwen3*`, `gemma*`) covering: does it
   accept thinking, in what form, and what reasoning budget is
   realistic. Config escape hatch for unknown models. Until this
   lands, an empty answer cannot be diagnosed precisely — goulash
   cannot distinguish "ignored the level" from "burned the budget",
   and the error message says exactly that.
2. **`#@` working context** — design settled enough to argue from, not
   yet built: [working-context](../architecture/working-context.md).
   The user wants a design pass *before* code. Key resolved points:
   LLM-mediated (natural language, `PIN:`/`UNPIN:`/`PINCLEAR` verbs,
   read-only capability performed by goulash not the shell); atomic
   promotion for a file, **checkpointed** for a tree (a directory cook
   can take hours and must be useful while incomplete); no file
   watching — a `*` dirty marker plus asked-for re-cook; secrets gate
   on a per-provider `trusted` flag, not on content; chrome shows the
   active `@` with a percent meter while cooking.
3. **Text-entry settings.** `#/settings` cycles presets, which is
   wrong for numbers. The memory browser's compose field is the
   pattern — generalize it so a setting can declare *entry* over
   *cycle*.
4. **`#/study`** — background worker mining transcripts into memories
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

## Test sweep before promoting `dev`

Mid-stream vending (chip before prose, no double-vend); menu open/close
on a short window and inside tmux (it SIGWINCHes the shell twice per
visit); `#/settings` cycling, especially `max_tokens` at 2048; thinking
on a model that supports it; memory at volume (browse/filter/delete/
compose); a drag-resize; and the regression sweep — vim/less
(alt-screen suspends chat), Ctrl-C mid-generation, long output with
commentary firing, `##` chat with CMD-first.
