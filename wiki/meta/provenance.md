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
  **not trademark clearance**, and may have gone stale.
- Architecture is design-stage reasoning; nothing here has been validated
  by an implementation yet (see [build plan](../product/build-plan.md)).

## Editing

Later decisions supersede this snapshot — edit the topic pages directly
(per [wiki-conventions](wiki-conventions.md)) rather than preserving the
brainstorm's wording.
