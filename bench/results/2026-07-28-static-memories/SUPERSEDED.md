# Superseded run — memories were a constant

804 pass-b rows collected 2026-07-28 before the S2 arm was fixed.

The harness pinned the memory block to a fixed string, so it never
changed during a session. S2 (`MemPos::Suffix`) exists to price the
prefix invalidation that a REMEMBER/FORGET causes under the shipped
`MemPos::BeforeLog` shape — and with a constant block there is no
invalidation to price. S2-vs-S1 here compares two *static* prefixes, so
its +445ms median is a position artifact at identical prompt length, not
the cost the arm was built to measure.

## Still valid from this run

- **Pass A** (192 probes) is unaffected: it is single-turn, and no probe
  mutates memory. `bench/QUIRKS.md` stands.
- **S1 vs S3** is a fair comparison — neither arm mutates memory, and the
  only difference between them is directive order. The headline holds:
  command vend rate 56% -> 81% across 266 paired cells, +19ms latency.
- **Cache behaviour is real and good**: prompt-eval *falls* as the log
  grows (gemma3:12b S1: 6925ms -> 2739ms while the prompt grows
  1624 -> 6440 chars), confirming the stable prefix works.

## Not valid

- Any S2 conclusion.
- Cross-run comparison with the new results: prompts differ, because the
  memory block now changes mid-session.

Kept for audit. The replacement run is `2026-07-28/`.
