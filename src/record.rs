use crate::config::Config;
use crate::sense::State;
use base64::Engine as _;
use serde_json::json;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::time::{SystemTime, UNIX_EPOCH};

/// JSONL session transcript: raw output plus state annotations.
///
/// The leaves of the memory tree (wiki: architecture/block-history.md).
/// Typed input is deliberately never recorded at this layer — the
/// echo-off privacy invariant is enforced by omission, not filtering.
pub struct Recorder {
    out: Option<BufWriter<File>>,
    record_output: bool,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Trim the history directory to its limits, oldest first.
///
/// Returns what went, so the caller can say so. Deleting a developer's
/// transcripts silently is the kind of helpfulness nobody asked for —
/// and this runs at startup, where the alternative to a line of output
/// is finding out later that last Tuesday is gone.
///
/// `keep` is never considered: it is the file this session is about to
/// write, and a sweep that can delete the thing it is standing on is a
/// sweep that eventually does.
fn sweep(dir: &std::path::Path, max_mb: u64, max_sessions: usize, keep: &std::path::Path) -> (usize, u64) {
    let Ok(rd) = fs::read_dir(dir) else {
        return (0, 0);
    };
    let mut files: Vec<(SystemTime, u64, std::path::PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p != keep && p.extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|p| {
            let m = p.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), p))
        })
        .collect();
    // Newest first, so the walk keeps what is worth keeping and the
    // budget runs out on the old.
    files.sort_by_key(|(t, ..)| std::cmp::Reverse(*t));
    let budget = max_mb.saturating_mul(1024 * 1024);
    let (mut kept_bytes, mut kept_n) = (0u64, 0usize);
    let (mut gone_n, mut gone_bytes) = (0usize, 0u64);
    for (_, len, path) in files {
        kept_bytes += len;
        kept_n += 1;
        if kept_bytes > budget || kept_n > max_sessions {
            if fs::remove_file(&path).is_ok() {
                gone_n += 1;
                gone_bytes += len;
            }
            kept_bytes -= len;
            kept_n -= 1;
        }
    }
    (gone_n, gone_bytes)
}

impl Recorder {
    pub fn new(cfg: &Config) -> Recorder {
        if !cfg.record.enabled {
            return Recorder {
                out: None,
                record_output: false,
            };
        }
        let mut swept = None;
        let out = Config::dir()
            .map(|d| d.join("history"))
            .and_then(|dir| {
                fs::create_dir_all(&dir).ok()?;
                let name = format!("session-{}-{}.jsonl", now_ms(), std::process::id());
                let path = dir.join(name);
                let f = File::create(&path).ok()?;
                let (n, bytes) = sweep(&dir, cfg.record.max_mb, cfg.record.max_sessions, &path);
                if n > 0 {
                    swept = Some((n, bytes));
                }
                Some(f)
            })
            .map(BufWriter::new);
        // Said, not silent. Written before the session takes the screen,
        // so it lands in the scrollback rather than fighting the band.
        if let Some((n, bytes)) = swept {
            eprintln!(
                "goulash: history trimmed to {} MB / {} sessions \u{2014} removed {n} older \
                 session{} ({} MB)",
                cfg.record.max_mb,
                cfg.record.max_sessions,
                if n == 1 { "" } else { "s" },
                bytes / (1024 * 1024)
            );
        }
        Recorder {
            out,
            record_output: cfg.record.output,
        }
    }

    fn write(&mut self, value: serde_json::Value) {
        if let Some(w) = self.out.as_mut()
            && serde_json::to_writer(&mut *w, &value).is_ok()
        {
            let _ = w.write_all(b"\n");
        }
    }

    fn flush(&mut self) {
        if let Some(w) = self.out.as_mut() {
            let _ = w.flush();
        }
    }

    pub fn start(&mut self, argv: &[String], rows: u16, cols: u16) {
        self.write(json!({
            "t": now_ms() as u64, "ev": "start",
            "argv": argv, "rows": rows, "cols": cols,
            "goulash": env!("CARGO_PKG_VERSION"),
        }));
        self.flush();
    }

    pub fn state(&mut self, s: State) {
        self.write(json!({
            "t": now_ms() as u64, "ev": "state",
            "fg": if s.fg_shell { "shell" } else { "child" },
            "echo": s.echo, "icanon": s.icanon, "alt": s.alt_screen,
        }));
        self.flush();
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.write(json!({"t": now_ms() as u64, "ev": "resize", "rows": rows, "cols": cols}));
        self.flush();
    }

    /// Raw child output, base64 to preserve exact bytes.
    pub fn output(&mut self, chunk: &[u8]) {
        if self.record_output {
            let b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
            self.write(json!({"t": now_ms() as u64, "ev": "out", "b64": b64}));
        }
    }

    // Block-boundary events from the shell integration (osc::Mark).

    pub fn prompt(&mut self) {
        self.write(json!({"t": now_ms() as u64, "ev": "prompt"}));
        self.flush();
    }

    pub fn cmd_start(&mut self, text: &str) {
        self.write(json!({"t": now_ms() as u64, "ev": "cmd", "text": text}));
        self.flush();
    }

    pub fn cmd_end(&mut self, code: i32) {
        self.write(json!({"t": now_ms() as u64, "ev": "cmd_end", "code": code}));
        self.flush();
    }

    pub fn cwd(&mut self, path: &str) {
        self.write(json!({"t": now_ms() as u64, "ev": "cwd", "path": path}));
        self.flush();
    }

    pub fn engine_ready(&mut self, provider: &str, model: &str) {
        self.write(json!({
            "t": now_ms() as u64, "ev": "engine",
            "provider": provider, "model": model,
        }));
        self.flush();
    }

    pub fn memory(&mut self, op: &str, id: u64, text: &str) {
        self.write(json!({"t": now_ms() as u64, "ev": "memory", "op": op, "id": id, "text": text}));
        self.flush();
    }

    pub fn engine_debug(&mut self, raw: &str) {
        self.write(json!({"t": now_ms() as u64, "ev": "engine_debug", "raw": raw}));
        self.flush();
    }

    pub fn aside_answer(&mut self, text: &str, ok: bool) {
        self.write(json!({"t": now_ms() as u64, "ev": "answer", "text": text, "ok": ok}));
        self.flush();
    }

    pub fn aside(&mut self, text: &str) {
        self.write(json!({"t": now_ms() as u64, "ev": "aside", "text": text}));
        self.flush();
    }

    /// A researched finding arriving from the slow lane, and what
    /// happened to it. The log recorded nothing about research at all,
    /// so "did slow answer, and did its answer land?" was unanswerable
    /// from outside — which is a whole class of bug you cannot even see.
    pub fn finding(&mut self, turn: u64, cmd: Option<&str>, outcome: &str) {
        self.write(json!({
            "t": now_ms() as u64, "ev": "finding",
            "turn": turn, "cmd": cmd, "outcome": outcome,
        }));
        self.flush();
    }

    pub fn suggest(&mut self, id: u64, cmd: &str, why: &str, vendor: &str) {
        self.write(json!({
            "t": now_ms() as u64, "ev": "suggest",
            "id": id, "cmd": cmd, "why": why, "vendor": vendor,
        }));
        self.flush();
    }

    pub fn accept(&mut self, id: u64) {
        self.write(json!({"t": now_ms() as u64, "ev": "accept", "id": id}));
        self.flush();
    }

    pub fn end(&mut self, code: i32) {
        self.write(json!({"t": now_ms() as u64, "ev": "end", "code": code}));
        self.flush();
    }
}
