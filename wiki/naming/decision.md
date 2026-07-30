# Naming: Decided — goulash

**Status: settled.** The name is **goulash**, and it is now load-bearing
in places that are expensive to change: the crate name and
`version = "0.4.0"` in `Cargo.toml`, the `goulash.dev` homepage, the
Homebrew formula, `~/.goulash/`, the `#/` command prefix, and the
binary every test invokes.

Reopening it would be a rename across all of those plus any installed
config. The bar for that is a real problem with the name — a trademark
conflict, or a collision that actually confuses users — not a preference.

The shortlist below is kept as the reasoning behind the choice, per
[wiki-conventions](../meta/wiki-conventions.md) rule 5. It is history,
not an open question.

## The shortlist it came from

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
