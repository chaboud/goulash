# Positioning: Coach, Not Agent Overlord

> **Your shell, with a coach.**

Goulash is a system that **watches, advises, and offers executable
suggestions while leaving control with the user**. That sentence — coined
while evaluating the name CoaSH ("Conversationally Assisted Shell",
pronounced *coach*) — survived the [naming churn](../naming/decision.md)
as the product's actual thesis.

## The stance

- **Human-controlled collaborator, not agent overlord** (the AllySH
  framing). The LLM never runs anything; the user accepts, edits, or
  ignores [suggestions](../interaction/down-arrow-protocol.md).
- **Veneer, not replacement** (the Lavash framing): a thin intelligent
  layer across whichever shell the user already prefers — dotfiles,
  plugins, and muscle memory untouched
  ([pty-overlay](../architecture/pty-overlay.md)).
- **Advises rather than commands** (the CounselSH framing); **quietly
  tends the session** and prepares suggestions in the background (the
  TendSH framing).
- **Delegation is explicit and bounded** — the user forks work on purpose
  ([delegated-agents](../interaction/delegated-agents.md)); nothing
  self-initiates in the live terminal.

## Non-negotiable trust properties

These fall out of the architecture, not policy promises:

1. Interactive apps are never interfered with —
   [three gates](../architecture/input-ownership.md).
2. Secrets typed with echo off are never recorded —
   [opaque-blocks](../architecture/opaque-blocks.md).
3. TUI screens aren't streamed to the LLM — opaque lifecycle blocks only.
4. Remote sessions aren't silently surveilled —
   [ssh is a boundary](../architecture/remote-and-multiplexers.md).

## Elevator pitch

Wrap your existing shell in `goulash "$SHELL"`. Keep working exactly as
before. When you want help, type `# a question` or press Down past the
end of history. Everything else is pass-through.
