# `##` Chat Mode

`#` is a one-shot aside. `##` **flips the script**: it opens a chat
session with the LLM, where the LLM has tooling to explore history and
suggest (eventually run) commands. The escalation ladder:

```
#   ask       one-shot aside at the prompt
##  chat      conversational mode, LLM gets tools
```

## Layout

Chat takes over most of the terminal, **but the top third stays the live
shell**. The shell is never fully hidden — it remains the primary
artifact, and the user can watch commands land while chatting. Exiting
`##` returns the full screen to the shell. Rendering builds on the same
compositing machinery as the [status rows](../architecture/status-rows.md).

## What the LLM can do in chat mode

- Explore the full chat + observation history via
  [retrieval tools](../architecture/memory-hierarchy.md) — drill from
  summaries down to raw blocks.
- Propose commands, which surface as accept-able actions.
- Run delegated work: "go find this shit for me", "run operations that
  do XX" — the user monitors rather than drives.

## Where do its commands run?

**Not in the user's live shell.** Agent-initiated commands execute in
[delegated PTYs](delegated-agents.md) (their own threads/branches in
[block history](../architecture/block-history.md)), with results and
pulse status visible in the chat pane. The live shell's keyboard and
prompt remain user-owned at all times — the
[input-ownership](../architecture/input-ownership.md) invariants are
never suspended, even in chat mode.

## Autonomy dial (door kept open)

Design the permission model now, ship the conservative end first:

```
suggest-only     LLM proposes; user runs           (default)
accept-each      LLM queues commands; user approves one by one
auto-evaluated   LLM runs within a granted scope; user monitors
```

`auto-evaluated` is the steward model — bounded scopes (filesystem
subtree, no network, dry-run first, …) defined in
[delegated-agents](delegated-agents.md). Scope grants should be explicit,
visible, and revocable.

## Note on `##` as syntax

`#`/`##` are valid shell comments, so a line that leaks to a bare shell
is harmless. Genuine interactive comments are rare; still, keep a
configurable escape (e.g. `#!` or a leading space) for users who really
do type comments at the prompt.
