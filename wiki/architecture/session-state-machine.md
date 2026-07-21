# Session State Machine

Goulash models the wrapped session as a small state machine, driven by
[job-control observation](input-ownership.md) and
[shell hooks](shell-integration.md).

```
PROMPT
  shell owns terminal
  # directives enabled
  LLM suggestion potentially available

COMMAND
  shell or child command executing
  capture ordinary output
  no input interception

INTERACTIVE_CHILD
  another foreground process group owns terminal
  transparent PTY forwarding
  do not touch arrows

PROMPT
  child exited
  command block committed
  LLM may produce next suggestion
```

## Transitions

- **PROMPT → COMMAND**: shell reports command execution starting
  (zsh `preexec`; bash accept-line via Readline integration).
- **COMMAND → INTERACTIVE_CHILD**: `tcgetpgrp()` shows a process group
  other than the shell's owning the PTY.
- **INTERACTIVE_CHILD → PROMPT** (or COMMAND): the child exits and the
  shell regains the foreground; on next prompt display (`precmd` /
  `PROMPT_COMMAND`) the completed command block is committed to
  [block history](block-history.md).

## What each state permits

| | record output | record input | `#` asides | Down-arrow suggestion |
|---|---|---|---|---|
| PROMPT | — | yes (command line) | **yes** | **yes** (gates 2+3) |
| COMMAND | yes | no interception | no | no |
| INTERACTIVE_CHILD | lifecycle only ([opaque](opaque-blocks.md)) | pass-through only; never when ECHO off | no | no |

## Unknown shells

Shells without an [integration adapter](shell-integration.md) can't
signal PROMPT reliably, so they run in a degraded generic mode: PTY
recording and lifecycle blocks work, Down-arrow magic is disabled
(Alt-Down chord instead).
