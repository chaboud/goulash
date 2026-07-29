# Residency: KV cost, and why goulash should share a load

Measured 2026-07-29 on Apple M4 / 24 GB, ollama 0.32.1 + LM Studio.

## How big is a 118k KV cache?

Measured directly — `ollama ps` reports resident bytes, which includes
the KV cache, so loading the same model at different `num_ctx` isolates
it. On `qwen3.5:0.8b` (1.0 GB on disk):

| `num_ctx` | resident | KV over the 2k baseline |
|---|---|---|
| 2,048 | 1.08 GB | — |
| 8,192 | 1.21 GB | +0.13 GB |
| 32,768 | 1.57 GB | +0.48 GB |
| 131,072 | 3.33 GB | **+2.25 GB** |

Roughly **17.5 MB per 1k tokens**, near-linear. At full context the KV
cache is **more than double the model's own weights** on a 0.8B model.

For `google/gemma-4-e4b` at LM Studio's default 118,272: total resident
was 7.9 GB against a 1.4 GB idle baseline. I did not isolate weights from
KV for that one, and gemma uses sliding-window attention (most layers do
not hold full-context KV), so do **not** extrapolate the 0.8B rate to it
— an early analytic estimate of 33.9 GB was simply wrong, and the model
obviously fit in 24 GB.

The transferable point: **context length is a first-class memory cost,
frequently exceeding the weights.** Running at 118k when 8k is needed is
pure waste, and it is what triggered the memory-pressure alert.

## `num_ctx` is part of a model's load identity

This is the finding that matters for sharing. Same model, varying only
the requested context:

| step | `load_duration` | what happened |
|---|---|---|
| cold load at ctx=8192 | 1573 ms | loads |
| **same** ctx=8192 again | **206 ms** | reused |
| ctx=**16384** | **1847 ms** | **full reload** |
| back to ctx=8192 | **1875 ms** | **full reload again** |

Changing `num_ctx` **evicts and reloads**. A resident model is only
reusable by a request that asks for the same context.

### Why that is a problem for an overlay

goulash is a background citizen on a machine the user is also using. It
currently sends `num_ctx: 8192` unconditionally and picks the *smallest
installed* model, with no reference to what is already loaded. So:

- If the user has `gemma4:12b` resident at 32k and goulash asks for
  `qwen3.5:0.8b` at 8k, ollama now holds **two** models — or evicts the
  user's.
- If the user's model is the same one goulash picks but at a different
  context, **every goulash ask evicts and reloads it**, and the user's
  next request reloads it back. Two processes thrashing a multi-GB model
  between them, each paying seconds.

Observed cold-load cost across 17 models: **median 4281 ms, p90
7214 ms**, up to 7.7 s for `gemma3:12b`. Warm TTFT median is 2314 ms —
so a needless reload costs about **2x an entire warm answer**, and for
the big models far more.

## What sharing would look like

Both engines already expose what is resident: ollama's `/api/ps` (name,
size, expiry) and `lms ps`. Neither is consulted today.

A residency-aware `pick_model` would prefer, in order:

1. an explicitly configured model (unchanged — the user's pin wins)
2. **a model already resident**, adopting its context length
3. the first installed favorite
4. smallest installed (today's fallback)

Cost of adopting a resident model: it may be larger or slower than the
watcher-tier default goulash would otherwise choose. Benefit: zero load
latency, zero additional memory, and no eviction of the user's working
model. Given a median 4.3 s load, adopting a warm 12B is very plausibly
faster end-to-end than cold-loading a 0.8B — and it is the difference
between being a good citizen and a hostile one.

The same logic argues for **not** pinning `num_ctx` when a resident model
already has a workable one: goulash's 8192 is a memory bound, not a
correctness requirement, and insisting on it is what forces the reload.

### On LM Studio specifically

`num_ctx` never reaches LM Studio at all — it is an ollama option with no
OpenAI-request equivalent, so each model loads at whatever its saved
config says (`qwen3-1.7b`: 8192; `google/gemma-4-e4b`: **118272**). The
bench now preloads with an explicit `--context-length`; the product has
no equivalent lever short of the `lms` CLI or LM Studio's native REST
API.

## Status

Findings only — no product change made. `pick_model`
(`src/engine/mod.rs`) still selects smallest-installed and `num_ctx` is
still sent unconditionally. The residency-aware selection above is a
proposal with measurements attached, not something this project
implemented.
