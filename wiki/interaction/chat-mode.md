# `##` Chat Mode

**Status: v1 shipped** — the "chat has focus" flow below (toggle,
grown area, transcript, `#`-free follow-ups, Up-handoff to the shell
line, Esc/`##` exit, alt-screen suspend). The delegated-agent tooling
and autonomy rungs remain design.

`#` is a one-shot aside. `##` **flips the script**: it opens a chat
session with the LLM, where the LLM has tooling to explore history and
suggest (eventually run) commands. The escalation ladder:

```
#   ask       one-shot aside at the prompt
##  chat      conversational mode, LLM gets tools
```

## Layout: push the splitter, no compositing

**Decision: no compositing, no faking it.** The chat pane is created the
same way the [status rows](../architecture/status-rows.md) are — by
shrinking the inner PTY's winsize. Entering `##` is, from the shell's
point of view, **just a window size change**:

```
normal:   inner PTY rows = N - status_rows
## mode:  inner PTY rows = N / 3          ← SIGWINCH; shell reflows itself
          chat pane owns the reclaimed rows
exit ##:  resize back; shell reflows again
```

The top third stays the live shell — never hidden, still running, still
streaming output. The shell and any TUI in it reflow exactly as they
would in a real terminal resize (same churn you accept when splitting a
tmux pane). Goulash renders only its own pane; it never repaints the
shell's content. If we can't get there immediately, `##` waits — we don't
ship a fake.

## Modality: `##` is a toggle, not a focus fight

We are in **one mode or another** — like tmux with an unfocused split:

- In shell mode, all keys go to the shell (per the
  [three gates](../architecture/input-ownership.md)).
- After `##` + Enter, **all keys go to chat**. The shell keeps running
  and rendering in its third, but receives no input — you can't touch
  the command window (not even Ctrl-C to its foreground job) until you
  toggle back with `##` + Enter from the chat side.
- There is never a moment where two consumers contend for the keyboard.

This makes input routing trivial and unambiguous: modal focus, not
arbitration.

## "Chat has focus" v1: reaching commands from the chat line

The flow for firing a suggested command while chatting:

- **Up from the chat input** moves focus to the suggestion slot — the
  same single-slot scrollable stack as
  [down-arrow cycling](down-arrow-protocol.md); further Up/Down walk
  older/newer turns.
- **Selecting a command hands it to the real shell line** (bracketed
  paste, focus flips to shell): the user edits with their *own* line
  editor — zle/readline, their bindings, their muscle memory — and
  Enter fires it as an ordinary user command. `##` returns to chat.
- We do **not** re-implement an editable command line inside the chat
  pane. Emulating readline badly is a tarpit, and firing from inside
  goulash's pane would cross the input-ownership line below: paste
  without execute is the invariant. **Decided: keep it pure.** Chat is
  for the multi-turn conversation (no retyping `#` every turn); when a
  command comes up, you drop to the shell and hit Enter yourself.
  There is no Claude-Code-style act-observe loop here — that lives on
  the autonomy dial (`accept-each` and up), opted into explicitly,
  never reached by drift.
- Entering `##` expands the bottom space by a few rows when available —
  a *user-initiated* resize, consistent with the menu rule in
  [settings-and-nav](settings-and-nav.md). A foreground alt-screen app
  (vim) suspends chat focus until it exits.

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

## Note on `#`/`##` as syntax

`#`/`##` are valid shell comments, so a line that leaks to a bare shell
is harmless. For users who genuinely want a comment at the prompt, the
escape is **`\#`** — goulash strips the backslash and passes a literal
`#…` line through to the shell untouched
([interaction model](model.md)).
