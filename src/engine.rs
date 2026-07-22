use crate::config::EngineConfig;
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
                        command and its result. If you have ONE genuinely \
                        useful short tip or fix, say it (add a CMD: line if \
                        a command applies). A CMD: line is for a command the \
                        user would plausibly run next — never invent \
                        busywork like logging, note-taking, or echo \
                        commands. Most turns deserve no comment: when in \
                        doubt, reply exactly: PASS"
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

fn worker(mut cfg: EngineConfig, jobs: mpsc::Receiver<Job>, ev: mpsc::Sender<Event>, wr: OwnedFd) {
    let path_set = crate::vendor::path_executable_set();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout(Duration::from_secs(120))
        .build();

    // Probe chain, v1: ollama (explicit or auto-detected). "none" was
    // filtered by the caller. (wiki: llm-engine.md probe chain)
    let mut state = probe_ollama(&agent, &cfg);
    if let Some((model, _)) = &state {
        announce(&ev, &wr, model);
        if cfg.prewarm {
            warm_marked(&agent, &cfg, model, &ev, &wr);
        }
    }

    while let Ok(first) = jobs.recv() {
        // Late binding: ollama may have started after goulash did, so an
        // unbound engine re-probes whenever work arrives.
        if state.is_none() {
            state = probe_ollama(&agent, &cfg);
            if let Some((model, _)) = &state {
                announce(&ev, &wr, model);
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
                        announce(&ev, &wr, model);
                        pending_warm = true;
                    }
                    None => unreachable_engine(&ev, &wr, &cfg),
                },
                Job::Rebind => {
                    cfg.model = None;
                    state = probe_ollama(&agent, &cfg);
                    match &state {
                        Some((model, _)) => {
                            announce(&ev, &wr, model);
                            pending_warm = true;
                        }
                        None => unreachable_engine(&ev, &wr, &cfg),
                    }
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
            proactive,
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
                &agent, &cfg, model, &question, &context, &memories, &ev, &wr, proactive,
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

fn announce(ev: &mpsc::Sender<Event>, wr: &OwnedFd, model: &str) {
    let _ = ev.send(Event::Ready {
        provider: "ollama".to_string(),
        model: model.to_string(),
    });
    notify(wr);
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
to them.\n\n";

#[allow(clippy::too_many_arguments)]
fn generate(
    agent: &ureq::Agent,
    cfg: &EngineConfig,
    model: &str,
    question: &str,
    context: &str,
    memories: &str,
    ev: &mpsc::Sender<Event>,
    wr: &OwnedFd,
    proactive: bool,
) -> Result<String, String> {
    // Volatile parts (current time, question) go AFTER the stable prefix.
    // The command directive is repeated at point-of-use: small models
    // lose instructions that only appear at the top of a long prompt.
    // Prompt shape (stable-prefix first): preamble, pinned memories,
    // session log, then the volatile time/question/directive suffix.
    let prompt = format!(
        "{PREAMBLE}{memories}Session log (oldest first):\n{context}\n\
         Current local time: {}\nQuestion: {question}\n\
         Reply with ONE short prose line. If a shell command applies, add a \
         second line formatted exactly as: CMD: <command>\nAnswer:",
        local_now()
    );
    let mut body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": cfg.stream,
        // Reasoning models (qwen3+, deepseek-r1) otherwise spend the
        // entire token budget in a separate `thinking` field, returning
        // an empty `response` — a blank bar in the field.
        "think": false,
        "options": {
            "temperature": 0.2,
            "num_predict": cfg.max_tokens as i64,
            "num_ctx": cfg.num_ctx as i64,
            "stop": ["\n\n"],
        },
    });
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
            .map(|s| s.trim().to_string())
            .ok_or_else(|| "malformed engine response".to_string());
    }

    // Streaming: one JSON object per line; forward throttled partials so
    // the bar fills in as tokens arrive.
    let reader = std::io::BufReader::new(resp.into_reader());
    let mut acc = String::new();
    let mut last_emit = Instant::now();
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if let Some(tok) = v["response"].as_str() {
            acc.push_str(tok);
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
    Ok(acc.trim().to_string())
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
