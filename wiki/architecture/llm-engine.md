# LLM Engine: Providers, Caching, Local Models

## Selectable providers

Provider choice is a plugin boundary: Anthropic / OpenAI / others / local
(llama.cpp, ollama, platform engines). Different roles can bind to
different providers — see the two-tier split below.

### Implemented: two wires, several servers (`src/wire.rs`)

```toml
[engine]
provider    = "auto"                     # auto | ollama | openai | none
                                         # lmstudio/llamacpp/vllm = openai
host        = "http://127.0.0.1:11434"   # ollama
openai_host = "http://127.0.0.1:1234"    # LM Studio's default port
api_key_env = ""                         # env var holding a bearer token
```

**One OpenAI-compatible wire reaches LM Studio, llama.cpp's server and
vLLM**, because all three expose the same `/v1`. `auto` tries ollama
first (it is the zero-config default) and falls back to the
OpenAI-compatible port; naming a provider explicitly skips the probe, so
a user who says `lmstudio` gets a clear "unreachable" instead of a silent
fallback to the other one.

**The load-bearing choice is `/v1/completions`, not
`/v1/chat/completions`.** goulash sends a raw prompt, and that is not
incidental — the whole design below is a stable prefix plus prefix KV
caching. Chat-completions hands prompt assembly to the server's template,
which moves the prefix boundary out from under us and can silently stop
the cache from hitting. Same prompt bytes, different envelope; a unit
test asserts the two wires produce byte-identical `prompt` fields, and
the e2e fake asserts the preamble still arrives intact.

What the wires do *not* share:

| | ollama | OpenAI-compatible |
|---|---|---|
| budget | `options.num_predict` | `max_tokens` |
| context window | `options.num_ctx` | server-side, not per-request |
| truncation floor | `options.num_keep` | server-side |
| seed | `options.seed` | `seed` |
| residency | `keep_alive` | server-side |
| reasoning | `think` (bool or level) | `reasoning_effort` (level only) |
| stream | newline-delimited JSON | SSE, `data:` … `[DONE]` |
| model list | `/api/tags` (+ sizes) | `/v1/models` (names only) |
| capabilities | `/api/show` | *nothing* |

Three consequences worth stating, because each is a place the naive port
would break:

- **Sending ollama's fields to a strict server is a 400, not a shrug** —
  so the OpenAI body omits them rather than letting them ride. The e2e
  fake rejects them deliberately, and that assertion has been checked by
  deliberately leaking one.
- **The server truncates from the LEFT when the prompt outgrows the
  window**, and the left is where the preamble and the pinned memories
  live — so the thing dropped first is the grammar and the facts the
  user explicitly asked us to remember, silently. `num_keep` (default
  512 tokens) is the floor that stops it. There is no OpenAI equivalent;
  there it is a server setting, so on LM Studio the protection has to be
  configured at the server. Which is also why the stats row now reports
  estimated prompt tokens against `num_ctx` with a `!` when over: until
  it did, "was my prompt truncated?" had no answer from inside a
  session, and a whole class of bug was unfalsifiable.
- **No `/api/show` means no provider opinion on reasoning**, so
  `Source::Table` in [model-capabilities](model-capabilities.md) simply
  stays authoritative — the precedence chain was already built for a
  provider that won't say.
- **No sizes means no smallest-installed default.** There the fallback is
  the first model the server lists, which for LM Studio is the one you
  last loaded — the closest thing it has to an intent.

`reasoning_effort` is a **spelling**, not a fourth dialect:
`Caps::effort_field` derives it from the model's existing `Think::Levels`
classification, so `models.rs` stays the single place that knows what a
model can do. A `Think::Bool` model has no standard way to say this over
that wire, so it is omitted rather than guessed at — the budget
allowance, which is the half that actually prevents a blank bar, applies
either way.

### Per-lane bindings

Fast and slow can be different models, different servers, or different
machines. `[engine]` is the default for both; `[engine.slow_lane]`
overrides only what it names:

```toml
[engine]
provider = "ollama"                        # small, local, answers now
model    = "qwen3:4b"

[engine.slow_lane]                         # bigger, elsewhere, researches
provider    = "lmstudio"
openai_host = "http://192.168.1.9:1234"
trusted     = "yes"
```

**An absent (or identical) `[engine.slow_lane]` means one binding
serving both roles**, and that is the load-bearing default: two lanes on
one model must not mean two model loads, two KV caches, or two entries
in the same server's queue. `lanes_split()` is what decides, and it
compares resolved values rather than trusting the table's presence — a
slow table that restates the same settings is not a reason to bind
twice.

Consequences of a lane being genuinely elsewhere:

- **Its model menu comes from its own server.** `#?/model` lists what
  that box has; the fast lane's inventory would be a lie there.
- **Its capabilities are resolved against its own backend**, so a
  reasoning slow model and a non-reasoning fast one each get the right
  dialect and the right budget.
- **`#?/model` binds it**, using the same sigil scoping the cancels
  already established: `#/x` is global, `#?/x` is slow's. Naming a slow
  model is also what *splits* a shared binding, since pointing the
  research role at a second model is the whole reason to separate them.

Binding via `#?/model` lasts the session and is not persisted — the lane
lives in a TOML *table*, and the surgical `persist_model` writer only
knows how to rewrite the single scalar. The notice says so rather than
implying it stuck.

### Trust is stated, not inferred

```toml
trusted = "auto"     # auto | yes | no, per lane
```

An earlier draft made "no API key" imply "local, therefore trusted".
That was rejected: **no api key is a coincidence, not consent.** Trust
is now its own setting, resolved per lane, with exactly one narrow
inference behind `auto` — a loopback host is this machine, and nothing
else qualifies. Anything unrecognised falls to *not* trusted, because
the failure directions are not symmetric: wrongly withholding a file
costs an answer, wrongly sending one cannot be undone.

`yes` is what a user with a GPU box on their own LAN sets, which `auto`
could never work out; `no` is what someone sets for a local server they
happen not to trust. Both override auto in their own direction.

`#/status` names it **only when it is not the safe case** — a line that
says "trusted" beside every lane trains people to stop reading it. And
the marker goes at the *front* of the line: `#/status` shares one bar
row, whatever falls off the end is whatever the user does not get to
read, and that must never be the part saying something off-box can see
their pinned files. (Found by a test: the warning was being truncated
away.)

What is **not built**: enforcement. Nothing yet consults
`Backend::trusted` before putting pinned file content in a prompt —
[working-context](working-context.md) is where that lands, along with
the skip-list and the explicit confirm.

## Functional out of the box: the probe chain

Goulash must operate with **zero config editing** on first run. At
startup (with consent, and cached thereafter), probe in order and bind
the best available engine to the watcher role:

```
1. ollama detected (localhost:11434)          → use it, no setup at all
2. macOS + Apple Silicon + Apple Intelligence → Apple Foundation Models
                                                 (watcher tier only)
3. offer: goulash bootstrap local             → fetch/build llama.cpp +
                                                 vetted small model
4. API key present (env or providers.toml)    → cloud provider
5. none of the above                          → no-LLM tier: overlay,
                                                 history, status rows,
                                                 and the deterministic
                                                 suggestion vendors
```

Level 5 matters — and it isn't dumb: the [PTY overlay](pty-overlay.md)
and [block history](block-history.md) never require an LLM, and the
rules/history/n-gram [suggestion vendors](suggestion-vendors.md)
(thefuck-style corrections, fish-style history matches) make goulash
worth installing before any model is configured.

### Platform engines (mac-first notes)

- **Apple Foundation Models** ("it's fucking there"): zero download,
  free, on-device, private — ideal watcher-tier economics. Caveats: the
  framework API is Swift-only (needs a small shim called from
  [Rust](implementation.md)), requires a recent macOS on Apple Silicon,
  the model is small (~3B-class) with a modest context window, and its
  content-safety layer can refuse benign-but-scary-looking shell content.
  All fine for turn summarization and region labeling; never the thinker.
- **MLX**: fast on Apple Silicon but Python-centric tooling and immature
  Rust bindings — skip for now; revisit if mlx-rs matures.
- **ONNX Runtime**: cross-platform, but the LLM model/quantization
  ecosystem is far weaker than GGUF/llama.cpp — skip.
- **llama.cpp** remains the only engine exposing KV save/restore, so it
  stays the *preferred* watcher when the user runs bootstrap; ollama and
  Apple FM plugins declare `kv_save_restore: no` and the context
  assembler adapts (per-turn watcher jobs barely need persistent KV
  anyway).

## Caching stance: local-first

**Decision:** cache-efficiency matters for the **local case**; portable
cached state across cloud providers is a pipe dream and we don't chase
it. But the provider plug-in model is shaped so we *could* get there if
the landscape changes.

- **llama.cpp (the case we optimize):** KV-cache save/restore is **real**
  (`llama_state_*` / session files). A resident local model holds a
  persistent session over the whole terminal lifetime, cheaply appending
  new blocks as they happen — and can snapshot/restore state across
  goulash restarts.
- **API providers (Anthropic, OpenAI, …):** prompt caching is
  prefix-based, server-side, TTL-bounded, and opaque. A provider plugin
  *may* exploit it by keeping an append-mostly context — stable
  `[system + epoch summary]` prefix, append-only tail, compaction only at
  epoch boundaries (one deliberate cache miss each). That's an
  optimization inside the plugin, not an architectural commitment.

### Provider plug-in contract (sketch)

Each provider plugin declares its caching capabilities so the context
assembler can adapt:

```
capabilities:
  kv_save_restore:   yes | no      (llama.cpp: yes)
  prefix_cache:      yes | no      (cloud: yes, opaque)
  preferred_shape:   append-mostly | free-form
```

## Latency mechanics (field-driven)

First real answers worked but were slow. The fix ladder, cheapest first:

1. **Model residency**: ollama unloads idle models (~5 min); a cold ask
   pays full load before any token. goulash does NOT extend that — the
   server's TTL is the server's to set, and `keep_alive` is unset by
   default. A slower first ask after a gap is the price, and it is worth
   less than half an hour of someone else's VRAM.
2. **Streaming** into the bar/band: perceived latency beats total.
3. **Stable-prefix prompting**: ollama prefix-caches KV against the
   previous request per model — the append-mostly epoch shape gets
   cache hits *locally today*, exactly as the caching stance predicted.
   A rebuilt-each-time sliding window defeats it. Plus a hard prompt
   budget (raw 8×1200-char tails is heavy eval for a 2B).
4. **Background compaction** (M5): the rolling watcher continuously
   shrinks old chunks; retrieval serves summary chunks as the norm,
   raw on drill-down.
5. **Adaptive depth**: every ask/answer is timestamped in the
   transcript, so measured answer latency can tune the context budget —
   deep raw context when fast, lean on summaries when sluggish.
   Depth/correctness vs. latency becomes a closed loop, not a constant.

## Two-tier engine

| Tier | Model | Runs | Jobs |
|---|---|---|---|
| **Watcher** | small local model via llama.cpp (optional bundled copy) | resident, persistent KV | rolling cleanup / [region markers](memory-hierarchy.md), staleness checks, cheap [suggestion](../interaction/suggestion-list.md) drafts |
| **Thinker** | big API model (selectable) | on demand | `#` asides, [`##` chat](../interaction/chat-mode.md), [delegated agents](../interaction/delegated-agents.md), hard suggestions |

This keeps the always-on observation loop free (no per-token API cost, no
privacy egress for raw observation) while heavy reasoning gets a frontier
model. The local tier is **optional but bootstrappable**: never a hard
dependency of the core overlay, but goulash should be able to help set it
up (`goulash bootstrap local` — fetch/build llama.cpp or point at an
existing server, download a vetted small model, wire the watcher role).

## Privacy note

Only the watcher tier sees the raw firehose by default. What leaves the
machine for an API provider is the assembled context (summaries + recent
tail), already filtered by the
[echo-off and opaque-block invariants](opaque-blocks.md).
