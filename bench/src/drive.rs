//! The bench's vocabulary, spoken to the shipped engine.
//!
//! The harness thinks in *shapes* and *levers* — where memories sit,
//! whether the command comes first, how much reasoning is asked for —
//! because those are the things it sweeps. The engine thinks in a wire
//! format and a `Gen`. This module is the only place the two meet.
//!
//! It deliberately owns **no** prompt text. Every byte it emits comes
//! from [`goulash::engine::build_prompt`] and
//! [`goulash::engine::directive_for`], because a harness with its own
//! copy of the prompt measures the copy — and the two drift the first
//! time either is touched, quietly invalidating every number in the
//! reports. The shapes below reorder what the engine produces; they
//! never re-word it.

use goulash::engine::{PREAMBLE, build_prompt, directive_for};
use goulash::models::{Caps, caps_for};
use goulash::wire::{Backend, Client, Gen, Wire};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

/// Where pinned memories sit relative to the session log.
///
/// A *cache* lever, not a wording one. The memory block leads with a
/// live `(N/25 slots)` count, so under [`MemPos::BeforeLog`] every
/// REMEMBER rewrites a string sitting in front of the whole session log
/// and invalidates the prefix behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum MemPos {
    /// Shipped shape: between the preamble and the session log.
    #[default]
    BeforeLog,
    /// After the log but still inside the stable prefix.
    AfterLog,
    /// In the volatile suffix, alongside the time and question.
    Suffix,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PromptShape {
    pub memories: MemPos,
    /// Which machine facts to divulge. Defaults to the SHIPPED config,
    /// so a sweep measures the prompt users actually get — the platform
    /// line included. Getting this wrong is silent and total: every
    /// number would describe a prompt that nothing sends.
    pub divulge: goulash::config::DivulgeConfig,
    /// Ask for `CMD:` before the prose line.
    pub command_first: bool,
    /// Replace the shipped preamble. `None` uses the real one.
    pub preamble: Option<&'static str>,
    /// Replace the shipped directive. `None` uses the real one.
    pub directive: Option<&'static str>,
}

/// How much reasoning to ask for, in the bench's terms. Translated into
/// whatever dialect the bound model actually speaks — a boolean, a named
/// effort level, or nothing at all for a model that rejects the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Think {
    #[default]
    Off,
    On,
    Level(&'static str),
}

impl Think {
    fn level(self) -> &'static str {
        match self {
            Think::Off => "off",
            Think::On => "medium",
            Think::Level(l) => l,
        }
    }
}

pub struct GenRequest {
    pub model: String,
    pub prompt: String,
    pub stream: bool,
    pub temperature: f64,
    pub max_tokens: usize,
    pub num_ctx: usize,
    pub stop: Vec<String>,
    pub think: Think,
    pub keep_alive: String,
}

/// What one generation cost, in the terms the reports quote.
#[derive(Debug, Clone, Default)]
pub struct GenStats {
    /// Client-side time to the first content token. The only latency
    /// number a user can feel.
    pub ttft_ms: Option<u64>,
    pub total_ms: u64,
    pub load_ms: Option<u64>,
    pub prompt_tokens: Option<u64>,
    /// Time spent evaluating the prompt. **The cache signal.** Token
    /// counts cannot see a prefix hit — ollama reports the full prompt
    /// either way — but the time to evaluate it collapses when the
    /// prefix is already in the KV cache.
    pub prompt_eval_ms: Option<u64>,
    pub eval_tokens: Option<u64>,
    pub eval_ms: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub stop_reason: Option<String>,
}

/// Assemble a prompt in the given shape.
///
/// `now` is frozen by the caller. The current time sits in the volatile
/// suffix, so a live clock makes two runs of the same cell differ and
/// turns a latency comparison into noise.
pub fn shape_prompt(
    shape: &PromptShape,
    memories: &str,
    log: &str,
    question: &str,
    now: &str,
    proactive: bool,
) -> String {
    let directive = shape
        .directive
        .unwrap_or_else(|| directive_for(shape.command_first, proactive, false));

    // The shipped builder puts memories between the preamble and the
    // log. The other two positions are produced by moving the block, not
    // by rebuilding the prompt — so every arm still emits engine bytes.
    let (mem_before, mem_after, mem_suffix) = match shape.memories {
        MemPos::BeforeLog => (memories, "", ""),
        MemPos::AfterLog => ("", memories, ""),
        MemPos::Suffix => ("", "", memories),
    };
    let log = format!("{log}\n{mem_after}");
    let cards = mem_suffix;

    let facts = goulash::facts::block(&shape.divulge);
    let built = build_prompt(
        &facts, mem_before, "", &log, cards, question, directive, now,
    );

    match shape.preamble {
        None => built,
        // A preamble override is a wording lever; swap it in place so
        // everything downstream of it is still the engine's.
        Some(p) => built.replacen(PREAMBLE, p, 1),
    }
}

/// Run one generation against a server, returning the raw text and what
/// it cost. Drives `wire.rs` directly — same body, same parsing, same
/// endpoint the product uses.
pub fn generate(
    agent: &ureq::Agent,
    host: &str,
    wire: Wire,
    req: &GenRequest,
    on_partial: &mut dyn FnMut(&str),
) -> Result<(String, GenStats), String> {
    let cl = Client {
        agent: agent.clone(),
        be: Backend {
            wire,
            host: host.to_string(),
            key: String::new(),
            trusted: true,
        },
    };
    // ASK the server what this model can do, exactly as the product
    // does (engine.rs: show_thinks). Passing None here fell back to the
    // family table and never probed — so a model whose build disagrees
    // with the table was measured under the wrong assumption, silently,
    // which is the one thing a characterization harness must not do.
    let caps: Caps = caps_for(
        &req.model,
        goulash::engine::show_thinks(&cl, &req.model),
        &Default::default(),
    );
    let level = req.think.level();
    let stop: Vec<&str> = req.stop.iter().map(String::as_str).collect();

    let mut body = wire.body(&Gen {
        model: &req.model,
        prompt: &req.prompt,
        stream: req.stream,
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        num_ctx: req.num_ctx,
        stop: &stop,
        think: caps.think_field(level),
        effort: caps.effort_field(level),
        keep_alive: &req.keep_alive,
        num_keep: 0,
        seed: None,
    });

    // OpenAI-compatible servers omit usage from a stream unless asked.
    if req.stream && wire != Wire::Ollama {
        body["stream_options"] = serde_json::json!({"include_usage": true});
    }

    let t0 = Instant::now();
    let resp = cl
        .post(&cl.gen_url())
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;

    let mut text = String::new();
    let mut stats = GenStats::default();
    let mut ttft: Option<Instant> = None;

    if req.stream {
        let rdr = BufReader::new(resp.into_reader());
        for line in rdr.lines() {
            let line = line.map_err(|e| e.to_string())?;
            // The final JSONL object carries the timings; keep it so the
            // cache signal survives streaming.
            if wire == Wire::Ollama {
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && v["done"].as_bool() == Some(true)
                {
                    absorb_ollama(&mut stats, &v);
                }
            } else if let Some(p) = line.trim().strip_prefix("data:") {
                // An OpenAI-compatible stream carries finish_reason on
                // the last content chunk and usage only in the trailing
                // one (and only if asked). Absorbing whatever is present
                // is the difference between a performance table and a
                // column of "None".
                let p = p.trim();
                if p != "[DONE]"
                    && let Ok(v) = serde_json::from_str::<Value>(p)
                {
                    if v["choices"][0]["finish_reason"].is_string() {
                        stats.stop_reason = v["choices"][0]["finish_reason"]
                            .as_str()
                            .map(str::to_string);
                    }
                    if v["usage"].is_object() {
                        absorb_openai(&mut stats, &v);
                    }
                }
            }
            if let Some(c) = wire.chunk(&line) {
                if !c.text.is_empty() {
                    if ttft.is_none() {
                        ttft = Some(Instant::now());
                    }
                    text.push_str(&c.text);
                    on_partial(&text);
                }
                if c.done {
                    break;
                }
            }
        }
    } else {
        let raw = resp.into_string().map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if wire == Wire::Ollama {
            absorb_ollama(&mut stats, &v);
        } else {
            absorb_openai(&mut stats, &v);
        }
        text = wire.text(&v).unwrap_or_default();
        if !text.is_empty() {
            ttft = Some(Instant::now());
            on_partial(&text);
        }
    }

    stats.total_ms = t0.elapsed().as_millis() as u64;
    stats.ttft_ms = ttft.map(|t| t.duration_since(t0).as_millis() as u64);
    Ok((text, stats))
}

fn ms(v: &Value, k: &str) -> Option<u64> {
    v[k].as_u64().map(|ns| ns / 1_000_000)
}

fn absorb_ollama(s: &mut GenStats, v: &Value) {
    s.load_ms = ms(v, "load_duration");
    s.prompt_eval_ms = ms(v, "prompt_eval_duration");
    s.eval_ms = ms(v, "eval_duration");
    s.prompt_tokens = v["prompt_eval_count"].as_u64();
    s.eval_tokens = v["eval_count"].as_u64();
    s.stop_reason = v["done_reason"].as_str().map(str::to_string);
}

fn absorb_openai(s: &mut GenStats, v: &Value) {
    let u = &v["usage"];
    s.prompt_tokens = u["prompt_tokens"].as_u64();
    s.eval_tokens = u["completion_tokens"].as_u64();
    // Where a server reports it. Not every one does, and a missing value
    // is not zero — it is unknown, which the reports have to say.
    s.reasoning_tokens = u["completion_tokens_details"]["reasoning_tokens"]
        .as_u64()
        .or_else(|| u["reasoning_tokens"].as_u64());
    s.stop_reason = v["choices"][0]["finish_reason"]
        .as_str()
        .map(str::to_string);
}

/// A default agent with a timeout long enough for a cold load of a large
/// model, which is minutes on a laptop.
#[allow(dead_code)]
pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(600))
        .build()
}
