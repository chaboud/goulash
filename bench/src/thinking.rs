//! Pass T: what does reasoning actually cost, and does it stay short?
//!
//! The token cap exists to bound what lands in the status band. But
//! providers meter reasoning and visible output on the *same* counter, so
//! a reasoning model spends the display budget thinking and returns
//! nothing — 25% of all empty answers ended at `stop_reason = length`.
//!
//! Meanwhile the cap does no display work at all: answers that arrive use
//! a median of 32 tokens and p90 of 77, against a ceiling of 256. The
//! prompt is what keeps answers short.
//!
//! So the questions here are:
//!
//! 1. Given room, does a reasoning model think *and then* answer?
//! 2. How much room does it actually need — what should the allowance be?
//! 3. With reasoning on and room to spare, does the **visible** answer
//!    stay one line? That is the display contract, and if it holds, the
//!    cap was never protecting it.
//! 4. Does ollama honour a graded `think` level?

use crate::journal::{Journal, cell_key};
use crate::load_catalog;
use crate::sweep::{NUM_CTX, agent, preload_lmstudio, provider_for, run_one, seed_memory, unload_all};
use goulash::engine::{MemPos, PromptShape, Think};
use std::collections::BTreeMap;
use std::path::Path;

struct Probe {
    id: &'static str,
    think: Think,
    /// Display budget.
    max_tokens: usize,
    /// Allowance on top, when reasoning is enabled.
    reasoning: usize,
}

fn probes() -> Vec<Probe> {
    vec![
        // Shipped today: reasoning suppressed, 256 total.
        Probe {
            id: "T0-off-256",
            think: Think::Off,
            max_tokens: 256,
            reasoning: 0,
        },
        // Reasoning on, no extra room — reproduces the failure.
        Probe {
            id: "T1-on-256-noalw",
            think: Think::On,
            max_tokens: 256,
            reasoning: 0,
        },
        // Reasoning on with a real allowance. Does it finish and answer?
        Probe {
            id: "T2-on-256+1024",
            think: Think::On,
            max_tokens: 256,
            reasoning: 1024,
        },
        // Generous: sizes the allowance a model actually wants.
        Probe {
            id: "T3-on-256+4096",
            think: Think::On,
            max_tokens: 256,
            reasoning: 4096,
        },
        // Graded effort, if the provider honours it.
        Probe {
            id: "T4-low-256+1024",
            think: Think::Level("low"),
            max_tokens: 256,
            reasoning: 1024,
        },
        Probe {
            id: "T5-high-256+1024",
            think: Think::Level("high"),
            max_tokens: 256,
            reasoning: 1024,
        },
        // Reasoning off but a big display budget: isolates whether a
        // larger cap alone makes answers longer. If prose stays short
        // here, the cap is doing no display work — which is the claim.
        Probe {
            id: "T6-off-2048",
            think: Think::Off,
            max_tokens: 2048,
            reasoning: 0,
        },
    ]
}

/// One easy and one genuinely hard question: reasoning models should
/// spend more on the second, and the allowance has to cover the worse
/// case, not the average.
const QUESTIONS: &[(&str, &str)] = &[
    ("easy", "what's eating space in here, biggest first"),
    (
        "hard",
        "from data.json give me name and bytes for every failed item, \
         sorted by bytes descending, as tab-separated output",
    ),
];

const CTX: &str = "$ cd /Users/dev/project [exit 0, 08:59:12]\n\
$ ls [exit 0, 08:59:30]\nCargo.toml  data.json  src  target\n";

pub fn run(dir: &Path) -> std::io::Result<()> {
    let catalog = load_catalog();
    let mut j = Journal::open(dir)?;
    let agent = agent();
    let paths = goulash::vendor::path_executable_set();
    let ps = probes();
    let memories = seed_memory().context_block();
    let shape = PromptShape {
        memories: MemPos::BeforeLog,
        command_first: true,
        ..PromptShape::default()
    };

    let mut planned = Vec::new();
    for c in &catalog.cell {
        for p in &ps {
            for (qid, _) in QUESTIONS {
                planned.push(cell_key(
                    "pass-t",
                    &c.provider,
                    &c.model,
                    p.id,
                    qid,
                ));
            }
        }
    }
    j.write_manifest(&planned)?;
    println!(
        "pass-t: {} cells x {} budgets x {} questions = {} generations ({} done)",
        catalog.cell.len(),
        ps.len(),
        QUESTIONS.len(),
        planned.len(),
        j.completed_in("pass-t")
    );

    for cell in &catalog.cell {
        let todo: Vec<(&Probe, &(&str, &str))> = ps
            .iter()
            .flat_map(|p| QUESTIONS.iter().map(move |q| (p, q)))
            .filter(|(p, (qid, _))| {
                !j.is_done(&cell_key("pass-t", &cell.provider, &cell.model, p.id, qid))
            })
            .collect();
        if todo.is_empty() {
            println!("  [skip] {} — complete", cell.model);
            continue;
        }
        println!("  {} ({}) — {} to go", cell.model, cell.provider, todo.len());
        if cell.provider.starts_with("openai") {
            preload_lmstudio(&cell.model, NUM_CTX, "3m");
        }
        let provider = provider_for(cell);
        for (p, (qid, question)) in todo {
            run_one(
                &mut j,
                provider.as_ref(),
                &agent,
                cell,
                "pass-t",
                p.id,
                &shape,
                qid,
                0,
                question,
                false,
                CTX,
                &memories,
                &paths,
                &["\n\n".to_string()],
                p.think,
                p.reasoning,
                p.max_tokens,
            );
        }
        unload_all(&agent, "http://127.0.0.1:11434");
    }
    summarize(dir);
    Ok(())
}

pub fn summarize(dir: &Path) {
    let rows: Vec<_> = Journal::rows(dir)
        .into_iter()
        .filter(|r| r.pass == "pass-t")
        .collect();
    if rows.is_empty() {
        return;
    }
    println!(
        "\n  {:<18} {:>6} {:>9} {:>10} {:>11} {:>11}",
        "budget", "n", "answered", "hit cap", "med tokens", "med prose"
    );
    let mut by: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for r in &rows {
        by.entry(r.shape.clone()).or_default().push(r);
    }
    let med = |mut v: Vec<u64>| {
        if v.is_empty() {
            0
        } else {
            v.sort_unstable();
            v[v.len() / 2]
        }
    };
    for p in probes() {
        let Some(rs) = by.get(p.id) else { continue };
        let answered: Vec<_> = rs
            .iter()
            .filter(|r| !r.text.trim().is_empty() || r.command.is_some())
            .collect();
        println!(
            "  {:<18} {:>6} {:>9} {:>10} {:>11} {:>11}",
            p.id,
            rs.len(),
            format!("{}/{}", answered.len(), rs.len()),
            rs.iter()
                .filter(|r| r.stop_reason.as_deref() == Some("length"))
                .count(),
            med(rs.iter().filter_map(|r| r.eval_tokens).collect()),
            med(answered.iter().map(|r| r.text.chars().count() as u64).collect()),
        );
    }
    println!(
        "\n  'med prose' is the VISIBLE answer length in chars. If it stays flat\n  \
         as the budget grows, the cap was never what kept answers short — the\n  \
         prompt was, and reasoning can safely have its own allowance."
    );
}
