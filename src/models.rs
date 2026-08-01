//! Per-model capabilities: what a model will actually do when asked to
//! think, and what that costs. (wiki: architecture/model-capabilities.md)
//!
//! Thinking is not one dial. `gemma3n` rejects the request outright,
//! `gpt-oss` takes named effort levels and will happily spend a whole
//! budget reasoning, `qwen3` takes a boolean, `deepseek-r1` reasons
//! whether you ask or not. Handing all of them the same `think` field
//! produced the worst possible failure: a blank answer with no way to
//! say why.
//!
//! Three sources, in increasing authority:
//!
//! 1. **The table below** — family patterns. It knows the things a
//!    provider will not tell us: which *form* the request takes, and how
//!    many tokens the model realistically burns reasoning.
//! 2. **The provider** — ollama's `/api/show` reports a `capabilities`
//!    list. Ground truth for *whether*, silent about form and cost.
//! 3. **The user** — `[models."name"]` in config.toml, for a model that
//!    shipped after this table did.

use serde::Deserialize;
use std::collections::HashMap;

/// How a model accepts a request to reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Think {
    /// Doesn't reason. The `think` field is omitted entirely — some
    /// providers reject it rather than ignore it.
    None,
    /// `think: true|false`.
    Bool,
    /// `think: "low"|"medium"|"high"`.
    Levels,
}

impl Think {
    fn parse(s: &str) -> Option<Think> {
        match s {
            "none" | "off" | "false" => Some(Think::None),
            "bool" | "on" | "true" => Some(Think::Bool),
            "levels" | "level" => Some(Think::Levels),
            _ => None,
        }
    }
}

/// Where a resolved capability came from — worth surfacing, because
/// "the table guessed" and "the model said so" deserve different
/// confidence in an error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Matched a family pattern in the table.
    Table,
    /// The provider reported it.
    Provider,
    /// `[models."…"]` in config.toml.
    User,
    /// Nothing matched; these are assumptions.
    Guess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    pub model: String,
    pub think: Think,
    /// Tokens this model realistically spends reasoning at `medium`.
    /// The response budget is topped up by this so reasoning cannot eat
    /// the answer — the whole reason a 20B reasoner returns a blank bar.
    pub reasoning_tokens: usize,
    /// Reasons whether or not it was asked to (`deepseek-r1`, `qwq`,
    /// `gpt-oss`). Thinking "off" still costs tokens, so the allowance
    /// applies regardless of the setting.
    pub always_reasons: bool,
    pub source: Source,
}

impl Caps {
    /// What to send as the provider's `think` field, or None to omit it.
    pub fn think_field(&self, level: &str) -> Option<serde_json::Value> {
        match self.think {
            Think::None => None,
            Think::Bool => Some(serde_json::json!(matches!(
                level,
                "on" | "true" | "low" | "medium" | "high"
            ))),
            Think::Levels => Some(match level {
                "low" | "medium" | "high" => serde_json::json!(level),
                "on" | "true" => serde_json::json!("medium"),
                _ => serde_json::json!(false),
            }),
        }
    }

    /// The same intent for an OpenAI-compatible server, which spells it
    /// `reasoning_effort`. A *spelling* difference, not a fourth
    /// dialect, so `models.rs` stays the one place that knows what a
    /// model can do.
    ///
    /// `none` is the value with teeth and it must travel for every
    /// dialect that can reason at all. LM Studio also exposes the same
    /// dial natively at `POST /api/v1/chat` as `reasoning:
    /// off|on|low|medium|high`, and answers a level a model cannot do
    /// with a 400 that NAMES what it supports — *"Supported settings:
    /// 'off', 'on'"* — which is the LM Studio equivalent of ollama's
    /// `/api/show` and the obvious way to learn a dialect from an
    /// OpenAI-compatible server. Not wired up: `reasoning_effort` on
    /// the endpoint we already use gets the same result and travels to
    /// hosted providers too.
    pub fn effort_field(&self, level: &str) -> Option<&'static str> {
        // Only a model that cannot reason gets nothing. Everything else
        // is told, including — especially — when the answer is "don't".
        //
        // This used to bail unless the model spoke `Levels`, so every
        // BOOL-dialect model (all of gemma4, all of qwen3) was sent no
        // reasoning control at all on an OpenAI wire and reasoned on
        // every single ask. And even a Levels model fell through to
        // `None` at `off`, which is the one setting that most needed to
        // travel. Measured on LM Studio, gemma-4-e4b, identical prompt:
        //
        //   nothing sent          17.7s   366 tokens, 1296 chars of it reasoning
        //   reasoning_effort=none  1.6s    27 tokens, none of it reasoning
        //
        // That 10x was never LM Studio being slow. It was us never
        // asking it to stop.
        //
        // `none` is in the spec, not a local quirk: OpenAI's
        // `ReasoningEffort` is
        // `none | minimal | low | medium | high | xhigh | max`,
        // default `medium`. So this travels to a hosted endpoint
        // unchanged. The named levels are ignored by a model that only
        // has on/off, which is harmless — asking such a model for
        // `high` means "reason", and it does.
        if self.think == Think::None {
            return None;
        }
        match level {
            "off" | "false" | "" => Some("none"),
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "on" | "true" => Some("medium"),
            _ => None,
        }
    }

    /// The same intent again, in LM Studio's native vocabulary for
    /// `POST /api/v1/chat`: `off | low | medium | high | on`.
    ///
    /// Distinct from `effort_field` because this endpoint **errors** on a
    /// setting the model cannot do — the docs say so and the server
    /// proves it: asking a bool-dialect model for `low` returns
    /// *"Reasoning setting 'low' is not supported by model
    /// 'google/gemma-4-e4b'. Supported settings: 'off', 'on'"*.
    ///
    /// So the dialect has to be respected on the way out rather than
    /// discovered from a failure. A model with only on/off is asked for
    /// `on`, never `high` — which is also the honest reading of the
    /// user's setting: they asked it to think, and this is what thinking
    /// means for that model.
    pub fn reasoning_field(&self, level: &str) -> Option<&'static str> {
        let wants = !matches!(level, "off" | "false" | "");
        match self.think {
            Think::None => None,
            Think::Bool => Some(if wants { "on" } else { "off" }),
            Think::Levels => Some(match level {
                "low" => "low",
                "medium" => "medium",
                "high" => "high",
                "on" | "true" => "medium",
                _ => "off",
            }),
        }
    }

    /// Tokens to add to the response budget to cover masked reasoning.
    /// A model that always reasons gets its allowance even at `off`,
    /// because the spend happens either way.
    pub fn allowance(&self, level: &str, fallback: usize) -> usize {
        let base = if self.reasoning_tokens > 0 {
            self.reasoning_tokens
        } else {
            fallback
        };
        if self.think == Think::None {
            return 0;
        }
        match level {
            "off" | "false" | "" => {
                if self.always_reasons {
                    base
                } else {
                    0
                }
            }
            "low" => base / 2,
            "high" => base * 2,
            _ => base,
        }
    }

    /// One-line summary for `#/status`, which already names the model.
    pub fn note(&self) -> &'static str {
        match self.think {
            Think::None => "does not reason",
            Think::Bool => "reasons on/off",
            Think::Levels => "takes reasoning levels",
        }
    }
}

/// Family patterns, matched **longest-prefix-first** against the model
/// name — so `phi4-reasoning` beats `phi4` and `qwen3-coder` beats
/// `qwen3` without depending on the order of this list.
///
/// `(pattern, think style, reasoning tokens at medium, always reasons)`
const TABLE: &[(&str, Think, usize, bool)] = &[
    // Reasoners with named effort levels.
    ("gpt-oss", Think::Levels, 2048, true),
    // Reasoners with a boolean switch.
    ("deepseek-r1", Think::Bool, 2048, true),
    ("deepseek-v3.1", Think::Bool, 1536, false),
    ("qwq", Think::Bool, 2048, true),
    ("magistral", Think::Bool, 1536, true),
    ("qwen3", Think::Bool, 1024, false),
    ("phi4-reasoning", Think::Bool, 1536, true),
    ("phi4-mini-reasoning", Think::Bool, 1024, true),
    ("exaone-deep", Think::Bool, 1024, true),
    ("cogito", Think::Bool, 1024, false),
    ("granite3.2", Think::Bool, 512, false),
    ("granite3.3", Think::Bool, 512, false),
    ("smollm3", Think::Bool, 512, false),
    // gemma4 reasons; gemma3 does not, and longest-prefix keeps them
    // apart. Both spellings, because ollama tags it `gemma4:e4b` and an
    // OpenAI-compatible server serves the same weights as
    // `google/gemma-4-e4b`.
    //
    // The bare `gemma` row below is right for gemma3 and was silently
    // wrong here. On ollama it never showed, because `/api/show`
    // reports `thinking` and overrules the table. An OpenAI-compatible
    // server has no such probe, so the table stood — and gemma4 on LM
    // Studio reasoned on every ask, 1300-1900 characters of it, for
    // 20s an answer. Measured reasoning: ~350-500 tokens.
    ("gemma4", Think::Bool, 1024, false),
    ("gemma-4", Think::Bool, 1024, false),
    // Same family, no reasoning — these must not inherit the prefix.
    ("qwen3-coder", Think::None, 0, false),
    ("qwen3-embedding", Think::None, 0, false),
    ("deepseek-coder", Think::None, 0, false),
    // Plain instruction models.
    ("gemma", Think::None, 0, false),
    ("codegemma", Think::None, 0, false),
    ("llama", Think::None, 0, false),
    ("tinyllama", Think::None, 0, false),
    ("codellama", Think::None, 0, false),
    ("mistral", Think::None, 0, false),
    ("mixtral", Think::None, 0, false),
    ("devstral", Think::None, 0, false),
    ("codestral", Think::None, 0, false),
    ("phi3", Think::None, 0, false),
    ("phi4", Think::None, 0, false),
    ("qwen2", Think::None, 0, false),
    ("starcoder", Think::None, 0, false),
    ("dolphin", Think::None, 0, false),
    ("nomic-embed", Think::None, 0, false),
    ("mxbai-embed", Think::None, 0, false),
];

/// A user's `[models."name"]` entry. Every field optional: an override
/// patches the table rather than replacing it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Override {
    /// `none` | `bool` | `levels`.
    pub thinking: Option<String>,
    pub reasoning_tokens: Option<usize>,
    pub always_reasons: Option<bool>,
}

pub type Overrides = HashMap<String, Override>;

/// The comparable part of a model name: registry path and digest
/// stripped, tag kept off, lowercased. `hf.co/foo/Qwen3-8B-GGUF:Q4` →
/// `qwen3-8b-gguf`.
fn stem(model: &str) -> String {
    let name = model.rsplit('/').next().unwrap_or(model);
    let name = name.split(['@', ':']).next().unwrap_or(name);
    name.to_lowercase()
}

/// Resolve capabilities for a model. `provider_thinks` is what the
/// provider reported (`Some(true)`/`Some(false)`), or None when it
/// couldn't say — an older server, or a probe that failed.
pub fn caps_for(model: &str, provider_thinks: Option<bool>, over: &Overrides) -> Caps {
    let stem = stem(model);
    let hit = TABLE
        .iter()
        .filter(|(pat, ..)| stem.starts_with(pat))
        .max_by_key(|(pat, ..)| pat.len());

    let mut caps = match hit {
        Some((_, think, tokens, always)) => Caps {
            model: model.to_string(),
            think: *think,
            reasoning_tokens: *tokens,
            always_reasons: *always,
            source: Source::Table,
        },
        None => Caps {
            model: model.to_string(),
            think: Think::Bool,
            reasoning_tokens: 0, // falls back to the configured budget
            always_reasons: false,
            source: Source::Guess,
        },
    };

    // The provider outranks the table on *whether*: it is describing the
    // model actually installed, and this table ages.
    // Where they disagree the provider wins, and says so — a
    // contradiction is exactly when the reader needs to know which one
    // they are looking at.
    match provider_thinks {
        Some(false) if caps.think != Think::None => {
            caps.think = Think::None;
            caps.reasoning_tokens = 0;
            caps.always_reasons = false;
            caps.source = Source::Provider;
        }
        Some(true) if caps.think == Think::None => {
            // A newer sibling of a non-reasoning family, or a family
            // that grew the ability. Believe the provider; keep the form
            // conservative and let the configured budget stand in.
            caps.think = Think::Bool;
            caps.reasoning_tokens = 0;
            caps.source = Source::Provider;
        }
        // They agree. A table hit is the better citation of the two —
        // it also knows the cost — but a guess is upgraded, because the
        // provider just confirmed it.
        Some(_) if caps.source == Source::Guess => caps.source = Source::Provider,
        _ => {}
    }

    // The user outranks everyone, including the model itself.
    if let Some(o) = over.get(model).or_else(|| over.get(stem.as_str())) {
        if let Some(t) = o.thinking.as_deref().and_then(Think::parse) {
            caps.think = t;
        }
        if let Some(n) = o.reasoning_tokens {
            caps.reasoning_tokens = n;
        }
        if let Some(a) = o.always_reasons {
            caps.always_reasons = a;
        }
        caps.source = Source::User;
    }
    caps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(model: &str) -> Caps {
        caps_for(model, None, &Overrides::new())
    }

    /// gemma3 and gemma4 are different animals under one prefix, and
    /// the difference only bites where there is no provider probe to
    /// correct it — an OpenAI-compatible server. Without this split,
    /// gemma4 on LM Studio reasoned on every ask and nothing said so.
    #[test]
    fn gemma4_reasons_and_gemma3_does_not() {
        // No probe: exactly what an OpenAI-compatible server gives us.
        for m in [
            "gemma4:e4b",
            "gemma4:12b",
            "google/gemma-4-e4b",
            "google/gemma-4-12b-qat",
        ] {
            let c = caps_for(m, None, &Overrides::new());
            assert_eq!(c.think, Think::Bool, "{m} reasons");
            assert_eq!(
                c.effort_field("off"),
                Some("none"),
                "{m} must be silenceable"
            );
        }
        for m in ["gemma3:4b", "gemma3:27b", "gemma:7b", "codegemma:7b"] {
            assert_eq!(caps(m).think, Think::None, "{m} does not reason");
        }
    }

    #[test]
    fn longest_prefix_wins_over_table_order() {
        // phi4-reasoning must not be read as phi4, nor qwen3-coder as qwen3.
        assert_eq!(caps("phi4-reasoning:14b").think, Think::Bool);
        assert_eq!(caps("phi4:latest").think, Think::None);
        assert_eq!(caps("qwen3-coder:30b").think, Think::None);
        assert_eq!(caps("qwen3:8b").think, Think::Bool);
        assert_eq!(caps("deepseek-r1:7b").think, Think::Bool);
        assert_eq!(caps("deepseek-coder-v2:16b").think, Think::None);
    }

    #[test]
    fn stems_strip_registry_and_tag() {
        assert_eq!(stem("hf.co/unsloth/Qwen3-8B-GGUF:Q4_K_M"), "qwen3-8b-gguf");
        assert_eq!(stem("gemma3n:e4b"), "gemma3n");
        assert_eq!(
            caps("hf.co/unsloth/Qwen3-8B-GGUF:Q4_K_M").think,
            Think::Bool
        );
    }

    #[test]
    fn think_field_matches_the_models_dialect() {
        assert_eq!(caps("gemma3:4b").think_field("high"), None);
        assert_eq!(
            caps("qwen3:8b").think_field("high"),
            Some(serde_json::json!(true))
        );
        assert_eq!(
            caps("qwen3:8b").think_field("off"),
            Some(serde_json::json!(false))
        );
        assert_eq!(
            caps("gpt-oss:20b").think_field("low"),
            Some(serde_json::json!("low"))
        );
        // "on" has no meaning for a level model: land in the middle.
        assert_eq!(
            caps("gpt-oss:20b").think_field("on"),
            Some(serde_json::json!("medium"))
        );
    }

    /// The gap that cost a 10x: nothing asserted that a BOOL-dialect
    /// model gets a reasoning control on an OpenAI wire, so it got none
    /// and reasoned on every ask. `off` has to travel for every dialect
    /// that can reason -- it is the setting with teeth.
    #[test]
    fn every_reasoning_model_can_be_told_to_stop() {
        for m in ["gemma4:e4b", "qwen3:8b", "gpt-oss:20b", "deepseek-r1:7b"] {
            let c = caps_for(m, Some(true), &Overrides::new());
            assert_eq!(
                c.effort_field("off"),
                Some("none"),
                "{m} must be silenceable"
            );
            assert!(c.effort_field("high").is_some(), "{m} must be askable");
        }
        // A model that cannot reason is handed nothing at all: some
        // servers reject the field rather than ignoring it.
        let plain = caps_for("gemma3:4b", Some(false), &Overrides::new());
        assert_eq!(plain.effort_field("off"), None);
        assert_eq!(plain.effort_field("high"), None);
        // Levels still map through unchanged.
        let oss = caps_for("gpt-oss:20b", None, &Overrides::new());
        assert_eq!(oss.effort_field("low"), Some("low"));
        assert_eq!(oss.effort_field("on"), Some("medium"));
    }

    #[test]
    fn allowance_scales_and_survives_off_when_reasoning_is_unavoidable() {
        let oss = caps("gpt-oss:20b");
        assert_eq!(oss.allowance("low", 512), 1024);
        assert_eq!(oss.allowance("medium", 512), 2048);
        assert_eq!(oss.allowance("high", 512), 4096);
        // Always reasons: "off" is a request it cannot honour, so the
        // budget still has to cover the spend.
        assert_eq!(oss.allowance("off", 512), 2048);
        // A switchable reasoner really does stop.
        assert_eq!(caps("qwen3:8b").allowance("off", 512), 0);
        // Non-reasoner: nothing, ever.
        assert_eq!(caps("gemma3:4b").allowance("high", 512), 0);
        // Unknown model: the configured budget is the base.
        assert_eq!(caps("who-knows:1b").allowance("medium", 512), 512);
    }

    #[test]
    fn provider_outranks_the_table_both_ways() {
        // Table says gemma can't; provider says this one can.
        let up = caps_for("gemma9:4b", Some(true), &Overrides::new());
        assert_eq!(up.think, Think::Bool);
        assert_eq!(up.source, Source::Provider);
        // Table says qwen3 can; provider says this build can't.
        let down = caps_for("qwen3:8b", Some(false), &Overrides::new());
        assert_eq!(down.think, Think::None);
        assert_eq!(down.source, Source::Provider);
        assert_eq!(down.allowance("high", 512), 0);
        // Agreement leaves the table's citation (and its budget) intact.
        let agreed = caps_for("gpt-oss:20b", Some(true), &Overrides::new());
        assert_eq!(agreed.source, Source::Table);
        assert_eq!(agreed.allowance("medium", 512), 2048);
        // A guess the provider confirms is no longer a guess.
        let confirmed = caps_for("mystery:8b", Some(true), &Overrides::new());
        assert_eq!(confirmed.source, Source::Provider);
    }

    #[test]
    fn user_override_outranks_everything() {
        let mut over = Overrides::new();
        over.insert(
            "gemma3:4b".to_string(),
            Override {
                thinking: Some("levels".to_string()),
                reasoning_tokens: Some(300),
                always_reasons: None,
            },
        );
        let c = caps_for("gemma3:4b", Some(false), &over);
        assert_eq!(c.think, Think::Levels);
        assert_eq!(c.allowance("medium", 512), 300);
        assert_eq!(c.source, Source::User);
    }

    #[test]
    fn unknown_model_is_a_hedged_guess() {
        let c = caps("brandnew:70b");
        assert_eq!(c.source, Source::Guess);
        assert_eq!(c.think, Think::Bool);
    }
}
