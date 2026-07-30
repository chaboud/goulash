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
}

/// Facts about this machine, told to the model so it stops guessing.
///
/// Measured honestly: none of these produced a *statistically*
/// detectable improvement — 4/109 platform errors at baseline against
/// 3/131, 3/131 and 1/127, every confidence interval overlapping, and
/// ~1235 commands per arm needed to resolve a halving where ~120 were
/// available. `platform` is on anyway because "prove it helps" is the
/// wrong bar for a statement that is free and certainly true; the right
/// bar is "prove it hurts", and nothing suggests it does.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DivulgeConfig {
    /// OS, userland flavour and shell, plus the BSD/GNU differences that
    /// account for 78% of observed platform errors (`du --max-depth` is
    /// 62 of 78 on its own). ~50 tokens, and verified free: per-token
    /// prompt-eval is identical with and without it once cached
    /// (750 vs 760us by turn 10).
    pub platform: bool,
    /// Which of a curated tool set is installed. Debug: targets
    /// absent-tool references, which fire 25 times in 4002 commands, and
    /// carries a curation problem (which tools, maintained by whom).
    pub tools: bool,
    /// Every executable on PATH — ~3900 tokens. Debug only: showed no
    /// benefit at 7x the context, and nearly doubled prompt-eval.
    /// Replaces `tools` rather than adding to it.
    pub full_path: bool,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            thinking: "off".into(),
            response_tokens: 512,
            reasoning_tokens: 0,
        }
    }
}

impl Default for DivulgeConfig {
    fn default() -> Self {
        Self {
            platform: true,
            tools: false,
            full_path: false,
        }
    }
}

/// One inference tier.
///
/// FAST and SLOW run the SAME model by default, differing only in
/// whether reasoning is on — so switching tiers costs nothing. Naming a
/// different model per tier is allowed but means a load on every switch
/// (206ms reuse vs 1847ms reload), so it is opt-in rather than the shape
/// the design assumes.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TierConfig {
    /// Empty = use `[engine] model`.
    pub model: String,
    /// `off` | `auto` | `forced`.
    pub thinking: String,
    pub response_tokens: usize,
    pub reasoning_tokens: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    /// "auto" probes for a local engine (ollama today); "ollama" requires
    /// it; "none" disables LLM features entirely.
    pub provider: String,
    pub host: String,
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
    /// Cap on the tokens that become the *visible* answer.
    ///
    /// Deliberately generous: measured across ~4000 generations, answers
    /// that arrive use a median of 32 tokens (p90 77) and visible prose
    /// runs ~61 chars, so this ceiling never binds. Brevity comes from
    /// the directive and from the band clamping at draw time, not from
    /// starving the budget. (bench/THINKING.md)
    pub response_tokens: usize,
    /// Allowance for reasoning, spent *on top of* `response_tokens` and
    /// only when thinking is enabled.
    ///
    /// Providers meter reasoning and output on one counter, so a shared
    /// cap means a reasoning model spends the display budget thinking and
    /// returns nothing. 4096 is a floor, not a luxury: with a 1024
    /// allowance 19 of 32 cells still ran out mid-thought.
    pub reasoning_tokens: usize,
    /// `off` (suppress) | `auto` (only where it is known to help, and
    /// never where the model cannot do it) | `forced` (debug).
    ///
    /// `auto` MUST consult capability first — ollama returns HTTP 400
    /// `"<model>" does not support thinking` rather than degrading, and 8
    /// of 24 measured cells fail that way.
    pub thinking: String,
    /// Floor for the context window: adopt whatever a resident model is
    /// already loaded with, and only intervene below this.
    ///
    /// `num_ctx` is part of a model's load identity — asking for a
    /// different one evicts and reloads (206ms reuse vs 1847ms reload),
    /// so insisting on an exact value thrashes whatever the user has
    /// loaded. (bench/RESIDENCY.md)
    pub num_ctx_min: usize,
    /// Pin the context window exactly. `None` uses `num_ctx_min` as a
    /// floor instead. For reproducibility and for bounding KV memory on a
    /// small machine.
    pub num_ctx: Option<usize>,
    /// Prefer a model the engine already has loaded over the
    /// smallest-installed default.
    ///
    /// Off by default: it changes *which model answers you*, which is a
    /// bigger behavioural shift than the numbers alone justify. The
    /// numbers do favour it — cold load is p50 4281ms / p90 7214ms
    /// against a warm TTFT of 2314ms, so adopting a warm 12B is very
    /// plausibly faster than cold-loading a 0.8B.
    pub prefer_resident: bool,
    /// Machine facts told to the model. See [`DivulgeConfig`].
    pub divulge: DivulgeConfig,
    /// The immediate tier. `#` answers from here first, always.
    pub fast: TierConfig,
    /// The considered tier. `#` amends with it underneath; `#?` goes
    /// straight here and has FAST compress the result.
    pub slow: TierConfig,
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

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_string(),
            host: "http://127.0.0.1:11434".to_string(),
            model: None,
            favorites: Vec::new(),
            keep_alive: "30m".to_string(),
            stream: true,
            context_max_chars: 12_000,
            tail_chars: 800,
            response_tokens: 512,
            reasoning_tokens: 4096,
            thinking: "off".to_string(),
            num_ctx_min: 8192,
            num_ctx: None,
            prefer_resident: false,
            divulge: DivulgeConfig::default(),
            fast: TierConfig {
                model: String::new(),
                // FAST exists to be immediate; thinking is what makes it
                // not immediate.
                thinking: "off".into(),
                response_tokens: 512,
                reasoning_tokens: 0,
            },
            slow: TierConfig {
                model: String::new(),
                thinking: "auto".into(),
                response_tokens: 512,
                // 4096 is a floor: at 1024, 19 of 32 cells still ran out
                // mid-thought. (bench/THINKING.md)
                reasoning_tokens: 4096,
            },
            prewarm: true,
            debug: false,
            commentary: true,
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
}

impl Default for StatusConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rows: 1,
            band: true,
            band_rows: 1,
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

    /// Surgically set any dotted key, preserving comments and formatting.
    /// Same write-through `--config set` uses, so `#/thinking auto save`
    /// and the CLI cannot drift.
    pub fn persist(key: &str, value: &str) -> Result<(), String> {
        let path = Self::dir().ok_or("no home dir")?.join("config.toml");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut doc: toml_edit::DocumentMut =
            text.parse().map_err(|e| format!("config parse: {e}"))?;
        let parts: Vec<&str> = key.split('.').collect();
        if parts.len() < 2 {
            return Err("key must be dotted".into());
        }
        let parsed: toml_edit::Value = value
            .parse()
            .unwrap_or_else(|_| toml_edit::Value::from(value));
        let mut node = doc.as_table_mut();
        for seg in &parts[..parts.len() - 1] {
            if node.get(seg).is_none() {
                node[seg] = toml_edit::table();
            }
            node = node[seg]
                .as_table_mut()
                .ok_or_else(|| format!("{seg} is not a table"))?;
        }
        node[parts[parts.len() - 1]] = toml_edit::value(parsed);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, doc.to_string()).map_err(|e| e.to_string())
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
    use super::edit_model;

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
}
