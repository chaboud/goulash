# Implementation: Language, Crates, Config

## Language: Rust

Decided. The workload is exactly Rust's home turf: PTY plumbing, termios
and job-control ioctls, a VT-state tracker in the hot byte-forwarding
path, FFI to local inference engines, and **single static binary**
distribution — a shell wrapper must be a zero-dependency install.

### Ecosystem starting points

| Need | Candidate crates |
|---|---|
| PTY allocation, cross-platform | `portable-pty` (from WezTerm), or raw `nix`/`rustix` |
| termios / `tcgetpgrp` / winsize / SIGWINCH | `nix` / `rustix` |
| VT-state tracking ([status-rows](status-rows.md)) | `alacritty_terminal`, `wezterm-term`, `vt100` — steal the parser, not the renderer |
| Async engine loop, provider I/O | `tokio` |
| llama.cpp | FFI bindings, or HTTP to `llama-server` (see [llm-engine](llm-engine.md)) |
| Config | `serde` + `toml` |

The core stays POSIX-portable; platform-specific code is confined to
provider plugins ([llm-engine](llm-engine.md)) and packaging.

## Config: `~/.goulash/`

A directory, not a lone file — history storage needs a home anyway:

```
~/.goulash/
  config.toml      main config: reserved rows, split height, keybinds,
                   provider selection, autonomy defaults
  providers.toml   per-provider settings and credentials references
  history/         block-history store (memory tree lives here)
  state/           llama.cpp KV snapshots, suggestion state, etc.
```

- TOML, edited by hand or by the in-terminal
  [settings UI](../interaction/settings-and-nav.md) — UI changes write
  back to the file, so there is one source of truth.
- Ship with a **functional default config**: goulash must do something
  useful on first run with zero editing — see the provider probe chain
  in [llm-engine](llm-engine.md).
- Open: honor `$XDG_CONFIG_HOME` as an alternative root
  ([open-questions](../product/open-questions.md)).
