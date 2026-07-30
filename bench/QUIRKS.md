# QUIRKS — do models need per-model adapters?

Complete Pass A: **288 probes, 24 cells** (17 ollama + 7 LM Studio),
2026-07-29. Each probe isolates one setting against a fixed tiny context;
`bench/src/probe.rs` defines them.

**Answer: no per-model table is needed. Two provider-level capabilities
are, and one of them was invisible until both endpoints were measured.**

---

## 1. The chat template is what turns reasoning on

The headline result, and it only appears when the same model is run
through both LM Studio endpoints:

| model | endpoint | answered | reasoning tokens |
|---|---|---|---|
| `qwen/qwen3-8b` | `/v1/completions` (raw) | **12/12** | **0** |
| `qwen/qwen3-8b` | `/v1/chat/completions` | **1/12** | **1450** |
| `google/gemma-4-e4b` | `/v1/completions` (raw) | **8/12** | **0** |
| `google/gemma-4-e4b` | `/v1/chat/completions` | **0/12** | 546 |

Same model, same weights, same server, same prompt text. Through raw
completions the model does **no reasoning at all** and answers normally.
Through the chat endpoint it spends everything on reasoning and returns
empty `content` — 254 of 255 tokens on the baseline probe.

The model's chat template enables thinking; the raw endpoint bypasses the
template entirely. That single fact explains the whole LM Studio
empty-answer story, and it reframes the earlier
`enable_thinking: false` finding (`results/step0/LMSTUDIO.md`): that
kwarg is a *chat-template variable*, which is why it can only make things
worse — it perturbs the template rather than disabling reasoning.

**Consequence:** goulash should prefer `/v1/completions` for local
OpenAI-compatible servers. It is also the better cache shape (the raw
endpoint takes our stable-prefix string verbatim, the way ollama's
`/api/generate` does). Hosted providers offer chat only, so a future
Anthropic/OpenAI adapter inherits this problem and will need
provider-specific reasoning control.

### Correction (2026-07-30): chat was starved, not broken

The table above was measured at a 256-token ceiling, and it conflates
two effects. The chat template really does turn reasoning on — that part
holds. But the **empty answers were a budgeting bug on our side**, not a
property of the endpoint.

`GenRequest::wire_max_tokens` added the reasoning allowance only when
`think != Off`. That asks "did we ask for reasoning" when the question
that matters is "can reasoning happen" — and on the chat path the
template answers yes no matter what we sent. So chat requests went out
with a 256-token ceiling, reasoning spent 253 of them, and `content`
came back empty with `finish=length`. Re-measured at the shipped 0.4.0
budget, through the same endpoint and the same weights:

| model | endpoint | before | after |
|---|---|---|---|
| `qwen/qwen3-8b` | chat | `eval=255 reason=253 length` → empty | `eval=1367 reason=896 stop` → answers |
| `google/gemma-4-e4b` | chat | `eval=255 reason=253 length` → empty | `eval=1271 reason=659 stop` → answers |

Reasoning wanted 659–896 tokens. It was never going to fit in 256.

This weakens the "prefer raw" recommendation to a preference rather than
a necessity: raw is still the better cache shape and still cheaper (no
reasoning tokens at all), but chat is now *usable* rather than fatal,
which matters because hosted providers offer nothing else.

It also generalises past this endpoint. `deepseek-r1:14b` accepts
ollama's `think:false` and reasons anyway (§2), so "off" is a request on
every provider we speak to and a guarantee on none. The allowance is now
applied unconditionally.

Raw completions are not free: `google/gemma-4-e4b` sometimes *continues*
the prompt instead of answering (`"how do I list files by size, largest
first\nAnswer:"`), which is 4 of its 12 raw failures. `qwen3-8b`,
`qwen3-4b`, `qwen3-1.7b` and `gemma-4-12b-qat` are clean on raw.

## 2. `think: false` is load-bearing on ollama, for half the catalog

Omitting the field empties the answer on 10 of 17 ollama cells: every
`qwen3.5` size, `gemma4:e2b`/`:12b`/`:e4b`, `qwen3:14b`, `deepseek-r1`.

Inert on `llama3.2:3b`, `gemma3:4b`, `gemma3:12b`, `mistral`,
`llama3.1:8b` — nothing to suppress.

### The stop sequence is exonerated

`stop: ["\n\n"]` was the top pre-registered suspect and it is not the
cause. With `think` omitted, `qwen3.5:0.8b` returns `eval_tokens=4,
stop_reason=stop` — which looks exactly like the stop sequence cutting
off a reply that opened with a blank line. Removing `stop` as well:

| probe | eval_tokens | stop_reason | output |
|---|---|---|---|
| `no-think-field` (stop on) | 4 | `stop` | empty |
| `no-think-no-stop` | 256 | `length` | empty |

It spends the entire budget and still emits nothing. One mechanism —
reasoning spend — with two signatures. Widening or dropping `stop` would
not have helped.

## 3. `mistral-nemo` answers questions with memory writes

Five of seven probes returned nothing but `REMEMBER: [3] prefers ls -Sh`.
The pinned-memory block hijacks it. Goulash does **not** treat this as an
error (`engine.rs` checks `remembers` before declaring failure), so the
user asks a question, sees nothing, and silently gains a memory slot.

Being tested in Pass P (`V4-memguard`): whether naming `REMEMBER:` a
*tool* rather than leaving it to look like an instruction fixes it.

## 4. Refusal on the smallest model

`qwen3.5:0.8b` on a plain ffmpeg-syntax question: *"I cannot convert
video files to web-friendly formats like H264 MP4 directly in this
terminal environment; I am an AI assistant and not capable of…"*

It appears to think it is being asked to *perform* the conversion. Pass P
`V1-not-executing` tests whether saying "you write command text, you
never run anything" fixes it — or whether this is a capability floor and
the model simply should not be the watcher default.

## 5. Non-findings worth recording

- **Fencing: zero** across all 288 probes. The parser cannot strip a
  ```` ``` ```` block; it never needs to. Deprioritise.
- **`max_tokens: 32`** honoured by every cell that works at all.
- **No model needed different prompt wording to work** — the shipped
  preamble worked everywhere it worked at all. (Whether *better* wording
  helps the failures is Pass P's question, not this one's.)

## 6. Measurement honesty: ~6% flake at the empty/ok boundary

The same 17 ollama cells ran `no-think-field` in two independent runs.
**One flipped** (`gemma4:e4b-mlx`: empty → ok). At temperature 0.2, a
cell sitting near the boundary is not perfectly reproducible.

So single-probe results are directional, not definitive. The family-level
patterns (all qwen3.5, all gemma4) rest on 4-5 cells agreeing and are
solid; any single cell's verdict carries roughly a 1-in-16 flip risk.

## 7. Cells excluded

`gpt-oss:20b`, `deepseek-coder:33b-q3_K_S`, `gemma3:27b`, `devstral:24b`
(13-17 GB) and `qwen/qwen3.6-35b-a3b` (22 GB) are above the 12 GB cap and
have no trustworthy data. Their only prior results came from a run during
memory saturation. Rerun with `GOULASH_BENCH_MAX_GB=24` on an idle
machine.

---

## Verdict

No per-model quirks table. Two provider-level capabilities:

| capability | who needs it |
|---|---|
| working reasoning suppression | ollama: `think:false` (works). OpenAI-compat: **use `/v1/completions`** — the chat template is what enables reasoning, and no request-level kwarg reliably disables it. |
| endpoint choice per model | `gemma-4-e4b` echoes the prompt on raw; the qwen3 family and `gemma-4-12b-qat` are clean. |

Plus two goulash-side bugs, independent of model choice:

1. `<think>` reaches the status bar when suppression fails —
   `split_answer` has no notion of a reasoning tag.
2. A memory-only reply renders as silence plus an unannounced memory
   write.
