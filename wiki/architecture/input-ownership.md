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

That is zsh. ZLE hands the widget `HISTNO` and `HISTCMD`, so it can
answer "is there history left?" exactly.

### When the line editor cannot answer

Readline cannot. A `bind -x` handler can run on a key, but bash only
began exposing the line being edited — `READLINE_LINE`, `READLINE_POINT`
— in **4.0**, and macOS still ships **3.2.57**, where a handler can see
nothing and change nothing. Measured, not inferred: on 3.2 the handler
fires and `READLINE_LINE` is unset; on 5.2 it reads and rewrites the
line. There is no version of gate 3 available on Apple's bash.

So for shells whose editor cannot be asked, goulash answers gate 3 from
the overlay, with the narrowest proxy that is still honest:

```
nothing typed since the prompt, and no Up pressed yet
```

In exactly that state readline's own Down is a **no-op** — it is already
at the end of history, with nothing to move to. Claiming it therefore
takes nothing from anyone. The moment any other key arrives, ownership
latches back to the shell for the rest of that line, and a fresh prompt
resets it. goulash also tracks what it typed, so Down again replaces it
with the next slot and Up walks back to the empty line — the same single
axis zsh gets, kept honest by the fact that goulash put every one of
those characters there itself.

## The cardinal rule

**Do not intercept Down at the raw PTY layer while anything else could
want it.** Gates 1 and 2 are not negotiable: an interactive program owns
the PTY, and no prompt means no claim. Where the shell's editor can
arbitrate, it must — that is zsh, and the widget stays the mechanism.

The rule was originally absolute, and answered shells with no editor
arbitration by giving them a chord instead (Alt-Down). That was wrong in
one direction: the chord is undiscoverable, the README promises plain
Down to everyone, and bash users got a key that did nothing. The rule is
now scoped to its actual intent — never take Down from a legitimate
claimant — and an empty, untouched, prompt-active line has none.

The residual risk is a Readline plugin that binds Down at an empty fresh
prompt. It is a much smaller surface than ZLE's (where autosuggest,
fzf-tab and menu selection all live, and where goulash still refuses to
intercept), and it is the price of the gesture working at all on the
bash people actually have.

## See also
- [opaque-blocks.md](opaque-blocks.md) — what happens while gate 1 fails
- [../interaction/model.md](../interaction/model.md) — the user-facing behavior these gates protect
