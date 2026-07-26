# Shell Integration Adapters

The [PTY overlay](pty-overlay.md) works with any shell; small per-shell
integrations make it *smart*. They provide gate 2 and gate 3 of
[input ownership](input-ownership.md) and the transition signals for the
[state machine](session-state-machine.md).

## The wire protocol (implemented)

Adapters talk to goulash over a **private OSC channel** embedded in the
shell's own output — no sockets, no files, survives ssh by construction:

```
ESC ] 7770 ; A                BEL    prompt displayed        (precmd)
ESC ] 7770 ; B ; <b64 cmd>    BEL    command about to run    (preexec)
ESC ] 7770 ; D ; <exit code>  BEL    command finished
ESC ] 7770 ; P ; <b64 cwd>    BEL    cwd report
```

Goulash strips these from the stream before it reaches the real terminal
(a bare terminal would ignore them anyway) and records them as
prompt/cmd/cmd_end/cwd events in [block history](block-history.md).
Payloads are base64 so command text can contain anything. The scripts
live in `shell/goulash.zsh` and `shell/goulash.bash` and are inert
(`return 0`) outside a goulash session.

## Auto-injection (implemented)

Requiring an rc-file edit before `#` and Down work was an onboarding
failure (field-tested). The adapter scripts are **embedded in the
binary** and injected automatically when goulash launches `zsh` or
`bash` with plain flags (`-i`/`-l` only):

- **zsh**: the ZDOTDIR trick — a generated dotdir whose
  `.zshenv`/`.zprofile`/`.zshrc` stubs source the user's real files
  first, restore `ZDOTDIR`, then source the adapter.
- **bash**: `--rcfile` pointing at a generated wrapper that sources
  `~/.bashrc` then the adapter.

Custom args, unknown shells, or `[shell] auto_integrate = false` fall
back to untouched passthrough plus the manual one-line source.

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

## Why `#`, `#/`, `#@` are safe as shell syntax

By specification, not by luck. POSIX defines a comment as: a word
beginning with `#` causes that word and everything up to the next
newline to be **discarded during tokenization**. Expansions happen
*after* tokenization, so `$(...)`, backticks, `${...}`, quotes,
redirects and pipes inside an aside are inert — there is no stage at
which they can run. `/` and `@` carry no meaning after `#`; `#/model`
and `#@ notes.md` are just comment text.

The one mechanism that sits *outside* the grammar is **history
expansion**, which runs before tokenization — so comment rules would
not protect it. Both shells close that hole explicitly:

- **bash**: the history library's `history_comment_char` is `#`, and
  when it starts a word the rest of the line is excluded from history
  expansion.
- **zsh**: history expansion happens in the lexer, which strips
  comments first when `INTERACTIVE_COMMENTS` is set.

That option is the one real dependency: **zsh leaves
`interactive_comments` unset for interactive shells**, so the adapter
does `setopt interactivecomments`. Load order matters and is handled —
the generated rc sources the user's `.zshrc` *first*, then ours, so
nothing downstream can unset it.

Note the asymmetry: `#` asides are intercepted by the **zsh** ZLE
accept-line widget, which reads `$BUFFER` before the shell lexes it
(so goulash sees exactly what was typed, pre-expansion). Bash has no
equivalent interception today — asides there are just comments the
shell ignores, and bash users reach suggestions via **Alt-Down**.
