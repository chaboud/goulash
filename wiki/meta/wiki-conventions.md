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

## Layout

```
wiki/
  home.md              map of content + mermaid mind map
  naming/              name shortlist, survivors, graveyard, criteria
  architecture/        PTY overlay, input ownership, states, history
  interaction/         # asides, down-arrow, delegated agents
  product/             positioning, build plan, open questions
  meta/                these conventions, provenance
```
