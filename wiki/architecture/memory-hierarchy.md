# Memory Hierarchy & Context Retrieval

What happens at the prompt will, at times, **wildly outstrip LLM
capacity**. The answer is a hierarchical memory over
[block history](block-history.md), with lossy *attention* on top of
lossless *capture*.

## The two-layer deal

- **Capture is lossless.** The bottom of the tree is always the pure
  history of raw data (subject only to explicit size caps for pathological
  output — a block that emits gigabytes keeps head/tail plus an elision
  marker pointing at spooled raw). Raw is never edited or deleted by any
  LLM process.
- **Attention is lossy on purpose.** What the live model *sees* is a
  "UDP most-recent-ish" view: the recent tail verbatim, with a scrollback
  window to find bearings, and tools to fully explore the rest on demand.

## The tree

> **Promotion between levels is the unit of background work, and it is
> what bounds it** — each level is geometrically smaller than the one
> below, so there is only ever a finite amount of cooking left to do.
> See [ambient-research](ambient-research.md).



```
region:   "cleanup in ~/src/junk"     "built libfoo"      "debugged CI issue"
             │                            │                     │
markers:  cmd cmd aside cmd           cmd cmd cmd sugg      cmd ## chat cmd
             │                            │                     │
leaves:   raw blocks (pure history: bytes, exit codes, timings)
```

- **Commands and user interactions are natural timeline markers** — they
  already delimit blocks. **Turn-based summarization is the baseline**:
  the zsh/bash command boundary (command vs. output) is the simple,
  honest fencepost, and summarization can operate purely on those turns
  with no LLM-invented structure at all.
- **A rolling cleanup LLM may set markers and signify regions**: an async
  background pass (cheap [local model](llm-engine.md) territory) that
  groups spans into labeled regions — "did cleanup here, built source
  over there, dealt with an issue there."
- **Markers are summarizer *hints*, not restrictions.** A later
  summarization pass may honor, ignore, or redraw them. Binding
  retrieval hard to LLM-drawn boundaries could turn toxic (a bad region
  boundary would distort everything summarized within it); the
  turn-based fenceposts are always there to fall back on.
- Regions can nest; the hierarchy is prompt/block-based, not
  time-sliced.

## Retrieval: logarithmic ramp-off

Context assembly for any LLM call walks the tree newest-to-oldest:

```
now ──────────────────────────────────────────► past
raw verbatim │ block summaries │ region summaries │ epoch summary
   (recent)  │                 │                   │   (ancient)
```

Roughly: full raw for the recent window, then increasingly coarse
summaries at logarithmically growing spans. Keeps context bounded no
matter how long the session runs.

### The structure: fixed-count tiers, overflow promotes

Concretely, each tier holds a **fixed number of items**, and overflow is
what drives compression:

```
tier 0    last ~20 blocks      raw, verbatim
tier 1    next ~20 groups      k blocks compressed into one group
tier 2    next ~20 areas       k groups compressed into one area
tier 3    …                    eras
```

When tier N is full, its oldest k items are promoted into a single item
in tier N+1. Three things fall out of that, and they are the reason to
prefer it to a decay curve:

- **Storage and prompt cost are bounded by construction** — a fixed
  count per tier, with each tier's items geometrically denser than the
  one below. Bytes fall off with depth, which is exactly the ramp.
- **The horizon problem dissolves.** There is no "decay with age"
  heuristic to tune; there is overflow. Age is an emergent property of
  position, not an input.
- **It is the same machine as the throttle.** Promotion is the unit of
  background work ([ambient-research](ambient-research.md)), and the
  promotion rate at tier N+1 is 1/k the rate at tier N. The structure
  that bounds the *context* is the structure that bounds the *work*.

### Count for structure, wall clock for meaning

The tempting model is a live-monitoring one — 10s / 1m / 1h / 1d / 1w /
1y buckets, RRD-style. It is the wrong primary axis here, because
terminal work is **bursty**: two hundred commands in an hour, then
nothing for two days. Pure wall-clock tiers dump that whole burst into
one bucket and lose it to a single summary; pure count tiers leave
three-week-old blocks sitting at full fidelity in tier 0.

So: **tier by count, cut on gaps.** A tier's contents also break at a
large idle gap, because a gap is a natural boundary in both senses — the
user went away, *and* they almost certainly came back to something else.
That yields groups that mean something ("the hour on the CI failure")
rather than arbitrary ones ("blocks 21–40"), which is the same argument
the turn-based fenceposts make above.

Every summary then carries its **wall-clock span** as metadata, so
retrieval can still be temporal — "what was I doing in that repo last
Tuesday" — without the structure being time-sliced.

### This is also the cache layout

Log ramp-off rewrites older context, which fights prefix caching (see
below). Fixed-count tiers make that *better* rather than worse, because
compaction now happens only on overflow — and overflow at tier N+1 is
k times rarer than at tier N. So:

- **tier 0 churns constantly** — volatile, belongs at the end, near the
  question
- **tiers 2 and up are nearly static** — cache-warm, belongs in the
  stable prefix

Which is the layout the engine already uses for memories, pins and cards
([llm-engine](llm-engine.md)). **The level-of-detail structure and the
cache structure want the same arrangement**, so there is nothing to
trade off.

## Adaptive retrieval depth

The ramp-off need not be a fixed curve: answer latency is measured on
every ask (transcript timestamps), so the context budget can adapt —
more raw verbatim when the engine is fast, harder lean on summaries
when it's slow. Depth/correctness vs. latency is a tunable closed loop
per machine and per model.

## The other axis: a flat durable store

Everything above decays, because everything above is about **what
happened**. There is a second, orthogonal store for **what is true** —
and it does not decay, is not automatic, and stays deliberately small.

| | temporal tiers | durable store |
|---|---|---|
| holds | what happened | what is true |
| shape | log-compressed, tiered | flat |
| growth | automatic | deliberate |
| size | large, bounded by tiers | **small, and that is the point** |
| lives in | the session, then the project's artifacts | `~/.goulash` |

Two kinds of thing belong in it, and the second is the interesting one:

- **About the user** — their tools, their idioms, the fixes they keep
  re-deriving. This is `memory.toml` today ([agent-memory](agent-memory.md)).
- **About us, in relation to them** — which suggestions get pulled and
  which get ignored, which models actually work here, what this person
  finds annoying. That is self-knowledge, and it is what makes a coach
  improve rather than merely persist.

Together they are the *relationship*, which is why they follow the user
across machines while project content stays with its tree
([ambient-research](ambient-research.md#two-stores-and-the-rule-between-them)).

### Graduation, and why the bar has to be high

The decaying store feeds the permanent one. A fact that keeps being true
as the tiers compress around it has earned a place that does not decay:

- observed repeatedly, across sessions rather than within one
- asserted explicitly (a `REMEMBER:` line, or the user saying so)
- **survived** — still true after the span it came from has been
  compressed twice

The bar matters because a durable store that accepts everything is not a
durable store, it is another log with worse retrieval. Small is the
feature.

### A durable store with no invalidation path is a liability

This is the failure mode worth designing against up front, because it
arrives quietly. A stale durable fact is **worse than no fact**: it is
asserted with confidence, it rides in the stable prefix on every single
ask, and nothing in the temporal tiers will contradict it once the
evidence has aged out.

"They use npm" survives the switch to pnpm forever unless something
actively kills it. So durable entries carry a **last-confirmed**
timestamp and are re-validated rather than merely retained — a fact
nothing has re-confirmed in a long time is a candidate for eviction, not
a permanent resident. The anti-poisoning invariants below protect
summaries from hiding truth; this protects durable facts from outliving
it.

## Anti-poisoning invariants

Summaries are the retrieval index, so a hallucinated summary can *hide*
truth. Therefore:

1. **Raw is truth; summaries are hints.** Nothing at the leaf level is
   ever rewritten.
2. **Every marker/summary carries pointers** to the exact raw span it
   covers.
3. **Drill-down tools are always available** to the interactive LLM:
   expand a region, fetch a raw range, grep the raw history. If a summary
   smells wrong, the model (or user) can check it against the leaves.

## Interaction with prompt caching

Log-ramp-off naturally *rewrites* older context, which fights
prefix-based LLM caching. Resolution — compact only at epoch boundaries,
keep the prefix append-mostly between them — is covered in
[llm-engine.md](llm-engine.md).
