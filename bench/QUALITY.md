# Quality — blind-graded

> **Status, 2026-07-31: every graded number below is from the 2026-07-28
> corpus and describes the PRE-MERGE engine.** It was measured with the
> `["\n\n"]` stop sequence still on the answer path, without
> capability-gated thinking, and with the context negotiation that
> reloaded the model on every ask. All three are fixed in 0.4.0, and all
> three plausibly move quality numbers.
>
> A re-run against the merged engine is **in flight** (Pass B,
> `results/2026-07-31/`). It is mechanical-only so far: **nothing in the
> new run has been graded**, because grading is a separate blind pass
> over a finished corpus. §0 records what the new run already shows
> without a grader; everything from §1 down is the old corpus, kept
> because it is a real measurement of a real corpus and its method still
> stands — not because it describes what ships today.

**313 of 313 sampled answers graded**, blind (model, provider and shape
hidden), then joined back to provenance. Eight questions across 24 model
cells and both engines.

Scale: `correct` 0=wrong/harmful … 3=fully does the ask; `idiom` 0=bizarre
… 3=how a practitioner would write it; `fit` 0=unusable in a status bar
… 3=crisp one-liner.

---

## 0. What the 0.4.0 re-run shows so far (mechanical, ungraded)

Partial: **~3,700 of 14,280 generations**, 8 of 30 cells complete. Rates
below are compliance, not correctness — a model can hit every one of
these and still be wrong, which is exactly why the graded pass exists.

| model | p50 ttft | p50 total | answered | `CMD:` | 1-line |
|---|---|---|---|---|---|
| `qwen3.5:0.8b` | 1.2 s | 2.1 s | 89% | 57% | 93% |
| `llama3.2:3b` | 1.9 s | 3.2 s | **64%** | 53% | 94% |
| `gemma3:4b` | 2.2 s | 4.9 s | 100% | 96% | 85% |
| `qwen3.5:2b` | 2.7 s | 4.7 s | 100% | 96% | 100% |
| `llama3.1:8b` | 2.6 s | 4.9 s | 99% | 97% | 98% |
| `qwen3.5:4b` | 7.7 s | 10.9 s | 100% | 93% | 93% |
| `qwen3.5:9b` | 10.3 s | 14.0 s | 100% | 95% | 100% |

Three things are already clear enough to act on:

- **`llama3.2:3b` answers 64% of the time.** The other 35% is
  `mem-only` — it replies with a bare `REMEMBER:` line instead of an
  answer. That is the `mistral-nemo` failure mode from
  [QUIRKS](QUIRKS.md) §6 appearing on a second model, so it is a
  *prompt* problem, not one model's quirk. It should not be a default.
- **Cost does not buy *compliance*.** `qwen3.5:4b` is 3.5× slower than
  `gemma3:4b` for the same 100% answered and a slightly *worse* command
  rate. `qwen3.5:9b` is 4.7× slower for no gain at all.

  **But read §3 before concluding anything from that.** On the graded
  corpus `gemma3:4b` scores **0.46 correct and 0.00 idiom — third from
  bottom**, while `qwen3.5:4b` scores 1.46. So `gemma3:4b` emits a
  well-formed command on essentially every question and the command is
  usually *wrong*. It is the single sharpest illustration in this file
  of why compliance is not quality: the model that looks best in §0
  looks nearly worst in §3, and §3 is the one that measures whether the
  answer helps. Any "which model should we ship" call has to wait for
  the graded pass on the new corpus.
- **Empty answers have collapsed.** 4–7% per shape, against 18–28% in
  the equivalent early cells of the pre-merge run. Dropping the stop
  sequence and gating thinking on capability is why — see
  [QUIRKS](QUIRKS.md) §1 and §2.

Shape comparison, the mechanical half of §1 below:

| shape | p50 total | empty | `CMD:` |
|---|---|---|---|
| S1 (shipped) | 5.17 s | 4% | 72% |
| S3 (`CMD:` first) | 5.03 s | 5% | **75%** |
| S7 | 4.70 s | 5% | 74% |

Command-first still shows a small vend-rate edge and no latency or
compliance penalty, which is consistent with §1's graded conclusion of
no quality difference. **It is not itself evidence about quality** —
`CMD:` rate counts whether a command was emitted, never whether it was
the right one.

**Do not cite §0 as a quality result.** It is compliance and latency on
a partial corpus. The moment the sweep finishes, the blind pass has to
run before anything here becomes a claim about how good the answers are.

## 1. Command-first costs nothing — and my earlier number was overread

| shape | n | correct | idiom | fit |
|---|---|---|---|---|
| S1 (prose first) | 121 | 1.40 | 1.16 | 1.59 |
| S3 (`CMD:` first) | 192 | 1.23 | 0.99 | 1.44 |

**Use the paired figure, not those means** — the sample holds more S3 rows
than S1, so the unpaired columns compare different mixes of model and
question. Paired on the same (model, question) under both shapes:

**n=112, mean S3−S1 = −0.05** — 19 better, 24 worse, **69 unchanged**.

An earlier partial pass (2 questions, n=22 pairs) reported **+0.14** and I
described S3 as marginally ahead. At 5× the pairs it is −0.05. Both are
noise around zero; the honest statement is **no detectable quality
difference in either direction**, which is what matters — the 52% → 77%
vend-rate gain is free either way.

## 2. The `data-log-status` collapse is the sharpest result here

| question | n | correct | perfect | zero |
|---|---|---|---|---|
| `no-command-needed` (explain) | 47 | 1.96 | 28 | 14 |
| `jq-extract` | 48 | 1.60 | 20 | 16 |
| `text-explain-pipefail` (explain) | 24 | 1.54 | 7 | 6 |
| `disk-size` | 48 | 1.33 | 15 | 19 |
| `git-undo` | 48 | 1.23 | 16 | 21 |
| `tree-view` | 48 | 1.00 | 7 | 21 |
| `data-csv-group` | 25 | 0.96 | 5 | 14 |
| **`data-log-status`** | **25** | **0.24** | **0** | **19** |

**Zero of 25 answers were correct.** Not one.

The cause is instructive: the question is "count how many requests
returned each status code", and the fixture had only run `wc -l
access.log` — so the model **never saw a log line**. It had to guess the
field position. Most guessed `$9`, the correct answer for standard Apache
combined format. This log's timestamp carries no timezone, so it is one
field shorter and the status sits at `$8`. `$9` is the byte count.

That is the [evidence-beats-knowledge](../wiki/architecture/machine-facts.md)
finding in its starkest form. On `why-failed`, where the actual error text
*was* in the log, **89% of models that answered named the cause correctly
— including the 1 GB one**. Here, with the evidence absent, a 14B model
guesses wrong. The difference is not model capability; it is whether the
answer was in front of it.

By contrast, `data-csv-group` (0.96) had the header *visible* via `head -5
sales.csv`, and models still frequently summed the wrong column — so
visible-but-unread is a separate failure from never-shown.

## 3. Model ranking

n=13 each, so this is now reasonably solid rather than indicative.

| model | correct | idiom | fit |
|---|---|---|---|
| `gemma4:12b-mlx` | **2.77** | 2.46 | 3.00 |
| `gemma4:12b` | 2.54 | 2.08 | 2.85 |
| `qwen3:14b` | 2.31 | 2.23 | 2.69 |
| `qwen/qwen3-8b` (lms-raw) | 2.23 | 1.62 | 1.00 |
| `gemma4:e4b` | 2.15 | 1.77 | 2.54 |
| `gemma4:e4b-mlx` | 2.15 | 1.92 | 2.46 |
| `mistral-nemo` | 2.08 | 2.08 | 2.08 |
| `gemma3:12b` | 1.85 | 1.46 | 2.77 |
| `qwen/qwen3-4b` (lms-raw) | 1.77 | 1.46 | 1.08 |
| `llama3.1:8b` | 1.69 | 1.23 | 2.38 |
| `gemma4:e2b` | 1.62 | 1.38 | 2.38 |
| `qwen3.5:9b` | 1.54 | 0.85 | 1.54 |
| `qwen3.5:4b` | 1.46 | 1.08 | 1.62 |
| `google/gemma-4-12b-qat` (lms-raw) | 1.31 | 1.08 | 0.92 |
| `mistral` | 1.23 | 0.77 | 1.77 |
| `llama3.2:3b` | 0.92 | 0.85 | 1.85 |
| `qwen3.5:2b` | 0.62 | 0.38 | 0.46 |
| `gemma3:4b` | 0.46 | **0.00** | 0.54 |
| `qwen3.5:0.8b` | 0.27 | 0.20 | 0.93 |
| `google/gemma-4-e4b` (lms-raw) | 0.17 | 0.33 | 0.50 |
| `qwen/qwen3-1.7b` (lms-raw) | 0.15 | 0.15 | 0.54 |
| `qwen/qwen3-8b` (**lms-chat**) | **0.00** | 0.00 | 0.00 |
| `google/gemma-4-e4b` (**lms-chat**) | **0.00** | 0.00 | 0.00 |
| `deepseek-r1:14b` | **0.00** | 0.00 | 0.00 |

Every zero is a **mechanical** failure already documented, not bad
reasoning:

- **`deepseek-r1:14b`** leaks a bare `<think>` tag into the band on every
  answer ([QUIRKS](QUIRKS.md) §2). It accepts `think: false` and reasons
  anyway.
- **Both `lms-chat` cells** return empty `content` with everything in
  `reasoning_content` — the chat template enables reasoning
  ([QUIRKS](QUIRKS.md) §1). The same weights via `/v1/completions` score
  2.23 and 0.17.
- **`google/gemma-4-e4b` raw** echoes the prompt back instead of answering.

Fix those three and the table changes shape. The `gemma4` 12B pair topping
it also supports the [residency](RESIDENCY.md) argument: a warm 12B is
both the most accurate *and* plausibly faster end-to-end than cold-loading
something small.

`fit` is worth reading separately — `gemma3:12b` is mid-table on
correctness (1.85) but near the top on fit (2.77), while every LM Studio
raw cell scores ≤1.08 because those models ramble to the 256-token cap.

## 4. Failure modes, counted

Across all 313:

- **Empty / `<think>` leak** — the single largest bucket of zeros.
- **Command inline in prose, nothing vends** — the answer is right but
  arrives as `` `jq -r …` `` inside a sentence. ~10 cases.
- **Explanation concatenated into the command** — a paragraph after a
  semicolon, so the vended text would run garbage. ~12 cases.
- **Backticks inside the `CMD:` line** — vended text includes the
  backticks and fails. ~6 cases.
- **Session-log format bleed** — `goulash:` prefixes, `[exit 0, 09:04:50]`
  stamps, and `REMEMBER: 1/25` fragments appearing *inside* commands.
- **Degeneration** — "and the and the and the", "or or or", runs of
  zeroes. Confined to the smallest models.
- **Invented figures** — "node_modules, which is 60.1 GB" in a sandbox of
  a few kilobytes.
- **Platform errors** — GNU-only `du --max-depth`, `ls --sort=size`,
  `PROCINFO` on a BSD box.

## 5. The dangerous class: confidently wrong

`git-undo` asked to undo a commit **keeping changes staged**. Of 48:

| outcome | n |
|---|---|
| correct (`--soft`) | 16 |
| **confidently wrong** — plain `git reset`, prose asserting it keeps changes staged | 9 |
| destructive — `git checkout --`, `reset --soft --hard` (git takes the last flag, so: hard) | 3 |
| broken / empty / invalid flags | 20 |

The middle row is the one that matters. `rm -rf` looks dangerous, so a
user hesitates. `git reset HEAD~1` with *"undoes the last commit while
keeping changes staged"* attached does not — and it unstages everything.
No vend-bias dial or destructive-pattern gate catches that; only
correctness does.

## Method and audit

- Blind: model, provider and shape stripped, answers grouped by question
  and ordered by a hash of the cell key, so nothing correlated with
  provenance was visible while scoring.
- Every grade is one line in `results/2026-07-28/grades.jsonl` with a
  rationale; `blind_sample_keys.jsonl` joins ids to provenance.
- `goulash-bench replay DIR <id>` prints the exact prompt and raw
  response behind any grade.

**Which run:** everything from §1 down is `results/2026-07-28/`, the
pre-merge engine. The 0.4.0 re-run is `results/2026-07-31/` and is
ungraded (§0).

**Coverage:** 313 of 2299 total answers — the stratified sample from
`sample_blind.py`, chosen for decision value (paired shapes, checkable
answers) rather than breadth. The remaining ~1990 are ungraded, and the
five expanded-corpus sessions are not represented at all.

**Not done:** the open (non-blind) second pass over the same corpus, which
would surface per-model patterns that blind grading structurally cannot.
