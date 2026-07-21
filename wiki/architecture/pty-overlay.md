# PTY Overlay

The core primitive: Goulash allocates a pseudo-terminal and launches the
user's chosen shell on the slave side, sitting on the master side itself.

## Invocation

```
goulash zsh
goulash bash
goulash fish
goulash "$SHELL"
```

Technically the wrapped shell is a **child interactive shell**, not a
"subshell" in the shell-language sense. This resolved the early open
question of "process-chain sensing vs. load-with-a-subshell-argument":
the answer is *both* — wrap a child shell in a PTY, and use terminal job
control on that PTY to sense who owns input
([input-ownership.md](input-ownership.md)).

## Data flow

```
real terminal
    │  keystrokes ↓ / rendered output ↑
    ▼
Goulash  ←──→  LLM/history engine
 PTY master
    │
 PTY slave
    │
 bash/zsh (child interactive shell)
    │
 foreground command or TUI
```

Everything the user types passes through Goulash on its way to the shell;
everything the shell and its children print passes through Goulash on its
way to the screen. Goulash can therefore:

- record the session into [block history](block-history.md);
- detect prompt/command/TUI phases ([session-state-machine.md](session-state-machine.md));
- inject text (a suggestion) **only** when the shell's line editor asks
  for it ([shell-integration.md](shell-integration.md));
- pass keys through untouched whenever an interactive child owns the
  terminal.

## Why not process-tree inspection?

Walking the process tree to guess "is vim running?" is weaker than
job-control state because of:

- pipelines containing several processes
- `exec`
- shell functions and builtins
- subshells
- ssh
- programs that fork helpers

The **foreground process group** is the terminal-native answer to "who
owns the keyboard?" — details in [input-ownership.md](input-ownership.md).

## PTY-level hygiene

Goulash also inspects PTY state directly, e.g. the echo flag:

```
ECHO off → do not record typed input
```

so password prompts and secret-entry tools never land in LLM history —
see [opaque-blocks.md](opaque-blocks.md).
