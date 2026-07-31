# Working on goulash

goulash sits between a person and their shell, all day, every keystroke.
That position is the whole product and it is unforgiving: a bug here is
not a wrong answer in a box you can close, it is the terminal behaving
strangely while someone is trying to work. Land the experience or do not
ship the change.

## The failure that keeps happening

Every serious bug in this codebase so far has had the same shape:
**it looked like it worked.**

- The settings menu saved a value, showed it, and forgot it at the next
  launch — a dotted section written as one quoted key.
- Context negotiation sent `0` to mean "leave the model alone". The
  server read it as a real request and reloaded, on every single ask,
  which also threw away the cached prompt.
- The expert toggle inserted a row above itself, so the cursor slid off
  and you could not turn it back off.
- A test compared goulash against bare zsh and reported green four times
  while both sides were empty.
- `limit` rendered as a row that silently ignored Enter.

None of these announced anything. That is the bar: **a control that does
not take must say so.** Silence is the bug, not just the wrong value.

## Rules that follow from it

**Verify the consequence you did not think of.** When a change has
several observable effects, checking one manufactures confidence.
Removing a stop sequence was verified by "did an answer arrive" and
never by "did the model stop talking". Splitting the suggestion chip into
two colours was verified on screen and broke every test matching its
text. Ask what else moved.

**Measure before you diagnose, and show the measurement.** A confident
paragraph of reasoning cost a whole session here: the slowness was
attributed to invisible reasoning, from inference, and the real cause
was a per-ask model reload that took ten minutes to find once anyone
actually looked at the wire. If a claim has no number next to it, it is
a guess — say so.

**Distrust your own probes.** More than one wrong conclusion here came
from a broken script rather than a broken product: an all-zero timing
run, a regex eaten by a `+`, an e2e suite diagnosed while a benchmark
saturated the machine. Sanity-check the instrument before the subject.

**Finish the whole edit.** Renaming a setting means the table, the apply
arm, the help text, the config doc, the changelog, the wiki and the
tests. Half a rename is worse than none: the value persists, the arm
never fires, and nothing complains. Grep for the old name everywhere,
including tests' own fixtures — a stale `slow = "ingest"` in a test
config hid a whole broken block.

**Compose, do not accumulate.** If parsing the same string happens in
two places, it belongs in one function with a test. `split_row` exists
because the same `name: value` parse was inline in two apply paths and
one of them forgot the parenthesised aside.

**Never solve timing with sleeps.** There is always a real signal —
a mark, a hook, an event, held input. Sleeps here are a standing
instruction to find the actual flow instead.

**Do not run things on the user's behalf.** goulash suggests commands;
it does not execute them, probe binaries with `--help`, or block
interaction. Deriving facts from the filesystem is fine. Running a
third-party binary to learn about it is not — a shimmed command can
install, update, or change the machine.

**Ask before spending the user's machine.** Benchmarks, model loads and
long sweeps run on the laptop someone is trying to use. Say what it will
cost and how to stop it, and clean up after.

## Before calling something done

- `cargo test` and `cargo clippy` clean, and every commit builds on its
  own — this repo bisects.
- `python3 tests/e2e.py` — and know the baseline, because a failure
  count means nothing without one. `python3 tests/e2e.py <test_name>`
  runs a subset.
- Docs updated in the same change, not after: README, CHANGELOG, and the
  wiki page that owns the concept.
- Say plainly what you did NOT verify. An honest gap is worth more than
  a confident summary.
