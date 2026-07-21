# Naming: Runner-Ups

Names that survived the direct shell/AI-CLI screen but didn't make the
final [shortlist](decision.md) — or did (lavash, flsh) and are detailed
here. "Survived the screen" means no prominent existing shell or AI
command-line product under the exact name was found; it is **not**
trademark clearance.

## The live non-goulash candidates

### Lavash
**LAVASH — LLM-Aware Veneer Across SHells** (alt: *Lightweight AI Veneer
Across SHells*, arguably better for explaining the non-invasive philosophy).

"Veneer" is exactly what's being built: a thin intelligent layer across
whichever shell the user already prefers. Collisions found: an
Elixir/Phoenix library and an effectively-placeholder Cargo package —
neither a shell or AI-terminal product. Best architectural acronym of the
whole exercise.

### FLSH / FLESH
**FLESH — Fast LLM-Encapsulated SHell.** "Encapsulated" describes the
[PTY-wrapper architecture](../architecture/pty-overlay.md) well.
Collisions: an active Rust crate named `flesh` and an older PyPI package —
neither a shell, so not a direct category collision, but the
executable/package namespace isn't clean. Brand character: memorable,
compact, slightly cyberpunk, slightly gross, and suggests the LLM
"putting flesh on" the shell. Bare `FLSH` reads as a stripped-vowel
"flash", is awkward to say, and already names a research software library.

## Concept names (conversational identity)

- **CoaSH** — *Conversationally Assisted Shell*, pronounced "coach".
  Was the pre-goulash favorite: communicates guidance without autonomous
  control. Materially better than CASH. Its spirit survives in the
  tagline and [positioning](../product/positioning.md).
- **AsideSH** — the `#` interaction is literally an aside to the shell.
  Conceptually exact; see [../interaction/model.md](../interaction/model.md).
- **PluSH** — Personal LLM User Shell. Natural word, friendly.
- **BluSH** — Block-aware LLM User Shell. Surprisingly good fit for the
  [block-history architecture](../architecture/block-history.md).
- **FloSH** — Flow-aware LLM Overlay Shell. Captures live tail,
  historical backfill, delegated workflow.
- **AideSH**, **KinSH**, **PalSH**, **AllySH** — assistant/companion
  positioning.

## Architecture names

**ThreadSH**, **BranchSH**, **BlockSH**, **PulseSH**, **TraceSH** —
each maps to a real mechanism: threads/branches of
[delegated work](../interaction/delegated-agents.md), the block
transcript, the most-recent-state replaceable update, and causal history
(command → result → question → suggestion → action).

## Delegated-agent names

**ScoutSH** (good for `# go recon this folder`), **StewardSH** (bounded
delegation, permission scopes), **PalSH**, **KinSH**, **CounselSH**,
**SageSH**, **MuseSH**, **TendSH** — mostly better as names for the
*agent* than the product.

## Four-letter survivors (compromised)

- **GESH** — best remaining four-letter candidate; searches surfaced
  mainly a developer handle, not a product. *Generative Evaluator SHell.*
- **YESH** — no direct shell product surfaced; common personal name.
  *Your Enhanced SHell.* ("Pretty good" per the brainstorm.)
- **QESH** — clean in the shell category but drowned by
  Quality/Environment/Safety/Health software.
- **OLSH / ELSH / LLSH / ULSH** — accurate initialisms, poor spoken names.

## Food shortlist (beyond goulash/lavash)

Ranked: **Relish** (Reactive LLM Interface for SHells — heavily occupied),
**Radish** (executable taken by several CLI projects), **Salsa**
(Shell-Aware LLM Sidecar Assistant — impossibly broad namespace),
**Tahini** (Terminal-Aware History and Intelligence — good *subsystem*
name), **Aioli** (AI Overlay for Line Interaction — perhaps too cute),
**Borsh** (visually too close to Bash), **Kasha** (Knowledge-Augmented
Shell Assistant — better as the agent's name).

## See also
- [candidate-graveyard.md](candidate-graveyard.md) — the dead
- [criteria.md](criteria.md) — how the screen worked
