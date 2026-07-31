# Open Questions

Unresolved design questions, gathered from the brainstorm. Answering one
should turn into edits on the linked pages.

## Naming
- Final call among **goulash / flsh (FLESH) / lavash** —
  [decision](../naming/decision.md). Working name is goulash; decide
  before public release, not before code.
- If goulash wins: do subsystems get food names (Tahini for history,
  Kasha for the agent) or is that too cute?

## Input & editor integration
- How good can bash gate-3 (history-position) awareness get without
  patching Readline? `bind -x` prototype vs. deeper integration —
  [shell-integration](../architecture/shell-integration.md).
- Exact suggestion *rendering*: ghost text after the cursor, replace the
  line, or a below-prompt band? (Ghost text interacts with other
  autosuggestion plugins like zsh-autosuggestions.)
- Conflict policy when the user already runs autosuggestion/completion
  plugins that bind Down.

## History & context
- Block store format and location; retention and size limits —
  [block-history](../architecture/block-history.md).
- Per-block raw caps for pathological output (head/tail + spool) —
  [memory-hierarchy](../architecture/memory-hierarchy.md).
- Epoch-boundary policy: when to compact the serving prefix (time?
  token count? region closure?) — [llm-engine](../architecture/llm-engine.md).
- Redaction beyond echo-off: patterns (tokens, keys) in *output*?
- How does the rolling cleanup loop get audited — can the user see and
  edit region markers?

## Rendering & suggestion UX
- Minimal VT-state tracker scope: exactly which sequences must be parsed
  to keep [status rows](../architecture/status-rows.md) uncorrupted?
- Suggestion list focus UX: inline in the status rows vs. a transient
  popover above them; how much of the list is visible unfocused?
- Bracketed-paste injection edge cases (readline < 8.1 defaults, shells
  with paste hooks) — [suggestion-list](../interaction/suggestion-list.md).
- ~~`##` pane rendering approach~~ **Resolved: push the splitter** —
  shrink the inner PTY (plain winsize change), no compositing, modal
  toggle — [chat-mode](../interaction/chat-mode.md). Remaining detail:
  whether chat scrollback persists across toggles. (Splitter ratio is
  now a live-adjustable setting —
  [settings-and-nav](../interaction/settings-and-nav.md).)
- Left-edge gesture: chord alternative for overloaded-arrow setups; how
  much of the settings tree gets in-UI exposure vs. file-only —
  [settings-and-nav](../interaction/settings-and-nav.md).
- `~/.goulash` vs. `$XDG_CONFIG_HOME` —
  [implementation](../architecture/implementation.md).

## TUIs & boundaries
- Opt-in protocol for TUIs that *want* to integrate rather than be
  opaque — [opaque-blocks](../architecture/opaque-blocks.md).
- Remote marker protocol over ssh; per-pane tmux launch ergonomics —
  [remote-and-multiplexers](../architecture/remote-and-multiplexers.md).

## Delegated agents
- Permission-scope model for stewards (filesystem, network, exec) —
  [delegated-agents](../interaction/delegated-agents.md).
- Sub-tab UX: inside the terminal (alternate screen? tabs?) or a
  companion view?

## Visibility & seen-model
- Scroll-back pinning/awareness: **accepted as impossible natively**;
  optional futures = OSC 133 mirroring (native prompt-jumping in
  emulator scrollback), per-emulator APIs, or opt-in captive scroll
  rendered from block history.
- Seen-exposure annotations: dwell threshold, focus-reporting
  (CSI ?1004h) plumbing, and how the summarizer weighs "seen" blocks —
  [block-history](../architecture/block-history.md).
- Feed the *visible screen region* as ask-context (vt100 mirror already
  has it) vs. recent blocks — when does each win?

## Agent memory (backlog — [agent-memory](../architecture/agent-memory.md))
- Auto-remember policy: when may commentary store a note without being
  asked, and how loudly is that surfaced?
- Prime-store curation: promotion/demotion/eviction for the most
  expensive prompt real estate in the system.
- Bank retrieval ranking (keyword/BM25 → embeddings) and injection
  budget per ask.
- Scope: per-project (cwd-keyed) vs. global memories; cross-machine
  sync.

## Engine
- Which local model for the watcher tier, and minimum viable hardware —
  [llm-engine](../architecture/llm-engine.md).
- `goulash bootstrap local` shape: build llama.cpp vs. fetch binaries vs.
  wire an existing server (ollama etc.); which vetted starter model.
- Apple Foundation Models shim: package the Swift bridge as a separate
  optional binary, or link it into the mac build?
- Probe-chain consent UX: how loudly does first-run announce which
  engine it found and what data flows where?
- Latency budget for suggestions; when does the engine *proactively*
  prepare one vs. only on demand (cost/privacy trade-off)?
- Rules vendor: which curated subset of thefuck's rules ships first;
  confidence threshold for passive vending —
  [suggestion-vendors](../architecture/suggestion-vendors.md).
- Is a tiny bundled continuation/spell-check model ever worth it beyond
  rules + history + n-grams?
- ~~Whether `#/` commands take args~~ **Resolved: one argument, the
  single most obvious swivel** (`#/model watcher`, `#/split 40`); bare
  form opens the selector — [settings-and-nav](../interaction/settings-and-nav.md).
  Remaining: the v1 command set.
- Heckle band: tone/verbosity dial, when commentary is worth the pixels,
  and how hard to rate-limit the sass —
  [heckle-mode](../interaction/heckle-mode.md).
- Autonomy dial: scope-grant language and UI for `accept-each` /
  `auto-evaluated` modes — [chat-mode](../interaction/chat-mode.md).
- **Prompt templating on OpenAI-compatible servers.** *(opened
  2026-07-31, blocking-ish for 0.4.0)* ollama's `/api/generate` applies
  the model's chat template by default; LM Studio's `/v1/completions`
  applies nothing. goulash sends the same bytes to both, so on LM Studio
  the model *completes* the prompt instead of following it — Gemma
  degenerates into a repetition loop, qwen3 answers a question nobody
  asked, and neither reports an error. Three ways out, none free:
  (a) apply the model's template client-side, which needs per-model
  knowledge goulash has deliberately avoided; (b) use
  `/v1/chat/completions` and pay the reasoning cost that made us leave
  it; (c) document it and let the user pick the endpoint. Measured in
  [bench/QUIRKS.md](../../bench/QUIRKS.md) §3.
