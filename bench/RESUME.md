# Where this stands — paused 2026-07-28 19:57

Branch `engine-characterization`, 9 commits, `cargo test` green (43 + 8).

## Resume

```sh
./bench/run.sh bench/results/2026-07-28
```

Idempotent. Completed cells are skipped; safe to interrupt with Ctrl-C or
`pkill -TERM -f bench/run.sh` (trap unloads both engines in ~1s).

State at pause: 178 rows, part-way through Pass A (288 probes), then
Pass B (24 cells x 3 shapes x 19 asks = 1368). Nothing resident, 76% free.

## Findings so far

**Settled — `bench/QUIRKS.md`** (from a complete 192-probe Pass A on the
previous run; single-turn probes, unaffected by the S2 fix):

- `think: false` is load-bearing for 12 of 24 cells (all qwen3.5, all
  gemma4, qwen3:14b); inert on llama/mistral/gemma3.
- **`stop: ["\n\n"]` is exonerated** — it was the top suspect and never
  truncated a non-reasoning model to empty. The `no-think-no-stop` probe
  proved the single mechanism is reasoning spend: remove the stop
  sequence and qwen3.5 burns all 256 tokens and still emits nothing.
- `think: false` does **not** work on `deepseek-r1:14b` — it reasons
  anyway and `<think>` reaches the status bar. Product bug.
- Neither LM Studio endpoint is clean: raw completions echo the prompt,
  chat gives reasoning-only empties (qwen3-8b: 254 of 255 tokens).
- `mistral-nemo` answers questions with bare `REMEMBER:` lines; goulash
  renders that as silence plus a silent memory write.
- Zero fencing in 192 probes — deprioritise fence handling.

**Strong, from the superseded run** (S1-vs-S3 is unaffected by the memory
bug; neither arm mutates memory):

- **`CMD:` first raises command vend rate 56% -> 81%** across 266 paired
  cells, at +19ms. Biggest lever found so far.
- Prefix cache works: prompt-eval *falls* as the log grows
  (`gemma3:12b` S1: 6925ms -> 2739ms while the prompt grows
  1624 -> 6440 chars).

**Open:**

- S2 (memory-position) has no valid data yet — the arm was fixed this
  session and has not been run.
- Long-command headroom: 5 probes queued (ffmpeg/jq/find at 256 vs 1024
  tokens), never run. Answers "what should `max_tokens` be".
- Blind grading has not started; it needs Pass B.
- Four heavyweight cells (`gpt-oss:20b`, `deepseek-coder:33b`,
  `gemma3:27b`, `devstral:24b`) have only contaminated data, collected
  while the machine was saturated. Rerun with
  `GOULASH_BENCH_MAX_GB=24` on an idle machine.

## Product changes made (all behaviour-preserving, tests green)

`src/lib.rs` (new), `src/engine/{mod,prompt,provider}.rs` (split from
`engine.rs`). Provider trait with ollama + OpenAI-compat impls;
`build_prompt` parameterized by `PromptShape`; generation settings moved
from literals into `GenRequest`; `keep_alive` now maps to LM Studio's
`ttl` (it was silently ignored there before).

No new `config.toml` knobs — deliberately. Which levers deserve a
user-facing setting is what the measurement is for.

## Two things to distrust

Both are corrections to my own earlier claims, kept visible on purpose:

1. I reported "two distinct empty-answer mechanisms". Wrong — one
   mechanism, two signatures. See QUIRKS section 1.
2. The S2 arm measured nothing for a full run because memories were a
   constant, so there was no invalidation to price. Fixed, with a test
   asserting S1 diverges before the session log while S2 does not.

`bench/results/2026-07-28-static-memories/SUPERSEDED.md` records exactly
what survives from that run and what does not.
