# Changelog

Notable changes per release. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Goulash is pre-1.0, so minor versions may change behaviour. Settings that
move carry a note here and keep working from an existing `config.toml`
where that is possible.

## [Unreleased]

Nothing yet.

## [0.4.0] — unreleased

The measurement release. Engine defaults stopped being guesses: they come
from ~5,500 generations across 24 model cells and two engines, and every
one is traceable to the run that chose it
([`wiki/architecture/levers.md`](wiki/architecture/levers.md),
[`bench/`](bench/)).

### Added
- **Two tiers.** `#` answers fast, then amends underneath in the same
  turn; `#?` goes straight to the slow tier and returns one mediated
  result. Configurable per tier via `[engine.fast]` / `[engine.slow]`.
- **Activity dots** in the status chip show which tier is working.
- `--config print | path | set K V | reset [K]`. `reset` **removes** the
  key rather than writing today's value, so improved defaults reach
  existing installs.
- `#/fast`, `#/slow`, `#/thinking`, `#/divulge`, and a fuller `#/status`.
- `engine.divulge.platform` (default on) tells the model the OS, shell
  and userland, so it stops suggesting Linux-only flags on a Mac.
  `divulge.tools` and `divulge.full_path` exist but default off — neither
  showed a measurable benefit, and `full_path` costs ~3900 tokens.
- `engine.num_ctx_min` / `engine.num_ctx`: adopt a resident model's
  context when it is large enough, and only nudge the host when it is
  not. Changing context forces a multi-second reload (206 ms reuse vs
  1847 ms reload), so goulash avoids provoking one.

### Changed
- **Reasoning gets its own token allowance** instead of competing with
  the display budget. Providers meter both on one counter, so a shared
  cap meant a reasoning model spent the answer budget thinking and
  returned nothing.
- `response_tokens` 256 → 1024. The old ceiling did no display work:
  answers that arrive use a median of 32 tokens. Brevity comes from the
  prompt and from the band clamping at draw time.
- Thinking is a setting (`off` / `auto` / `forced`) and is
  **capability-checked**. ollama returns HTTP 400 for models that cannot
  think rather than degrading, so `auto` asks first and never sends the
  field where it would fail.
- The prompt asks for the command **before** the prose. Vend rate on `#`
  asks 52% → 77%, with no detectable quality change (paired blind
  grading, n=112).

### Fixed
- **Empty answers on OpenAI-compatible chat endpoints.** The reasoning
  allowance was added only when thinking was requested — but a chat
  template reasons whatever we send, and `deepseek-r1` reasons through
  ollama's `think:false`. 87% of chat rows came back empty with the
  budget spent on reasoning. The allowance is now unconditional.
- **Dropped an internal `stop` sequence** that truncated valid answers.
  Answer rate 81% → 94%, and it was categorically fatal with reasoning
  on: a blank line inside the thinking tripped it after ~4 tokens.
- **Idle repaint.** The status bar no longer redraws when nothing is
  happening — it was emitting ~807 B/s forever, which is real cost over
  ssh and on battery.
- A per-request `Box::leak` in the engine worker leaked memory on every
  ask; moved to once per worker.

## [0.3.0] — 2026-07

### Added
- `##` chat focus: a multi-turn pane that hands commands back to the
  shell line, with the slot stack browsable in-chat on one spatial axis.
- Menu primitive: bare `#/model` opens a modal, filterable selector.
- Suggestion slot history, and `#/model` persistence.
- Model crash fuse.

### Changed
- Two ingress contracts: a `#` ask demands a command; unprompted
  commentary has to earn one. Heckle freely, command rarely.

### Fixed
- Live-session freeze involving SS3 arrows and blocking warm-up.

## [0.2.0] — 2026-07

First installable release.

### Added
- Homebrew formula; cross-compiled Intel-mac target built on the arm64
  runner.
- Rules vendor and the suggestion pipeline; zsh plain-Down pull; `#`
  aside interception.
- Shell integration over a private OSC channel, producing real command
  blocks.

### Fixed
- Vanishing status bar (erase-below now triggers a same-batch repaint).
- Status-bar flicker and the scroll-region race.
- macOS build: `TIOCSCTTY` cast for `ioctl`.

## [0.1.0] — 2026-07

Milestone 0: a transparent PTY wrapper with a reserved status row, plus
session sensing and CI. The design wiki was seeded in the same period —
see [`wiki/meta/provenance.md`](wiki/meta/provenance.md).

[Unreleased]: https://github.com/chaboud/goulash/compare/v0.3.0...HEAD
[0.4.0]: https://github.com/chaboud/goulash/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/chaboud/goulash/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/chaboud/goulash/releases/tag/v0.2.0
