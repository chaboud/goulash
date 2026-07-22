# goulash

**Goulash — Generic Overlay User-LLM-Augmented
SHell**: an LLM-aware veneer that wraps the shell you already use,
watches the session, and offers advice and executable suggestions while
leaving control with you, the user.

> Your shell, with a coach.

Design lives in the wiki: start at **[wiki/home.md](wiki/home.md)**, or
jump straight to the [architecture overview](wiki/architecture/overview.md)
and the [build plan](wiki/product/build-plan.md).


## Install

```sh
cargo install goulash                                      # from crates.io
brew tap chaboud/goulash https://github.com/chaboud/goulash
```

Releases are tag-driven (`v*` → `.github/workflows/release.yml`):
binaries for mac (arm64/x86_64) and linux (x86_64/arm64), with the
in-repo Homebrew formula refreshed automatically.

## Usage

Run `goulash` and use your shell exactly as before. A small goulash
area lives at the bottom; everything below is reachable with `#`, `##`,
and the arrow keys.

### `#` — ask a question, get an answer + a pullable command

Type `#` and a question at your prompt (it's a shell comment — recorded
in history, never executed):

```text
$ #how do I list files with human-readable sizes?
$ ▊
─ ↓ suggestion: ls -lh ─────────────── # message to chat · #/help for help ─
 #how do I list files with human-readable sizes?
 Use ls -lh; -h prints sizes like 1.2M instead of bytes.
                                           goulash │ zsh │ prompt # 80x20+4
```

The orange chip on the rule is a **suggested command**. Press **Down**
(past the end of history) and it lands on your prompt for editing —
Enter runs it, as your own command. With `commentary` on (the default),
goulash also reviews each command turn unprompted and may leave one tip
in the same spot.

### Arrows — one spatial axis

Shell history lives above your prompt; goulash's suggestions live
below. Up and Down just move along the line:

```text
   ↑  zsh history (as always)
── your empty prompt ──────────────── neutral
   ↓  newest suggested command       ( ↓ again: older · ↑: back up )
```

Down past the newest keeps going into the **history of suggested
commands** — even ones whose moment has passed — with the position
(`↑ 3/7 ↓`) shown at the right of the rule. Up retraces to your empty
prompt, and past that it's plain zsh history. Editing the line at any
point ends the browsing; your text is never touched.

### `##` — chat, without retyping `#`

`##` (or `## question`) flips into a chat panel; follow-ups need no
prefix. The shell keeps running above.

```text
$ ##how could I find large files in /Volumes?
─ ## chat ──────────────────────────── ⏎ send · ↓ command · ## or esc back ─
 # how could I find large files in /Volumes?
 goulash: use find with -size — the suggestion below does it.
 ## can you skip directories I can't read?▊
 ↓ suggestion: find /Volumes -size +100M 2>/dev/null
                                           goulash │ zsh │ prompt # 80x16+8
```

The suggestion sits at the **bottom of the panel**: Down moves onto it
(it turns orange — `↑ find /Volumes … · 1/7`), Down again browses
older, and **Enter places the selected command on your real shell
line** — where a second Enter executes it. Goulash never runs commands
itself. `##`, Esc, or opening a fullscreen app returns you to the
shell.

### `#/` — goulash's own controls

```text
#/model            modal model picker: type to filter, ⏎ selects & saves
#/model NAME       try a model for this session (add `save` to persist)
#/memory on        give the model a small pinned memory (REMEMBER/FORGET)
#/commentary off   quiet the per-turn heckling
#/status · #/help
```

## Build & run

```
cargo build
./target/debug/goulash            # wraps $SHELL
./target/debug/goulash zsh        # or name a shell
```

**Shell integration is automatic** for zsh and bash launched with plain
flags — goulash injects its hooks (ZDOTDIR trick / `--rcfile` wrapper)
on top of your normal rc files, no editing required. That's what powers
command blocks, `#` asides, and the plain-Down suggestion pull.

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
## Status

Working today: transparent PTY wrapper with reserved status row,
session sensing and transcripts, shell integration (command blocks
with text/exit/cwd), deterministic thefuck-style suggestions — typo a
command and press **Down** (past end of history) to pull the fix into
your prompt; Down again cycles candidates. **`#` asides are answered by
a local LLM**: if ollama is running (`localhost:11434`), goulash finds
it automatically and `# why did that fail` gets a one-line answer in
the bar, with recent command context. No ollama → features degrade
gracefully. Generic shells: **Alt-Down**.

```toml
# ~/.goulash/config.toml (all optional)
[engine]
provider = "auto"          # auto | ollama | none
host = "http://127.0.0.1:11434"
# model = "gemma4:e2b"     # pin exactly; beats favorites and auto-pick
favorites = []             # e.g. ["gemma4:e2b", "llama3.2:1b"] — first
                           # installed favorite wins; else smallest model
keep_alive = "30m"         # keep the model resident ("" = server default)
stream = true              # tokens flow into the bar as they arrive
prewarm = true             # load the model in the background at bind/switch
max_tokens = 256           # runaway backstop per answer (not a latency lever)
# debug = true             # record raw model output in the transcript
num_ctx = 8192             # requested context window (bounds KV memory)
context_max_chars = 12000  # prompt budget; log epoch-trims to half
tail_chars = 800           # per-command output kept in context
```

The goulash area holds a **fixed height** (rule row with the pullable
suggestion cutting in, question, answer, chrome bottom-right — knobs:
`[status] band`, `band_rows`); rows blank when idle, so the terminal
never resizes mid-session. With `commentary` on (default), the model
reviews each command turn and may volunteer one short tip — toggle live
with `#/commentary`. If the model proposes a
command (`CMD:` line), it drops into the suggestion list — press
**Down** to pull it into your prompt. Reasoning-model "thinking" is
disabled (it burned the whole token budget invisibly). `#` asides are
recorded in shell history — **Up** recalls them to edit and re-fire.
Debugging: everything lands in `~/.goulash/history/session-*.jsonl`.

At the prompt: bare `#/model` opens a **modal selector** — type to
filter, ↑↓ to move, Enter selects *and persists* to config.toml
(`auto` restores the probe order), Esc backs out. Typed forms:
`#/model <name>` switches for this session only; `#/model <name> save`
persists. Persistence is guarded by a **crash fuse**
(`~/.goulash/state.toml`): a model that took the machine down mid-load
is never auto-bound again until an explicit retry survives. Also
`#/status`, `#/help`.

**`##` chat mode**: `## <question>` (or bare `##`) flips focus to a
chat pane — the goulash area grows a few rows, and follow-ups need no
`#` prefix. When a command comes up, **Up** hands it to your real
shell line (your editor, your Enter — goulash never runs commands);
`##` or Esc returns to the shell. **Down** at the prompt cycles a
history of past suggested commands (with their chat text) even after
the live suggestion cleared.

**Memory** (off by default): `#/memory on` gives the model a flat,
slot-limited pinned store (25 slots × ≤240 chars, durable in
`~/.goulash/memory.toml`) baked into the prompt's stable prefix. The
model saves with a `REMEMBER: <note>` line and drops with
`FORGET: <id>` — a revision is both in one reply. You hold the same
levers: `#/memory add|delete|modify|find|limit`, plus bare `#/memory`
for status.



## License

Dual-licensed under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. Unless you explicitly
state otherwise, any contribution intentionally submitted for
inclusion in goulash by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or
conditions.
