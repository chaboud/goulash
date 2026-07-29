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
    /// Unix seconds when the row landed. Not used in any measurement —
    /// it exists so a run can be audited after the fact (e.g. "which rows
    /// overlapped that other process?"), which was impossible when the
    /// journal carried no wall-clock at all.
    pub at: u64,

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
    ///
    /// Takes an exclusive lock. Two sweeps running at once contend for the
    /// same GPU and unload each other's models between cells, so every
    /// latency number either of them records is garbage — and the damage
    /// is silent, which is worse. Learned the hard way.
    pub fn open(dir: &Path) -> std::io::Result<Journal> {
        std::fs::create_dir_all(dir)?;
        Self::acquire_lock(dir)?;
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

    /// Refuse to start if another sweep holds this directory. A stale lock
    /// from a killed run is reclaimed — the pid is checked, not trusted.
    fn acquire_lock(dir: &Path) -> std::io::Result<()> {
        let lock = dir.join("sweep.lock");
        if let Ok(text) = std::fs::read_to_string(&lock)
            && let Ok(pid) = text.trim().parse::<i32>()
            && pid != std::process::id() as i32
            // SAFETY: kill(pid, 0) only tests for existence.
            && unsafe { libc::kill(pid, 0) } == 0
        {
            return Err(std::io::Error::other(format!(
                "another sweep (pid {pid}) is running in {}. \
                 Two sweeps contend for the GPU and unload each other's \
                 models, so both sets of timings would be worthless. \
                 Stop it first, or use a different results directory.",
                dir.display()
            )));
        }
        std::fs::write(&lock, std::process::id().to_string())
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
    ///
    /// Merges rather than overwrites. A filtered invocation (a memory cap,
    /// a single-model rerun) plans fewer cells than full coverage, and
    /// truncating the manifest to that subset would quietly redefine
    /// "complete" as whatever the last narrow run happened to cover.
    pub fn write_manifest(&self, keys: &[String]) -> std::io::Result<()> {
        let path = self.dir.join("manifest.jsonl");
        let mut all: Vec<String> = Vec::new();
        if let Ok(f) = File::open(&path) {
            all.extend(
                BufReader::new(f)
                    .lines()
                    .map_while(Result::ok)
                    .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l).ok())
                    .filter_map(|v| v["key"].as_str().map(String::from)),
            );
        }
        all.extend(keys.iter().cloned());
        all.sort();
        all.dedup();
        let mut f = File::create(&path)?;
        for k in &all {
            writeln!(f, "{}", serde_json::json!({ "key": k }))?;
        }
        f.sync_all()
    }

    /// Planned cells still missing — the honest answer to "is this done?"
    /// even after a run that was narrowed by a filter.
    pub fn outstanding(dir: &Path) -> Vec<String> {
        let done: HashMap<String, Row> = Self::completed_keys(&dir.join("journal.jsonl"));
        let Ok(f) = File::open(dir.join("manifest.jsonl")) else {
            return Vec::new();
        };
        BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l).ok())
            .filter_map(|v| v["key"].as_str().map(String::from))
            .filter(|k| !done.contains_key(k))
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, text: &str) -> Row {
        Row {
            key: key.to_string(),
            pass: "pass-b".into(),
            text: text.into(),
            command: Some("ls -S".into()),
            ..Row::default()
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("goulash-bench-test-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// The core resume guarantee: rows survive a process boundary and a
    /// reopened journal knows exactly what already landed.
    #[test]
    fn rows_survive_reopen() {
        let d = tmp("reopen");
        {
            let mut j = Journal::open(&d).unwrap();
            j.record(&row("a/1", "first"), "PROMPT-1").unwrap();
            j.record(&row("a/2", "second"), "PROMPT-2").unwrap();
        } // journal dropped: simulates the process dying here

        let j = Journal::open(&d).unwrap();
        assert!(j.is_done("a/1") && j.is_done("a/2"));
        assert!(!j.is_done("a/3"), "unseen key must not be claimed done");
        assert_eq!(j.completed_in("pass-b"), 2);
        assert_eq!(j.completed_in("pass-a"), 0, "counts must be per pass");
    }

    /// Resume must recover the recorded *answer*, not just the key —
    /// the session log is rebuilt from it, and a wrong replay silently
    /// changes every prompt built afterwards.
    #[test]
    fn resume_recovers_answer_text_for_log_replay() {
        let d = tmp("replay");
        {
            let mut j = Journal::open(&d).unwrap();
            j.record(&row("a/1", "disk is mostly target/"), "P").unwrap();
        }
        let j = Journal::open(&d).unwrap();
        let prev = j.recorded("a/1").expect("row must be recoverable");
        assert_eq!(prev.text, "disk is mostly target/");
        assert_eq!(prev.command.as_deref(), Some("ls -S"));
    }

    /// A hard kill can leave a half-written final line. That row must be
    /// treated as absent and redone, not silently skipped.
    #[test]
    fn torn_final_line_is_redone_not_skipped() {
        let d = tmp("torn");
        {
            let mut j = Journal::open(&d).unwrap();
            j.record(&row("a/1", "complete"), "P").unwrap();
        }
        // Simulate a truncated write of a second row.
        let mut f = OpenOptions::new()
            .append(true)
            .open(d.join("journal.jsonl"))
            .unwrap();
        write!(f, "{{\"key\":\"a/2\",\"pass\":\"pass-b\",\"tex").unwrap();
        drop(f);

        let j = Journal::open(&d).unwrap();
        assert!(j.is_done("a/1"), "intact row still counts");
        assert!(!j.is_done("a/2"), "torn row must be redone");
    }

    /// Re-recording a key (a redone cell) must not double-count it.
    #[test]
    fn rerecording_a_key_is_idempotent_for_counting() {
        let d = tmp("idem");
        let mut j = Journal::open(&d).unwrap();
        j.record(&row("a/1", "v1"), "P").unwrap();
        j.record(&row("a/1", "v2"), "P").unwrap();
        assert_eq!(j.completed_in("pass-b"), 1);
        assert_eq!(j.recorded("a/1").unwrap().text, "v2", "latest wins");
    }

    #[test]
    fn prompts_are_recoverable_for_audit() {
        let d = tmp("prompts");
        {
            let mut j = Journal::open(&d).unwrap();
            j.record(&row("a/1", "x"), "THE EXACT PROMPT").unwrap();
        }
        assert_eq!(
            Journal::prompt_for(&d, "a/1").as_deref(),
            Some("THE EXACT PROMPT")
        );
    }
}
