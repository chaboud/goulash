# Naming: Current State

**Status: shortlist, not final.** The active candidates are **goulash**,
**flsh**, and **lavash**. Goulash is the working name (it's the repo name),
but the real priority has shifted from naming to building — see
[../product/build-plan.md](../product/build-plan.md).

## The shortlist

| Name | Expansion | Why it survives |
|---|---|---|
| **goulash** | Generic Overlay for Universal LLM-Augmented SHells | Most memorable and product-like; metaphor (everything mixed in one pot) fits the [block history](../architecture/block-history.md) model. Only soft collisions. |
| **flsh** | Fast LLM SHell (or **FLESH** — Fast LLM-Encapsulated SHell) | Compact, four glyphs, no direct shell collision. "Encapsulated" describes the [PTY wrapper](../architecture/pty-overlay.md) well. Weak pronunciation as `flsh`; slightly cyberpunk as FLESH. |
| **lavash** | LLM-Aware Veneer Across SHells | Best architectural acronym — "veneer" is exactly the non-invasive overlay philosophy ([positioning](../product/positioning.md)). No prominent shell/AI-terminal collision found. |

## The earlier ballot

An advisory ranking from the brainstorm (before the shortlist settled):

1. Goulash
2. Lavash
3. Flesh
4. Yesh
5. Gesh

Summary of that vote: Goulash is the name people will remember and tell
someone else about; Lavash is the cleverer acronym; FLESH is the best
compact/cyberpunk option.

## Naming scheme sketch (under the working name)

```
Product:       Goulash
Executable:    goulash
Shell adapter: goulash-zsh / goulash-bash
Agent/tasks:   perhaps Tahini or Kasha (see runner-ups)
Chat mark:     #
Tagline:       "Your shell, with a coach."
```

## See also
- [goulash.md](goulash.md) — collision detail on the working name
- [runner-ups.md](runner-ups.md) — names that were good but not chosen
- [candidate-graveyard.md](candidate-graveyard.md) — names that are dead
- [criteria.md](criteria.md) — the screening method
