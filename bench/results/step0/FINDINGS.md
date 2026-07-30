# Step 0 — establishing the cache metric

Run 2026-07-28, ollama 0.32.1, `qwen3.5:0.8b`, Apple M4 / 24 GB.

Purpose: before the sweep reports any cache number, find out which
server-side signal actually reveals a KV prefix-cache hit — and whether
the stable-prefix design is worth optimizing at all.

## 1. `prompt_eval_count` cannot see the cache

Two sequential calls sharing a ~6.6 KB prefix:

| call | prompt_eval_count | prompt_eval_ms | load_ms |
|---|---|---|---|
| 1 (establish) | 3947 | 2137.4 | 2071.0 |
| 2 (shared prefix) | **3947** | **592.4** | 187.4 |

The count is **identical** — ollama reports *total* prompt tokens, not
evaluated ones. Anything keying on a count drop measures nothing.

**The duration is the signal.**

## 2. Prefix cache confirmed, isolated from model warmth

The drop above is confounded: call 1 also paid a 2.1 s model load. The
control is a warm call with a *different* prefix of the same length.
All four calls below are warm, same token count:

| call | prefix | prompt_eval_ms |
|---|---|---|
| A | P1 (establish) | 644.2 |
| B | P1 (**shared**) | **619.4** |
| C | P2 (**different**, same length) | **2118.9** |
| D | P1 again, after C | 599.9 |

**B vs C — 619 ms vs 2119 ms — is prefix reuse, not warmth: 71% less
prompt-eval time at an identical token count.**

D also hit after C displaced it, so **ollama caches more than one
prefix**. Interleaving models in a sweep is less destructive than
assumed, though model-major ordering is still kept for memory reasons.

## 3. How the saving scales (medians of 3, distinct cold prefixes)

| blocks | tokens | cold_ms | warm_ms | saved |
|---|---|---|---|---|
| 10 | 543 | 294.4 | 25.6 | 91% |
| 40 | 2196 | 1148.8 | 564.3 | 51% |
| 120 | 6857 | 4138.3 | 699.2 | 83% |
| 240 | 14154 | 10569.2 | 944.5 | 91% |

Cold cost is linear in tokens (~0.55-0.75 ms/1000 tok). **Warm cost is
strongly sublinear**: 2196 → 14154 tokens is 6.4x the input for 1.7x the
time, a marginal ~32 µs/token against ~750 µs/token cold — roughly 4%.

> The probe script's automatic verdict ("floor SCALES") is **wrong**. It
> compares only the first and last row, where the endpoints happen to be
> the fully-cached 543-token case and the anomalous 2196-token case. Read
> the per-row `saved` column instead. Verdict logic since corrected.

### Conclusion

At realistic session sizes a cache miss costs **~10x** a hit (10.6 s vs
0.9 s at 14k tokens). Keeping the prefix byte-stable is a **large** lever,
not a rounding error — which raises the stakes on the two known
invalidation sources:

- `memory.rs:148` — the memory block leads with a live `(N/25 slots)`
  count and sits *before* the session log, so any REMEMBER/FORGET
  rewrites the prefix. This is what `MemPos::Suffix` (S2) prices.
- `session.rs:1119` — the epoch trim drains from the *front* of
  `ctx_log`, shifting every downstream token position and forcing a full
  re-eval.

## 4. Open anomaly for the sweep

The 2196-token row shows only 51% savings, and the warm figure is
**reproducible across two independent runs** (562.8 ms, 564.3 ms) — not
noise. Something near ~2k tokens defeats part of the reuse; batch-size or
context-window boundary is the obvious suspect. Left for per-model
characterization in Pass B rather than chased here.

## Metric decision

| metric | provider | role |
|---|---|---|
| `prompt_eval_duration` at fixed token count | ollama | **primary** |
| TTFT held flat as the log grows | any, client-side | **portable fallback** |
| `prompt_eval_count` | ollama | unusable — reports total |
| `usage.prompt_tokens` | OpenAI-compat | unusable — reports total |

LM Studio exposes no prompt-eval timing over the OpenAI API, so
TTFT-flatness carries the cache measurement there. Recorded in `Caps` as
`reports_prompt_eval_time`.

## Reproduce

```sh
python3 bench/results/step0/floor_probe.py     # writes floor_result.txt
```
