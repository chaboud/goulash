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

Blind-graded quality: S1 correct 1.42 vs S3 **1.41**; paired on the same
model+question, S3−S1 = **+0.14** (5 better, 4 worse, 13 same). The vend
gain is free.

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

### A4. `reasoning_tokens` — **SETTLED: ≥4096, separate from display**

```toml
[engine]
response_tokens  = 256     # what lands in the band
reasoning_tokens = 4096    # allowance on top, only when thinking is on
```

The cap must not be one meter. Measured with `stop` removed so the budget
was actually reachable: at +1024, **19 of 32 still truncated**; at +4096,
15 of 32. So 4096 is the floor for a usable allowance, not a generous one.

### A5. `response_tokens` — **SETTLED: can be raised freely**

The current 256 does **no display work**. Answers that arrive use a
median of **32** tokens (p90 77); visible prose is p50 **61 chars**;
median prose stayed **40–77 chars in every Pass T arm**, including ones
with 4096 tokens available. Only 12 of 634 answered rows ever reached
256.

The prompt keeps answers short, not the ceiling. Raise it for `##` chat;
it will not make the band sprawl.

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

### A7. Situated context — **PENDING**

```toml
[engine]
situated = "platform"   # off | platform | platform+tools
```

Real platform-error rate is **2.0%** (78 of 3961 commands), of which
**`du --max-depth` is 62** — one flag on one command. An earlier 6.9%
figure was a measurement artifact: 68% of its hits came from questions
whose *subject* was the flag ("what does `-P` do in grep"), which
`bench/gnucheck.py` now excludes.

- **platform line** (~50 tok) names `du -d` explicitly, so it targets 78%
  of the real errors.
- **tools line** (~55 tok) targets absent-tool references, which fire
  **25 times in 4002 commands (0.6%)** — 23 of them `rg`.
- **full PATH** (~3900 tok) showed **zero** benefit on gemma4 at 7× the
  context. Completeness does not beat curation even when caching makes
  size cheap.

Verdict pending the offender run (qwen3.5:4b / llama3.2:3b / qwen3.5:9b,
~5% base rate each — gemma4 was 0-1% and could only produce noise).

**Derive, never store.** A stored fact cannot notice it has gone stale;
`brew install fd` and a pinned slot lies with authority. Derivation is
deterministic (byte-identical across runs, so the cache is unaffected)
and costs ~4 ms — in the worker at startup, never on the ask path.

---

## B. Performance levers

### B1. `num_ctx_min` / `num_ctx` — **SETTLED: two settings, not one**

```toml
[engine]
num_ctx_min = 8192      # auto: adopt what is loaded, but at least this
# num_ctx  = 8192       # forced: pin exactly (reproducibility, memory bound)
```

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

### B2. Model selection — **SETTLED: prefer resident**

```toml
[engine]
prefer_resident = true
```

Current `pick_model`: configured → favourites → *smallest installed*, with
no reference to what is loaded. Proposed: configured → **resident** →
favourites → smallest.

Cold load is **median 4281 ms, p90 7214 ms** (7.7 s for `gemma3:12b`)
against a warm TTFT median of **2314 ms**. Adopting a warm 12B is very
plausibly faster end-to-end than cold-loading a 0.8B — and the graded
ranking put `gemma4:12b` at the *top* for correctness, so the big warm
model is not a compromise.

### B3. `keep_alive` — **SETTLED: keep, and map it**

Residency control. Was silently ignored on OpenAI-compat entirely; now
maps to LM Studio's `ttl` (seconds). LM Studio JIT-loads at a **1-hour**
default TTL otherwise.

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
| context is load-time | LM Studio yes | `num_ctx` vs `num_ctx_min` |
| platform-error rate | measured | whether `situated` earns its tokens |

Observed family patterns worth defaulting on:

- **qwen3.5 / qwen3** — reasoning models; `think:false` load-bearing;
  ~5% platform errors; the highest situated-context payoff.
- **gemma4** — reasoning models; `think:false` load-bearing; **0–1%**
  platform errors; top of the graded correctness ranking; situated
  context earns little.
- **llama3.2 / mistral / gemma3** — no reasoning; `think` inert; ~5%
  platform errors on llama3.2.
- **deepseek-r1** — accepts `think:false` and reasons anyway, leaking
  `<think>` into the band. Needs a reasoning-tag strip regardless.
- **mistral-nemo** — answers ordinary questions with bare `REMEMBER:`
  lines. Memory block should be withheld from it, or it should not ship.

---

## D. What I would actually put in `config.toml`

Keeping the zero-setup promise: everything below has a working default
and nobody must ever open the file.

```toml
[engine]
thinking         = "off"       # off | auto | forced
response_tokens  = 512         # raised: 256 was never the binding limit
reasoning_tokens = 4096        # only spent when thinking is on
num_ctx_min      = 8192        # auto-adopt a resident model's context
prefer_resident  = true
situated         = "platform"  # off | platform | platform+tools  (PENDING)
keep_alive       = "30m"

[engine.vend_bias]             # OPEN
ask       = 0.9
proactive = 0.3
```

Internal, not exposed: `command_first` (always on), `stop` (removed),
prompt shape, endpoint choice (a capability), `tail_chars`,
`context_max_chars`.

Live-adjustable via `#/`: `thinking`, `situated`, `vend_bias`, `model` —
the ones a user might plausibly want to change mid-session.

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
