# LLM Engine: Providers, Caching, Local Models

## Selectable providers

Provider choice is a plugin boundary: Anthropic / OpenAI / others / local
(llama.cpp). Different roles can bind to different providers — see the
two-tier split below.

## Caching stance: local-first

**Decision:** cache-efficiency matters for the **local case**; portable
cached state across cloud providers is a pipe dream and we don't chase
it. But the provider plug-in model is shaped so we *could* get there if
the landscape changes.

- **llama.cpp (the case we optimize):** KV-cache save/restore is **real**
  (`llama_state_*` / session files). A resident local model holds a
  persistent session over the whole terminal lifetime, cheaply appending
  new blocks as they happen — and can snapshot/restore state across
  goulash restarts.
- **API providers (Anthropic, OpenAI, …):** prompt caching is
  prefix-based, server-side, TTL-bounded, and opaque. A provider plugin
  *may* exploit it by keeping an append-mostly context — stable
  `[system + epoch summary]` prefix, append-only tail, compaction only at
  epoch boundaries (one deliberate cache miss each). That's an
  optimization inside the plugin, not an architectural commitment.

### Provider plug-in contract (sketch)

Each provider plugin declares its caching capabilities so the context
assembler can adapt:

```
capabilities:
  kv_save_restore:   yes | no      (llama.cpp: yes)
  prefix_cache:      yes | no      (cloud: yes, opaque)
  preferred_shape:   append-mostly | free-form
```

## Two-tier engine

| Tier | Model | Runs | Jobs |
|---|---|---|---|
| **Watcher** | small local model via llama.cpp (optional bundled copy) | resident, persistent KV | rolling cleanup / [region markers](memory-hierarchy.md), staleness checks, cheap [suggestion](../interaction/suggestion-list.md) drafts |
| **Thinker** | big API model (selectable) | on demand | `#` asides, [`##` chat](../interaction/chat-mode.md), [delegated agents](../interaction/delegated-agents.md), hard suggestions |

This keeps the always-on observation loop free (no per-token API cost, no
privacy egress for raw observation) while heavy reasoning gets a frontier
model. The local tier is **optional but bootstrappable**: never a hard
dependency of the core overlay, but goulash should be able to help set it
up (`goulash bootstrap local` — fetch/build llama.cpp or point at an
existing server, download a vetted small model, wire the watcher role).

## Privacy note

Only the watcher tier sees the raw firehose by default. What leaves the
machine for an API provider is the assembled context (summaries + recent
tail), already filtered by the
[echo-off and opaque-block invariants](opaque-blocks.md).
