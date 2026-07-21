# Naming: Screening Criteria & Method

The rules of thumb that emerged while screening dozens of candidates.
Results live in [decision.md](decision.md),
[runner-ups.md](runner-ups.md), and the
[graveyard](candidate-graveyard.md).

## What was screened for

The screen checked whether a name is already occupied **specifically by
shells, terminal tools, and AI/CLI products** — not merely whether the
word exists as a brand somewhere. Passing means "no prominent existing
shell or AI command-line product under the exact name was found." It is
**not legal/trademark clearance**.

## Collision severity tiers

1. **Hard category collision** — an existing shell, terminal, REPL, or
   AI-CLI under the same name (slosh, slsh, gosh, pish, yosh…).
   Disqualifying.
2. **Namespace collision** — the executable, crate, or PyPI name is
   taken by non-shell software (flesh, goulash). Not disqualifying, but
   the namespace isn't pristine.
3. **Association hazard** — the name means something bad or confusing
   (SLSH the cybercrime alliance; ZOSH vs. spoken "zsh"; slush = messy).
4. **Search ownership** — can you own the phrase? "Goulash shell" is
   ownable even though "goulash" is food-dominated; QESH is technically
   free but drowned by an established business acronym.

## Form preferences

- **Shorter is better**: four letters, one vowel or none, one syllable —
  but that territory is exhausted ([graveyard](candidate-graveyard.md)).
- **Pronounceable beats clever**: FLSH loses to FLESH; CoaSH works
  because it says "coach".
- **The name should describe the architecture**: overlay / veneer /
  encapsulated / block-aware expansions ranked higher than generic
  AI-adjectives. Best expansions doubled as design documentation —
  see [../architecture/overview.md](../architecture/overview.md).
- **Real words beat initialisms**: memorability and word-of-mouth won the
  final ranking (goulash > gesh despite gesh being "cleaner").

## The trade at the end

> The old `goulash` command is real, so this is not pristine namespace
> territory — but it is dramatically better than taking the 37th obscure
> four-letter shell name.
