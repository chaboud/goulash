# Down-Arrow Protocol: `goulash-down-or-suggest`

The suggestion reveal is implemented as a **shell-editor operation**, not
a raw-PTY key intercept — the cardinal rule of
[input ownership](../architecture/input-ownership.md).

## Behavior

```
goulash-down-or-suggest:
    if ordinary history-forward can move:
        do ordinary history-forward
    else if an LLM suggestion exists:
        expose or insert the suggestion
    else:
        do nothing
```

History navigation always wins; the suggestion appears only *past the end
of history* — so muscle memory is never violated.

## Preconditions

All [three gates](../architecture/input-ownership.md) must hold:
shell is the foreground process group, primary prompt is active, and the
line editor confirms we're past current history. Otherwise Down Arrow is
passed through untouched (completion menus, multiline editing, vi mode,
fuzzy finders, and every interactive app keep their keys).

## Per-shell implementation

| Shell | Mechanism | Notes |
|---|---|---|
| zsh | ZLE widget | ZLE knows history position; should be clean |
| bash | Readline; `bind -x` initially | simple version has edge cases; perfect gate-3 awareness may need deeper Readline work |
| fish | reader integration | |
| other | **not bound** — `Alt-Down` chord instead | see [shell-integration](../architecture/shell-integration.md) |

## History record

Each suggestion — and whether it was accepted, edited, or ignored — is a
suggestion block in [block history](../architecture/block-history.md),
feeding the causal trace the LLM reasons over.
