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

## Deferred wrap: why the cursor restore is DECSC, not CUP

A second, unrelated bug lived in the same family — *rows that fill the
last cell* — and it produced a much stranger symptom: a tab-completion
listing that cleared **one row short**, leaving an orphan row of
filenames under the prompt.

The mechanism. When a glyph lands in the terminal's final column the
cursor enters **deferred wrap**: it reads as still on that row, but the
next glyph moves to the next line. No escape sequence can name that
state. Goulash used to end each paint with an absolute `CUP` rebuilt
from its vt100 mirror — which restores the *position* and silently
cancels the *flag*. The shell's next character then overwrote the last
cell instead of wrapping, every row below shifted by one, and ZLE's own
line accounting — which still assumed the wrap happened — cleared one
row too few.

Two things made this reachable in ordinary use: goulash paints on an
**idle tick** whether or not anything changed, so it interrupts a line
editor at arbitrary moments; and nothing tells ZLE it happened.

The fix is to stop describing the cursor and let the terminal remember
it: `ESC 7` / `ESC 8` (DECSC/DECRC) save and restore position,
attributes, **and the wrap flag**. The one cost is that the emulator has
a single save slot, shared with the child — a child that saves, gets
painted over, then restores would get our cursor back. Line editors do
not use DECSC, and full-screen apps that do live on the alternate
screen where the band is suspended, so the window is narrow. DECSC does
not carry cursor *visibility*, which is re-asserted from the mirror
either way.

Both levers are live under [`#/debug`](../interaction/settings-and-nav.md):
`cursor_save = decsc|absolute` to A/B the fix itself, `idle_repaint` to
find out whether the unprovoked paint is buying anything, and
`wrap_guard` to defer a paint whenever the inner cursor is parked in the
last column — belt-and-braces above the real fix.

**Verified in code and by unit test** (byte shape: DECSC first, no CUP
after DECRC). The behavioural half needs a real emulator — e2e drives a
PTY with nothing interpreting the other end — so confirmation that the
completion residue is gone is a hands-on check.

## Geometry repair: let ZLE re-derive, don't tiptoe

goulash resizes the inner PTY whenever its own area changes height — a
menu opening, chat taking focus. zsh gets a SIGWINCH and redraws, but
ZLE's accounting for a line it has already drawn can describe the old
geometry, and a **wrapped** line redrawn shorter then clears one row too
few. The tail of the previous line is left stranded on screen
(field-reported: a recalled `#/settings` with `rh | head -n 10 …` still
hanging off the end of it).

The tempting fix was to stop resizing — draw menus inside the fixed
band, as v0.3.0 did. That was **rejected as a fix**, though it may still
be right as a UI choice:

- It would not have worked. v0.3.0 had residue too, from the
  tab-completion path, without any menu resize. The screenshots that
  reported this show `cursor_save: decsc` already active. Resize is an
  aggravator, not the cause — the *wrap* is the cause, which is the
  hazard family this page already documents.
- It cannot help a real terminal drag, which produces the same SIGWINCH
  and which no layout choice on our side avoids.

The actual repair is to stop tiptoeing around ZLE's bookkeeping and make
it re-derive:

```zsh
TRAPWINCH() { zle reset-prompt }
```

**Unconditional, and that matters.** The documented idiom is
`zle && zle reset-prompt`, but `zle` reports *inactive* inside a WINCH
trap even while ZLE is reading — measured under a pty — so the guard
suppresses exactly the case that needs it. When ZLE really is idle the
call is a harmless no-op returning 0. A user's own `TRAPWINCH`, or a
`trap ... WINCH`, is captured and run first.

**Unverified.** The e2e harness drives a pty with nothing rendering the
far end, so an under-erase is *invisible* to it by construction — the
missing bytes were never sent. A deterministic repro built for this
found nothing, which is a statement about the harness, not the bug.
Confirmation is a hands-on check.

(That blind spot is now closable: `pyte` gives the harness a real screen
model, so a future test can compare rendered screens rather than byte
streams. Worth doing before the next terminal-hackery change.)

## Colour: orange is selection

The strip has three chip colours and one rule between them.

| | |
|---|---|
| orange (`208`, black text) | **the thing Enter pulls right now** |
| grey (`238`, white text) | a chip that is present but not selected |
| the chrome grey (`100`) | goulash's own state, never pullable |

Orange does **not** mean "suggestion" and does not mark a category. When
a turn carries both a fast answer and a
[researched finding](two-lane-engagement.md), browsing into the finding
turns it orange and turns fast's chip grey — the colour moves with the
selection rather than labelling what kind of thing each row is.

Category marking was the first attempt and it was the wrong axis: a
finding is already visibly a finding, because it is indented under the
answer it fills in. What a glance cannot otherwise recover is what the
next keystroke will do, and that is what the strongest colour is spent
on.
