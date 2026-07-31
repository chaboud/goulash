# QUIRKS — do models need per-model adapters?

**Re-run 2026-07-31 against the merged 0.4.0 engine.** Complete Pass A:
**360 probes across 30 cells**, of which 23 answered and 7 were
unreachable (§6). Each probe isolates one setting against a fixed tiny
context; `bench/src/probe.rs` defines them.

This supersedes the 2026-07-29 run (288 probes, 24 cells), which
measured the pre-merge engine — before the stop sequence was dropped,
before capability-gated thinking, and before the context negotiation was
fixed. Numbers that moved are named in place rather than overwritten.

**Answer, on more cells: no per-model *quirks* table is needed** — no
model needed different wording, different settings, or a special case to
work. Two provider-level capabilities are needed. The engine carries one
of them and **not the other**: prompt templating on OpenAI-compatible
servers is unhandled, and it is the sharpest open risk in this release
(§3).

---

## 1. Omitting `think` is the most destructive thing we can do

The headline, and it is not close. Empty-answer rate by probe, out of
the 23 reachable cells:

| probe | empty | median tokens |
|---|---|---|
| `no-stop` | **3/23** | 29 |
| `no-command-needed` | 4/23 | 29 |
| `baseline` | 5/23 | 31 |
| `fence-bait` | 5/23 | 28 |
| `tight-budget` | 6/23 | 29 |
| **`no-think-field`** | **14/23** | 34 |
| **`no-think-no-stop`** | **13/23** | 256 |

Sending **no** `think` field roughly triples the empty rate, ~20% → ~60%.
The cells it kills are the ones that reason by default and were never
told not to: every `gemma4` variant, every `qwen3.5` size, `qwen3:14b`,
`deepseek-r1:14b`. Given the field they answer; denied it they spend the
budget thinking and return nothing.

Note the median on `no-think-no-stop`: **256 tokens, the ceiling.** Not a
terse model stopping early — a model talking until it is cut off. That is
what invisible reasoning looks like from outside, and it is why the
2026-07-29 note recorded two signatures (`4 tokens, stop` and `256
tokens, length`) for one mechanism.

**This is the finding that validates capability-gated thinking.** goulash
asks `/api/show` what the model can do and sends `think: false` to
anything that reports `thinking`; the probe above is what happens the day
that stops working. The rule is not "reasoning is expensive" — it is
**say something rather than nothing.** Silence is read as consent to
reason.

*(Supersedes §2 of 2026-07-29, "10 of 17 ollama cells". Same conclusion,
wider sample, and now stated as a rate rather than a list.)*

## 2. The stop sequence was worse than neutral

`no-stop` has the **lowest** empty rate of any probe — 3/23, below
baseline's 5. Dropping it does not merely cost nothing; it recovers cells
the stop sequence was truncating to nothing:

| cell | with `["\n\n"]` | without |
|---|---|---|
| `google/gemma-4-e4b` (raw) | **empty** | 3 tokens |
| `qwen/qwen3-4b` (raw) | **empty** | 255 tokens |
| `deepseek-r1:14b` | 42 tokens | 256 tokens |

*(This revises "the stop sequence is exonerated" from 2026-07-29. That
finding was correct on its own terms — `stop` was not the cause of the
`no-think-field` empties — but it was read at the time as "`stop` is
harmless". On the wider sample it is not: it has its own victims,
distinct from the reasoning ones.)*

## 3. The chat template turns reasoning on, and it is absolute

Both `openai-chat` cells returned **empty on all twelve probes**,
including ones that work through `/v1/completions` on the same server
with the same weights:

| model | `/v1/completions` | `/v1/chat/completions` |
|---|---|---|
| `qwen/qwen3-8b` | 34 tokens | **empty ×12** |
| `google/gemma-4-e4b` | 1 token | **empty ×12** |

Read that raw column with the correction below in mind: neither cell was
*answering*. One completed the text, the other emitted a token and
stopped.

The 2026-07-30 correction argued the chat path was *starved* rather than
broken — measured at a 256 ceiling, it never got to finish. That remains
the mechanism.

### Correction (2026-07-31): "prefer `/v1/completions`" was wrong

That advice stood in this file for two days and it is **not safe**. It
came from seeing chat return empty and raw return non-empty, and reading
non-empty as working. It is not.

The two endpoints are not the same call with a different wrapper.
**ollama's `/api/generate` applies the model's chat template by default;
LM Studio's `/v1/completions` applies nothing.** goulash treats them as
equivalent, and they are not. Sent the identical bare prompt
`"Say hello."`:

| engine / endpoint | model | result |
|---|---|---|
| ollama `/api/generate` | `gemma4:12b` | `Hello! How can I help you today?` ✓ |
| LM Studio `/v1/completions` | `google/gemma-4-e4b` | `SaySay hello. SaySay hello. SaySay…` |
| LM Studio `/v1/completions` | `qwen/qwen3-8b` | `Then, write a short paragraph about your favorite animal…` |

Two distinct failures, and neither reports an error:

- **Gemma degenerates without its turn markers.** Supplying them by hand
  through the same endpoint fixes it outright — `google/gemma-4-e4b`
  goes from an infinite `SaySay` loop to `Hello.` in 1.1 s with
  `finish_reason: stop`. This is a template problem, not a model
  problem.
- **qwen3 does not degenerate; it *completes*.** Given an instruction it
  continues the text rather than following it — inventing a follow-up
  exercise, or answering as a fictional teenager. Coherent, non-empty,
  and not an answer. This is the more dangerous of the two, because
  every mechanical metric scores it as success.

**Everything measured through `openai-raw` in this file and in
[QUALITY](QUALITY.md) is therefore measuring untemplated completion, not
instruction-following.** Those cells' quality numbers should not be
compared against ollama's. Pass A's `google/gemma-4-e4b` row — recorded
as "answers" — was a single junk token followed by a stop.

**For the product:** goulash's OpenAI-compatible wire sends the prompt
verbatim to `/v1/completions`, so pointing 0.4.0 at LM Studio with a
Gemma model produces garbage, and with a qwen model produces plausible
nonsense. It needs to either apply the model's template or use the chat
endpoint and pay the reasoning cost. That is an open 0.4.0 question, not
a settled one — see [open-questions](../wiki/product/open-questions.md).

### `google/gemma-4-12b-qat` is separately broken

Turn markers do **not** rescue it. Templated, it emits
`<|channel|>` — a gpt-oss/harmony special token that has no business in
a Gemma vocabulary — and fragments like `_end_turn>`, which is
`<end_of_turn>` mis-split. That is a wrong or corrupt tokenizer in the
downloaded file, not a prompting issue.

It is also the single most expensive cell in the grid: 47 s per
generation, every generation hitting the token ceiling because it never
emits a stop token. 6.3 hours of the sweep to produce 476 walls of
zeroes. **Re-download it or drop it from the catalog.**

## 4. `gpt-oss:20b` cannot be rescued by settings

Empty on `baseline`, `no-stop` *and* `no-think-field` alike. It reasons
whether or not asked, and 256 tokens is not enough to reason and answer.
`models.rs` carries it as `Think::Levels` with `always_reasons: true` and
a 2048-token allowance, which is the right shape: it needs budget, not a
dialect.

## 5. Non-findings worth recording

**Nothing fenced. 0 of 276 answers** — including `fence-bait`, built to
provoke it. Fencing was a real problem in an earlier era of this prompt
and is now simply absent. No stripping logic is needed, and none should
be added on suspicion. *(Confirms 2026-07-29 at 0/288.)*

**No provider reports reasoning tokens.** 0 of 276 rows carried a
non-zero count — not ollama, not LM Studio raw. The `reasoning tok`
column in the report card is structurally empty for local engines, so the
**only** local signal for reasoning spend is the empty rate in §1. Worth
knowing before anyone tries to budget against a number that does not
exist.

**Stop reasons:** 223 `stop`, 52 `length`, 1 unknown. The `length` runs
cluster in the `-1024` headroom probes, which is what those exist for.

## 6. Cells excluded — and what that costs us

Seven of thirty unreachable, 84 probes skipped:

- Six uninstalled since the catalog was written: `mistral`,
  `mistral-nemo`, `gemma3:12b`, `devstral:24b`, `gemma3:27b`,
  `mixtral:8x7b-q2_K`. Verified that ollama does **not** auto-pull on
  `/api/generate`, so they fail immediately and cost nothing — but the
  catalog should be trimmed to the box.
- `qwen/qwen3.6-35b-a3b` (22 GB) failed to serve from LM Studio.

Two 2026-07-29 findings therefore stand **unverified on 0.4.0**, because
their cells did not run: `mistral-nemo` answering questions with memory
writes (5 of 7 probes returning only `REMEMBER:`), and the smallest-model
refusal on `qwen3.5:0.8b`. Neither is retracted; neither is re-confirmed.

The 13-17 GB cells that the 2026-07-29 run excluded for memory pressure
**did** run this time (`gpt-oss:20b`, `deepseek-coder:33b`) and their
results are in §4 and the report card.

## 7. Measurement honesty

The ~6% flake at the empty/ok boundary recorded on 2026-07-29 has not
been re-measured — that required running the same probe set twice, and
this run went once. Treat any single cell's verdict as carrying roughly a
1-in-16 flip risk; the family-level patterns in §1 rest on four or five
cells agreeing and are solid.

---

## Verdict

Unchanged, on 25% more cells: **no per-model adapter table is required.**
What is required sits at the provider layer and is already built —

| capability | who needs it |
|---|---|
| working reasoning suppression | ollama: ask `/api/show`, send `think: false` — this works and is shipped |
| **prompt templating** | OpenAI-compat: `/v1/completions` applies NO template, so an instruction prompt is completed rather than followed. **Open, and the one thing 0.4.0 does not handle** (§3) |

The second row is the sharpest open risk in this release: goulash's
OpenAI-compatible path is measurably wrong on LM Studio, silently, and
the failure scores as success on every mechanical metric.

Get either wrong and the failure is silent — a blank bar and no reason
given, which is the shape of every serious defect in this codebase
([wiki/meta/care.md](../wiki/meta/care.md)).

Two goulash-side items from 2026-07-29 remain open and are independent of
model choice: `split_answer` has no notion of a `<think>` tag if
suppression ever fails, and a memory-only reply renders as silence plus
an unannounced memory write.
