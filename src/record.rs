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

impl Recorder {
    pub fn new(cfg: &Config) -> Recorder {
        if !cfg.record.enabled {
            return Recorder {
                out: None,
                record_output: false,
            };
        }
        let out = Config::dir()
            .map(|d| d.join("history"))
            .and_then(|dir| {
                fs::create_dir_all(&dir).ok()?;
                let name = format!("session-{}-{}.jsonl", now_ms(), std::process::id());
                File::create(dir.join(name)).ok()
            })
            .map(BufWriter::new);
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
