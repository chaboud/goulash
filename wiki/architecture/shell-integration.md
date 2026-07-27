# Shell Integration Adapters

The [PTY overlay](pty-overlay.md) works with any shell; small per-shell
integrations make it *smart*. They provide gate 2 and gate 3 of
[input ownership](input-ownership.md) and the transition signals for the
[state machine](session-state-machine.md).

## The contract

> **The shell needs to run like it would without goulash. Goulash just
> adds the down arrow, and the way to dispatch via a `#`, which resolves
> async.**

Everything below is in service of that sentence, and it is worth reading
as a *limit* rather than a summary. Two additions, both user-initiated,
neither of which can fire on its own:

- **Down** — and only past the end of history, on a single-line buffer.
  Every other Down is whatever Down already did, including whatever a
  plugin bound there.
- **`#`** — caught before the shell lexes it, answered on another
  thread. Nothing blocks the prompt, ever.

That is the whole surface. No options are set, no semantics change, no
widget is replaced without being delegated to. `test_adapter_fidelity`
and `test_rc_loading` exist to make the claim falsifiable, and
`test_rc_loading` is **differential** — it runs the shell bare and under
goulash and demands the same startup sequence, so the reference is the
real shell rather than our idea of it.

**Where we still deviate, stated plainly** (each is a deliberate trade,
not an oversight):

| | |
|---|---|
| the arrows | claimed by design — this is the feature |
| `\#` | blanked rather than passed to the lexer. A comment does nothing either way, so it is observably identical, but the mechanism differs |
| bash: one `history 1` per prompt | what buys bash the `#` surface at all. Net **fewer** forks than before, since the cwd report now only fires when the cwd moves |
| bash `-l` | emulated, because bash has no `--profile-file`. Startup files and the adapter are right; `shopt -q login_shell` reads false and `$0` is not `-bash` |

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

**zsh leaves `interactive_comments` unset for interactive shells**, and
the adapter used to `setopt interactivecomments` to close this. It no
longer does — the dependency was illusory and the option was expensive.
See below.

Each shell reaches the aside a different way:

- **zsh** intercepts at the ZLE `accept-line` widget, which reads
  `$BUFFER` before the shell lexes it — goulash sees exactly what was
  typed, pre-expansion, and the line never reaches the parser.
- **bash** has no equivalent hook (a `#` line is a comment, so it never
  executes and the DEBUG trap never fires). What bash *does* do is
  record it in history, so the aside is recovered at the next prompt by
  noticing that the history number advanced onto a line starting with
  `#`. One turn later than zsh, and invisible at that scale.

## `interactivecomments` was a dependency we didn't have

Measured, not argued (dev, 2026-07). Setting the option cost more than
it bought, and the thing it was buying was already covered. **Removed.**

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
reasonable thing to want — it can become a setting rather than a
dependency, because the aside path no longer relies on it. **Not built**;
recorded here so the shape is known:

| `[shell] interactive_comments` | |
|---|---|
| `off` (today, and the only behaviour) | zsh exactly as the user configured it; Tab behaves like bare zsh |
| `on` (unbuilt) | `setopt` it, **and** install a Tab widget so completion still works |

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
the arrows and the async `#` interception. Seven deviations were found
under a pty, and all seven are now fixed — covered by
`test_adapter_fidelity`, which loads a plugin that wraps the same
widgets and asserts it still runs.

| deviation | what it broke | how it works now |
|---|---|---|
| `setopt interactivecomments` | Tab on `#` lines; `echo a # b` for *every* command | not set; the widget blanks the buffer instead |
| `zle -N accept-line` | **clobbered** any prior widget outright. We load after `.zshrc`, so atuin, `magic-enter`, syntax-highlighting wrappers were silently dropped | `$widgets[accept-line]` is re-registered under a private name and delegated to |
| `zle -N bracketed-paste` | same clobber; killed `bracketed-paste-magic` / `-url-magic` | same |
| `bindkey ^[[A ^[OA ^[[B ^[OB` | clobbered zsh-history-substring-search, fzf, autosuggestions; the fallback called the *builtin* `up-line-or-history`, not what was bound | `bindkey -L` captures the prior widget per sequence; the goulash path only takes over past the end of history |
| zsh `add-zsh-hook precmd` appends | our `$?` read ran **last**, behind every user hook — so the exit code goulash reported was whatever the last hook left | a capture hook is moved to the front of `precmd_functions` and returns the status unchanged, so user hooks see what they always saw. A classic bare `precmd` function is wrapped for the same reason |
| bash preexec armed at the *start* of `PROMPT_COMMAND` | a plugin's own prompt hook fired the DEBUG trap, consumed the arming, and the user's real command was never reported — goulash sat in `cmd` state forever | arming moved to a `__goulash_ready` appended **last**, plus a history-number check so an unmoved history number can never be mistaken for a new command |
| bash `trap ... DEBUG` | replaced any existing DEBUG trap (bash-preexec, direnv, profilers) | the previous trap is captured and `eval`'d first |
| `$(base64)` in `precmd` | two forks per prompt just to report an unchanged cwd | cwd is sent only when it moves |

Two of those fixes are subtle enough to be worth stating on their own,
because both fail **silently** and both present as "the shell echoes
every line and runs none of them":

- **`zle -N` inside `$(...)` registers the widget in a subshell.** The
  capture has to set a global, not print one.
- **bash withholds the DEBUG trap from functions *and* from sourced
  files** unless `functrace` is on. `trap -p DEBUG` from inside the
  adapter reports the default, so the plugin we meant to preserve looks
  like it was never there. `PROMPT_COMMAND` is evaluated in the
  top-level context and is the only place the real answer is visible,
  so the read happens there and the value is passed in as an argument.
  Setting a trap from inside a function is fine; only reading one is
  blocked.

## rc-file loading: what actually gets sourced

`test_rc_loading` is **differential, not hardcoded**: it runs the shell
bare, runs it under goulash with the same flags, and demands the same
startup files in the same order with `$ZDOTDIR` reading the same way.
The bare run is the reference — anything else is goulash changing how
the machine starts a shell. Both shells, with and without `-l`.

**zsh**, via the ZDOTDIR stubs. `ZDOTDIR` is swapped to the user's value
(or **unset**, when they never had one) around each of their files and
swapped straight back, so `fpath+=$ZDOTDIR/functions` in a `.zshenv`
resolves the way it would without goulash. The `.zshrc` stub restores it
for good, which is also what makes `.zlogin` and `.zlogout` load
natively — zsh expands `$ZDOTDIR` afresh for each startup file, so
neither needs a stub of its own.

The bugs here were about ZDOTDIR's *value*, not about files being
skipped: it pointed at goulash's own directory while the user's
`.zshenv` and `.zprofile` ran, and was set to `$HOME` afterwards when
the user had never set it — flipping every `[[ -n $ZDOTDIR ]]` test.
(`.zlogin` did load, by the accident of the `.zshrc` stub restoring
ZDOTDIR before zsh looked for it. The differential test is what
established that; reading the code suggested otherwise.)

**bash login shells** used to get *zero* integration, silently. Verified:
`bash -l -i --rcfile X` reads neither `X` nor `~/.bashrc` — bash ignores
`--rcfile` for login shells and reads `/etc/profile` then the first of
`~/.bash_profile`, `~/.bash_login`, `~/.profile`, and there is no
`--profile-file` to point anywhere else. Since `prepare()` accepts `-l`
and `--login`, the only way to reach a login bash is to drop the login
flag and replay that sequence in the generated rcfile before sourcing
the adapter. Two things the emulation does not reproduce: `shopt -q
login_shell` reads false, and `$0` is not `-bash`.

Bash's Tab was never affected by any of this: readline doesn't lex
comments, so `# Fa` completes to `# Farm` there with
`interactive_comments` on — verified.
