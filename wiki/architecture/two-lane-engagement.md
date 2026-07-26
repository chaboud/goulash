# Two-Lane Engagement: Fast Speaks, Slow Researches

**Status: design. Nothing here is built.** The pieces it builds on —
[`#@` working context](working-context.md), the digest tier, the
[suggestion protocol](suggestion-vendors.md) — are.

Today goulash has one engagement: you type `#`, a fast model answers from
the [stable prefix](llm-engine.md), a command lands in the slot stack.
Sub-second, cache-warm, never in your way.

This adds a second, slower engagement that **never gets the microphone**.

```
        you ──#──▶ ┌──────┐ ──── answer + CMD ────▶ band / slot stack
                   │ FAST │
        you ──#?─▶ └──────┘ ◀─── suggestion ───┐
                       │                        │
                       └──── dispatch ────▶ ┌──────┐
                                            │ SLOW │ tools · skills · VLM
                                            └──────┘ writes .goulash assets
```

## The invariant: we only ever hear from fast

Everyone builds the smart model as the front door with a fast router in
front of it. This is the other way round. **Fast is the only voice.**
Slow is a contributor of suggestions, and its output reaches the user
only by fast choosing to relay it.

That is not a hedge against slow being wrong. It is three things at once:

- **One register.** If slow's text sometimes appeared verbatim and
  sometimes got adapted, the user would be hearing two voices with no
  way to tell which. Inconsistency is worse than occasional flattening.
- **One holistic reasoner.** Fast has the session log, the memories, the
  working context, and the last thirty seconds. Slow has the artifact.
  Only one of those is the picture the user is living in.
- **The autonomy rule, one level up.** Goulash suggests and the user
  runs. Now slow suggests and fast relays. Slow structurally cannot
  seize the band or the slot stack, so adding an agent does not cost the
  flow guarantee.

### Instruct, with a floor. Never constrain.

Fast **is** the consistency enforcement — that is its job, not a risk to
be mitigated around. So fast is *told* to relay slow's findings
faithfully; nothing forces it to.

This is the pattern already running everywhere in the codebase and it
should not be broken here of all places: `CMD:` is instructed and
parsed, not forced. So are `PIN:` and `REMEMBER:`. The digest tier asks
for a length and falls back to the [deterministic
outline](working-context.md) when the model ignores it — a floor beneath
failure, not a cage around the choice.

A rejected design, recorded because it is the tempting one: routing a
follow-up to slow *programmatically* when a researched slot is selected.
Spatial, deterministic, and wrong — it replaces fast's judgement instead
of informing it. **Provenance is given to fast as a fact** ("this came
from the researcher; its reasoning is at handle N"), and fast decides.

The corollary is a discipline on **slow**, not on fast: slow's output
must arrive **pre-shaped** — a command, one line in the house register,
reasoning retained separately. A researcher that hands over finished
goods makes faithful relay the path of least resistance. Handing fast
eight hundred words and hoping is where drift would actually enter.

## The grammar: the sigil chooses who works

One rule covers the whole surface: **the second character is a
destination, and you always hear fast.** `#?` is not a bypass — slow
researches, fast still relays — so it slots into the existing pattern
rather than being an exception to it.

| At the prompt | In chat | Destination |
|---|---|---|
| `# …` | *(bare)* | fast |
| `#? …` | `? …` | slow researches, fast relays |
| `#@ …` | `@ …` | [working context](working-context.md) |
| `#/ …` | `/ …` | goulash itself |

Chat drops the `#` because you are already inside. The selector is
identical in both columns, so there is nothing new to learn.

### `##?` is an ingress, not a mode

`##?` means "open chat, first question goes to slow" — not "open a chat
that stays slow". Nothing else in goulash is modal except chat itself;
`#/` and `#@` are per-line. A sticky lane would need an unset gesture, an
indicator, and a rule for what a bare line means: three concepts to save
one keystroke.

And it is a keystroke worth paying. A slow turn costs real seconds and
real GPU, so typing `?` each time keeps it deliberate rather than
ambient — the same reason `#@/path` is explicit. In a sticky slow chat,
"thanks, now rename that file" burns a research cycle on nothing.

If research sessions turn out to be common, stickiness is a small
addition and the chat chip has room to show it. Cheap to add, awkward to
remove.

### Bare `#?` asks for help without hardcoding help

`?` reads as "help" to a lot of people, and that instinct should be
rewarded rather than corrected. A bare `#?` does not print a canned
syntax card — it tells the model *the user typed a bare `?` and may not
know the syntax; they may be asking for help*, and the answer comes back
in the one voice, in context. Instruct, don't hardcode.

Floor, as everywhere: with no engine bound there is no answer, so a
plain syntax line stands in.

Other collisions, for the record. A chat line genuinely starting with
`?` routes to slow — escapable with `\?`, same as everywhere. Spanish is
unaffected: `¿cómo…` opens with `¿`. And `#?` is shell-safe by the same
specification as `#@` ([shell-integration](shell-integration.md)):
comments are discarded at tokenization, so the glob character never
globs.

## When slow engages: a ladder, not a toggle

| Value | Slow runs | Cost |
|---|---|---|
| `off` | never | zero |
| `manual` | only `#?` / `?` | exactly what was asked for |
| **`ingest`** *(default)* | + on `#@` — classify, card, wiki | bounded, and **the user triggered it by pinning** |
| `volunteer` | + on ordinary `#` asks | unbounded — fires on everything typed |

`ingest` versus `volunteer` is the line that matters, and `ingest` is
the default because it is the original promise exactly: pin something,
lose no speed in general operation, gain more thoughtful options. The
work is bounded and user-triggered. `volunteer` is where the GPU burns
continuously for questions that already had a good answer — opt-in, and
honest about why.

One hard rule at every setting: **slow never touches the proactive
commentary path.** Heckling every command turn through a research lane
is the most expensive possible version of the least important feature.

## Lanes

A lane is a role, not necessarily a model — "billing address and mailing
address", which may be the same address. Slow is fast with a bigger
budget, no latency pressure, and tools.

The problem the lanes exist to solve is concrete: **ollama caches
against the previous request**. Fast's whole latency story is a
byte-stable prefix. A slow call interleaved on the same context evicts
it, so the next `#` ask pays full prefill — the devil we designed the
prefix around. Then fast evicts slow's.

It is KV duplication, not weights. The runtimes differ on **affinity** —
whether you can say *this request goes to that slot*:

| Runtime | Parallelism | Lane affinity | Notes |
|---|---|---|---|
| **llama.cpp** | `--parallel N` | **`id_slot` per request** (`-1` = any idle) | `cache_prompt` reuses that slot's prefix; explicit `--parallel` splits `--ctx-size` evenly across slots |
| **ollama** | `OLLAMA_NUM_PARALLEL` (auto 4 or 1) | none exposed | RAM scales `NUM_PARALLEL × CONTEXT_LENGTH` |
| **vLLM** | continuous batching, PagedAttention | n/a — automatic prefix caching | ~16–20× ollama concurrency; NVIDIA-oriented, heavy |
| **vLLM-MLX / vMLX** | continuous batching | n/a | Apple Silicon; prefix caching, paged KV, MCP tool integration |
| **LM Studio** | llama.cpp parallel slots, extended to MLX | inherits llama.cpp's | `llmster` headless daemon |

Two findings worth keeping:

- **Only llama.cpp lets us pin a lane.** On ollama a fast ask can land
  on the slot slow just warmed, so the collision returns
  probabilistically rather than being designed out.
- **`--kv-unified` is better than duplication.** A shared pool lets
  several sequence ids point at the *same* KV entries for a common
  prefix. If fast and slow share a preamble that is one cache referenced
  twice, not two caches. llama.cpp only.

### Do not build the abstraction before the measurement

The elaborate version of this is probably unnecessary. **Ollama slots on
one model, or simply two models separately cached, may just do the job.**
Both are a config change rather than an architecture. Measure the actual
cache damage on a real session before writing a lane scheduler — the
fallback (`idle-backoff`: slow runs only after the shell has been quiet,
yields at a prompt turn) is universal, needs no runtime support, and
reuses the queue discipline the [digest tier](working-context.md)
already has.

Sketch, if it turns out to be needed:

```toml
[lane.fast]
model = "gemma3:4b"

[lane.slow]
model = "qwen3:8b"      # omit to share fast's weights
slot = 1                # llama.cpp only
strategy = "instance"   # instance | idle-backoff
```

Whatever the shape, goulash should **state the memory cost** when a
second lane is enabled rather than letting someone find it by OOM.

One thing that pushes toward two models regardless: **small models are
bad at multi-step tool loops**, and gemma3 does not do function calling
at all. "Same model for most users" likely holds for `#?` deep answers
and breaks the moment MCP tools are in play.

## Ingest becomes classification

`#@` stops being only compression. The bare (non-`/`) form hands the
artifact to a longer-running ingest loop with tools, which decides what
to *build* from it: a digest, an [LLM wiki](memory-hierarchy.md), a
skill, an index — or that it is a dataset best summarised statistically.
The user can steer in words: `#@ take a look at ./images and we'll work
with it`.

Same two-dialect split as [today](working-context.md): `#@/…` is the
pure deterministic API, bare `#@` is the model's judgement.

**One carve-out.** "Directive" is not a document type, it is a
*privilege*. Dataset / reference / runbook / code / wiki-material all
describe how to *handle* content, and being wrong costs a worse digest.
Directive describes how much the content *governs goulash*, and it is
the only class where a README in a repo you happened to `cd` into can
promote itself into your instructions. The loop classifies freely across
handling classes; directive stays an explicit user act (`#@/directive`),
which the model may *suggest* and never assume. One line of policy, and
the injection story stays boring.

## Capabilities: principled interfaces, not a hand-rolled taxonomy

Slow's reach is **MCP**, plus a skills engine, plus a VLM for images.
This is better than goulash inventing a tool vocabulary: MCP is an
enumerable capability set with a discovery protocol, and the trust
boundary becomes "which servers did you install", which users already
understand.

Two things it does not solve for free:

- **MCP servers are not read-only.** A filesystem server has write
  tools. "Slow is read-only" becomes "slow is as read-only as your
  servers". `readOnlyHint` exists in tool annotations but is advisory,
  so enforcement is goulash's: default-deny anything not annotated
  read-only, explicit per-tool allowlist to go further. The guarantee
  has to stay stateable in one sentence.
- **Skills split in two.** Markdown plus resources is context, and safe.
  Skills that ship scripts are execution, and that is the line goulash
  has held since the beginning. Decide which half ships before the word
  enters the docs.

The **writable** exception stays narrow: slow writes `.goulash` assets
only — wikis, notes, digests, cards. Everything carries provenance
(source path, mtime, model, timestamp), and **a derived asset never
outranks its source**. Without that rule slow reads its own wiki,
refines it, writes it back, and drifts with no ground truth.

### Artifacts belong to a slot

Goulash gets to decide what to build — that is its own space. What the
user gets is not an approval prompt but **visibility and
reversibility**, which is the trade the [memory store](agent-memory.md)
already makes: browse it, see who wrote it, delete it with a confirm.

So every derived artifact **hangs off the `#@` slot that produced it**,
and the lifecycle falls out for free: drop the pin and its card, digest
and wiki go with it; LRU-evict the slot and its artifacts go too. No
orphans in `.goulash`, and no separate reaper to write.

The concrete consequence: **`#@` needs the browser treatment `#/memory`
got.** A notice line listing 50 slots is unusable — the exact argument
that turned the memory list into a menu. Bare `#@` opens a pin browser:
path, tier, freshness, and the artifacts nested under each slot, with
arm-then-confirm delete. The [menu primitive](../interaction/settings-and-nav.md)
already exists; this is another instance of it.

## The answer shape

Every researched finding is three layers, and they map onto surfaces
that already exist:

| Layer | Surface | Who sees it |
|---|---|---|
| The command | slot stack | pulled with Down, like any suggestion |
| One line, fast-answer-sized | band | relayed by fast |
| Full reasoned answer | retained in slow's transcript | on request; the receipt |

Fast may **adapt** the line — substitute the real filename, match the
user's shell — but adapting is not re-summarising. The retained
reasoning exists so the user can go look and so slow can answer an
alteration with its own context intact. It is a receipt, not an audit
mechanism.

## Amend: the one mechanism, wearing two hats

Slow never appends. **It amends what was.**

A finding always has a *lineage of origin* — the turn it came from — and
it lands there, in place, rather than at the top of the stack. `?` in
chat and `volunteer` on ordinary asks are therefore not two behaviours
to design: they are one mechanism seen from two places. Fast answered;
slow researched; fast relays the improvement into the turn that asked.

**If the user has moved on, we move on.** An amendment for a turn that
is no longer current does not jump the queue, does not interrupt, and
does not re-take the band. It is simply *there* when the user browses
back. This is the flow guarantee surviving a feature that could easily
have broken it: nothing arriving late can ever seize attention.

That also disposes of the awkward case — an amendment landing after the
user already pulled and ran the original. It cannot mislead, because the
superseded command stays visible in the lineage; it is unambiguous which
one was run.

**Lineage is a DAG; the walk is flat.** Internally a turn can carry its
origin and its amendment. For display that graph flattens into the same
single up/down axis it always was — an *insert* into a flat lineage, not
a tree to navigate. Commands stay the anchor and the spatial rule holds.

When slow finds nothing better there is no amendment and no pair. The
turn stays single, which is what keeps the paired rendering rare enough
to mean something.

### Rendering a pair

- The researched command takes the chip, in the usual
  [orange](status-rows.md). Fast's original **insets below it**, in
  blue — secondary reads as secondary at a glance.
- Down walks into the inset before moving on: depth-first within a turn,
  then the next turn. One axis, still.
- **A pair only ever exists within a single turn.** That constraint is
  what stops this becoming a general tree.
- Lineage is intact, so the explanation can say what changed and why.

The blue: `\x1b[0;97;48;5;25m` — white on 256-colour 25, a deep royal
blue. It recedes against the orange (208) instead of competing with it,
the way brighter 26/33 would, and white-on-25 carries the contrast that
orange gets from black text. Adjustable once it has been seen on a real
terminal next to the orange.

## `#?` — the deliberate door, and it never blocks

`#?` asks slow directly. Fast does not also answer the same question
(two competing answers is the failure mode), but **the shell never
waits**: the band says `researching …`, you keep working, the finding
arrives as a suggestion when it arrives.

Never blocking is a stated invariant here, not an intention — a `#?`
that stalls a prompt turn recreates the exact go-to-the-browser-and-wait
loop goulash exists to kill. It wants a test, and a visible in-flight
indicator in the [chrome](status-rows.md), the way the ingest meter
works.

## Retention

`.goulash` stores **derived artifacts only** — digests, outlines, cards,
wikis, classifications — keyed by path + mtime + size. Never copies of
source. Raw is always re-readable from disk; it is the *cook* that was
expensive, staleness becomes structural (key changes → miss → re-cook),
and nothing of the user's is duplicated. That turns a 500 MB budget into
something like **10 MB**.

Cap slots (~50 `#@` entries) and total derived bytes, evict by last use,
never evict something currently pinned. Both user-configurable. This is
the same store [`#/study`](../product/build-plan.md) needs, so they
should share one rather than growing two.

## SWA, and the card

Sliding-window attention punishes distant context in a way that is
sharper than "it is slower": tokens outside the window reach the model
only through the few full-attention layers, so a pinned block is
*differentially less effective* the further back it sits. That collides
with the current placement, which puts working context **above** the
session log for cache stability — as far from the question as possible.

Cache-optimal and attention-optimal point in opposite directions. The
way out is to stop treating a pin as one blob:

- **A card** — five to ten lines immediately before the question: what
  this is, the three key invocations, the one constraint that makes
  commands wrong. Cheap to re-emit at the tail, lands inside the SWA
  window.
- **The bulk** — stays in the stable prefix, cache-warm, for detail.

The card also gives classification something concrete to produce: "what
kind of document is this" is vague, "write the card for this document"
is a task a small model does well and a human can eyeball.

## Budget, and the ceiling

Two consequences to accept knowingly:

- Slow's material is a **fourth claimant** on fast's prompt budget,
  alongside working context, [memories](agent-memory.md) and the session
  log — and the only one that arrives unpredictably. The shares and
  eviction order in [working-context](working-context.md) need an entry
  for it.
- **Fast is the expressiveness ceiling.** Slow can do excellent work and
  fast is what the user hears. That is not an argument for bypassing
  fast; it is an argument that fast should not be the smallest thing
  that runs, and that the full answer stays retrievable.

## Build order

1. **The card.** Smallest possible step, tests the SWA thesis, needs no
   new architecture — the existing digest path can produce it. If a
   well-placed eight-line card does not measurably beat a prefix-only
   digest, everything downstream is being built on sand.
2. **Lanes as config, plus idle-backoff.** No slow model yet. Prove the
   plumbing and measure the real cache damage on ollama before deciding
   whether slots or a second model are needed at all.
3. **Slow as a role on the same weights** — bigger budget, no latency
   pressure, emitting into the existing suggestion protocol with
   provenance handed to fast as a fact. This is the mediation inversion
   in its cheapest form, and it answers the question the whole design
   rests on: **is a researched suggestion arriving forty seconds late
   delightful, or annoying?** Nobody knows yet, including this page.
4. **The `#@` pin browser**, before anything writes a durable artifact.
   Slot-associated assets are only safe because they are visible and
   deletable; shipping the writer first would fill `.goulash` with
   things nobody can see. Same unmet prerequisite
   [`#/study`](../product/build-plan.md) has — build the surface once,
   use it for both.
5. MCP, skills, VLM — after (3) answers that.

## Open questions

1. **Where does the retained reasoning surface?** [`##` chat](../interaction/chat-mode.md)
   is the natural home, but a researched slot might want its own
   expansion gesture.
2. **How does a follow-up reach slow's transcript?** Fast decides — but
   fast needs a handle to name, and slow needs its thread to still
   exist. Threaded state is new; the session log is flat and
   append-only.
3. **What cancels a research job**, and does `#@/cancel` cover it or
   does slow need its own?
4. **Is a card per pin, or a card per pin *per question*?** The second
   is better and destroys the cache.
5. **Does a stale amendment still amend?** Suggestions clear on cwd
   change today. A finding landing after a `cd` should be dropped or
   marked by that same rule rather than inventing a new one — but
   "dropped" throws away work that may still be right.

Settled in conversation, recorded so they are not re-opened: `##?` is an
ingress rather than a mode; amendments insert at their origin and never
jump the queue; a pair renders only within one turn; artifacts belong to
their slot; classification is the model's call except for `directive`.

## Related

- [working-context](working-context.md) — `#@`, tiers, budgets, the digest queue
- [llm-engine](llm-engine.md) — the worker, prefix caching, why prefill is the devil
- [model-capabilities](model-capabilities.md) — per-model dialects; the slow lane will need this more, not less
- [suggestion-vendors](suggestion-vendors.md) — the protocol slow emits into
- [positioning](../product/positioning.md) — coach, not agent overlord
