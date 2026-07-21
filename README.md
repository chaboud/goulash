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

Milestone 0 in progress: transparent PTY wrapper with the reserved
status row (shrunken-winsize trick), config loading, resize and
exit-code propagation.

## Build & run

```
cargo build
./target/debug/goulash            # wraps $SHELL
./target/debug/goulash zsh        # or name a shell
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
