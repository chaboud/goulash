# Input Ownership: Who Owns the Down Arrow?

The founding constraint: Goulash must read what's going on in the shell
**without fucking with interactive apps that need to take the Down key**
(vim, less, fzf, ssh, REPLs, completion menus…).

## The three gates

Down Arrow may reveal an LLM suggestion **only when all three are true**:

```
1. Is the underlying shell the foreground process group?
2. Has the shell declared that its primary prompt is active?
3. Does the shell-specific line editor say we're past current history?
```

## Gate 1 — terminal job control

The terminal already maintains a foreground process group, and
`tcgetpgrp()` on the [PTY](pty-overlay.md) reports which process group
currently controls it. When vim, less, fzf, ssh, a REPL, or any other
interactive program runs, that group owns the PTY — Goulash passes every
key through untouched. This is the terminal-native answer; process-tree
walking is unreliable (pipelines, `exec`, builtins, subshells, ssh,
forked helpers).

## Gate 2 — prompt-active declaration

Foreground group alone is insufficient: when bash executes a builtin, the
shell may remain the foreground process even though no prompt is shown.
Tiny [shell integrations](shell-integration.md) (zsh `precmd`/`preexec`,
bash `PROMPT_COMMAND`, fish prompt events) tell Goulash explicitly when
the primary prompt is up. This also drives the
[session state machine](session-state-machine.md).

## Gate 3 — line-editor position

Even at an active prompt, Down Arrow may have legitimate meaning: a
completion menu is open, a multiline command is being edited, vi editing
mode, a fuzzy finder widget… So the suggestion behavior lives *inside*
the shell's line editor as a widget
([goulash-down-or-suggest](../interaction/down-arrow-protocol.md)) that
first tries ordinary history-forward and only offers the suggestion when
history is exhausted.

## The cardinal rule

**Do not intercept Down at the raw PTY layer.** Plugins — completion
selectors, fuzzy finders, ZLE widgets, Readline menus, vi mode — may want
Down Arrow even while the shell is technically active. Interception
belongs in the editor layer, where the shell can arbitrate. For shells
without integration, use an unambiguous chord instead (Alt-Down) — see
[shell-integration.md](shell-integration.md).

## See also
- [opaque-blocks.md](opaque-blocks.md) — what happens while gate 1 fails
- [../interaction/model.md](../interaction/model.md) — the user-facing behavior these gates protect
