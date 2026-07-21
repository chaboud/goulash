# LLM Engine: Providers, Caching, Local Models

## Selectable providers

Provider choice is a plugin boundary: Anthropic / OpenAI / others / local
(llama.cpp, ollama, platform engines). Different roles can bind to
different providers — see the two-tier split below.

## Functional out of the box: the probe chain

Goulash must operate with **zero config editing** on first run. At
startup (with consent, and cached thereafter), probe in order and bind
the best available engine to the watcher role:

```
1. ollama detected (localhost:11434)          → use it, no setup at all
2. macOS + Apple Silicon + Apple Intelligence → Apple Foundation Models
                                                 (watcher tier only)
3. offer: goulash bootstrap local             → fetch/build llama.cpp +
                                                 vetted small model
4. API key present (env or providers.toml)    → cloud provider
5. none of the above                          → no-LLM tier: overlay,
                                                 history, status rows,
                                                 and the deterministic
                                                 suggestion vendors
```

Level 5 matters — and it isn't dumb: the [PTY overlay](pty-overlay.md)
and [block history](block-history.md) never require an LLM, and the
rules/history/n-gram [suggestion vendors](suggestion-vendors.md)
(thefuck-style corrections, fish-style history matches) make goulash
worth installing before any model is configured.

### Platform engines (mac-first notes)

- **Apple Foundation Models** ("it's fucking there"): zero download,
  free, on-device, private — ideal watcher-tier economics. Caveats: the
  framework API is Swift-only (needs a small shim called from
  [Rust](implementation.md)), requires a recent macOS on Apple Silicon,
  the model is small (~3B-class) with a modest context window, and its
  content-safety layer can refuse benign-but-scary-looking shell content.
  All fine for turn summarization and region labeling; never the thinker.
- **MLX**: fast on Apple Silicon but Python-centric tooling and immature
  Rust bindings — skip for now; revisit if mlx-rs matures.
- **ONNX Runtime**: cross-platform, but the LLM model/quantization
  ecosystem is far weaker than GGUF/llama.cpp — skip.
- **llama.cpp** remains the only engine exposing KV save/restore, so it
  stays the *preferred* watcher when the user runs bootstrap; ollama and
  Apple FM plugins declare `kv_save_restore: no` and the context
  assembler adapts (per-turn watcher jobs barely need persistent KV
  anyway).

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

## Latency mechanics (field-driven)

First real answers worked but were slow. The fix ladder, cheapest first:

1. **Model residency**: ollama unloads idle models (~5 min); a cold ask
   pays full load before any token. Pass `keep_alive` (configurable) so
   the watcher-tier model stays resident.
2. **Streaming** into the bar/band: perceived latency beats total.
3. **Stable-prefix prompting**: ollama prefix-caches KV against the
   previous request per model — the append-mostly epoch shape gets
   cache hits *locally today*, exactly as the caching stance predicted.
   A rebuilt-each-time sliding window defeats it. Plus a hard prompt
   budget (raw 8×1200-char tails is heavy eval for a 2B).
4. **Background compaction** (M5): the rolling watcher continuously
   shrinks old chunks; retrieval serves summary chunks as the norm,
   raw on drill-down.
5. **Adaptive depth**: every ask/answer is timestamped in the
   transcript, so measured answer latency can tune the context budget —
   deep raw context when fast, lean on summaries when sluggish.
   Depth/correctness vs. latency becomes a closed loop, not a constant.

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
