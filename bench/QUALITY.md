# Quality — blind-graded

68 answers graded blind (model, provider and shape hidden), then joined
back. Two questions, chosen because they have checkable answers:

- **`jq-extract`** — "pull every `.items[].name` out of data.json". One
  right answer, several near-misses that look right.
- **`git-undo`** — "undo my last commit but **keep the changes staged**".
  A correctness trap: `--soft` keeps them staged, plain `git reset`
  (mixed) *unstages* them.

Scale: `correct` 0=wrong/harmful … 3=fully does the ask; `idiom` 0=bizarre
… 3=how a practitioner would write it; `fit` 0=unusable in a status bar
… 3=crisp one-liner.

## 1. Command-first costs nothing in quality

The open question from the shape sweep, now answered:

| shape | n | correct | idiom | fit |
|---|---|---|---|---|
| S1 (prose first) | 36 | 1.42 | 1.25 | 1.61 |
| S3 (`CMD:` first) | 32 | **1.41** | 1.31 | 1.44 |

Paired on the same (model, question) under both shapes: mean S3−S1 =
**+0.14**, with 5 better, 4 worse, 13 unchanged.

**No quality cost.** So the 56% → 81% vend-rate gain from command-first
is free, and the earlier concern that it might trade accuracy for
compliance does not hold.

## 2. The `git-undo` trap: most models get it wrong, and 9 lie about it

Of 32 graded answers to "keep the changes staged":

| outcome | n |
|---|---|
| correct — `git reset --soft HEAD~1` (or `HEAD^`) | **10** |
| **confidently wrong** — plain `git reset HEAD~1`, with prose asserting it keeps changes staged | **9** |
| broken, empty, or invalid flags | 13 |

The middle row is the dangerous one. These answers *look* right:

> `git reset HEAD~1` — "The previous commit is undone while keeping
> changes staged."

That is a mixed reset. It unstages everything. The prose states the
opposite of what the command does, and the command lands on the user's
prompt line ready to run.

Also seen: `--keep-index` (a `git stash` flag, not `git reset`),
`--staged` (not a flag at all), and `git reset HEAD~ --soft .` (`--soft`
with a pathspec is a hard git error). Those at least fail loudly.

This is a worse failure mode than the destructive commands catalogued in
[the vend-bias backlog entry](../wiki/product/build-plan.md): `rm -rf`
looks dangerous, so a user hesitates. `git reset HEAD~1` with a
reassuring sentence does not.

## 3. Model ranking

Small n — treat as indicative, not settled.

| model | n | correct | idiom | fit |
|---|---|---|---|---|
| `gemma4:12b` | 3 | **3.00** | 2.33 | 2.67 |
| `gemma4:12b-mlx` | 4 | **3.00** | 2.50 | 3.00 |
| `qwen/qwen3-4b` (raw) | 3 | 2.67 | 2.33 | 1.67 |
| `gemma4:e2b` | 3 | 2.33 | 1.67 | 2.67 |
| `gemma4:e4b-mlx` | 4 | 2.00 | 2.25 | 2.50 |
| `gemma3:12b` | 3 | 2.00 | 2.00 | 2.67 |
| `qwen3.5:4b` | 4 | 1.50 | 1.25 | 1.50 |
| `gemma4:e4b` | 4 | 1.25 | 1.25 | 2.25 |
| `qwen/qwen3-8b` (raw) | 5 | 1.00 | 1.20 | 0.40 |
| `mistral:latest` | 3 | 1.00 | 1.00 | 2.00 |
| `llama3.2:3b` | 3 | 1.00 | 0.67 | 1.33 |
| `google/gemma-4-e4b` (raw) | 8 | 0.12 | 0.25 | 0.38 |
| `qwen3.5:2b` | 3 | 0.00 | 0.00 | 0.33 |
| `deepseek-r1:14b` | 3 | 0.00 | 0.00 | 0.00 |
| `qwen/qwen3-1.7b` (raw) | 3 | 0.00 | 0.00 | 0.00 |

The zeroes are mechanical failures already documented elsewhere, not bad
reasoning: `deepseek-r1:14b` leaks a bare `<think>` tag to the band
([QUIRKS](QUIRKS.md) §2), and `google/gemma-4-e4b` echoes the prompt back
on raw completions ([QUIRKS](QUIRKS.md) §1). Fix those and their scores
would change.

The gemma4 12B pair topping the table is notable given the residency
argument: a warm 12B is both the most accurate here *and*, per
[RESIDENCY](RESIDENCY.md), plausibly faster end-to-end than cold-loading a
small model.

## 4. Failure modes worth naming

Recurring across both questions, independent of correctness:

- **Command inside the prose, nothing vended** — the answer is right but
  arrives as `` `jq -r …` `` in a sentence. 4 of 68.
- **Backticks inside the `CMD:` line** — the vended text includes the
  backticks and would fail if run. 3 of 68.
- **Prose/command contradiction** — "I need to see the file first"
  attached to a working command; "unstages the last commit" attached to
  `--soft`.
- **Transcript-format bleed** — `goulash:` prefixes copied out of the
  session log into the answer, sometimes doubled.
- **Sandbox path leakage** — one answer embedded the bench's own
  `/var/folders/…` temp path; another stashed a list of real repo files.
- **Essays** — correct commands buried under six lines of explanation
  (`fit` 0). Rare but total: unusable in a one-line band.

## Coverage and honesty

This grades **2 of 58 questions** and 68 of 2299 answers. It was chosen
for decision value, not coverage: the shape comparison needed paired
answers, and `git-undo` was picked precisely because it has a subtle
right answer.

Every grade is auditable — `bench/results/2026-07-28/grades.jsonl` holds
the score and a one-line rationale per id, `blind_sample_keys.jsonl`
joins ids to provenance, and `goulash-bench replay DIR <id>` prints the
exact prompt and raw response behind any of them.

Not yet done: the open (non-blind) second pass, the remaining 6 sampled
questions, and any grading of the expanded-corpus sessions.
