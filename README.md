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
brew install goulash                                       # via Homebrew
curl -fsSL https://goulash.dev/install.sh | sh             # prebuilt binary
```

Releases are tag-driven (`v*` → `.github/workflows/release.yml`):
binaries for mac (arm64/x86_64) and linux (x86_64/arm64), with the
in-repo Homebrew formula refreshed automatically.

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

At the prompt: `#/model` lists installed models, `#/model <name>`
switches live, `#/status`, `#/help`.

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
