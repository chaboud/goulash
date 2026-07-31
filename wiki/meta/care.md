# Care: why this program needs more of it than most

goulash sits between a person and their shell, all day, on every
keystroke. Nothing else in the stack has that position, and it changes
what a defect costs. A wrong answer in a chat window is a wrong answer
you close. A wrong byte here is *the terminal behaving strangely* while
someone is trying to work — and they will not know whether to blame
goulash, zsh, the emulator, or themselves.

That asymmetry is the whole argument for the discipline on this page.
It is not craftsmanship for its own sake. It is that the blast radius of
a small mistake is somebody's working day.

## The recurring failure: it looked like it worked

Every serious defect found so far shares one shape. Not a crash, not an
error message — a **silent no-op that renders as success**.

| What the user saw | What was happening |
|---|---|
| A setting saved, and shown | Written as a quoted key; never read back. Reverted at next launch |
| "goulash got slow" | A model reload on *every* ask, from the code written to avoid reloads — which also blew the prompt cache |
| An expert toggle that flipped once | The row it revealed was inserted *above* it, so the cursor slid off |
| `limit`, in the memory menu | Rendered, accepted Enter, did nothing |
| A green test suite | A differential comparing two empty buffers |

None announced anything. The failure is not the wrong value; it is that
nothing said so.

**The rule that follows:** a control that does not take must say so. Any
path that can silently do nothing is a bug, whatever it does the rest of
the time. See also [status-rows](../architecture/status-rows.md) — the
band exists precisely so the product has somewhere to be honest.

## What that demands of a change

**Verify the consequence you did not think of.** When a change has
several observable effects, verifying one manufactures confidence.
Removing a stop sequence was checked against "did an answer arrive" and
never "did the model stop talking". Splitting the suggestion chip into
two colours was checked on screen and broke every test matching its
text.

**Measurement outranks reasoning, and it has to be shown.** This is
[convention 8](wiki-conventions.md) pointed at code rather than pages.
A confident, wrong paragraph cost a full session: the slowness was
attributed to invisible reasoning by inference, when the wire had the
answer the whole time. A claim with no number beside it is a guess and
should be labelled one.

**Distrust the instrument.** More than one wrong conclusion here came
from a broken probe rather than a broken product — an all-zero timing
run, a regex eaten by a `+`, a test suite diagnosed while a benchmark
saturated the machine. Sanity-check the tool before the subject.

**Finish the whole edit.** A rename touches the table, the apply arm,
the help text, the config doc, the changelog, this wiki, and the tests —
*including tests' own fixtures*. A stale `slow = "ingest"` in a test
config hid an entire broken block, because an unknown value falls back
to the first option without complaint.

**Compose rather than accumulate.** Two copies of the same parse is one
copy and one latent divergence.

**Never solve timing with a sleep.** There is always a real signal: a
mark, a hook, an event, held input. See
[shell-integration](../architecture/shell-integration.md).

**The machine belongs to the user.** goulash suggests; it does not run
commands, probe binaries with `--help`, or block interaction. Long
sweeps and model loads run on a laptop somebody is using — say what they
cost and how to stop them.

## Status

Standing guidance, not a proposal. The engineering-facing version lives
in [`CLAUDE.md`](../../CLAUDE.md) at the repo root, where a contributor
or an agent will actually meet it; this page is the *why*, and the two
should be changed together.

See also: [wiki-conventions](wiki-conventions.md) ·
[provenance](provenance.md) · [state-of-play](../product/state-of-play.md)
