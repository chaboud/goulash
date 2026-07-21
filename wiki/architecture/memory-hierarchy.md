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
