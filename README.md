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

Working today (no LLM yet): transparent PTY wrapper with reserved
status row, session sensing and transcripts, shell integration
(command blocks with text/exit/cwd), and deterministic thefuck-style
suggestions — typo a command, and with zsh integration press **Down**
(past end of history) to pull the fix into your prompt; `#` asides are
intercepted and recorded. Generic shells: **Alt-Down**. LLM engines
land next.

## Build & run

```
cargo build
./target/debug/goulash            # wraps $SHELL
./target/debug/goulash zsh        # or name a shell
```

Shell integration (recommended — gives goulash real command blocks:
command text, exit codes, cwd):

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
