# goulash

**Focus on the flow.**
Think about the problem, not the syntax.

## Why?
We don't always remember the syntax for all terminal commands.  So there's a familiar loop:

Start typing command → forget esoteric syntax → go to Google/ChatGPT/Claude to ask → notice email/slack/Hacker-News → get lunch → come back to prompt confused about what was happening → re-understand the situation → start typing command → ...

Context switching has a cost.

Cognitive load is bad.

But Large Language Models are pretty good at esoteric syntax.

## What?
Goulash is an LLM-aware overlay for the shell you already use. It watches the session and offers advice and executable suggestions *in your terminal*, but every keystroke and every command is still yours.

<p align="center">
  <img src="docs/GoulashHashes.gif" alt="goulash demo: asking a question at the prompt, then pulling the suggested command down onto the shell line" width="760">
</p>

Goulash doesn't run commands.

Goulash doesn't pop up windows.

Goulash doesn't take over.

This is your terminal... with a navigator.



## Install
```sh
# You need an engine for processing.  Ollama with gemma4:e4b works well, for example  

cargo install goulash # from crates.io

# or, with Homebrew
brew tap chaboud/goulash https://github.com/chaboud/goulash
brew install goulash
```

## Usage
Run `goulash` and use your shell exactly as before. Goulash provides suggestions *below* your prompt, like a *future history*.  Just press the down arrow if you see something you like.

### `#` ask a question, get an answer + a pullable command
Type `#` and a question at your prompt (it's a shell comment recorded in history, never executed).

The LLM can provide a suggested command. Press down (past the end of shell history) and it lands on your prompt for editing.  Enter runs it as your own command. Goulash also reviews each command turn unprompted and may leave a tip in the same spot.

### Style - Arrows: one spatial axis

Shell history lives above your prompt; goulash's suggestions live below. Up and Down just move along the line:

```text
   ↑  zsh history (as always)
── your empty prompt ──────────────── neutral
   ↓  newest suggested command       ( ↓ again: older · ↑: back up )
```

Down past the newest keeps going into the **history of suggested commands** with the position (`↑ 3/7 ↓`) shown at the right of the rule. Up retraces to your empty prompt, and past that it's plain zsh/bash history.

### `##` chat, without retyping `#`
`##` (or `## question`) flips into a chat panel; follow-ups need no prefix. The shell keeps running above.  When you select a command to copy to the prompt, you're back in your shell.

### `#/` goulash's menu controls
```text
#/model            modal model picker: type to filter, ⏎ selects & saves
#/model NAME       try a model for this session (add `save` to persist)
#/memory on        give the model a small pinned memory (REMEMBER/FORGET)
#/commentary off   quiet the per-turn heckling
#/status
#/help
```

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
```

## License
Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in goulash by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
