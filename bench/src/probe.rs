//! Pass A: setting-tolerance probe.
//!
//! Cheap per-cell checks of the setting matrix, run before the sweep so
//! broken cells are known rather than discovered eight hours in. Each
//! probe isolates one setting against a fixed, tiny context.

use crate::journal::{Journal, cell_key};
use crate::load_catalog;
use crate::sweep::{agent, provider_for, run_one, shapes, unload};
use std::path::Path;

/// A single tolerance question, and the setting it varies.
struct Probe {
    id: &'static str,
    question: &'static str,
    stop: Vec<String>,
    think: Option<bool>,
    max_tokens: usize,
}

fn probes() -> Vec<Probe> {
    let nl = vec!["\n\n".to_string()];
    vec![
        // Baseline: exactly what the product sends today.
        Probe {
            id: "baseline",
            question: "how do I list files by size, largest first",
            stop: nl.clone(),
            think: Some(false),
            max_tokens: 256,
        },
        // The top suspect. A model that opens with a blank line, or
        // separates prose from CMD: with one, truncates to nothing —
        // and engine.rs blames "thinking model?" for it.
        Probe {
            id: "no-stop",
            question: "how do I list files by size, largest first",
            stop: Vec::new(),
            think: Some(false),
            max_tokens: 256,
        },
        // Is think:false doing anything, or is it inert / harmful?
        Probe {
            id: "no-think-field",
            question: "how do I list files by size, largest first",
            stop: nl.clone(),
            think: None,
            max_tokens: 256,
        },
        // Does the model respect a tight budget, and what does it drop?
        Probe {
            id: "tight-budget",
            question: "how do I list files by size, largest first",
            stop: nl.clone(),
            think: Some(false),
            max_tokens: 32,
        },
        // Fence bait: the parser's bare-command fallback strips single
        // backticks but not a ```bash block.
        Probe {
            id: "fence-bait",
            question: "show me the command to extract .items[].name from data.json",
            stop: nl.clone(),
            think: Some(false),
            max_tokens: 256,
        },
        // Should produce prose and NO command.
        Probe {
            id: "no-command-needed",
            question: "what does the -P flag do in grep",
            stop: nl,
            think: Some(false),
            max_tokens: 256,
        },
    ]
}

const CTX: &str = "$ cd /Users/dev/project [exit 0, 08:59:12]\n\
$ ls [exit 0, 08:59:30]\nCargo.toml  data.json  src  target\n";

pub fn run(dir: &Path) -> std::io::Result<()> {
    let catalog = load_catalog();
    let mut j = Journal::open(dir)?;
    let agent = agent();
    let paths = goulash::vendor::path_executable_set();
    let probes = probes();
    // Shipped shape only: Pass A asks about settings, not ordering.
    let (shape_name, shape) = shapes().into_iter().next().unwrap();

    let planned: Vec<String> = catalog
        .cell
        .iter()
        .flat_map(|c| {
            probes
                .iter()
                .map(move |p| cell_key("pass-a", &c.provider, &c.model, shape_name, p.id))
        })
        .collect();
    j.write_manifest(&planned)?;
    println!(
        "pass-a: {} cells x {} probes = {} generations ({} already done)",
        catalog.cell.len(),
        probes.len(),
        planned.len(),
        j.completed_in("pass-a")
    );

    for cell in &catalog.cell {
        let todo: Vec<&Probe> = probes
            .iter()
            .filter(|p| {
                !j.is_done(&cell_key(
                    "pass-a",
                    &cell.provider,
                    &cell.model,
                    shape_name,
                    p.id,
                ))
            })
            .collect();
        if todo.is_empty() {
            println!("  [skip] {} — complete", cell.model);
            continue;
        }
        println!("  {} ({}, {:.1} GB)", cell.model, cell.provider, cell.gb);
        let provider = provider_for(cell);
        for p in todo {
            run_one(
                &mut j,
                provider.as_ref(),
                &agent,
                cell,
                "pass-a",
                shape_name,
                &shape,
                p.id,
                0,
                p.question,
                false,
                CTX,
                &paths,
                &p.stop,
                p.think,
                p.max_tokens,
            );
        }
        unload(cell, &agent);
    }
    summarize(dir);
    Ok(())
}

/// What Pass A is for: name the cells that cannot participate, and the
/// settings that hurt, before committing hours to them.
pub fn summarize(dir: &Path) {
    let rows: Vec<_> = Journal::rows(dir)
        .into_iter()
        .filter(|r| r.pass == "pass-a")
        .collect();
    if rows.is_empty() {
        return;
    }
    println!("\n  {:<34} {:>8} {:>8} {:>7} {:>7}", "cell", "baseline", "no-stop", "fenced", "errors");
    let mut models: Vec<String> = rows.iter().map(|r| r.model.clone()).collect();
    models.sort();
    models.dedup();
    for m in models {
        let mine: Vec<_> = rows.iter().filter(|r| r.model == m).collect();
        let state = |id: &str| {
            mine.iter()
                .find(|r| r.step == id)
                .map(|r| {
                    if r.error.is_some() {
                        "ERR".to_string()
                    } else if r.empty {
                        "EMPTY".to_string()
                    } else {
                        format!("{}ms", r.total_ms)
                    }
                })
                .unwrap_or_else(|| "-".into())
        };
        let fenced = mine.iter().filter(|r| r.fenced).count();
        let errors = mine.iter().filter(|r| r.error.is_some()).count();
        println!(
            "  {:<34} {:>8} {:>8} {:>7} {:>7}",
            m,
            state("baseline"),
            state("no-stop"),
            fenced,
            errors
        );
    }
    println!(
        "\n  EMPTY under baseline but not under no-stop => the stop sequence is\n  \
         the culprit, not the model. That distinction is the point of Pass A."
    );
}
