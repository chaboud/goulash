# Block History

The transcript model: the session is not a flat scrollback but a sequence
of **typed blocks** — commands, output, chat asides, task branches, and
summaries woven into one history. (This model is strong enough that it
nearly named the product: BluSH, BlockSH, WeaveSH, ThreadSH, TraceSH and
PulseSH were all [names for aspects of it](../naming/runner-ups.md).)

## Block types (working set)

| Block | Contents | Source |
|---|---|---|
| **Command block** | command line, exit status, captured output, cwd, timing | committed when the next prompt appears ([state machine](session-state-machine.md)) |
| **Opaque block** | lifecycle only: program, duration, exit status, observable side effects | interactive children / TUIs ([opaque-blocks.md](opaque-blocks.md)) |
| **Aside block** | `#` question and the LLM's answer | [interaction model](../interaction/model.md) |
| **Suggestion block** | proposed command, whether accepted/edited/ignored | [down-arrow protocol](../interaction/down-arrow-protocol.md) |
| **Task/branch block** | a delegated agent's thread: its own report and context, branching from the main timeline | [delegated agents](../interaction/delegated-agents.md) |
| **Pulse block** | most-recent-state, *replaceable* update — newer state overwrites rather than appends | live status of background work |

## Properties

- **Causal trace**: the history preserves the chain
  *command → result → question → suggestion → action* (the TraceSH idea) —
  this is what makes LLM context assembly meaningful rather than a raw
  scrollback dump.
- **Live tail + historical backfill**: the engine maintains attention over
  recent activity while older context arrives asynchronously (the
  FloSH/AttnSH idea).
- **Replaceable updates**: pulse blocks let long-running work update its
  status without flooding history.
- **Branching**: each delegated task is a branch from the main terminal
  timeline with its own sub-history, merged back as a report block.

## Privacy invariant

Input typed while the PTY has ECHO off is **never** recorded into any
block — see [opaque-blocks.md](opaque-blocks.md).
