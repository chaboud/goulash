# Machine facts: what goulash knows that a chatbot doesn't

*Design theory written after the [characterization sweep](../../bench/).
Tier 1 shipped in 0.4.0 as `engine.divulge`; the later tiers are still
argument. Numbers are measured; conclusions drawn from them are not.*

**Naming.** This page used to be called "situated context" and the
setting `situated`. Both were jargon — the word does no work a reader
can use. The shipped setting is `engine.divulge`, the code that derives
these lines is `src/facts.rs`, and this page is named after them.

## The thesis

goulash's advantage over pasting a question into a browser is not a
better model. It will usually be a *worse* model — a 4B running locally
against a frontier model in a tab. The advantage is that goulash is
**inside the machine the answer is about**.

Almost none of that reaches the prompt. The session log carries commands
and output; everything else the model must guess. And the sweep shows it
guesses badly in exactly the ways local knowledge would fix.

## Tier 1 — facts goulash already has, and does not send

The cheapest possible wins. All static or near-static, all one line.

### Platform and userland

**2.1% of 4355 vended commands used GNU-only syntax on a Darwin box** —
91 commands, counted by `bench/gnucheck.py`.

| form | count | on BSD |
|---|---|---|
| `du --max-depth` | **70** | wants `-d` |
| `grep -P` | 11 | no `-P` at all — fails |
| `ls --time-style` | 3 | not a flag |
| `xargs -r` | 3 | not a flag |
| `stat -c` | 2 | wants `-f` |
| `find -printf` | 2 | not supported |

> **Correction.** This table first read **6.9%** with `grep -P` at 114,
> and that number was wrong — 68% of its hits came from questions whose
> *subject* was the flag. Asked "what does `-P` do in grep", a model
> replying `grep -P "<regex>" file` is answering, not erring. Excluding
> explain-type questions, and hits where the flag appears in the question
> text, the rate is 2.1%. The shape changes with it: this is
> overwhelmingly **one flag**, not a broad Linux-assumption problem.

These are not subtle reasoning failures. They are a model assuming Linux
because most shell text on the internet is Linux. `uname -s` plus a
sentence — *"BSD userland: `sed -i ''`, `du -d`, no `grep -P`"* — is a
handful of tokens, and it targets a narrow, concentrated error.

The narrowness cuts both ways, and it is the honest read: a 2.1% rate
dominated by a single flag is a much weaker argument for a general
context-divulging feature than 6.9% spread across seven forms would have
been. Tier 1 still earns its ~50 tokens (measured free once cached), but
it is a small fix to a small problem, not the headline it looked like.

Worth noting the sweep *also* asked "what does the `-P` flag do in grep"
as an explain-only control, and models answered confidently about
Perl-compatible regex. On this machine `-P` does not exist. Every one of
those answers was wrong here and right in the abstract — the failure
mode an assistant running *on your box* should not have. (Those rows are
excluded from the 2.1% above; being right in the abstract is what was
asked for.)

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

## What to test, and what got settled

Ordered by measured-failure-fixed per token spent:

1. **Platform line** (~50 tokens) — **SHIPPED** as
   `engine.divulge.platform = true`. Verified free once cached:
   per-token prompt-eval is identical with and without it (750 vs 760 µs
   by turn 10).
2. **Relevant-tools line** (~30 tokens) — **SHIPPED but default off**
   (`engine.divulge.tools`). It targets absent-tool references, which
   fire 25 times in 4002 commands, and it carries an unsolved curation
   problem: which tools, maintained by whom. The full-PATH variant
   (`divulge.full_path`, ~3900 tokens) showed **no benefit at 7× the
   context** and nearly doubled prompt-eval — debug only.
3. ~~**Verify-before-vend**~~ — **REJECTED, and it should never have
   been proposed.** The idea was to run the tool (`--help`, `command
   -v`) before suggesting it. goulash does not run things. Suggesting a
   command the user chooses to execute is the entire safety model; going
   and executing something to check it inverts that. It is also unsafe
   on its own terms — if an executable has been replaced, invoking it
   with `--help` may install, update, or modify the system, and goulash
   would have done that unprompted. See
   [positioning](../product/positioning.md).
4. **Structured summaries over raw tails** — the `jq-extract` evidence
   says shape beats volume, and it is testable with the harness as-is.
   **OPEN.**
5. **Two-pass retrieval** — the only thing that plausibly touches
   `git reset --soft`. Most expensive, most interesting. **OPEN.**

Items 4 and 5 are architecture, not a wording experiment.

## The uncomfortable part

A frontier model in a browser tab would get `--soft` right without any of
this. Machine facts are not a way to make a 4B into a frontier model;
they are a way to make it right about *this machine*, which the frontier
model cannot be. The two failure modes are different, and only one of
them is goulash's to fix.

Which suggests the honest framing: **machine facts close the gap on
environment errors, and do nothing for knowledge errors.** The 2.1% is
the first kind. `git reset --soft` is the second. Knowing which is which
should decide where the effort goes — and probably also decides which
tier of model gets asked.

And on the corrected numbers the second kind is much the larger problem.
`git-undo` was answered correctly by 16 of 48; nine more were
*confidently wrong* — plain `git reset` with prose asserting it keeps
changes staged ([QUALITY](../../bench/QUALITY.md) §5). No amount of
telling the model about this machine fixes that.
