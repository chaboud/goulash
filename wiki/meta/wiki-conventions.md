# Wiki Conventions

This wiki is organized as a **mind-map in markdown** — in the spirit of
Karpathy's advocacy for knowledge kept as small, densely-linked plain
markdown files that are equally legible to humans and to LLMs ingesting
the repo.

## Rules

1. **One concept per page.** If a section grows its own identity, split
   it into a new page and link it.
2. **Link densely.** Every page links to its neighbors (`See also` when
   links don't fit inline). No orphan pages; everything is reachable from
   [../home.md](../home.md).
3. **Relative links, lowercase-kebab filenames**, so links work on
   GitHub, in editors, and for LLM crawlers alike.
4. **Plain markdown + mermaid only.** No build step, no site generator.
5. **Decisions carry their reasoning.** Record *why* (and what was
   rejected — e.g. the [naming graveyard](../naming/candidate-graveyard.md)),
   not just the conclusion.
6. **Status over false certainty.** Mark open items and route them to
   [../product/open-questions.md](../product/open-questions.md); update
   pages when a question resolves.
7. **Write for ingestion.** Assume a future LLM (or contributor) reads
   pages out of order; each page should stand alone with enough links to
   recover context.
8. **Measurement outranks design.** A page that argues loses to a run
   that measured. When a number here is superseded, **correct it in place
   with a dated note** naming the old value — do not quietly overwrite
   it. A reader who remembers the old figure needs to know it moved and
   why, and a silent edit destroys exactly that.
9. **No machine-generated pages.** Everything here is written, not
   summarized into place by a tool. Six `*_summary.md` files were once
   committed by a summarization run; five were empty and the sixth
   contained raw model output with terminal escape sequences in it. All
   were deleted. If a tool writes a page, it gets reviewed and rewritten
   before it lands.

## Layout

```
wiki/
  home.md              map of content + mermaid mind map
  naming/              the decision, survivors, graveyard, criteria
  architecture/        PTY overlay, input ownership, states, levers
  interaction/         # asides, down-arrow, chat, settings, heckle
  product/             positioning, build plan, distribution, open questions
  meta/                these conventions, provenance
bench/                 the measurement side: harness + findings
```

`bench/` is not part of the wiki — it is excluded from the published
crate and holds the characterization harness and its reports
(`QUALITY.md`, `QUIRKS.md`, `RESIDENCY.md`, `THINKING.md`,
`HEADROOM.md`). Wiki pages cite it; it is the evidence they rest on.
