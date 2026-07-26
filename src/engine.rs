use crate::config::EngineConfig;
use crate::models::{Caps, Overrides, Source, Think, caps_for};
use std::io::BufRead;
use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::time::{Duration, Instant};

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
    /// A `CMD:` line arrived mid-stream — vend it now, before the prose
    /// finishes. The matching Answer carries `command: None`.
    Command(String),
    /// Final answer: prose plus an optional candidate command, which the
    /// session vends into the suggestion list (pullable with Down).
    Answer {
        text: String,
        command: Option<String>,
        proactive: bool,
        remembers: Vec<String>,
        forgets: Vec<u64>,
        /// Paths the model asked to pin, and whether it asked to drop
        /// everything (context.rs line protocol).
        pins: Vec<String>,
        pinclear: bool,
    },
    /// A pin's compressed form, or its card (None when the model gave
    /// nothing usable — the pin keeps its deterministic version).
    Digest {
        id: u64,
        text: Option<String>,
        card: bool,
    },
    /// A researched finding for `turn`. Fast relays it; the session
    /// amends the turn it came from, never the top of the stack.
    Finding {
        turn: u64,
        text: String,
        command: Option<String>,
        /// The full reasoning, retained rather than shown — the receipt
        /// behind the one line the user actually reads.
        reasoning: String,
    },
    /// Research started (`Some(turn)`) or went idle (`None`).
    Researching(Option<u64>),
    Error(String),
    Models(Vec<String>),
    /// Resolved capabilities for the newly bound model. The session
    /// keeps these so the UI can tell the truth about what `thinking`
    /// will do here instead of offering a dial that goes nowhere.
    Caps(Caps),
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
        /// The `#@` working context block (context.rs), riding in the
        /// stable prefix next to memories.
        pinned: String,
        /// Pin *cards* — a few lines per pin, emitted next to the
        /// question instead of in the stable prefix (context.rs). The
        /// prefix copy is cache-warm but far away; sliding-window models
        /// attend to what is near, so the payload gets restated where it
        /// will actually be read.
        cards: String,
        proactive: bool,
        /// A `#@ <natural language>` request: the model is being asked
        /// to resolve a path and answer in PIN verbs, not to advise.
        pin_ask: bool,
    },
    /// Background ingest: compress a pinned file to fit its budget, or
    /// write its card (context.rs). Always yields to interactive asks —
    /// a cook that makes the user wait is worse than no cook.
    Digest {
        id: u64,
        label: String,
        source: String,
        target: usize,
        /// A card is the same machinery aimed at a different position:
        /// a handful of lines for next to the question, rather than a
        /// compression for the stable prefix.
        card: bool,
    },
    /// The slow lane: a considered answer for a turn that fast already
    /// answered. Never speaks — the finding goes back to fast, which
    /// relays it. Supersedes by default (terminals are serial).
    Research {
        /// The turn this will amend. Findings land at their origin, so
        /// this is carried the whole way round.
        turn: u64,
        question: String,
        context: String,
        memories: String,
        pinned: String,
    },
    /// Abandon every queued digest (`#@/cancel`).
    CancelDigests,
    /// Abandon research in flight and pending (`#?/cancel`).
    CancelResearch,
    SetModel(String),
    /// Live tuning: key/value applied to the worker's own config copy.
    SetOption(String, String),
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
    pub fn start(cfg: EngineConfig, over: Overrides) -> std::io::Result<Engine> {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (ev_tx, ev_rx) = mpsc::channel::<Event>();
        let (rd, wr) = nix::unistd::pipe()?;
        std::thread::spawn(move || worker(cfg, over, job_rx, ev_tx, wr));
        Ok(Engine {
            job_tx,
            events: ev_rx,
            wake: rd,
        })
    }

    pub fn ask(
        &self,
        question: String,
        context: String,
        memories: String,
        pinned: String,
        cards: String,
    ) {
        let _ = self.job_tx.send(Job::Ask {
            question,
            context,
            memories,
            pinned,
            cards,
            proactive: false,
            pin_ask: false,
        });
    }

    /// `#@ <natural language>`: resolve what the user means against a
    /// listing of candidates and answer in PIN verbs. Read-only, and
    /// goulash performs the read — the model never gets a shell.
    pub fn ask_pin(&self, request: String, candidates: String, pinned: String) {
        let question = format!(
            "The user wants to change the pinned working context. Their \
             request: {request}\nFiles in the current directory: \
             {candidates}\nAnswer with 'PIN: <path>' for each file or \
             directory to pin (relative paths are fine), or 'PINCLEAR' to \
             unpin everything. Add at most ONE short line of prose. If \
             nothing plausibly matches, say so and pin nothing."
        );
        let _ = self.job_tx.send(Job::Ask {
            question,
            context: String::new(),
            memories: String::new(),
            pinned,
            cards: String::new(),
            proactive: false,
            pin_ask: true,
        });
    }

    /// Unprompted per-turn review; coalescing lets a user ask supersede it.
    pub fn ask_proactive(
        &self,
        context: String,
        memories: String,
        pinned: String,
        cards: String,
    ) {
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
            pinned,
            cards,
            proactive: true,
            pin_ask: false,
        });
    }

    pub fn set_model(&self, model: String) {
        let _ = self.job_tx.send(Job::SetModel(model));
    }

    pub fn set_option(&self, key: &str, value: &str) {
        let _ = self
            .job_tx
            .send(Job::SetOption(key.to_string(), value.to_string()));
    }

    pub fn rebind(&self) {
        let _ = self.job_tx.send(Job::Rebind);
    }

    pub fn list_models(&self) {
        let _ = self.job_tx.send(Job::ListModels);
    }

    pub fn digest(&self, id: u64, label: String, source: String, target: usize, card: bool) {
        let _ = self.job_tx.send(Job::Digest {
            id,
            label,
            source,
            target,
            card,
        });
    }

    pub fn cancel_digests(&self) {
        let _ = self.job_tx.send(Job::CancelDigests);
    }

    pub fn research(
        &self,
        turn: u64,
        question: String,
        context: String,
        memories: String,
        pinned: String,
    ) {
        let _ = self.job_tx.send(Job::Research {
            turn,
            question,
            context,
            memories,
            pinned,
        });
    }

    pub fn cancel_research(&self) {
        let _ = self.job_tx.send(Job::CancelResearch);
    }
}

fn notify(wr: &OwnedFd) {
    let _ = nix::unistd::write(wr, b"e");
}

fn worker(
    mut cfg: EngineConfig,
    over: Overrides,
    jobs: mpsc::Receiver<Job>,
    ev: mpsc::Sender<Event>,
    wr: OwnedFd,
) {
    let path_set = crate::vendor::path_executable_set();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout(Duration::from_secs(120))
        .build();

    // Probe chain, v1: ollama (explicit or auto-detected). "none" was
    // filtered by the caller. (wiki: llm-engine.md probe chain)
    let mut state = probe_ollama(&agent, &cfg);
    // What the bound model can actually do. Re-resolved on every bind,
    // never guessed at ask time.
    let mut caps = caps_for("", None, &over);
    if let Some((model, _)) = &state {
        caps = announce(&agent, &cfg, &over, &ev, &wr, model);
        if cfg.prewarm {
            warm_marked(&agent, &cfg, model, &ev, &wr);
        }
    }

    // Background ingest, strictly second-class: queued here, drained one
    // per loop pass, and only after any interactive work in the same
    // pass. `digest_total` is the batch size the meter counts against.
    let mut digest_queue: std::collections::VecDeque<Job> = std::collections::VecDeque::new();
    let mut digest_total = 0usize;
    // At most one research job is live. A newer `#?` replaces it; the
    // displaced one is dropped, or kept for backfill if configured.
    let mut pending_research: Option<Job> = None;
    let mut backfill: std::collections::VecDeque<Job> = std::collections::VecDeque::new();

    loop {
        // Blocking recv only when there is no background work waiting —
        // otherwise poll, so a quiet channel means "get on with the
        // cooking" rather than "sleep".
        let idle = digest_queue.is_empty() && pending_research.is_none() && backfill.is_empty();
        let first = if idle {
            match jobs.recv() {
                Ok(j) => Some(j),
                Err(_) => break,
            }
        } else {
            match jobs.try_recv() {
                Ok(j) => Some(j),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        };
        let Some(first) = first else {
            // Research outranks ingest: the user asked for it.
            if run_research(
                &agent,
                &cfg,
                &caps,
                &state,
                &mut pending_research,
                &mut backfill,
                &ev,
                &wr,
            ) {
                continue;
            }
            run_one_digest(
                &agent,
                &cfg,
                &caps,
                &state,
                &mut digest_queue,
                &mut digest_total,
                &ev,
                &wr,
            );
            continue;
        };
        // Late binding: ollama may have started after goulash did, so an
        // unbound engine re-probes whenever work arrives.
        if state.is_none() {
            state = probe_ollama(&agent, &cfg);
            if let Some((model, _)) = &state {
                caps = announce(&agent, &cfg, &over, &ev, &wr, model);
                if cfg.prewarm {
                    warm_marked(&agent, &cfg, model, &ev, &wr);
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
                    Some((model, _)) => {
                        *model = m.clone();
                        cfg.model = Some(m);
                        caps = announce(&agent, &cfg, &over, &ev, &wr, model);
                        pending_warm = true;
                    }
                    None => unreachable_engine(&ev, &wr, &cfg),
                },
                Job::SetOption(k, v) => match k.as_str() {
                    "thinking" => cfg.thinking = v,
                    "slow" => cfg.slow = v,
                    "command_first" => cfg.command_first = v == "true" || v == "on",
                    "max_tokens" => {
                        if let Ok(n) = v.parse() {
                            cfg.max_tokens = n;
                        }
                    }
                    _ => {}
                },
                Job::Rebind => {
                    cfg.model = None;
                    state = probe_ollama(&agent, &cfg);
                    match &state {
                        Some((model, _)) => {
                            caps = announce(&agent, &cfg, &over, &ev, &wr, model);
                            pending_warm = true;
                        }
                        None => unreachable_engine(&ev, &wr, &cfg),
                    }
                }
                Job::Digest { .. } => {
                    digest_queue.push_back(job);
                    digest_total = digest_total.max(digest_queue.len());
                }
                // Supersede, not queue: terminals are serial, and three
                // questions in a row is usually one mind changing rather
                // than three answers wanted. `backfill_abandoned` gives
                // the loser a second life instead of a queue slot.
                Job::Research { .. } => {
                    if let Some(old) = pending_research.replace(job)
                        && cfg.backfill_abandoned
                    {
                        backfill.push_back(old);
                    }
                }
                Job::CancelResearch => {
                    pending_research = None;
                    backfill.clear();
                    let _ = ev.send(Event::Researching(None));
                    notify(&wr);
                }
                Job::CancelDigests => {
                    // Report every abandoned pin so the session can put
                    // their meters away.
                    for j in digest_queue.drain(..) {
                        if let Job::Digest { id, card, .. } = j {
                            let _ = ev.send(Event::Digest {
                                id,
                                text: None,
                                card,
                            });
                        }
                    }
                    digest_total = 0;
                    notify(&wr);
                }
                Job::ListModels => match &state {
                    Some((_, installed)) => {
                        let _ = ev.send(Event::Models(installed.clone()));
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
            && let Some((model, _)) = &state
        {
            warm_marked(&agent, &cfg, model, &ev, &wr);
        }
        if let Some(Job::Ask {
            question,
            context,
            memories,
            pinned,
            cards,
            proactive,
            pin_ask,
        }) = latest_ask
        {
            let Some((model, _)) = &state else {
                unreachable_engine(&ev, &wr, &cfg);
                continue;
            };
            let _ = ev.send(Event::Busy {
                model: model.clone(),
                warm: false,
            });
            notify(&wr);
            let result = generate(
                &agent, &cfg, &caps, model, &question, &context, &memories, &pinned, &cards,
                &ev, &wr, proactive, pin_ask,
            );
            let _ = ev.send(Event::Idle);
            let _ = match result {
                Ok((ans, early)) => {
                    if cfg.debug {
                        let _ = ev.send(Event::Debug(ans.clone()));
                    }
                    let (rest, remembers, forgets) = extract_memory_ops(&ans);
                    let (rest, pins, pinclear) = crate::context::extract_pin_ops(&rest);
                    let (text, mut command) = split_answer(&rest, &path_set);
                    // Already vended mid-stream: don't hand it over twice.
                    if command.is_some() && command == early {
                        command = None;
                    }
                    if text.is_empty()
                        && command.is_none()
                        && early.is_none()
                        && remembers.is_empty()
                        && forgets.is_empty()
                        && pins.is_empty()
                        && !pinclear
                    {
                        if proactive {
                            Ok(()) // silent pass
                        } else {
                            ev.send(Event::Error(empty_answer_reason(&cfg, &caps)))
                        }
                    } else {
                        ev.send(Event::Answer {
                            text,
                            command,
                            proactive,
                            remembers,
                            forgets,
                            pins,
                            pinclear,
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
        // Interactive work for this pass is done. Research first (the
        // user asked for it), then ingest. One at a time either way, so
        // a new ask never waits behind more than a single job.
        if run_research(
            &agent,
            &cfg,
            &caps,
            &state,
            &mut pending_research,
            &mut backfill,
            &ev,
            &wr,
        ) {
            continue;
        }
        run_one_digest(
            &agent,
            &cfg,
            &caps,
            &state,
            &mut digest_queue,
            &mut digest_total,
            &ev,
            &wr,
        );
    }
    drop(wr);
}

/// Run the live research job, if there is one. Returns whether it did
/// anything, so the caller can give research priority over ingest.
///
/// The whole point of the slow lane in one function: a bigger budget, no
/// latency pressure, and an answer that goes to **fast** rather than to
/// the user. Bounded by wall clock, because a lane with no deadline is
/// a lane that can hang the backlog behind it.
#[allow(clippy::too_many_arguments)]
fn run_research(
    agent: &ureq::Agent,
    cfg: &EngineConfig,
    caps: &Caps,
    state: &Option<(String, Vec<String>)>,
    pending: &mut Option<Job>,
    backfill: &mut std::collections::VecDeque<Job>,
    ev: &mpsc::Sender<Event>,
    wr: &OwnedFd,
) -> bool {
    let job = pending.take().or_else(|| backfill.pop_front());
    let Some(Job::Research {
        turn,
        question,
        context,
        memories,
        pinned,
    }) = job
    else {
        return false;
    };
    let Some((model, _)) = state else {
        let _ = ev.send(Event::Researching(None));
        notify(wr);
        return true;
    };
    let _ = ev.send(Event::Researching(Some(turn)));
    let _ = ev.send(Event::Busy {
        model: model.clone(),
        warm: false,
    });
    notify(wr);
    let out = research_once(
        agent, cfg, caps, model, &question, &context, &memories, &pinned,
    );
    let _ = ev.send(Event::Idle);
    let _ = ev.send(Event::Researching(None));
    if let Ok((text, command, reasoning)) = out
        && !(text.is_empty() && command.is_none())
    {
        let _ = ev.send(Event::Finding {
            turn,
            text,
            command,
            reasoning,
        });
    }
    notify(wr);
    true
}

/// The considered answer. Same model as fast by default — "slow" is a
/// role, not necessarily a second set of weights — with a budget that
/// buys thinking room and a contract that keeps the output relayable:
/// a command, one line, and the reasoning kept separately.
#[allow(clippy::too_many_arguments)]
fn research_once(
    agent: &ureq::Agent,
    cfg: &EngineConfig,
    caps: &Caps,
    model: &str,
    question: &str,
    context: &str,
    memories: &str,
    pinned: &str,
) -> Result<(String, Option<String>, String), String> {
    let budget = (cfg.max_tokens * 4).clamp(512, 4096);
    let prompt = format!(
        "{PREAMBLE}{memories}{pinned}Session log (oldest first):\n{context}\n\
         Current local time: {}\nQuestion: {question}\n\
         Take your time and get this RIGHT rather than fast \u{2014} another \
         model already gave a quick answer, and yours only earns its keep \
         by being better.\nAnswer in exactly this shape:\n\
         CMD: <the command, if one applies>\n\
         <ONE short line a terminal status bar can hold>\n\
         REASON: <why, including what you ruled out \u{2014} this is kept \
         for follow-up questions, not shown>\n\
         If you have nothing better than an obvious answer, reply exactly: \
         PASS",
        local_now()
    );
    let mut body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "options": {
            "temperature": 0.3,
            "num_predict": (budget + caps.allowance(&cfg.thinking, cfg.thinking_tokens)) as i64,
            "num_ctx": cfg.num_ctx as i64,
        },
    });
    if let Some(think) = caps.think_field(&cfg.thinking) {
        body["think"] = think;
    }
    if !cfg.keep_alive.is_empty() {
        body["keep_alive"] = serde_json::json!(cfg.keep_alive);
    }
    let resp = agent
        .post(&format!("{}/api/generate", cfg.host))
        .timeout(Duration::from_secs(cfg.slow_max_secs))
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let raw = v["response"].as_str().unwrap_or("").trim();
    if raw.is_empty() || raw.trim_end_matches(['.', '!']) == "PASS" {
        return Err("nothing better".to_string());
    }
    Ok(split_finding(raw))
}

/// Pull a finding apart into the line the user sees, the command, and
/// the reasoning that is kept but not shown.
fn split_finding(raw: &str) -> (String, Option<String>, String) {
    let mut command = None;
    let mut line = String::new();
    let mut reason = String::new();
    let mut in_reason = false;
    for l in raw.lines() {
        let t = l.trim();
        if let Some(r) = t.strip_prefix("REASON:") {
            in_reason = true;
            reason.push_str(r.trim());
            reason.push('\n');
        } else if in_reason {
            reason.push_str(l);
            reason.push('\n');
        } else if let Some(c) = t.strip_prefix("CMD:") {
            let c = c.trim();
            if !c.is_empty() {
                command = Some(c.to_string());
            }
        } else if !t.is_empty() && line.is_empty() {
            line = t.to_string();
        }
    }
    (line, command, reason.trim().to_string())
}

/// Compress one queued pin, if there is one and a model to do it with.
/// Failure is not fatal anywhere: the pin keeps the deterministic
/// outline it has had since the moment it was made.
#[allow(clippy::too_many_arguments)]
fn run_one_digest(
    agent: &ureq::Agent,
    cfg: &EngineConfig,
    caps: &Caps,
    state: &Option<(String, Vec<String>)>,
    queue: &mut std::collections::VecDeque<Job>,
    total: &mut usize,
    ev: &mpsc::Sender<Event>,
    wr: &OwnedFd,
) {
    let Some(Job::Digest {
        id,
        label,
        source,
        target,
        card,
    }) = queue.pop_front()
    else {
        return;
    };
    let Some((model, _)) = state else {
        let _ = ev.send(Event::Digest {
            id,
            text: None,
            card,
        });
        notify(wr);
        return;
    };
    let _ = ev.send(Event::Busy {
        model: model.clone(),
        warm: false,
    });
    notify(wr);
    let text = digest_once(agent, cfg, caps, model, &label, &source, target, card).ok();
    let _ = ev.send(Event::Idle);
    let _ = ev.send(Event::Digest { id, text, card });
    if queue.is_empty() {
        *total = 0;
    }
    notify(wr);
}

/// The compression call. No session log, no memories, no working
/// context: a digest is a pure function of one document, which also
/// means it never disturbs the prefix cache the asks depend on.
#[allow(clippy::too_many_arguments)]
fn digest_once(
    agent: &ureq::Agent,
    cfg: &EngineConfig,
    caps: &Caps,
    model: &str,
    label: &str,
    source: &str,
    target: usize,
    card: bool,
) -> Result<String, String> {
    // Characters in, tokens out: ~4 chars per token, with headroom so a
    // slightly long answer is still usable rather than cut mid-table.
    let budget = (target / 3).clamp(128, 2048);
    // Two positions, two jobs. The digest replaces a document in the
    // stable prefix and wants completeness within its budget. The card
    // sits next to the question and wants the two or three things you
    // would tell someone in a corridor — completeness there is a waste
    // of the only tokens a sliding-window model reliably attends to.
    let prompt = if card {
        format!(
            "Write a {target}-character crib for this reference, to sit \
             next to a question as a reminder. ONE line saying what the \
             tool is, then at most three of its most useful exact \
             invocations, then any single rule that would make a command \
             wrong. Nothing else \u{2014} no preamble, no prose, no \
             closing remark.\n\n=== {label} ===\n{source}\n=== end \
             ===\nCrib:"
        )
    } else {
        format!(
            "Compress this reference document to under {target} characters, \
             for another model to use when suggesting shell commands.\n\
             KEEP: exact command names, flags, arguments, paths, env vars, \
             file formats, and any rule or constraint that would make a \
             command wrong.\nDROP: prose, rationale, history, examples that \
             repeat a flag already listed.\nWrite terse lines, not \
             paragraphs. No preamble, no closing remarks \u{2014} output only \
             the compressed reference.\n\n=== {label} ===\n{source}\n=== end \
             ===\nCompressed reference:"
        )
    };
    let mut body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "options": {
            "temperature": 0.1,
            "num_predict": budget as i64,
            "num_ctx": cfg.num_ctx as i64,
        },
    });
    // Reasoning here would spend the budget arguing with itself about a
    // document it is only meant to shorten.
    if let Some(think) = caps.think_field("off") {
        body["think"] = think;
    }
    if !cfg.keep_alive.is_empty() {
        body["keep_alive"] = serde_json::json!(cfg.keep_alive);
    }
    let resp = agent
        .post(&format!("{}/api/generate", cfg.host))
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let out = v["response"].as_str().unwrap_or("").trim().to_string();
    if out.is_empty() {
        return Err("empty digest".to_string());
    }
    Ok(out)
}

/// Bind: tell the session what model it has, and what that model can
/// do. Capability resolution is metadata only (`/api/show` does not load
/// weights), so this stays off the slow path that once pinned the worker.
fn announce(
    agent: &ureq::Agent,
    cfg: &EngineConfig,
    over: &Overrides,
    ev: &mpsc::Sender<Event>,
    wr: &OwnedFd,
    model: &str,
) -> Caps {
    let _ = ev.send(Event::Ready {
        provider: "ollama".to_string(),
        model: model.to_string(),
    });
    let caps = caps_for(model, show_thinks(agent, cfg, model), over);
    let _ = ev.send(Event::Caps(caps.clone()));
    notify(wr);
    caps
}

/// Does the provider itself say this model reasons? ollama's `/api/show`
/// carries a `capabilities` list on recent servers. None means it
/// wouldn't say (old server, or the call failed) — the table decides.
fn show_thinks(agent: &ureq::Agent, cfg: &EngineConfig, model: &str) -> Option<bool> {
    let resp = agent
        .post(&format!("{}/api/show", cfg.host))
        .timeout(Duration::from_secs(2))
        .send_string(&serde_json::json!({"model": model}).to_string())
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&resp.into_string().ok()?).ok()?;
    let caps = v["capabilities"].as_array()?;
    Some(caps.iter().any(|c| c.as_str() == Some("thinking")))
}

/// The model returned nothing. With capabilities resolved we can name
/// the actual cause instead of listing suspects.
fn empty_answer_reason(cfg: &EngineConfig, caps: &Caps) -> String {
    let asked = cfg.thinking != "off" && !cfg.thinking.is_empty();
    let model = &caps.model;
    match caps.think {
        // It reasons and was asked to (or does anyway): the budget is
        // the suspect, and it is the one thing the user can raise.
        Think::Levels | Think::Bool if asked || caps.always_reasons => format!(
            "empty answer from {model} \u{2014} it spent the budget \
             reasoning; raise thinking_tokens (#/settings) or \
             '#/thinking off'"
        ),
        // Asked to think, cannot. We should not have sent the field at
        // all, so this is a plain short-budget or bad-prompt answer.
        Think::None if asked => format!(
            "empty answer from {model} \u{2014} it does not reason \
             (thinking has no effect here); try another model (#/model) \
             or a bigger response budget (#/settings)"
        ),
        _ if caps.source == Source::Guess => format!(
            "empty answer from {model} \u{2014} unknown model, so goulash \
             is guessing at its reasoning support; see [models] in \
             config.toml, or try another model (#/model)"
        ),
        _ => format!(
            "empty answer from {model} \u{2014} try another model \
             (#/model) or a bigger response budget (#/settings)"
        ),
    }
}

fn unreachable_engine(ev: &mpsc::Sender<Event>, wr: &OwnedFd, cfg: &EngineConfig) {
    let _ = ev.send(Event::Error(format!("no engine reachable at {}", cfg.host)));
    notify(wr);
}

/// Ask the server to load the model (empty generate) so the first real
/// ask doesn't pay the cold start. Best-effort; blocks only the worker.
/// Bracketed by Busy/Idle: the load is the crash fuse's dangerous window.
fn warm_marked(
    agent: &ureq::Agent,
    cfg: &EngineConfig,
    model: &str,
    ev: &mpsc::Sender<Event>,
    wr: &OwnedFd,
) {
    let _ = ev.send(Event::Busy {
        model: model.to_string(),
        warm: true,
    });
    notify(wr);
    let mut body = serde_json::json!({"model": model});
    if !cfg.keep_alive.is_empty() {
        body["keep_alive"] = serde_json::json!(cfg.keep_alive);
    }
    let _ = agent
        .post(&format!("{}/api/generate", cfg.host))
        .send_string(&body.to_string());
    let _ = ev.send(Event::Idle);
    notify(wr);
}

fn probe_ollama(agent: &ureq::Agent, cfg: &EngineConfig) -> Option<(String, Vec<String>)> {
    let resp = agent
        .get(&format!("{}/api/tags", cfg.host))
        .timeout(Duration::from_secs(1))
        .call()
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&resp.into_string().ok()?).ok()?;
    let installed: Vec<String> = v["models"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let model = pick_model(&v, cfg.model.as_deref(), &cfg.favorites)?;
    Some((model, installed))
}

/// Selection order: configured model, then the first favorite that is
/// installed (a favorite matches exactly or up to the ':tag'), then the
/// SMALLEST installed model — one-line status-bar answers want the
/// watcher-tier default; heavyweights are opt-in.
fn pick_model(
    tags: &serde_json::Value,
    configured: Option<&str>,
    favorites: &[String],
) -> Option<String> {
    if let Some(m) = configured {
        return Some(m.to_string());
    }
    let models = tags["models"].as_array()?;
    let names: Vec<&str> = models.iter().filter_map(|m| m["name"].as_str()).collect();
    for fav in favorites {
        if let Some(hit) = names
            .iter()
            .find(|n| **n == fav.as_str() || n.split(':').next() == Some(fav.as_str()))
        {
            return Some(hit.to_string());
        }
    }
    models
        .iter()
        .filter_map(|m| {
            let name = m["name"].as_str()?;
            let size = m["size"].as_u64().unwrap_or(u64::MAX);
            Some((size, name))
        })
        .min()
        .map(|(_, name)| name.to_string())
}

/// Byte-stable preamble: identical across asks so the provider's KV
/// prefix cache (ollama caches against the previous request) re-uses the
/// preamble + unchanged session-log prefix; only the appended tail and
/// the question get re-evaluated.
const PREAMBLE: &str = "You are goulash, an assistant living in the user's \
terminal status bar. Answer tersely in ONE short line of plain text, no \
markdown. Each command carries the local time it ran; treat old output as \
stale. The log also contains the running conversation: '#' lines are \
earlier user questions, 'goulash:' lines are your earlier replies, and \
'CMD:' lines are commands you suggested — follow-up questions refer back \
to them.\n\
When a file would let you answer better — a command reference, a runbook, \
a config — you may suggest that the user pin it, with a normal command \
line: 'CMD: #@/path <file>'. Goulash reads it into your working context. \
Suggest it; never assume it happened.\n\n";

#[allow(clippy::too_many_arguments)]
fn generate(
    agent: &ureq::Agent,
    cfg: &EngineConfig,
    caps: &Caps,
    model: &str,
    question: &str,
    context: &str,
    memories: &str,
    pinned: &str,
    cards: &str,
    ev: &mpsc::Sender<Event>,
    wr: &OwnedFd,
    proactive: bool,
    pin_ask: bool,
) -> Result<(String, Option<String>), String> {
    // Volatile parts (current time, question) go AFTER the stable prefix.
    // The command directive is repeated at point-of-use: small models
    // lose instructions that only appear at the top of a long prompt.
    // Prompt shape (stable-prefix first): preamble, pinned memories,
    // session log, then the volatile time/question/directive suffix.
    // Two modes of ingress, two contracts: a `#` ask is usually fishing
    // for a runnable command; unprompted commentary earns its CMD line.
    // Command-first puts the payload where truncation cannot reach it —
    // and lets the suggestion vend before the prose finishes.
    let directive = match (cfg.command_first, proactive) {
        _ if pin_ask => {
            "Answer ONLY with PIN: / PINCLEAR lines plus at most one short \
             prose line. No CMD: line."
        }
        (true, false) => {
            "Reply with the command FIRST, on its own line, formatted \
             exactly as: CMD: <command> — required whenever any shell \
             command could accomplish, fix, or demonstrate what was \
             asked. Then ONE short prose line explaining it."
        }
        (true, true) => {
            "If a genuinely useful next command exists, put it FIRST on \
             its own line, formatted exactly as: CMD: <command>. Then \
             ONE short prose line."
        }
        (false, false) => {
            "Reply with ONE short prose line. If any shell command could \
             accomplish, fix, or demonstrate what was asked, you MUST add \
             a second line formatted exactly as: CMD: <command>"
        }
        (false, true) => {
            "Reply with ONE short prose line. Add a second line formatted \
             exactly as: CMD: <command> ONLY if a genuinely useful next \
             command exists."
        }
    };
    // Stable-prefix order, most stable first: preamble, memories,
    // working context, session log. A pin changes far less often than
    // the log, so it belongs above it in the prefix.
    // ...and the cards ride at the very end, with the question. That
    // position is re-prefilled on every ask, which is exactly why they
    // are kept to a few hundred characters — and it is the only place a
    // pin lands inside a sliding-window model's attention.
    let prompt = format!(
        "{PREAMBLE}{memories}{pinned}Session log (oldest first):\n{context}\n\
         Current local time: {}\n{cards}Question: {question}\n{directive}\nAnswer:",
        local_now()
    );
    // Reasoning models (qwen3+, deepseek-r1) otherwise spend the entire
    // token budget in a separate `thinking` field, returning an empty
    // `response` — a blank bar in the field. The model's own dialect
    // decides the shape of the request and the size of the top-up
    // (models.rs); a model that cannot reason is not sent the field at
    // all, because some providers reject it rather than ignore it.
    let mut body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": cfg.stream,
        "options": {
            "temperature": 0.2,
            "num_predict": (cfg.max_tokens
                + caps.allowance(&cfg.thinking, cfg.thinking_tokens)) as i64,
            "num_ctx": cfg.num_ctx as i64,
            "stop": ["\n\n"],
        },
    });
    if let Some(think) = caps.think_field(&cfg.thinking) {
        body["think"] = think;
    }
    if !cfg.keep_alive.is_empty() {
        body["keep_alive"] = serde_json::json!(cfg.keep_alive);
    }
    let resp = agent
        .post(&format!("{}/api/generate", cfg.host))
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;

    if !cfg.stream {
        let text = resp.into_string().map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        return v["response"]
            .as_str()
            .map(|s| (s.trim().to_string(), None))
            .ok_or_else(|| "malformed engine response".to_string());
    }

    // Streaming: one JSON object per line; forward throttled partials so
    // the bar fills in as tokens arrive.
    let reader = std::io::BufReader::new(resp.into_reader());
    let mut acc = String::new();
    let mut last_emit = Instant::now();
    let mut early: Option<String> = None;
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if let Some(tok) = v["response"].as_str() {
            acc.push_str(tok);
        }
        // The payoff of command-first: the CMD line completes long
        // before the prose does, so the suggestion can be pullable
        // while the explanation is still arriving. Proactive turns wait
        // for the end — a PASS must not leave a command behind.
        if !proactive
            && early.is_none()
            && let Some(cmd) = complete_cmd_line(&acc)
        {
            early = Some(cmd.clone());
            let _ = ev.send(Event::Command(cmd));
            notify(wr);
        }
        if v["done"].as_bool() == Some(true) {
            break;
        }
        if !proactive && !acc.is_empty() && last_emit.elapsed() >= Duration::from_millis(150) {
            let _ = ev.send(Event::Partial(acc.clone()));
            notify(wr);
            last_emit = Instant::now();
        }
    }
    Ok((acc.trim().to_string(), early))
}

/// A `CMD:` line that has been fully received (its newline has arrived),
/// so it is safe to vend before the rest of the answer streams in.
fn complete_cmd_line(acc: &str) -> Option<String> {
    for line in acc.lines() {
        if (!acc.ends_with(line) || acc.ends_with('\n'))
            && let Some(c) = line.trim().strip_prefix("CMD:")
        {
            let c = c.trim();
            if !c.is_empty() {
                return Some(c.to_string());
            }
        }
    }
    None
}

fn tm_now() -> libc::tm {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as libc::time_t;
    // SAFETY: localtime_r fills the tm struct; zeroed is a valid init.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    tm
}

/// "2026-07-21 01:10:53" — volatile, kept out of the stable prefix.
pub fn local_now() -> String {
    let tm = tm_now();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// "01:10:53" — stamped onto session-log block headers (stable once
/// written, so it doesn't break the prefix cache).
pub fn hms() -> String {
    let tm = tm_now();
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// Pull REMEMBER:/FORGET: tool lines out of an answer; returns the
/// remaining text plus the requested memory operations.
fn extract_memory_ops(raw: &str) -> (String, Vec<String>, Vec<u64>) {
    let mut rest = Vec::new();
    let mut remembers = Vec::new();
    let mut forgets = Vec::new();
    for line in raw.lines() {
        let l = line.trim();
        if let Some(note) = l.strip_prefix("REMEMBER:") {
            if !note.trim().is_empty() {
                remembers.push(note.trim().to_string());
            }
        } else if let Some(id) = l.strip_prefix("FORGET:") {
            if let Ok(n) = id.trim().trim_matches(['[', ']']).parse::<u64>() {
                forgets.push(n);
            }
        } else {
            rest.push(line);
        }
    }
    (rest.join("\n"), remembers, forgets)
}

/// Split a raw answer into (prose, candidate command): the first
/// non-empty non-CMD line is the prose (one-line contract enforced
/// here), and the first `CMD: ...` line is the command. Small models
/// often reply with a bare command and no tag, so a fallback treats a
/// short line whose first word is a PATH executable as the command.
fn split_answer(
    raw: &str,
    path_set: &std::collections::HashSet<String>,
) -> (String, Option<String>) {
    let mut text = String::new();
    let mut command = None;
    for line in raw.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(c) = l.strip_prefix("CMD:") {
            if command.is_none() && !c.trim().is_empty() {
                command = Some(c.trim().to_string());
            }
        } else if text.is_empty() {
            text = l.to_string();
        }
    }
    if command.is_none() {
        for line in raw.lines().take(4) {
            let l = line.trim().trim_matches('`');
            let words: Vec<&str> = l.split_whitespace().collect();
            if !l.is_empty()
                && !l.starts_with('#')
                && (1..=8).contains(&words.len())
                && !l.ends_with(['.', '!', '?'])
                && path_set.contains(words[0])
            {
                command = Some(l.to_string());
                break;
            }
        }
    }
    (text, command)
}

#[cfg(test)]
mod answer_tests {
    use super::split_answer;
    use std::collections::HashSet;

    fn paths(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn text_only() {
        assert_eq!(
            split_answer("It is Tuesday.\n", &paths(&[])),
            ("It is Tuesday.".into(), None)
        );
    }

    #[test]
    fn text_and_command() {
        let (t, c) = split_answer(
            "Disk is mostly node_modules.\nCMD: du -sh * | sort -h\n",
            &paths(&[]),
        );
        assert_eq!(t, "Disk is mostly node_modules.");
        assert_eq!(c.as_deref(), Some("du -sh * | sort -h"));
    }

    #[test]
    fn command_first_and_rambling() {
        let (t, c) = split_answer(
            "\nCMD: git pull\nRun this to update.\nExtra ramble.",
            &paths(&[]),
        );
        assert_eq!(t, "Run this to update.");
        assert_eq!(c.as_deref(), Some("git pull"));
    }

    #[test]
    fn bare_command_fallback() {
        // The field case: model answers `ls -lhR` with no CMD: tag.
        let (t, c) = split_answer("ls -lhR", &paths(&["ls", "du"]));
        assert_eq!(t, "ls -lhR");
        assert_eq!(c.as_deref(), Some("ls -lhR"));
        // Prose sentences never trip the fallback.
        let (_, c2) = split_answer("Use the ls command to list.", &paths(&["ls"]));
        assert_eq!(c2, None);
    }
}

#[cfg(test)]
mod pick_tests {
    use super::pick_model;
    use serde_json::json;

    fn no_favs() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn picks_smallest_model_by_default() {
        let tags = json!({"models": [
            {"name": "gemma3:12b", "size": 8_100_000_000u64},
            {"name": "llama3.2:1b", "size": 1_300_000_000u64},
            {"name": "qwen2.5:7b", "size": 4_700_000_000u64},
        ]});
        assert_eq!(
            pick_model(&tags, None, &no_favs()).as_deref(),
            Some("llama3.2:1b")
        );
    }

    #[test]
    fn configured_model_wins() {
        let tags = json!({"models": [{"name": "llama3.2:1b", "size": 1u64}]});
        assert_eq!(
            pick_model(&tags, Some("gemma3:12b"), &["llama3.2:1b".to_string()]).as_deref(),
            Some("gemma3:12b")
        );
    }

    #[test]
    fn first_installed_favorite_wins() {
        let tags = json!({"models": [
            {"name": "gemma3:12b", "size": 8_100_000_000u64},
            {"name": "qwen2.5:7b", "size": 4_700_000_000u64},
        ]});
        let favs = vec!["notinstalled:1b".to_string(), "qwen2.5".to_string()];
        assert_eq!(
            pick_model(&tags, None, &favs).as_deref(),
            Some("qwen2.5:7b"),
            "favorite matches through the :tag"
        );
    }

    #[test]
    fn missing_sizes_still_pick_something() {
        let tags = json!({"models": [{"name": "mystery"}]});
        assert_eq!(
            pick_model(&tags, None, &no_favs()).as_deref(),
            Some("mystery")
        );
    }
}

#[cfg(test)]
mod stream_tests {
    use super::{complete_cmd_line, empty_answer_reason};
    use crate::config::EngineConfig;
    use crate::models::{Overrides, caps_for};

    #[test]
    fn cmd_vends_only_once_the_line_is_whole() {
        // Still arriving: the command may yet gain more characters.
        assert_eq!(complete_cmd_line("CMD: ls -l"), None);
        // Newline landed: safe to hand over.
        assert_eq!(
            complete_cmd_line("CMD: ls -lh\n"),
            Some("ls -lh".to_string())
        );
        // Prose already following it is equally conclusive.
        assert_eq!(
            complete_cmd_line("CMD: du -sh *\nThat sums each entry."),
            Some("du -sh *".to_string())
        );
        assert_eq!(complete_cmd_line("no command here\n"), None);
        assert_eq!(complete_cmd_line("CMD:   \n"), None);
    }

    #[test]
    fn empty_answers_are_diagnosed_from_capabilities() {
        let over = Overrides::new();
        let mut cfg = EngineConfig {
            thinking: "high".into(),
            ..Default::default()
        };

        // Reasoner asked to reason: the budget is the culprit, and the
        // advice must be the lever that actually helps.
        let oss = caps_for("gpt-oss:20b", None, &over);
        let msg = empty_answer_reason(&cfg, &oss);
        assert!(msg.contains("spent the budget reasoning"), "{msg}");

        // Non-reasoner: say so, rather than blaming a dial that did
        // nothing here.
        let gemma = caps_for("gemma3:4b", None, &over);
        let msg = empty_answer_reason(&cfg, &gemma);
        assert!(msg.contains("does not reason"), "{msg}");

        // Thinking off, plain model: no reasoning talk at all.
        cfg.thinking = "off".into();
        let msg = empty_answer_reason(&cfg, &gemma);
        assert!(!msg.contains("reason"), "{msg}");

        // A reasoner that always reasons is still the budget's fault
        // even with the dial off.
        let msg = empty_answer_reason(&cfg, &oss);
        assert!(msg.contains("spent the budget reasoning"), "{msg}");

        // Unknown model: admit the guess and point at the escape hatch.
        let unknown = caps_for("mystery:8b", None, &over);
        let msg = empty_answer_reason(&cfg, &unknown);
        assert!(msg.contains("[models]"), "{msg}");
    }
}
