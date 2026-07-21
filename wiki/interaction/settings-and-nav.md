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
#/status      engine, provider, watcher health, memory-tree stats
#/model       pick the model for a role (watcher/thinker)
#/provider    pick/configure a provider, run bootstrap
#/split       jump straight to the split-height adjuster
#/help        list available #/ commands
```

- Selectors are the same widgets as the menus — arrows, Enter, Esc —
  and obey the same [write-back rule](../architecture/implementation.md).
- `#/` is unambiguous inside the `#` namespace: a leading `/` can't
  start a sensible LLM question, and tab-completion on `#/` can list the
  command set right at the prompt.
- Same commands work from `##` chat as `/status`, `/model`, … (the `#`
  is already implied there).

## Menus too — "both" is the answer

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
