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

## History record

Each pull — accepted, edited, ignored, or expired — is a suggestion block
in [block history](../architecture/block-history.md).
