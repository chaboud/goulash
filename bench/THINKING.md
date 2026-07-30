# Thinking: what it costs, and what actually blocks it

Pass T, 480 generations, 2026-07-29. All rows below are the **16
thinking-capable cells only**, so every arm is scored on the same
population — the other 8 cells return HTTP 400 and cannot participate.

| arm | reasoning | stop | budget | answered | hit cap | med tokens | **med prose** |
|---|---|---|---|---|---|---|---|
| T0 | off | `\n\n` | 256 | **29/36** | 6 | 48 | 41 ch |
| T2 | on | `\n\n` | 256+1024 | 11/36 | 6 | 39 | 61 ch |
| T3 | on | `\n\n` | 256+4096 | 11/36 | 3 | 41 | 40 ch |
| **T7** | on | **none** | 256+1024 | **19/32** | 19 | **1279** | 70 ch |
| **T8** | on | **none** | 256+4096 | **22/32** | 15 | **3574** | 77 ch |
| **T9** | off | **none** | 256 | **30/32** | 10 | 50 | 46 ch |

## 1. The stop sequence, not the budget, was strangling reasoning

Compare T2 and T7 — identical except for `stop`:

- **T2** (stop on): 11/36 answered, median **39** tokens generated.
- **T7** (stop off): 19/32 answered, median **1279** tokens generated.

With `stop: ["\n\n"]` in place, a reasoning model emits ~39 tokens and
halts. The blank line that separates or precedes its thinking trips the
stop sequence, so the allowance is never touched no matter how large it
is — which is why T2 and T3 are identical at 11/36 despite a 4x
difference in budget.

**The first Pass T run measured this and I misread it as a budget
result.** All its arms held the stop sequence; the budget never got a
chance to matter.

## 2. Given room, reasoning models do use it — and want more than 1024

Once `stop` is gone, the budget starts doing real work:

- T7 (+1024): median **1279** tokens, **19/32** answered, but **19 of 32
  still hit the cap**.
- T8 (+4096): median **3574** tokens, **22/32** answered, **15 still hit
  the cap**.

So 1024 is plainly too small, 4096 is better and still not enough for
half the cells. A reasoning allowance that actually finishes the job
looks like **4096 minimum**, and even then a meaningful fraction gets
truncated mid-thought.

## 3. Thinking still costs reliability

Same cells, reasoning off vs on, both without the stop sequence:

- **T9** (off): **30/32** answered — 94%.
- **T8** (on, +4096): **22/32** — 69%.

Reasoning is a **25-point reliability drop** even when given four
thousand tokens of room. For a status-bar assistant whose contract is one
short line, that is a bad trade at the watcher tier. `off` remains the
right default; `auto` should mean "on only where it is known to help",
not "on wherever supported".

## 4. The visible answer stays short no matter what

Median visible prose across every arm: **40-77 characters**. T8 had 4096
tokens available and produced 77 characters.

This is the point that matters for the display budget: **the cap is not
what keeps answers short — the prompt is.** A one-line contract in the
directive, plus a band that clamps at render
(`session.rs` `wrap_chars`), already bound what the user sees. So the
token budget can be raised for reasoning without any risk of the band
filling up.

## 5. Dropping the stop sequence helps non-reasoning models too

T0 vs T9 — reasoning off in both, differing only in `stop`:

- T0 (stop on): 29/36 = 81%
- T9 (stop off): 30/32 = 94%

This refines the Pass A verdict a third time. The honest statement is
now: `stop: ["\n\n"]` **never truncated a working non-reasoning model to
empty** — that finding stands — but it is not free either. It costs
roughly ten points of answer rate even without reasoning, and it is
categorically fatal with reasoning on.

The sequence exists to enforce the one-line contract. Points 4 and 5
together suggest it is not earning that: the prompt and the band already
enforce brevity, and the stop sequence is buying nothing while costing
answers.

## Recommendations

| setting | recommendation | evidence |
|---|---|---|
| `stop` | **drop `["\n\n"]`** | +13 pts answer rate with reasoning off; the difference between 39 and 1279 tokens with it on; brevity already enforced by prompt + band |
| `thinking` | `off` default, `auto` capability-gated, `forced` for debug | 8 of 24 cells hard-400; reasoning costs 25 pts reliability even at 4096 |
| reasoning allowance | **>= 4096** when thinking is on | 19/32 still truncated at 1024; 15/32 at 4096 |
| `max_tokens` (display) | can be raised freely | median visible prose 40-77 ch regardless of budget |

## Caveat

`answered` here means non-empty after `split_answer`. Whether those
answers are *good* is a grading question, not one this table settles —
reasoning could plausibly buy quality where it costs reliability, and
blind grading is what would show it.
