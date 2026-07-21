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

Above the flat block sequence sits a hierarchy of LLM-set region markers
and summaries used for context retrieval —
[memory-hierarchy.md](memory-hierarchy.md).

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

## The seen-model: displayed ≠ seen

Every byte the terminal ever displayed transits goulash, and the vt100
mirror knows the *current* screen cell-for-cell. That supports three
tiers of knowledge:

1. **On screen now** — known exactly (unexploited so far; "what's this
   error?" should mean the visible one, not the last N blocks).
2. **Ever displayed** — known exactly (it's the transcript). The
   emulator's scrollback *contents* are no mystery; only the viewport
   position during scroll-back is unknowable (accepted limitation).
3. **Plausibly seen by the human** — modelable: dwell time (content
   present across quiescent frames) plus terminal focus reporting
   (CSI ?1004h focus in/out events). A block can carry a seen-exposure
   annotation — "user watched this error for 8s" vs. "scrolled past
   unseen" — which is real signal for answer quality and for the
   summarizer's notion of what mattered.

## Privacy invariant

Input typed while the PTY has ECHO off is **never** recorded into any
block — see [opaque-blocks.md](opaque-blocks.md).
