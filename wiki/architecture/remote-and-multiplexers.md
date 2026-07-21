# ssh and tmux Boundaries

Goulash treats remote sessions and multiplexers as **boundaries**, not
things to pierce transparently.

## ssh

- Goulash running *outside* ssh regards the entire remote session as a
  single [opaque interactive child](opaque-blocks.md): it knows an ssh
  session to a given host ran for some duration and exited, nothing more.
- Remote awareness requires Goulash — or compatible
  [shell markers](shell-integration.md) — installed on the remote host.

## tmux

- Goulash *outside* tmux sees tmux itself as one interactive child, not
  each pane. The pane-level semantics are invisible from the outer PTY.
- For full semantics, run Goulash **inside each tmux pane** (i.e.,
  tmux's `default-command` launches `goulash "$SHELL"` per pane).

## Why not pierce?

Both cases are just instances of the general rule from
[input ownership](input-ownership.md): once another program owns the
terminal, Goulash forwards bytes and records lifecycle. Trying to parse
a remote shell's or a multiplexer's inner state from the byte stream is
the same trap as process-tree walking — fragile guessing where a clean
integration point (run Goulash where the shell actually is) exists.

## Open items

See [../product/open-questions.md](../product/open-questions.md) for
unresolved details (e.g., ergonomics of per-pane launch, remote marker
protocol).
