# Down-Arrow Protocol: `goulash-down-or-suggest`

How the user pulls from the async [suggestion list](suggestion-list.md)
into the command line. Implemented as a **shell-editor operation**, never
a raw-PTY key intercept — the cardinal rule of
[input ownership](../architecture/input-ownership.md).

## Behavior

```
goulash-down-or-suggest:
    if ordinary history-forward can move:
        do ordinary history-forward
    else if the suggestion list is non-empty:
        focus the list (ordering freezes)
        further ↓/↑ scroll it; Enter/Tab accepts; Esc dismisses
    else:
        do nothing
```

History navigation always wins; the list opens only *past the end of
history* — muscle memory is never violated. While focused, new
suggestions queue instead of reordering the list, and acceptance resolves
a suggestion **ID**, not a row number
([suggestion-list.md](suggestion-list.md)).

## Preconditions

All [three gates](../architecture/input-ownership.md) must hold: shell is
the foreground process group, primary prompt is active, line editor
confirms we're past current history. Otherwise Down Arrow passes through
untouched (completion menus, multiline editing, vi mode, fuzzy finders,
and every interactive app keep their keys).

## Insertion mechanics

Accepted text lands in the command line for editing — via the shell's
line editor where integrated, or **bracketed-paste injection** to the PTY
master (`ESC[200~ … ESC[201~`) on generic shells, which inserts without
executing. Details: [suggestion-list.md](suggestion-list.md).

## Per-shell implementation

| Shell | Mechanism | Notes |
|---|---|---|
| zsh | ZLE widget | ZLE knows history position; should be clean |
| bash | Readline; `bind -x` initially | simple version has edge cases; perfect gate-3 awareness may need deeper Readline work |
| fish | reader integration | |
| other | `Alt-Down` chord + bracketed paste | see [shell-integration](../architecture/shell-integration.md) |

## Slot history: cycling past the newest suggestion (shipped, two-way)

One continuous axis: zsh history **above**, the neutral empty line at
zero, the slot stack **below**. Down steps older; Up slides back
newer and lands on neutral, where the next Up is plain zsh history.
The shell-side trick that makes Up safe: the zsh adapter wraps
`bracketed-paste`, so it records exactly what goulash pasted —
"is this line an untouched slot?" is then a local buffer comparison
that cannot drift, and any edit instantly returns Up to native
history. The session resolves every pull with a (possibly empty)
bracketed paste so the shell's expectation flag always clears.

The pullable slot is a **single-slot view over the history of
(suggestion, chat message) turns** — just like shell history, but for
our side of the conversation:

```
↓ past history end   pull newest suggestion (slot 1)
↓ again              slot 2 (older) — buffer repastes, band shows that
                     turn's chat text
↑                    walk back newer; past slot 1, restore the original
                     buffer and return to normal shell history
```

Rules:

- **Browsing freezes the goulash area.** While the user is in the slot
  stack, new suggestions/answers queue instead of repainting — no
  mutation under the user's cursor. Unfreeze on edit, Enter, or Ctrl-C;
  never on a timer.
- **Editing the buffer exits the stack.** The existing buffer-match
  guard already does this: if the line no longer equals a slot's text,
  cycling stops touching it.
- **The rule row shows position**: scroll indicators (`▲ 2/7 ▼`) ride
  the right end of the rule — the same real estate as the idle ingress
  tip, which they replace while browsing. Arrows only render for
  directions that can actually move.
- The stack holds turns **that vended a command** — decided, not open:
  commands are the anchor. Their chat text rides along in the band;
  prose-only turns are reachable through `##` chat scroll instead —
  cycling a slot whose entry pastes nothing would feel broken. Capped
  (~50).
- In `##` chat mode, **Up from the chat line lands on this same slot**
  ([chat-mode](chat-mode.md)) — one mechanism, two doors.

## History record

Each pull — accepted, edited, ignored, or expired — is a suggestion block
in [block history](../architecture/block-history.md).
