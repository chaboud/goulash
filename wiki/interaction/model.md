# Interaction Model

Two primitives, both available only when the user is at an idle prompt
([the three gates](../architecture/input-ownership.md)):

## 1. The `#` aside

A line starting with `#` at the prompt is addressed to the LLM, not the
shell — literally an *aside* (the AsideSH insight,
[runner-ups](../naming/runner-ups.md)). `#` is already the shell comment
character, so a stray aside that reaches a plain shell is harmless.

```
$ tar -xzvf release.tgz
$ # what did that just unpack, roughly?
$ # go recon this folder        ← spawns a delegated agent
```

Asides and their answers are recorded as aside blocks in
[block history](../architecture/block-history.md). Delegation-style
asides fork background work: [delegated-agents.md](delegated-agents.md).

## 2. The Down-arrow suggestion

At an idle prompt, when ordinary history is exhausted, Down Arrow reveals
the LLM's suggested next command — inserted into the command line for the
user to edit or accept. Exact semantics:
[down-arrow-protocol.md](down-arrow-protocol.md).

The suggestion is an *offer*, never an action. The user runs everything.
This is the heart of the [coach positioning](../product/positioning.md).

## Fallback bindings

On shells without a [line-editor integration](../architecture/shell-integration.md):

```
Alt-Down       show suggestion
# question     message LLM
```

## What the LLM sees when you ask

Context is assembled from [block history](../architecture/block-history.md):
the causal trace of recent commands, results, asides, and suggestions —
with a live tail of recent activity and asynchronous backfill of older
context. TUI contents and echo-off input are excluded by construction
([opaque-blocks](../architecture/opaque-blocks.md)).
