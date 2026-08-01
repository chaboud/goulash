//! Provider wire formats.
//!
//! goulash assembles one **stable-prefix string**, because the whole
//! engine design is that prefix plus the provider's KV cache (wiki:
//! llm-engine.md). Where that string goes differs by server, and the
//! difference is bigger than an envelope:
//!
//! - **ollama `/api/generate`** applies the model's chat template to it.
//! - **OpenAI-compatible `/v1/chat/completions`** applies it too, at the
//!   cost of moving the prefix boundary. **This is the default.**
//! - **OpenAI-compatible `/v1/completions`** applies *nothing*. Better
//!   for the cache, and wrong for everything else: an instruction sent
//!   there is continued rather than followed.
//!
//! That last point cost a release. The path defaulted to raw
//! completions and Gemma answered in repetition loops while qwen3 wrote
//! fluent replies to questions nobody asked — silently, with every
//! mechanical metric green. Cache shape is worth optimising; it is not
//! worth an answer that is not an answer. (bench/QUIRKS.md §3)
//!
//! Everything provider-shaped lives here. The engine builds a `Gen` and
//! reads back text; it never learns which server it is talking to.

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire {
    Ollama,
    /// OpenAI-compatible `/v1/completions` — a **raw** completion
    /// endpoint. Opt-in only, spelled `openai-raw`.
    ///
    /// It applies NO chat template. That is the whole difference from
    /// ollama's `/api/generate`, which templates by default, and it is
    /// not a nuance: an instruction prompt sent here is *continued*
    /// rather than followed. Gemma degenerates into a repetition loop
    /// without its turn markers; qwen3 writes a plausible paragraph
    /// answering nobody's question. Neither reports an error, and every
    /// mechanical metric scores the qwen case as a success.
    ///
    /// Kept because it is the honest shape for measuring prefix caching
    /// — it takes our stable-prefix string verbatim — and because a
    /// server that pre-templates would want it. Not for general use.
    /// (bench/QUIRKS.md §3)
    OpenAi,
    /// OpenAI-compatible `/v1/chat/completions`. **The default** for
    /// every OpenAI-compatible spelling.
    ///
    /// The server applies the model's own template, which is what makes
    /// an instruction behave like an instruction. It also means hosted
    /// providers work, since they offer nothing else.
    ///
    /// The cost is real and was measured: the template moves the prefix
    /// boundary, and it reasons whether or not we ask. Given a budget
    /// that is not starving it, the same weights that returned empty
    /// `content` at a 256-token ceiling answer normally, spending
    /// 659-896 tokens thinking first. That is a price worth paying for
    /// answers that are answers. (bench/QUIRKS.md §1, §3)
    OpenAiChat,
    /// LM Studio's own `POST /api/v1/chat`. **The default for LM
    /// Studio**, and better than the OpenAI-compatible shim on three
    /// counts that matter here:
    ///
    /// - `reasoning: off|low|medium|high|on` is a documented per-request
    ///   dial that the server maps onto whatever switch the model
    ///   actually has, instead of us guessing a chat-template kwarg;
    /// - reasoning arrives as its OWN event type, so it cannot leak into
    ///   the band the way a stray `<think>` tag does on other wires;
    /// - `stats` reports `reasoning_output_tokens`, `tokens_per_second`
    ///   and `time_to_first_token_seconds` — the reasoning spend that no
    ///   other local engine will tell us (bench/QUIRKS.md §5).
    LmStudio,
}

/// Where the engine is talking, resolved once by the probe chain rather
/// than re-derived from config at each call site — `host` alone cannot
/// say which dialect answers on it.
#[derive(Clone, Debug)]
pub struct Backend {
    pub wire: Wire,
    pub host: String,
    /// Bearer token, for the case where the endpoint is not on this
    /// machine. Empty for LM Studio and ollama.
    pub key: String,
    /// May this backend be shown pinned file content? Resolved from an
    /// explicit setting, never inferred from some other field having a
    /// convenient value — "no api key" is a coincidence, not consent.
    pub trusted: bool,
}

impl Backend {
    pub fn label(&self) -> &'static str {
        match self.wire {
            Wire::Ollama => "ollama",
            Wire::OpenAi => "openai-raw",
            Wire::OpenAiChat => "openai",
            Wire::LmStudio => "lmstudio",
        }
    }
}

/// `yes` | `no` | `auto`. Auto is the only inference we do, and it is
/// the narrow one: a loopback host is this machine, and nothing else
/// qualifies. Anything ambiguous resolves to NOT trusted, because the
/// failure directions are not symmetric — wrongly withholding a file
/// costs an answer, wrongly sending one cannot be undone.
pub fn resolve_trust(setting: &str, host: &str) -> bool {
    match setting {
        "yes" | "true" | "always" => true,
        "no" | "false" | "never" => false,
        _ => is_loopback(host),
    }
}

fn is_loopback(host: &str) -> bool {
    let h = host
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let h = h.split('/').next().unwrap_or("");
    // Strip the port, taking care not to mangle a bracketed IPv6 host.
    let host_only = if let Some(rest) = h.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        h.split(':').next().unwrap_or("")
    };
    matches!(host_only, "localhost" | "::1" | "0.0.0.0")
        || host_only == "127.0.0.1"
        || host_only.starts_with("127.")
}

/// An HTTP agent bound to one resolved backend. Everything the engine
/// sends goes through here, so the bearer token is attached in exactly
/// one place and a local server never sees an empty `Authorization`.
pub struct Client {
    pub agent: ureq::Agent,
    pub be: Backend,
}

impl Client {
    pub fn post(&self, url: &str) -> ureq::Request {
        // JSON, explicitly. `send_string` labels the body `text/plain`,
        // which ollama tolerates and an OpenAI-compatible server does
        // not: LM Studio answers **415 Unsupported Media Type** and the
        // whole provider looks dead. Set in the one place every request
        // already funnels through, so no call site can forget it.
        self.auth(self.agent.post(url))
            .set("Content-Type", "application/json")
    }

    pub fn get(&self, url: &str) -> ureq::Request {
        self.auth(self.agent.get(url))
    }

    fn auth(&self, r: ureq::Request) -> ureq::Request {
        if self.be.key.is_empty() {
            r
        } else {
            r.set("Authorization", &format!("Bearer {}", self.be.key))
        }
    }

    pub fn gen_url(&self) -> String {
        self.be.wire.gen_url(&self.be.host)
    }

    pub fn models_url(&self) -> String {
        self.be.wire.models_url(&self.be.host)
    }
}

/// What one generation cost, as the server measured it.
///
/// Server-side numbers, not ours: a client stopwatch cannot separate
/// prompt processing from generation, and cannot see reasoning spend at
/// all. `reasoning` is the one goulash could never obtain before — no
/// local engine reported it, so the only signal for a model burning its
/// budget on thinking was an empty answer (bench/QUIRKS.md §5).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GenStats {
    /// Tokens generated, reasoning included where the server counts it.
    pub out_tokens: u64,
    /// Of which were reasoning. Zero is a real answer here, not a
    /// missing one — the wires that cannot say leave `None` upstream.
    pub reasoning_tokens: u64,
    pub tokens_per_second: f64,
    pub ttft_ms: u64,
}

/// One generation request, in the terms the engine cares about.
pub struct Gen<'a> {
    pub model: &'a str,
    pub prompt: &'a str,
    pub stream: bool,
    pub temperature: f64,
    pub max_tokens: usize,
    pub num_ctx: usize,
    pub stop: &'a [&'a str],
    /// ollama's `think` field, already in the model's own dialect.
    pub think: Option<Value>,
    /// The same intent for an OpenAI-compatible server, which spells it
    /// `reasoning_effort` and only accepts the level form.
    pub effort: Option<&'a str>,
    /// And again in LM Studio's native vocabulary (`off|low|medium|
    /// high|on`), already narrowed to what this model supports — that
    /// endpoint errors on a setting the model cannot do.
    pub reasoning: Option<&'a str>,
    pub keep_alive: &'a str,
    /// Leading prompt tokens the server must retain when the context
    /// overflows. llama.cpp truncates from the LEFT, which is exactly
    /// where the preamble and the pinned memories live — so without
    /// this, the first thing dropped is the grammar and the facts the
    /// user asked us to remember. Zero means "don't send it".
    pub num_keep: usize,
    /// Fixed sampling seed, so a field report can be reproduced. None
    /// leaves the server to its own randomness.
    pub seed: Option<i64>,
}

/// One decoded step of a stream.
pub struct Chunk {
    pub text: String,
    pub done: bool,
}

impl Wire {
    pub fn parse(s: &str) -> Option<Wire> {
        match s {
            "ollama" => Some(Wire::Ollama),
            // Every friendly spelling lands on CHAT, because chat is the
            // one that applies the model's template. This defaulted to
            // raw completions for one release and it was a regression,
            // not a choice: `engine-characterization` shipped
            // `OpenAiCompat { chat: true }` and the port inverted it.
            // The failure was silent — see the OpenAi variant's docs.
            // LM Studio gets its OWN api, not the compatibility shim:
            // it is the only local server with a documented reasoning
            // dial, and it reports the reasoning spend afterwards.
            "lmstudio" | "lms" => Some(Wire::LmStudio),
            "openai" | "openai-chat" | "chat" | "llamacpp" | "vllm" => Some(Wire::OpenAiChat),
            // Raw completions, named explicitly so nobody arrives here
            // by accident.
            "openai-raw" | "completions" => Some(Wire::OpenAi),
            _ => None,
        }
    }

    pub fn gen_url(&self, host: &str) -> String {
        match self {
            Wire::Ollama => format!("{host}/api/generate"),
            Wire::OpenAi => format!("{host}/v1/completions"),
            Wire::OpenAiChat => format!("{host}/v1/chat/completions"),
            Wire::LmStudio => format!("{host}/api/v1/chat"),
        }
    }

    pub fn models_url(&self, host: &str) -> String {
        match self {
            Wire::Ollama => format!("{host}/api/tags"),
            Wire::OpenAi | Wire::OpenAiChat => format!("{host}/v1/models"),
            Wire::LmStudio => format!("{host}/api/v1/models"),
        }
    }

    /// Where to ask what is *loaded right now*, and at what context.
    ///
    /// Ollama only. An OpenAI-compatible server exposes an inventory but
    /// not a residency view, so there is nothing to negotiate against
    /// there and context handling degrades to "send what we were told".
    pub fn ps_url(&self, host: &str) -> Option<String> {
        match self {
            Wire::Ollama => Some(format!("{host}/api/ps")),
            Wire::OpenAi | Wire::OpenAiChat => None,
            // Residency is in the model listing, not a separate call.
            Wire::LmStudio => None,
        }
    }

    /// Read the terminal event's accounting, if this wire has one.
    ///
    /// Called on every streamed line and cheap to miss: a line that is
    /// not the end simply answers None.
    pub fn gen_stats(&self, line: &str) -> Option<GenStats> {
        let payload = match self {
            Wire::Ollama => line.trim(),
            _ => line.trim().strip_prefix("data:")?.trim(),
        };
        let v: Value = serde_json::from_str(payload).ok()?;
        match self {
            Wire::Ollama => {
                if v["done"].as_bool() != Some(true) {
                    return None;
                }
                let eval = v["eval_count"].as_u64().unwrap_or(0);
                let eval_ns = v["eval_duration"].as_u64().unwrap_or(0);
                Some(GenStats {
                    out_tokens: eval,
                    // ollama does not separate reasoning from output.
                    reasoning_tokens: 0,
                    tokens_per_second: if eval_ns > 0 {
                        eval as f64 * 1e9 / eval_ns as f64
                    } else {
                        0.0
                    },
                    // Load excluded: it is a property of the model being
                    // cold, not of this answer.
                    ttft_ms: v["prompt_eval_duration"].as_u64().unwrap_or(0) / 1_000_000,
                })
            }
            Wire::LmStudio => {
                if v["type"].as_str()? != "chat.end" {
                    return None;
                }
                let st = if v["result"]["stats"].is_object() {
                    &v["result"]["stats"]
                } else {
                    &v["stats"]
                };
                Some(GenStats {
                    out_tokens: st["total_output_tokens"].as_u64().unwrap_or(0),
                    reasoning_tokens: st["reasoning_output_tokens"].as_u64().unwrap_or(0),
                    tokens_per_second: st["tokens_per_second"].as_f64().unwrap_or(0.0),
                    ttft_ms: (st["time_to_first_token_seconds"].as_f64().unwrap_or(0.0) * 1000.0)
                        as u64,
                })
            }
            // The spec's own mechanism: with `stream_options.include_usage`
            // an extra chunk arrives before `data: [DONE]` carrying usage
            // for the whole request, with `choices` empty. We ask for it
            // in `body()`, so the stats row works on a hosted endpoint
            // too — this used to return None with a comment saying we
            // "do not ask", which was true and did not have to be.
            Wire::OpenAi | Wire::OpenAiChat => {
                let u = &v["usage"];
                if !u.is_object() {
                    return None;
                }
                let out = u["completion_tokens"].as_u64().unwrap_or(0);
                Some(GenStats {
                    out_tokens: out,
                    // Where a server reports it; absent is not zero, but
                    // the row shows reasoning only when non-zero anyway.
                    reasoning_tokens: u["completion_tokens_details"]["reasoning_tokens"]
                        .as_u64()
                        .unwrap_or(0),
                    // Not in the spec's usage block. Left at zero rather
                    // than timed client-side, which would measure our own
                    // scheduling as much as the server's.
                    tokens_per_second: 0.0,
                    ttft_ms: 0,
                })
            }
        }
    }

    pub fn body(&self, g: &Gen) -> Value {
        match self {
            Wire::Ollama => {
                let mut b = json!({
                    "model": g.model,
                    "prompt": g.prompt,
                    "stream": g.stream,
                    "options": {
                        "temperature": g.temperature,
                        "num_predict": g.max_tokens as i64,
                    },
                });
                // Zero means "say nothing", and it has to be spelled by
                // ABSENCE. Sent as a number it is not ignored: ollama
                // takes it as a real request, clamps it to a few tokens,
                // and reloads the model to that — so the one value that
                // meant "leave the load alone" was the one that reloaded
                // it every single ask.
                if g.num_ctx > 0 {
                    b["options"]["num_ctx"] = json!(g.num_ctx as i64);
                }
                if !g.stop.is_empty() {
                    b["options"]["stop"] = json!(g.stop);
                }
                if g.num_keep > 0 {
                    b["options"]["num_keep"] = json!(g.num_keep as i64);
                }
                if let Some(s) = g.seed {
                    b["options"]["seed"] = json!(s);
                }
                if let Some(t) = &g.think {
                    b["think"] = t.clone();
                }
                if !g.keep_alive.is_empty() {
                    b["keep_alive"] = json!(g.keep_alive);
                }
                b
            }
            Wire::OpenAi => {
                let mut b = json!({
                    "model": g.model,
                    "prompt": g.prompt,
                    "stream": g.stream,
                    "temperature": g.temperature,
                    "max_tokens": g.max_tokens as i64,
                });
                if !g.stop.is_empty() {
                    b["stop"] = json!(g.stop);
                }
                // No num_ctx, no keep_alive, no num_keep: all three are
                // server-side settings there, not per-request ones.
                // Sending them anyway would be noise at best and a 400
                // at worst. `seed` IS in the OpenAI schema.
                if let Some(s) = g.seed {
                    b["seed"] = json!(s);
                }
                if let Some(e) = g.effort {
                    b["reasoning_effort"] = json!(e);
                }
                // Spec: the usage chunk only arrives if asked for.
                if g.stream {
                    b["stream_options"] = json!({"include_usage": true});
                }
                b
            }
            Wire::LmStudio => {
                let mut b = json!({
                    "model": g.model,
                    "input": g.prompt,
                    "stream": g.stream,
                    "temperature": g.temperature,
                    "max_output_tokens": g.max_tokens as i64,
                });
                // Already in this endpoint's own vocabulary, and already
                // narrowed to what the model can do (models.rs
                // reasoning_field) — this API ERRORS on a setting the
                // model does not support rather than ignoring it.
                if let Some(r) = g.reasoning {
                    b["reasoning"] = json!(r);
                }
                // NO context_length, ever. Forcing a window does not
                // resize anything — it makes LM Studio spin up a NEW
                // MODEL INSTANCE for that request. Watched by instance
                // id across four otherwise identical asks:
                //
                //   omitted          0.60s   instance :3
                //   context_length  10.53s   instance :2   <- new
                //   context_length   9.42s   instance :3   <- new again
                //   omitted          1.24s   instance :3
                //
                // Ten seconds of model LOAD per ask, and the instance
                // config stayed at its own 118272 throughout, so the
                // request did not even get the window it demanded.
                //
                // Third time this shape has bitten in one release —
                // ollama `num_ctx: 0`, ollama omission, now this — so
                // the rule, stated once and for all engines: **ride
                // along with the window that is loaded; never demand
                // one.** A context window is part of a model's load
                // identity everywhere we speak, so asking for a
                // different one is asking for a reload: it blows the
                // prefix cache the whole engine design rests on, and
                // can evict the model outright.
                //
                // Not stored: goulash keeps its own session log, and a
                // second copy on the server is state nobody asked for.
                b["store"] = json!(false);
                b
            }
            Wire::OpenAiChat => {
                // One user message carrying the whole stable-prefix
                // string. The template will wrap it; that is the cost of
                // the endpoint and the reason it is not the local
                // default.
                let mut b = json!({
                    "model": g.model,
                    "messages": [{"role": "user", "content": g.prompt}],
                    "stream": g.stream,
                    "temperature": g.temperature,
                    "max_tokens": g.max_tokens as i64,
                });
                if !g.stop.is_empty() {
                    b["stop"] = json!(g.stop);
                }
                if let Some(s) = g.seed {
                    b["seed"] = json!(s);
                }
                if let Some(e) = g.effort {
                    b["reasoning_effort"] = json!(e);
                }
                // Spec: the usage chunk only arrives if asked for.
                if g.stream {
                    b["stream_options"] = json!({"include_usage": true});
                }
                // Deliberately NOT sending chat_template_kwargs
                // {enable_thinking: false}. Measured against LM Studio +
                // qwen3: that kwarg EMPTIES `content` and routes the
                // whole answer into `reasoning_content` — the exact
                // failure it looks like it prevents.
                b
            }
        }
    }

    /// The whole answer, from a non-streamed response body.
    pub fn text(&self, v: &Value) -> Option<String> {
        match self {
            Wire::Ollama => v["response"].as_str().map(|s| s.to_string()),
            Wire::OpenAi => v["choices"][0]["text"]
                .as_str()
                .map(|s| s.to_string())
                // A server pointed at chat-completions by mistake still
                // answers; reading it costs one line and turns a blank
                // bar into a working one.
                .or_else(|| {
                    v["choices"][0]["message"]["content"]
                        .as_str()
                        .map(|s| s.to_string())
                }),
            // `output` is a list of typed blocks; take the message ones
            // and leave the reasoning blocks where they are.
            Wire::LmStudio => Some(
                v["output"]
                    .as_array()?
                    .iter()
                    .filter(|b| b["type"] == "message")
                    .filter_map(|b| b["content"].as_str())
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            Wire::OpenAiChat => v["choices"][0]["message"]["content"]
                .as_str()
                .map(|s| s.to_string())
                // Some servers put an empty string in `content` and the
                // whole answer in `reasoning_content`. Reading it is the
                // difference between a blank bar and an answer.
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    v["choices"][0]["message"]["reasoning_content"]
                        .as_str()
                        .map(|s| s.to_string())
                }),
        }
    }

    /// One line of a stream. `None` means "nothing in this line" — a
    /// keep-alive, a blank separator, or SSE's terminator.
    ///
    /// ollama streams newline-delimited JSON; OpenAI-compatible servers
    /// stream SSE, where every payload line is prefixed `data: ` and the
    /// stream ends with a literal `data: [DONE]` rather than a flag on
    /// the last object.
    pub fn chunk(&self, line: &str) -> Option<Chunk> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        match self {
            Wire::Ollama => {
                let v: Value = serde_json::from_str(line).ok()?;
                Some(Chunk {
                    text: v["response"].as_str().unwrap_or("").to_string(),
                    done: v["done"].as_bool() == Some(true),
                })
            }
            // Typed events. `reasoning.delta` is deliberately dropped:
            // it is the model's scratch work, and the whole reason a
            // stray <think> tag has ever reached the band on another
            // wire is that there it arrives in the same stream as the
            // answer. Here it simply is not the answer.
            Wire::LmStudio => {
                let payload = line.strip_prefix("data:")?.trim();
                let v: Value = serde_json::from_str(payload).ok()?;
                match v["type"].as_str()? {
                    "message.delta" => Some(Chunk {
                        text: v["content"].as_str().unwrap_or_default().to_string(),
                        done: false,
                    }),
                    "chat.end" => Some(Chunk {
                        text: String::new(),
                        done: true,
                    }),
                    _ => None,
                }
            }
            Wire::OpenAi | Wire::OpenAiChat => {
                let payload = line.strip_prefix("data:")?.trim();
                if payload == "[DONE]" {
                    return Some(Chunk {
                        text: String::new(),
                        done: true,
                    });
                }
                let v: Value = serde_json::from_str(payload).ok()?;
                let text = v["choices"][0]["text"]
                    .as_str()
                    .or_else(|| v["choices"][0]["delta"]["content"].as_str())
                    .unwrap_or("")
                    .to_string();
                // finish_reason lands on the last content-bearing chunk;
                // [DONE] still follows, so this is belt-and-braces for
                // servers that omit it.
                let done = !v["choices"][0]["finish_reason"].is_null();
                Some(Chunk { text, done })
            }
        }
    }

    /// Model names, from the provider's list endpoint.
    pub fn models(&self, v: &Value) -> Vec<String> {
        let (arr, key) = match self {
            Wire::Ollama => (&v["models"], "name"),
            Wire::LmStudio => (&v["models"], "key"),
            Wire::OpenAi | Wire::OpenAiChat => (&v["data"], "id"),
        };
        arr.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m[key].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req<'a>(prompt: &'a str, stop: &'a [&'a str]) -> Gen<'a> {
        Gen {
            model: "m",
            prompt,
            stream: false,
            temperature: 0.2,
            max_tokens: 128,
            num_ctx: 8192,
            stop,
            think: None,
            effort: None,
            reasoning: None,
            keep_alive: "30m",
            num_keep: 512,
            seed: Some(7),
        }
    }

    /// The head of the prompt is the grammar and the pinned memories,
    /// and llama.cpp truncates from the left — so `num_keep` is what
    /// stops an overflowing context from eating exactly the things we
    /// most needed it to keep. It has no OpenAI equivalent (server-side
    /// there), but `seed` does, so a field report can be reproduced on
    /// either wire.
    #[test]
    fn num_keep_is_ollama_only_and_seed_crosses_both() {
        let o = Wire::Ollama.body(&req("p", &[]));
        let a = Wire::OpenAi.body(&req("p", &[]));
        assert_eq!(o["options"]["num_keep"], 512);
        assert_eq!(o["options"]["seed"], 7);
        assert!(a.get("num_keep").is_none() && a["options"].is_null());
        assert_eq!(a["seed"], 7);
    }

    /// Both are opt-out, and opting out means the key is absent rather
    /// than sent as a zero the server would honour literally.
    #[test]
    fn zero_num_keep_and_no_seed_are_simply_not_sent() {
        let mut g = req("p", &[]);
        g.num_keep = 0;
        g.seed = None;
        let o = Wire::Ollama.body(&g);
        let a = Wire::OpenAi.body(&g);
        assert!(o["options"].get("num_keep").is_none());
        assert!(o["options"].get("seed").is_none());
        assert!(a.get("seed").is_none());
    }

    /// `num_ctx: 0` is the caller saying "I have no opinion", and the
    /// only way to say that to ollama is to leave the key out.
    ///
    /// Sent as a literal zero it is not ignored: ollama treats it as a
    /// real window, clamps it to a handful of tokens, and reloads the
    /// model to match — measured, on a model that was already resident
    /// at 8192, as a 5.3 s eviction down to 2048. `negotiate_ctx`
    /// returned 0 to MEAN "leave the load alone", so the option that was
    /// supposed to cost nothing reloaded the model on every ask.
    /// The server's own accounting, off each wire's terminal event.
    /// `reasoning_output_tokens` is the number no local engine reported
    /// before, and the reason the stats row can now show a model
    /// burning its budget on thinking instead of leaving it inferable
    /// only from an empty answer.
    #[test]
    fn generation_stats_come_off_the_terminal_event() {
        let lms = r#"data: {"type":"chat.end","result":{"stats":{"input_tokens":911,
            "total_output_tokens":485,"reasoning_output_tokens":458,
            "tokens_per_second":21.4,"time_to_first_token_seconds":1.9}}}"#;
        let g = Wire::LmStudio.gen_stats(lms).expect("chat.end parses");
        assert_eq!(g.out_tokens, 485);
        assert_eq!(g.reasoning_tokens, 458);
        assert_eq!(g.ttft_ms, 1900);
        assert!((g.tokens_per_second - 21.4).abs() < 0.01);
        // Anything that is not the end event is silence, not a zero.
        assert_eq!(
            Wire::LmStudio.gen_stats(r#"data: {"type":"message.delta","content":"hi"}"#),
            None
        );

        let oll = r#"{"done":true,"eval_count":32,"eval_duration":1000000000,
            "prompt_eval_duration":1600000000}"#;
        let g = Wire::Ollama.gen_stats(oll).expect("done line parses");
        assert_eq!(g.out_tokens, 32);
        assert_eq!(g.ttft_ms, 1600);
        assert!((g.tokens_per_second - 32.0).abs() < 0.01);
        // ollama cannot separate reasoning from output; 0 is honest.
        assert_eq!(g.reasoning_tokens, 0);
        assert_eq!(
            Wire::Ollama.gen_stats(r#"{"done":false,"response":"x"}"#),
            None
        );

        // The OpenAI wires: usage arrives in its own chunk before
        // `data: [DONE]`, with `choices` empty — the shape the spec
        // describes for `stream_options.include_usage`, which body()
        // now asks for.
        let oai = r#"data: {"choices":[],"usage":{"completion_tokens":128,
            "completion_tokens_details":{"reasoning_tokens":96}}}"#;
        let g = Wire::OpenAiChat.gen_stats(oai).expect("usage chunk parses");
        assert_eq!(g.out_tokens, 128);
        assert_eq!(g.reasoning_tokens, 96);
        // An ordinary content chunk carries no usage and must stay quiet
        // rather than reporting a confident zero.
        assert_eq!(
            Wire::OpenAiChat.gen_stats(r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#),
            None
        );
    }

    /// Asked for explicitly, because the spec says usage is withheld
    /// unless you do — and without it the stats row is blank on exactly
    /// the wires a hosted provider uses.
    #[test]
    fn streaming_openai_requests_its_usage_chunk() {
        let mut g = req("p", &[]);
        g.stream = true;
        for w in [Wire::OpenAi, Wire::OpenAiChat] {
            assert_eq!(w.body(&g)["stream_options"]["include_usage"], true, "{w:?}");
        }
        g.stream = false;
        for w in [Wire::OpenAi, Wire::OpenAiChat] {
            assert!(
                w.body(&g).get("stream_options").is_none(),
                "{w:?} not streaming"
            );
        }
    }

    #[test]
    fn a_window_of_zero_is_an_absent_key_not_a_zero() {
        let mut g = req("p", &[]);
        g.num_ctx = 0;
        assert!(Wire::Ollama.body(&g)["options"].get("num_ctx").is_none());
        // ...and a real window still travels.
        g.num_ctx = 8192;
        assert_eq!(Wire::Ollama.body(&g)["options"]["num_ctx"], 8192);
    }

    #[test]
    fn the_prompt_bytes_are_identical_across_wires() {
        // The point of /v1/completions over /v1/chat/completions: the
        // stable prefix the KV cache depends on survives the move.
        let p = "PREAMBLE...\nSession log:\nx\nQuestion: y\nAnswer:";
        let o = Wire::Ollama.body(&req(p, &[]));
        let a = Wire::OpenAi.body(&req(p, &[]));
        assert_eq!(o["prompt"], a["prompt"]);
        assert_eq!(o["prompt"].as_str().unwrap(), p);
    }

    #[test]
    fn budget_and_stop_move_to_their_openai_homes() {
        let b = Wire::OpenAi.body(&req("p", &["\n\n"]));
        assert_eq!(b["max_tokens"], 128);
        assert_eq!(b["stop"], serde_json::json!(["\n\n"]));
        // Both of these are server-side settings on an OpenAI wire, and
        // a strict server 400s on unknown fields rather than ignoring.
        assert!(b.get("options").is_none());
        assert!(b.get("keep_alive").is_none());
        assert!(b.get("num_ctx").is_none());
    }

    #[test]
    fn reasoning_crosses_as_effort_not_think() {
        let mut g = req("p", &[]);
        g.think = Some(serde_json::json!("high"));
        g.effort = Some("high");
        let o = Wire::Ollama.body(&g);
        let a = Wire::OpenAi.body(&g);
        assert_eq!(o["think"], "high");
        assert!(o.get("reasoning_effort").is_none());
        assert_eq!(a["reasoning_effort"], "high");
        assert!(a.get("think").is_none());
    }

    #[test]
    fn sse_terminator_is_a_done_not_a_parse_error() {
        assert!(Wire::OpenAi.chunk("data: [DONE]").unwrap().done);
        assert!(Wire::OpenAi.chunk("").is_none());
        assert!(Wire::OpenAi.chunk(": keep-alive").is_none());
    }

    #[test]
    fn sse_deltas_accumulate() {
        let c = Wire::OpenAi
            .chunk(r#"data: {"choices":[{"text":"ls ","finish_reason":null}]}"#)
            .unwrap();
        assert_eq!(c.text, "ls ");
        assert!(!c.done);
        let c = Wire::OpenAi
            .chunk(r#"data: {"choices":[{"text":"-la","finish_reason":"stop"}]}"#)
            .unwrap();
        assert_eq!(c.text, "-la");
        assert!(c.done);
    }

    #[test]
    fn ollama_streaming_is_unchanged() {
        let c = Wire::Ollama
            .chunk(r#"{"response":"ls","done":false}"#)
            .unwrap();
        assert_eq!(c.text, "ls");
        assert!(!c.done);
        assert!(
            Wire::Ollama
                .chunk(r#"{"response":"","done":true}"#)
                .unwrap()
                .done
        );
    }

    #[test]
    fn model_lists_read_from_each_shape() {
        let o = serde_json::json!({"models":[{"name":"qwen3:4b"},{"name":"gemma3:1b"}]});
        assert_eq!(Wire::Ollama.models(&o), vec!["qwen3:4b", "gemma3:1b"]);
        let a = serde_json::json!({"data":[{"id":"qwen3-4b"},{"id":"llama-3.2-3b"}]});
        assert_eq!(Wire::OpenAi.models(&a), vec!["qwen3-4b", "llama-3.2-3b"]);
    }

    #[test]
    fn a_chat_completions_answer_is_still_read() {
        let v = serde_json::json!({"choices":[{"message":{"content":"hi"}}]});
        assert_eq!(Wire::OpenAi.text(&v).as_deref(), Some("hi"));
    }

    #[test]
    fn trust_is_stated_and_auto_only_believes_loopback() {
        for h in [
            "http://127.0.0.1:11434",
            "http://localhost:1234",
            "http://[::1]:1234",
            "http://127.0.0.53:1234",
        ] {
            assert!(resolve_trust("auto", h), "{h} is this machine");
        }
        for h in [
            "https://api.openai.com",
            "http://192.168.1.9:1234",
            "http://gpu.lan:1234",
            "http://10.0.0.5:11434",
        ] {
            assert!(!resolve_trust("auto", h), "{h} is not");
        }
        // Stated always wins, in both directions — including trusting a
        // box on your own LAN, which auto cannot know about.
        assert!(resolve_trust("yes", "https://api.openai.com"));
        assert!(!resolve_trust("no", "http://127.0.0.1:11434"));
        // Anything unrecognised falls to auto, not to trust.
        assert!(!resolve_trust("maybe", "https://api.openai.com"));
    }

    #[test]
    fn urls() {
        assert_eq!(
            Wire::OpenAi.gen_url("http://localhost:1234"),
            "http://localhost:1234/v1/completions"
        );
        assert_eq!(
            Wire::OpenAi.models_url("http://localhost:1234"),
            "http://localhost:1234/v1/models"
        );
        // Every friendly spelling must land on CHAT. Raw completions
        // apply no template, so an instruction prompt is continued
        // instead of followed -- reachable only by asking for it by
        // name. This inverted once, silently, for a whole release.
        for s in ["openai", "openai-chat", "chat", "llamacpp", "vllm"] {
            assert_eq!(Wire::parse(s), Some(Wire::OpenAiChat), "{s} must be chat");
        }
        // LM Studio has a better door of its own.
        for s in ["lmstudio", "lms"] {
            assert_eq!(Wire::parse(s), Some(Wire::LmStudio), "{s} is native");
        }
        for s in ["openai-raw", "completions"] {
            assert_eq!(Wire::parse(s), Some(Wire::OpenAi), "{s} is opt-in raw");
        }
        assert_eq!(Wire::parse("nope"), None);
    }
}
