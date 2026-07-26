# `#@` Working Context: Pinned Files as Near-Tool-Use

**Status: design, to be worked through together before building.**

`#@ <path>` pins a file (or a directory) into a **working context** that
rides in the prompt's [stable prefix](llm-engine.md), next to
[pinned memories](agent-memory.md). Instructions, a guide, a runbook —
whatever the user says matters right now.

```
#@ commandRef.md          pin a file
#@ ./deploy/              pin a tree (walked, capped, compacted)
#@                        list what's pinned, with sizes and freshness
#@ drop 2                 unpin
```

## Why it's the highest-leverage feature on the list

The killer case is **a vendor-authored command guide**: drop a
`commandRef.md` next to a proprietary CLI, and the model can suggest
correct invocations for a tool it has never seen. That is *kind of*
tool use — no function-calling protocol, no schema negotiation, just
the reference material in front of the model when it writes a
[`CMD:` line](suggestion-vendors.md). It turns goulash from "knows
common Unix" into "knows **your** tools" with a file a team can check
into their repo.

**Nothing about the autonomy model changes**: goulash still only ever
*suggests* commands, pulled and run by the user
([chat-mode](../interaction/chat-mode.md) keeps it pure). The pin
changes what it knows to suggest, not who runs it.

## Visible in the chrome

A pin is state the user must be able to see at a glance, so the
[status chrome](status-rows.md) carries it: the active `@` (basename,
`+N` when several) and, while cooking, a **percentage meter** — ingest
is async and can take real time on a tree or a large digest, and a
silent multi-second cook is exactly the "am I frozen?" failure we hit
with model loads. Done cooking, the meter collapses back to the plain
`@ name` marker.

## Ingest tiers (the budget is the design)

The stable prefix is the KV-cache asset; a fat pin destroys the
latency work. So ingest is **compaction, not concatenation**, chosen by
size against a hard `context_files_max_chars` budget:

| Tier | When | What lands in the prompt |
|---|---|---|
| **Verbatim** | fits the budget | the text as-is |
| **Digest** | over budget | **LLM compression** — the model rewrites it down, biased toward commands, flags, invariants |
| **Outline** | tree, or still over after compression | structure + headings + per-file one-liners, drillable later |

The trigger is mechanical: if the content exceeds the share of the
context window the pin is allowed, it gets compressed rather than
truncated. Truncation would cut a command guide in half mid-table;
compression keeps the invocations and drops the prose.

Digesting costs a model call, so it is **async and background**: the
pin registers immediately with an `ingesting …` marker, the session
stays responsive, and the digest swaps in when it lands. Never block a
prompt turn on an ingest.

## Freshness

Cheap `stat` per ask; on mtime/size change, re-ingest in the background
and **keep serving the old digest until the new one is ready**. Stale
beats stalled. Digests cache under `~/.goulash/context/` keyed by
path + mtime + size, so re-pinning across sessions is free.

## Hazards to settle before writing code

- **Secret sweeping.** A directory pin can inhale `.env`, keys, history
  files. Needs a skip list, `.gitignore` respect, a size/count cap, and
  a *visible* list of what was actually ingested — the
  [privacy invariant](block-history.md) is about typed input; this is a
  new exposure surface and deserves its own rule.
- **Cloud egress.** With a local engine a pin never leaves the machine.
  With a metered/remote provider it does, on every ask. That asymmetry
  should be explicit at pin time, not buried.
- **Budget sharing.** Working context + [memories](agent-memory.md) +
  session log all compete for the same prefix. One accounting, one
  visible number (`#/status`), and a defined eviction order.
- **Epoch churn.** Every re-ingest invalidates the prefix cache. Batch
  and debounce; a file being edited in another pane must not re-cook
  the prompt on every save.

## Open questions (for the design session)

1. **Scope**: per-cwd (project-local pins, auto-restored on `cd` into
   the tree) or global? Per-cwd feels right for `commandRef.md`, global
   for a personal style guide — possibly both, with the list showing
   which is which.
2. **Digest authorship**: model-written (better, costs a call, needs a
   model that's up) or deterministic extraction (headings, code
   fences, first-N-lines — instant, dumber)? Deterministic as the
   fallback when no engine is bound, at minimum.
3. **Retrieval vs pinning**: always-in-prefix (simple, cache-friendly,
   bounded) or top-k retrieval per ask (scales further, breaks the
   stable prefix)? Start pinned; the [memory bank](agent-memory.md)
   tier is where retrieval belongs.
4. **Auto-offer**: if a repo has `goulash.md` / `AGENTS.md` / a
   `README` with a command section, do we offer to pin it on `cd`?
   Nice, but goulash never acts unasked — a one-line notice at most.
5. **Globs and URLs**: `#@ docs/*.md`, `#@ https://…`? Both plausible,
   both widen the hazard surface.
