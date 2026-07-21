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
- How much output per command block is kept verbatim vs. summarized?
- Redaction beyond echo-off: patterns (tokens, keys) in *output*?

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

## Engine
- Which LLM(s), local vs. remote, and latency budget for suggestions.
- When does the engine *proactively* prepare a suggestion vs. only on
  demand (cost/privacy trade-off)?
