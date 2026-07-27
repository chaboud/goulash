# Ambient Research: Observant Salient Elevation

Every rung of the [engagement ladder](two-lane-engagement.md#when-slow-engages-a-ladder-not-a-toggle)
is triggered by a keystroke — `#?`, `#@`, `#`. But the terminal is a
continuous stream of intent, and the GPU is idle most of the time. The
trigger does not have to be a keystroke.

The value is not "an agent watches your terminal" — that has been tried
and it is annoying. It is **observant salient elevation**: noticing what
matters in what already happened, and having somewhere to put it that
does not interrupt.

## Why this is close

Three things that would normally be the hard part are already built and
load-bearing:

- **Background work that yields to the human.** The digest queue
  (`src/engine.rs`) is a `VecDeque` of jobs that always loses to an
  interactive ask, meters its progress, and cancels wholesale. Getting
  out of the way is the problem that is usually hard.
- **Late arrival is already safe.** Findings land at their **origin
  turn**, never at the top of the stack, so nothing arriving late can
  seize attention. That invariant is what makes unprompted work
  survivable at all.
- **The corpus already exists on disk.** `record.rs` has been writing
  `cmd` / `cmd_end` / `cwd` / `prompt` / `out` as timestamped JSONL for
  every session ever run. The chunks of the observed are durable and
  replayable today. Nothing reads them.

## The hierarchy is the throttle

[memory-hierarchy](memory-hierarchy.md) already describes the tree:
leaves (raw blocks) → markers → regions. What ambient research adds is
that **promotion between levels is the unit of work**, and that is
exactly what bounds it.

```
areas      ~N/k²   ← promoting here is rare and cheap
groups     ~N/k
blocks      N      ← promoting here is frequent and cheap
```

Each level is geometrically smaller than the one below, so the total
work available is bounded by the shape of the tree rather than by a
timer. You cannot spin, because there is only ever a finite amount of
cooking left to do. **The cooking of the hierarchy is the opportunity to
work, and it is the throttle.**

And the promotion step is not new machinery: **a promotion is a digest.**
`run_one_digest` already takes a large thing and produces a smaller one
under a budget, asynchronously, yielding to interactive work. Blocks → a
group summary is the same operation as file → digest, pointed at a
different input. The tier that exists for `#@` generalises.

Priority between levels is the obvious one: promote whatever is furthest
behind, deepest first, since a stale group makes every area above it
wrong.

## Three stages, three different throttles

Reasoning over what happened has three parts, and they need separate
governors because their costs are shaped differently:

| | | bounded by | converges? |
|---|---|---|---|
| **1. Condense** | ingest, compress, form the tree | **input** — there is a finite amount of unpromoted material | yes, naturally |
| **2. Infer** | derive facts from the tree | **nothing intrinsic** — you can always ask the tree another question | **no, without help** |
| **3. Raise** | elevate what is salient | **output** — the band holds one row, and few things deserve it | yes, naturally |

Stages 1 and 3 self-limit. **Stage 2 is the one that would burn a
laptop**, and it is where the design effort belongs.

### Everything is delta-driven, which is what makes stage 2 finite

Give inference the same property the other two have for free: run it
over **what changed**, never over the whole tree. New material, or
material whose inputs were invalidated. Then every stage owns a
work-list that empties, and "done" is the same statement three times.

Without that, stage 2 is an infinite loop wearing a feature's clothes —
there is always another cross-reference to draw.

### Drain deepest-first, with a fast path

The stages form a pipeline, so **the deeper stage always wins**:
inferring from a half-cooked tree wastes the work and produces facts
that then have to be invalidated. A burst of new blocks correctly pushes
inference back, and that self-tunes — while the human is busy the tree
is churning, so inferences drawn then would be short-lived anyway, and
the human does not want findings mid-flow either.

One exception, or the most useful finding arrives last: **high-salience
items skip the queue.** A build that just failed does not wait behind
the promotion backlog. Everything else takes its turn.

### The stages are a ladder, and rung one is free

They map onto the same shape as the
[slow ladder](two-lane-engagement.md#when-slow-engages-a-ladder-not-a-toggle),
which is better than three separate numbers because each rung is
independently useful:

| | runs | risk |
|---|---|---|
| `off` | nothing | none |
| **`condense`** | stage 1 | **none — it never speaks and never infers** |
| `infer` | 1 + 2 | writes artifacts; confabulation surface |
| `raise` | 1 + 2 + 3 | findings appear unbidden |

`condense` is the interesting rung: building the tree makes **every ask
the user makes themselves** better, with no surfacing risk whatsoever.
It is the one that could reasonably be on by default, and the opt-in
starts at `infer`.

Budgets sit on top: a per-window total (the ~30 seconds below) and,
within it, a cap per stage — so a pathological promotion backlog cannot
starve inference forever, and runaway inference cannot starve the tree.

### Inference must not eat its own output

Stage 2 writing durable facts creates a loop: inferred facts become
input to later inference. That is exactly how confabulation compounds —
a model reasoning about its own guesses three levels deep, with each
layer laundering the last one's uncertainty into apparent fact.

So provenance carries **depth**, not just origin, and inference over
inferred material is depth-limited. Raw is truth; a summary is a hint;
an inference from a summary is a weaker hint; an inference from *that*
is not worth having.

## Quiescence is provable, not a backoff

"We have reasoned over what we have, and we are done" is a **fixed
point**, not a heuristic:

> every unit above the salience threshold has been examined at its
> current input hash, no new blocks have arrived, and no artifact has
> changed that would invalidate a prior verdict.

When it holds, the worker returns to a blocking `recv` and burns
nothing. That is the difference between a daemon and a system that
converges — and it is the honest answer to "is this eating my battery":
not *we back off*, but **there is nothing left to think about, and here
is how you can see that.**

So it should be visible. The chrome already carries state; `settled`
versus `thinking` is one more word, and it is the word that makes the
whole mechanic trustworthy.

**The horizon comes free from the tiers.** An unbounded block set would
mean re-converging over an ever-larger corpus, but the
[fixed-count tiers](memory-hierarchy.md#the-structure-fixed-count-tiers-overflow-promotes)
mean the examinable set is bounded by construction: a block that falls
out of tier 0 is no longer a block, it is part of a group, and the group
is examined once instead of its members individually. There is no decay
heuristic to tune. **The set of things left to think about shrinks as
fast as it grows**, which is why the fixed point is reachable at all.

## Discipline: we are a command-prompt thing

**Rejected: an AC-only default.** Tying background work to the power
adapter is the wrong axis and a bad experience. The budget should be
coupled to *activity*, not to power state.

The rule instead — and the right shape for it is a **token bucket filled
by command activity**:

> Every command turn deposits tokens. Work spends them. An empty bucket
> means an idle machine, whatever else is outstanding. Unless explicitly
> told `#/study` or `#/cook`, that is all we do.

Four properties that make this better than a timer:

- **Idling is unconditional.** An empty bucket stops work even with a
  huge backlog, so "the user walked away" and "the machine went quiet"
  are the same event. Convergence stays opportunistic; we never need the
  fixed point to be *reached* for the laptop to be cool.
- **The activity that creates the work also funds it.** More commands
  means more to think about *and* more budget to think with, and the two
  scale together without a coefficient to tune.
- **Deposit by salience, not by count.** A failed build is worth more
  tokens than an `ls`, and a `#` ask — someone signalling confusion — is
  worth more still. The interesting work funds itself first.
- **A cap on the bucket.** An afternoon of commands must not bank hours
  of grinding for one idle evening. Burst credit, not a savings account.

Spend is per **unit of work** rather than per second, so a slow model
does not get charged for being slow and a fast one does not get to do
more for the same activity. What the user paid for is thought, not
wall-clock.

Two windows, both bounded, both already sensed:

- **After a prompt turn settles** — the human is reading or thinking,
  the GPU is free, and any keystroke cancels. *Typing is the cancel*;
  no new verb, no stolen Ctrl-C.
- **While a command runs** — the human is watching output, not typing.
  `cmd` state with no keystrokes is stealable time too, and it is time
  we are otherwise wasting.

This is what makes the "away for three hours" case safe without needing
the fixed point to be reached: we do not grind for three hours. We do
thirty seconds after the last action and stop. Convergence becomes
**opportunistic rather than a goal to chase** — we resume next time
there is action, and if the corpus is never fully cooked, nothing is
harmed.

`#/study` and `#/cook` are the explicit escape hatch for "go grind" —
user-initiated, so the cost is chosen rather than imposed.

## Two stores, and the rule between them

The split already exists implicitly: `memory.toml` is a global,
user-scoped, model-written store, and pins are project-shaped (they are
paths). Making it explicit:

| | holds | scope | travels |
|---|---|---|---|
| `~/.goulash/` | **habits** — your tools, your idioms, the fixes you keep re-deriving | you | with you, across machines |
| per-project artifacts (under `~/.goulash/`, keyed by project) | **content** — this tree's build system, layout, runbooks | that tree | only via explicit export |

**The invariant: global holds habits, never content.** That single rule
is what stops the worst failure — the global store learning something in
one client's repository and surfacing it in another's. "This user prefers
`rg`" is portable. "The auth service talks to Redis on 6380" is not, and
must never leave the tree it came from.

### We do not write to our targets

**All goulash writes go under `~/.goulash/`.** Project artifacts are
stored there, keyed by a stable project identity, not scattered into the
directories being worked in.

This was considered and rejected the other way round — a `.goulash/`
directory created inside each project — for reasons that are not
aesthetic:

- **Nobody gets to be `.DS_Store`.** A tool that litters other people's
  trees with its own bookkeeping is a tool people learn to `find -delete`.
  Writing into a directory we were merely *invited to read* is a
  category error.
- **Config-identity switching breaks it.** Field experience with a
  `.claude` in a home directory and a separate `~/.claude_employer`:
  switching between them corrupted state that assumed a single identity.
  So `$GOULASH_HOME` stays overridable, project artifacts are keyed by a
  stable project identity, and **switching homes starts cold rather than
  corrupt.** Cold is recoverable; corrupt is not.

### Session pins and location pins are different lifetimes

`#@ eero.md — use this for our debugging session` and "this is what is
true about this place" are not the same act, and collapsing them would
lose the distinction that matters. But they are the same *verb* at two
lifetimes:

| | scope | lives | restored |
|---|---|---|---|
| **session pin** — `#@ <file>` | this conversation | until the session ends or you unpin | no |
| **location pin** — pin *for here* | this tree | in `~/.goulash`, keyed by location | **on return** |

A location pin is the answer to "every time I come back to this repo I
end up pinning the same three files". Resolution is nearest-ancestor,
like `.git`, so pinning at a repo root applies throughout it — and the
restore is the user's own earlier decision coming back, not context
appearing from nowhere.

The chrome marker and the browser already exist; location pins are a
section in the same list rather than a second surface.

### `.paprikash` is a manifest, not a payload

Which resolves what `.paprikash` actually *is*: **the project's
suggestion of what your location pins should be.** Not a new mechanism —
a file the humans who own a tree commit, saying "here is what matters
here", which ingesting turns into location pins you own.

```toml
# .paprikash — committed by the humans who own this tree
research = "off"                  # a restriction: honoured on sight
read     = ["docs/", "Makefile"]  # a suggested pin set
avoid    = ["vendor/", "*.pem"]
notes    = "tests run with `just test`, never run migrations by hand"
```

Only the middle part is pinning. `research` and `avoid` are
restrictions, which are not pins at all and follow the sight rule below;
`notes` is prose, quoted to the model as *data this repository asserts*.

So the layering is clean, and each layer already has a home:

```
.paprikash        the project's suggestion      (in-tree, read-only, untrusted)
   ↓ ingest, on the user's Enter
location pins     this user's standing context  (~/.goulash, keyed by tree)
   ↓ every ask
session pins      what we are doing right now   (#@, ephemeral)
```

### The project speaks, we only listen

A project may contain a `.paprikash` — **human-authored, read-only,
never created or modified by goulash.** It is the mechanism by which a
repository tells goulash something about itself, and it is the *only*
in-tree file in the design.

Plausible content:

```toml
# .paprikash — committed by the humans who own this tree
research = "off"                  # stay dumb here
read     = ["docs/", "Makefile"]  # what actually matters
avoid    = ["vendor/", "*.pem"]
notes    = "tests run with `just test`, never run migrations by hand"
```

#### Finding one is a suggestion, not an action

A `.paprikash` is content from a repository you cloned — untrusted input
heading for a prompt, which is a live injection surface. So **nothing in
it is read into context until the user says so**, and the way they say
so is the mechanic goulash already has:

```
$ cd ~/src/someone-elses-thing
  ↓ suggestion: #@/ingest ./.paprikash
```

Discovery *suggests*; the user pulls with Down and presses Enter. **Their
own keystroke is the consent** — no new modal, no dialog, no trust
prompt, and it is the same gesture they use for every other suggestion.
The one primitive this whole product is built on turns out to be exactly
the right shape for "should I trust this file?".

Beyond consent, the real win is **discoverability**. A silent auto-ingest
means context arrives and the user never learns where it came from; a
suggestion tells them the file exists at all. And it is declinable by
inaction — no dismiss button, no modal, they simply do not pull it.

Two things it needs in order not to become nagware:

- **A decline is remembered too.** Three states per (tree, file hash):
  *unasked* / *offered and not taken* / *ingested* — and only the first
  produces a suggestion. Declining by inaction has to be recorded as a
  decline, or every `cd` into that tree re-asks forever, which is how a
  polite mechanic turns into an obnoxious one. Kept in `~/.goulash`
  against the project, so it survives sessions.
- **It rides the `cd` turn.** A directory change is a turn that almost
  never has a competing suggestion, whereas a command that just failed
  has one that matters far more. Discovery is the lowest-priority thing
  in the slot stack and should never displace a fix.

The file is content-hashed, so an edit re-opens the question — including
one that was previously declined, since the thing being declined has
changed — rather than silently riding in on old consent.

#### Restrictions are still honoured on sight

One exception, and it is the asymmetry that makes the rest safe:
**anything that makes goulash do *less* is obeyed without ingest.**

- `research = "off"`, `avoid = ["*.pem"]` — honoured on discovery,
  because the worst case from an untrusted source is that we are
  unhelpful, and because a tree that wants us quiet should not need
  every visitor to opt in first. That is precisely the case that matters
  when someone clones a repository full of things we should not be
  reading.
- Everything else — what to read, what to believe, what to say — waits
  for the ingest. Prose in `notes` is then quoted to the model as *data
  this repository asserts*, never as instruction.

So a hostile `.paprikash` can make goulash useless in that tree and
nothing worse. That is an acceptable worst case, and a far easier
property to hold than "sanitise arbitrary text", which nobody has ever
managed.

#### The setting

```toml
[context]
paprikash = "suggest"    # suggest | auto | off
```

- **`suggest`** *(default)* — notice one, offer the ingest, honour
  restrictions meanwhile.
- **`auto`** — ingest on sight. For people working only in trees they
  own, where the prompt is friction rather than safety.
- **`off`** — never look. Discovery itself is the thing being disabled,
  so nothing is read at all, including restrictions.

**Export is explicit.** A `.paprikash` is only ever written by a user
asking for one. A committed one means a new teammate's goulash starts
warm — it already knows the build system, the test command, why that one
script exists. That warm start is the strongest thing in this design,
and it stays opt-in on both ends: the project chooses to publish, and
the reader chooses to ingest.

## Salience: what deserves a pass

The missing primitive is not compute, it is ranking. Most of it needs no
model at all:

- a non-zero exit the [rules vendor](suggestion-vendors.md) could not fix
- the same command repeated three or more times with variations —
  someone is iterating, which means someone is stuck
- a long output tail nobody scrolled
- a new cwd with an unfamiliar project shape
- **a block adjacent to a `#` ask**

That last one is the good one: the human already told us they were
confused there. **The transcript is labelled data** — we do not have to
guess what was salient, because people have been marking it for us the
whole time.

Annealing then has a clean stopping rule: re-examine a unit only when
its *inputs* change — a file hash, a new related block, a new artifact.
Otherwise it is settled.

## What lands where

Ambient work has no origin turn, which is a problem given that findings
must land at their origin. It resolves by splitting the output:

- **Findings** — a command and a line — need a turn. But a failed build
  *is* a turn, with an ID, an exit code and an output tail. Ambient work
  provoked by a block lands on that block. No new machinery: the amend
  mechanic already targets a turn.
- **Artifacts** — a wiki page, a digest, a note — need no turn. They
  land in the store and are browsable through the
  [menu primitive](../interaction/settings-and-nav.md), which the pin
  browser already is.

Default output is **silence**. Slow already answers `PASS` for "nothing
better"; ambient needs the same primitive one level up — *nothing worth
keeping* writes nothing. An accumulating pile of mediocre artifacts is
the real failure mode, and it arrives slowly enough that you do not
notice until it is bad.

And the voice does not change: **show up with a recommendation, that is
the whole list.** "I noticed you have been struggling with…" is
insufferable. A command and a line.

## Confabulation compounds

A wrong artifact poisons every later pass that reads it. This promotes
the content-hashed artifact cache from an optimisation to a
**correctness requirement**: every artifact records the input and hash it
was derived from, so staleness is detectable and provenance is
checkable. Derived data that conflicts is thrown away and re-derived
rather than merged — which also answers staleness for free.

## `#/study` and ambient are the same machine

Offline over the transcript history, online over the current session:
same salience ranking, same promotion step, same artifact store,
different clock. Worth building it that way from the start rather than
discovering it later.

## Status

**Nothing here is built.** What exists: the yielding background queue,
the digest tier that promotions would reuse, findings-land-at-origin,
`PASS`, the menu primitive, per-lane bindings so this can run on another
box, and the transcript corpus.

What is missing, roughly in order:

| | | size |
|---|---|---|
| **A** | Blocks addressable in memory. `Block` is built in `session.rs` then flattened into a `ctx_log: String`; keep the `Vec<Block>` (`blocks_seen` is already the ID counter). | ~50 lines |
| **B** | An activity signal to the engine. The worker's `idle` means "my queue is empty", not "the human is idle". The session knows and never says. | ~30 lines |
| **C** | The artifact store: keyed by project, content-hashed, with provenance. | ~250 lines |
| **D** | Promotion bookkeeping: `unexamined → examined(hash, verdict)`, plus the level-behind priority. Falls out of A. | ~50 lines |
| **E** | The 30-second discipline, and the horizon. | ~50 lines |

**The first slice** — ambient on failed blocks only, at idle, producing
findings that land on that block's turn, no artifact store yet. A + B
plus a job type reusing `run_research` verbatim. It answers the only
question that matters before spending days on C: **does a finding you
did not ask for feel like a gift or an intrusion?**
