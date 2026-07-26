# `#@` Working Context: Pinned Files as Near-Tool-Use

**Status: v1 built** (`src/context.rs`). This page is both the design
and the record of what shipped; sections marked *(not yet)* are the
parts still ahead. See [state-of-play](../product/state-of-play.md).

`#@ <path>` pins a file (or a directory) into a **working context** that
rides in the prompt's [stable prefix](llm-engine.md), next to
[pinned memories](agent-memory.md). Instructions, a guide, a runbook —
whatever the user says matters right now.

```
#@/path commandRef.md     pin a file          — deterministic, no model
#@/path ./deploy/         pin a tree          — walked, capped, outlined
#@/path                   (blank) unset       — stop anchoring on anything
#@/unset                  drop every pin
#@/drop 2                 unpin one
#@/list                   what's pinned, with sizes and freshness
#@                        same as /list
#@ <anything else>        hand it to the model (below)
```

## Two dialects, and why the pure one earns its keep

The `/` forms are a **deterministic path API**: no model, no ambiguity.
That buys three things at once. They work with no engine bound. They are
exactly testable, which the natural-language form can never be. And —
because `#@/path ref.md` is an ordinary shell comment line that goulash
intercepts — **the model can suggest one**:

```
CMD: #@/path commandRef.md
```

That arrives as a normal pullable suggestion: Down puts it on your
prompt, Enter runs it, goulash reads the file. The model asking for
context is just another suggestion the user accepts or ignores — no new
protocol, no new trust boundary, and the same "goulash suggests, you
run" rule as everything else. The preamble tells the model the verb
exists.

## `#@` is LLM-mediated, not a path parser

A literal path resolves directly (fast path, no model call). Anything
else is **handed to the model**, which decides what to do with it:

```
#@ eero.md                                        → pin that file
#@ can we reference the synology guide in my ~/?  → find it, pin it
#@ let's stop using this @ and go back to blank   → clear the pins
```

The model answers in the same line protocol that already carries
`CMD:`/`REMEMBER:` — `PIN: <path>`, `UNPIN: <id>`, `PINCLEAR` — and the
[chrome](status-rows.md) reports **what it chose**, which doubles as
the audit trail. Resolution needs candidates, so the model gets a
cheap directory listing to work from.

**The capability granted is read-only, and goulash performs it, not
the shell**: list a directory, read a file. No execution, ever — the
model never gets a shell, and a mis-resolved pin costs a wasted read,
not a side effect. That is the whole reason this can be
natural-language without an approval prompt in front of it.

## Why it's the highest-leverage feature on the list

The killer case is **a vendor-authored command guide**: drop a
`commandRef.md` next to a proprietary CLI, and the model can suggest
correct invocations for a tool it has never seen. That is *kind of*
tool use — no function-calling protocol, no schema negotiation, just
the reference material in front of the model when it writes a
[`CMD:` line](suggestion-vendors.md). It turns goulash from "knows
common Unix" into "knows **your** tools" with a file a team can check
into their repo.

**Nothing about the autonomy model changes**: goulash still only ever
*suggests* commands, pulled and run by the user
([chat-mode](../interaction/chat-mode.md) keeps it pure). The pin
changes what it knows to suggest, not who runs it.

## Visible in the chrome

A pin is state the user must be able to see at a glance, so the
[status chrome](status-rows.md) carries it: the active `@` (basename,
`+N` when several) and, while cooking, a **percentage meter** — ingest
is async and can take real time on a tree or a large digest, and a
silent multi-second cook is exactly the "am I frozen?" failure we hit
with model loads. Done cooking, the meter collapses back to the plain
`@ name` marker.

## Ingest tiers (the budget is the design) *(digest tier: not yet)*

The stable prefix is the KV-cache asset; a fat pin destroys the
latency work. So ingest is **compaction, not concatenation**, chosen by
size against a hard `context_files_max_chars` budget:

| Tier | When | What lands in the prompt | Built |
|---|---|---|---|
| **Verbatim** | fits its share | the text as-is | yes |
| **Outline** | over its share | structure kept, prose dropped — headings, fences, tables, flag-bearing lines | yes, deterministic |
| **Digest** | large documents | **LLM compression**, biased toward commands and invariants | not yet |

v1 ships Verbatim and a **deterministic** Outline. That ordering is
deliberate rather than lazy: the extraction is instant, it works with no
engine bound, and it gives a pin something useful to say the moment it
is made — which is the same "useful while incomplete" rule the tree
checkpointing follows, applied to one file. The LLM digest is strictly
better for a large document and is the next tier; it swaps in *behind* a
deterministic first pass rather than replacing it.

The budget (`[engine] context_files_max_chars`, default 6000) is a
**total**, split equally between pins. Equal shares beat clever
weighting: the user picked these files deliberately, and a scheme that
quietly starves one is worse than one that outlines both. Tier is
computed at emit time from the live share, never stored, so the label
can never disagree with what was actually sent.

The trigger is mechanical: if the content exceeds the share of the
context window the pin is allowed, it gets compressed rather than
truncated. Truncation would cut a command guide in half mid-table;
compression keeps the invocations and drops the prose.

Digesting costs a model call, so it is **async and background**: the
pin registers immediately with an `ingesting …` marker, the session
stays responsive, and the digest swaps in when it lands. Never block a
prompt turn on an ingest.

### Promotion: atomic for a file, checkpointed for a tree

The two cases are nothing alike in human time, and the promotion rule
follows that, not cache theory:

- **A markdown file** cooks in seconds — read, maybe one compression
  call, done. Promote **atomically** on completion: one epoch, and it
  lands before the user finishes typing their next command.
- **A directory can take hours.** Holding everything back until the
  last file would make the pin useless for the whole run — the wrong
  trade even though it is the cache-optimal one. So promote at
  **checkpoints** (per-file, or per-N-files): a handful of epochs
  spread over an hour is nothing, and each one makes the pin more
  useful *now*. A tree ingest is a job, and it must be useful while
  incomplete — including if it never finishes.

While a checkpoint is being assembled, the in-flight piece rides the
*volatile suffix* (after the stable prefix), so an ask mid-cook sees
the freshest partial and pays prefill only for the tail.

A long cook needs **throttling** — background ingest must yield to the
user's interactive asks, never race them for the GPU — and a **cancel**,
which the mediated syntax gives for free:

```
#@ hey, nevermind, cancel that shit      → PINCANCEL
```

### No file watching — re-cook is asked for

Goulash does not watch the filesystem and pounce. A cheap `stat` at
prompt turns is enough to notice drift and mark the pin **dirty
(`*`)** in the chrome; acting on it is the user's call:

```
#@ hey buddy, can you reload that markdown for me?
```

This is deliberate: no inotify/FSEvents platform code, no autosave
storm re-cooking the prompt, no surprise GPU spend, and the
epoch-churn hazard mostly evaporates because every promotion is
user-initiated. An `auto_recook` toggle is a later convenience, not
the default.

## Freshness

Cheap `stat` at prompt turns marks a pin dirty; the old digest keeps
serving until the user asks for a re-cook (above). Stale beats
stalled, and stale-and-labelled beats both. Digests cache under
`~/.goulash/context/` keyed by path + mtime + size, so re-pinning
across sessions is free.

## What v1 actually reads, and what it refuses

- **Relative paths resolve against the SHELL's cwd**, learned from the
  OSC wire — not goulash's own process directory, which is wherever the
  binary was launched an hour ago. `~/` is expanded; nothing else is,
  because this is a path, not a command line.
- **Binaries are refused**, not pasted: a NUL in the first 8 KB and the
  pin fails with a reason. A hard 512 KB read cap sits above that.
- **A tree is bounded** — 64 files, 3 levels, with `.git`, `target`,
  `node_modules`, `.venv` and friends skipped, and dotfiles ignored. It
  is a convenience, not a crawler; the note says when the cap was hit.
- **A failed pin changes nothing.** Missing path, unreadable file,
  binary: the error is reported and the context is untouched.
- **Re-pinning the same path replaces it** and keeps its id — that *is*
  the re-cook — rather than stacking a duplicate.

## Hazards to settle before writing code

- **Secrets: the rule is egress, not content.** Reading a file with
  credentials in it is not a leak when the model is yours — it is the
  point. "*# can we put my AWS keys in this command?*" is a feature,
  and filtering it would be paternalistic nonsense on your own
  machine. So the gate is a **per-provider `trusted` flag**, inferred
  smartly (loopback → trusted; anything else → untrusted until said
  otherwise) and always overridable — because someone's own GPU box on
  the LAN is *remote by address and trusted by ownership*, and address
  alone cannot tell you that. Trusted → no content filtering.
  Untrusted → skip-list + explicit confirm, since that is where a pin
  actually leaves the building, on every ask.
- **Budget shares, not one pot.** Working context +
  [memories](agent-memory.md) + session log compete for the same
  prefix, so each gets a percentage of a total derived from `num_ctx`,
  with a defined eviction order when over: **session log trims first**
  (it is the most regenerable), then working context degrades a tier
  (verbatim → digest → outline), and **memories evict last** — they
  are the smallest and the most deliberately curated. Shares and the
  live totals visible in `#/status`.
- **Epoch churn.** Every promotion invalidates the prefix cache — but
  since goulash never watches files and re-cooks are user-asked, churn
  is bounded by intent rather than by an editor's autosave. What
  remains to bound is a *tree* cook's checkpoint rate: coarse enough
  that a long job costs a handful of epochs, not hundreds.

## Open questions (for the design session)

1. **Scope** *(v1: session-only, deliberately)*: per-cwd (project-local
   pins, auto-restored on `cd` into the tree) or global? Per-cwd feels
   right for `commandRef.md`, global for a personal style guide —
   possibly both, with the list showing which is which. v1 persists
   nothing, which is the one choice that does not pre-empt this: pins
   are deliberate and cheap to re-make, and a stored pin would have to
   pick a scope before the question was settled.
2. **Digest authorship**: model-written (better, costs a call, needs a
   model that's up) or deterministic extraction (headings, code
   fences, first-N-lines — instant, dumber)? Deterministic as the
   fallback when no engine is bound, at minimum.
3. **Retrieval vs pinning**: always-in-prefix (simple, cache-friendly,
   bounded) or top-k retrieval per ask (scales further, breaks the
   stable prefix)? Start pinned; the [memory bank](agent-memory.md)
   tier is where retrieval belongs.
4. **Auto-offer**: if a repo has `goulash.md` / `AGENTS.md` / a
   `README` with a command section, do we offer to pin it on `cd`?
   Nice, but goulash never acts unasked — a one-line notice at most.
5. **Globs and URLs**: `#@ docs/*.md`, `#@ https://…`? Both plausible,
   both widen the hazard surface.
