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

---

## Machine-derived memories (proposed)

Falls out of the [situated-context](situated-context.md) work: the
experiment there prepends environment facts to the stable prefix so the
model stops guessing about the machine. That is *exactly* what this
store already does — durable facts, pinned into the cached prefix,
inspectable and editable. Building a second mechanism for it was a
mistake; the facts belong here.

`Slot.by` already distinguishes `"user"` from `"llm"`. Add `"machine"`.

### What would be seeded

Derived at session start by reading, never by executing — a `--help`
probe would break the no-run invariant and can trip installers on shimmed
binaries:

- **platform + userland**, from `uname` and `$SHELL`. Targets a measured
  **6.9% of 2395 vended commands** using GNU-only syntax on a BSD box
  (`grep -P` 114 times against a grep that has no `-P`).
- **installed tools**, from a `read_dir` of PATH intersected with the set
  a shell assistant actually reaches for.

### Why memory is the better home than a bespoke preamble

- It is already in the cached prefix, so the cost is one fill per session.
- It is already **inspectable** (`#/memory`) and **editable**. A user who
  disagrees can delete or override, which a hardcoded preamble does not
  allow.
- It already persists, so refresh can be lazy — the platform never
  changes; the tool set changes when you `brew install` something.
- Provenance is already modelled, so the band can show *why* the model
  believed something.

### The contradiction case, which is the interesting part

The sweep seeded a memory reading **"prefers fd over find"** on a machine
where **`fd` is not installed**. Models duly reached for it. An asserted
memory outranked reality, and the feature meant to personalise made
answers worse — memory without grounding is worse than no memory, because
it carries authority.

With machine slots present, that contradiction is *detectable*: a
`"user"` slot recommending a tool that a `"machine"` slot says is absent
is a flag worth raising rather than a silent wrong answer. Which suggests
machine slots should be **more** trusted than asserted ones on questions
of fact, and less on questions of preference.

### Open

- **Slot accounting.** Machine slots should not eat the user's 25. Either
  a separate budget or a separate block.
- **The volatile header.** `context_block()` leads with a live
  `(N/25 slots)` count, so any change to the store rewrites a string
  sitting in front of the whole session log — the invalidation the S2 arm
  was built to price. Machine slots make the store bigger and refreshable,
  which makes fixing that header a prerequisite rather than a nicety.
- **Refresh policy.** Startup only, or re-derive when PATH changes?
- **Default.** Memory currently defaults *off*. Machine facts are the
  cheapest, least personal, most obviously-correct thing the store could
  hold, so they are the natural argument for defaulting it on.
