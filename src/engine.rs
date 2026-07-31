use crate::config::{EngineConfig, LaneConfig};
use crate::models::{Caps, Overrides, Source, Think, caps_for};
use crate::wire::{Backend, Client, Gen, Wire};
use std::io::BufRead;
use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The LLM engine: a worker thread so inference latency never touches the
/// PTY loop. Events come back over an mpsc channel; a self-pipe byte wakes
/// the session's poll(). (wiki: architecture/llm-engine.md)
/// Which piece of work a `Busy`/`Idle` pair brackets.
///
/// The crash fuse wants every generation and does not care which; the
/// lane indicator cares a great deal, because fast and slow run at the
/// same time. A single "something is generating" flag would light the
/// fast lane while the slow one researched, which is precisely the
/// moment the indicator exists to distinguish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    /// A `#` ask on the fast lane.
    Ask,
    /// The slow lane researching a turn.
    Research,
    /// Compressing or carding a `#@` pin.
    Digest,
    /// Loading a model so the first ask does not pay for it.
    Warm,
}

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
    /// How big the prompt we just sent actually was. Nothing else
    /// measures this: the session knows the size of its own pieces, but
    /// only the worker knows what the assembled prompt came to — and
    /// "did the head of my prompt get truncated away?" is otherwise
    /// unanswerable, which makes a whole class of bug unfalsifiable.
    Prompt {
        chars: usize,
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
    /// Plain feedback that is not an error — binding the research
    /// lane, for one, which must not masquerade as `Ready` (the session
    /// tracks the FAST model from that and would name the wrong one).
    Notice(String),
    /// How the lanes are bound right now, for `#/status`.
    Lanes(String),
    /// Runaway diagnostics, counted where the work actually happens.
    ///
    /// The session dispatches from a dozen scattered call sites and can
    /// only report what it *believes* it sent; the worker sees every job
    /// exactly once. Queue depths are invisible to the session
    /// altogether, and both have run away before — an uncapped backfill
    /// is also a worker that never blocks. Emitted only on change.
    Meter {
        asks: u64,
        research: u64,
        digests: u64,
        queued: usize,
        backfill: usize,
    },
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
        kind: Work,
    },
    /// The in-flight work returned (however it went). Carries its kind
    /// so a listener clears the same lane it set.
    Idle {
        kind: Work,
    },
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
    SetModel {
        slow: bool,
        name: String,
    },
    /// Live tuning: key/value applied to the worker's own config copy.
    SetOption(String, String),
    /// Forget any pinned model and re-run the probe chain (auto).
    Rebind,
    ListModels {
        slow: bool,
    },
    /// How are the lanes bound right now (for `#/status`)?
    DescribeLanes,
}

/// One lane's binding: where it talks, what it bound there, and what
/// that model can do. Fast and slow are the same shape — "slow" is a
/// role, not a different kind of thing.
struct Lane {
    cl: Client,
    /// (bound model, everything installed on that server)
    state: Option<(String, Vec<String>)>,
    caps: Caps,
    cfg: LaneConfig,
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
    pub fn ask_proactive(&self, context: String, memories: String, pinned: String, cards: String) {
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

    pub fn set_model(&self, name: String) {
        let _ = self.job_tx.send(Job::SetModel { slow: false, name });
    }

    /// `#?/model`: bind the research lane, reusing the `#?` scoping the
    /// cancel verbs already established.
    pub fn set_slow_model(&self, name: String) {
        let _ = self.job_tx.send(Job::SetModel { slow: true, name });
    }

    pub fn list_slow_models(&self) {
        let _ = self.job_tx.send(Job::ListModels { slow: true });
    }

    pub fn describe_lanes(&self) {
        let _ = self.job_tx.send(Job::DescribeLanes);
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
        let _ = self.job_tx.send(Job::ListModels { slow: false });
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

/// One agent shape, used for every backend.
fn new_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout(Duration::from_secs(120))
        .build()
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
    let agent = new_agent();

    // Probe chain: an explicitly named provider, or each known local
    // server in turn. "none" was filtered by the caller.
    // (wiki: llm-engine.md probe chain)
    let cl = resolve_backend(agent.clone(), &cfg.fast_lane());
    let mut state = probe_models(&cl, &cfg.fast_lane());
    // The slow lane, when it is genuinely elsewhere. `None` means the
    // two roles share one binding — which is the point of resolving it
    // at all: two lanes on one model must not mean two model loads, two
    // KV caches, or two entries in the same server's queue.
    let mut slow: Option<Lane> = if cfg.lanes_split() {
        let lane_cfg = cfg.slow_lane();
        let scl = resolve_backend(agent, &lane_cfg);
        let sstate = probe_models(&scl, &lane_cfg);
        let scaps = match &sstate {
            Some((m, _)) => caps_for(m, show_thinks(&scl, m), &over),
            None => caps_for("", None, &over),
        };
        Some(Lane {
            cl: scl,
            state: sstate,
            caps: scaps,
            cfg: lane_cfg,
        })
    } else {
        None
    };
    // What the bound model can actually do. Re-resolved on every bind,
    // never guessed at ask time.
    let mut caps = caps_for("", None, &over);
    if let Some((model, _)) = &state {
        caps = announce(&cl, &over, &ev, &wr, model);
        if cfg.prewarm {
            warm_marked(&cl, &cfg, model, &ev, &wr);
        }
    }

    // Background ingest, strictly second-class: queued here, drained one
    // per loop pass, and only after any interactive work in the same
    // pass. `digest_total` is the batch size the meter counts against.
    let mut digest_queue: std::collections::VecDeque<Job> = std::collections::VecDeque::new();
    let mut digest_total = 0usize;
    let mut last_meter = (u64::MAX, u64::MAX, u64::MAX, usize::MAX, usize::MAX);
    let (mut n_asks, mut n_research, mut n_digests) = (0u64, 0u64, 0u64);
    // At most one research job is live. A newer `#?` replaces it; the
    // displaced one is dropped, or kept for backfill if configured.
    let mut pending_research: Option<Job> = None;
    let mut backfill: std::collections::VecDeque<Job> = std::collections::VecDeque::new();

    loop {
        // Blocking recv only when there is no background work waiting —
        // otherwise poll, so a quiet channel means "get on with the
        // cooking" rather than "sleep".
        let meter = (
            n_asks,
            n_research,
            n_digests,
            digest_queue.len(),
            backfill.len(),
        );
        if meter != last_meter {
            last_meter = meter;
            let _ = ev.send(Event::Meter {
                asks: meter.0,
                research: meter.1,
                digests: meter.2,
                queued: meter.3,
                backfill: meter.4,
            });
            notify(&wr);
        }
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
            // Research runs on the SLOW lane when there is one — a
            // different model, a different server, or a different
            // machine — and otherwise on the shared binding.
            let (rcl, rcaps, rstate) = match &slow {
                Some(l) => (&l.cl, &l.caps, &l.state),
                None => (&cl, &caps, &state),
            };
            if run_research(
                rcl,
                &cfg,
                rcaps,
                rstate,
                &mut pending_research,
                &mut backfill,
                &ev,
                &wr,
            ) {
                continue;
            }
            run_one_digest(
                &cl,
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
            state = probe_models(&cl, &cfg.fast_lane());
            if let Some((model, _)) = &state {
                caps = announce(&cl, &over, &ev, &wr, model);
                if cfg.prewarm {
                    warm_marked(&cl, &cfg, model, &ev, &wr);
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
                Job::Ask { .. } => {
                    n_asks += 1;
                    latest_ask = Some(job);
                }
                // Naming a slow model is what SPLITS the lanes: until
                // then the research role rides the fast binding, and
                // pointing it at a second model is the whole reason to
                // separate them.
                Job::SetModel { slow: true, name } => {
                    let lane = slow.get_or_insert_with(|| {
                        let lane_cfg = cfg.slow_lane();
                        let scl = resolve_backend(new_agent(), &lane_cfg);
                        let sstate = probe_models(&scl, &lane_cfg);
                        Lane {
                            cl: scl,
                            state: sstate,
                            caps: caps_for("", None, &over),
                            cfg: lane_cfg,
                        }
                    });
                    match lane.state.as_mut() {
                        Some((model, _)) => {
                            *model = name.clone();
                            lane.cfg.model = Some(name.clone());
                            lane.caps = caps_for(&name, show_thinks(&lane.cl, &name), &over);
                            let _ = ev.send(Event::Notice(format!(
                                "research lane \u{2192} {name} ({})",
                                lane.cl.be.label()
                            )));
                            notify(&wr);
                        }
                        None => unreachable_engine(&ev, &wr, &cfg),
                    }
                }
                Job::SetModel { slow: false, name } => match state.as_mut() {
                    Some((model, _)) => {
                        *model = name.clone();
                        cfg.model = Some(name);
                        caps = announce(&cl, &over, &ev, &wr, model);
                        pending_warm = true;
                    }
                    None => unreachable_engine(&ev, &wr, &cfg),
                },
                Job::SetOption(k, v) => match k.as_str() {
                    "thinking" => cfg.thinking = v,
                    "slow_max_tokens" => {
                        cfg.slow_lane.max_tokens =
                            (v != "same as fast").then(|| v.parse().ok()).flatten();
                    }
                    "slow_thinking" => {
                        cfg.slow_lane.thinking =
                            (v != "same as fast").then(|| v.to_string());
                    }
                    "slow" => cfg.slow = v,
                    "command_first" => cfg.command_first = v == "true" || v == "on",
                    // Machine facts. Live because the reason to turn them
                    // off is to see whether they are earning their tokens,
                    // and that comparison is worthless if it needs a
                    // restart in the middle.
                    "platform" => cfg.divulge.platform = v == "true" || v == "on",
                    // Which server a lane talks to. Rebinding is the
                    // caller's job: changing the provider invalidates
                    // the bound model, and picking the new one is a
                    // probe, not a field assignment.
                    "provider" => cfg.provider = v,
                    "slow_provider" => {
                        cfg.slow_lane.provider =
                            (v != "same as fast").then(|| v.to_string());
                    }
                    "divulge_tools" => cfg.divulge.tools = v == "true" || v == "on",
                    "divulge_path" => cfg.divulge.full_path = v == "true" || v == "on",
                    "max_tokens" => {
                        if let Ok(n) = v.parse() {
                            cfg.max_tokens = n;
                        }
                    }
                    _ => {}
                },
                Job::Rebind => {
                    cfg.model = None;
                    state = probe_models(&cl, &cfg.fast_lane());
                    match &state {
                        Some((model, _)) => {
                            caps = announce(&cl, &over, &ev, &wr, model);
                            pending_warm = true;
                        }
                        None => unreachable_engine(&ev, &wr, &cfg),
                    }
                }
                Job::Digest { .. } => {
                    n_digests += 1;
                    digest_queue.push_back(job);
                    digest_total = digest_total.max(digest_queue.len());
                }
                // Supersede, not queue: terminals are serial, and three
                // questions in a row is usually one mind changing rather
                // than three answers wanted. `backfill_abandoned` gives
                // the loser a second life instead of a queue slot.
                Job::Research { .. } => {
                    n_research += 1;
                    if let Some(old) = pending_research.replace(job)
                        && cfg.backfill_abandoned
                    {
                        push_backfill(&mut backfill, old);
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
                Job::DescribeLanes => {
                    let _ = ev.send(Event::Lanes(describe_lanes(&cl, &state, &caps, &slow)));
                    notify(&wr);
                }
                Job::ListModels { slow: want_slow } => {
                    // A split lane may be a different SERVER, so its
                    // menu has to come from that server's inventory —
                    // the fast one's would be a lie there.
                    let listing = match (want_slow, &slow) {
                        (true, Some(l)) => &l.state,
                        _ => &state,
                    };
                    match listing {
                        Some((_, installed)) => {
                            let _ = ev.send(Event::Models(installed.clone()));
                            notify(&wr);
                        }
                        None => unreachable_engine(&ev, &wr, &cfg),
                    }
                }
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
            warm_marked(&cl, &cfg, model, &ev, &wr);
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
                kind: Work::Ask,
            });
            notify(&wr);
            let result = generate(
                &cl, &cfg, &caps, model, &question, &context, &memories, &pinned, &cards, &ev, &wr,
                proactive, pin_ask,
            );
            let _ = ev.send(Event::Idle { kind: Work::Ask });
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
        let (rcl, rcaps, rstate) = match &slow {
            Some(l) => (&l.cl, &l.caps, &l.state),
            None => (&cl, &caps, &state),
        };
        if run_research(
            rcl,
            &cfg,
            rcaps,
            rstate,
            &mut pending_research,
            &mut backfill,
            &ev,
            &wr,
        ) {
            continue;
        }
        run_one_digest(
            &cl,
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
/// Backfill capacity. Each entry carries a whole prompt — question,
/// memories, pins, and up to `context_max_chars` of session log — so an
/// uncapped queue is megabytes, and `backfill.is_empty()` is one of the
/// three conditions for the worker to block rather than poll. An
/// unbounded backfill therefore means a worker that never idles.
const BACKFILL_CAP: usize = 8;

/// Queue an abandoned research job, dropping the OLDEST when full.
///
/// Oldest-first is the right eviction: backfill exists to give a
/// superseded question a second life, and the question the user moved
/// on from longest ago is the one least worth reviving. Growth here was
/// unconditional while draining was not — the same shape as the
/// `idle_ticks` overflow, and reachable whenever someone asks faster
/// than research completes, which is exactly what supersede is FOR.
fn push_backfill(q: &mut std::collections::VecDeque<Job>, job: Job) {
    while q.len() >= BACKFILL_CAP {
        q.pop_front();
    }
    q.push_back(job);
}

#[allow(clippy::too_many_arguments)]
fn run_research(
    cl: &Client,
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
        kind: Work::Research,
    });
    notify(wr);
    let out = research_once(
        cl, cfg, caps, model, &question, &context, &memories, &pinned,
    );
    let _ = ev.send(Event::Idle { kind: Work::Research });
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
    cl: &Client,
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
    let body = cl.be.wire.body(&Gen {
        model,
        prompt: &prompt,
        stream: false,
        temperature: 0.3,
        max_tokens: budget.max(slow_max_tokens(cfg)),
        num_ctx: ctx_for(cl, cfg, model),
        stop: &[],
        // The SLOW lane's own dial, which follows fast unless told
        // otherwise. The lanes exist to differ here: fast has to be
        // immediate, slow is the one that can afford to think.
        think: caps.think_field(slow_thinking(cfg)),
        effort: caps.effort_field(slow_thinking(cfg)),
        keep_alive: &cfg.keep_alive,
        num_keep: cfg.num_keep,
        seed: (cfg.seed >= 0).then_some(cfg.seed),
    });
    let resp = cl
        .post(&cl.gen_url())
        .timeout(Duration::from_secs(cfg.slow_max_secs))
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let raw = cl.be.wire.text(&v).unwrap_or_default();
    let raw = raw.trim();
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
        } else if let Some(c) = t.strip_prefix("CMD:").map(clean_cmd).as_deref() {
            let c = c.trim();
            if !c.is_empty() {
                command = Some(c.to_string());
            }
        } else if let Some(sa) = t.strip_prefix("SAY:") {
            if line.is_empty() && !sa.trim().is_empty() {
                line = sa.trim().to_string();
            }
        } else if !t.is_empty() && line.is_empty() && !is_tagged(t) {
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
    cl: &Client,
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
        kind: Work::Digest,
    });
    notify(wr);
    let text = digest_once(cl, cfg, caps, model, &label, &source, target, card).ok();
    let _ = ev.send(Event::Idle { kind: Work::Digest });
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
    cl: &Client,
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
    let body = cl.be.wire.body(&Gen {
        model,
        prompt: &prompt,
        stream: false,
        temperature: 0.1,
        max_tokens: budget,
        num_ctx: ctx_for(cl, cfg, model),
        stop: &[],
        // Reasoning here would spend the budget arguing with itself
        // about a document it is only meant to shorten.
        think: caps.think_field("off"),
        effort: caps.effort_field("off"),
        keep_alive: &cfg.keep_alive,
        num_keep: cfg.num_keep,
        seed: (cfg.seed >= 0).then_some(cfg.seed),
    });
    let resp = cl
        .post(&cl.gen_url())
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let out = cl.be.wire.text(&v).unwrap_or_default().trim().to_string();
    if out.is_empty() {
        return Err("empty digest".to_string());
    }
    Ok(out)
}

/// Bind: tell the session what model it has, and what that model can
/// do. Capability resolution is metadata only (`/api/show` does not load
/// weights), so this stays off the slow path that once pinned the worker.
fn announce(
    cl: &Client,
    over: &Overrides,
    ev: &mpsc::Sender<Event>,
    wr: &OwnedFd,
    model: &str,
) -> Caps {
    let _ = ev.send(Event::Ready {
        provider: cl.be.label().to_string(),
        model: model.to_string(),
    });
    let caps = caps_for(model, show_thinks(cl, model), over);
    let _ = ev.send(Event::Caps(caps.clone()));
    notify(wr);
    caps
}

/// Does the provider itself say this model reasons? ollama's `/api/show`
/// carries a `capabilities` list on recent servers. None means it
/// wouldn't say (old server, or the call failed) — the table decides.
pub fn show_thinks(cl: &Client, model: &str) -> Option<bool> {
    // Only ollama will say. An OpenAI-compatible listing is names and
    // nothing else, so None here is the honest answer and the family
    // table in models.rs stays authoritative — which is exactly the
    // precedence it was already built for (Source::Table).
    if cl.be.wire != Wire::Ollama {
        return None;
    }
    let resp = cl
        .post(&format!("{}/api/show", cl.be.host))
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
             reasoning; raise max_tokens (#/settings) or \
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
    cl: &Client,
    cfg: &EngineConfig,
    model: &str,
    ev: &mpsc::Sender<Event>,
    wr: &OwnedFd,
) {
    let _ = ev.send(Event::Busy {
        model: model.to_string(),
        warm: true,
        kind: Work::Warm,
    });
    notify(wr);
    // ollama loads a model on an empty generate; an OpenAI-compatible
    // server has no such call, so the warm is the smallest real request
    // that will do it — one token, thrown away.
    let body = match cl.be.wire {
        Wire::Ollama => {
            let mut b = serde_json::json!({"model": model});
            if !cfg.keep_alive.is_empty() {
                b["keep_alive"] = serde_json::json!(cfg.keep_alive);
            }
            b
        }
        Wire::OpenAi | Wire::OpenAiChat => cl.be.wire.body(&Gen {
            model,
            prompt: "",
            stream: false,
            temperature: 0.0,
            max_tokens: 1,
            num_ctx: ctx_for(cl, cfg, model),
            stop: &[],
            think: None,
            effort: None,
            keep_alive: "",
            // A warm-up request carries no prompt worth protecting and
            // wants no reproducibility; both would be noise here.
            num_keep: 0,
            seed: None,
        }),
    };
    let _ = cl.post(&cl.gen_url()).send_string(&body.to_string());
    let _ = ev.send(Event::Idle { kind: Work::Warm });
    notify(wr);
}

/// Which server are we talking to, and where. Resolved ONCE, because a
/// host alone cannot say which dialect answers on it — and because a
/// probe that ran per-call would put a network round trip on the ask
/// path. An explicit provider is honoured without probing (so a user
/// who names it gets a clear "unreachable" rather than a silent
/// fallback to the other one); `auto` tries ollama first, then the
/// OpenAI-compatible port, since ollama is the zero-config default and
/// LM Studio has to be running deliberately.
fn resolve_backend(agent: ureq::Agent, lane: &LaneConfig) -> Client {
    let key = if lane.api_key_env.is_empty() {
        String::new()
    } else {
        std::env::var(&lane.api_key_env).unwrap_or_default()
    };
    let mk = |wire: Wire| {
        let host = match wire {
            Wire::Ollama => lane.host.clone(),
            Wire::OpenAi | Wire::OpenAiChat => lane.openai_host.clone(),
        };
        Backend {
            trusted: crate::wire::resolve_trust(&lane.trusted, &host),
            host,
            wire,
            key: key.clone(),
        }
    };
    if let Some(wire) = Wire::parse(&lane.provider) {
        return Client {
            agent,
            be: mk(wire),
        };
    }
    let mut cl = Client {
        agent,
        be: mk(Wire::Ollama),
    };
    if reachable(&cl) {
        return cl;
    }
    cl.be = mk(Wire::OpenAi);
    cl
}

/// What `#/status` prints about the lanes. Trust is named per backend
/// because a split lane can be somewhere else entirely, and "which of
/// these two may see my pinned files" is exactly the question a user
/// with a laptop model and a hosted one needs answered.
fn describe_lanes(
    cl: &Client,
    state: &Option<(String, Vec<String>)>,
    caps: &Caps,
    slow: &Option<Lane>,
) -> String {
    let name = |st: &Option<(String, Vec<String>)>| {
        st.as_ref()
            .map(|(m, _)| m.clone())
            .unwrap_or_else(|| "none".to_string())
    };
    let out = match slow {
        Some(l) => format!(
            "fast {}@{} \u{2502} slow {}@{}",
            name(state),
            cl.be.label(),
            name(&l.state),
            l.cl.be.label()
        ),
        None => format!("{}@{} \u{b7} {}", name(state), cl.be.label(), caps.note()),
    };
    // Trust is called out only when it is NOT the safe case. A line that
    // says "trusted" beside every lane trains people to stop reading it;
    // a marker that appears exactly when something off-box can see the
    // pinned files is the one worth noticing.
    let untrusted: Vec<&str> = match slow {
        Some(l) => [
            (!cl.be.trusted).then_some("fast"),
            (!l.cl.be.trusted).then_some("slow"),
        ]
        .into_iter()
        .flatten()
        .collect(),
        None => (!cl.be.trusted)
            .then_some("this lane")
            .into_iter()
            .collect(),
    };
    // The warning goes FIRST. This line shares a bar row with the rest
    // of `#/status`, and whatever falls off the end is whatever the
    // user does not get to read — which must never be the part that
    // says something off-box can see their pinned files.
    if !untrusted.is_empty() {
        return format!("untrusted: {} \u{b7} {out}", untrusted.join("+"));
    }
    out
}

fn reachable(cl: &Client) -> bool {
    cl.get(&cl.models_url())
        .timeout(Duration::from_secs(1))
        .call()
        .is_ok()
}

fn probe_models(cl: &Client, lane: &LaneConfig) -> Option<(String, Vec<String>)> {
    let resp = cl
        .get(&cl.models_url())
        .timeout(Duration::from_secs(1))
        .call()
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&resp.into_string().ok()?).ok()?;
    let installed = cl.be.wire.models(&v);
    let model = pick_model(
        cl.be.wire,
        &v,
        &installed,
        lane.model.as_deref(),
        &lane.favorites,
    )?;
    Some((model, installed))
}

/// Selection order: configured model, then the first favorite that is
/// installed (a favorite matches exactly or up to the ':tag'), then the
/// SMALLEST installed model — one-line status-bar answers want the
/// watcher-tier default; heavyweights are opt-in.
fn pick_model(
    wire: Wire,
    listing: &serde_json::Value,
    names: &[String],
    configured: Option<&str>,
    favorites: &[String],
) -> Option<String> {
    if let Some(m) = configured {
        return Some(m.to_string());
    }
    for fav in favorites {
        if let Some(hit) = names
            .iter()
            .find(|n| n.as_str() == fav.as_str() || n.split(':').next() == Some(fav.as_str()))
        {
            return Some(hit.clone());
        }
    }
    // Smallest-installed needs a size, and only ollama reports one. An
    // OpenAI-compatible listing is names and nothing else, so the
    // fallback there is simply the first — the server's own order, which
    // for LM Studio is the model you last loaded.
    if wire == Wire::OpenAi {
        return names.first().cloned();
    }
    listing["models"]
        .as_array()?
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
pub const PREAMBLE: &str = "You are goulash, an assistant living in the user's \
terminal status bar. Answer tersely in ONE short line of plain text, no \
markdown. Each command carries the local time it ran; treat old output as \
stale. The log also contains the running conversation: '#' lines are \
earlier user questions, 'goulash:' lines are your earlier replies, and \
'CMD:' lines are commands you suggested — follow-up questions refer back \
to them.\n\
Answer in TAGGED LINES. Every line begins with a tag, and a line holds \
its tag and nothing else \u{2014} no explanation, no dash, no comment \
after the value:\n\
SAY: <one short line of plain text>\n\
CMD: <a shell command, complete and runnable on its own>\n\
Never put prose on a CMD line: the user runs that line verbatim.\n\
When a file would let you answer better — a command reference, a runbook, \
a config — you may suggest that the user pin it, with a normal command \
line: 'CMD: #@/path <file>'. Goulash reads it into your working context. \
Suggest it; never assume it happened.\n\
Never invent a path. Every file or directory you name must be one that \
appears in this prompt — in the working context, the session log, or the \
user's question. Pinned items are shown as '@label (/real/path)': use the \
path in the parentheses, never the label, and never guess where goulash \
keeps its own files.\n\n";

#[allow(clippy::too_many_arguments)]
/// What context to ask the provider for — usually nothing.
///
/// `num_ctx` is part of a model's load identity, so naming one evicts
/// and reloads anything loaded at a different size (measured: 206ms to
/// reuse a warm model, 1847ms to reload it). Both engines already let
/// the user set a default, so the polite behaviour is to take it.
///
/// Two exceptions:
///   * `num_ctx` set explicitly: the user wants exactly this, pinned.
///   * the resident window is below `num_ctx_min`: too small to hold a
///     session log, so nudge the host and eat the reload.
///
/// Returns 0 only when there is genuinely nothing to say — no floor
/// configured and nothing resident. The wire layer omits the option
/// entirely at zero, which is the *only* way to say nothing: a literal
/// `num_ctx: 0` is a request, not a silence.
///
/// `resident` is None when we could not ask (an OpenAI-compatible
/// server has no residency view) — which is *not* the same as "nothing
/// is loaded", so it falls through to asking for the floor only when we
/// genuinely know the model is cold.
/// The window to send, resolved against what the server actually has
/// loaded right now.
///
/// Queried per generation rather than cached. A cache is exactly wrong
/// here: if the user loads a different model behind our back, a stale
/// value makes us send a `num_ctx` that forces the reload this function
/// exists to avoid. One localhost round-trip is ~1ms against a
/// generation measured in hundreds, so correctness is nearly free.
fn ctx_for(cl: &Client, cfg: &EngineConfig, model: &str) -> usize {
    if cfg.num_ctx > 0 {
        return cfg.num_ctx;
    }
    let loaded = cl.resident();
    // An empty list means "could not tell" as often as "nothing loaded"
    // — an OpenAI-compatible server has no residency view at all — so it
    // is only read as cold when the server answered and said so.
    let found = loaded
        .iter()
        .find(|(n, _)| n == model || n.split(':').next() == model.split(':').next())
        .map(|(_, c)| *c);
    negotiate_ctx(cfg, found.filter(|c| *c > 0))
}

/// The prompt, assembled exactly as an ask sends it.
///
/// Carved out of `generate` so the characterization bench can build the
/// **real** prompt rather than a copy of it. A harness with its own
/// prompt builder measures the harness: the two drift the first time
/// either is touched, and every conclusion drawn from the numbers
/// silently stops applying to the shipped product.
///
/// `now` is a parameter rather than a call to `local_now()` for the same
/// reason — the current time sits in the volatile suffix, so a live
/// clock makes two runs of the same cell differ and turns a latency
/// comparison into noise. The product passes the real clock; the bench
/// freezes it.
///
/// Order is most-stable-first, which is what makes prefix caching work:
/// preamble, machine facts, memories, pinned files, session log, then
/// the volatile tail (time, cards, question, directive).
#[allow(clippy::too_many_arguments)]
pub fn build_prompt(
    facts: &str,
    memories: &str,
    pinned: &str,
    context: &str,
    cards: &str,
    question: &str,
    directive: &str,
    now: &str,
) -> String {
    format!(
        "{PREAMBLE}{facts}{memories}{pinned}Session log (oldest first):\n{context}\n\
         Current local time: {now}\n{cards}Question: {question}\n{directive}\nAnswer:"
    )
}

/// Which directive a turn gets, by ingress and ordering. Public for the
/// same reason as `build_prompt`: the bench sweeps `command_first`, and
/// it has to sweep the real wording.
pub fn directive_for(command_first: bool, proactive: bool, pin_ask: bool) -> &'static str {
    match (command_first, proactive) {
        _ if pin_ask => {
            "Answer ONLY with PIN: / PINCLEAR lines plus at most one short \
             prose line. No CMD: line."
        }
        
        // Nothing may follow `CMD: <command>` on these lines. Mimicry is
        // what teaches a small model the format, so a directive that
        // trails an em-dash and an explanation after the placeholder
        // teaches exactly that — and the user then pulls a command with
        // prose welded onto the end and runs it. Field-caught.
        // The grammar itself lives in the byte-stable preamble; these
        // only say which tags apply this turn, and in what order. A
        // uniform tagged grammar is what lets a small model keep the
        // format: with one tagged line and untagged prose it has to
        // infer when to STOP tagging, and that is the inference that
        // welds an explanation onto the end of a command.
        (true, false) => {
            "Answer with exactly two lines:\n\
             CMD: <command>\n\
             SAY: <one short line explaining it>\n\
             Include the CMD line whenever any shell command could \
             accomplish, fix, or demonstrate what was asked."
        }
        (true, true) => {
            "Answer with:\n\
             CMD: <command>\n\
             SAY: <one short line>\n\
             Include the CMD line ONLY if a genuinely useful next \
             command exists; otherwise send the SAY line alone."
        }
        (false, false) => {
            "Answer with exactly two lines:\n\
             SAY: <one short line>\n\
             CMD: <command>\n\
             Include the CMD line whenever any shell command could \
             accomplish, fix, or demonstrate what was asked."
        }
        (false, true) => {
            "Answer with:\n\
             SAY: <one short line>\n\
             CMD: <command>\n\
             Include the CMD line ONLY if a genuinely useful next \
             command exists; otherwise send the SAY line alone."
        }
    }
}

/// What the slow lane asks for, falling back to the fast lane's dial.
///
/// The lanes exist to differ here: fast is the one that has to be
/// immediate, slow is the one that can afford to think. One shared
/// setting made that impossible to express.
fn slow_thinking(cfg: &EngineConfig) -> &str {
    cfg.slow_lane.thinking.as_deref().unwrap_or(&cfg.thinking)
}

/// The slow lane's ceiling, falling back to the fast lane's.
fn slow_max_tokens(cfg: &EngineConfig) -> usize {
    cfg.slow_lane.max_tokens.unwrap_or(cfg.max_tokens)
}

fn negotiate_ctx(cfg: &EngineConfig, resident: Option<usize>) -> usize {
    if cfg.num_ctx > 0 {
        return cfg.num_ctx;
    }
    match resident {
        // Loaded and roomy enough: ask for exactly what is loaded.
        //
        // Not "say nothing". Silence is not neutral here — ollama reads
        // an absent `num_ctx` as the model's own default (131072 for
        // gemma4:e4b, measured) and reloads to reach it. The only
        // request that matches the current load identity, and so the
        // only one that does not evict, is the resident number itself.
        Some(c) if c >= cfg.num_ctx_min => c,
        // Loaded but too small: the expensive case, on purpose.
        Some(_) if cfg.nudge_small_context => cfg.num_ctx_min,
        // Too small, and we were told not to nudge: take what is there
        // rather than reload to something we were asked not to ask for.
        Some(c) => c,
        // Nothing to preserve — ask for the floor.
        None => cfg.num_ctx_min,
    }
}

fn generate(
    cl: &Client,
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
    let directive = directive_for(cfg.command_first, proactive, pin_ask);
    // Stable-prefix order, most stable first: preamble, machine facts,
    // memories, working context, session log. A pin changes far less
    // often than the log, so it belongs above it in the prefix.
    // ...and the cards ride at the very end, with the question. That
    // position is re-prefilled on every ask, which is exactly why they
    // are kept to a few hundred characters — and it is the only place a
    // pin lands inside a sliding-window model's attention.
    //
    // The facts sit directly after the preamble because they are the
    // most stable thing in the prompt — they change when the machine
    // changes, which is approximately never — so they cost one cache
    // miss ever, and every later turn reads them for free.
    let facts = crate::facts::block(&cfg.divulge);
    let prompt = build_prompt(
        &facts, memories, pinned, context, cards, question, directive, &local_now(),
    );
    let _ = ev.send(Event::Prompt {
        chars: prompt.chars().count(),
    });
    // Reasoning models (qwen3+, deepseek-r1) otherwise spend the entire
    // token budget in a separate `thinking` field, returning an empty
    // `response` — a blank bar in the field. The model's own dialect
    // decides the shape of the request and the size of the top-up
    // (models.rs); a model that cannot reason is not sent the field at
    // all, because some providers reject it rather than ignore it.
    let body = cl.be.wire.body(&Gen {
        model,
        prompt: &prompt,
        stream: cfg.stream,
        temperature: 0.2,
        max_tokens: cfg.max_tokens,
        num_ctx: ctx_for(cl, cfg, model),
        // No stop sequence. It was `["\n\n"]` here — the last path still
        // carrying it, after research/digest/warm had already dropped it.
        //
        // Measured over ~5,500 generations: removing it lifts the answer
        // rate 81% -> 94%. And with reasoning on it is not a degradation
        // but a wall — a blank line inside or before the thinking trips
        // it, so the model emits ~4 tokens and halts no matter how large
        // the budget. That defeats the allowance right above: paying for
        // reasoning does not help if generation stops mid-thought.
        //
        // It existed to enforce the one-line contract, which `directive`
        // and the band's wrap already enforce. (bench/QUIRKS.md §3)
        stop: &[],
        think: caps.think_field(&cfg.thinking),
        effort: caps.effort_field(&cfg.thinking),
        keep_alive: &cfg.keep_alive,
        num_keep: cfg.num_keep,
        seed: (cfg.seed >= 0).then_some(cfg.seed),
    });
    let resp = cl
        .post(&cl.gen_url())
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;

    if !cfg.stream {
        let text = resp.into_string().map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        return cl
            .be
            .wire
            .text(&v)
            .map(|s| (s.trim().to_string(), None))
            .ok_or_else(|| "malformed engine response".to_string());
    }

    // Streaming: line-delimited either way — one JSON object per line
    // from ollama, `data: {...}` SSE from an OpenAI-compatible server.
    // Forward throttled partials so the bar fills in as tokens arrive.
    let reader = std::io::BufReader::new(resp.into_reader());
    let mut acc = String::new();
    let mut last_emit = Instant::now();
    let mut early: Option<String> = None;
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        // A line we cannot read is a keep-alive or a blank separator,
        // not a failure: SSE is full of both, and aborting the stream
        // over one would lose an answer that was arriving fine.
        let Some(chunk) = cl.be.wire.chunk(&line) else {
            continue;
        };
        acc.push_str(&chunk.text);
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
        if chunk.done {
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

/// Does this line already carry one of the grammar's tags? Used only to
/// stop a stray verb line being mistaken for the prose answer.
fn is_tagged(l: &str) -> bool {
    const TAGS: &[&str] = &[
        "CMD:",
        "SAY:",
        "REASON:",
        "REMEMBER:",
        "FORGET:",
        "PIN:",
        "PINCLEAR",
    ];
    TAGS.iter().any(|t| l.starts_with(t))
}

/// Strip trailing prose from a `CMD:` line.
///
/// An em-dash or en-dash is never a shell operator, so one appearing
/// unquoted in a command line means the model kept explaining past the
/// end of the command. Cutting there is at worst equivalent — the shell
/// would have choked on it anyway — and at best turns a line that
/// errors fifteen times into one that runs.
///
/// Deliberately narrow. ` -- ` is a real shell idiom (`git log --`), and
/// `#` inside a command can be a fragment or a literal, so neither is
/// safe to cut on.
fn clean_cmd(s: &str) -> String {
    let cut = s
        .char_indices()
        .find(|(_, c)| matches!(c, '\u{2014}' | '\u{2013}'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s[..cut].trim().to_string()
}

/// A `CMD:` line that has been fully received (its newline has arrived),
/// so it is safe to vend before the rest of the answer streams in.
fn complete_cmd_line(acc: &str) -> Option<String> {
    for line in acc.lines() {
        if (!acc.ends_with(line) || acc.ends_with('\n'))
            && let Some(c) = line.trim().strip_prefix("CMD:").map(clean_cmd).as_deref()
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
pub fn extract_memory_ops(raw: &str) -> (String, Vec<String>, Vec<u64>) {
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
pub fn split_answer(
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
        if let Some(c) = l.strip_prefix("CMD:").map(clean_cmd).as_deref() {
            if command.is_none() && !c.trim().is_empty() {
                command = Some(c.trim().to_string());
            }
        } else if let Some(t) = l.strip_prefix("SAY:") {
            if text.is_empty() && !t.trim().is_empty() {
                text = t.trim().to_string();
            }
        } else if text.is_empty() && !is_tagged(l) {
            // Lenient: a model that ignores the grammar entirely still
            // gets its prose through. The tag is what makes the FORMAT
            // unambiguous, not what makes the answer usable.
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
mod backfill_tests {
    use super::{BACKFILL_CAP, Job, push_backfill};
    use std::collections::VecDeque;

    fn job(n: u64) -> Job {
        Job::Research {
            turn: n,
            question: String::new(),
            context: String::new(),
            memories: String::new(),
            pinned: String::new(),
        }
    }

    fn turns(q: &VecDeque<Job>) -> Vec<u64> {
        q.iter()
            .map(|j| match j {
                Job::Research { turn, .. } => *turn,
                _ => u64::MAX,
            })
            .collect()
    }

    #[test]
    fn backfill_is_bounded_and_drops_the_stalest() {
        let mut q = VecDeque::new();
        for n in 0..(BACKFILL_CAP as u64 + 5) {
            push_backfill(&mut q, job(n));
        }
        assert_eq!(q.len(), BACKFILL_CAP, "an unbounded queue is megabytes");
        assert_eq!(
            turns(&q),
            (5..(BACKFILL_CAP as u64 + 5)).collect::<Vec<_>>(),
            "the question moved on from longest ago is evicted first"
        );
    }
}

#[cfg(test)]
mod answer_tests {
    use super::{split_answer, split_finding};
    use std::collections::HashSet;

    #[test]
    fn prose_welded_onto_a_cmd_line_is_cut_off() {
        // Field bug: the directive itself showed "CMD: <command> — ..."
        // and the model mimicked it, so the user pulled a command with
        // an explanation on the end and the shell ran the explanation as
        // arguments. An em-dash is never a shell operator, so cutting
        // there is at worst equivalent to what the shell would do.
        let (_t, c) = split_answer(
            "CMD: du -ah . | sort -rh | head -n 10 \u{2014} This lists the top 10 \
             largest files\nSAY: disk hogs",
            &paths(&[]),
        );
        assert_eq!(c.as_deref(), Some("du -ah . | sort -rh | head -n 10"));
    }

    #[test]
    fn the_say_tag_is_read_and_stripped() {
        let (t, c) = split_answer("CMD: ls -la\nSAY: lists everything", &paths(&[]));
        assert_eq!(t, "lists everything");
        assert_eq!(c.as_deref(), Some("ls -la"));
    }

    #[test]
    fn an_untagged_answer_still_works() {
        // Leniency is the point: the tag makes the FORMAT unambiguous,
        // it is not a precondition for the answer being usable.
        let (t, c) = split_answer("lists everything\nCMD: ls -la", &paths(&[]));
        assert_eq!(t, "lists everything");
        assert_eq!(c.as_deref(), Some("ls -la"));
    }

    #[test]
    fn a_stray_verb_line_is_not_mistaken_for_prose() {
        let (t, _c) = split_answer("REMEMBER: they use pnpm\nSAY: noted", &paths(&[]));
        assert_eq!(t, "noted", "a REMEMBER line is not the answer text");
    }

    #[test]
    fn findings_speak_the_same_grammar() {
        let (line, cmd, reason) = split_finding(
            "CMD: rg -n todo \u{2014} searches recursively\nSAY: faster than grep\n\
             REASON: ripgrep skips .gitignore",
        );
        assert_eq!(cmd.as_deref(), Some("rg -n todo"));
        assert_eq!(line, "faster than grep");
        assert!(reason.contains("gitignore"));
    }

    #[test]
    fn a_dash_inside_a_quoted_argument_is_left_alone() {
        // The cut is on em/en dashes only, so ordinary flags survive.
        let (_t, c) = split_answer("CMD: git log --oneline -n 20", &paths(&[]));
        assert_eq!(c.as_deref(), Some("git log --oneline -n 20"));
    }

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
    use crate::wire::Wire;
    use serde_json::json;

    /// The names the wire would have extracted from that listing —
    /// pick_model is handed them rather than re-deriving, because the
    /// two providers report them under different keys.
    fn names(tags: &serde_json::Value) -> Vec<String> {
        Wire::Ollama.models(tags)
    }

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
            pick_model(Wire::Ollama, &tags, &names(&tags), None, &no_favs()).as_deref(),
            Some("llama3.2:1b")
        );
    }

    #[test]
    fn configured_model_wins() {
        let tags = json!({"models": [{"name": "llama3.2:1b", "size": 1u64}]});
        assert_eq!(
            pick_model(
                Wire::Ollama,
                &tags,
                &names(&tags),
                Some("gemma3:12b"),
                &["llama3.2:1b".to_string()]
            )
            .as_deref(),
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
            pick_model(Wire::Ollama, &tags, &names(&tags), None, &favs).as_deref(),
            Some("qwen2.5:7b"),
            "favorite matches through the :tag"
        );
    }

    #[test]
    fn missing_sizes_still_pick_something() {
        let tags = json!({"models": [{"name": "mystery"}]});
        assert_eq!(
            pick_model(Wire::Ollama, &tags, &names(&tags), None, &no_favs()).as_deref(),
            Some("mystery")
        );
    }
}


#[cfg(test)]
mod ctx_tests {
    use super::negotiate_ctx;
    use crate::config::EngineConfig;

    fn cfg(pin: usize, floor: usize, nudge: bool) -> EngineConfig {
        let mut c = EngineConfig::default();
        c.num_ctx = pin;
        c.num_ctx_min = floor;
        c.nudge_small_context = nudge;
        c
    }

    /// An explicit window is a pin: the user accepts the reload.
    #[test]
    fn a_pinned_window_always_wins() {
        assert_eq!(negotiate_ctx(&cfg(4096, 8192, true), Some(32768)), 4096);
        assert_eq!(negotiate_ctx(&cfg(4096, 8192, true), None), 4096);
    }

    /// The default case, and the whole point: a model is loaded with
    /// room to spare, so keep it loaded. Sending 8192 at a model loaded
    /// with 32768 evicts it — 206ms became 1847ms.
    ///
    /// "Keep it" is spelled by echoing the resident number, NOT by
    /// staying silent. This test used to assert 0, and 0 is the one
    /// answer that reloads: absent, ollama reaches for the model's own
    /// default (131072 on gemma4:e4b) and evicts to get there. The
    /// negotiation was reloading the model on every ask precisely
    /// because it was trying not to.
    #[test]
    fn a_roomy_resident_model_is_asked_for_what_it_already_has() {
        assert_eq!(negotiate_ctx(&cfg(0, 8192, true), Some(32768)), 32768);
        assert_eq!(negotiate_ctx(&cfg(0, 8192, true), Some(8192)), 8192);
    }

    /// Too small to hold a session log: pay the reload, once.
    #[test]
    fn a_cramped_resident_model_is_nudged() {
        assert_eq!(negotiate_ctx(&cfg(0, 8192, true), Some(2048)), 8192);
    }

    /// ...unless the user said never provoke a reload — in which case
    /// we take the cramped window as it stands, and say so on the wire.
    #[test]
    fn nudging_can_be_refused() {
        assert_eq!(negotiate_ctx(&cfg(0, 8192, false), Some(2048)), 2048);
    }

    /// Nothing loaded, so nothing to protect — ask for the floor.
    #[test]
    fn a_cold_model_gets_the_floor() {
        assert_eq!(negotiate_ctx(&cfg(0, 8192, true), None), 8192);
    }

    /// With a resident model, the negotiated value is never 0 — because
    /// 0 does not reach the server as a window at all, and every path
    /// that produced it was an eviction wearing a "leave it alone" hat.
    #[test]
    fn a_resident_model_never_negotiates_to_silence() {
        for floor in [0, 2048, 8192, 131072] {
            for nudge in [true, false] {
                for res in [512, 2048, 8192, 131072] {
                    assert_ne!(
                        negotiate_ctx(&cfg(0, floor, nudge), Some(res)),
                        0,
                        "floor={floor} nudge={nudge} resident={res}"
                    );
                }
            }
        }
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
