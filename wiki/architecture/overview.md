# Architecture Overview

Goulash is a **PTY overlay**: it wraps an existing interactive shell,
observes everything flowing through the terminal, maintains a structured
[block history](block-history.md) of the session, and feeds an LLM engine
that can answer `#` asides and offer executable suggestions — without
ever fighting the shell or interactive apps for the keyboard.

```
real terminal
    │
    ▼
Goulash ───── LLM/history engine
 PTY master
    │
 PTY slave
    │
 bash/zsh
    │
 foreground command or TUI
```

## The one-sentence contract

> Read everything, interfere with nothing — unless all the gates say the
> user is at an idle prompt and asked for help.

## Core decisions (each has its own page)

| Decision | Page |
|---|---|
| Launch as `goulash $SHELL`, allocating a PTY and running the real shell on the slave side | [pty-overlay.md](pty-overlay.md) |
| Keyboard ownership decided by terminal job control (`tcgetpgrp`) + shell hooks, **not** process-tree walking | [input-ownership.md](input-ownership.md) |
| Session modeled as a small state machine: PROMPT / COMMAND / INTERACTIVE_CHILD | [session-state-machine.md](session-state-machine.md) |
| Down-arrow suggestion implemented in the shell's own line editor (ZLE/Readline), never at the raw PTY layer | [shell-integration.md](shell-integration.md), [../interaction/down-arrow-protocol.md](../interaction/down-arrow-protocol.md) |
| Transcript stored as typed blocks (command, output, chat, task, summary) woven into one history | [block-history.md](block-history.md) |
| TUIs (vim, less, fzf, ssh, REPLs) recorded as opaque lifecycle blocks; echo-off input never recorded | [opaque-blocks.md](opaque-blocks.md) |
| ssh sessions and tmux are treated as boundaries, not transparently pierced | [remote-and-multiplexers.md](remote-and-multiplexers.md) |

## Terminology

- **Overlay / veneer** — Goulash sits *around* the shell, not inside it;
  the user's shell, dotfiles, plugins, and muscle memory are untouched.
  (This word choice drove the [naming](../naming/decision.md).)
- **Aside** — a `#`-prefixed line addressed to the LLM instead of the
  shell: [../interaction/model.md](../interaction/model.md).
- **Delegated agent** — background work spawned from the session into its
  own tab/thread: [../interaction/delegated-agents.md](../interaction/delegated-agents.md).

## What Goulash is *not*

Not an agent that drives your terminal, not a new shell dialect, not a
replacement prompt. See [../product/positioning.md](../product/positioning.md).
