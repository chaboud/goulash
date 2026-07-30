# Situated Context: what goulash knows that a chatbot doesn't

*Speculative. Written after the [characterization sweep](../../bench/) as
a design theory, not a plan. Numbers are measured; the conclusions drawn
from them are argument.*

## The thesis

goulash's advantage over pasting a question into a browser is not a
better model. It will usually be a *worse* model — a 4B running locally
against a frontier model in a tab. The advantage is that goulash is
**situated**: it is inside the machine the answer is about.

Almost none of that situation currently reaches the prompt. The session
log carries commands and output; everything else the model must guess.
And the sweep shows it guesses badly in exactly the ways being situated
would fix.

## Tier 1 — facts goulash already has, and does not send

The cheapest possible wins. All static or near-static, all one line.

### Platform and userland

**6.9% of 2395 vended commands used GNU-only syntax on a Darwin box.**

| form | count | on BSD |
|---|---|---|
| `grep -P` | **114** | no `-P` at all — fails |
| `du --max-depth` | 40 | wants `-d` |
| `ls --time-style` | 5 | not a flag |
| `xargs -r` | 3 | not a flag |
| `stat -c` | 2 | wants `-f` |
| `find -printf` | 2 | not supported |
| `date -d` | 1 | wants `-v` |

These are not subtle reasoning failures. They are a model assuming Linux
because most shell text on the internet is Linux. `uname -s` plus a
sentence — *"BSD userland: `sed -i ''`, `du -d`, no `grep -P`"* — is a
handful of tokens against a 6.9% failure rate.

Worth noting the sweep *also* asked "what does the `-P` flag do in grep"
as an explain-only control, and models answered confidently about
Perl-compatible regex. On this machine `-P` does not exist. Every one of
those answers was wrong in situ and right in the abstract, which is
precisely the failure mode a situated assistant should not have.

### Installed executables

goulash already computes `path_executable_set()` — every binary on PATH —
and uses it *only* to decide whether an untagged line looks like a
command. The model never sees it.

On this machine: `jq` yes, `ffmpeg` yes, `tree` yes, `zstd` yes — but
`fd` **no**, `rg` **no**, `gsed` **no**.

And the seeded memory in the sweep read *"prefers fd over find"*.
Models duly reached for `fd`. **`fd` is not installed.** The memory
feature, meant to personalize, was steering toward a tool that does not
exist — memory without grounding is worse than no memory, because it
carries authority.

The full list is too big for a prompt, but the *relevant* subset is not:
the tools a shell assistant would reach for is a set of maybe forty
names, and reporting which of those are present is one short line.

### Shell and terminal

Which shell (`zsh` globbing, `**`, arrays differ from bash) and the
terminal width — a command that must fit a status bar is a different
answer from one that may sprawl.

## Tier 2 — cheap state, already partly present

The session log carries command, exit code and output tail. What it does
not carry:

- **Structured git state** when in a repo — branch, dirty/clean, ahead
  or behind. `git-undo` answers proposed stashing a leaked list of the
  *bench's own* files because the model had no model of the repo.
- **The shape of data being asked about.** `jq-extract` had `cat
  data.json` in the log, and models still guessed `.[].items[].name`
  against a `{"items": [...]}` document. The information was present as
  raw text and not used. A one-line *schema* — `object with .items[]:
  {name, status, bytes}` — is smaller than the JSON and far harder to
  misread.

That second point is the interesting one: **more context did not help;
better-shaped context would have.** Raw output is what goulash has;
summarized structure is what a model can act on. That is an argument for
the [rolling watcher](memory-hierarchy.md) doing real work — not
compaction for budget's sake, but *reshaping for legibility*.

## Tier 3 — retrieval, for the cases reasoning cannot reach

`git reset --soft` vs plain `git reset` sank 9 of 32 answers, and those 9
were *confidently* wrong: the command unstages, the prose promised it
would keep changes staged. No amount of environment context fixes that.
The distinction is not situational — it is documented, in
`git-reset(1)`, in a paragraph the model half-remembers.

So: retrieve it. The obstacle is ordering — you must know which command
before you can fetch its documentation. Which suggests a **two-pass
shape**:

1. cheap pass: *what tool would answer this?* (one word, ~50ms locally)
2. fetch the local `--help` or the man page synopsis for that tool
3. real pass: answer with that text in context

The retrieved text is authoritative *for this machine* — the local man
page describes the local binary, so it fixes the BSD/GNU problem and the
flag-semantics problem with one mechanism. It costs a second inference
and a page of tokens, which is exactly the kind of trade the
[two-tier engine](llm-engine.md) exists to make: watcher-tier does pass
1, thinker-tier does pass 3.

A cheaper cousin: **verify instead of inform.** Before vending, check the
command's flags against the local binary's help text. Do not fix it —
just decline to vend, or mark it unverified. That catches all 114
`grep -P` cases without a second inference.

## Tier 4 — the feedback loop nobody else can have

This is the one that is genuinely goulash's alone.

goulash sees what happens *next*. Whether the user pulled the suggestion
down. Whether they edited it before running. What they changed. Whether
it exited zero. A browser chat sees none of this; it hands over an answer
and goes blind.

Every accepted-and-run suggestion is a labelled positive. Every
pulled-then-edited one is a **correction pair** — the diff between what
was suggested and what was run is the most precise training signal in the
entire system, and it is per-user and per-machine. Every suggestion
ignored is a weak negative.

What that could buy, in rough order of ambition:

- **Immediate**: if a suggestion was run and failed, feed the failure
  back for a second attempt. goulash already has the exit code.
- **Session**: "you edited my last three suggestions to use `-d` instead
  of `--max-depth`" — a within-session correction that fixes the whole
  BSD class after one observation.
- **Durable**: promote repeated corrections into
  [agent memory](agent-memory.md) — but *grounded* ones, derived from
  observed behaviour rather than asserted. That is the fix for the `fd`
  problem: a memory earned by watching, not typed in.

The privacy shape is favourable too — all of it stays local, and it is
observation of the user's own machine rather than egress.

## Tier 5 — asking for checkable answers

Softer, but cheap:

- **Prefer forms that fail safely.** `git reset --soft` over `git reset`
  is not just more correct here, it is more *recoverable*. A vend-bias
  system could weigh recoverability alongside confidence.
- **Ask for the dry run.** For destructive shapes, vend `rm -i` or the
  `--dry-run` form and let the user drop the guard. The
  [vend-bias dial](../product/build-plan.md) is the natural home.
- **Self-consistency on hard asks.** Two samples, compare; disagreement
  is a confidence signal. Costs a second inference, only worth it where
  the bias dial says confidence matters.

## What I would test first

Ordered by measured-failure-fixed per token spent:

1. **Platform line** (~15 tokens) — 6.9% of commands are wrong for the
   local userland. Almost certainly the best ratio in the whole system.
2. **Relevant-tools line** (~30 tokens) — kills the `fd` class, and
   grounds memory in what exists.
3. **Verify-before-vend** (no tokens, pure local check) — catches the
   same 6.9% from the other direction; the two together would say
   whether informing or checking is the better mechanism.
4. **Structured summaries over raw tails** — the `jq-extract` evidence
   says shape beats volume, and it is testable with the harness as-is.
5. **Two-pass retrieval** — the only thing that plausibly touches
   `git reset --soft`. Most expensive, most interesting.

The first three are measurable with a Pass-P-style wording experiment in
an afternoon. The last two are architecture.

## The uncomfortable part

A frontier model in a browser tab would get `--soft` right without any of
this. Situated context is not a way to make a 4B into a frontier model;
it is a way to make it right about *this machine*, which the frontier
model cannot be. The two failure modes are different, and only one of
them is goulash's to fix.

Which suggests the honest framing: **situated context closes the gap on
environment errors, and does nothing for knowledge errors.** 6.9% is the
first kind. `git reset --soft` is the second. Knowing which is which
should decide where the effort goes — and probably also decides which
tier of model gets asked.
