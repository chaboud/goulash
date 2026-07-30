//! The provider seam: one trait, two wire protocols.
//!
//! [`Ollama`] speaks `/api/generate` (JSONL). [`OpenAiCompat`] speaks
//! `/v1/{chat/completions,completions}` (SSE), which is the same shape as
//! LM Studio, llama.cpp server, vLLM, OpenRouter, and the hosted APIs —
//! one adapter buys the lot.

use std::io::BufRead;
use std::time::{Duration, Instant};

/// Reasoning control.
///
/// Separate from [`GenRequest::max_tokens`] on purpose. The token cap
/// exists to bound what lands in the status band, but reasoning and
/// visible output draw on the same meter, so a reasoning model spends the
/// *display* budget thinking and emits nothing. Measured: 25% of empty
/// answers ended at `stop_reason = length`, while answers that do arrive
/// use a median of 32 tokens — the cap was never what kept them short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Think {
    /// Send nothing; the model's own default applies.
    #[default]
    Default,
    /// Suppress reasoning (ollama `think: false`).
    Off,
    On,
    /// Graded effort — ollama accepts these for some models, and the
    /// OpenAI shape spells it `reasoning_effort`.
    Level(&'static str),
}

pub struct ModelInfo {
    pub name: String,
    /// On-disk bytes. OpenAI-compatible servers don't report this, so it
    /// is 0 there and the smallest-model auto-pick degrades to
    /// first-listed.
    pub size: u64,
}

/// One generation, fully parameterized. The settings that used to be
/// literals inside `generate()` live here so a harness can sweep them.
/// They are deliberately *not* config.toml knobs yet — which of them
/// earns a user-facing setting is a question the measurement answers.
#[derive(Debug, Clone)]
pub struct GenRequest {
    pub model: String,
    pub prompt: String,
    pub stream: bool,
    pub temperature: f32,
    pub max_tokens: usize,
    pub num_ctx: usize,
    pub stop: Vec<String>,
    /// Reasoning control; see [`Think`].
    pub think: Think,
    /// Extra tokens allowed *on top of* `max_tokens` to absorb reasoning,
    /// so thinking cannot consume the display budget.
    ///
    /// Providers meter reasoning and output together, so this is applied
    /// by widening the wire-level cap. The visible answer stays short
    /// because the prompt asks for one line — not because the budget
    /// starves it.
    pub reasoning_tokens: usize,
    pub keep_alive: String,
}

impl GenRequest {
    /// What actually goes on the wire: display budget plus the reasoning
    /// allowance, since no provider meters them separately.
    ///
    /// The allowance is **unconditional**, and deliberately so. It used to
    /// be added only for `think != Off`, which asked the wrong question —
    /// "did we ask for reasoning" rather than "can reasoning happen". No
    /// engine lets us answer the second one with a yes/no:
    ///
    /// - OpenAI-compatible **chat** applies the model's template, and for
    ///   qwen3/gemma-class weights that template reasons no matter what we
    ///   sent. The one kwarg that disables it (`enable_thinking:false`)
    ///   *empties* `content` instead, which is why we do not send it.
    /// - Even ollama, which honours `think:false` for most models, does not
    ///   for all: `deepseek-r1:14b` accepts the flag and reasons anyway.
    ///
    /// So "off" is a request, never a guarantee, and budgeting on it
    /// produced the exact failure it was meant to prevent — 93% empty
    /// answers on the chat path, every one of them `finish=length` with
    /// the whole budget spent on reasoning we had not accounted for.
    ///
    /// Unused headroom is free: it caps a ceiling, it does not reserve
    /// anything, and measured answers use a median of 32 tokens.
    pub fn wire_max_tokens(&self) -> usize {
        self.max_tokens.saturating_add(self.reasoning_tokens)
    }
}

#[derive(Debug, Clone, Default)]
pub struct GenStats {
    /// Client-side time to the first content token.
    pub ttft_ms: Option<u64>,
    pub total_ms: u64,
    pub load_ms: Option<u64>,
    pub prompt_tokens: Option<u64>,
    /// **The cache signal.** Both providers report *total* prompt tokens
    /// regardless of cache hits, so counts reveal nothing; this collapses
    /// when the KV prefix is reused (measured 2119ms -> 619ms at an
    /// identical token count). See bench/results/step0/FINDINGS.md.
    /// `None` on OpenAI-compatible servers, which don't expose it.
    pub prompt_eval_ms: Option<u64>,
    pub eval_tokens: Option<u64>,
    pub eval_ms: Option<u64>,
    /// Tokens spent on reasoning rather than the visible answer. LM Studio
    /// reports this; ollama does not. It is the meter behind the
    /// thinking-vs-response budget question — a model can burn its whole
    /// `max_tokens` here and return nothing.
    pub reasoning_tokens: Option<u64>,
    pub stop_reason: Option<String>,
}

/// What a provider can actually honor. Lets the bench attribute a
/// failure to the *endpoint* rather than blaming the model — e.g. a
/// model that looks like it ignored `stop` when the server dropped the
/// field on the floor.
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    pub stop_sequences: bool,
    pub reasoning_control: bool,
    pub reports_prompt_eval_time: bool,
    /// LM Studio fixes context length when the model is *loaded*, so
    /// sweeping `num_ctx` there means reloading rather than re-requesting.
    pub context_len_is_load_time: bool,
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    fn probe(&self, agent: &ureq::Agent, host: &str) -> Option<Vec<ModelInfo>>;
    /// Can this model reason, learned WITHOUT asking it to.
    ///
    /// ollama answers HTTP 400 for `think` on a model that lacks it, so
    /// discovering by trying costs a failed ask. `/api/show` reports
    /// `capabilities` instead, read-only and free. `None` = unknown, which
    /// callers treat as "cannot".
    fn can_think(&self, _agent: &ureq::Agent, _host: &str, _model: &str) -> Option<bool> {
        None
    }
    /// Models the engine already has loaded, with the context each was
    /// loaded at, most-recently-used first.
    ///
    /// The context matters as much as the name: `num_ctx` is part of a
    /// model's load identity, so asking for a different one evicts and
    /// reloads (206ms reuse vs 1847ms reload). Knowing what is already
    /// there is what lets goulash leave it alone.
    fn resident(&self, _agent: &ureq::Agent, _host: &str) -> Vec<(String, usize)> {
        Vec::new()
    }
    /// Ask the server to load the model (empty generation) so the first
    /// real ask doesn't pay the cold start. Best-effort.
    fn warm(&self, agent: &ureq::Agent, host: &str, model: &str, keep_alive: &str);
    /// `on_partial` receives the *accumulated* answer text so far.
    fn generate(
        &self,
        agent: &ureq::Agent,
        host: &str,
        req: &GenRequest,
        on_partial: &mut dyn FnMut(&str),
    ) -> Result<(String, GenStats), String>;
    fn caps(&self) -> Caps;
}

// ---------------------------------------------------------------- ollama

pub struct Ollama;

impl Provider for Ollama {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn probe(&self, agent: &ureq::Agent, host: &str) -> Option<Vec<ModelInfo>> {
        let resp = agent
            .get(&format!("{host}/api/tags"))
            .timeout(Duration::from_secs(1))
            .call()
            .ok()?;
        let v: serde_json::Value = serde_json::from_str(&resp.into_string().ok()?).ok()?;
        Some(
            v["models"]
                .as_array()?
                .iter()
                .filter_map(|m| {
                    Some(ModelInfo {
                        name: m["name"].as_str()?.to_string(),
                        size: m["size"].as_u64().unwrap_or(u64::MAX),
                    })
                })
                .collect(),
        )
    }

    fn can_think(&self, agent: &ureq::Agent, host: &str, model: &str) -> Option<bool> {
        let resp = agent
            .post(&format!("{host}/api/show"))
            .timeout(Duration::from_secs(3))
            .send_string(&serde_json::json!({ "model": model }).to_string())
            .ok()?;
        let v: serde_json::Value = serde_json::from_str(&resp.into_string().ok()?).ok()?;
        Some(
            v["capabilities"]
                .as_array()?
                .iter()
                .any(|c| c.as_str() == Some("thinking")),
        )
    }

    fn resident(&self, agent: &ureq::Agent, host: &str) -> Vec<(String, usize)> {
        let Ok(resp) = agent
            .get(&format!("{host}/api/ps"))
            .timeout(Duration::from_secs(2))
            .call()
        else {
            return Vec::new();
        };
        let Ok(text) = resp.into_string() else {
            return Vec::new();
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Vec::new();
        };
        v["models"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        Some((
                            m["name"].as_str()?.to_string(),
                            m["context_length"].as_u64().unwrap_or(0) as usize,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn warm(&self, agent: &ureq::Agent, host: &str, model: &str, keep_alive: &str) {
        let mut body = serde_json::json!({ "model": model });
        if !keep_alive.is_empty() {
            body["keep_alive"] = serde_json::json!(keep_alive);
        }
        let _ = agent
            .post(&format!("{host}/api/generate"))
            .send_string(&body.to_string());
    }

    fn generate(
        &self,
        agent: &ureq::Agent,
        host: &str,
        req: &GenRequest,
        on_partial: &mut dyn FnMut(&str),
    ) -> Result<(String, GenStats), String> {
        let mut options = serde_json::json!({
            "temperature": req.temperature,
            "num_predict": req.wire_max_tokens() as i64,
        });
        // Zero means "do not send it" — let the server keep whatever the
        // user configured, and keep whatever is already loaded.
        if req.num_ctx > 0 {
            options["num_ctx"] = serde_json::json!(req.num_ctx as i64);
        }
        if !req.stop.is_empty() {
            options["stop"] = serde_json::json!(req.stop);
        }
        let mut body = serde_json::json!({
            "model": req.model,
            "prompt": req.prompt,
            "stream": req.stream,
            "options": options,
        });
        match req.think {
            Think::Default => {}
            Think::Off => body["think"] = serde_json::json!(false),
            Think::On => body["think"] = serde_json::json!(true),
            // ollama accepts a level string for models that grade effort.
            Think::Level(l) => body["think"] = serde_json::json!(l),
        }
        if !req.keep_alive.is_empty() {
            body["keep_alive"] = serde_json::json!(req.keep_alive);
        }

        let started = Instant::now();
        let resp = agent
            .post(&format!("{host}/api/generate"))
            .send_string(&body.to_string())
            .map_err(|e| e.to_string())?;

        if !req.stream {
            let text = resp.into_string().map_err(|e| e.to_string())?;
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let answer = v["response"]
                .as_str()
                .map(|s| s.trim().to_string())
                .ok_or_else(|| "malformed engine response".to_string())?;
            let mut stats = ollama_stats(&v);
            stats.total_ms = started.elapsed().as_millis() as u64;
            return Ok((answer, stats));
        }

        // One JSON object per line; forward accumulated partials so the
        // bar fills in as tokens arrive.
        let reader = std::io::BufReader::new(resp.into_reader());
        let mut acc = String::new();
        let mut stats = GenStats::default();
        for line in reader.lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            if let Some(tok) = v["response"].as_str()
                && !tok.is_empty()
            {
                if acc.is_empty() {
                    stats.ttft_ms = Some(started.elapsed().as_millis() as u64);
                }
                acc.push_str(tok);
                on_partial(&acc);
            }
            if v["done"].as_bool() == Some(true) {
                let done = ollama_stats(&v);
                stats = GenStats {
                    ttft_ms: stats.ttft_ms,
                    ..done
                };
                break;
            }
        }
        stats.total_ms = started.elapsed().as_millis() as u64;
        Ok((acc.trim().to_string(), stats))
    }

    fn caps(&self) -> Caps {
        Caps {
            stop_sequences: true,
            reasoning_control: true,
            reports_prompt_eval_time: true,
            context_len_is_load_time: false,
        }
    }
}

fn ollama_stats(v: &serde_json::Value) -> GenStats {
    let ms = |k: &str| v[k].as_u64().map(|n| n / 1_000_000);
    GenStats {
        ttft_ms: None,
        total_ms: 0,
        load_ms: ms("load_duration"),
        prompt_tokens: v["prompt_eval_count"].as_u64(),
        prompt_eval_ms: ms("prompt_eval_duration"),
        eval_tokens: v["eval_count"].as_u64(),
        eval_ms: ms("eval_duration"),
        // ollama folds thinking into eval_count; no separate meter.
        reasoning_tokens: None,
        stop_reason: v["done_reason"].as_str().map(String::from),
    }
}

// --------------------------------------------------------- openai-compat

/// `/v1/...` — LM Studio, llama.cpp server, vLLM, OpenRouter, cloud.
pub struct OpenAiCompat {
    /// `/v1/chat/completions` (templated) vs `/v1/completions` (raw).
    ///
    /// Goulash's prompt is a single stable-prefix string, which the raw
    /// completions endpoint takes verbatim — the same shape ollama gets.
    /// Chat wraps it in the model's template, which perturbs the prefix
    /// and may cost cache hits. Hosted providers only offer chat, so the
    /// cost of that mapping is worth measuring rather than assuming.
    pub chat: bool,
    /// Send `chat_template_kwargs: {enable_thinking: false}` on chat.
    ///
    /// **Default off, on evidence.** Measured against LM Studio +
    /// qwen3-1.7b, this kwarg *empties* `content` and routes the whole
    /// answer into `reasoning_content` — the exact empty-answer failure it
    /// was meant to prevent. Without it the same model answers normally.
    /// See bench/results/step0/LMSTUDIO.md.
    pub suppress_reasoning: bool,
}

impl Default for OpenAiCompat {
    fn default() -> Self {
        Self {
            chat: true,
            suppress_reasoning: false,
        }
    }
}

impl Provider for OpenAiCompat {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn probe(&self, agent: &ureq::Agent, host: &str) -> Option<Vec<ModelInfo>> {
        let resp = agent
            .get(&format!("{host}/v1/models"))
            .timeout(Duration::from_secs(1))
            .call()
            .ok()?;
        let v: serde_json::Value = serde_json::from_str(&resp.into_string().ok()?).ok()?;
        Some(
            v["data"]
                .as_array()?
                .iter()
                .filter_map(|m| {
                    Some(ModelInfo {
                        name: m["id"].as_str()?.to_string(),
                        size: 0,
                    })
                })
                .collect(),
        )
    }

    fn warm(&self, agent: &ureq::Agent, host: &str, model: &str, _keep_alive: &str) {
        // No load-only call in the OpenAI shape; a 1-token generation is
        // the cheapest way to force JIT load.
        let req = GenRequest {
            model: model.to_string(),
            prompt: "hi".into(),
            stream: false,
            temperature: 0.0,
            max_tokens: 1,
            num_ctx: 0,
            stop: Vec::new(),
            think: Think::Default,
            reasoning_tokens: 0,
            keep_alive: String::new(),
        };
        let _ = self.generate(agent, host, &req, &mut |_| {});
    }

    fn generate(
        &self,
        agent: &ureq::Agent,
        host: &str,
        req: &GenRequest,
        on_partial: &mut dyn FnMut(&str),
    ) -> Result<(String, GenStats), String> {
        let path = if self.chat {
            "/v1/chat/completions"
        } else {
            "/v1/completions"
        };
        let mut body = serde_json::json!({
            "model": req.model,
            "stream": req.stream,
            "temperature": req.temperature,
            "max_tokens": req.wire_max_tokens() as i64,
        });
        if self.chat {
            body["messages"] = serde_json::json!([{"role": "user", "content": req.prompt}]);
            // Opt-in only: see the field docs. Servers that don't know the
            // kwarg ignore it, but the ones that honor it may hurt.
            if self.suppress_reasoning && req.think == Think::Off {
                body["chat_template_kwargs"] = serde_json::json!({"enable_thinking": false});
            }
        } else {
            body["prompt"] = serde_json::json!(req.prompt);
        }
        if !req.stop.is_empty() {
            body["stop"] = serde_json::json!(req.stop);
        }
        if let Think::Level(l) = req.think {
            body["reasoning_effort"] = serde_json::json!(l);
        }
        if req.stream {
            body["stream_options"] = serde_json::json!({"include_usage": true});
        }
        // Residency control. `keep_alive` is ollama's; LM Studio's
        // equivalent is `ttl`, in seconds, applied to JIT-loaded models.
        // Without it a JIT load sits resident for LM Studio's default
        // hour — on a 24 GB machine that is someone else's RAM. Sent only
        // when the caller asked for a specific residency; servers that do
        // not know the field ignore it.
        if let Some(secs) = keep_alive_secs(&req.keep_alive) {
            body["ttl"] = serde_json::json!(secs);
        }

        let started = Instant::now();
        let resp = agent
            .post(&format!("{host}{path}"))
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
            .map_err(|e| e.to_string())?;

        if !req.stream {
            let text = resp.into_string().map_err(|e| e.to_string())?;
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let answer = self
                .content_of(&v["choices"][0], false)
                .ok_or_else(|| "malformed engine response".to_string())?;
            let mut stats = openai_stats(&v);
            stats.stop_reason = v["choices"][0]["finish_reason"].as_str().map(String::from);
            stats.total_ms = started.elapsed().as_millis() as u64;
            return Ok((answer.trim().to_string(), stats));
        }

        // SSE: `data: {json}` lines, terminated by `data: [DONE]`.
        let reader = std::io::BufReader::new(resp.into_reader());
        let mut acc = String::new();
        let mut stats = GenStats::default();
        for line in reader.lines() {
            let line = line.map_err(|e| e.to_string())?;
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                break;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
                continue;
            };
            // A usage-only final chunk carries no choices.
            if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
                stats.prompt_tokens = u["prompt_tokens"].as_u64();
                stats.eval_tokens = u["completion_tokens"].as_u64();
                stats.reasoning_tokens = u["completion_tokens_details"]["reasoning_tokens"].as_u64();
            }
            let choice = &v["choices"][0];
            if let Some(reason) = choice["finish_reason"].as_str() {
                stats.stop_reason = Some(reason.to_string());
            }
            if let Some(tok) = self.content_of(choice, true)
                && !tok.is_empty()
            {
                if acc.is_empty() {
                    stats.ttft_ms = Some(started.elapsed().as_millis() as u64);
                }
                acc.push_str(&tok);
                on_partial(&acc);
            }
        }
        stats.total_ms = started.elapsed().as_millis() as u64;
        Ok((acc.trim().to_string(), stats))
    }

    fn caps(&self) -> Caps {
        Caps {
            // Verified honored on LM Studio (finish_reason=stop).
            stop_sequences: true,
            // No *working* control: the only lever (enable_thinking) empties
            // the answer rather than suppressing the thinking. Reasoning
            // spend is observable after the fact via GenStats.reasoning_tokens.
            reasoning_control: false,
            // `stats` exists in the response but carries no timing — it is
            // `{}` on chat and speculative-decode counters on completions.
            // Cache measurement here must use client-side TTFT.
            reports_prompt_eval_time: false,
            context_len_is_load_time: true,
        }
    }
}

impl OpenAiCompat {
    /// Chat puts text under `delta`/`message`; completions under `text`.
    fn content_of(&self, choice: &serde_json::Value, streaming: bool) -> Option<String> {
        if !self.chat {
            return choice["text"].as_str().map(String::from);
        }
        let slot = if streaming { "delta" } else { "message" };
        choice[slot]["content"].as_str().map(String::from)
    }
}

/// ollama-style duration ("30m", "90s", "1h", "0") to whole seconds.
/// Returns None for an empty string — meaning "server's own schedule".
fn keep_alive_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num, mult) = match s.chars().last()? {
        'h' => (&s[..s.len() - 1], 3600),
        'm' => (&s[..s.len() - 1], 60),
        's' => (&s[..s.len() - 1], 1),
        _ => (s, 1),
    };
    num.trim().parse::<f64>().ok().map(|n| (n * mult as f64) as u64)
}

fn openai_stats(v: &serde_json::Value) -> GenStats {
    GenStats {
        prompt_tokens: v["usage"]["prompt_tokens"].as_u64(),
        eval_tokens: v["usage"]["completion_tokens"].as_u64(),
        reasoning_tokens: v["usage"]["completion_tokens_details"]["reasoning_tokens"].as_u64(),
        ..GenStats::default()
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn req(think: Think) -> GenRequest {
        GenRequest {
            model: "m".into(),
            prompt: "p".into(),
            stream: false,
            temperature: 0.2,
            max_tokens: 256,
            num_ctx: 8192,
            stop: vec![],
            think,
            reasoning_tokens: 1024,
            keep_alive: String::new(),
        }
    }

    /// Thinking draws on its OWN allowance — the display budget is never
    /// what starves the answer.
    #[test]
    fn reasoning_widens_the_wire_cap() {
        assert_eq!(req(Think::On).wire_max_tokens(), 1280);
        assert_eq!(req(Think::Level("low")).wire_max_tokens(), 1280);
        assert_eq!(req(Think::Default).wire_max_tokens(), 1280);
    }

    /// **Off is a request, not a guarantee**, so the allowance survives
    /// it. This asserted the opposite until it was measured: an OpenAI
    /// chat template reasons whatever we send, and `deepseek-r1:14b`
    /// reasons through ollama's `think:false`. Budgeting on our own
    /// intent returned 93% empty answers on the chat path, each one
    /// `finish=length` with the display budget spent on reasoning.
    ///
    /// The ceiling is not a reservation — unused headroom costs nothing,
    /// and answers that arrive use a median of 32 tokens.
    #[test]
    fn the_allowance_survives_thinking_being_off() {
        assert_eq!(req(Think::Off).wire_max_tokens(), 1280);
    }

    #[test]
    fn saturates_rather_than_overflowing() {
        let mut r = req(Think::On);
        r.max_tokens = usize::MAX;
        assert_eq!(r.wire_max_tokens(), usize::MAX);
    }
}
