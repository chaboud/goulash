# Quality — blind-graded

**313 of 313 sampled answers graded**, blind (model, provider and shape
hidden), then joined back to provenance. Eight questions across 24 model
cells and both engines.

Scale: `correct` 0=wrong/harmful … 3=fully does the ask; `idiom` 0=bizarre
… 3=how a practitioner would write it; `fit` 0=unusable in a status bar
… 3=crisp one-liner.

---

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

That is the [evidence-beats-knowledge](../wiki/architecture/situated-context.md)
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

**Coverage:** 313 of 2299 total answers — the stratified sample from
`sample_blind.py`, chosen for decision value (paired shapes, checkable
answers) rather than breadth. The remaining ~1990 are ungraded, and the
five expanded-corpus sessions are not represented at all.

**Not done:** the open (non-blind) second pass over the same corpus, which
would surface per-model patterns that blind grading structurally cannot.
