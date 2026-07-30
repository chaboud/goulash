use crate::config::EngineConfig;
use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub mod prompt;
pub mod provider;

pub use prompt::{
    MemPos, PREAMBLE, PromptShape, build_prompt, directive, extract_memory_ops, hms, local_now,
    split_answer,
};
pub use provider::{
    Caps, GenRequest, GenStats, ModelInfo, Ollama, OpenAiCompat, Provider, Think,
};

/// Generation settings the product ships with. Deliberately constants
/// rather than config.toml knobs: the bench varies them through
/// [`GenRequest`] directly, and which of them earns a user-facing setting
/// is a question the measurement answers.
pub const DEFAULT_TEMPERATURE: f32 = 0.2;

/// No stop sequence.
///
/// goulash used to send `["\n\n"]` to enforce the one-line contract. It
/// does not enforce it — the directive and the band's render-time clamp
/// already do — and it destroys valid answers: a model that emits a blank
/// line before its reply, or between the prose and the `CMD:` line, gets
/// guillotined. Measured: 37 tokens generated and an EMPTY answer shown.
/// Removing it lifted the answer rate 81% -> 94%, and with reasoning
/// enabled it is categorically fatal (median 39 tokens generated vs 1279
/// without). Kept as a `GenRequest` field so the bench can still sweep it.
/// (bench/THINKING.md)
pub const DEFAULT_STOP: &[&str] = &[];
pub const DEFAULT_THINK: Think = Think::Off;
/// Resolve `[engine] thinking` against what the model can actually do.
///
/// `auto` must never send the field to a model that cannot reason:
/// ollama answers HTTP 400 `"<model>" does not support thinking` rather
/// than ignoring it, and 8 of 24 measured cells fail that way. A user who
/// writes `forced` gets it anyway and owns the error — discovery protects
/// the default path, it does not overrule an instruction.
pub fn resolve_think(setting: &str, model_can_think: Option<bool>) -> Think {
    match setting {
        "forced" => Think::On,
        "auto" => match model_can_think {
            Some(true) => Think::On,
            // Unknown is treated as cannot: suppressing costs a little
            // quality on a model that could have reasoned, while guessing
            // wrong costs a hard 400 and no answer at all.
            _ => Think::Off,
        },
        _ => Think::Off,
    }
}

/// The LLM engine: a worker thread so inference latency never touches the
/// PTY loop. Events come back over an mpsc channel; a self-pipe byte wakes
/// the session's poll(). (wiki: architecture/llm-engine.md)
pub enum Event {
    Ready {
        provider: String,
        model: String,
    },
    /// Streaming partial: accumulated answer text so far.
    Partial(String),
    /// Final answer: prose plus an optional candidate command, which the
    /// session vends into the suggestion list (pullable with Down).
    Answer {
        text: String,
        command: Option<String>,
        proactive: bool,
        remembers: Vec<String>,
        forgets: Vec<u64>,
    },
    Error(String),
    Models(Vec<String>),
    /// Raw model output, emitted when [engine] debug = true.
    Debug(String),
    /// A load/generation is starting on this model — the dangerous
    /// window the crash fuse marks (state.rs). `warm` distinguishes a
    /// model load (worth a "loading …" notice) from an ordinary ask.
    Busy {
        model: String,
        warm: bool,
    },
    /// The in-flight work returned (however it went).
    Idle,
}

pub enum Job {
    Ask {
        question: String,
        context: String,
        memories: String,
        proactive: bool,
    },
    SetModel(String),
    /// Forget any pinned model and re-run the probe chain (auto).
    Rebind,
    ListModels,
}

pub struct Engine {
    job_tx: mpsc::Sender<Job>,
    pub events: mpsc::Receiver<Event>,
    /// poll() this; a readable byte means events are waiting.
    pub wake: OwnedFd,
}

impl Engine {
    pub fn start(cfg: EngineConfig) -> std::io::Result<Engine> {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (ev_tx, ev_rx) = mpsc::channel::<Event>();
        let (rd, wr) = nix::unistd::pipe()?;
        std::thread::spawn(move || worker(cfg, job_rx, ev_tx, wr));
        Ok(Engine {
            job_tx,
            events: ev_rx,
            wake: rd,
        })
    }

    pub fn ask(&self, question: String, context: String, memories: String) {
        let _ = self.job_tx.send(Job::Ask {
            question,
            context,
            memories,
            proactive: false,
        });
    }

    /// Unprompted per-turn review; coalescing lets a user ask supersede it.
    pub fn ask_proactive(&self, context: String, memories: String) {
        let question = "Without being asked, briefly review the most recent \
                        command and its result — one short observation, \
                        tip, or wry aside is always welcome. Add a CMD: \
                        line ONLY when there is a genuinely useful command \
                        the user would plausibly run next: most \
                        observations need no command, and inventing \
                        busywork (logging, note-taking, echo) is worse \
                        than none. Only if you truly have nothing worth \
                        saying, reply exactly: PASS"
            .to_string();
        let _ = self.job_tx.send(Job::Ask {
            question,
            context,
            memories,
            proactive: true,
        });
    }

    pub fn set_model(&self, model: String) {
        let _ = self.job_tx.send(Job::SetModel(model));
    }

    pub fn rebind(&self) {
        let _ = self.job_tx.send(Job::Rebind);
    }

    pub fn list_models(&self) {
        let _ = self.job_tx.send(Job::ListModels);
    }
}

fn notify(wr: &OwnedFd) {
    let _ = nix::unistd::write(wr, b"e");
}

/// A bound engine: which provider, where, and on which model.
struct Bound {
    provider: Box<dyn Provider>,
    host: String,
    model: String,
    installed: Vec<String>,
    /// Learned once at bind, via a read-only capability query — never by
    /// asking the model to think and seeing if it 400s.
    can_think: Option<bool>,
}

fn worker(mut cfg: EngineConfig, jobs: mpsc::Receiver<Job>, ev: mpsc::Sender<Event>, wr: OwnedFd) {
    let path_set = crate::vendor::path_executable_set();
    // Machine facts, derived ONCE in this worker thread and reused for the
    // session. Deriving costs ~4ms — fine here, indefensible on the ask
    // path — and leaking a single &'static str is bounded, unlike doing it
    // per generation.
    let preamble: &'static str = Box::leak(
        format!("{}{}", crate::facts::block(&cfg.divulge), PREAMBLE).into_boxed_str(),
    );
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout(Duration::from_secs(120))
        .build();

    let mut state = probe(&agent, &cfg);
    if let Some(b) = &state {
        announce(&ev, &wr, b);
        if cfg.prewarm {
            warm_marked(&agent, &cfg, b, &ev, &wr);
        }
    }

    while let Ok(first) = jobs.recv() {
        // Late binding: the server may have started after goulash did, so
        // an unbound engine re-probes whenever work arrives.
        if state.is_none() {
            state = probe(&agent, &cfg);
            if let Some(b) = &state {
                announce(&ev, &wr, b);
                if cfg.prewarm {
                    warm_marked(&agent, &cfg, b, &ev, &wr);
                }
            }
        }
        // Drain the queue before working: control jobs apply immediately,
        // and only the NEWEST ask survives — answering stale questions
        // serially just burns GPU on answers nobody is waiting for.
        // Warms are deferred to AFTER the drain (at most one, for the
        // final model), so a slow model load never pins a queued
        // ListModels — the menu answers from cache instantly.
        let mut latest_ask = None;
        let mut pending_warm = false;
        let mut job = first;
        loop {
            match job {
                Job::Ask { .. } => latest_ask = Some(job),
                Job::SetModel(m) => match state.as_mut() {
                    Some(b) => {
                        b.model = m.clone();
                        b.can_think = b.provider.can_think(&agent, &b.host, &b.model);
                        cfg.model = Some(m);
                        announce(&ev, &wr, b);
                        pending_warm = true;
                    }
                    None => unreachable_engine(&ev, &wr, &cfg),
                },
                Job::Rebind => {
                    cfg.model = None;
                    state = probe(&agent, &cfg);
                    match &state {
                        Some(b) => {
                            announce(&ev, &wr, b);
                            pending_warm = true;
                        }
                        None => unreachable_engine(&ev, &wr, &cfg),
                    }
                }
                Job::ListModels => match &state {
                    Some(b) => {
                        let _ = ev.send(Event::Models(b.installed.clone()));
                        notify(&wr);
                    }
                    None => unreachable_engine(&ev, &wr, &cfg),
                },
            }
            match jobs.try_recv() {
                Ok(next) => job = next,
                Err(_) => break,
            }
        }
        if pending_warm
            && cfg.prewarm
            && let Some(b) = &state
        {
            warm_marked(&agent, &cfg, b, &ev, &wr);
        }
        if let Some(Job::Ask {
            question,
            context,
            memories,
            proactive,
        }) = latest_ask
        {
            let Some(b) = &state else {
                unreachable_engine(&ev, &wr, &cfg);
                continue;
            };
            let _ = ev.send(Event::Busy {
                model: b.model.clone(),
                warm: false,
            });
            notify(&wr);
            let result = generate(
                &agent, &cfg, b, preamble, &question, &context, &memories, &ev, &wr,
                proactive,
            );
            let _ = ev.send(Event::Idle);
            let _ = match result {
                Ok(ans) => {
                    if cfg.debug {
                        let _ = ev.send(Event::Debug(ans.clone()));
                    }
                    let (rest, remembers, forgets) = extract_memory_ops(&ans);
                    let (text, command) = split_answer(&rest, &path_set);
                    if text.is_empty()
                        && command.is_none()
                        && remembers.is_empty()
                        && forgets.is_empty()
                    {
                        if proactive {
                            Ok(()) // silent pass
                        } else {
                            let model = &b.model;
                            ev.send(Event::Error(format!(
                                "empty answer from {model} (thinking model? try \
                                 #/model or raise max_tokens)"
                            )))
                        }
                    } else {
                        ev.send(Event::Answer {
                            text,
                            command,
                            proactive,
                            remembers,
                            forgets,
                        })
                    }
                }
                Err(e) => {
                    if proactive {
                        Ok(()) // commentary failures stay silent
                    } else {
                        ev.send(Event::Error(e))
                    }
                }
            };
            notify(&wr);
        }
    }
    drop(wr);
}

fn announce(ev: &mpsc::Sender<Event>, wr: &OwnedFd, b: &Bound) {
    let _ = ev.send(Event::Ready {
        provider: b.provider.name().to_string(),
        model: b.model.clone(),
    });
    notify(wr);
}

fn unreachable_engine(ev: &mpsc::Sender<Event>, wr: &OwnedFd, cfg: &EngineConfig) {
    let _ = ev.send(Event::Error(format!("no engine reachable at {}", cfg.host)));
    notify(wr);
}

/// Ask the server to load the model so the first real ask doesn't pay the
/// cold start. Best-effort; blocks only the worker. Bracketed by
/// Busy/Idle: the load is the crash fuse's dangerous window.
fn warm_marked(
    agent: &ureq::Agent,
    cfg: &EngineConfig,
    b: &Bound,
    ev: &mpsc::Sender<Event>,
    wr: &OwnedFd,
) {
    let _ = ev.send(Event::Busy {
        model: b.model.clone(),
        warm: true,
    });
    notify(wr);
    b.provider.warm(agent, &b.host, &b.model, &cfg.keep_alive);
    let _ = ev.send(Event::Idle);
    notify(wr);
}

/// Which providers to try, in order. `auto` walks the local chain —
/// ollama (:11434), LM Studio (:1234), llama.cpp server (:8080) — and
/// binds whatever answers first. (wiki: product/distribution.md)
fn candidates(cfg: &EngineConfig) -> Vec<(Box<dyn Provider>, String)> {
    match cfg.provider.as_str() {
        "ollama" => vec![(Box::new(Ollama), cfg.host.clone())],
        "openai" | "lmstudio" => {
            vec![(Box::new(OpenAiCompat::default()), cfg.host.clone())]
        }
        _ => vec![
            (Box::new(Ollama), cfg.host.clone()),
            (
                Box::new(OpenAiCompat::default()),
                "http://127.0.0.1:1234".to_string(),
            ),
            (
                Box::new(OpenAiCompat::default()),
                "http://127.0.0.1:8080".to_string(),
            ),
        ],
    }
}

fn probe(agent: &ureq::Agent, cfg: &EngineConfig) -> Option<Bound> {
    for (provider, host) in candidates(cfg) {
        let Some(models) = provider.probe(agent, &host) else {
            continue;
        };
        let resident = if cfg.prefer_resident {
            provider.resident(agent, &host)
        } else {
            Vec::new()
        };
        let Some(model) = pick_model(&models, cfg.model.as_deref(), &cfg.favorites, &resident)
        else {
            continue;
        };
        let installed = models.into_iter().map(|m| m.name).collect();
        let can_think = provider.can_think(agent, &host, &model);
        return Some(Bound {
            provider,
            host,
            model,
            installed,
            can_think,
        });
    }
    None
}

/// Selection order: configured model, then the first favorite that is
/// installed (a favorite matches exactly or up to the ':tag'), then the
/// SMALLEST installed model — one-line status-bar answers want the
/// watcher-tier default; heavyweights are opt-in.
fn pick_model(
    models: &[ModelInfo],
    configured: Option<&str>,
    favorites: &[String],
    resident: &[String],
) -> Option<String> {
    if let Some(m) = configured {
        return Some(m.to_string());
    }
    // A model the engine already holds costs no load at all, and taking it
    // avoids evicting whatever the user is working with. Ranked above
    // favourites because a warm larger model very plausibly beats a cold
    // smaller one end to end. (bench/RESIDENCY.md)
    if let Some(hit) = resident
        .iter()
        .find(|r| models.iter().any(|m| &&m.name == r))
    {
        return Some(hit.clone());
    }
    for fav in favorites {
        if let Some(hit) = models
            .iter()
            .find(|m| m.name == *fav || m.name.split(':').next() == Some(fav.as_str()))
        {
            return Some(hit.name.clone());
        }
    }
    models
        .iter()
        .min_by_key(|m| m.size)
        .map(|m| m.name.clone())
}

#[allow(clippy::too_many_arguments)]
fn generate(
    agent: &ureq::Agent,
    cfg: &EngineConfig,
    b: &Bound,
    preamble: &'static str,
    question: &str,
    context: &str,
    memories: &str,
    ev: &mpsc::Sender<Event>,
    wr: &OwnedFd,
    proactive: bool,
) -> Result<String, String> {
    let think = resolve_think(&cfg.thinking, b.can_think);
    let req = GenRequest {
        model: b.model.clone(),
        prompt: build_prompt(
            &PromptShape {
                preamble: Some(preamble),
                ..PromptShape::default()
            },
            memories,
            context,
            question,
            &local_now(),
            proactive,
        ),
        stream: cfg.stream,
        temperature: DEFAULT_TEMPERATURE,
        max_tokens: cfg.response_tokens,
        num_ctx: cfg.num_ctx.unwrap_or(cfg.num_ctx_min),
        stop: DEFAULT_STOP.iter().map(|s| s.to_string()).collect(),
        think,
        reasoning_tokens: cfg.reasoning_tokens,
        keep_alive: cfg.keep_alive.clone(),
    };
    // Throttle partials so the bar fills in without repainting per token.
    let mut last_emit = Instant::now();
    let mut on_partial = |acc: &str| {
        if !proactive && last_emit.elapsed() >= Duration::from_millis(150) {
            let _ = ev.send(Event::Partial(acc.to_string()));
            notify(wr);
            last_emit = Instant::now();
        }
    };
    b.provider
        .generate(agent, &b.host, &req, &mut on_partial)
        .map(|(text, _stats)| text)
}

#[cfg(test)]
mod pick_tests {
    use super::{ModelInfo, pick_model};

    fn models(pairs: &[(&str, u64)]) -> Vec<ModelInfo> {
        pairs
            .iter()
            .map(|(n, s)| ModelInfo {
                name: n.to_string(),
                size: *s,
            })
            .collect()
    }

    fn no_favs() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn picks_smallest_model_by_default() {
        let m = models(&[
            ("gemma3:12b", 8_100_000_000),
            ("llama3.2:1b", 1_300_000_000),
            ("qwen2.5:7b", 4_700_000_000),
        ]);
        assert_eq!(
            pick_model(&m, None, &no_favs(), &[]).as_deref(),
            Some("llama3.2:1b")
        );
    }

    #[test]
    fn configured_model_wins() {
        let m = models(&[("llama3.2:1b", 1)]);
        assert_eq!(
            pick_model(&m, Some("gemma3:12b"), &["llama3.2:1b".to_string()], &[]).as_deref(),
            Some("gemma3:12b")
        );
    }

    #[test]
    fn first_installed_favorite_wins() {
        let m = models(&[("gemma3:12b", 8_100_000_000), ("qwen2.5:7b", 4_700_000_000)]);
        let favs = vec!["notinstalled:1b".to_string(), "qwen2.5".to_string()];
        assert_eq!(
            pick_model(&m, None, &favs, &[]).as_deref(),
            Some("qwen2.5:7b"),
            "favorite matches through the :tag"
        );
    }

    #[test]
    fn missing_sizes_still_pick_something() {
        let m = models(&[("mystery", u64::MAX)]);
        assert_eq!(pick_model(&m, None, &no_favs(), &[]).as_deref(), Some("mystery"));
    }

    /// A resident model outranks the smallest-installed default: taking
    /// it costs no load (p50 4281ms saved) and does not evict whatever the
    /// user is working with.
    #[test]
    fn resident_model_outranks_smallest() {
        let m = models(&[
            ("gemma4:12b", 7_600_000_000),
            ("qwen3.5:0.8b", 1_000_000_000),
        ]);
        assert_eq!(
            pick_model(&m, None, &no_favs(), &["gemma4:12b".into()]).as_deref(),
            Some("gemma4:12b")
        );
        // ...but an explicit pin still wins, and a resident model that is
        // not installed here is ignored.
        assert_eq!(
            pick_model(&m, Some("qwen3.5:0.8b"), &no_favs(), &["gemma4:12b".into()]).as_deref(),
            Some("qwen3.5:0.8b")
        );
        assert_eq!(
            pick_model(&m, None, &no_favs(), &["not-installed".into()]).as_deref(),
            Some("qwen3.5:0.8b")
        );
    }

    /// OpenAI-compatible servers report no size, so every entry ties at 0
    /// and auto-pick degrades to first-listed rather than failing.
    #[test]
    fn zero_sizes_degrade_to_first_listed() {
        let m = models(&[("qwen/qwen3-8b", 0), ("qwen/qwen3-1.7b", 0)]);
        assert_eq!(
            pick_model(&m, None, &no_favs(), &[]).as_deref(),
            Some("qwen/qwen3-8b")
        );
    }
}

#[cfg(test)]
mod think_tests {
    use super::{Think, resolve_think};

    /// `auto` must never send `think` to a model that cannot reason:
    /// ollama answers HTTP 400 rather than ignoring it, so a wrong guess
    /// costs the whole answer.
    #[test]
    fn auto_respects_capability_and_unknown_means_no() {
        assert_eq!(resolve_think("auto", Some(true)), Think::On);
        assert_eq!(resolve_think("auto", Some(false)), Think::Off);
        assert_eq!(resolve_think("auto", None), Think::Off);
    }

    /// A user who writes `forced` gets it and owns the error — discovery
    /// protects the default path, it does not overrule an instruction.
    #[test]
    fn forced_overrides_capability() {
        assert_eq!(resolve_think("forced", Some(false)), Think::On);
        assert_eq!(resolve_think("forced", None), Think::On);
    }

    #[test]
    fn off_and_garbage_both_suppress() {
        assert_eq!(resolve_think("off", Some(true)), Think::Off);
        assert_eq!(resolve_think("nonsense", Some(true)), Think::Off);
    }
}
