# Agent Memory: Remember-as-a-Tool

**Status: v1 shipped — the prime store, as a flat slot-limited file.**
`#/memory on` (default off) enables `~/.goulash/memory.toml`: 25 slots ×
≤240 chars (~55 tokens), baked into the stable prefix, written by the
model via `REMEMBER:`/`FORGET:` lines (a revision = both in one reply;
forgets apply first so a full store can still be revised) and by the
user via `#/memory add|delete|modify|find|limit`. The memory-bank tier,
memory vendor, and promotion policy below remain backlog. The slot
count is sized for hand-curation; **`#/study` changes that** — a
worker mining the transcripts writes far more than a human would, so
the cap goes to 50–100 slots when it lands, together with a real
memory browser to review what the machine wrote
([build-plan](../product/build-plan.md)). Original design follows.

Give goulash the ability to
*decide to remember things* — the first durable, cross-session memory,
distinct from the [session log](memory-hierarchy.md) (which dies with
the session) and curated by the agent rather than recorded
automatically.

## Two stores, both useful

| Store | Size | Read path | Write discipline |
|---|---|---|---|
| **Prime memory** | tiny (hard cap — it taxes every prompt) | always in context, inside the **stable prefix** — changes are epoch events, so it stays [prefix-cache friendly](llm-engine.md) | only things worth paying for on every single ask |
| **Memory bank** | large | searched on demand; keyword/BM25 first, embeddings later; top-k injected when relevant to the question | cheap to add, ranked at retrieval |

## Two positions, and the one we forgot

The store is emitted **twice**, for the same reason a pin is
([working-context](working-context.md)): the prefix copy is complete and
cache-warm and sits at the furthest point in the prompt from the
question, which is not where a sliding-window model looks.

That argument produced the pin cards and then was not applied here —
until a slot recording that macOS `du` wants `-d <depth>` sat in the
prefix while the model suggested GNU `--max-depth=1` twice running. So
the slots most relevant to *this* question are now restated next to it,
inside a 400-character budget, ranked by keyword overlap with newest
first on a tie. Crude beside embeddings; instant, engine-free, and
incapable of failing, which is what a floor has to be.

Two smaller things in the same repair. The prefix block leads with the
**notes**, not the `REMEMBER:`/`FORGET:` protocol that used to occupy
the first four lines — instructions are for the rare turn that writes a
memory, the notes are for every turn. And `num_keep`
([llm-engine](llm-engine.md)) now defends the prefix copy from
left-truncation, because a store that gets silently dropped when the
context overflows is worse than no store: it looks like it is working.

## The write path: a tool the model invokes

Extend the line protocol that already works for small models (`CMD:`):

```
REMEMBER: the deploy needs `make release TARGET=prod` — plain make breaks signing
```

A `REMEMBER:` line in any answer (asked or [commentary](../interaction/heckle-mode.md))
gets stored — bank by default, prime only via explicit promotion. User
paths too: `#/remember <note>`, `#/memory` (list), `#/forget <n>`, and
natural-language asides ("# remember that…") the model turns into a
`REMEMBER:` line itself.

## The killer use case

Hard-to-remember commands. Once remembered, they power a **memory
vendor** in the [suggestion pipeline](suggestion-vendors.md): when the
current context matches a remembered command's situation, it vends into
the suggestion list — one Down-press away, same accept path as
everything else. Remembering a command once means never composing it
again.

## Storage & trust

- Plain files under `~/.goulash/memory/` (`prime.md`, `bank.jsonl`) —
  user-readable, user-editable, greppable; no opaque database.
- Every note carries a timestamp and provenance (what block/ask it was
  derived from), per the
  [anti-poisoning invariants](memory-hierarchy.md).
- The prime store is the highest-leverage prompt real estate in the
  system: curation (promotion/demotion/expiry) matters more than
  capacity. Open questions below.

## Open questions

Tracked in [open-questions](../product/open-questions.md): auto-remember
policy for the watcher (when does commentary decide something is
memorable without being told?), bank retrieval ranking, prime-store
eviction, cross-machine sync, and whether memories are per-project
(cwd-scoped) or global.
