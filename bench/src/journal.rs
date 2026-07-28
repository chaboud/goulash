//! Append-only, crash-safe result journal.
//!
//! The sweep is hours long and will be interrupted — by a laptop lid, a
//! flight, an OOM on a 17 GB model. Every completed cell is durable the
//! instant it finishes, and a restart skips exactly what already landed.
//!
//! Two files, both append-only:
//!   `manifest.jsonl` — every planned cell, written before any work.
//!                      Makes "what is missing" answerable without
//!                      re-deriving the plan.
//!   `journal.jsonl`  — one row per completed cell, flushed and fsynced.
//!   `prompts.jsonl`  — full prompt text, split out so resume stays fast.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Identifies one unit of work. Stable across runs — it is the resume key.
pub fn cell_key(pass: &str, provider: &str, model: &str, shape: &str, step: &str) -> String {
    format!("{pass}/{provider}/{model}/{shape}/{step}")
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Row {
    pub key: String,
    pub pass: String,
    pub provider: String,
    pub model: String,
    pub tier: String,
    pub shape: String,
    pub step: String,
    pub turn_index: usize,

    /// What was asked, verbatim — so a grader never has to reconstruct it.
    pub question: String,
    pub prompt_chars: usize,
    pub prompt_hash: u64,
    /// Model output, verbatim and unparsed. The audit anchor.
    pub raw: String,

    // ---- parsed by goulash's REAL parser, not a reimplementation
    pub text: String,
    pub command: Option<String>,
    pub remembers: Vec<String>,
    pub forgets: Vec<u64>,

    // ---- timings and counters straight off the provider
    pub ttft_ms: Option<u64>,
    pub total_ms: u64,
    pub load_ms: Option<u64>,
    pub prompt_tokens: Option<u64>,
    /// Ollama only. The direct cache signal — collapses on a prefix hit
    /// while the token count stays flat.
    pub prompt_eval_ms: Option<u64>,
    pub eval_tokens: Option<u64>,
    pub eval_ms: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub stop_reason: Option<String>,

    // ---- mechanical checks (no taste involved)
    pub empty: bool,
    pub fenced: bool,
    pub prose_lines: usize,
    pub has_cmd_tag: bool,
    pub error: Option<String>,
}

pub struct Journal {
    dir: PathBuf,
    journal: File,
    prompts: File,
    /// Completed rows, not just their keys: a resumed session has to
    /// re-append the *same* answer text to its session log, or the
    /// prompts it builds diverge from the original run and every cache
    /// and latency comparison silently drifts.
    done: HashMap<String, Row>,
}

impl Journal {
    /// Open (or reopen) a run directory, loading whatever already landed.
    pub fn open(dir: &Path) -> std::io::Result<Journal> {
        std::fs::create_dir_all(dir)?;
        let done = Self::completed_keys(&dir.join("journal.jsonl"));
        Ok(Journal {
            dir: dir.to_path_buf(),
            journal: Self::append(&dir.join("journal.jsonl"))?,
            prompts: Self::append(&dir.join("prompts.jsonl"))?,
            done,
        })
    }

    fn append(path: &Path) -> std::io::Result<File> {
        OpenOptions::new().create(true).append(true).open(path)
    }

    /// Rows already recorded. A row that fails to parse is treated as
    /// absent, so a torn final line from a hard kill just gets redone.
    fn completed_keys(path: &Path) -> HashMap<String, Row> {
        let Ok(f) = File::open(path) else {
            return HashMap::new();
        };
        BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str::<Row>(&l).ok())
            .map(|r| (r.key.clone(), r))
            .collect()
    }

    pub fn is_done(&self, key: &str) -> bool {
        self.done.contains_key(key)
    }

    /// The recorded result for a key, so resume can replay its effect on
    /// the session log instead of guessing.
    pub fn recorded(&self, key: &str) -> Option<&Row> {
        self.done.get(key)
    }

    pub fn completed(&self) -> usize {
        self.done.len()
    }

    /// Progress within one pass. A run directory holds pass-a and pass-b
    /// together, so the overall count would otherwise overshoot the
    /// pass's own total and read as nonsense mid-sweep.
    pub fn completed_in(&self, pass: &str) -> usize {
        self.done.values().filter(|r| r.pass == pass).count()
    }

    /// Record the plan before doing any work, so an interrupted run can
    /// still be audited for coverage.
    pub fn write_manifest(&self, keys: &[String]) -> std::io::Result<()> {
        let mut f = File::create(self.dir.join("manifest.jsonl"))?;
        for k in keys {
            writeln!(f, "{}", serde_json::json!({ "key": k }))?;
        }
        f.sync_all()
    }

    /// Durable before returning: an interrupt after this call cannot lose
    /// the row, which is the whole point of the design.
    pub fn record(&mut self, row: &Row, prompt: &str) -> std::io::Result<()> {
        writeln!(
            self.prompts,
            "{}",
            serde_json::json!({ "key": row.key, "prompt": prompt })
        )?;
        self.prompts.flush()?;
        writeln!(self.journal, "{}", serde_json::to_string(row)?)?;
        self.journal.flush()?;
        self.journal.sync_data()?;
        self.done.insert(row.key.clone(), row.clone());
        Ok(())
    }

    /// Every recorded row, for reporting and grading.
    pub fn rows(dir: &Path) -> Vec<Row> {
        let Ok(f) = File::open(dir.join("journal.jsonl")) else {
            return Vec::new();
        };
        BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str::<Row>(&l).ok())
            .collect()
    }

    /// The prompt sent for one key — `replay` shows it verbatim so a
    /// grade can be checked against the exact input that produced it.
    pub fn prompt_for(dir: &Path, key: &str) -> Option<String> {
        let f = File::open(dir.join("prompts.jsonl")).ok()?;
        BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l).ok())
            .find(|v| v["key"].as_str() == Some(key))
            .and_then(|v| v["prompt"].as_str().map(String::from))
    }
}
