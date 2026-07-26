# Model Capabilities: Thinking Is Not One Dial

A settings dial that silently does nothing is worse than no dial. That
is what `#/thinking` was before this: one `think` field sprayed at every
model, and when a 20B reasoner came back with a blank bar, goulash could
only shrug and list suspects.

The models genuinely disagree, and not on a spectrum:

| Model | Wants | Behaviour |
|---|---|---|
| `gemma3`, `llama3`, `mistral` | nothing | no reasoning at all; some providers **reject** the field rather than ignore it |
| `qwen3`, `granite3.3`, `cogito` | `think: true` | switchable — off really is off |
| `gpt-oss` | `think: "low"\|"medium"\|"high"` | named effort levels, and it will spend the whole budget |
| `deepseek-r1`, `qwq`, `magistral` | `think: true` | reasons **whether or not you ask** |

That last row is the one that breaks naive handling: `thinking = off`
does not stop the spend, so the response budget still has to cover it or
the answer arrives empty.

## Three sources, in increasing authority

[`src/models.rs`](../../src/models.rs) resolves a `Caps` per bound model
from three places, each covering what the others cannot:

1. **The table** — family patterns matched **longest-prefix-first**, so
   `phi4-reasoning` beats `phi4` and `qwen3-coder` beats `qwen3` without
   depending on list order. The table knows the two things no provider
   reports: which *dialect* the request takes, and how many tokens the
   model realistically burns reasoning.
2. **The provider** — ollama's `/api/show` returns a `capabilities`
   list. Ground truth for *whether*, silent on form and cost. It is
   metadata only (no weights load), so it rides the bind path without
   reviving the [blocking-prewarm](../product/state-of-play.md) stall.
3. **The user** — `[models."name"]` in `config.toml`, for a model that
   shipped after this table did.

Where the provider and the table **disagree, the provider wins and says
so** — `Source::Provider` instead of `Source::Table` — because a
contradiction is exactly when the reader needs to know which one they
are looking at. Where they agree, the table keeps the citation: it also
carries the budget. An unknown model is a `Source::Guess`, and every
message that touches it admits as much.

```toml
[models."gpt-oss:20b"]
thinking = "levels"       # none | bool | levels
reasoning_tokens = 2048   # realistic spend at medium
always_reasons = true     # "off" is a request it cannot honour
```

Keys match the full name or the bare stem, so `[models.qwen3]` covers
every tag. Names are normalised first: registry path and tag stripped,
lowercased, so `hf.co/unsloth/Qwen3-8B-GGUF:Q4_K_M` resolves as
`qwen3-8b-gguf` and still lands on the `qwen3` family.

## What the caps actually drive

- **The wire.** `Think::None` omits `think` entirely — not `false`,
  *absent* — because rejection-not-ignoring is a real provider
  behaviour. `Bool` sends a boolean, `Levels` sends the level string
  (and maps a bare `on` to `medium`, since "on" means nothing to a model
  that grades effort).
- **The budget.** The reasoning allowance is the *model's* number scaled
  by the level (low ½, medium 1×, high 2×), falling back to the
  configured `thinking_tokens` only when the model is unknown. A model
  that always reasons gets its allowance even at `off`. This is the
  separation that keeps [reasoning from eating the
  answer](llm-engine.md).
- **The truth in the UI.** `#/settings` annotates the thinking row —
  `(no effect — this model doesn't reason)`, `(this model reasons
  regardless)`, `(unverified for this model)` — and `#/thinking high`
  says the same thing at the moment you set it. `#/status` reports the
  resolved capability.
- **The diagnosis.** An empty answer can finally name one cause instead
  of two. It reasons and was asked to → *it spent the budget reasoning*,
  raise `thinking_tokens`. It cannot reason → *thinking has no effect
  here*, the budget or the model is the problem. Unknown model → say
  that goulash is guessing, and point at `[models]`.

## Why a table at all

Because the alternative is worse in both directions. Probing at runtime
(ask, see if it comes back empty, retry differently) burns a real
generation to learn something static, and does it on the user's first
impression. Trusting only `/api/show` gets whether-but-not-what: it
cannot tell you that `gpt-oss` wants a level string, and it certainly
cannot tell you 2048 tokens is a realistic medium.

A table ages — that is its known cost, and it is why the provider
outranks it and the user outranks everyone. What it buys is that the
common case is right on the first ask, with no probe and no config.

## Related

- [llm-engine](llm-engine.md) — the worker, the budgets, the prefix cache
- [settings-and-nav](../interaction/settings-and-nav.md) — `#/settings`
- [state-of-play](../product/state-of-play.md) — where this sat in the queue
