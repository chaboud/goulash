# LM Studio capability probe

Run 2026-07-28. LM Studio server on `:1234`, `qwen/qwen3-1.7b`.

Every line here lands in `Caps` (`src/engine/provider.rs`). Guessing any
of them means blaming a model for a provider limitation.

| capability | verdict | evidence |
|---|---|---|
| `/v1/completions` (raw prompt) | **yes** | 18.5 s first call (JIT load), fast after |
| `/v1/chat/completions` | **yes** | 393 ms warm |
| SSE streaming | **yes** | multi-chunk, client-side TTFT usable |
| `stream_options.include_usage` | **yes** | usage arrives in a final chunk |
| `stop` sequences | **yes** | `finish_reason=stop`, text cut at the blank line |
| prompt-eval **timing** | **no** | see below |
| reasoning control | **no** | see below — the lever backfires |
| `usage…reasoning_tokens` | **yes** | 263 of 299 completion tokens |
| context length per request | **no** | load-time only (`lms load --context-length`) |

## `stats` exists but carries no timing

The response has a top-level `stats` key, which is why a naive
key-presence check says "timing available". It is not:

- chat: `stats = {}`
- completions: `stats = {"total_draft_tokens_count": 0,
  "accepted_draft_tokens_count": 0, …}` — speculative-decode counters

So `reports_prompt_eval_time: false`. **Cache measurement on LM Studio
must use client-side TTFT-flatness**, not a server-side duration. Ollama
remains the only provider with the direct signal.

## `enable_thinking: false` is worse than not sending it

The obvious lever for qwen3 reasoning suppression is
`chat_template_kwargs: {"enable_thinking": false}`. It is accepted (no
400) — and it makes things worse:

| request | `content` | `reasoning_content` |
|---|---|---|
| no kwarg | `"\n\nThe result of $2 + 2$ is **4**. …"` | populated |
| `enable_thinking: false` | **`""`** | populated |

With the kwarg the answer lands **entirely** in `reasoning_content` and
`content` comes back empty — which is exactly the empty-answer failure
the kwarg was supposed to prevent, and exactly what
`engine.rs`'s "empty answer from {model} (thinking model? …)" reports.

**Consequence:** `OpenAiCompat.suppress_reasoning` defaults **off**.
Sending it by default would have broken every qwen3 model on LM Studio
out of the box. Reasoning spend stays observable after the fact through
`GenStats.reasoning_tokens`.

This is the first confirmed entry for `QUIRKS.md`, and it is a *provider*
quirk rather than a model one — the same weights behave differently
through ollama's `think: false`.

## Incidental quality signal

Asked "how do I list files by size", qwen3-1.7b returned a rambling
multi-sentence answer recommending Windows PowerShell twice. Not scored
here — noted because the one-line contract is clearly not free.

## Reproduce

```sh
~/.lmstudio/bin/lms server start
python3 bench/results/step0/lmstudio_caps.py
```

> The probe script's own predicates were too loose on first write
> (key-presence for `stats`, substring for `<think>`); both were
> corrected after checking raw responses. Trust the table above.
