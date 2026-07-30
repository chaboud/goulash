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

---

## Unloading a model does not release its GPU allocation

Found while chasing three memory-pressure alerts during a long run.

At the point of the third alert:

| source | value |
|---|---|
| `Pages wired down` | **20.44 GB** of 24 |
| GPU driver `Alloc system memory` | **20.4 GB** |
| GPU driver `In use system memory` | **18.3 GB** |
| sum of *all* process RSS | **6.96 GB** |
| models actually resident | one, 4.6 GB |

`keep_alive: 0` and `lms unload --all` both do exactly what they claim —
the model leaves, `/api/ps` goes empty — but the GPU allocation behind it
is not returned. Wired memory is kernel-locked and non-pageable, so it
cannot be swapped out either; the machine simply loses it. Across
hundreds of load/unload cycles it accrued until only ~1.5 GB was free and
swap had climbed to 4.5 GB.

**Restarting both servers reclaims it:**

| | before | after |
|---|---|---|
| wired | 20.0 GB | **3.6 GB** |
| free | 8% | **82%** |
| GPU alloc | 20.4 GB | **2.5 GB** |
| GPU in use | 18.3 GB | **0.8 GB** |

Neither engine exposes a lighter lever. The bench now restarts both
between passes.

### Why this matters beyond the bench

Any long-lived process that cycles models hits this — and goulash cycles
models by design: `#/model` switching, the crash fuse rebinding, the
probe chain re-binding after a server restart. A goulash session left
open for a day, with a few model switches, will leak wired memory the
user cannot see in Activity Monitor's process list and cannot reclaim
without restarting ollama.

It also sharpens the residency argument above: **adopting an
already-resident model avoids the load/unload cycle entirely**, which is
the only way to not accumulate this at all.

### Caveat on late Pass B timings

Cells that ran while wired memory was high were competing with swap.
Latency for the last stretch of Pass B may be inflated; the run
completed at 1368/1368 but its tail should be checked for upward drift
before any cross-cell latency comparison is trusted.

### Does the accumulated pressure invalidate the timings? No.

Fixed prompt, `qwen3.5:0.8b`, 7 reps per condition, nothing else on the
GPU (`bench/results/step0/pressure_probe.py`):

| condition | free | wired | prompt_eval | eval |
|---|---|---|---|---|
| clean | 77% | 5.4 GB | 563.0 ms | 199.5 ms |
| **pressured** (12B loaded alongside) | 42% | 14.0 GB | **575.9 ms** | 198.0 ms |
| clean again | 77% | 5.2 GB | 575.0 ms | 198.5 ms |

**+2% prompt-eval, -1% eval.** The two clean measurements bracket the
pressured one, so this is reproducible rather than lucky. Memory pressure
does not materially affect inference latency as long as the working model
still fits — which matches the mechanism: wired pages cost *capacity*,
not speed, until the active set has to swap.

Corroborating: within-model latency drift across Pass B was a median
**+1%**, and the two latest-running ollama cells showed +0% and -23%.

**Limitation, stated rather than buried:** the probe reached 42% free /
14 GB wired. The worst state observed during Pass B was ~5% free / 20 GB
wired, which this did not reproduce. So the result rules out an effect at
moderate pressure, not at the extreme. Given two independent measures
agreeing at ~+1-2%, the Pass B latency numbers are treated as sound; a
re-run of the six cells that ran past the 1.5h mark remains a cheap
option (~340 generations, ~40 min) if that assumption is ever load-bearing
for a decision.

### What actually dominates latency: contention, not memory

The first attempt at this probe reported "clean" at 1326 ms against
"pressured" at 660 ms — a nonsense ordering caused by a concurrent
validation sweep holding the GPU during the first measurement. Wired
memory across the three samples was 14.2, 22.2 and 5.4 GB with no
monotonic relationship to latency at all.

**Concurrent GPU work moves latency by ~2x; memory pressure moves it by
~2%.** The probe now refuses to start when any bench process is running,
because relying on remembering to check has failed three times in this
project (two concurrent sweeps, one mid-flight unload, one confounded
probe).
