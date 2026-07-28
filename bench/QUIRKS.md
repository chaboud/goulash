# QUIRKS — do models need per-model adapters?

From Pass A (192 probes, 26 cells, 2026-07-28). Each probe isolates one
setting against a fixed tiny context; `bench/src/probe.rs` defines them.

**Answer: yes, but along two axes only — reasoning suppression and
endpoint shape.** Everything else was uniform. No model needed
per-model prompt wording, and none of the parser's suspected failure
modes materialised.

---

## 1. `think: false` is load-bearing for half the catalog

Omitting the field empties the answer on **12 of 24** capped cells: every
`qwen3.5` size, every `gemma4` variant, `qwen3:14b`, and the LM Studio
`qwen/qwen3-8b` and `google/gemma-4-e4b`.

It is inert (no reasoning to suppress) on `llama3.2:3b`, `gemma3:4b`,
`gemma3:12b`, `mistral`, `llama3.1:8b`, `qwen/qwen3-1.7b`,
`qwen/qwen3-4b`, `google/gemma-4-12b-qat`.

### The stop sequence is *not* the cause — corrected

An early reading of this data was wrong and is worth recording. With
`think` omitted, `qwen3.5:0.8b` returns `eval_tokens=4,
stop_reason=stop`, which looks exactly like the stop sequence
guillotining a reply that opened with a blank line. It isn't. The
`no-think-no-stop` probe removes `stop` as well:

| probe | eval_tokens | stop_reason | output |
|---|---|---|---|
| `no-think-field` (stop on) | 4 | `stop` | empty |
| `no-think-no-stop` | 256 | `length` | empty |

Without the stop sequence it spends the **entire budget** and still emits
nothing. One mechanism — reasoning spend — with two failure signatures.
Removing or widening `stop` would not have helped; only reasoning control
does. Same result for all 13 affected cells.

**`stop: ["\n\n"]` was the top pre-registered suspect going in, and it is
exonerated.** It never truncated a non-reasoning model to empty.

## 2. `think: false` does not work on `deepseek-r1:14b`

Ollama accepts the field and the model reasons anyway, leaking the raw
tag into the answer:

```
baseline (think:false) → "<think>\nOkay, the user is asking how to list files by size…"
```

`split_answer` then takes `<think>` as the prose line, so **the user's
status bar shows `<think>`**. This is a visible product bug, not just a
benchmark artifact.

Inverted fix: this is the one model that does **better** with `think`
omitted *and* `stop` removed — that combination produced a real answer
(`"To list files by size from largest to smallest: CMD: ls -lrta…"`).

## 3. LM Studio: neither endpoint is clean

Both `/v1/completions` and `/v1/chat/completions` fail, differently.

**Raw completions → prompt echo and format bleed.** The model continues
the text instead of answering:

| model | raw output |
|---|---|
| `google/gemma-4-e4b` | `"how do I list files by size, largest first"` (echoes the question) |
| `qwen/qwen3-1.7b` | `"CMD: ls -S\nAnswer: CMD: ls -S \| sort -nr"` (re-emits the `Answer:` scaffold) |

**Chat template → reasoning-only empties**, now quantified via
`usage.completion_tokens_details.reasoning_tokens`:

| model | eval_tokens | reasoning_tokens | content |
|---|---|---|---|
| `qwen/qwen3-8b` | 255 | **254** | empty |
| `google/gemma-4-e4b` | 7 | 4 | empty |

254 of 255 tokens spent thinking. And per
`results/step0/LMSTUDIO.md`, the available lever
(`chat_template_kwargs: {enable_thinking: false}`) makes this *worse*, not
better — it empties `content` outright.

`google/gemma-4-12b-qat` works fine on raw completions, so this is
model-specific rather than a blanket endpoint failure.

## 4. `mistral-nemo` answers questions with memory writes

Five of seven probes returned nothing but a memory-tool line:

```
REMEMBER: [3] prefers ls -Sh
```

The pinned-memory block in the prompt hijacks it. Goulash does **not**
treat this as an error — `engine.rs` checks `remembers` before declaring
failure — so the user asks a question, sees nothing, and silently gains a
memory slot. Arguably worse than an error.

Only `no-command-needed` (a pure-explanation question) got a real answer,
and even that came back prefixed `goulash: `, mimicking the transcript
format from the session log.

## 5. Non-findings worth recording

- **Fencing: zero.** Not one of 192 probes wrapped a command in
  ```` ``` ````. The parser's inability to strip a fenced block is real
  but never fires. Deprioritise.
- **`max_tokens: 32`** was honoured by every model that works at all; no
  cell mangled its output under a tight budget.
- **No model needed different prompt wording.** The shipped
  preamble/directive worked everywhere it worked at all.

## 6. Over-vending commands

Asked `"what does the -P flag do in grep"` — a pure explanation question
with a `CMD:` line explicitly optional — **10 of 26 cells attached a
command anyway**, including every `gemma4` variant, `gemma3:12b`,
`devstral:24b` and `mistral-nemo`. Correctness of the prose is left to
blind grading; the over-vending itself is mechanical and counted here.

## 7. Cells excluded, and why their data is untrustworthy

`gpt-oss:20b`, `deepseek-coder:33b-q3_K_S`, `gemma3:27b`, `devstral:24b`
ran in the first uncapped pass, **while the machine was memory-saturated**
(6% free, actively swapping). Their results are contaminated and are not
reported as model behaviour:

- `deepseek-coder:33b` — `stop_reason=None`, `eval_tokens=None`: the
  stream never delivered a terminal chunk.
- `gemma3:27b` — `"UseUse"`, garbled.
- `gpt-oss:20b` — 55 tokens generated, empty `response`. Plausibly the
  harmony-format channel split rather than memory pressure, but not
  separable from the contamination here.

Rerun under `GOULASH_BENCH_MAX_GB=24` on an idle machine before drawing
any conclusion about these four.

---

## Verdict

A per-model *quirks table* is **not** needed. Two provider-level
capabilities are:

| lever | who needs it |
|---|---|
| reasoning suppression that actually works | qwen3.5 · gemma4 · qwen3 families (12 cells) |
| a working reasoning control on OpenAI-compat | all LM Studio reasoning models — none exists today |
| endpoint choice (chat vs raw) per model | `gemma-4-e4b` echoes on raw; `12b-qat` is fine |

Plus two goulash-side bugs the sweep surfaced, both independent of model
choice:

1. `<think>` reaches the status bar when suppression fails
   (`split_answer` has no notion of a reasoning tag).
2. A memory-only reply renders as silence plus an unannounced memory
   write.
