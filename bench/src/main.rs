//! Engine characterization harness.
//!
//! Drives goulash's *real* engine — `build_prompt`, the `Provider` impls,
//! `split_answer` — so results describe the product rather than a
//! reimplementation of it. Everything is resumable: see `journal.rs`.
//!
//!   goulash-bench pass-a [DIR]      tolerance probe, prunes broken cells
//!   goulash-bench pass-b [DIR]      the sweep: shapes x scenarios x cells
//!   goulash-bench pass-p [DIR]      prompt-wording variants on the failure cases
//!   goulash-bench pass-t [DIR]      thinking vs display budget: what does reasoning cost?
//!   goulash-bench report [DIR]      report card from whatever has landed
//!   goulash-bench blind  [DIR]      shuffled corpus for grading
//!   goulash-bench replay [DIR] KEY  exact prompt + raw response for one cell

mod journal;
mod probe;
mod report;
mod sweep;
mod thinking;
mod variants;

use serde::Deserialize;
use std::path::PathBuf;

/// Frozen clock. `Current local time:` sits in the volatile suffix, so a
/// live clock would make two runs of the same cell produce different
/// bytes and drift every latency comparison.
pub const NOW: &str = "2026-07-28 09:00:00";

// Memories come from a live `MemoryStore` (see `sweep::seed_memory`), not
// a constant. An earlier revision pinned them to a fixed string, which
// silently neutered the S2 arm: with the block never changing there is no
// prefix invalidation to price, and S2-vs-S1 degenerates into comparing
// two static prefixes.

#[derive(Deserialize, Clone, Debug)]
pub struct Cell {
    pub model: String,
    pub provider: String,
    pub host: String,
    pub tier: String,
    pub gb: f64,
    /// Recorded in the report rather than hidden: on a 24 GB box these
    /// partly measure memory pressure, not the model.
    #[serde(default)]
    pub contends: bool,
}

#[derive(Deserialize)]
pub struct Catalog {
    pub cell: Vec<Cell>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Step {
    pub kind: String,
    /// Which scripted session this step belongs to. Steps are grouped by
    /// session and each session replays with its own fresh log and memory
    /// store, so one corpus can cover several distinct work contexts
    /// (a rust repo, a data-wrangling session, git surgery) instead of
    /// one long implausible one. Defaults to "main" so existing step ids
    /// — and therefore existing journal keys — are unchanged.
    #[serde(default = "default_session")]
    pub session: String,
    #[serde(default)]
    pub id: String,
    pub hms: String,
    #[serde(default)]
    pub cmd: Option<String>,
    #[serde(default)]
    pub exit: Option<i32>,
    #[serde(default)]
    pub tail: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub ask: Option<String>,
}

fn default_session() -> String {
    "main".to_string()
}

#[derive(Deserialize)]
pub struct Scenarios {
    pub step: Vec<Step>,
}

fn base() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `GOULASH_BENCH_ONLY=substr[,substr]` narrows the catalog — for smoke
/// tests, for rerunning a single model that failed, and for splitting a
/// long sweep across sittings.
pub fn load_catalog() -> Catalog {
    let p = base().join("catalog.toml");
    let text =
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    let mut cat: Catalog =
        toml::from_str(&text).unwrap_or_else(|e| panic!("parse catalog.toml: {e}"));
    // Footprint ceiling. This box has 24 GB shared with a desktop, and a
    // 14 GB model does not just measure slowly — it makes the machine
    // unusable while it runs.
    if let Ok(cap) = std::env::var("GOULASH_BENCH_MAX_GB") {
        if let Ok(max) = cap.parse::<f64>() {
            let before = cat.cell.len();
            cat.cell.retain(|c| c.gb <= max);
            eprintln!(
                "catalog capped at {max} GB: {} of {before} cell(s) kept",
                cat.cell.len()
            );
        }
    }
    if let Ok(filter) = std::env::var("GOULASH_BENCH_ONLY") {
        let pats: Vec<String> = filter
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        cat.cell.retain(|c| {
            let hay = format!("{} {} {}", c.model, c.provider, c.tier).to_lowercase();
            pats.iter().any(|p| hay.contains(p))
        });
        eprintln!("catalog filtered to {} cell(s) by {filter:?}", cat.cell.len());
    }
    cat
}

pub fn load_scenarios() -> Scenarios {
    let p = base().join("scenarios.toml");
    let text = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parse scenarios.toml: {e}"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let dir = args
        .get(1)
        .filter(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .unwrap_or_else(|| base().join("results/latest"));

    let result = match cmd {
        "pass-a" => probe::run(&dir),
        "pass-b" => sweep::run(&dir),
        "report" => report::run(&dir),
        "blind" => report::blind(&dir),
        "grade" => report::grade(&dir),
        "pass-p" => variants::run(&dir),
        "pass-t" => thinking::run(&dir),
        "replay" => match args.get(2) {
            Some(key) => report::replay(&dir, key),
            None => {
                eprintln!("replay needs a cell key");
                std::process::exit(2)
            }
        },
        _ => {
            println!(
                "usage: goulash-bench <pass-a|pass-b|pass-p|pass-t|report|blind|grade|replay> [DIR] [KEY]\n\
                 \n\
                 All passes are resumable: rerun the same command and only\n\
                 cells missing from journal.jsonl are executed."
            );
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("goulash-bench: {e}");
        std::process::exit(1);
    }
}
