use serde::Deserialize;
use std::path::PathBuf;

/// Loaded from `~/.goulash/config.toml`; every field has a working default
/// so goulash runs with no config file at all.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub status: StatusConfig,
    pub record: RecordConfig,
    pub shell: ShellConfig,
    pub engine: EngineConfig,
    pub debug: DebugConfig,
    /// Per-model capability overrides, keyed by model name (with or
    /// without the `:tag`) — the escape hatch for a model that shipped
    /// after goulash's table did. See models.rs.
    ///
    /// ```toml
    /// [models."gpt-oss:20b"]
    /// thinking = "levels"       # none | bool | levels
    /// reasoning_tokens = 2048
    /// ```
    pub models: crate::models::Overrides,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    /// "auto" probes each known local engine in turn; "ollama",
    /// "openai" (also spelled lmstudio / llamacpp / vllm — one wire,
    /// several servers) pin one; "none" disables LLM features entirely.
    pub provider: String,
    /// Where ollama answers.
    pub host: String,
    /// Where an OpenAI-compatible server answers. LM Studio's default
    /// port; llama.cpp's server and vLLM speak the same `/v1`.
    pub openai_host: String,
    /// Name of the environment variable holding a bearer token, for the
    /// case where the endpoint is NOT on this machine. Empty for LM
    /// Studio and ollama, which want no auth.
    pub api_key_env: String,
    /// Whether this backend may be shown pinned file content
    /// (wiki: working-context.md). `yes` | `no` | `auto`, where auto
    /// means "trust a loopback host, nothing else". Stated, never
    /// inferred from some other setting having a convenient value —
    /// trust is a decision, not a side effect.
    pub trusted: String,
    /// Per-lane overrides. Everything above is the default for both
    /// lanes; anything set here applies to the SLOW lane only, so the
    /// two can be different models, different servers, or different
    /// machines. Absent (or identical) means one lane serving both,
    /// which is the common case and costs nothing extra.
    pub slow_lane: LaneOverride,
    /// Model name; overrides favorites and auto-pick.
    pub model: Option<String>,
    /// Preference-ordered favorites: the first one installed wins during
    /// auto-pick (before falling back to smallest-installed).
    pub favorites: Vec<String>,
    /// Keep the model resident between asks (ollama keep_alive; "" to
    /// let the server unload on its own schedule).
    pub keep_alive: String,
    /// Stream tokens into the bar as they arrive.
    pub stream: bool,
    /// Prompt budget: max chars of session-log context sent per ask;
    /// the log epoch-trims to half this at a block boundary.
    pub context_max_chars: usize,
    /// Per-block output-tail chars kept in the session log.
    pub tail_chars: usize,
    /// Cap on the *response* — the part that lands in the band. Streaming
    /// means a longer tail costs nothing until it is actually generated,
    /// so this is a runaway backstop, not a latency lever.
    pub max_tokens: usize,
    /// Extra allowance for masked reasoning spend, added to `max_tokens`
    /// when `thinking` is on. Thinking shares the provider's single
    /// token meter, so without this a thinking model eats the answer.
    pub thinking_tokens: usize,
    /// off | low | medium | high. Reasoning models put these tokens in a
    /// separate field and can spend the whole budget invisibly, which is
    /// why the default is off.
    pub thinking: String,
    /// Emit the `CMD:` line BEFORE the prose. The command is the payload:
    /// putting it first means truncation eats the explanation instead of
    /// the command, and the suggestion can vend mid-stream.
    pub command_first: bool,
    /// When the slow lane engages, as a ladder rather than a toggle:
    ///
    /// - `off` — never.
    /// - `manual` — only when asked, `#?` / `?`.
    /// - `ingest` — also on `#@`, to classify and card a pin. Bounded,
    ///   and the user triggered it by pinning. **Default.**
    /// - `volunteer` — also on ordinary `#` asks, contributing an
    ///   amendment when it finds something better. Unbounded: fires on
    ///   everything typed.
    ///
    /// (wiki: architecture/two-lane-engagement.md)
    pub slow: String,
    /// A superseded or stale research job can be picked up later and
    /// amended into the turn it belonged to. Costs nothing in attention
    /// — an amendment for an old turn never intrudes — and costs compute
    /// for something nobody may look at.
    pub backfill_abandoned: bool,
    /// Hard bounds on one research job, since slow tool-calls and can
    /// therefore spin. Reported when hit rather than silently truncated.
    pub slow_max_steps: usize,
    pub slow_max_secs: u64,
    /// Total characters all `#@` pinned files together may spend in the
    /// stable prefix. Shared equally between pins; anything over its
    /// share is outlined rather than truncated (context.rs).
    pub context_files_max_chars: usize,
    /// Context window requested from the provider; bounds KV memory
    /// (ollama may otherwise load models at huge default contexts).
    pub num_ctx: usize,
    /// Load the model in the background at bind/switch time so the first
    /// ask doesn't pay the cold start.
    pub prewarm: bool,
    /// Record raw engine responses into the session transcript for
    /// debugging (ev: "engine_debug").
    pub debug: bool,
    /// Proactive commentary: after each command turn the model may
    /// volunteer one short tip (or stay silent). Toggle live with
    /// #/commentary.
    pub commentary: bool,
}

/// Everything that decides WHERE a lane talks and WHAT it binds. The
/// rest of `EngineConfig` is behaviour, shared by both lanes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneConfig {
    pub provider: String,
    pub host: String,
    pub openai_host: String,
    pub api_key_env: String,
    pub trusted: String,
    pub model: Option<String>,
    pub favorites: Vec<String>,
}

/// The same keys, all optional: unset means "inherit from `[engine]`".
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct LaneOverride {
    pub provider: Option<String>,
    pub host: Option<String>,
    pub openai_host: Option<String>,
    pub api_key_env: Option<String>,
    pub trusted: Option<String>,
    pub model: Option<String>,
    pub favorites: Option<Vec<String>>,
}

impl LaneOverride {
    fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.host.is_none()
            && self.openai_host.is_none()
            && self.api_key_env.is_none()
            && self.trusted.is_none()
            && self.model.is_none()
            && self.favorites.is_none()
    }
}

impl EngineConfig {
    /// The lane that answers the user directly.
    pub fn fast_lane(&self) -> LaneConfig {
        LaneConfig {
            provider: self.provider.clone(),
            host: self.host.clone(),
            openai_host: self.openai_host.clone(),
            api_key_env: self.api_key_env.clone(),
            trusted: self.trusted.clone(),
            model: self.model.clone(),
            favorites: self.favorites.clone(),
        }
    }

    /// The research lane: the fast lane with `[engine.slow_lane]`
    /// applied over it.
    pub fn slow_lane(&self) -> LaneConfig {
        let base = self.fast_lane();
        let o = &self.slow_lane;
        LaneConfig {
            provider: o.provider.clone().unwrap_or(base.provider),
            host: o.host.clone().unwrap_or(base.host),
            openai_host: o.openai_host.clone().unwrap_or(base.openai_host),
            api_key_env: o.api_key_env.clone().unwrap_or(base.api_key_env),
            trusted: o.trusted.clone().unwrap_or(base.trusted),
            // A slow override with no model of its own still means a
            // separate lane — a different HOST is a perfectly good
            // reason to split, and the model there may auto-pick
            // differently anyway.
            model: o.model.clone().or(base.model),
            favorites: o.favorites.clone().unwrap_or(base.favorites),
        }
    }

    /// Is the slow lane worth resolving separately? When it isn't, both
    /// roles share one binding — which is the point: two lanes on one
    /// model must not mean two model loads.
    pub fn lanes_split(&self) -> bool {
        !self.slow_lane.is_empty() && self.fast_lane() != self.slow_lane()
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_string(),
            host: "http://127.0.0.1:11434".to_string(),
            openai_host: "http://127.0.0.1:1234".to_string(),
            api_key_env: String::new(),
            trusted: "auto".to_string(),
            slow_lane: LaneOverride::default(),
            model: None,
            favorites: Vec::new(),
            keep_alive: "30m".to_string(),
            stream: true,
            context_max_chars: 12_000,
            tail_chars: 800,
            max_tokens: 512,
            thinking_tokens: 512,
            thinking: "off".to_string(),
            command_first: true,
            slow: "ingest".to_string(),
            backfill_abandoned: false,
            slow_max_steps: 8,
            slow_max_secs: 180,
            context_files_max_chars: 6000,
            num_ctx: 8192,
            prewarm: true,
            debug: false,
            commentary: true,
        }
    }
}

/// Terminal hackery, behind `#/debug` — the drawer for behaviours that
/// are real levers on how goulash talks to the emulator, but which most
/// people should never have to think about. Defaults are the shipped
/// behaviour; every one of these exists so a field problem can be
/// bisected live instead of by rebuilding.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DebugConfig {
    /// How the cursor is put back after painting the reserved rows.
    ///
    /// `decsc` uses the terminal's own save/restore (ESC 7 / ESC 8),
    /// which is the only form that preserves **deferred wrap** — the
    /// state a terminal is in after a glyph lands in the last column,
    /// where the cursor reads as "still on this row" but the next glyph
    /// wraps. `absolute` re-homes with CUP from our mirror, which cannot
    /// express that flag and so silently cancels it. (wiki:
    /// architecture/status-rows.md)
    pub cursor_save: String,
    /// The unprovoked repaint every few idle ticks: insurance against
    /// output we mis-parsed, paid for with a write into a stream the
    /// line editor believes it owns. Turn it off to find out whether it
    /// is buying anything.
    pub idle_repaint: bool,
    /// Skip a paint while the inner cursor sits in the last column,
    /// deferring it to the next tick. Belt-and-braces on top of
    /// `cursor_save = "decsc"`, and a way to isolate wrap effects.
    pub wrap_guard: bool,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            cursor_save: "decsc".to_string(),
            idle_repaint: true,
            wrap_guard: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ShellConfig {
    /// Auto-inject shell integration (ZDOTDIR trick for zsh, --rcfile
    /// wrapper for bash) when launching a known shell with plain flags.
    pub auto_integrate: bool,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            auto_integrate: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RecordConfig {
    pub enabled: bool,
    /// Also record raw output bytes (base64) in the transcript, not just
    /// state annotations.
    pub output: bool,
}

impl Default for RecordConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            output: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StatusConfig {
    pub enabled: bool,
    pub rows: u16,
    /// The heckle band: extra reserved rows above the status row for the
    /// asked question and the answer/explanation text.
    pub band: bool,
    /// Max explanation rows in the band (question row is extra).
    pub band_rows: u16,
    /// Item rows a menu tries to show. The area grows to fit while a
    /// menu is open and gives the rows back on close — capped so the
    /// shell keeps at least MENU_MIN_INNER rows.
    pub menu_rows: u16,
}

impl Default for StatusConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rows: 1,
            band: true,
            band_rows: 1,
            menu_rows: 8,
        }
    }
}

impl Config {
    pub fn dir() -> Option<PathBuf> {
        std::env::var_os("GOULASH_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".goulash")))
    }

    pub fn load() -> Config {
        let Some(path) = Self::dir().map(|d| d.join("config.toml")) else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                eprintln!("goulash: ignoring bad config {}: {err}", path.display());
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    /// Surgically set (or clear, for auto) `[engine] model` in
    /// config.toml, preserving the user's comments and formatting —
    /// never a full re-serialize.
    pub fn persist_model(name: Option<&str>) -> Result<(), String> {
        let path = Self::dir().ok_or("no home dir")?.join("config.toml");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let edited = edit_model(&text, name)?;
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, edited).map_err(|e| e.to_string())
    }

    /// Rows kept out of the inner PTY's world. With the band enabled the
    /// goulash area holds a FIXED height (rule + question + text rows +
    /// chrome) so the terminal never resizes mid-session.
    pub fn reserved_rows(&self) -> u16 {
        if !self.status.enabled {
            return 0;
        }
        if self.status.band {
            3 + self.status.band_rows.clamp(1, 4)
        } else {
            self.status.rows.clamp(1, 8)
        }
    }
}

impl Config {
    /// Surgically set any `[section] key = value` in config.toml. Values
    /// are typed from the string: `true/false` -> bool, digits -> int,
    /// everything else -> string.
    pub fn persist_key(section: &str, key: &str, value: &str) -> Result<(), String> {
        let path = Self::dir().ok_or("no home dir")?.join("config.toml");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut doc: toml_edit::DocumentMut =
            text.parse().map_err(|e| format!("config parse: {e}"))?;
        if doc.get(section).is_none() {
            doc[section] = toml_edit::table();
        }
        doc[section][key] = match value {
            "true" => toml_edit::value(true),
            "false" => toml_edit::value(false),
            v => match v.parse::<i64>() {
                Ok(n) => toml_edit::value(n),
                Err(_) => toml_edit::value(v),
            },
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, doc.to_string()).map_err(|e| e.to_string())
    }
}

fn edit_model(text: &str, name: Option<&str>) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("config parse: {e}"))?;
    match name {
        Some(n) => {
            if doc.get("engine").is_none() {
                doc["engine"] = toml_edit::table();
            }
            doc["engine"]["model"] = toml_edit::value(n);
        }
        None => {
            if let Some(t) = doc.get_mut("engine").and_then(|e| e.as_table_mut()) {
                t.remove("model");
            }
        }
    }
    Ok(doc.to_string())
}

#[cfg(test)]
mod persist_tests {
    use super::{Config, EngineConfig, edit_model};

    #[test]
    fn surgical_edit_preserves_comments() {
        let src = "# my precious comment\n[engine]\nhost = \"http://x\" # inline note\n";
        let out = edit_model(src, Some("gemma3:4b")).unwrap();
        assert!(out.contains("# my precious comment"));
        assert!(out.contains("# inline note"));
        assert!(out.contains("model = \"gemma3:4b\""));
        // auto: the key is removed, comments still intact
        let back = edit_model(&out, None).unwrap();
        assert!(!back.contains("model ="));
        assert!(back.contains("# my precious comment"));
    }

    #[test]
    fn edit_works_on_empty_config() {
        let out = edit_model("", Some("qwen3:1.7b")).unwrap();
        assert!(out.contains("[engine]"));
        assert!(out.contains("model = \"qwen3:1.7b\""));
    }

    fn engine_from(toml_src: &str) -> EngineConfig {
        let c: Config = toml::from_str(toml_src).expect("parses");
        c.engine
    }

    #[test]
    fn one_lane_serves_both_until_told_otherwise() {
        let e = engine_from("[engine]\nmodel = \"qwen3:4b\"\n");
        assert!(
            !e.lanes_split(),
            "an absent [engine.slow_lane] must not cost a second binding \
             \u{2014} two lanes on one model would mean two model loads"
        );
        assert_eq!(e.fast_lane(), e.slow_lane());
    }

    #[test]
    fn a_slow_override_inherits_everything_it_does_not_state() {
        let e = engine_from(
            "[engine]\n\
             provider = \"ollama\"\n\
             host = \"http://127.0.0.1:11434\"\n\
             model = \"qwen3:4b\"\n\
             favorites = [\"a\", \"b\"]\n\
             [engine.slow_lane]\n\
             model = \"qwen3:30b\"\n",
        );
        assert!(e.lanes_split());
        let slow = e.slow_lane();
        assert_eq!(slow.model.as_deref(), Some("qwen3:30b"));
        assert_eq!(slow.host, "http://127.0.0.1:11434", "host inherited");
        assert_eq!(slow.provider, "ollama", "provider inherited");
        assert_eq!(slow.favorites, vec!["a", "b"], "favorites inherited");
        assert_eq!(e.fast_lane().model.as_deref(), Some("qwen3:4b"));
    }

    #[test]
    fn the_lanes_can_be_different_servers_entirely() {
        // The billing-vs-mailing-address case: a small local model
        // answering, a big one elsewhere researching.
        let e = engine_from(
            "[engine]\n\
             provider = \"ollama\"\n\
             [engine.slow_lane]\n\
             provider = \"lmstudio\"\n\
             openai_host = \"http://192.168.1.9:1234\"\n\
             trusted = \"yes\"\n",
        );
        assert!(e.lanes_split());
        assert_eq!(e.fast_lane().provider, "ollama");
        assert_eq!(e.slow_lane().provider, "lmstudio");
        assert_eq!(e.slow_lane().openai_host, "http://192.168.1.9:1234");
        // Stated trust survives a host auto would refuse.
        assert_eq!(e.slow_lane().trusted, "yes");
        assert_eq!(e.fast_lane().trusted, "auto");
    }

    #[test]
    fn a_slow_table_that_changes_nothing_does_not_split() {
        let e = engine_from("[engine]\nmodel = \"m\"\n[engine.slow_lane]\nmodel = \"m\"\n");
        assert!(
            !e.lanes_split(),
            "restating the same values is not a reason to bind twice"
        );
    }
}
