//! Provider wire formats.
//!
//! goulash sends a **raw completion prompt**, not chat messages, and
//! that is not incidental: the whole engine design is a stable prefix
//! plus prefix KV caching (wiki: llm-engine.md). Chat-completions hands
//! prompt assembly to the server's template, which moves the prefix
//! boundary out from under us and can silently stop the cache from
//! hitting. So the OpenAI-compatible path targets `/v1/completions`,
//! which llama.cpp's server, LM Studio and vLLM all expose — **same
//! prompt bytes, different envelope**.
//!
//! Everything provider-shaped lives here. The engine builds a `Gen` and
//! reads back text; it never learns which server it is talking to.

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire {
    Ollama,
    /// OpenAI-compatible `/v1`: LM Studio, llama.cpp server, vLLM, and
    /// the hosted API itself.
    OpenAi,
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
            Wire::OpenAi => "openai",
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
        self.auth(self.agent.post(url))
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
    pub keep_alive: &'a str,
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
            "openai" | "lmstudio" | "llamacpp" | "vllm" => Some(Wire::OpenAi),
            _ => None,
        }
    }

    pub fn gen_url(&self, host: &str) -> String {
        match self {
            Wire::Ollama => format!("{host}/api/generate"),
            Wire::OpenAi => format!("{host}/v1/completions"),
        }
    }

    pub fn models_url(&self, host: &str) -> String {
        match self {
            Wire::Ollama => format!("{host}/api/tags"),
            Wire::OpenAi => format!("{host}/v1/models"),
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
                        "num_ctx": g.num_ctx as i64,
                    },
                });
                if !g.stop.is_empty() {
                    b["options"]["stop"] = json!(g.stop);
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
                // No num_ctx and no keep_alive: both are server-side
                // settings there, not per-request ones. Sending them
                // anyway would be noise at best and a 400 at worst.
                if let Some(e) = g.effort {
                    b["reasoning_effort"] = json!(e);
                }
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
            Wire::OpenAi => {
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
            Wire::OpenAi => (&v["data"], "id"),
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
            keep_alive: "30m",
        }
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
        assert_eq!(Wire::parse("lmstudio"), Some(Wire::OpenAi));
        assert_eq!(Wire::parse("nope"), None);
    }
}
