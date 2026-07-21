# Suggestion Vendors

The [suggestion list](../interaction/suggestion-list.md) doesn't care
where suggestions come from. **Vendors** are pluggable sources behind one
interface; the LLM is merely the most expensive one. This is what makes
the no-LLM tier genuinely useful instead of "politely absent."

## The vendor ladder

| Vendor | Cost | Latency | Needs LLM? | Vends |
|---|---|---|---|---|
| **Rules** (thefuck-style) | free | µs | no | corrections after a failed command |
| **History/fuzzy** | free | µs | no | fish-style completions from your own history; typo fixes via edit distance against PATH, history, and cwd contents |
| **N-gram** | free | µs | no | next-command prediction from personal command bigrams ("after `git add` you usually `git commit`") |
| **Watcher** ([local model](llm-engine.md)) | ~free | ms–s | local | contextual drafts, staleness checks |
| **Thinker** (API model) | $$ | s | yes | hard, multi-step suggestions |

All vendors run through the same pipeline: same
[staleness tagging](../interaction/suggestion-list.md), same ID-bound
acceptance, same [block-history](block-history.md) recording. Each
suggestion carries **vendor attribution** so the list can show where it
came from.

## The rules vendor (thefuck, done better)

thefuck's model — last command + exit status + output → corrected
command — is exactly suggestion-vending, and goulash has **strictly
better inputs**: thefuck must scrape or even *re-run* commands to see
their output; goulash already observed the command, exit code, cwd, and
output in block history. No re-execution hack, no Python startup lag.

- Reimplement a **curated subset** of the rules natively in
  [Rust](implementation.md) (thefuck is MIT — port and attribute; also
  survey existing Rust reimplementations of it for prior art). Its
  ~200-rule long tail is niche; start with the top few dozen
  (typo'd binary, `git push --set-upstream`, missing `sudo`, wrong
  package-manager verb, …).
- Fire on `exit != 0`, vend only on a **crisp match** — a correction
  appearing in the status row before the user even reacts is magic; a
  wrong guess after every failure is noise. Confidence-gate hard.
- UX beats the original: no `fuck` invocation needed — the fix is
  already sitting in the list when you look down. (An explicit trigger
  alias can exist for the faithful.)

## Rules stay on even when LLMs are present

Deterministic vendors are not a fallback tier — they're the **fast
path**, always on. A rules hit lands in microseconds while the watcher
is still thinking; if both produce the same fix, dedupe keeps the
rules-attributed one and the LLM spend is saved. The
[probe chain's](llm-engine.md) "run dumb" level thus isn't dumb at all:
overlay + history + rules/fuzzy/n-gram vendors make goulash worth
installing before any model is configured — which is also the killer
zero-setup demo: install, typo `gti status`, the fix is waiting in the
status row.

## Not doing (yet)

A tiny bundled neural continuation/spell-check model. The deterministic
vendors plus personal-history n-grams likely cover most of that value at
zero complexity; revisit only if the gap proves real —
[open-questions](../product/open-questions.md).
