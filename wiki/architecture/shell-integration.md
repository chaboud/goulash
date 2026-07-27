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

**zsh leaves `interactive_comments` unset for interactive shells**, so
the adapter does `setopt interactivecomments`. Load order is handled —
the generated rc sources the user's `.zshrc` *first*, then ours.

Note the asymmetry: `#` asides are intercepted by the **zsh** ZLE
accept-line widget, which reads `$BUFFER` before the shell lexes it
(so goulash sees exactly what was typed, pre-expansion). Bash has no
equivalent interception today — asides there are just comments the
shell ignores, and bash users reach suggestions via **Alt-Down**.

That asymmetry is also the reason the option turns out to be
unnecessary — see below.

## `interactivecomments` is a dependency we don't have

Measured, not argued (dev, 2026-07). Setting the option costs more than
it buys, and the thing it was buying was already covered.

**What it costs.** Under `interactivecomments`, zsh lexes `# mini` as a
comment, so the completion system is handed `CURRENT=0`, `words=()` and
an empty `PREFIX`. There is no current word — nothing to filter by and
nothing to insert — so Tab dumps an **unfiltered listing of the whole
directory** on the first press, and a second press splices a match in
at the cursor instead of replacing the word:

```
#@ Fa   nointeractivecomments  →  1 tab: "#@ Farm"   (correct)
#@ Fa   interactivecomments    →  1 tab: everything in cwd, buffer unchanged
                                  2 tabs: "#@ FaFarmAnimals.md"
```

That second line is where the mangled `#/modelAnimals.md` rows in
history come from — a filename welded onto a `#` command with no
separator. It is *not* an under-erase and has nothing to do with the
[deferred-wrap hazard](status-rows.md#deferred-wrap-why-the-cursor-restore-is-decsc-not-cup),
which is what it looks like. Every `#` line does it, including `#/mod`,
which has no file argument at all. And the option is not scoped to
asides: `echo a # b` silently changes meaning for every command the
user types.

**What it was buying.** History expansion runs *before* tokenization, so
comment rules can't protect an aside containing `!!` — the argument for
needing the option. But the aside never reaches the parser: the
accept-line widget can push it into history with `print -s` and blank
`$BUFFER`, so the shell parses an empty line. Probed under a real pty
with `nointeractivecomments`: `# what does !! do` and
`# and $(echo BAD) too` both arrive at the widget **verbatim** — no
expansion, no substitution, no error. Not letting the parser see the
line is a stronger guarantee than asking it to treat the line as a
comment.

The `\#` escape changes shape rather than disappearing: it still means
"this one is not for goulash", and since a comment does nothing either
way, the widget can blank it too for the same observable result.

**If a user wants the option anyway** — and `echo a # b` stripping is a
reasonable thing to want — it becomes a setting rather than a
dependency, because the aside path no longer relies on it:

| `[shell] interactive_comments` | |
|---|---|
| `off` (default) | leave zsh exactly as the user configured it; Tab behaves like bare zsh |
| `on` | `setopt` it, **and** install the Tab widget below so completion still works |

The Tab widget is the compensating half, needed only in the `on` case:
swap the `#…` sigil for a same-role non-comment stand-in (`: `), run
whatever widget was bound to `^I` before us, then swap the sigil back.
The line is no longer a comment for the duration of the completion, so
`PREFIX` is correct and matches replace the word. Prototyped and
measured working (`#@ Fa` → `#@ Farm`, `#@ /abs/path/Fa` → `…/Farm`,
`# Fa` → `# Farm`), with `#/`-verb completion from a static table as a
bonus. Cost: the buffer briefly reads `: ` instead of `#` if the
underlying widget is interactive (`menu-select`, `fzf-tab`).

## Adapter fidelity audit

The contract is that goulash changes **nothing** about the shell except
the Down arrow and the async `#` interception. Six live deviations, all
verified under a pty:

| deviation | what it breaks | fix |
|---|---|---|
| `setopt interactivecomments` | Tab completion on `#` lines; `echo a # b` semantics for *every* command | don't set it (above) |
| `zle -N accept-line` | **clobbers** any prior widget — `zle -lL` shows only ours. We load after `.zshrc`, so atuin, `magic-enter`, and syntax-highlighting wrappers are silently dropped | capture with `zle -lL accept-line`, delegate instead of calling `.accept-line` |
| `zle -N bracketed-paste` | same clobber; kills `bracketed-paste-magic` and `bracketed-paste-url-magic` | same |
| `bindkey ^[[A ^[OA ^[[B ^[OB` | clobbers zsh-history-substring-search, fzf, autosuggestions; our fallback calls the *builtin* `up-line-or-history`, not what was bound | capture with `bindkey -L`, delegate |
| `add-zsh-hook precmd` appends | our `$?` capture runs **last**, after every user precmd hook — any hook that runs a command clobbers the exit code we report | prepend into `precmd_functions` |
| `$(base64)` in `precmd` | two forks per prompt for the cwd, two more per aside — latency we added to every prompt | encode in-shell, or send cwd only when it changes |

`zle -lL <widget>` and `bindkey -L <seq>` both hand back the prior
definition, so **capture-and-delegate costs about three lines per
hook**. This needs a fidelity test: drive a pty with a fake plugin that
wraps `accept-line` and binds the arrows, then assert the plugin still
runs with the adapter loaded.

## rc-file loading: what actually gets sourced

zsh, via the ZDOTDIR stubs — correct order (`.zshenv` → `.zprofile` →
`.zshrc`), with three gaps:

- **No `.zlogin` or `.zlogout` stub.** `prepare()` accepts `-l` and
  `--login`, so a login zsh under goulash silently skips the user's
  `~/.zlogin`.
- **`$ZDOTDIR` still points at goulash's dir during `.zshenv` and
  `.zprofile`** — only the `.zshrc` stub restores it. A dotfile setup
  that does `fpath+=$ZDOTDIR/functions` in `.zshenv` gets the wrong
  directory.
- When the user had no `ZDOTDIR`, the stub sets it to `$HOME`; real zsh
  leaves it **unset**, so `[[ -n $ZDOTDIR ]]` tests flip.

bash is **broken for login shells**. Verified: `bash -l -i --rcfile X`
does not read `X` — bash ignores `--rcfile` for login shells and reads
`/etc/profile` then `~/.bash_profile` / `~/.bash_login` / `~/.profile`.
`prepare()` accepts `-l`/`--login` and takes the `--rcfile` path anyway,
so `goulash bash -l` gets **zero integration and no warning**. The
non-login path is fine. Fix: for a login bash, inject via a generated
profile (or `--init-file` after re-exec) rather than `--rcfile`, or fall
back to passthrough with a visible notice.

Bash's Tab is unaffected by any of this: readline doesn't lex comments,
so `# Fa` completes to `# Farm` there with `interactive_comments` on —
verified.
