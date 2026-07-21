# Status Rows: The Bottom of the Terminal Is Ours

Like byobu's status bar: Goulash reserves **1–2 rows (configurable) at
the bottom of the terminal** as its own space — where async
[suggestions](../interaction/suggestion-list.md), pulse status of
[delegated work](../interaction/delegated-agents.md), and mode indicators
live, without ever touching the prompt line.

## The trick: shrink the inner PTY

Do what tmux does for its status line — **lie about the terminal size**:

```
real terminal:      rows = N
inner PTY winsize:  rows = N - reserved
outer scroll region (DECSTBM): rows 1 .. N - reserved
```

The wrapped shell — and everything inside it, including inner tmux,
full-screen TUIs, alternate-screen apps — believes the terminal is
`N - reserved` rows tall. The reserved rows are **outside its world**:
no yielding, no repaint fights, no coordination protocol. On `SIGWINCH`,
subtract and propagate.

## What this costs

Goulash must track enough VT state (cursor position, alternate screen,
scroll regions) to draw its rows without corruption — i.e., a **partial
terminal emulator** is unavoidable. This was already implicitly required
for observation ([opaque-blocks](opaque-blocks.md) detection); drawing
makes it explicit. Keep it minimal: track state, never re-render the
inner screen. There is **no compositor anywhere in the design** — the
[`##` chat pane](../interaction/chat-mode.md) reuses this exact
mechanism, just with a bigger number: push the splitter by shrinking the
inner PTY further and own the reclaimed rows.

**Window-size management is the load-bearing wall of the whole product**:
status rows, the `##` splitter, and tmux nesting all reduce to doing
winsize/SIGWINCH/scroll-region bookkeeping exactly right.

## Nesting: byobu/tmux both ways

- **Goulash inside tmux/byobu:** just a program on a pane's PTY — works
  by construction; each pane can run its own Goulash
  ([remote-and-multiplexers](remote-and-multiplexers.md)).
- **tmux/byobu inside Goulash:** the inner tmux gets the shrunken size
  and stacks its own status bar above our rows — same experience as
  byobu-over-tmux today. Livable; a config flag can auto-hide our rows
  when an inner multiplexer is detected, for purists.

## Why this rocks for interaction

The status rows give the async LLM a place to be visible **without any
claim on the keyboard or the prompt line**. Suggestion *preview* lives
here; suggestion *acceptance* is a deliberate user action
([down-arrow](../interaction/down-arrow-protocol.md) or chord). That
split — passive display in our rows, active insertion only on request —
takes most of the pressure off fragile line-editor integration and is the
cleanest expression of the [three gates](input-ownership.md).

## Principled interaction point

Goulash is command-invocation driven by nature: its moment to engage the
user is **when there's a prompt and a blinky cursor**
([state machine](session-state-machine.md) PROMPT state). The status rows
are the only channel allowed to update outside that moment — and they're
display-only.
