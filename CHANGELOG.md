# Changelog

Notable changes per release. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Goulash is pre-1.0, so minor versions may change behaviour. Settings that
move carry a note here and keep working from an existing `config.toml`
where that is possible.

## [Unreleased]

### Changed
- **The `terminal` settings group is now `nerd stuff`.** It stopped
  being about the emulator once the working bar and `slow_via_fast`
  moved in; the name promised terminal knobs and delivered a drawer of
  goulash's own internals. `#/debug` still opens it.
- **Drop-downs explain the value you are standing on.** A row's values
  were a bare list of words, and `waldorf` beside `query` is a quiz with
  no question. Every drop-down now carries the same one-line help the
  settings row does — including `custom…`, and including `provider`,
  where `auto` means *go find a local server* on the fast lane and
  *follow the fast lane* on the slow one.

## [0.4.0] — unreleased

Two lines of work merged: a lane/context/interaction release driven by
daily use, and a measurement release that replaced guessed engine
defaults with ones traceable to ~5,500 generations across 24 model cells
and two engines ([`bench/`](bench/)).

### Added
- **Two lanes.** `#` answers fast; `#?` sends the slow lane to research
  the same turn and amend underneath it. `#?/model` binds the research
  lane, `#?/cancel` stops it, and `engine.slow` is a ladder — `manual`
  (default), `query`, `waldorf` — saying when the lane speaks up
  *unasked*. There is no `off` rung: `#?` **is** the request for this
  lane and a pin always goes to it, so a setting able to refuse either
  would make the key silently dead.
- **Per-lane provider, model, thinking and token budget.**
  `[engine.slow_lane]` overrides any of them, so research can run on a
  bigger box or a hosted model while `#` stays local. Anything left
  `auto` follows the fast lane by being **absent** from the file, never
  a frozen copy of what fast happens to say today — so improving a fast
  default improves slow with it. Slow thinking defaults to `medium`,
  because a research pass that does not think is just a slower copy of
  the answer you already have.
- **`#@` working context.** Pin a file or directory into the prompt's
  stable prefix; a vendor's command reference makes the model correct
  about a CLI it has never seen. Read-only, performed by goulash, never
  the shell. Pins carry a *card* — a few lines restated next to the
  question, where a sliding-window model actually attends.
- **Menus.** `#/settings`, `#/help`, `#/debug`, `#/memory` and the `#@`
  pin browser, all on one primitive: scroll, type to filter, Enter acts.
- **Per-model capability dialects** (`models.rs`). Thinking is not one
  dial: `gemma3n` rejects the request, `gpt-oss` takes named levels,
  `qwen3` a boolean, `deepseek-r1` reasons whether asked or not.
- **OpenAI-compatible provider** (`wire.rs`) — LM Studio, llama.cpp,
  vLLM, and any hosted `/v1`. Targets `/v1/chat/completions`, because
  that is the endpoint that applies the model's own template. The raw
  `/v1/completions` endpoint applies none, and an instruction sent
  there is *continued* rather than followed — Gemma degenerates into a
  repetition loop, qwen3 answers a question nobody asked. Available as
  `provider = "openai-raw"` for measuring prefix caching, which is the
  one thing it is genuinely better at.
- **Machine facts** (`engine.divulge.platform`, on by default). The
  prompt now names the OS, shell and userland: *"BSD differs from GNU:
  `du -d N` not `--max-depth`…"*. Measured over 4355 vended commands, 91
  used GNU-only syntax on a Darwin box and **70 of the 91 were that one
  flag** — suggested, watched to fail, and suggested again.
- **`--config print | path | set K V | reset [K]`**, handled before the
  tty check so it works over ssh and in scripts. `reset` removes a key
  rather than writing today's value, so improved defaults still arrive.
- **Lane activity dots** in the status chip — cyan for fast, amber for
  slow, one per lane because both run at once.
- **Runaway stats row** (`#/settings stats`) — queue depths and counters,
  because every bug found in review was something whose growth was
  unconditional while its clearing was not.
- **A library target**, so the characterization bench drives the real
  prompt builder, wire format and answer parser instead of a copy.

### Changed
- **`num_ctx` is negotiated, not demanded.** It is part of a model's load
  identity, so asking for a size evicts anything loaded at a different
  one. goulash asks the server what is resident and requests **exactly
  that** when it is large enough, falling back to `num_ctx_min` only
  when the loaded window is genuinely too small to hold a session log.
  Staying silent is not an option that means what it looks like: an
  absent `num_ctx` is a request for the model's own default, not a
  request to keep what is loaded.
- **One token budget, generous** (`max_tokens`, 512 → 8192), covering
  reasoning and answer together. The separate thinking budget is gone:
  reasoning is not ours to ration — a chat template reasons whatever we
  send, and `deepseek-r1` reasons through `think:false` — so splitting
  the meter only ever produced empty answers. Brevity comes from the
  directive and the band clamp; answers that arrive use a median of 32
  tokens.
- The prompt asks for the command **before** the prose. Vend rate on `#`
  asks 52% → 77%, with no detectable quality change (paired blind
  grading, n=112).

### Fixed
- **The stop sequence on the main answer path.** `["\n\n"]` survived on
  `generate()` after the newer paths had dropped it. Removing it lifts
  the answer rate 81% → 94%, and with reasoning on it was a wall rather
  than a cost: a blank line inside the thinking halted output after ~4
  tokens, defeating the budget computed three lines above it.
- **Idle repaint.** An idle goulash wrote to the terminal 3,600 times an
  hour — 555 B/s, and DECSTBM is not a free no-op. The repaint is now
  armed by output and decays to a 30 s interval.
- **Card budget overshoot.** `cards_block()` charged the card body
  against its 400-char budget while also emitting a `@label (path):`
  header, so six pins overshot by 300.
- Overflow panic after ~64 s idle when `idle_repaint` was off; a
  research queue that grew without limit; escapes leaking into repaired
  comments; a repeated `#?` doing nothing through adjacent dedup.
- **Tests reached the developer's real ollama.** Half the e2e suite never
  pinned an engine, so it measured live model output — one test asserting
  that `#` was intercepted by goulash was reading a genuine answer about
  backslashes. Every test now points at a dead port.
- **A model reload on every single ask.** `num_ctx` negotiation returned
  `0` to mean "leave the loaded model alone", and the ollama request
  body sent that `0` as a literal window. Neither half was silence:
  ollama read it as a real request, clamped it to a couple of thousand
  tokens and reloaded to match; the next ask then saw a window below the
  floor and asked for 8192, reloading again. Five to six seconds of
  model load per question, from the one function written to prevent
  evictions. Measured on a resident model: `num_ctx: 0` → 5.3 s reload,
  key omitted → 6.7 s reload, resident value echoed → none.
- **The platform line named the wrong shell.** It read `$SHELL`, which
  is the *login* shell, so `goulash bash` from a zsh login told the
  model "zsh" — and got zsh-flavoured advice for a bash prompt. It now
  names the shell goulash actually launched.
- **`limit` in the memory menu did nothing.** It rendered as a row that
  silently ignored Enter. It is a text field now: `limit: 25 (press
  enter to edit)`, type a number, Enter.
- **The expert toggle moved out from under the cursor.** `nerd stuff` is
  a debug-gated group, so turning expert on inserted a row *above* the
  switch and the selection slid onto whatever took its place — a switch
  you could not turn back off. Everything expert reveals now sits below
  it.

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
