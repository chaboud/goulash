//! Pass B: the sweep.
//!
//! Unit of work is a **scripted session**, not an isolated call: turns
//! replay in order with `ctx_log` accumulating exactly as `session.rs`
//! builds it. That is the only way prefix-cache behavior is observable,
//! and the same run doubles as the quality corpus.

use crate::journal::{Journal, Row, cell_key};
use crate::{Cell, MEMORIES, NOW, Step, load_catalog, load_scenarios};
use goulash::engine::{
    GenRequest, MemPos, Ollama, OpenAiCompat, PromptShape, Provider, build_prompt,
    extract_memory_ops, split_answer,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Duration;

/// Shipped defaults, mirrored from `engine::DEFAULT_*`.
const MAX_TOKENS: usize = 256;
const NUM_CTX: usize = 8192;
const TAIL_CHARS: usize = 800;
const CONTEXT_MAX_CHARS: usize = 12_000;

pub fn shapes() -> Vec<(&'static str, PromptShape)> {
    vec![
        // Shipped shape: memories ahead of the log.
        (
            "S1",
            PromptShape {
                memories: MemPos::BeforeLog,
                command_first: false,
            },
        ),
        // Prices the memory.rs:148 prefix invalidation.
        (
            "S2",
            PromptShape {
                memories: MemPos::Suffix,
                command_first: false,
            },
        ),
        // Command first: latency-to-useful, and whether it costs quality.
        (
            "S3",
            PromptShape {
                memories: MemPos::BeforeLog,
                command_first: true,
            },
        ),
    ]
}

/// `openai-raw` takes goulash's stable-prefix string verbatim, the same
/// shape ollama gets — the apples-to-apples comparison. `openai-chat`
/// wraps it in the model's chat template, which is all hosted providers
/// offer; running both prices that mapping instead of assuming it's free.
pub fn provider_for(c: &Cell) -> Box<dyn Provider> {
    match c.provider.as_str() {
        "ollama" => Box::new(Ollama),
        "openai-raw" => Box::new(OpenAiCompat {
            chat: false,
            suppress_reasoning: false,
        }),
        _ => Box::new(OpenAiCompat {
            chat: true,
            // Off by evidence: it empties `content` on LM Studio + qwen3.
            suppress_reasoning: false,
        }),
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
        self.text.push_str(&format!("$ {cmd} [exit {exit}, {hms}]\n"));
        let t: String = tail.chars().take(TAIL_CHARS).collect();
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
        if self.text.len() <= CONTEXT_MAX_CHARS {
            return;
        }
        let keep = CONTEXT_MAX_CHARS / 2;
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

/// Free a model's memory before the next one loads. On 24 GB this is the
/// difference between measuring a model and measuring swap.
pub fn unload(cell: &Cell, agent: &ureq::Agent) {
    match cell.provider.as_str() {
        "ollama" => {
            let body = serde_json::json!({"model": cell.model, "keep_alive": 0});
            let _ = agent
                .post(&format!("{}/api/generate", cell.host))
                .send_string(&body.to_string());
        }
        _ => {
            let lms = format!(
                "{}/.lmstudio/bin/lms",
                std::env::var("HOME").unwrap_or_default()
            );
            let _ = std::process::Command::new(lms)
                .args(["unload", "--all"])
                .output();
        }
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
    provider: &dyn Provider,
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
    paths: &std::collections::HashSet<String>,
    stop: &[String],
    think: Option<bool>,
    max_tokens: usize,
) -> Option<(String, Option<String>)> {
    let key = cell_key(pass, &cell.provider, &cell.model, shape_name, step_id);
    let prompt = build_prompt(shape, MEMORIES, log, question, NOW, proactive);
    let req = GenRequest {
        model: cell.model.clone(),
        prompt: prompt.clone(),
        stream: true,
        temperature: 0.2,
        max_tokens,
        num_ctx: NUM_CTX,
        stop: stop.to_vec(),
        think,
        // Deliberately short. Residency only has to outlast one cell's
        // turns — the sweep unloads explicitly when it moves on. A long
        // keep_alive means an interrupted run strands a multi-GB model in
        // memory for the rest of that window, on a machine someone is
        // still trying to use.
        keep_alive: "3m".to_string(),
    };

    let outcome = provider.generate(agent, &cell.host, &req, &mut |_| {});
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
            row.remembers = remembers;
            row.forgets = forgets;
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
            Some((text, command))
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
    let provider = provider_for(cell);
    let stop: Vec<String> = vec!["\n\n".to_string()];
    let mut log = SessionLog::new();

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
                    // Replay the recorded answer's effect on the log so
                    // later turns build byte-identical prompts to the
                    // original run. Without this, resuming silently
                    // changes the thing being measured.
                    let one_line = prev.text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !prev.empty
                        && !one_line.trim_matches(['.', '!']).eq_ignore_ascii_case("PASS")
                    {
                        log.answer(&one_line, prev.command.as_deref());
                    }
                    continue;
                }
                let parsed = run_one(
                    j,
                    provider.as_ref(),
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
                    paths,
                    &stop,
                    Some(false),
                    MAX_TOKENS,
                );
                if let Some((text, command)) = parsed {
                    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !one_line.trim_matches(['.', '!']).eq_ignore_ascii_case("PASS") {
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
        "pass-b: {} cells x {} shapes x {} asks = {total} generations ({} already done)",
        catalog.cell.len(),
        shapes.len(),
        asks.len(),
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
        for (shape_name, shape) in &shapes {
            run_session(
                &mut j,
                cell,
                shape_name,
                shape,
                &scenarios.step,
                &agent,
                &paths,
            );
        }
        unload(cell, &agent);
        println!("    done ({}/{} overall)", j.completed_in("pass-b"), total);
    }
    println!("pass-b complete: {}/{}", j.completed_in("pass-b"), total);
    Ok(())
}
