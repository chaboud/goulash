# Heckle Mode

> **Status: v1 implemented.** Layout (top to bottom of the reserved
> area): suggestion row first — Down from the shell reaches it
> immediately — with the static chrome (identity/shell/state `#`
> geometry) right-justified in its own shading, visually separate from
> the agent's colored content block; then the question row; then
> exactly `band_rows` explanation rows. The band is **fixed-height
> while open** (no mid-answer jitter) and resizes only at open/close —
> pure winsize arithmetic — and as of v2, the area holds a constant
> height with blank rows when idle (field feedback: mid-session
> resizing sucks). Proactive commentary (`#/commentary`, default on)
> reviews each command turn and may volunteer one tip — the
> rate-limited-sass rule enforced by the PASS contract. Knobs:
> `[status] band`, `band_rows`, `[engine] commentary`.

MST3K energy: alongside vended commands, the agent gets a small band of
**visible commentary space** — because sometimes the explanation is more
valuable than the command.

## The band

- Lives in goulash's reserved area, directly above the
  [status row](../architecture/status-rows.md): up to a configured
  number of lines (**default 3 visible**), scrollable when commentary
  runs longer.
- **Collapsible** with a carat toggle (and `#/heckle` /
  `#/heckle off|on|N` — [settings-and-nav](settings-and-nav.md)).
  Collapsed state persists to config.
- Opening/closing the band is just more winsize arithmetic: reserved
  rows go from `1` to `1 + heckle_lines` and the inner PTY resizes
  accordingly — the same load-bearing machinery as everything else.

## What goes in it

- **Explanation attached to a suggestion**: the
  [suggestion list](suggestion-list.md) shows *what*; the heckle band
  shows *why* — "your push failed because the branch has no upstream;
  this sets it and pushes." Selecting a suggestion surfaces its
  explanation.
- **Running commentary** on what's happening — the watcher noticing the
  thing you didn't ("that rm just followed a failed cd — you deleted
  from the wrong directory").
- Vendor-attributed like everything else; deterministic
  [vendors](../architecture/suggestion-vendors.md) can heckle too
  (rules vendor explanations are canned strings — free and instant).

## Rules of the heckle

1. **Display-only, like all reserved-row content** — the band never
   takes keyboard focus except through explicit
   [navigation](settings-and-nav.md) into it: from the down space, Tab
   cycles between suggestion list and heckle band (Shift+Tab reverses)
   for scrolling longer commentary.
2. **Replaceable, not append-only**: commentary updates in place
   (pulse-block semantics) — it's a ticker, not a log. The full record
   still lands in [block history](../architecture/block-history.md).
3. **Rate-limited sass.** Commentary must clear a usefulness bar, same
   spirit as the suggestion confidence gate. An agent that quips on
   every command is a passenger-seat backseat driver; tone/verbosity
   is a setting ([open questions](../product/open-questions.md)).

## Why this is more than a cute feature

It resolves a real tension: a bare command in a list is opaque exactly
when trust matters most (unfamiliar flags, destructive operations). The
band gives every suggestion an inline "why" without the user having to
enter [`##` chat](chat-mode.md) — the escalation ladder gets a half-step.
