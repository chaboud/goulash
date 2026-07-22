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
    /// Hard cap on generated tokens per answer (the one-line contract is
    /// otherwise unenforced and small models will ramble on your GPU).
    pub max_tokens: usize,
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
            max_tokens: 256,
            num_ctx: 8192,
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
