use crate::config::EngineConfig;
use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::time::Duration;

/// The LLM engine: a worker thread so inference latency never touches the
/// PTY loop. Events come back over an mpsc channel; a self-pipe byte wakes
/// the session's poll(). (wiki: architecture/llm-engine.md)
pub enum Event {
    Ready { provider: String, model: String },
    Answer(String),
    Error(String),
}

pub struct Job {
    pub question: String,
    pub context: String,
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

    pub fn ask(&self, question: String, context: String) {
        let _ = self.job_tx.send(Job { question, context });
    }
}

fn notify(wr: &OwnedFd) {
    let _ = nix::unistd::write(wr, b"e");
}

fn worker(cfg: EngineConfig, jobs: mpsc::Receiver<Job>, ev: mpsc::Sender<Event>, wr: OwnedFd) {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout(Duration::from_secs(120))
        .build();

    // Probe chain, v1: ollama (explicit or auto-detected). "none" was
    // filtered by the caller. (wiki: llm-engine.md probe chain)
    let Some(model) = probe_ollama(&agent, &cfg) else {
        // Nothing found: park, holding the wake pipe's write end open —
        // dropping it would storm the session's poll() with POLLHUP.
        // recv() errors out when the session drops the Engine.
        while jobs.recv().is_ok() {}
        drop(wr);
        return;
    };
    let _ = ev.send(Event::Ready {
        provider: "ollama".to_string(),
        model: model.clone(),
    });
    notify(&wr);

    while let Ok(job) = jobs.recv() {
        let result = generate(&agent, &cfg.host, &model, &job);
        let _ = match result {
            Ok(ans) => ev.send(Event::Answer(ans)),
            Err(e) => ev.send(Event::Error(e)),
        };
        notify(&wr);
    }
}

fn probe_ollama(agent: &ureq::Agent, cfg: &EngineConfig) -> Option<String> {
    let resp = agent
        .get(&format!("{}/api/tags", cfg.host))
        .timeout(Duration::from_secs(1))
        .call()
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&resp.into_string().ok()?).ok()?;
    if let Some(m) = &cfg.model {
        return Some(m.clone());
    }
    v["models"][0]["name"].as_str().map(String::from)
}

fn generate(agent: &ureq::Agent, host: &str, model: &str, job: &Job) -> Result<String, String> {
    let prompt = format!(
        "{}\nQuestion: {}\nAnswer (one short line, plain text):",
        job.context, job.question
    );
    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "options": {"temperature": 0.2},
    });
    let resp = agent
        .post(&format!("{host}/api/generate"))
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    v["response"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "malformed engine response".to_string())
}

/// Session context assembled from recent command blocks; the terse-answer
/// instruction lives here so every provider gets the same contract.
pub fn build_context(blocks: &[(String, i32, String)], cwd: &str) -> String {
    let mut s = String::from(
        "You are goulash, an assistant living in the user's terminal status \
         bar. Answer tersely in ONE short line of plain text, no markdown.\n\n\
         Recent terminal activity (oldest first):\n",
    );
    for (cmd, code, tail) in blocks {
        s.push_str(&format!("$ {cmd}   [exit {code}]\n"));
        let t = tail.trim();
        if !t.is_empty() {
            s.push_str(t);
            s.push('\n');
        }
    }
    if !cwd.is_empty() {
        s.push_str(&format!("cwd: {cwd}\n"));
    }
    s
}
