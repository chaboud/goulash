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

/// What goulash tells the model about the machine it is running on.
///
/// Three independent switches, not a ladder — `full_path` *replaces*
/// `tools` rather than adding to it.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct DivulgeConfig {
    /// OS, userland and shell. **On by default.**
    ///
    /// Not because it measurably helps — at n≈120 per arm nothing here
    /// reaches significance — but because "prove it helps" is the wrong
    /// test for a statement that is free and certainly true. Verified
    /// free: per-token prompt-eval is identical with and without it
    /// (750 vs 760 us by turn 10), because it lives in the cached prefix.
    /// The right test is "prove it hurts", and nothing suggests it does.
    pub platform: bool,
    /// Which of a curated tool set is installed. Debug: targets
    /// absent-tool references, which fire 25 times in 4002 commands, and
    /// carries an unsolved curation problem (which tools, maintained by
    /// whom).
    pub tools: bool,
    /// Every executable on PATH — ~3900 tokens. Debug only: no benefit
    /// at 7x the context, and it nearly doubled prompt-eval.
    pub full_path: bool,
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
    /// Cap on ONE generation, reasoning and answer together.
    ///
    /// A **runaway backstop, not a brevity lever**. Brevity comes from
    /// the directive asking for one short line and from the band
    /// clamping at draw time; measured across ~4000 generations, answers
    /// that arrive use a median of 32 tokens, so this ceiling does no
    /// display work at all. Streaming means an unspent ceiling costs
    /// nothing until it is actually generated.
    ///
    /// There is deliberately no separate thinking budget. Providers
    /// meter reasoning and output on one counter, and **we cannot
    /// control whether a model reasons**: a chat template turns it on
    /// regardless of what we send, `deepseek-r1` reasons through
    /// `think:false`, and the one kwarg that disables it on an
    /// OpenAI-compatible server empties `content` instead. Splitting the
    /// budget therefore only ever produced starvation — an empty answer
    /// with the whole allowance spent thinking. So the cap is single and
    /// generous, and whatever the engine does inside it is the engine's
    /// business.
    pub max_tokens: usize,
    /// off | low | medium | high. Reasoning models put these tokens in a
    /// separate field and can spend the whole budget invisibly, which is
    /// why the default is off.
    pub thinking: String,
    /// Emit the `CMD:` line BEFORE the prose. The command is the payload:
    /// putting it first means truncation eats the explanation instead of
    /// the command, and the suggestion can vend mid-stream.
    pub command_first: bool,
    /// When the slow lane engages *unasked*, as a ladder rather than a
    /// toggle:
    ///
    /// - `manual` — only when you ask, `#?` / `?`. **Default.**
    /// - `query` — on `#` as well as `#?`.
    /// - `waldorf` — whenever fast runs, heckling from the balcony
    ///   after the fact. Unbounded: fires on everything typed.
    ///
    /// There is no `off`. `#?` IS the request for this lane, and a
    /// pin always goes to it — a setting that could refuse either
    /// would make the key silently dead. The ladder says when the lane
    /// speaks up on its own, nothing more.
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
    /// Bounds on a directory pin's walk. A tree pin is a convenience,
    /// not a crawler — but "convenience" for a source tree is a couple
    /// of hundred files, not sixty, and the right number depends on the
    /// tree, so it is yours to set. What lands in the prompt is bounded
    /// by `context_files_max_chars` regardless; this bounds the READ.
    pub context_tree_max_files: usize,
    pub context_tree_max_depth: usize,
    /// Context window requested from the provider; bounds KV memory
    /// (ollama may otherwise load models at huge default contexts).
    /// Context window to demand, **exactly**, on every request.
    ///
    /// Zero means "negotiate" — see `num_ctx_min`. A non-zero value is a
    /// pin: the user wants this window and accepts the reload it costs.
    pub num_ctx: usize,
    /// The smallest window goulash can work in.
    ///
    /// `num_ctx` is part of a model's *load identity*, so naming one
    /// evicts and reloads anything already loaded at a different size —
    /// measured 206ms to reuse a warm model against 1847ms to reload it.
    /// Both engines already let the user choose a default (ollama has a
    /// context slider, LM Studio stores one per model), so the polite
    /// behaviour is to take whatever they picked and say nothing.
    ///
    /// The exception is a window too small to hold a session log. Then
    /// goulash asks for this floor and eats the reload, once.
    pub num_ctx_min: usize,
    /// Whether to nudge a host whose loaded window is below the floor.
    ///
    /// Off means "never provoke a reload": goulash works in whatever is
    /// loaded, however small, and the user owns the consequence.
    pub nudge_small_context: bool,
    /// Leading prompt tokens the server must keep when the context
    /// overflows. It truncates from the LEFT, and the left is where the
    /// grammar and the pinned memories live — so the default is sized
    /// to cover the preamble plus a full memory store. 0 disables.
    pub num_keep: usize,
    /// Fixed sampling seed. Negative leaves the server to its own
    /// randomness; set it when you want a field report to reproduce.
    pub seed: i64,
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
    /// Machine facts prepended to the cached prefix (facts.rs).
    pub divulge: DivulgeConfig,
    /// The shell goulash actually launched, for the platform line.
    ///
    /// Runtime, not a config key — it is an observation, and a stale
    /// one written into a file would outlive the session that made it
    /// true. `$SHELL` cannot answer this: it names the LOGIN shell, so
    /// `goulash bash` from a zsh login told the model "zsh" and got
    /// zsh-flavoured advice for a bash prompt.
    #[serde(skip)]
    pub shell: String,
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
    /// Token ceiling for this lane alone. `None` follows the fast lane.
    /// A considered answer can afford a longer one than a heckle.
    pub max_tokens: Option<usize>,
    /// Reasoning effort for this lane alone. `None` follows the fast
    /// lane — which is the common case, and the reason the two lanes
    /// exist at all is that it often should NOT: fast answers, slow
    /// thinks.
    pub thinking: Option<String>,
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
            // Generous on purpose: it is a backstop against a
            // runaway, and a ceiling that binds is how a reasoning
            // model returns nothing at all.
            max_tokens: 8192,
            thinking: "off".to_string(),
            command_first: true,
            slow: "manual".to_string(),
            backfill_abandoned: false,
            slow_max_steps: 8,
            slow_max_secs: 180,
            context_files_max_chars: 6000,
            context_tree_max_files: 256,
            context_tree_max_depth: 4,
            // Zero: negotiate rather than demand. This used to be a
            // flat 8192 sent on every request, which silently reloaded
            // any model the user had loaded at a different size.
            num_ctx: 0,
            num_ctx_min: 8192,
            nudge_small_context: true,
            num_keep: 512,
            seed: -1,
            prewarm: true,
            debug: false,
            commentary: true,
            divulge: DivulgeConfig::default(),
            shell: String::new(),
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
    /// Show the sharp settings in `#/settings`.
    ///
    /// Persistent on purpose — Android's developer options, not a
    /// session toggle. Someone who needed these once will need them
    /// again, and hiding them behind a switch that forgets is the same
    /// as not having the switch.
    pub show_advanced: bool,
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
    /// The unprovoked repaint after output settles: insurance against
    /// something we mis-parsed, paid for with a write into a stream the
    /// line editor believes it owns.
    ///
    /// **Off by default.** It is a guess that the band is damaged, and
    /// the cure writes to the terminal at a moment nothing asked it to.
    /// Every paint that matters is already triggered by something real.
    pub idle_repaint: bool,
    /// Skip a paint while the inner cursor sits in the last column,
    /// deferring it to the next tick. Belt-and-braces on top of
    /// `cursor_save = "decsc"`, and a way to isolate wrap effects.
    pub wrap_guard: bool,
    /// Show the working bar — the sweeping indicator that says the
    /// command in the slot belongs to the PREVIOUS question while a new
    /// answer is on its way.
    ///
    /// On by default because the thing it prevents is pulling a command
    /// you did not ask for. Off is for anyone who finds motion in the
    /// periphery worse than that risk, which is a real preference.
    pub working_bar: bool,
    /// Hand every slow finding to fast to re-voice before it is shown.
    ///
    /// The wiki's original contract was "you always hear fast" — slow
    /// researches, fast relays — so there is one voice and slow's output
    /// never reaches the band unmediated. Measured against it: a
    /// competent slow model already returns house-shaped output (CMD,
    /// one line, REASON), so the relay is a second round trip buying a
    /// reformat of something already formatted. Off by default for that
    /// reason; on to get the documented behaviour back, and to compare.
    pub slow_via_fast: bool,
    /// Show the wave for goulash's own unprompted commentary too.
    ///
    /// Off: an observation you did not ask for is not worth moving
    /// something in your peripheral vision. The lane dots already say
    /// the engine is busy, and that is the right weight for work nobody
    /// is waiting on.
    pub working_bar_on_watch: bool,
    /// How long the head takes to move one cell, in ms. Lower is
    /// smoother and writes more.
    pub working_bar_step_ms: u64,
    /// How long the bar takes to slide in, and (at ~70% of this) back
    /// out. The entrance and exit are what the eye actually reads, so
    /// this is the knob that decides whether it feels like a slide or a
    /// pop.
    pub working_bar_grow_ms: u64,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            show_advanced: false,
            cursor_save: "decsc".to_string(),
            idle_repaint: false,
            wrap_guard: false,
            slow_via_fast: false,
            working_bar: true,
            working_bar_on_watch: false,
            // Both of these MUST appear in the menu's value lists
            // (session.rs TERMINAL_ROWS) — a default the row cannot
            // find renders fine and then cycles from somewhere else.
            working_bar_step_ms: 150,
            working_bar_grow_ms: 350,
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
    /// Show runaway diagnostics in the chrome row: memory, engine call
    /// counts, queue depths, context size, disk. Off by default — this
    /// is for watching whether something climbs, not for daily use.
    pub stats: bool,
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
            stats: false,
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
        let node = descend(&mut doc, section)?;
        let item = match value {
            "true" => toml_edit::value(true),
            "false" => toml_edit::value(false),
            v => match v.parse::<i64>() {
                Ok(n) => toml_edit::value(n),
                Err(_) => toml_edit::value(v),
            },
        };
        node.insert(key, item);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, doc.to_string()).map_err(|e| e.to_string())
    }

    /// Delete one key, leaving its section and everything else alone.
    ///
    /// "Follow the other lane" has to be the ABSENCE of an override, not
    /// a copy of what the other lane currently says. Writing the value
    /// would freeze it: the setting would read "same as fast" while
    /// silently no longer tracking it.
    /// Settings whose value is not one goulash knows.
    ///
    /// An unknown value is not a parse error — the file is still valid
    /// TOML — and that is exactly the problem. The menu falls back to
    /// the first option, the setting is dead, and nothing says so. A
    /// RETIRED name is the common case: `slow = "ingest"` sat in a test
    /// fixture through a whole release for precisely this reason.
    ///
    /// Reported once at startup beside the model, not enforced. The
    /// file is the user's; goulash's job is to say it is being ignored,
    /// not to overrule it.
    pub fn warnings(&self) -> Vec<String> {
        fn known(out: &mut Vec<String>, key: &str, got: &str, allowed: &[&str]) {
            if !got.is_empty() && !allowed.contains(&got) {
                out.push(format!("{key}={got:?} unknown, using {}", allowed[0]));
            }
        }
        // `Wire::parse` is the authority on provider spellings; listing
        // them again here would be a second copy to forget to update.
        fn provider(out: &mut Vec<String>, key: &str, got: &str) {
            if !got.is_empty()
                && got != "auto"
                && got != "none"
                && crate::wire::Wire::parse(got).is_none()
            {
                out.push(format!("{key}={got:?} unknown, probing instead"));
            }
        }
        const THINK: &[&str] = &["off", "low", "medium", "high"];
        let mut w = Vec::new();
        let e = &self.engine;
        known(&mut w, "engine.thinking", &e.thinking, THINK);
        known(&mut w, "engine.slow", &e.slow, &["manual", "query", "waldorf"]);
        known(
            &mut w,
            "debug.cursor_save",
            &self.debug.cursor_save,
            &["decsc", "absolute"],
        );
        provider(&mut w, "engine.provider", &e.provider);
        if let Some(t) = e.slow_lane.thinking.as_deref() {
            known(&mut w, "engine.slow_lane.thinking", t, THINK);
        }
        if let Some(p) = e.slow_lane.provider.as_deref() {
            provider(&mut w, "engine.slow_lane.provider", p);
        }
        w
    }

    pub fn remove_key(section: &str, key: &str) -> Result<(), String> {
        let path = Self::dir().ok_or("no home dir")?.join("config.toml");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut doc: toml_edit::DocumentMut =
            text.parse().map_err(|e| format!("config parse: {e}"))?;
        let mut node = doc.as_table_mut() as &mut dyn toml_edit::TableLike;
        for seg in section.split('.') {
            match node.get_mut(seg).and_then(|i| i.as_table_like_mut()) {
                Some(t) => node = t,
                // Nothing there is nothing to remove, and creating the
                // table on the way to deleting from it would be absurd.
                None => return std::fs::write(&path, doc.to_string()).map_err(|e| e.to_string()),
            }
        }
        node.remove(key);
        std::fs::write(&path, doc.to_string()).map_err(|e| e.to_string())
    }
}

/// Walk to `a.b.c`, creating tables on the way.
///
/// A dotted section is a PATH, not a name. `doc["engine.slow_lane"]`
/// creates a top-level table whose key is the literal string
/// `engine.slow_lane`, which TOML writes back quoted — and serde then
/// looks for a field called that on `Config`, finds none, and falls back
/// to defaults. So every sub-table the settings menu saved
/// (`engine.slow_lane`, `engine.divulge`) was written to a file, read
/// back as nothing, and silently reverted on the next launch.
fn descend<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    section: &str,
) -> Result<&'a mut dyn toml_edit::TableLike, String> {
    let mut node = doc.as_table_mut() as &mut dyn toml_edit::TableLike;
    for seg in section.split('.') {
        if node.get(seg).is_none() {
            node.insert(seg, toml_edit::table());
        }
        node = node
            .get_mut(seg)
            .and_then(|i| i.as_table_like_mut())
            .ok_or_else(|| format!("{seg} is not a table"))?;
    }
    Ok(node)
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
    use super::{Config, EngineConfig, descend, edit_model};

    /// A dotted section is a path. Writing it as one KEY produces
    /// `["engine.slow_lane"]` — valid TOML naming a top-level table
    /// that `Config` has no field for, so serde ignores it and every
    /// setting saved there reverts on the next launch. Silently: the
    /// menu shows the new value until you restart.
    ///
    /// This shipped. A user's file had `["engine.divulge"]` and
    /// `["engine.slow_lane"]` in it, both inert.
    #[test]
    fn a_dotted_section_nests_and_survives_a_round_trip() {
        let mut doc: toml_edit::DocumentMut =
            "[engine]\nmodel = \"m\"\n".parse().unwrap();
        descend(&mut doc, "engine.slow_lane")
            .unwrap()
            .insert("thinking", toml_edit::value("medium"));
        descend(&mut doc, "engine.divulge")
            .unwrap()
            .insert("platform", toml_edit::value(false));
        let text = doc.to_string();
        assert!(
            !text.contains("\"engine."),
            "sections must nest, not be quoted names: {text}"
        );
        // The half that actually matters: read it back the way the
        // product does, and see the values.
        let cfg: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg.engine.slow_lane.thinking.as_deref(), Some("medium"));
        assert!(!cfg.engine.divulge.platform);
        assert_eq!(cfg.engine.model.as_deref(), Some("m"), "siblings intact");
    }

    /// A retired value is still valid TOML, so nothing else catches it.
    /// This is the check that would have caught `slow = "ingest"`.
    #[test]
    fn a_retired_or_misspelled_value_is_reported() {
        let cfg: Config = toml::from_str(
            "[engine]\nslow = \"ingest\"\nthinking = \"maximum\"\nprovider = \"olama\"\n",
        )
        .unwrap();
        let w = cfg.warnings().join(" | ");
        assert!(w.contains("engine.slow"), "{w}");
        assert!(w.contains("engine.thinking"), "{w}");
        assert!(w.contains("engine.provider"), "{w}");
        // It says what it will do instead, or the warning is just noise.
        assert!(w.contains("using manual") && w.contains("probing"), "{w}");
    }

    /// A false warning is worse than none: it trains you to ignore the
    /// row. Defaults, aliases and lane overrides all have to stay quiet.
    #[test]
    fn everything_legitimate_stays_silent() {
        assert!(Config::default().warnings().is_empty(), "defaults");
        for src in [
            "[engine]\nprovider = \"lmstudio\"\n",
            "[engine]\nprovider = \"openai-chat\"\n",
            "[engine]\nprovider = \"none\"\n",
            "[engine]\nprovider = \"auto\"\n",
            "[engine.slow_lane]\nthinking = \"high\"\nprovider = \"vllm\"\n",
            "[debug]\ncursor_save = \"absolute\"\n",
        ] {
            let cfg: Config = toml::from_str(src).unwrap();
            assert!(cfg.warnings().is_empty(), "{src} -> {:?}", cfg.warnings());
        }
    }

    /// "Follow the fast lane" is the ABSENCE of a key, so removal has to
    /// find the same nested table the write created.
    #[test]
    fn removing_a_nested_key_leaves_its_neighbours() {
        let mut doc: toml_edit::DocumentMut = "[engine]\nmodel = \"m\"\n".parse().unwrap();
        let t = descend(&mut doc, "engine.slow_lane").unwrap();
        t.insert("thinking", toml_edit::value("high"));
        t.insert("provider", toml_edit::value("ollama"));
        descend(&mut doc, "engine.slow_lane").unwrap().remove("thinking");
        let cfg: Config = toml::from_str(&doc.to_string()).unwrap();
        assert_eq!(cfg.engine.slow_lane.thinking, None, "gone");
        assert_eq!(
            cfg.engine.slow_lane.provider.as_deref(),
            Some("ollama"),
            "and only that one"
        );
    }

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
