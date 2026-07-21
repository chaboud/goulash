# Opaque Blocks & Secret Hygiene

## Interactive apps become opaque blocks

While an interactive child (vim, less, fzf, a REPL…) owns the terminal
([INTERACTIVE_CHILD state](session-state-machine.md)), Goulash forwards
bytes transparently and does **not** stream every alternate-screen redraw
to the LLM. TUIs are opaque unless they explicitly integrate.

Goulash still observes and records the *lifecycle*:

```
vim started
vim ran for 4m12s
vim exited 0
cwd/repository changed
```

That lifecycle summary is committed to [block history](block-history.md)
as an **opaque block** — enough for the LLM to reason about what
happened ("you edited that file for four minutes") without ingesting a
TUI's screen noise.

## Echo state = secret hygiene

Goulash inspects the [PTY's](pty-overlay.md) termios state:

```
ECHO off → do not record typed input
```

Password prompts, `sudo`, ssh passphrases, and other secret-entry tools
turn echo off; their input must never land in LLM history. This is a hard
invariant of [block history](block-history.md), independent of any
LLM-side filtering.

## Boundary cases

ssh sessions and tmux are whole-terminal interactive children and get the
same opaque treatment — with their own page:
[remote-and-multiplexers.md](remote-and-multiplexers.md).
