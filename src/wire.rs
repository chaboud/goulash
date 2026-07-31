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

    /// What this server has loaded, and how big its window is. Empty on
    /// any failure — a residency check that cannot answer must not be
    /// mistaken for "nothing is loaded", so callers treat empty as
    /// "unknown" and leave the server's own setting alone.
    pub fn resident(&self) -> Vec<(String, usize)> {
        let Some(url) = self.be.wire.ps_url(&self.be.host) else {
            return Vec::new();
        };
        self.get(&url)
            .call()
            .ok()
            .and_then(|r| r.into_string().ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| self.be.wire.resident(&v))
            .unwrap_or_default()
    }
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

    /// `(model, context_length)` for each loaded model.
    pub fn resident(&self, v: &Value) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        if *self != Wire::Ollama {
            return out;
        }
        for m in v["models"].as_array().into_iter().flatten() {
            let name = m["model"]
                .as_str()
                .or_else(|| m["name"].as_str())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            // ollama reports the loaded window under context_length; an
            // older build omits it, and a missing number must not read
            // as "zero context" or every ask would force a reload.
            let ctx = m["context_length"].as_u64().unwrap_or(0) as usize;
            out.push((name.to_string(), ctx));
        }
        out
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
