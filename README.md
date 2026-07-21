# goulash

*Working name.* **Goulash — Generic Overlay for Universal LLM-Augmented
SHells**: an LLM-aware veneer that wraps the shell you already use,
watches the session, and offers advice and executable suggestions while
leaving control with the user.

> Your shell, with a coach.

Design lives in the wiki: start at **[wiki/home.md](wiki/home.md)**, or
jump straight to the [architecture overview](wiki/architecture/overview.md)
and the [build plan](wiki/product/build-plan.md).

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
```

At the prompt: `#/model` lists installed models, `#/model <name>`
switches live, `#/status`, `#/help`.

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
