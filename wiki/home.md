# Goulash Wiki — Map of Content

**Goulash** — *Generic Overlay for Universal LLM-Augmented SHells* — is an
LLM-aware veneer that wraps the shell you already use (bash, zsh, fish, …),
watches the session, and offers advice and executable suggestions while
leaving control with the user.

> Your shell, with a coach.

This wiki is a mind-map: many small, densely-linked pages, written to be
useful to both humans and LLMs ingesting the repo. Start anywhere; every
page links back out. Conventions are in [meta/wiki-conventions.md](meta/wiki-conventions.md).

## Mind map

```mermaid
mindmap
  root((Goulash))
    Naming
      Decision: Goulash
      Runner-ups
      Candidate graveyard
      Naming criteria
    Architecture
      PTY overlay
      Input ownership
      Session state machine
      Shell integration
      Block history
      Memory hierarchy
      LLM engine
      Two-lane engagement
      Status rows
      Opaque blocks
      ssh / tmux
    Interaction
      "#" aside
      "##" chat mode
      Suggestion list
      Down-arrow protocol
      Delegated agents
    Product
      Positioning
      Open questions
```

## Clusters

### Naming
- [naming/decision.md](naming/decision.md) — current shortlist (goulash / flsh / lavash), the ballot, working name
- [naming/goulash.md](naming/goulash.md) — expansion, collisions, brand assessment
- [naming/runner-ups.md](naming/runner-ups.md) — CoaSH, Lavash, FLESH, Yesh, Gesh, and the food shortlist
- [naming/candidate-graveyard.md](naming/candidate-graveyard.md) — every name that died, and what killed it
- [naming/criteria.md](naming/criteria.md) — what we learned about naming a shell in 2026

### Architecture
- [architecture/overview.md](architecture/overview.md) — the whole system in one page
- [architecture/pty-overlay.md](architecture/pty-overlay.md) — `goulash $SHELL`: PTY master/slave wrapper
- [architecture/input-ownership.md](architecture/input-ownership.md) — who owns the Down arrow: the three gates
- [architecture/session-state-machine.md](architecture/session-state-machine.md) — PROMPT / COMMAND / INTERACTIVE_CHILD
- [architecture/shell-integration.md](architecture/shell-integration.md) — zsh ZLE, bash Readline, fish, generic fallback
- [architecture/block-history.md](architecture/block-history.md) — the block-oriented transcript model
- [architecture/memory-hierarchy.md](architecture/memory-hierarchy.md) — raw leaves, LLM-set region markers, logarithmic ramp-off retrieval
- [architecture/llm-engine.md](architecture/llm-engine.md) — provider probe chain, local-first caching, watcher/thinker tiers
- [architecture/model-capabilities.md](architecture/model-capabilities.md) — thinking is not one dial: per-model schema, provider probe, config escape hatch
- [architecture/suggestion-vendors.md](architecture/suggestion-vendors.md) — rules (thefuck-style), history/n-gram, LLM vendors behind one interface
- [architecture/agent-memory.md](architecture/agent-memory.md) — remember-as-a-tool: prime store + searchable bank (backlog)
- [architecture/two-lane-engagement.md](architecture/two-lane-engagement.md) — fast speaks, slow researches: lanes, MCP capabilities, classification (design)
- [architecture/working-context.md](architecture/working-context.md) — `#@` pinned files as near-tool-use: deterministic + LLM-mediated pinning, budget tiers
- [architecture/status-rows.md](architecture/status-rows.md) — reserved bottom rows via shrunken inner PTY
- [architecture/implementation.md](architecture/implementation.md) — Rust, crate landscape, `~/.goulash/` config
- [architecture/opaque-blocks.md](architecture/opaque-blocks.md) — TUIs as opaque blocks; echo-off secret hygiene
- [architecture/remote-and-multiplexers.md](architecture/remote-and-multiplexers.md) — ssh and tmux boundaries

### Interaction
- [interaction/model.md](interaction/model.md) — the `#`/`##` escalation ladder and the prompt as interaction point
- [interaction/suggestion-list.md](interaction/suggestion-list.md) — async vended, insert-at-top, freeze-on-focus, staleness
- [interaction/down-arrow-protocol.md](interaction/down-arrow-protocol.md) — `goulash-down-or-suggest`
- [interaction/chat-mode.md](interaction/chat-mode.md) — `##`: chat with tools, top third stays shell, autonomy dial
- [interaction/delegated-agents.md](interaction/delegated-agents.md) — `# go recon this folder`, sub-tabs, scouts and stewards
- [interaction/settings-and-nav.md](interaction/settings-and-nav.md) — TV-remote spatial nav, `#/` commands with args, config write-back
- [interaction/heckle-mode.md](interaction/heckle-mode.md) — MST3K commentary band: explanations above the status row, collapsible

### Product
- [product/positioning.md](product/positioning.md) — coach, not agent overlord
- [product/build-plan.md](product/build-plan.md) — from wiki to working binary: MVP milestones
- [product/state-of-play.md](product/state-of-play.md) — handoff notes: branch state, what's on `dev`, next tasks, field findings
- [product/open-questions.md](product/open-questions.md) — unresolved design questions

### Meta
- [meta/wiki-conventions.md](meta/wiki-conventions.md) — how this wiki is organized
- [meta/provenance.md](meta/provenance.md) — where this thinking came from
