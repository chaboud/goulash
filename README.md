# goulash - a plain-language navigator for your shell

Just Down-arrow for your *future* history.

## Why?
*(If you are a terminal graybeard who hand-regexed raw curl to read this page, you can stop now and go back to being immortal.  For the rest of us...)*

We don't always remember the syntax for all terminal commands, and we shouldn't have to.  When we try to string together operations, there's a familiar loop:

Start typing command → forget *esoteric syntax* → go to Google/ChatGPT/Claude to ask → notice email/slack/Hacker-News → read a few papers/links/threads → get lunch → come back to prompt confused about what was happening in this terminal → re-understand the situation → start typing command → ...

Context switching has a cost.  Going to a browser or an agent breaks working flow.

But large language models (LLMs) are pretty good at *esoteric syntax*, even *small* large language models... brainwave! ka-ching! (note: this software is free.)

## What?
Goulash is an LLM-aware overlay for the shell you already use. It watches the session and offers advice and executable suggestions *in your terminal*, but every keystroke and every command is still yours.  Goulash doesn't run commands, pop up choices, or take over how you work.  It's your terminal... with a navigator.

<p align="center">
  <img src="docs/GoulashHashes.gif" alt="goulash demo: asking a question at the prompt, then pulling the suggested command down onto the shell line" width="760">
</p>

## Install

**1. Get an engine.** Goulash talks to a local model server. Either works;
goulash finds whichever is running.

```sh
# Ollama — https://ollama.com  (or: brew install ollama)
brew install ollama && ollama serve &

# or LM Studio — https://lmstudio.ai  (has a GUI; the CLI is `lms`)
lms server start
```

**2. Pull a model.** Any of these are good starting points — the first is
the smallest that still behaves, the last is the most accurate we
measured.

```sh
ollama pull gemma4:e4b      # ~9.6 GB · good default
ollama pull qwen3.5:4b      # ~3.4 GB · lighter
ollama pull gemma4:12b      # ~7.6 GB · best answers in our tests

# LM Studio equivalents
lms get qwen/qwen3-4b
lms get google/gemma-4-12b-qat
```

**3. Install goulash.**

```sh
cargo install goulash # from crates.io

# or, with Homebrew
brew tap chaboud/goulash https://github.com/chaboud/goulash
brew install goulash
```

Then just run `goulash`. With no config file at all it probes for a local
engine, picks a model, and starts working.

## Usage
Run `goulash` and use your shell exactly as before. Goulash provides suggestions *below* your prompt; your *future history*.  Just press the down arrow if you see something you like.

### `#` ask a question, get an answer + a pullable command
Type `#` and a question at your prompt (it's a shell comment recorded in history, never executed).

The LLM will provide a suggested command. Press down (past the end of shell history) and it lands on your prompt for editing.  Enter runs it as your own command. 
Goulash also reviews your shell use as you go and may leave a tip in the same spot.  Use a local model, and it's all private, all local, just like your terminal command history.

### Style - Arrows: one spatial axis

Shell history lives above your prompt; goulash's suggestions live below. Up and Down just move along the line:

```text
   ↑  zsh history (as always)
── your empty prompt ──────────────── neutral
   ↓  newest suggested command       ( ↓ again: older · ↑: back up )
```

Down past the newest keeps going into the **history of suggested commands** with the position (`↑ 3/7 ↓`) shown at the right of the rule. Up retraces to your empty prompt, and past that it's plain zsh/bash history.

### `##` longer chat
`##` (or `## question`) flips into a chat panel; follow-ups need no prefix. The shell keeps running above.  When you select a command to copy to the prompt, you're back in your shell.

### `#?` ask the slow one

Goulash runs two tiers over the same model. **FAST** answers immediately
with reasoning off. **SLOW** thinks first, which takes longer but is
often better.

- `# question` — FAST answers right away. If SLOW disagrees, its answer
  lands as a newer entry you can reach with Down. If it agrees, you see
  nothing extra.
- `#? question` — skip the quick answer. SLOW reasons at length, then
  FAST rewrites the result into one line and a command.

Two dots in the bottom-right chip show which tier is working:

```text
 goulash ·· │ zsh │ prompt     idle
 goulash •· │ zsh │ prompt     FAST   (one dot bouncing)
 goulash •• │ zsh │ prompt     SLOW   (the pair filling)
```

### `#/` goulash's menu controls
```text
#/model            modal model picker: type to filter, ⏎ selects & saves
#/model NAME       try a model for this session (add `save` to persist)
#/fast [on|off]    whether FAST volunteers tips on ordinary commands
#/slow [on|off]    whether SLOW amends a `#` answer  (`#?` works either way)
#/thinking auto    off | auto | forced — steers the SLOW tier
#/divulge tools    tell the model which tools are installed (debug)
#/memory on        give the model a small pinned memory (REMEMBER/FORGET)
#/commentary off   quiet the per-turn heckling
#/status
#/help
```
Add `save` to any of them to write it to your config as well as the
session.

## Settings

Nothing needs configuring — every setting has a working default. When you
do want to look:

```sh
goulash --config print          # every value, and whether it is yours or a default
goulash --config set engine.thinking auto
goulash --config reset engine.thinking   # back to the default
goulash --config reset                   # everything back (keeps a .bak)
```

`reset` **deletes** the setting rather than writing today's value, so your
config file only ever holds things you deliberately changed — and you
inherit better defaults in later versions instead of being pinned to an
old one.

The ones worth knowing about:

| setting | default | what it does |
|---|---|---|
| `engine.model` | *(auto)* | pin a model; unset means auto-pick |
| `engine.thinking` | `off` | `off` / `auto` (only where the model supports it) / `forced` |
| `engine.response_tokens` | `1024` | cap on the visible answer |
| `engine.reasoning_tokens` | `4096` | thinking allowance, spent *on top* of the answer budget |
| `engine.num_ctx_min` | `8192` | smallest context goulash can work in |
| `engine.num_ctx` | *(unset)* | pin a context exactly |
| `engine.keep_alive` | `30m` | how long the model stays resident |
| `engine.fast.watch` | `true` | FAST comments on ordinary commands |
| `engine.slow.ask` | `true` | SLOW amends a `#` answer |
| `engine.slow.watch` | `false` | SLOW amends unprompted tips too |
| `engine.divulge.platform` | `true` | tell the model your OS and shell |
| `engine.prefer_resident` | `false` | use an already-loaded model over the smallest one |
| `engine.commentary` | `true` | unprompted tips at all |

**Goulash tries not to disturb your engine.** It sends no context size
unless what is loaded is too small to work in — both Ollama and LM Studio
let you set that yourself, and changing it forces a multi-second reload.

## Changes

**0.4.0**
- Two tiers: `#` answers fast then amends, `#?` goes straight to the slow one
- Activity dots in the status chip show which tier is working
- `--config print | path | set | reset` — `reset` removes a key so defaults can improve
- Thinking is a setting (`off`/`auto`/`forced`) and is capability-checked, so it is never sent to a model that would reject it
- Reasoning gets its own token allowance instead of eating the display budget
- Dropped an internal stop sequence that was truncating valid answers
- Goulash no longer repaints the status bar when nothing is happening (was ~1 KB/s)
- `#/fast`, `#/slow`, `#/thinking`, `#/divulge`, and a fuller `#/status`
- Tells the model your OS and shell, so it stops suggesting Linux-only flags on a Mac

**0.3.0** — model picker, agent memory, `##` chat, crash fuse, slot history

## Nerd Stuff: Build & modify
```
cargo build
./target/debug/goulash            # wraps $SHELL
./target/debug/goulash zsh        # or name a shell
```

**Shell integration is automatic** for zsh and bash launched with plain flags — goulash injects its hooks (ZDOTDIR trick / `--rcfile` wrapper) on top of your normal rc files, no editing required. That's what powers command blocks, `#` asides, and the plain-Down suggestion pull.

Manual fallback (custom shells or `auto_integrate = false`):

```sh
# ~/.zshrc
[[ -n "$GOULASH" ]] && source /path/to/goulash/shell/goulash.zsh
# ~/.bashrc
[[ -n "$GOULASH" ]] && source /path/to/goulash/shell/goulash.bash
```

Config (optional): `~/.goulash/config.toml`

```toml
[status]
enabled = true
rows = 1
```

Tests (drives the binary under a real PTY):

```
cargo build && python3 tests/e2e.py
python3 tests/longrun.py    # leak + idle-cost probe, ~6 min
```

### Performance, and staying out of the way

Goulash sits in your terminal for as long as the terminal is open, so the
question is not only "is it fast" but "what does it cost while doing
nothing". Everything below is measured — the harness is in `bench/`, and
the write-ups it produced are in `bench/*.md`.

**Nothing happens when nothing happens.** An idle goulash emits **0 bytes
per second**. It used to repaint the status bar once a second as
insurance against a program scribbling on it — invisible, but ~1 KB/s,
which is 86 MB a day and a wakeup every second on battery. The repaint is
now gated on the inner shell having actually written something, because a
bar nothing has touched cannot have been disturbed.

**It does not grow.** Across 250 turns of commands, asks, chat and model
switches: file handles flat at 12, threads flat at 2, GPU handles at 0,
memory up 1.7 MB and levelling off. The one thing that does grow without
bound is the session transcript, at ~1.6 KB per turn.

**Derived facts are derived, never stored.** The OS/shell line goulash
tells the model is recomputed each run rather than cached — a stored fact
cannot notice it has gone stale, and `brew install` would leave it
confidently wrong. It costs ~4 ms, is computed once in the background at
startup, and never on the path of an answer you are waiting for.

**The prompt is built to be cached.** Everything stable — the preamble,
machine facts, the session log — comes first, and only the question and
timestamp change per ask. Local engines cache that prefix, and the effect
is large: re-evaluating a cold 14k-token prompt took 10.6 s against 0.9 s
warm. Adding ~180 tokens of machine facts to the front costs one cache
fill and then nothing (measured: 750 µs/token vs 760 with it absent, by
turn ten).

**Context size is the engine's business, not ours.** A model's context is
part of its load identity, so asking for a different one evicts and
reloads it — 206 ms to reuse versus 1847 ms to reload. Both Ollama and LM
Studio let you choose; goulash sends nothing unless what is loaded is too
small to work in.

**Thinking gets its own budget.** The token cap exists to keep answers
short enough for a status bar, but reasoning and the answer draw on one
counter — so a reasoning model would spend the display budget thinking
and return nothing. They are accounted separately now, which is what
makes `#?` possible at all.

## License
Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in goulash by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
