# The Suggestion List

Suggestion sources are **totally async** to interaction. Pluggable
[vendors](../architecture/suggestion-vendors.md) — deterministic rules,
history matching, n-grams, and LLMs — vend into one list surfaced in the
[status rows](../architecture/status-rows.md); each entry carries vendor
attribution. The user pulls from it when *they* want
([down-arrow protocol](down-arrow-protocol.md)).

## Semantics

- **Insert at top, only while unfocused.** New suggestions appear at the
  head of the list. The list is scrollable, so earlier suggestions remain
  reachable below.
- **It never grows out from under the cursor.** The moment the user
  focuses the list (down-arrow past history, or the chord), ordering
  **freezes**; new arrivals queue and are merged only after focus is
  released.
- **Selection binds to suggestion identity, not index.** Every suggestion
  has an ID; the "accept" action resolves the ID, never "row 3". This
  kills the race where the list shifts between glance and keypress even
  across freeze-boundary edge cases.

## Staleness

A suggestion is contextual to the moment it was generated. Each one is
tagged with its generation context: position in
[block history](../architecture/block-history.md), cwd, and relevant
state. When the world moves (commands run, cwd changes), suggestions
whose context no longer holds are **marked stale** (dimmed, pushed down)
or expired outright. A stale `rm`-flavored suggestion silently pointing
at a path whose meaning changed is the failure mode this exists to
prevent.

While focused, the list is also the entry point to goulash's wider
surface: **Left off the left edge** slides into settings/control —
[settings-and-nav.md](settings-and-nav.md).

## Delivery into the command line

Two mechanisms, complementary:

1. **Line-editor integration** (zsh ZLE / bash Readline) — precise,
   history-aware ([shell-integration](../architecture/shell-integration.md)).
2. **Bracketed-paste injection** — Goulash writes the accepted suggestion
   to the PTY master wrapped in bracketed-paste markers
   (`ESC[200~ … ESC[201~`), so any modern shell inserts it as editable
   text **without executing it**. This is shell-agnostic and makes the
   generic-shell fallback nearly as good as the integrated path.

Either way the text lands in the user's prompt for editing; nothing runs
until the user hits Enter ([positioning](../product/positioning.md)).

## History

Every vended suggestion — and whether it was accepted, edited, ignored,
or expired stale — is recorded as a suggestion block, feeding the causal
trace in [block history](../architecture/block-history.md).
