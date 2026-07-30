# Provenance

This wiki was seeded (July 2026) by distilling a naming-and-architecture
brainstorm conversation into structured pages. That conversation covered:

- an exhaustive screen of shell-name candidates (the `??sh` and
  `s-l-*-sh` namespaces, food words) → the [naming cluster](../naming/decision.md);
- the core architectural question — *how does an LLM overlay read the
  session without breaking interactive apps that need the Down key?* —
  answered with PTY wrapping + terminal job control + line-editor
  integration → the [architecture cluster](../architecture/overview.md);
- the interaction primitives (`#` asides, down-arrow suggestions,
  delegated agents) → the [interaction cluster](../interaction/model.md).

## Caveats inherited from the source

- Name-collision findings were web-search screens at a point in time,
  **not trademark clearance**, and may have gone stale. Still true — the
  name is [settled](../naming/decision.md) on product grounds, which is
  not the same as cleared.
- Architecture was design-stage reasoning when this was written. **That
  caveat has expired.** goulash is a working binary at 0.4.0; the PTY
  overlay, status rows, `#`/`##` interaction, memory store and LLM engine
  all ship, and the engine's settings were chosen from ~5,500 measured
  generations rather than from argument
  ([levers](../architecture/levers.md), [bench](../../bench/)).

## What the brainstorm got wrong

Worth recording, because the wiki's rule is that decisions carry their
reasoning — including when the reasoning was wrong:

- **The name was treated as an open shortlist far longer than it was.**
- **Design-stage confidence outran evidence.** Several pages asserted
  behaviour that measurement later contradicted; where that happened the
  pages now carry dated corrections rather than quiet edits (see
  [machine-facts](../architecture/machine-facts.md) on the platform-error
  rate, and [levers](../architecture/levers.md) §A1 and §A4).

## Editing

Later decisions supersede this snapshot — edit the topic pages directly
(per [wiki-conventions](wiki-conventions.md)) rather than preserving the
brainstorm's wording.
