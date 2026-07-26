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

`reserved` is dynamic: the status row plus the
[heckle band](../interaction/heckle-mode.md) when open (and the whole
bottom section grows to the chat pane in
[`##` mode](../interaction/chat-mode.md)). Every change is just another
winsize update to the inner PTY.

The wrapped shell — and everything inside it, including inner tmux,
full-screen TUIs, alternate-screen apps — believes the terminal is
`N - reserved` rows tall. The reserved rows are **outside its world**:
no yielding, no repaint fights, no coordination protocol. On `SIGWINCH`,
subtract and propagate.

## What this costs

Two hazards learned in the field: **ED (erase-below, `ESC[J`) is not
bounded by scroll regions** — line editors emit it on every refresh and
it wipes straight through the reserved rows, so the repaint must ride in
the *same write batch* as the erase (a repaint in a later frame renders
as flicker; no repaint renders as a vanished bar). And a scroll-region
reset followed by scrolling output *within one chunk* can drag the
reserved rows into the inner region — chunks must be split and the
region re-pinned at the trigger boundary.

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

## Known limitation: resize spray (partially fixed)

Dragging a terminal window still sprays some fragments into the
scrollback, though far less than before. Three causes were found and
fixed (dev, 2026-07): band rows filled the terminal's last cell, so
emulators flagged them soft-wrapped and reflowed them into extra lines
on every width change; the band left stale copies behind on rows it
vacated (now handed back by repainting from the vt100 mirror — never
blank-erased, since the inner world only rebuilds at a prompt turn);
and there was no debounce, so every SIGWINCH of a drag repainted into
a terminal still mid-reflow.

**What remains**: a vigorous drag still leaks fragments. The residue is
inherent to painting absolute rows into a screen the emulator is
concurrently reflowing — we write where the band *was* a moment ago.
Options if it ever becomes intolerable:

- **Full containment** (tmux-style): run the inner world on the
  alternate screen and own every cell, repainting from our mirror.
  Bulletproof, and a *lot* of machinery — plus it costs the native
  scrollback that makes the overlay feel like a normal terminal. The
  small elegance of the current approach is worth some spray.
- **Suppress painting while a resize is in flight** and repaint once
  the geometry has been stable for longer than the current 60 ms
  settle. Cheap to try; trades a briefly-empty band for less residue.
