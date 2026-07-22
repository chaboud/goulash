# Settings & Spatial Navigation

How the user drives goulash's own surfaces — the
[suggestion list](suggestion-list.md), menus, and settings — without a
mouse and without leaving the keyboard's home position. Source of truth
is always [`~/.goulash/config.toml`](../architecture/implementation.md);
the in-terminal UI reads and writes it.

## TV-remote model

Goulash's space is navigated **spatially, D-pad style**:

- **Down** past history enters our space (the suggestion list) —
  [down-arrow-protocol](down-arrow-protocol.md). In `##` chat, Down
  likewise reaches a menu.
- **Left off the left edge** of a list/menu slides into
  settings/control.
- Within a setting like **split height**, Up/Down adjust the value with
  **live effect** (the [splitter](../architecture/status-rows.md) moves
  as you arrow), Enter commits, Esc reverts.

- **Tab cycles panes within the down space**: once focus is in
  goulash's widgets, Tab moves between the suggestion list and the
  scrollable [heckle/annotation band](heckle-mode.md); Shift+Tab
  reverses. Because this only applies *after* entering the down space,
  shell tab-completion is never shadowed — at the prompt, Tab still
  belongs to the shell. (Watch for muscle-memory collisions with other
  tools' Tab conventions, e.g. Claude Code's — keep it rebindable.)

### Rules that keep it sane

1. **Spatial nav exists only inside goulash widgets** — lists, menus,
   value adjusters. Never in text fields (chat input, command line),
   where arrows mean editing. The shell side is untouched by definition
   ([input-ownership](../architecture/input-ownership.md)).
2. **Edges must be visible.** Off-the-edge moves are invisible
   affordances, and TV remotes only work because the screen shows the
   layout — so when a list is focused, the status row hints the map
   (e.g. `◀ settings · ↑↓ scroll · ⏎ accept · esc close`).
3. **Live-adjust + write-back.** Committed changes persist to
   `config.toml` immediately; no separate "save" step, no drift between
   UI and file.

## `#/` commands: direct addressing

Browsing (remote/menus) is for discovery; **`#/` commands are for going
straight there**. At the prompt, `#/name` opens a quick selector or
status surface in goulash's space:

```
#/status              engine, provider, watcher health, memory-tree stats
#/model               pick the model for a role (watcher/thinker)
#/provider            pick/configure a provider, run bootstrap
#/split               jump straight to the split-height adjuster
#/heckle              toggle/resize the heckle band
#/help                list available #/ commands
```

**Commands take one argument — the single most obvious swivel.** The
bare form opens the selector; the argued form goes straight to done:

```
#/model watcher       select which role you're re-binding
#/provider ollama     bind provider directly
#/split 40            set split height in one shot
#/heckle off          collapse the band
```

One argument, the obvious axis, nothing clever. Anything needing more
than that belongs in the selector or the TOML.

- Selectors are the same widgets as the menus — arrows, Enter, Esc —
  and obey the same [write-back rule](../architecture/implementation.md).
- `#/` is unambiguous inside the `#` namespace: a leading `/` can't
  start a sensible LLM question, and tab-completion on `#/` can list the
  command set right at the prompt.
- Same commands work from `##` chat as `/status`, `/model`, … (the `#`
  is already implied there).

## The menu primitive (v1 spec — get this one thing right)

Bare `#/model` (and later `#/service`, `#/settings`, `#/memory`) drops
into **one shared widget**: a filterable, scrollable list in the goulash
area. Every menu is this; there is no second kind.

```
Menu { title, breadcrumb, items[(label, detail)], filter, cursor, on_commit }
```

Resolved rules, in priority order:

1. **Modal, and only ever user-opened.** While a menu is up, goulash
   owns the keyboard completely — the first true exception to
   shell-owns-input, tolerable *only* because the user asked for it by
   name. Goulash never opens a menu on its own. Exit is bulletproof:
   Esc backs out one level, Esc at the top level closes, Ctrl-C always
   aborts everything from anywhere.
2. **Typing filters; there are no per-item hotkeys.** Shortcut letters
   can't coexist with type-to-filter (`g` must mean "gemma", not "jump
   to item G") — and even digits are out, because model names are full
   of them (`qwen2.5`, `gemma3:12b`). Filter-first is the fzf model and
   it wins. Backspace edits the filter; the filter is shown.
3. **Edges visible** (rule 2 above): the bottom row of the menu shows
   the live keymap — `↑↓ move · type to filter · ⏎ select · esc back` —
   plus `n/M` match count and scroll arrows when the list overflows.
4. **The band grows for the menu, then gives the rows back.** A fixed
   4-row area can't browse 14 models; the menu temporarily deepens the
   reserved area (more [winsize arithmetic](../architecture/status-rows.md),
   ~10 rows) and restores on close. This is a *user-initiated* resize —
   the "never resize mid-session" rule guards against goulash surprising
   the user, not against the user asking for room.
5. **Never block the shell.** Menus that need a probe (`/api/tags`)
   open instantly with a `probing…` row and fill in async.
6. **Multi-level = a stack of the same primitive.** `#/service` is just
   the level above models: providers → models. Esc pops one frame;
   breadcrumb shows where you are (`engine ▸ ollama ▸ model`).

Commit semantics (see write-back rule): **Enter selects for now AND
persists as the default** — TV-remote semantics; your TV doesn't forget
its input on power-cycle. `auto` is a first-class list entry that
restores the probe chain. The typed form `#/model <name>` stays
**session-only** — that's the "try it once" path, and the asymmetry is
deliberate: browsing is choosing a default, typing is an experiment.

Write-back edits `config.toml` **surgically** (`toml_edit`-style,
comments and formatting preserved) — never a full re-serialize, which
would dump every default and nuke the user's comments.

Spatial shortcuts cover the handful of live-tunable things (split
height, reserved row count, list visibility). A conventional menu (Down
in `##`, or from the settings pane) covers the long tail: provider
selection and [bootstrap](../architecture/llm-engine.md), autonomy dial
([chat-mode](chat-mode.md)), keybinds, redaction settings. Same
write-back rule. Power users skip all of it and edit the TOML.

## Open

Whether the left-edge gesture wants a chord alternative (for terminals
or users where arrow semantics feel overloaded), and how much of the
settings tree is worth exposing in-UI vs. file-only —
[open-questions](../product/open-questions.md).
