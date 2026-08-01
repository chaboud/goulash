# Levers: what to expose, and how to set it

Synthesis of the [characterization sweep](../../bench/) — ~5,500 measured
generations across 24 model cells and two engines. Every default below
carries the evidence that produced it, and anything unmeasured says so.

Each lever is tagged **SETTLED** (measured, confident), **PENDING**
(experiment in flight), or **OPEN** (not yet measured).

---

## A. Quality levers

### A1. `command_first` — **SETTLED: default ON**

Ask for `CMD:` before the prose line.

| | `#` asks | proactive | proactive PASS |
|---|---|---|---|
| prose first (S1) | 52% | 44% | 6% |
| **command first (S3)** | **77%** | 43% | 21% |

Blind-graded quality, paired on the same model+question: **n=112,
S3−S1 = −0.05** (19 better, 24 worse, 69 unchanged). Noise around zero —
**no detectable quality difference in either direction**, which is the
result that matters, because it makes the vend gain free.

*(An earlier partial pass — 2 questions, n=22 — read **+0.14** and this
page described S3 as marginally ahead. At 5× the pairs it is −0.05. Both
are noise; the small one was overread. See
[QUALITY](../../bench/QUALITY.md) §1.)*

It also fails in the right direction: if a budget ever binds, prose is
lost rather than the command — and prose is display-bound anyway.

**Not user-facing.** No user wants this knob; it is simply the better
prompt.

### A2. `stop` sequence — **SETTLED: drop `["\n\n"]`**

The most surprising result in the project.

| | reasoning off | reasoning on |
|---|---|---|
| with `stop` | 81% answered | 11/36, median **39** tokens |
| **without** | **94% answered** | 19/32, median **1279** tokens |

It never truncated a working non-reasoning model to empty — that Pass A
finding stands — but it is not free either: **+13 points of answer rate**
when removed. And with reasoning enabled it is categorically fatal: the
blank line inside or before the thinking trips it, so the model emits ~4
tokens and halts no matter how large the budget.

It exists to enforce the one-line contract, which the directive and the
band's `wrap_chars` already enforce. It buys nothing and costs answers.

**Not user-facing.** Remove it; keep `stop` as an internal
`GenRequest` field for benchmarking.

### A3. `thinking` — **SETTLED: `off | auto | forced`, default `off`**

```toml
[engine]
thinking = "off"        # off | auto | forced
```

- **`off`** — send `think: false`. Load-bearing for 12 of 24 cells (every
  qwen3.5, every gemma4, qwen3:14b); inert elsewhere.
- **`auto`** — on only where it is *known to help*, never "on wherever
  supported". Reasoning costs **25 points of reliability** even given
  4096 tokens of room (22/32 vs 30/32 with it off).
- **`forced`** — debug only.

**Capability gating is mandatory, not polite.** ollama returns
**HTTP 400 `"llama3.2:3b" does not support thinking`** — a hard error, not
a degradation. 8 of 24 cells fail this way. `auto` must consult
capability before sending the field.

Provider-specific: on OpenAI-compat, `chat_template_kwargs:
{enable_thinking: false}` makes things **worse** — it empties `content`
and routes everything to `reasoning_content`. Do not send it.

### A4. `reasoning_tokens` — **SETTLED: ≥4096, separate from display, applied unconditionally**

```toml
[engine]
response_tokens  = 1024    # what lands in the band
reasoning_tokens = 4096    # allowance on top, always
```

The cap must not be one meter. Measured with `stop` removed so the budget
was actually reachable: at +1024, **19 of 32 still truncated**; at +4096,
15 of 32. So 4096 is the floor for a usable allowance, not a generous one.

**The allowance is not conditional on asking for thinking** (corrected
2026-07-30). It used to be added only when `thinking != off`, which asks
whether *we* requested reasoning rather than whether reasoning can
happen — and nothing lets us answer the second question with a no:

- OpenAI-compatible **chat** applies the model's template, which reasons
  regardless of what we send; the one kwarg that disables it empties
  `content` instead (§A3).
- Even ollama, which honours `think:false` for most models, does not for
  all — `deepseek-r1:14b` accepts it and reasons anyway.

Budgeting on our own intent produced exactly the failure the split was
meant to prevent: **158 of 180 chat rows empty (87%)**, every one
`finish=length` with the display budget spent on reasoning. Re-measured
at the shipped budget, the same weights through the same endpoint answer
normally, using 659–896 reasoning tokens — never going to fit in 256.

FAST carries 2048 even though it asks for no thinking, so a model that
thinks anyway returns a *slow* answer rather than *no* answer.

### A5. `response_tokens` — **SETTLED: can be raised freely**

The old 256 did **no display work**. Answers that arrive use a median of
**32** tokens (p90 77); visible prose is p50 **61 chars**; median prose
stayed **40–77 chars in every Pass T arm**, including ones with 4096
tokens available. Only 12 of 634 answered rows ever reached 256.

The prompt keeps answers short, not the ceiling — so the ceiling is now
1024. A ceiling caps, it does not reserve; unused headroom is free, and
the cost of setting it too low is an empty answer, which A4 measured.

### A6. `vend_bias` — **OPEN: needs the scale designed**

```toml
[engine]
vend_bias.ask       = 0.9   # 0 = never vend, 1 = always
vend_bias.proactive = 0.3
```

The two surfaces already diverge (77% vs 43%), so the dial makes explicit
what is currently emergent in two directive strings. Over-vending on `#`
is **on-contract** — the user asked, and an imperfect command is a usable
example. Unprompted commentary is where it costs.

Risk half: 78 of ~4000 vended commands matched a destructive pattern,
mostly on a bait scenario. The 5 genuine non-sequiturs (`rm -rf .` in
answer to a disk-usage refinement; `git reset --hard` when asked to
*keep* changes) were all watcher-tier models. At low bias, skew toward
silence where confidence is lowest.

### A7. Divulging machine facts — **SETTLED: platform on, rest debug**

Telling the model facts about the machine it is running on. Three
independent toggles — not a ladder; `full_path` *replaces* `tools`
rather than adding to it.

```toml
[engine.divulge]
platform  = true    # "macOS, BSD userland, zsh. du -d not --max-depth, no grep -P"  (~50 tok)
tools     = false   # "installed: jq tree gh git... not installed: rg fd wget"       (~55 tok)
full_path = false   # all 1739 executables                                        (~3900 tok)
```

**None of the three showed a measurable effect.** Against the models that
actually make platform errors (`qwen3.5:4b`, `llama3.2:3b`, ~5% base
rate):

| toggle | errors | rate | 95% CI | p |
|---|---|---|---|---|
| none | 4/109 | 3.7% | [1.4, 9.1] | — |
| platform | 3/131 | 2.3% | [0.8, 6.5] | 0.70 |
| + tools | 3/131 | 2.3% | [0.8, 6.5] | 0.70 |
| full_path | 1/127 | 0.8% | [0.1, 4.3] | 0.18 |
| via memory store | 5/129 | 3.9% | [1.7, 8.8] | 1.00 |

Every interval overlaps. Detecting a halving of a 3.7% rate at 80% power
needs **~1235 commands per arm**; we have ~120, which is 10x short.

**`platform` still defaults ON**, because "prove it helps" is the wrong
test for a statement that is *free* and *certainly true*. The right test
is "prove it hurts", and nothing suggests it does:

- **Free, verified.** Per-token prompt-eval is identical with and without
  it — 3411 vs 3400 us on turns 1-3, 1561 vs 1587 mid-session, **750 vs
  760 on turn 10+**. It lives in the cached prefix, so it costs ~180
  tokens once and nothing thereafter.
- **True by construction.** `uname` and `$SHELL` are facts, not guesses.
  A wrong platform line is impossible in a way a wrong tools list is not.
- **Direction is right**, even if the effect cannot be resolved.

`tools` and `full_path` stay **off, as debug options**. `tools` targets
absent-tool references, which fire 25 times in 4002 commands (0.6%) and
carry a curation problem — which 36 tools, maintained by whom.
`full_path` costs 5766 prompt tokens against 862 and nearly doubled
prompt-eval on `qwen3.5:4b` (3700 -> 7290 ms); even if its 1-error result
were real it is a bad trade.

**Never via the memory store.** It was the only arm to go backwards, and
it agrees with the staleness argument: a stored fact cannot notice it has
gone out of date, and wrapping machine facts in "pinned memories — yours
to manage, REMEMBER/FORGET" invites the model to treat ground truth as an
editable opinion. Derive it every run; the derivation is deterministic,
so the cache is unaffected, and it costs ~4 ms in the worker.

---

## B. Performance levers

### B1. `num_ctx` — **SETTLED: goulash sends nothing**

```toml
[engine]
# num_ctx = 8192        # pin one exactly. Unset sends no window at all.
```

Every engine already has a place to configure this, and the person who
configured it meant it. goulash names a model and takes what it is
given.

The measurements below stand: a window IS part of a load identity, and
asking for a different one DOES evict. What does not follow is that
goulash should therefore choose the number. An earlier design read the
resident window, echoed it back, and demanded a floor when nothing was
loaded — and the result on one machine, a minute apart, was `ollama run
gemma4:e4b` giving that model 131072 tokens while goulash pinned the
same model at 8192 and held it there across every session. That is not
preserving a setting. That is overriding it with an invented number.

If a pinned window is too small for the prompt the request fails and
says so, which is how a setting someone chose should behave.

`num_ctx` is **part of a model's load identity**:

| | `load_duration` |
|---|---|
| same model, same ctx | **206 ms** (reused) |
| same model, ctx changed | **1847 ms** (full reload) |
| changed back | 1875 ms (reloads again) |

So sending an unconditional `num_ctx` evicts whatever the user has
resident and thrashes it. KV costs ~**17.5 MB per 1k tokens** — real, but
small enough that goulash need not fight for a *specific* small value.

On OpenAI-compat there is no request-level equivalent; LM Studio fixes it
at load, and each model's saved default differs wildly (`qwen3-1.7b`
8192, `gemma-4-e4b` **118272**). So `auto` = respect what they set is the
only behaviour that works on both engines.

### B2. Model selection — **SETTLED: a favourites list, then a floor**

`pick_model`, in order:

1. the configured model — nothing overrules it
2. the first installed `engine.favorites` entry, in the list's own order
3. the smallest installed model at or above `MIN_AUTO_PARAMS_B` (2.0)
4. the smallest installed model

Step 3 exists because "smallest installed" alone hands a new user
whatever tiny model they once pulled to try ollama out, and below ~2B
the one-line-plus-`CMD:` contract does not hold. Step 4 exists because a
floor that finds nothing must not leave goulash with no model at all.

Not "prefer resident": that was the same reflex as B1, reaching for what
the server happens to be holding. The favourites list says what actually
does this job well, which is a different and better question than what
is warm.

Cold load is **median 4281 ms, p90 7214 ms** (7.7 s for `gemma3:12b`)
against a warm TTFT median of **2314 ms**. Adopting a warm 12B is very
plausibly faster end-to-end than cold-loading a 0.8B — and the graded
ranking put `gemma4:12b` at the *top* for correctness, so the big warm
model is not a compromise.

### B3. `keep_alive` — **SETTLED: goulash sends nothing**

Residency is the server's policy, on the same reasoning as B1. Sending
`30m` on every request is not a hint: it overwrites the policy of a
service someone configured, and holds their VRAM for half an hour
because they once asked what time it was. Unset by default; set it if
you want a specific TTL.

The cost of not asking is a slower first ask after a gap, which is a
fine thing to pay.

### B4. Endpoint choice — **SETTLED: prefer `/v1/completions`**

The single largest provider effect measured:

| model | endpoint | answered | reasoning tokens |
|---|---|---|---|
| `qwen/qwen3-8b` | `/v1/completions` | **12/12** | **0** |
| `qwen/qwen3-8b` | `/v1/chat/completions` | **1/12** | **1450** |

Same weights, same server, same prompt. **The chat template is what
enables reasoning.** Raw completions bypass it entirely. It is also the
better cache shape — the raw endpoint takes goulash's stable-prefix
string verbatim, as ollama's `/api/generate` does.

Caveat: raw is not free — `gemma-4-e4b` sometimes *continues* the prompt
instead of answering. So this is a per-model preference, not a blanket
one, which is an argument for it being a **capability** rather than a
setting.

### B5. Prompt shape / memory position — **OPEN**

`memory.rs:148` leads its block with a live `(N/25 slots)` count, sitting
*before* the session log — so any REMEMBER/FORGET invalidates the whole
prefix behind it. The S2 arm was built to price this and its first run
measured nothing (memories were a constant); the fixed version has not
been analysed yet.

Regardless of the number, the header should not carry a volatile count.

### B6. Idle repaint — **SETTLED and shipped**

Was ~1 KB/s on a terminal doing nothing — 86 MB/day. Now gated on output
having arrived since the last repaint: **806.9 → 0.0 B/s**. No setting;
just correct.

---

## C. Model-smart defaults

The point of a capability table rather than a per-model quirks table.
Everything here is a *provider or model capability*, discovered once and
cached:

| capability | how it is learned | what it sets |
|---|---|---|
| supports thinking | ollama 400s if not | gates `thinking = auto` |
| reasoning suppression works | measured per model | `think: false` vs endpoint choice |
| honours `stop` | provider `Caps` | whether to send it at all |
| reports prompt-eval time | ollama yes, OpenAI no | which cache metric to trust |
| context is load-time | LM Studio yes | send neither; see B1 |
| platform-error rate | measured | whether `divulge.platform` earns its tokens |

Observed family patterns worth defaulting on:

- **qwen3.5 / qwen3** — reasoning models; `think:false` load-bearing;
  ~5% platform errors; the highest machine-facts payoff.
- **gemma4** — reasoning models; `think:false` load-bearing; **0–1%**
  platform errors; top of the graded correctness ranking; machine-facts
  context earns little.
- **llama3.2 / mistral / gemma3** — no reasoning; `think` inert; ~5%
  platform errors on llama3.2.
- **deepseek-r1** — accepts `think:false` and reasons anyway, leaking
  `<think>` into the band. Needs a reasoning-tag strip regardless.
- **mistral-nemo** — answers ordinary questions with bare `REMEMBER:`
  lines. Memory block should be withheld from it, or it should not ship.

---

## D. What actually shipped in `config.toml`

**This section was a proposal; as of 0.4.0 it is the shipped surface.**
Keeping the zero-setup promise: everything below has a working default
and nobody must ever open the file.

```toml
[engine]
thinking         = "off"       # off | auto | forced
response_tokens  = 1024        # raised: 256 was never the binding limit
reasoning_tokens = 4096        # allowance on top, always (see A4)
# num_ctx        = 8192        # unset: take the server's own window
# keep_alive     = "30m"       # unset: the server's own residency policy

[engine.divulge]
platform  = true               # OS, shell, terminal — ~50 tokens, free
tools     = false              # debug: curated tool inventory
full_path = false              # debug: every executable on PATH

[engine.fast]                  # volunteers on ordinary commands
watch = true
ask   = true
thinking = "off"

[engine.slow]                  # amends a # answer underneath FAST
watch = false                  # unprompted self-rewrites are noise
ask   = true
thinking = "auto"
```

**Where the shipped surface differs from this proposal, and why:**

| proposed | shipped | reason |
|---|---|---|
| `situated = "platform"` | `[engine.divulge]` booleans | "situated" is jargon that says nothing to a reader; the flags are independent, so they are flags |
| `prefer_resident = true` | dropped | a favourites list says what does this job well; what is warm is a different question (see B2) |
| `response_tokens = 512` | `1024` | see A5 |
| `reasoning_tokens` only when thinking on | always | see A4 — that condition was the bug |
| `[engine.vend_bias]` | not shipped | still **OPEN**; backlogged rather than guessed at |

Internal, not exposed: `command_first` (always on), `stop` (removed),
prompt shape, endpoint choice (a capability), `tail_chars`,
`context_max_chars`.

Live-adjustable via `#/`, as shipped: `#/thinking`, `#/divulge`,
`#/fast`, `#/slow`, `#/model`, `#/memory`, `#/commentary`, `#/status`,
`#/help` — the ones a user might plausibly want to change mid-session.
`vend_bias` is not among them because it does not exist yet.

---

## E. `--config`, and what "reset" should mean

Backlog item 4 in [build-plan](../product/build-plan.md), now with a
reason to exist: the sweep produced a dozen settings and none of them are
reachable without editing TOML by hand.

```
goulash --config                 # interactive walkthrough (offered, never required)
goulash --config print           # effective values AND where each came from
goulash --config path            # where the file lives
goulash --config set k v         # surgical write (reuses toml_edit)
goulash --config reset [k]       # back to defaults, whole file or one key
```

### Reset means *remove the key*, not *write the default value*

The important detail. Every setting has a working default in code, so
resetting should **delete the key** and let the default apply again —
never stamp today's default into the file.

Two things fall out of that:

- **Defaults can improve.** A user who reset in 0.4.0 gets 0.5.0's better
  default automatically. If reset wrote `response_tokens = 512`, they
  would be pinned to it forever without knowing.
- **`config.toml` stays small.** It holds only deliberate deviations, so
  `--config print` can honestly show *default* vs *set by you* — which is
  the whole value of the `print` subcommand.

`--config reset` with no key backs the old file up (`config.toml.bak`)
before truncating, because it is the one destructive thing in the CLI.

## F. Providers: how defaults should resolve

Today: one `provider` string, one `host`, a hardcoded probe chain. That
cannot express "try ollama, then LM Studio", cannot carry per-provider
settings, and has nowhere to put an API key.

```toml
[engine]
service = "auto-local"        # auto-local | <a provider name> | none

[[provider]]
name = "ollama"
kind = "ollama"               # kind implies wire protocol + capabilities
host = "http://127.0.0.1:11434"

[[provider]]
name = "lmstudio"
kind = "openai"
host = "http://127.0.0.1:1234"
endpoint = "completions"      # completions | chat

[[provider]]
name = "llamacpp"
kind = "openai"
host = "http://127.0.0.1:8080"
```

`auto-local` probes the list in order and binds the first that answers.
`#/service auto-local` restores it after any explicit pin.

**`kind` carries the measured defaults**, so a user adding a provider
gets the right behaviour without knowing any of this:

| kind | endpoint | reasoning control | cache metric | context |
|---|---|---|---|---|
| `ollama` | `/api/generate` | `think` field (works) | `prompt_eval_duration` | per request |
| `openai` | **`/v1/completions`** | none that works | TTFT flatness | load-time |

The endpoint default is the single most consequential provider setting
measured: `qwen/qwen3-8b` answered **12/12 via `/v1/completions` with zero
reasoning tokens**, and **1/12 via `/v1/chat/completions` with 1450** —
same weights, same server. The chat template is what turns reasoning on.
Cloud providers offer chat only, so a hosted `kind` inherits that problem
and needs its own reasoning control.

### Precedence — the actual answer to "how should defaults work"

Lowest to highest:

1. **compiled-in defaults** — everything works with no file at all
2. **provider `kind` defaults** — measured per protocol (table above)
3. **model capability** — discovered, not configured: does it support
   thinking (ollama 400s if not), does suppression work, does it honour
   `stop`. Overrides a *kind* default because it is a fact about reality.
4. **config file** — the user's deliberate deviations
5. **session commands** (`#/model`, `#/thinking`, …) — until exit

Capability sitting *above* kind defaults but *below* the config file is
the load-bearing choice: goulash should never send `think: true` to a
model that hard-400s on it, but if a user explicitly sets
`thinking = "forced"` they get it and own the error. Discovery protects
the default path; it does not overrule an instruction.

### Keys

Env by default (`api_key_env = "OPENAI_API_KEY"`), file value as
fallback, never in transcripts. Nothing in this project needed one — all
local — but the provider table is where it goes when it does.
