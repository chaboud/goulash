# Interaction Model

The principle: Goulash is **command-invocation driven**. Its moment to
engage the user is when there's a prompt and a blinky cursor
([three gates](../architecture/input-ownership.md)). The one exception is
display-only: the async engine may always be generating into the
[status rows](../architecture/status-rows.md) at the bottom of the
terminal.

## The escalation ladder

```
#    ask     one-shot aside to the LLM, answered inline
##   chat    conversational mode; LLM gets tools, can drive delegated work
↓    pull    accept an async suggestion into the command line
```

## 1. The `#` aside

A line starting with `#` at the prompt is addressed to the LLM, not the
shell — literally an *aside*. `#` is the shell comment character, so a
stray aside reaching a plain shell is harmless.

```
$ # I want to see all of the files created in february
```

When an answer contains command candidates, **selecting one puts the
command in the terminal** — into the prompt as editable text (same
insertion mechanics as the [suggestion list](suggestion-list.md)), so
when something is *nearly* right the user tweaks it and hits Enter.
Nothing runs on selection.

To pass a literal comment through to the shell, escape it: **`\#`** —
goulash strips the backslash and forwards the `#…` line untouched.

Asides and answers are recorded as aside blocks in
[block history](../architecture/block-history.md). Context comes from the
[memory hierarchy](../architecture/memory-hierarchy.md): recent raw
verbatim, logarithmic ramp-off beyond, drill-down tools for the rest.

## 2. `##` chat mode

`##` flips the script into a chat session — the LLM gets exploration and
command tooling, and the splitter pushes: the inner PTY shrinks to the
top third (a plain window-size change to the shell), chat owns the rest.
**Modal, not contended**: all keys go to chat until `##` + Enter toggles
back — like an unfocused tmux split. Full page: [chat-mode.md](chat-mode.md).

## 3. Async suggestions

The LLM is **totally async** to interaction. It vends suggestions into a
scrollable list shown in the status rows — inserting at top only while
the list is unfocused, never growing out from under the cursor. The user
pulls a suggestion into the command line deliberately; nothing ever runs
on its own. Full pages: [suggestion-list.md](suggestion-list.md),
[down-arrow-protocol.md](down-arrow-protocol.md).

## Fallback bindings

On shells without a [line-editor integration](../architecture/shell-integration.md),
bracketed-paste injection covers suggestion insertion, and:

```
Alt-Down       focus suggestion list
# question     message LLM
```

## What the LLM sees

Context is assembled from [block history](../architecture/block-history.md)
via the [memory hierarchy](../architecture/memory-hierarchy.md). TUI
contents and echo-off input are excluded by construction
([opaque-blocks](../architecture/opaque-blocks.md)).
