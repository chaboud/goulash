# Two-Lane Engagement: Fast Speaks, Slow Researches

**Status: partly built.** Working: the [card](working-context.md), the
`#@` pin browser, the `#?` / `?` grammar, the engagement ladder, cancel
scoping, and the slow lane itself — research dispatched alongside a fast
answer, findings amending the turn they came from, and the inset
rendering. **Not built:** slow's tools (it currently reasons with the
same context fast has, not with a toolset), lanes as separate model
instances, Right-arrow descent, and the artifact cache.

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

### `##` from *inside* chat: leave, and take a parting shot

`##` is a toggle. From outside it opens chat; from inside it closes it —
and it can carry a payload on the way out:

```
## that's great, let's work like that     say it, then back to the prompt
##? one more deep one                     ask slow, then back to the prompt
##                                        just leave
```

That is what makes a conversation end the way conversations do. You
discuss something, you reach a conclusion, and the last thing you say is
also the thing that puts you back at your shell — rather than typing a
final message, then separately hunting for the exit.

This is **additive**: Esc and Ctrl-C still leave chat exactly as they
always did. `##` with a payload is a convenience on top of the escapes,
never a replacement for them.

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

### Scheduling: fast first, current first

Interactive work always outranks research, and research for the turn the
user is *on* outranks research for a turn they have left.

Staleness here is about **lineage, not place**. A finding amends the
turn it came from; that the user has since `cd`-ed, or simply moved on,
does not make it wrong where it sits. Landing late is invisible to the
user by construction — it costs service load and nothing else.

Which leaves the question of what happens to work that goes stale
mid-flight, and both answers are defensible: **abandon** it and keep the
GPU, or service it **lazily** and let the finding land in its lineage
whenever it arrives. Lazy costs nothing in attention — an amendment for
an old turn never intrudes, by the rule above — but it does cost compute
for something nobody may look at. So it is a setting; at minimum a
[`#/debug`](../interaction/settings-and-nav.md) one, since it is exactly
the kind of thing whose right answer only shows up in use.

One hard rule at every setting: **slow never touches the proactive
commentary path.** Heckling every command turn through a research lane
is the most expensive possible version of the least important feature.

### N in a row: supersede, or parallel

A queue was the wrong instinct. **Terminals are overwhelmingly serial
interfaces** — a patient user backgrounds with Ctrl-Z or interrupts with
Ctrl-C, and neither is common. Asking three questions in a row is not a
request for three answers later; it is usually a user changing their
mind in public.

So the default is **supersede**, the same shape as the existing ask
coalescer — fast already works recent-first, and `#?` inheriting that
keeps one rule rather than two. The newest `#?` wins and the previous
one stops.

It does break chat lineage: a question you asked thirty seconds ago
silently stops being answered. **Backfill abandoned** is therefore a
setting of its own — an abandoned job can be picked up later and its
finding amended into the turn it belonged to, which costs nothing in
attention (an amendment for an old turn never intrudes) and costs
compute for something nobody may look at. Same choice as stale work,
same setting family. The
alternative is **parallel**, for someone who really did want all of
them, and either way **recent priority comes first**. A setting, at
minimum in `#/debug` — and this is an *N* case, not a two-element one,
so neither behaviour should be written as a special case for the second
job.

### Cancel: the sigil scopes it

The same rule that governs destinations governs cancellation, so there
is nothing extra to learn:

| | Stops |
|---|---|
| `#/cancel` | everything goulash has in flight |
| `#?/cancel` | research only |
| `#@/cancel` | ingest only |

**Ctrl-C is never taken.** It belongs to the shell, and reaching into
goulash's background work from it would be a surprise in the one place
surprises are least acceptable.

### Bounds on the tool loop

Slow tool-calls, so it can spin. Both caps, configurable, and reported
when hit: a **step cap** (how many tool calls in one job) and a
**wall-clock cap** (how long the whole job may take). Same discipline as
`MAX_DIGEST_ATTEMPTS` — a runaway that stops and says why beats one that
merely stops.

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

## Slow's toolset

**Not built.** Today slow is a *role* — same weights, four times the
budget, no latency pressure — reasoning over the same context fast has.
This is the shape the tools should take.

### Line protocol, not function calling

Small local models are bad at function calling and `gemma3` cannot do it
at all — but they are fine at emitting `READ: ./deploy/run.md`. Every
other capability in goulash already works this way (`CMD:`, `PIN:`,
`REMEMBER:`), so tools reuse the same parser, the same
instruct-with-a-floor discipline, and the same property that a model
which ignores the contract simply produces nothing rather than
crashing. It also keeps the toolset available on models that JSON
function calling would exclude.

### Two toolsets, because there are two jobs

**Research** answers a question. **Ingest** builds something from a pin.
They differ in the thing that matters — whether they write — so they are
separate sets rather than one list with a dangerous half.

#### Research: read to answer

| Verb | Backed by |
|---|---|
| `READ: <path>` | `read_text` — binary refusal and the 512 KB cap for free |
| `LIST: <dir>` | `WorkContext::candidates` |
| `STAT: <path>` | the freshness check |
| `FIND: <pattern>` | a bounded walk, the same shape as `collect` |

All read-only, all performed by goulash rather than the shell. Worst
case is a wasted read, which is what lets these run without an approval
prompt in front of them.

#### Ingest: read to build

Triggered by `#@`, this is the loop that decides *what a pinned thing
is* and *what to make of it*. It gets the research verbs plus:

| Verb | Effect |
|---|---|
| `CLASS: <kind>` | declare the handling class — dataset, reference, runbook, code, wiki-material |
| `CARD: <text>` | the few lines that ride beside the question |
| `DIGEST: <text>` | the compression that rides in the prefix |
| `WIKI: <name>` + body | an [LLM wiki](memory-hierarchy.md) page for this pin |
| `NOTE: <text>` | a short durable note |
| `DONE` | end the loop |

`CARD` and `DIGEST` already exist as single-shot calls; folding them
into the loop is what lets a model *read first, then write*, rather than
compressing whatever it was handed.

### Reads take paths. Writes take names.

This is the whole security shape, and it is one line:

> A read verb names a **path**. A write verb names an **artifact**.

`WIKI: eero-setup` writes to a location goulash chooses —
`.goulash/context/<content-hash>/eero-setup.md` — and there is no
argument in which the model can express a destination. Not a relative
path, not a traversal, not a symlink: the grammar has no slot for one.

Reads can afford a path because the worst case is a wasted read. Writes
cannot, so they are bounded by construction rather than by validation.
Every artifact carries provenance (source path, content hash, model,
timestamp), belongs to the pin that produced it, and is visible and
deletable in the [pin browser](working-context.md#the-pin-browser) —
which is why that browser had to exist before any of this.

### Bounds, and the floor

The loop is capped on **steps** and **wall clock** (`slow_max_steps`,
`slow_max_secs`), reported when hit rather than silently truncated.
`#?/cancel` and `#@/cancel` stop it. And the floor holds as everywhere:
if the loop produces nothing usable, the pin still has its deterministic
outline and card, so a failed ingest costs time and nothing else.

### What this actually unlocks: tree ingest done properly

Today a directory pin is walked and concatenated — crude, and capped at
64 files because that is all a single prompt can survive. With the loop,
a tree is read **file by file, by a model deciding what matters**, and
what comes out is a wiki page rather than a truncated concatenation.
That is the difference between "we pasted your repo" and "we read your
repo", and it is the case the user described from the start: *`#@ take a
look at ./images and we'll work with it`*.

It is also the expensive case — dozens of model calls, possibly an hour
— which is why checkpointing, the cancel verbs, and the chrome meter all
exist ahead of it.

### Then MCP

The four-plus-five verbs above are deliberately the smallest set that
tests whether a tool loop helps *at all* on a small local model. If a 4B
model flails at three sequential reads, MCP is a much larger surface for
the same failure — and the fix would be a bigger slow model, which loops
back to [lanes](#lanes) genuinely needing separate instances.

Once the loop is proven, MCP is the extension point, with the trust work
below.

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

**Amend by reference, never by rewrite.** The session log is
append-only and fast reads it as its own memory of the conversation, so
an amendment that silently replaced a `CMD:` line would leave fast's
context disagreeing with what the user is looking at. Turns carry
identity and the amendment names the one it revises. That is harder for
a small model to follow than a clean overwrite would be — and survivable,
which a divergence between fast's memory and the screen is not.

**Lineage is a DAG; the walk is flat.** Internally a turn can carry its
origin and its amendment. For display that graph flattens into the same
single up/down axis it always was — an *insert* into a flat lineage, not
a tree to navigate. Commands stay the anchor and the spatial rule holds.

When slow finds nothing better there is no amendment and no pair. The
turn stays single, which is what keeps the paired rendering rare enough
to mean something.

### Freeze-on-focus is now load-bearing

**The lineage never mutates under an active selection.** If the user is
focused — browsing the slot stack, or holding a selection in chat —
everything freezes. Amendments queue and land only once control returns
to the shell, and then they mutate *behind*.

This is the existing [freeze-on-focus](../interaction/suggestion-list.md)
rule, not a new one, but amend raises its stakes. Until now the freeze
protected against *insertion*: a new suggestion arriving while you
browsed would have shifted positions under your cursor. Amend introduces
**in-place mutation of an entry you may be looking at**, which is worse —
the thing you were reading becomes a different thing. Same rule, and now
it is the only thing standing between a researcher and the user's
attention.

The unfreeze points are the ones that already exist: edit, Enter,
Ctrl-C, return to neutral. Nothing new to learn, nothing new to teach.

### What slow may do when it arrives

**Show up with a recommendation. That is the whole list.** No band, no
prompt, no notification, no second voice — a command and a line, handed
to fast, landing in a lineage. Every question about slow's reach has the
same answer, and keeping it that short is what makes the rest of this
page safe.

### Solo findings need no marker

When fast has no command and slow does, the result is simply a turn with
one suggestion in it. That already happens without slow in the picture —
fast often has nothing worth vending — so it needs no new treatment, and
`#?` is just that case made explicit by the user rather than arrived at.

Which fixes what the blue actually means: **superseded, not researched.**
It marks that something was replaced, not where an answer came from. A
solo finding is the primary and wears the orange like any other. Less
chrome, and a sharper signal for the one case that needs one.

### Rendering a pair

- Fast's answer keeps the chip; the researched finding **insets below
  it** — an addition to the record, not a replacement of it.
- **Orange means selected, not "suggestion".** Whichever of the two
  Enter would pull is orange; the other is grey. That is the entire
  colour rule.
- Down walks in depth-first: fast's command, then the finding, then the
  next turn. (Right descending sideways is the other candidate.)
- **A pair only ever exists within a single turn.** That constraint is
  what stops this becoming a general tree.
- Lineage is intact, so the explanation can say what changed and why.

### Colour is selection, not category

The first version made the finding *blue* — a category marker saying
"this one came from research". Wrong axis. The user already knows a
finding is a finding, because it is indented under the answer it fills
in; what they cannot see at a glance is **what Enter does right now**.

So orange is the selection indicator and nothing else. The selected chip
is orange (`208`), everything else is grey
(`\x1b[0;97;48;5;238m`), and selecting the finding turns *fast's* chip
grey behind it. One glance answers the only question with a consequence.

That also removes a colour from the vocabulary rather than adding one,
which is the right direction for a three-row strip.

### Reaching the alternative: two mechanics, both kept

**Depth-first on Down.** The pair sits in the flat walk: Down steps into
the alternative before moving on to the next turn. Simple, needs no new
key, and costs something real — it puts a step between the user and "the
turn before this one", on an axis whose whole appeal is being
thoughtless.

**Down for turns, Right to descend.** Down keeps meaning *older*, and
Right descends into the researched alternative where one exists. The
second dimension appears only where something was actually superseded,
so the primary axis stays clean. Rendering: the alternative **overlays
the dim question row, indented, in its own colour**, and the question
truncates to a few characters and an ellipsis — it was not doing much
work there anyway.

Both stay on the table; the second is the one to build first, with
depth-first-on-Down as the fallback if the gate below turns out to feel
like a trick.

**Fast is primary.** The researched finding *fills in* beneath it rather
than taking its place. That keeps the stack reading as a clean
transcript of what actually happened, instead of a bait-and-switch where
the thing you were about to pull becomes a different thing — and
freeze-on-focus makes it simple besides, since nothing mutates while the
user is looking at it.

#### The hazard: Right is already spoken for

A pulled suggestion is **text on the shell line, and the user can arrow
around it and edit it**. Left and Right are ordinary cursor movement the
moment anything is on the buffer — and browsing *is* pasting in the
[Down-arrow protocol](../interaction/down-arrow-protocol.md), so there is
always something there.

This is the same shape as the gate problem Down already solved, and the
same machinery answers it: the zsh adapter's wrapped `bracketed-paste`
widget records **exactly** what goulash pasted. So Right can claim the
gesture only when all three hold — the buffer is byte-identical to what
goulash pasted, the cursor is at the end of it, and this slot actually
has an alternative. One edited character and Right is a cursor key
again, with no drift and no guessing.

That narrow window is the whole reason this might work. It is also the
reason it might not, and why the simpler mechanic stays.

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

- **A card** — a few lines immediately before the question: what this
  is, the key invocations, the one constraint that makes commands wrong.
  Cheap to re-emit at the tail, lands inside the SWA window.
- **The bulk** — stays in the stable prefix, cache-warm, for detail.

The card also gives classification something concrete to produce: "what
kind of document is this" is vague, "write the card for this document"
is a task a small model does well and a human can eyeball.

**Built** — see [working-context](working-context.md). 400 characters
shared across pins, newest first, with a deterministic floor (title plus
the most invocation-like lines) so a card exists before any model
answers. The remaining question is the one this was meant to settle, and
only a real session can: **does the near-question card measurably beat
the prefix-only digest?** If not, the SWA reasoning is wrong and the
slow lane's whole premise needs re-examining.

## The chrome has to give something up

Two lanes and a research indicator do not fit alongside what the chip
already carries. The order things go, worst-value first:

1. **`goulash` itself goes.** It is the one field that tells the user
   something they already know — they launched it, and they know how to
   type `exit`. Self-branding that costs characters on every row is the
   easiest thing in the chip to lose.
2. **The pin degrades in steps**: full label → a short slug (written by
   the same pass that writes the card, which is already reading the
   document) → bare `@`. Even the last step is worth keeping, because
   *whether anything is pinned* changes how the model behaves and the
   user should never have to wonder.
3. **State becomes a glyph.** A tiny ASCII bar climbing to a full block
   (`▁▂▃▄▅▆▇█`) says progress in one cell where `50%` takes three, and it
   reads as motion rather than arithmetic.

## The crash fuse learns about lanes

[`state.rs`](../src/state.rs) marks the dangerous window around a model
load so an unclean death can distrust the model that caused it. With two
lanes there are two loads, and an unlaned mark cannot tell which one to
blame — the likely failure is a big slow model taking the machine down
and a small fast model getting distrusted for it. The mark carries the
lane.

## Budget, and the ceiling

Two consequences to accept knowingly:

- **Shares.** Of the prompt budget derived from `num_ctx`: session log
  **45%**, working context **40%**, [memories](agent-memory.md) **15%**,
  with the agreed eviction order when over — log trims first (most
  regenerable), working context degrades a tier, memories evict last
  (smallest and most deliberately curated).

  Slow's relayed material turns out **not** to be a percentage claimant
  at all: it is one command and one line, bounded and tiny, riding in
  the volatile suffix like the cards. So there are three proportional
  claimants and two small fixed suffix allowances, rather than the five
  competing shares it first looked like.
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
4. ~~**The `#@` pin browser**~~ **built** — bare `#@` opens the menu
   primitive, with a `+ pin a file …` compose row and arm-then-confirm
   drop. Slot-associated assets are only safe because they are visible
   and deletable; shipping a writer first would fill `.goulash` with
   things nobody can see. Same surface [`#/study`](../product/build-plan.md)
   needs — built once, used for both.
5. MCP, skills, VLM — after (3) answers that.

## Open questions

1. **Can Right actually be claimed?** The gate is narrow by
   construction (unedited buffer, cursor at end, alternative exists).
   Whether that window is wide enough to feel like an affordance rather
   than a trick is a hands-on question — build it, try it, fall back to
   depth-first-on-Down if it does not land.
2. **Card selection per question.** Cards stay per-pin and cached; which
   pins get one is chosen per ask by **word overlap with the question**
   — ask about deploying and the deploy runbook's card rides along while
   an unrelated vendor reference's does not. Plain string matching, no
   model call, no cache churn. Whether it picks well enough in practice
   is the open part.

Settled in conversation, recorded so they are not re-opened: `##?` is an
ingress rather than a mode, and `##` from inside chat leaves *with* a
payload; concurrent research supersedes by default (terminals are
serial), recent first; the sigil scopes cancellation and Ctrl-C is never
taken; the crash-fuse mark carries its lane; the chrome sheds `goulash`
first and degrades the pin to a slug then a bare `@`; amendments insert at their origin, by
reference, and never jump the queue; a pair renders only within one turn
and the blue means *superseded*, not *researched*; solo findings need no
marker; slow gets pin **paths** and reads sources itself, so the
card/digest layer exists purely for fast; `#?` with slow off answers via
fast and says so; retention is a content-hashed cache that survives
unpinning ([working-context](working-context.md)); classification is the
model's call except for `directive`.

## Related

- [working-context](working-context.md) — `#@`, tiers, budgets, the digest queue
- [llm-engine](llm-engine.md) — the worker, prefix caching, why prefill is the devil
- [model-capabilities](model-capabilities.md) — per-model dialects; the slow lane will need this more, not less
- [suggestion-vendors](suggestion-vendors.md) — the protocol slow emits into
- [positioning](../product/positioning.md) — coach, not agent overlord
