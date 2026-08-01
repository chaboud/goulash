//! Report card, blind corpus, and single-cell replay.
//!
//! The report separates what is measured from what is judged. Everything
//! here is mechanical and reproduces on a rerun; qualitative scores live
//! in `grades.jsonl` and are joined in only after blind grading.

use crate::journal::{Journal, Row};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::Path;

/// How a row actually turned out.
///
/// Recomputed from stored fields rather than trusting `Row::empty`, which
/// conflates two different outcomes: `mistral-nemo` replies to ordinary
/// questions with a bare `REMEMBER:` line, which `extract_memory_ops`
/// strips — leaving no prose and no command, but a memory write. Goulash
/// itself does *not* treat that as an error (engine.rs checks `remembers`
/// too), so the user sees nothing and silently gains a memory.
#[derive(PartialEq, Clone, Copy)]
pub enum Outcome {
    Answered,
    /// Nothing but a memory write — the prompt's memory block hijacked it.
    MemoryOnly,
    /// Truncated by the stop sequence before emitting anything.
    EmptyByStop,
    /// Spent the whole budget without emitting content (thinking).
    EmptyByBudget,
    Empty,
    Error,
}

pub fn outcome(r: &Row) -> Outcome {
    if r.error.is_some() {
        return Outcome::Error;
    }
    let nothing = r.text.trim().is_empty() && r.command.is_none();
    if !nothing {
        return Outcome::Answered;
    }
    if !r.remembers.is_empty() || !r.forgets.is_empty() {
        return Outcome::MemoryOnly;
    }
    match r.stop_reason.as_deref() {
        // Budget exhausted with nothing to show: reasoning ate it.
        Some("length") => Outcome::EmptyByBudget,
        // A handful of tokens then the stop sequence fired: the model
        // opened with a blank line and got guillotined.
        Some("stop") if r.eval_tokens.unwrap_or(0) < 32 => Outcome::EmptyByStop,
        _ => Outcome::Empty,
    }
}

/// Did the budget cut the command off mid-flight?
///
/// Computed from stored fields rather than a new column, so it applies to
/// rows already on disk. `stop_reason == "length"` alone is not enough —
/// a model can hit the ceiling *after* a complete command while still
/// rambling prose. These are the signatures of a command that stops
/// mid-token: unbalanced quoting, a dangling pipe/continuation, or a
/// trailing bare flag.
pub fn command_truncated(r: &Row) -> bool {
    let Some(cmd) = r.command.as_deref() else {
        return false;
    };
    let c = cmd.trim_end();
    if c.is_empty() {
        return false;
    }
    let odd = |ch: char| c.matches(ch).count() % 2 == 1;
    odd('\'')
        || odd('"')
        || c.ends_with('|')
        || c.ends_with('\\')
        || c.ends_with("&&")
        || c.ends_with('-')
        || c.matches('(').count() != c.matches(')').count()
        // Hit the ceiling with no terminal punctuation at all.
        || (r.stop_reason.as_deref() == Some("length") && !c.ends_with(['\'', '"', ';']))
}

fn median(mut v: Vec<u64>) -> Option<u64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

fn pct(n: usize, d: usize) -> String {
    if d == 0 {
        "-".into()
    } else {
        format!("{:.0}%", 100.0 * n as f64 / d as f64)
    }
}

/// Deterministic id for blind grading — stable across runs so a grade
/// stays attached to its answer.
fn blind_id(key: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in key.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:06x}", h & 0xffffff)
}

pub fn run(dir: &Path) -> std::io::Result<()> {
    let rows = Journal::rows(dir);
    if rows.is_empty() {
        println!("no results in {}", dir.display());
        return Ok(());
    }
    let sweep: Vec<&Row> = rows.iter().filter(|r| r.pass == "pass-b").collect();
    let mut out = String::new();
    out.push_str("# Engine characterization — report card\n\n");
    out.push_str(&format!(
        "{} rows recorded ({} sweep). Mechanical metrics only; qualitative\n\
         scores are joined from grades.jsonl after blind grading.\n\n",
        rows.len(),
        sweep.len()
    ));

    // ---- per model
    out.push_str("## Per model\n\n");
    out.push_str(
        "`empty→stop` = truncated by the stop sequence before emitting anything.\n\
         `empty→budget` = spent the whole budget thinking. `mem-only` = replied\n\
         with a bare REMEMBER: line instead of answering. Load time is excluded\n\
         from latency (the first call per model pays it).\n\n\
         | model | provider | tier | p50 ttft | p50 total | answered | empty→stop | empty→budget | mem-only | fenced | 1-line | CMD: | reasoning tok |\n\
         |---|---|---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    let mut by_model: BTreeMap<(String, String), Vec<&Row>> = BTreeMap::new();
    for r in &sweep {
        by_model
            .entry((r.model.clone(), r.provider.clone()))
            .or_default()
            .push(r);
    }
    for ((model, provider), rs) in &by_model {
        let n = rs.len();
        let ok: Vec<&&Row> = rs.iter().filter(|r| r.error.is_none()).collect();
        // Subtract model load: the first call per model pays it, and
        // leaving it in makes a cold cell look like a slow one.
        let ttft = median(
            ok.iter()
                .filter_map(|r| r.ttft_ms.map(|t| t.saturating_sub(r.load_ms.unwrap_or(0))))
                .collect(),
        );
        let total = median(
            ok.iter()
                .map(|r| r.total_ms.saturating_sub(r.load_ms.unwrap_or(0)))
                .collect(),
        );
        let reasoning: u64 = ok.iter().filter_map(|r| r.reasoning_tokens).sum();
        let count = |o: Outcome| rs.iter().filter(|r| outcome(r) == o).count();
        out.push_str(&format!(
            "| `{model}` | {provider} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            rs.first().map(|r| r.tier.as_str()).unwrap_or("-"),
            ttft.map(|v| format!("{v}ms")).unwrap_or("-".into()),
            total.map(|v| format!("{v}ms")).unwrap_or("-".into()),
            pct(count(Outcome::Answered), n),
            pct(count(Outcome::EmptyByStop), n),
            pct(count(Outcome::EmptyByBudget), n),
            pct(count(Outcome::MemoryOnly), n),
            pct(rs.iter().filter(|r| r.fenced).count(), n),
            pct(rs.iter().filter(|r| r.prose_lines <= 1).count(), n),
            pct(rs.iter().filter(|r| r.has_cmd_tag).count(), n),
            if reasoning > 0 {
                reasoning.to_string()
            } else {
                "-".into()
            },
        ));
    }

    // ---- per shape: the ordering levers
    out.push_str("\n## Per prompt shape\n\n");
    out.push_str(
        "S1 = shipped (memories before log) · S2 = memories in suffix · S3 = CMD: first\n\n\
         | shape | p50 ttft | p50 total | p50 prompt-eval | empty | 1-line | CMD: |\n\
         |---|---|---|---|---|---|---|\n",
    );
    let mut by_shape: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
    for r in &sweep {
        by_shape.entry(r.shape.clone()).or_default().push(r);
    }
    for (shape, rs) in &by_shape {
        let ok: Vec<&&Row> = rs.iter().filter(|r| r.error.is_none()).collect();
        out.push_str(&format!(
            "| {shape} | {} | {} | {} | {} | {} | {} |\n",
            median(ok.iter().filter_map(|r| r.ttft_ms).collect())
                .map(|v| format!("{v}ms"))
                .unwrap_or("-".into()),
            median(ok.iter().map(|r| r.total_ms).collect())
                .map(|v| format!("{v}ms"))
                .unwrap_or("-".into()),
            median(ok.iter().filter_map(|r| r.prompt_eval_ms).collect())
                .map(|v| format!("{v}ms"))
                .unwrap_or("-".into()),
            pct(
                rs.iter()
                    .filter(|r| outcome(r) != Outcome::Answered && r.error.is_none())
                    .count(),
                rs.len()
            ),
            pct(rs.iter().filter(|r| r.prose_lines <= 1).count(), rs.len()),
            pct(rs.iter().filter(|r| r.has_cmd_tag).count(), rs.len()),
        ));
    }

    // ---- command headroom: is the ceiling capping the payload?
    let long: Vec<&Row> = rows
        .iter()
        .filter(|r| r.step.starts_with("long-") && r.error.is_none())
        .collect();
    if !long.is_empty() {
        out.push_str(
            "\n## Command headroom\n\n\
             `max_tokens` does no display work — the band clamps prose at render\n\
             and the paste path sends the command whole — so the ceiling only\n\
             ever caps the payload. Each question is run at the shipped 256 and\n\
             at 1024 to separate \"terse model\" from \"budget cut it off\".\n\n\
             | model | probe | cmd chars | stop | truncated |\n\
             |---|---|---|---|---|\n",
        );
        let mut sorted = long.clone();
        sorted.sort_by(|a, b| (&a.model, &a.step).cmp(&(&b.model, &b.step)));
        for r in &sorted {
            out.push_str(&format!(
                "| `{}` | {} | {} | {} | {} |\n",
                r.model,
                r.step,
                r.command.as_deref().map(str::len).unwrap_or(0),
                r.stop_reason.as_deref().unwrap_or("-"),
                if command_truncated(r) { "**yes**" } else { "" },
            ));
        }
        for budget in ["256", "1024"] {
            let at: Vec<&&Row> = long.iter().filter(|r| r.step.ends_with(budget)).collect();
            if at.is_empty() {
                continue;
            }
            let cut = at.iter().filter(|r| command_truncated(r)).count();
            let longest = at
                .iter()
                .filter_map(|r| r.command.as_deref().map(str::len))
                .max()
                .unwrap_or(0);
            out.push_str(&format!(
                "\nAt `max_tokens = {budget}`: {cut}/{} truncated, longest command {longest} chars.\n",
                at.len()
            ));
        }
    }

    // ---- cache: prompt-eval against a growing prefix (ollama only)
    out.push_str(
        "\n## Cache behaviour\n\n\
         Ollama only — LM Studio exposes no prompt-eval timing, so its cache\n\
         evidence is TTFT flatness in the per-turn table instead.\n\n\
         | model | shape | turns | prompt chars (first→last) | prompt-eval (first→last) |\n\
         |---|---|---|---|---|\n",
    );
    let mut by_session: BTreeMap<(String, String), Vec<&Row>> = BTreeMap::new();
    for r in sweep.iter().filter(|r| r.prompt_eval_ms.is_some()) {
        by_session
            .entry((r.model.clone(), r.shape.clone()))
            .or_default()
            .push(r);
    }
    for ((model, shape), mut rs) in by_session {
        rs.sort_by_key(|r| r.turn_index);
        let (Some(a), Some(b)) = (rs.first(), rs.last()) else {
            continue;
        };
        out.push_str(&format!(
            "| `{model}` | {shape} | {} | {} → {} | {}ms → {}ms |\n",
            rs.len(),
            a.prompt_chars,
            b.prompt_chars,
            a.prompt_eval_ms.unwrap_or(0),
            b.prompt_eval_ms.unwrap_or(0),
        ));
    }

    // ---- failures, named
    let failed: Vec<&&Row> = sweep.iter().filter(|r| r.error.is_some()).collect();
    if !failed.is_empty() {
        out.push_str(&format!("\n## Errors ({})\n\n", failed.len()));
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for r in &failed {
            *seen
                .entry(format!(
                    "{} — {}",
                    r.model,
                    r.error.clone().unwrap_or_default()
                ))
                .or_default() += 1;
        }
        for (what, n) in seen {
            out.push_str(&format!("- {n}x {what}\n"));
        }
    }

    // Coverage against the merged manifest: a filtered run (memory cap,
    // single-model rerun) covers less than full coverage, and the report
    // should say so rather than implying the matrix is complete.
    let missing = Journal::outstanding(dir);
    if !missing.is_empty() {
        let mut by_pass: BTreeMap<String, usize> = BTreeMap::new();
        for k in &missing {
            *by_pass
                .entry(k.split('/').next().unwrap_or("?").to_string())
                .or_default() += 1;
        }
        out.push_str("\n## Coverage gaps\n\nPlanned but not yet run:\n\n");
        for (pass, n) in by_pass {
            out.push_str(&format!("- {pass}: {n} cell(s)\n"));
        }
    }

    let path = dir.join("report.md");
    std::fs::write(&path, &out)?;
    println!("{out}");
    println!("wrote {}", path.display());
    Ok(())
}

/// Emit the corpus with model, provider and shape stripped, grouped by
/// question and shuffled deterministically. Priors about which models are
/// good cannot reach the scores if the scores are written against this.
pub fn blind(dir: &Path) -> std::io::Result<()> {
    let rows = Journal::rows(dir);
    let sweep: Vec<&Row> = rows
        .iter()
        .filter(|r| r.pass == "pass-b" && r.error.is_none())
        .collect();
    if sweep.is_empty() {
        println!("nothing to grade in {}", dir.display());
        return Ok(());
    }

    let mut by_step: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
    for r in &sweep {
        by_step.entry(r.step.clone()).or_default().push(r);
    }

    let mut out = String::new();
    out.push_str(
        "# Blind grading corpus\n\n\
         Answers only — no model, provider or shape. Grade each on the text\n\
         alone and write one row per id to `grades.jsonl`:\n\n\
         ```json\n\
         {\"id\":\"a1b2c3\",\"correct\":0-3,\"idiom\":0-3,\"fit\":0-3,\"why\":\"...\"}\n\
         ```\n\n\
         correct = does it actually do what was asked · idiom = is it how a\n\
         practitioner would write it · fit = does it fit one status-bar line\n\n",
    );
    let mut key_map = String::new();
    for (step, rs) in &by_step {
        let question = rs.first().map(|r| r.question.as_str()).unwrap_or("");
        out.push_str(&format!(
            "## {step}\n\n> {}\n\n",
            question.replace('\n', " ")
        ));
        // Deterministic shuffle: order by blind id, which is a hash of the
        // cell key and therefore uncorrelated with catalog order.
        let mut sorted = rs.clone();
        sorted.sort_by_key(|r| blind_id(&r.key));
        for r in sorted {
            let id = blind_id(&r.key);
            out.push_str(&format!(
                "- `[{id}]` {}{}\n",
                if r.text.is_empty() {
                    "(no prose)".to_string()
                } else {
                    r.text.clone()
                },
                r.command
                    .as_ref()
                    .map(|c| format!("\n      `CMD: {c}`"))
                    .unwrap_or_default(),
            ));
            key_map.push_str(&format!(
                "{}\n",
                serde_json::json!({"id": id, "key": r.key})
            ));
        }
        out.push('\n');
    }

    std::fs::write(dir.join("blind.md"), &out)?;
    let mut f = std::fs::File::create(dir.join("blind_keys.jsonl"))?;
    f.write_all(key_map.as_bytes())?;
    println!(
        "wrote {} ({} answers across {} questions)\nkey map: {}",
        dir.join("blind.md").display(),
        sweep.len(),
        by_step.len(),
        dir.join("blind_keys.jsonl").display()
    );
    Ok(())
}

/// The audit path: exact prompt in, exact raw response out, no summary.
pub fn replay(dir: &Path, key: &str) -> std::io::Result<()> {
    let rows = Journal::rows(dir);
    let Some(row) = rows
        .iter()
        .find(|r| r.key == key || blind_id(&r.key) == key)
    else {
        println!("no such cell: {key}");
        return Ok(());
    };
    println!("=== cell {} ===", row.key);
    println!(
        "model={} provider={} shape={} step={} turn={}",
        row.model, row.provider, row.shape, row.step, row.turn_index
    );
    println!(
        "ttft={:?} total={}ms prompt_eval={:?} prompt_tokens={:?} reasoning={:?} stop={:?}",
        row.ttft_ms,
        row.total_ms,
        row.prompt_eval_ms,
        row.prompt_tokens,
        row.reasoning_tokens,
        row.stop_reason
    );
    println!("\n=== PROMPT SENT ({} chars) ===", row.prompt_chars);
    match Journal::prompt_for(dir, &row.key) {
        Some(p) => println!("{p}"),
        None => println!("(prompt not recorded)"),
    }
    println!("\n=== RAW RESPONSE ===\n{}", row.raw);
    println!("\n=== PARSED (goulash's own split_answer) ===");
    println!("text    = {:?}", row.text);
    println!("command = {:?}", row.command);
    if let Some(e) = &row.error {
        println!("error   = {e}");
    }
    Ok(())
}

// ---------------------------------------------------------------- grading

#[derive(serde::Deserialize, Clone)]
pub struct Grade {
    pub id: String,
    /// Does the command actually do what was asked? 0-3.
    pub correct: u8,
    /// Is it how a practitioner would write it? 0-3.
    pub idiom: u8,
    /// Does the prose fit one status-bar line? 0-3.
    pub fit: u8,
    #[serde(default)]
    pub why: String,
}

fn load_grades(dir: &Path) -> Vec<Grade> {
    let Ok(f) = std::fs::File::open(dir.join("grades.jsonl")) else {
        return Vec::new();
    };
    std::io::BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with("//"))
        .filter_map(|l| serde_json::from_str::<Grade>(&l).ok())
        .collect()
}

/// Join blind grades back to provenance and write the quality report.
///
/// Kept separate from `run` so the mechanical report never waits on
/// grading, and so a grade can be traced to the exact answer that earned
/// it: every row here carries the blind id, and `replay <id>` prints the
/// prompt and raw response behind it.
pub fn grade(dir: &Path) -> std::io::Result<()> {
    let rows = Journal::rows(dir);
    let grades = load_grades(dir);
    if grades.is_empty() {
        println!(
            "no grades in {}.\nRun `blind` first, score blind.md into \
             grades.jsonl, then rerun this.",
            dir.join("grades.jsonl").display()
        );
        return Ok(());
    }
    let by_id: BTreeMap<String, &Row> = rows.iter().map(|r| (blind_id(&r.key), r)).collect();
    let _ = &by_id;

    let mut joined: Vec<(&Grade, &Row)> = Vec::new();
    let mut orphans = 0;
    for g in &grades {
        match by_id.get(&g.id) {
            Some(r) => joined.push((g, r)),
            None => orphans += 1,
        }
    }

    let mut out = String::new();
    out.push_str("# Quality — blind-graded\n\n");
    out.push_str(&format!(
        "{} grades joined to {} rows{}. Scored on answer text alone, with\n\
         model, provider and shape hidden; joined back afterwards. Each id\n\
         is replayable: `goulash-bench replay DIR <id>`.\n\n",
        joined.len(),
        rows.len(),
        if orphans > 0 {
            format!(" ({orphans} orphan grade(s) ignored)")
        } else {
            String::new()
        }
    ));

    let mean = |v: &[u8]| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64
        }
    };

    // ---- by model
    out.push_str("## By model\n\n| model | provider | n | correct | idiom | fit |\n|---|---|---|---|---|---|\n");
    let mut per_model: BTreeMap<(String, String), Vec<&Grade>> = BTreeMap::new();
    for (g, r) in &joined {
        per_model
            .entry((r.model.clone(), r.provider.clone()))
            .or_default()
            .push(g);
    }
    let mut ranked: Vec<_> = per_model.iter().collect();
    ranked.sort_by(|a, b| {
        let s = |gs: &Vec<&Grade>| mean(&gs.iter().map(|g| g.correct).collect::<Vec<_>>());
        s(b.1)
            .partial_cmp(&s(a.1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for ((model, provider), gs) in ranked {
        out.push_str(&format!(
            "| `{model}` | {provider} | {} | {:.2} | {:.2} | {:.2} |\n",
            gs.len(),
            mean(&gs.iter().map(|g| g.correct).collect::<Vec<_>>()),
            mean(&gs.iter().map(|g| g.idiom).collect::<Vec<_>>()),
            mean(&gs.iter().map(|g| g.fit).collect::<Vec<_>>()),
        ));
    }

    // ---- by shape: does CMD-first cost quality?
    out.push_str(
        "\n## By prompt shape\n\nThe question command-first has to answer: \
         does moving `CMD:` ahead of the prose cost answer quality?\n\n\
         | shape | n | correct | idiom | fit |\n|---|---|---|---|---|\n",
    );
    let mut per_shape: BTreeMap<String, Vec<&Grade>> = BTreeMap::new();
    for (g, r) in &joined {
        per_shape.entry(r.shape.clone()).or_default().push(g);
    }
    for (shape, gs) in &per_shape {
        out.push_str(&format!(
            "| {shape} | {} | {:.2} | {:.2} | {:.2} |\n",
            gs.len(),
            mean(&gs.iter().map(|g| g.correct).collect::<Vec<_>>()),
            mean(&gs.iter().map(|g| g.idiom).collect::<Vec<_>>()),
            mean(&gs.iter().map(|g| g.fit).collect::<Vec<_>>()),
        ));
    }

    // ---- worst answers, named, with their provenance
    let mut worst: Vec<&(&Grade, &Row)> = joined.iter().filter(|(g, _)| g.correct <= 1).collect();
    worst.sort_by_key(|(g, _)| g.correct);
    if !worst.is_empty() {
        out.push_str(&format!(
            "\n## Failures ({} scored 0-1 on correctness)\n\n",
            worst.len()
        ));
        for (g, r) in worst.iter().take(40) {
            out.push_str(&format!(
                "- **{}** `{}` {} — {}\n  - `{}`\n  - _{}_\n",
                g.correct,
                r.model,
                r.shape,
                r.step,
                r.command.as_deref().unwrap_or("(no command)"),
                g.why
            ));
        }
    }

    std::fs::write(dir.join("quality.md"), &out)?;
    println!("{out}");
    println!("wrote {}", dir.join("quality.md").display());
    Ok(())
}
