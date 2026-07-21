# Delegated Agents

Long-running or exploratory work can be handed off from the prompt:

```
$ # go recon this folder
```

The delegated task runs in its own **sub-tab / thread**, with its own
report and context, branching from the main terminal timeline — and
merges its results back as a task block in
[block history](../architecture/block-history.md).

## Model

- **Branching** (the BranchSH idea): each delegated task is a branch from
  the main timeline; the primary session is never blocked or polluted.
- **Threads** (ThreadSH): long-running delegations keep their own
  transcript and can be inspected independently.
- **Orbit** (OrbitSH): delegated agents orbit the primary session without
  interfering with it — they never take the interactive terminal's
  keyboard ([input ownership](../architecture/input-ownership.md) applies
  only to the user's live PTY).
- **Pulse updates**: a running task reports status via replaceable pulse
  blocks rather than appending noise.

## Character: scout & steward

The naming exploration produced a useful vocabulary
([runner-ups](../naming/runner-ups.md)):

- **Scout** — reconnaissance: read, summarize, report
  (`# go recon this folder`).
- **Steward** — bounded delegation with permission scopes: allowed to
  *do* things, but within explicitly granted bounds.

Both stay subordinate to the [coach positioning](../product/positioning.md):
the user delegates explicitly; agents never self-initiate actions in the
live session. (Kasha — *Knowledge-Augmented Shell Assistant* — and Tahini
— *Terminal-Aware History and Intelligence* — are candidate names for the
agent persona and the history subsystem respectively.)

## Open questions

Permission-scope design, sub-tab UX, and report format are unresolved —
tracked in [../product/open-questions.md](../product/open-questions.md).
