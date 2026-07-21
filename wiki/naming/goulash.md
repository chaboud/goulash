# Goulash (working name)

**GOULASH — Generic Overlay for Universal LLM-Augmented SHells**

The expansion describes the architecture accurately: a *generic overlay*
that can augment bash, zsh, fish, PowerShell, or whatever else is running
beneath it — see [../architecture/pty-overlay.md](../architecture/pty-overlay.md).

## Why it works

- Memorable, friendly, and product-like — a name people will repeat.
- The metaphor is genuinely fitting: commands, output, conversation,
  agents, and history all mixed together in one pot — which is literally
  the [block history](../architecture/block-history.md) transcript model.
- `goulash` as an executable name is distinctive and easy to type-complete.
- The tagline practically writes itself: **"Your shell, with a coach."**

## Known collisions (soft, not disqualifying)

| Collision | Category | Assessment |
|---|---|---|
| 2016 Go project shipping a `goulash` executable for Slack slash-command handling | CLI-adjacent, old | Category-distant, dormant |
| LLNL "Goulash" CUDA/scientific project | Scientific computing | Unrelated category |
| Current AI grocery company named Goulash | AI, but not dev tools | Different market |
| Old PyPI name | Packaging | Namespace not pristine |

None of these is an interactive shell or AI-terminal overlay, so they rate
as **soft collisions** under the [screening criteria](criteria.md).

## Downsides

- Search results are food-dominated; the ownable phrases are
  "Goulash shell" and "Goulash LLM shell", not bare "goulash".
- Seven letters — longer than the four-glyph ideal that drove the
  [??sh exploration](candidate-graveyard.md).

## See also
- [decision.md](decision.md) — shortlist status
- [runner-ups.md](runner-ups.md) — lavash and flsh, the other live candidates
