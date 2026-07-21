# LLM Engine: Providers, Caching, Local Models

## Selectable providers

Provider choice is a plugin boundary: Anthropic / OpenAI / others / local
(llama.cpp). Different roles can bind to different providers — see the
two-tier split below.

## The caching reality check

The wish: "cached state for the LLM up to a particular context, saved and
restored." What's actually available differs sharply by tier:

- **API providers (Anthropic, OpenAI, …):** prompt caching is
  **prefix-based, server-side, TTL-bounded, and opaque**. You cannot
  export or restore it; you can only *exploit* it by keeping a
  byte-stable prefix across calls. There is no portable KV-cache today.
- **llama.cpp:** KV-cache save/restore is **real**
  (`llama_state_*` / session files). A resident local model can hold a
  persistent session over the whole terminal lifetime, cheaply appending
  new blocks as they happen.

### Consequence: append-mostly context with epochs

The [log-ramp-off](memory-hierarchy.md) rewrites old context, which is
cache-hostile. So structure every API call's context as:

```
[ system + epoch summary ]   ← stable prefix; changes only at epoch boundaries
[ event log tail ]           ← append-only between epochs
```

Compaction (re-summarizing, re-ramping) happens only at **epoch
boundaries**, where one deliberate cache miss is paid and a new stable
prefix is minted. Between epochs, every call is a cache hit on the
prefix. The rolling-cleanup pass writes summaries *into the tree*
continuously, but the *serving prefix* only advances at epochs.

## Two-tier engine

| Tier | Model | Runs | Jobs |
|---|---|---|---|
| **Watcher** | small local model via llama.cpp (optional bundled copy) | resident, persistent KV | rolling cleanup / [region markers](memory-hierarchy.md), staleness checks, cheap [suggestion](../interaction/suggestion-list.md) drafts |
| **Thinker** | big API model (selectable) | on demand | `#` asides, [`##` chat](../interaction/chat-mode.md), [delegated agents](../interaction/delegated-agents.md), hard suggestions |

This keeps the always-on observation loop free (no per-token API cost, no
privacy egress for raw observation) while heavy reasoning gets a frontier
model. The local tier is **optional and pluggable** — bundling llama.cpp
is an ops/maintenance commitment, so it must never be a hard dependency
of the core overlay.

## Privacy note

Only the watcher tier sees the raw firehose by default. What leaves the
machine for an API provider is the assembled context (summaries + recent
tail), already filtered by the
[echo-off and opaque-block invariants](opaque-blocks.md).
