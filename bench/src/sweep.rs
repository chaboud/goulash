//! Pass B: the sweep.
//!
//! Unit of work is a **scripted session**, not an isolated call: turns
//! replay in order with `ctx_log` accumulating exactly as `session.rs`
//! builds it. That is the only way prefix-cache behavior is observable,
//! and the same run doubles as the quality corpus.

use crate::drive::{GenRequest, MemPos, PromptShape, Think, generate, shape_prompt};
use crate::journal::{Journal, Row, cell_key};
use crate::{Cell, NOW, Step, load_catalog, load_scenarios};
use goulash::engine::{extract_memory_ops, split_answer};
use goulash::memory::MemoryStore;
use goulash::wire::Wire;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::{Duration, Instant};

/// The shipped budgets, **read from the shipped config** rather than
/// mirrored into a constant.
///
/// They used to be a hand-copied `const MAX_TOKENS: usize = 256`, which
/// silently went stale when the product defaults moved and left the
/// sweep measuring a configuration nobody runs. Reading `Config`
/// directly makes that drift impossible.
/// The shipped budget, read from the shipped config rather than mirrored
/// into a constant here.
///
/// It used to be a hand-copied `const MAX_TOKENS: usize = 256`, which
/// went stale when the product default moved and left the sweep
/// measuring a configuration nobody runs.
pub fn budget() -> usize {
    goulash::config::Config::default().engine.max_tokens
}

/// Long-context mode. The shipped budget keeps prompts under ~3k tokens —
/// 36% of an 8192 window — so nothing in the normal corpus tests what
/// happens when a session log actually gets big, which is the steady
/// state of a terminal left open all day. These raise the ceiling so the
/// bulk session can push past 20k tokens.
fn env_usize(k: &str, default: usize) -> usize {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
pub fn num_ctx() -> usize {
    env_usize("GOULASH_BENCH_NUM_CTX", 8192)
}
fn tail_chars() -> usize {
    env_usize("GOULASH_BENCH_TAIL_CHARS", 800)
}
fn context_max_chars() -> usize {
    env_usize("GOULASH_BENCH_CONTEXT_MAX", 12_000)
}
/// Kept for call sites that want the shipped value.
pub const NUM_CTX: usize = 8192;

/// `GOULASH_BENCH_SHAPES=S3` (or `S1,S3`) restricts which prompt shapes
/// run.
///
/// The three-shape cross-product only needs to happen once, on one
/// session — it answers "which shape wins". Pressure-testing a finding
/// across many more situations is a different question, and running it at
/// three shapes would triple the cost to re-answer something already
/// settled.
pub fn shapes() -> Vec<(&'static str, PromptShape)> {
    let all = all_shapes();
    match std::env::var("GOULASH_BENCH_SHAPES") {
        Ok(f) => {
            let want: Vec<String> = f.split(',').map(|s| s.trim().to_uppercase()).collect();
            let kept: Vec<_> = all
                .into_iter()
                .filter(|(n, _)| want.iter().any(|w| w == n))
                .collect();
            eprintln!(
                "shapes restricted to {:?}",
                kept.iter().map(|(n, _)| *n).collect::<Vec<_>>()
            );
            kept
        }
        Err(_) => all,
    }
}

/// Static facts goulash can read without executing anything: `uname`,
/// `$SHELL`, and a `read_dir` of PATH. They go at the FRONT of the stable
/// prefix — static per machine, so cached once for the life of a session
/// and unable to perturb the session-log prefix behind them.
fn platform_line() -> String {
    let os = std::process::Command::new("uname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let shell = std::env::var("SHELL")
        .ok()
        .and_then(|s| s.rsplit('/').next().map(String::from))
        .unwrap_or_else(|| "sh".into());
    if os == "Darwin" {
        format!(
            "Environment: macOS ({os}), BSD userland, {shell} shell. BSD differs \
from GNU: 'du -d N' not '--max-depth', 'sed -i \"\"' not 'sed -i', 'date -v' not \
'date -d', 'stat -f' not 'stat -c'. BSD grep has NO -P.\n\n"
        )
    } else {
        format!("Environment: {os}, GNU userland, {shell} shell.\n\n")
    }
}

const CURATED: &[&str] = &[
    "jq", "yq", "rg", "fd", "ag", "tree", "bat", "delta", "fzf", "gh", "git", "docker", "kubectl",
    "tmux", "curl", "wget", "ffmpeg", "zstd", "pigz", "pv", "rsync", "gsed", "gawk", "ggrep",
    "gdate", "gstat", "gfind", "gtar", "python3", "node", "cargo", "go", "make", "cmake", "tar",
    "unzip",
];

/// Everything on PATH, not a curated slice. ~1700 names, ~3900 tokens —
/// which would be absurd in a volatile prompt but sits in the CACHED
/// prefix, so it costs one fill and is then free for the session. The
/// question this arm asks: when size is nearly free, does completeness
/// beat curation, or does the noise drown the signal?
fn full_path_line() -> String {
    let mut have: Vec<String> = goulash::vendor::path_executable_set().into_iter().collect();
    have.sort();
    format!(
        "Every executable on PATH ({} total): {}.\n\n",
        have.len(),
        have.join(" ")
    )
}

fn tools_line() -> String {
    let have = goulash::vendor::path_executable_set();
    let present: Vec<&str> = CURATED
        .iter()
        .copied()
        .filter(|t| have.contains(*t))
        .collect();
    let absent: Vec<&str> = CURATED
        .iter()
        .copied()
        .filter(|t| !have.contains(*t))
        .collect();
    format!(
        "Installed: {}. NOT installed, never suggest: {}.\n\n",
        present.join(" "),
        absent.join(" ")
    )
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn all_shapes() -> Vec<(&'static str, PromptShape)> {
    let base = goulash::engine::PREAMBLE;
    vec![
        // Shipped shape: memories ahead of the log.
        (
            "S1",
            PromptShape {
                memories: MemPos::BeforeLog,
                command_first: false,
                ..PromptShape::default()
            },
        ),
        // Prices the memory.rs:148 prefix invalidation.
        (
            "S2",
            PromptShape {
                memories: MemPos::Suffix,
                command_first: false,
                ..PromptShape::default()
            },
        ),
        // Command first: latency-to-useful, and whether it costs quality.
        (
            "S3",
            PromptShape {
                memories: MemPos::BeforeLog,
                command_first: true,
                ..PromptShape::default()
            },
        ),
        // S4/S5: situated context, prepended to the cacheable prefix.
        (
            "S4",
            PromptShape {
                divulge: Default::default(),
                memories: MemPos::BeforeLog,
                command_first: true,
                preamble: Some(leak(format!("{}{base}", platform_line()))),
                directive: None,
            },
        ),
        (
            "S5",
            PromptShape {
                divulge: Default::default(),
                memories: MemPos::BeforeLog,
                command_first: true,
                preamble: Some(leak(format!("{}{}{base}", platform_line(), tools_line()))),
                directive: None,
            },
        ),
        // S7: the SAME facts as S5, but delivered as machine-derived
        // memory slots rather than preamble text. Same bytes of content,
        // different framing — memory wraps them in "yours to manage,
        // REMEMBER/FORGET", which may read as more authoritative or may
        // invite the hijacking Pass P found. The shipped preamble is
        // unchanged here; see seed_memory_for().
        (
            "S7",
            PromptShape {
                memories: MemPos::BeforeLog,
                command_first: true,
                ..PromptShape::default()
            },
        ),
        // S6: platform + the ENTIRE PATH set, not a curated subset.
        (
            "S6",
            PromptShape {
                divulge: Default::default(),
                memories: MemPos::BeforeLog,
                command_first: true,
                preamble: Some(leak(format!(
                    "{}{}{base}",
                    platform_line(),
                    full_path_line()
                ))),
                directive: None,
            },
        ),
    ]
}

/// `openai-raw` takes goulash's stable-prefix string verbatim, the same
/// shape ollama gets — the apples-to-apples comparison. `openai-chat`
/// wraps it in the model's template, which is all a hosted provider
/// offers; running both prices that mapping instead of assuming it.
pub fn wire_for(c: &Cell) -> Option<Wire> {
    match c.provider.as_str() {
        "ollama" => Some(Wire::Ollama),
        "openai-raw" => Some(Wire::OpenAi),
        "openai-chat" => Some(Wire::OpenAiChat),
        _ => None,
    }
}

/// The session log, appended exactly as `session.rs` does it.
pub struct SessionLog {
    pub text: String,
}

impl SessionLog {
    pub fn new() -> Self {
        SessionLog {
            text: String::new(),
        }
    }

    pub fn block(&mut self, cmd: &str, exit: i32, hms: &str, tail: &str) {
        self.text
            .push_str(&format!("$ {cmd} [exit {exit}, {hms}]\n"));
        let t: String = tail.chars().take(tail_chars()).collect();
        if !t.trim().is_empty() {
            self.text.push_str(t.trim());
            self.text.push('\n');
        }
        self.epoch_trim();
    }

    pub fn cwd(&mut self, p: &str) {
        self.text.push_str(&format!("[cwd: {p}]\n"));
    }

    pub fn ask(&mut self, q: &str, hms: &str) {
        self.text.push_str(&format!("# {q} [asked {hms}]\n"));
    }

    pub fn answer(&mut self, one_line: &str, cmd: Option<&str>) {
        self.text.push_str(&format!("goulash: {one_line}\n"));
        if let Some(c) = cmd {
            self.text.push_str(&format!("CMD: {c}\n"));
        }
    }

    /// session.rs:1119 — drains from the FRONT at a block boundary. KV
    /// prefix caches are positional, so this costs a full re-eval; the
    /// sweep measures how much.
    fn epoch_trim(&mut self) {
        let cap = context_max_chars();
        if self.text.len() <= cap {
            return;
        }
        let keep = cap / 2;
        let mut start = self.text.len().saturating_sub(keep);
        while !self.text.is_char_boundary(start) {
            start += 1;
        }
        let cut = self.text[start..]
            .find("\n$ ")
            .map(|p| start + p + 1)
            .unwrap_or(start);
        self.text.drain(..cut);
    }
}

fn hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// A live memory store, seeded like a user who has been running goulash
/// for a while.
///
/// This has to be the *real* `MemoryStore`, not a fixed string: its
/// `context_block()` leads with a live `(N/25 slots)` count, so every
/// REMEMBER/FORGET rewrites the block — and under `MemPos::BeforeLog`
/// that sits ahead of the entire session log and invalidates the prefix
/// behind it. Pricing that invalidation is the whole point of the S2 arm,
/// and it cannot be measured against a constant.
///
/// `load(None)` leaves `path` unset, so nothing touches disk.
/// Memory as it stands for a given shape. S7 additionally seeds
/// machine-derived slots, so the situated facts arrive through the store
/// that already exists for durable facts rather than a bespoke preamble.
///
/// Note the deliberate contradiction this creates: slot 1 asserts a
/// preference for `fd`, and the machine slot reports `fd` absent. Whether
/// a model notices is exactly the question.
pub fn seed_memory_for(shape: &str) -> MemoryStore {
    let mut m = seed_memory();
    if shape == "S7" {
        let p = platform_line();
        let t = tools_line();
        let _ = m.add(p.trim(), "machine");
        let _ = m.add(t.trim(), "machine");
    }
    m
}

pub fn seed_memory() -> MemoryStore {
    let mut m = MemoryStore::load(None);
    m.enabled = true;
    let _ = m.add("prefers fd over find", "user");
    let _ = m.add("works mostly in Rust repositories", "user");
    m
}

/// Apply a turn's memory ops exactly as `session.rs` does: forgets first,
/// because a revision is FORGET + REMEMBER in one reply and the delete
/// must free the slot before the add when the store is full.
pub fn apply_memory_ops(store: &mut MemoryStore, remembers: &[String], forgets: &[u64]) {
    for id in forgets {
        store.delete(*id);
    }
    for note in remembers {
        let _ = store.add(note, "llm");
    }
}

/// The unprompted-commentary question, copied from `Engine::ask_proactive`.
const PROACTIVE_Q: &str = "Without being asked, briefly review the most recent \
command and its result — one short observation, tip, or wry aside is always \
welcome. Add a CMD: line ONLY when there is a genuinely useful command the user \
would plausibly run next: most observations need no command, and inventing \
busywork (logging, note-taking, echo) is worse than none. Only if you truly have \
nothing worth saying, reply exactly: PASS";

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(600))
        .build()
}

/// Free every resident model on BOTH engines before the next one loads.
///
/// Unloading only the finished cell's own provider is not enough: at the
/// ollama -> LM Studio boundary the previous engine's model is still
/// resident while the next one loads, and on 24 GB that overlap is the
/// difference between measuring a model and measuring swap. It also
/// covers a cell that died before its unload, and any model left over
/// from an earlier interrupted run.
///
/// Queries what is actually loaded rather than assuming, so nothing
/// depends on the catalog being in a particular order.
pub fn unload_all(agent: &ureq::Agent, ollama_host: &str) {
    if let Ok(resp) = agent
        .get(&format!("{ollama_host}/api/ps"))
        .timeout(Duration::from_secs(3))
        .call()
        && let Ok(text) = resp.into_string()
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
    {
        for m in v["models"].as_array().unwrap_or(&Vec::new()) {
            if let Some(name) = m["name"].as_str().or_else(|| m["model"].as_str()) {
                let body = serde_json::json!({"model": name, "keep_alive": 0});
                let _ = agent
                    .post(&format!("{ollama_host}/api/generate"))
                    .send_string(&body.to_string());
            }
        }
    }
    let lms = format!(
        "{}/.lmstudio/bin/lms",
        std::env::var("HOME").unwrap_or_default()
    );
    let _ = std::process::Command::new(lms)
        .args(["unload", "--all"])
        .output();
}

/// Yield the machine back to whoever else is using it.
///
/// The sweep is a background citizen: it is allowed to take hours, but it
/// is not allowed to make the desktop unusable. Before loading the next
/// model, wait for free memory to recover past a floor. A model load is
/// the moment the footprint jumps, so it is the right place to check —
/// and waiting costs only wall-clock, which this run has plenty of.
///
/// Returns the free percentage it proceeded at, for logging.
pub fn await_headroom(min_free_pct: u64, max_wait: Duration) -> u64 {
    let start = Instant::now();
    let mut waited = false;
    loop {
        let Some(pct) = free_pct() else { return 100 };
        if pct >= min_free_pct || start.elapsed() >= max_wait {
            if waited {
                println!(
                    "    resumed at {pct}% free after {}s",
                    start.elapsed().as_secs()
                );
            }
            return pct;
        }
        if !waited {
            println!("    waiting for memory: {pct}% free, want {min_free_pct}%");
            waited = true;
        }
        std::thread::sleep(Duration::from_secs(10));
    }
}

/// System-wide free memory percentage, via `memory_pressure`. None if the
/// tool is unavailable (non-macOS), in which case the caller proceeds.
fn free_pct() -> Option<u64> {
    let out = std::process::Command::new("memory_pressure")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find(|l| l.contains("free percentage"))
        .and_then(|l| l.rsplit(':').next())
        .and_then(|v| v.trim().trim_end_matches('%').parse().ok())
}

/// Pin an LM Studio model's context length before its cells run.
///
/// `num_ctx` is an ollama option; the OpenAI request body has no
/// equivalent, so on LM Studio it is simply not sent and each model loads
/// at whatever context its own saved config specifies. Measured:
/// `qwen/qwen3-1.7b` defaults to 8192 (matching what goulash asks for)
/// while `google/gemma-4-e4b` defaults to **118272** — a KV cache large
/// enough to dominate a 24 GB machine, and not remotely the same
/// experiment as the ollama cells running at 8192.
///
/// Loading explicitly makes the comparison honest and the memory
/// bounded. Best-effort: if `lms` is missing the cell still runs, just
/// at the model's default.
pub fn preload_lmstudio(model: &str, num_ctx: usize, keep_alive: &str) {
    let lms = format!(
        "{}/.lmstudio/bin/lms",
        std::env::var("HOME").unwrap_or_default()
    );
    let ttl = keep_alive.trim_end_matches(['m', 's', 'h']);
    let secs = match keep_alive.chars().last() {
        Some('h') => ttl.parse::<u64>().unwrap_or(1) * 3600,
        Some('s') => ttl.parse::<u64>().unwrap_or(60),
        _ => ttl.parse::<u64>().unwrap_or(3) * 60,
    };
    let out = std::process::Command::new(&lms)
        .args([
            "load",
            model,
            "--context-length",
            &num_ctx.to_string(),
            "--ttl",
            &secs.to_string(),
            "-y",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            println!("    preloaded {model} at ctx={num_ctx}");
        }
        Ok(o) => eprintln!(
            "    ! preload {model} failed: {}",
            String::from_utf8_lossy(&o.stderr)
                .lines()
                .next()
                .unwrap_or("")
        ),
        Err(e) => eprintln!("    ! preload {model}: {e}"),
    }
}

pub fn mechanical(raw: &str, text: &str, command: &Option<String>) -> (bool, bool, usize, bool) {
    let empty = text.trim().is_empty() && command.is_none();
    let fenced = raw.contains("```");
    let prose_lines = raw
        .lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with("CMD:")
                && !l.starts_with("REMEMBER:")
                && !l.starts_with("FORGET:")
        })
        .count();
    let has_cmd_tag = raw.lines().any(|l| l.trim().starts_with("CMD:"));
    (empty, fenced, prose_lines, has_cmd_tag)
}

#[allow(clippy::too_many_arguments)]
pub fn run_one(
    j: &mut Journal,
    wire: Wire,
    agent: &ureq::Agent,
    cell: &Cell,
    pass: &str,
    shape_name: &str,
    shape: &PromptShape,
    step_id: &str,
    turn_index: usize,
    question: &str,
    proactive: bool,
    log: &str,
    memories: &str,
    paths: &std::collections::HashSet<String>,
    stop: &[String],
    think: Think,
    max_tokens: usize,
) -> Option<(String, Option<String>, Vec<String>, Vec<u64>)> {
    let key = cell_key(pass, &cell.provider, &cell.model, shape_name, step_id);
    let prompt = shape_prompt(shape, memories, log, question, NOW, proactive);
    let req = GenRequest {
        model: cell.model.clone(),
        prompt: prompt.clone(),
        stream: true,
        temperature: 0.2,
        max_tokens,
        num_ctx: num_ctx(),
        stop: stop.to_vec(),
        think,
        // Deliberately short. Residency only has to outlast one cell's
        // turns — the sweep unloads explicitly when it moves on. A long
        // keep_alive means an interrupted run strands a multi-GB model in
        // memory for the rest of that window, on a machine someone is
        // still trying to use.
        keep_alive: "3m".to_string(),
    };

    let outcome = generate(agent, &cell.host, wire, &req, &mut |_| {});
    let mut row = Row {
        key: key.clone(),
        pass: pass.to_string(),
        provider: cell.provider.clone(),
        model: cell.model.clone(),
        tier: cell.tier.clone(),
        shape: shape_name.to_string(),
        step: step_id.to_string(),
        turn_index,
        question: question.to_string(),
        prompt_chars: prompt.len(),
        prompt_hash: hash(&prompt),
        at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        ..Row::default()
    };

    let parsed = match outcome {
        Ok((raw, stats)) => {
            let (rest, remembers, forgets) = extract_memory_ops(&raw);
            let (text, command) = split_answer(&rest, paths);
            let (empty, fenced, prose_lines, has_cmd_tag) = mechanical(&raw, &text, &command);
            row.raw = raw;
            row.text = text.clone();
            row.command = command.clone();
            row.remembers = remembers.clone();
            row.forgets = forgets.clone();
            row.ttft_ms = stats.ttft_ms;
            row.total_ms = stats.total_ms;
            row.load_ms = stats.load_ms;
            row.prompt_tokens = stats.prompt_tokens;
            row.prompt_eval_ms = stats.prompt_eval_ms;
            row.eval_tokens = stats.eval_tokens;
            row.eval_ms = stats.eval_ms;
            row.reasoning_tokens = stats.reasoning_tokens;
            row.stop_reason = stats.stop_reason;
            row.empty = empty;
            row.fenced = fenced;
            row.prose_lines = prose_lines;
            row.has_cmd_tag = has_cmd_tag;
            Some((text, command, remembers, forgets))
        }
        Err(e) => {
            row.error = Some(e);
            row.empty = true;
            None
        }
    };

    if let Err(e) = j.record(&row, &prompt) {
        eprintln!("  ! journal write failed: {e}");
    }
    parsed
}

/// Replay one scripted session for one (cell, shape).
fn run_session(
    j: &mut Journal,
    cell: &Cell,
    shape_name: &'static str,
    shape: &PromptShape,
    steps: &[Step],
    agent: &ureq::Agent,
    paths: &std::collections::HashSet<String>,
) {
    let Some(wire) = wire_for(cell) else {
        // Reported, never silent. A cell that produced no rows because
        // the build cannot reach it looks identical, in a summary, to a
        // cell that produced no rows because everything failed.
        eprintln!(
            "    skipping {} ({}) — this build has no chat path; \
             wire.rs targets /v1/completions",
            cell.model, cell.provider
        );
        return;
    };
    // Match the shipped 0.4.0 defaults: no stop sequence, and reasoning
    // gets its own allowance. The original sweep ran with stop:["\n\n"]
    // and no allowance, which starved every reasoning model — the three
    // cells that scored 0.00 in QUALITY.md were that configuration, not
    // their capability.
    let stop: Vec<String> = Vec::new();
    let mut log = SessionLog::new();
    let mut memory = seed_memory_for(shape_name);

    for (i, step) in steps.iter().enumerate() {
        match step.kind.as_str() {
            // Log-only: no model call, so it is replayed on resume for free.
            "block" => log.block(
                step.cmd.as_deref().unwrap_or(""),
                step.exit.unwrap_or(0),
                &step.hms,
                step.tail.as_deref().unwrap_or(""),
            ),
            "cwd" => log.cwd(step.cwd.as_deref().unwrap_or("")),
            "ask" | "proactive" => {
                let proactive = step.kind == "proactive";
                let question = if proactive {
                    PROACTIVE_Q.to_string()
                } else {
                    step.ask.clone().unwrap_or_default()
                };
                if !proactive {
                    log.ask(&question, &step.hms);
                }
                let key = cell_key("pass-b", &cell.provider, &cell.model, shape_name, &step.id);
                if let Some(prev) = j.recorded(&key) {
                    // Replay the recorded answer's effect on the log AND on
                    // the memory store, so later turns build byte-identical
                    // prompts to the original run. Skipping the memory ops
                    // would leave a resumed session with a different slot
                    // count — which is exactly the string S2 exists to
                    // measure, silently changed.
                    apply_memory_ops(&mut memory, &prev.remembers, &prev.forgets);
                    let one_line = prev.text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !prev.empty
                        && !one_line
                            .trim_matches(['.', '!'])
                            .eq_ignore_ascii_case("PASS")
                    {
                        log.answer(&one_line, prev.command.as_deref());
                    }
                    continue;
                }
                let parsed = run_one(
                    j,
                    wire,
                    agent,
                    cell,
                    "pass-b",
                    shape_name,
                    shape,
                    &step.id,
                    i,
                    &question,
                    proactive,
                    &log.text,
                    &memory.context_block(),
                    paths,
                    &stop,
                    Think::Off,
                    budget(),
                );
                if let Some((text, command, remembers, forgets)) = parsed {
                    apply_memory_ops(&mut memory, &remembers, &forgets);
                    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !one_line
                        .trim_matches(['.', '!'])
                        .eq_ignore_ascii_case("PASS")
                    {
                        log.answer(&one_line, command.as_deref());
                    }
                }
            }
            other => eprintln!("  ! unknown step kind {other:?}"),
        }
    }
}

pub fn run(dir: &Path) -> std::io::Result<()> {
    let catalog = load_catalog();
    let scenarios = load_scenarios();
    let mut j = Journal::open(dir)?;
    let agent = agent();
    let paths = goulash::vendor::path_executable_set();
    let shapes = shapes();

    let asks: Vec<&Step> = scenarios
        .step
        .iter()
        .filter(|s| s.kind == "ask" || s.kind == "proactive")
        .collect();
    let mut sessions: Vec<String> = scenarios.step.iter().map(|s| s.session.clone()).collect();
    sessions.sort();
    sessions.dedup();
    let mut planned: Vec<String> = Vec::new();
    for c in &catalog.cell {
        for (sn, _) in &shapes {
            for s in &asks {
                planned.push(cell_key("pass-b", &c.provider, &c.model, sn, &s.id));
            }
        }
    }
    j.write_manifest(&planned)?;
    let total = planned.len();
    println!(
        "pass-b: {} cells x {} shapes x {} asks across {} session(s) = {total} generations ({} already done)",
        catalog.cell.len(),
        shapes.len(),
        asks.len(),
        sessions.len(),
        j.completed_in("pass-b")
    );

    // Model-major: one model resident at a time. On 24 GB, interleaving
    // measures memory pressure instead of models.
    for cell in &catalog.cell {
        let mut remaining = 0;
        for (sn, _) in &shapes {
            for s in &asks {
                if !j.is_done(&cell_key("pass-b", &cell.provider, &cell.model, sn, &s.id)) {
                    remaining += 1;
                }
            }
        }
        if remaining == 0 {
            println!("  [skip] {} ({}) — complete", cell.model, cell.provider);
            continue;
        }
        println!(
            "  {} ({}, {:.1} GB{}) — {remaining} to go",
            cell.model,
            cell.provider,
            cell.gb,
            if cell.contends { ", contends" } else { "" }
        );
        await_headroom(15, Duration::from_secs(900));
        if cell.provider.starts_with("openai") {
            preload_lmstudio(&cell.model, NUM_CTX, "3m");
        }
        for (shape_name, shape) in &shapes {
            // Each session replays independently: fresh log, fresh memory
            // store. Sharing them across contexts would make later
            // sessions carry irrelevant history and quietly change what
            // is being measured.
            for sess in &sessions {
                let steps: Vec<Step> = scenarios
                    .step
                    .iter()
                    .filter(|s| &s.session == sess)
                    .cloned()
                    .collect();
                run_session(&mut j, cell, shape_name, shape, &steps, &agent, &paths);
            }
        }
        unload_all(&agent, "http://127.0.0.1:11434");
        println!("    done ({}/{} overall)", j.completed_in("pass-b"), total);
    }
    println!("pass-b complete: {}/{}", j.completed_in("pass-b"), total);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the entire S2 arm rests on: once a memory op lands,
    /// S1 must lose its cached prefix while S2 keeps it. If this fails,
    /// the S2 measurement is meaningless — which is exactly what happened
    /// when memories were a fixed constant.
    #[test]
    fn memory_mutation_invalidates_s1_prefix_but_not_s2() {
        let log = "$ ls [exit 0, 09:00:00]\nCargo.toml src\n";
        let mut mem = seed_memory();
        let before = mem.context_block();
        apply_memory_ops(&mut mem, &["prefers ripgrep".to_string()], &[]);
        let after = mem.context_block();
        assert_ne!(before, after, "a REMEMBER must change the block");
        assert!(
            after.contains("3/25 slots"),
            "the slot count is what makes it volatile, got: {}",
            after.lines().next().unwrap_or("")
        );

        let shared =
            |a: &str, b: &str| a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();

        let s1 = PromptShape {
            memories: MemPos::BeforeLog,
            command_first: false,
            ..PromptShape::default()
        };
        let a1 = build_prompt(&s1, &before, log, "q", NOW, false);
        let b1 = build_prompt(&s1, &after, log, "q", NOW, false);
        assert!(
            shared(&a1, &b1) < a1.find("Session log").unwrap(),
            "S1 must diverge BEFORE the session log — that is the cost"
        );

        let s2 = PromptShape {
            memories: MemPos::Suffix,
            command_first: false,
            ..PromptShape::default()
        };
        let a2 = build_prompt(&s2, &before, log, "q", NOW, false);
        let b2 = build_prompt(&s2, &after, log, "q", NOW, false);
        assert!(
            shared(&a2, &b2) > a2.find("Current local time").unwrap(),
            "S2 must keep the whole log in its shared prefix"
        );
    }

    /// Forgets before adds, so a revision (FORGET + REMEMBER in one reply)
    /// frees its slot before refilling it — matching session.rs.
    #[test]
    fn forgets_apply_before_adds() {
        let mut mem = seed_memory();
        mem.set_limit(2);
        apply_memory_ops(&mut mem, &["replacement note".to_string()], &[1]);
        let texts: Vec<&str> = mem.slots.iter().map(|s| s.text.as_str()).collect();
        assert!(!texts.contains(&"prefers fd over find"), "slot 1 forgotten");
        assert!(
            texts.contains(&"replacement note"),
            "add succeeded because the delete freed the slot first, got {texts:?}"
        );
    }

    /// Resume must reach the same memory state, or later prompts differ.
    #[test]
    fn replayed_ops_reproduce_the_same_block() {
        let ops = [
            (vec!["one".to_string()], vec![]),
            (vec!["two".to_string()], vec![1u64]),
        ];
        let build = || {
            let mut m = seed_memory();
            for (rem, forg) in &ops {
                apply_memory_ops(&mut m, rem, forg);
            }
            m.context_block()
        };
        assert_eq!(build(), build(), "replay must be deterministic");
    }
}
