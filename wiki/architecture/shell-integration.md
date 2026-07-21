# Shell Integration Adapters

The [PTY overlay](pty-overlay.md) works with any shell; small per-shell
integrations make it *smart*. They provide gate 2 and gate 3 of
[input ownership](input-ownership.md) and the transition signals for the
[state machine](session-state-machine.md).

## Per-shell plan

### zsh — first-class
- `precmd` / `preexec` hooks mark prompt display and command start
  (zsh exposes these specifically around prompt display and execution).
- A **ZLE widget** implements
  [goulash-down-or-suggest](../interaction/down-arrow-protocol.md);
  ZLE knows the history position, so this should be clean.

### bash — first-class, more work
- `PROMPT_COMMAND` runs before PS1 display → prompt-active signal.
- Readline integration for the Down-arrow widget. A simple `bind -x`
  implementation may work initially but will have edge cases; perfect
  history-position awareness (gate 3) may require deeper Readline
  integration.

### fish
- Prompt/event functions plus reader integration.

### Unknown / unsupported shells — generic PTY mode
- Full recording and [opaque-block](opaque-blocks.md) tracking still work.
- **No Down-Arrow magic.** Instead, unambiguous bindings:

```
Alt-Down       show suggestion
# question     message LLM
```

This gives broad shell compatibility without pretending every shell has
the same line-editor semantics.

## Packaging sketch

Under the [working name](../naming/decision.md):

```
goulash            core binary (PTY, history, LLM engine)
goulash-zsh        zsh adapter (hooks + ZLE widget)
goulash-bash       bash adapter (PROMPT_COMMAND + Readline)
```

## Remote hosts

Shell adapters only see the local shell. Remote awareness over ssh
requires Goulash or compatible shell markers on the remote host — see
[remote-and-multiplexers.md](remote-and-multiplexers.md).
