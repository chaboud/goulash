# `#@` Working Context: Pinned Files as Near-Tool-Use

**Status: design, to be worked through together before building.**

`#@ <path>` pins a file (or a directory) into a **working context** that
rides in the prompt's [stable prefix](llm-engine.md), next to
[pinned memories](agent-memory.md). Instructions, a guide, a runbook —
whatever the user says matters right now.

```
#@ commandRef.md          pin a file
#@ ./deploy/              pin a tree (walked, capped, compacted)
#@                        list what's pinned, with sizes and freshness
#@ drop 2                 unpin
```

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

## Ingest tiers (the budget is the design)

The stable prefix is the KV-cache asset; a fat pin destroys the
latency work. So ingest is **compaction, not concatenation**, chosen by
size against a hard `context_files_max_chars` budget:

| Tier | When | What lands in the prompt |
|---|---|---|
| **Verbatim** | fits the budget | the text as-is |
| **Digest** | over budget | **LLM compression** — the model rewrites it down, biased toward commands, flags, invariants |
| **Outline** | tree, or still over after compression | structure + headings + per-file one-liners, drillable later |

The trigger is mechanical: if the content exceeds the share of the
context window the pin is allowed, it gets compressed rather than
truncated. Truncation would cut a command guide in half mid-table;
compression keeps the invocations and drops the prose.

Digesting costs a model call, so it is **async and background**: the
pin registers immediately with an `ingesting …` marker, the session
stays responsive, and the digest swaps in when it lands. Never block a
prompt turn on an ingest.

### Partial or atomic? Both — split by cache position

An ask that arrives mid-cook should get the best context available,
but a digest that dribbles into the **stable prefix** re-prefills the
whole prompt on every update, and [prefill is the devil](llm-engine.md).
So the cook is staged:

- **While cooking**, whatever is ready lives in the *volatile suffix*,
  after the stable prefix. An ask mid-cook sees the partial and pays
  prefill only for the tail — the "information on the table" model,
  without the cache thrash.
- **On completion**, it promotes into the prefix **atomically**, in one
  epoch. Exactly one cache invalidation per ingest, whatever the file
  size.

A re-cook (file changed) is just another live ingest: meter shows, old
digest keeps serving from the prefix, new one promotes atomically when
done.

## Freshness

Cheap `stat` per ask; on mtime/size change, re-ingest in the background
and **keep serving the old digest until the new one is ready**. Stale
beats stalled. Digests cache under `~/.goulash/context/` keyed by
path + mtime + size, so re-pinning across sessions is free.

## Hazards to settle before writing code

- **Secrets: the rule is egress, not content.** On a local engine,
  reading a file with credentials in it is not a leak — it is the
  point. "*# can we put my AWS keys in this command?*" is a feature,
  and filtering it would be paternalistic nonsense on your own
  machine. So: **local provider → no content filtering**; remote or
  metered provider → skip-list + explicit confirm, because that is
  where a pin actually leaves the building, on every ask. The property
  that matters belongs to the provider, not the file.
- **Budget shares, not one pot.** Working context +
  [memories](agent-memory.md) + session log compete for the same
  prefix, so each gets a percentage of a total derived from `num_ctx`,
  with a defined eviction order when over: **session log trims first**
  (it is the most regenerable), then working context degrades a tier
  (verbatim → digest → outline), and **memories evict last** — they
  are the smallest and the most deliberately curated. Shares and the
  live totals visible in `#/status`.
- **Epoch churn.** Every promotion invalidates the prefix cache. The
  staged cook above bounds it to one epoch per ingest; a file being
  saved repeatedly in another pane still needs debouncing so an editor
  on autosave cannot re-cook the prompt continuously.

## Open questions (for the design session)

1. **Scope**: per-cwd (project-local pins, auto-restored on `cd` into
   the tree) or global? Per-cwd feels right for `commandRef.md`, global
   for a personal style guide — possibly both, with the list showing
   which is which.
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
