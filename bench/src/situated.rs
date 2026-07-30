//! Pass W: does telling the model where it is actually help?
//!
//! The sweep found **6.9% of 2395 vended commands used GNU-only syntax**
//! on a Darwin box — `grep -P` 114 times against a BSD grep that has no
//! `-P` at all, `du --max-depth` 40 times where BSD wants `-d`. And the
//! seeded memory said "prefers fd over find" on a machine with no `fd`.
//!
//! Both are things goulash already knows and never says. This tests two
//! additions, separately and together:
//!
//!   W1  platform + shell line
//!   W2  available-tools line (curated subset of the PATH set)
//!   W3  both
//!
//! Both lines are **static per machine**, so they go at the very front of
//! the stable prefix — cached once, never re-evaluated, and they cannot
//! perturb the session-log prefix behind them.
//!
//! Scoring is mechanical: a command either uses a GNU-only form or it
//! does not, and either names an absent binary or does not. No grading.

use crate::journal::{Journal, cell_key};
use crate::load_catalog;
use crate::sweep::{
    NUM_CTX, agent, await_headroom, preload_lmstudio, provider_for, run_one, seed_memory,
    unload_all,
};
use goulash::engine::{MemPos, PromptShape, Think};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

/// The shipped preamble, verbatim, so variants differ only by additions.
const BASE: &str = goulash::engine::PREAMBLE;

/// Facts goulash can read without running anything: `uname`, `$SHELL`,
/// and a `read_dir` of PATH. Nothing here executes a binary — checking a
/// tool by invoking `--help` would violate the core invariant (goulash
/// does not run commands) and can trigger installers on shimmed tools.
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
            "Environment: macOS ({os}) with BSD userland, {shell} shell. BSD tools \
             differ from GNU: use 'du -d N' not '--max-depth', 'sed -i \"\"' not \
             'sed -i', 'date -v' not 'date -d', 'stat -f' not 'stat -c'. BSD grep \
             has NO -P; use -E or perl.\n\n"
        )
    } else {
        format!("Environment: {os} with GNU userland, {shell} shell.\n\n")
    }
}

/// Which of the tools a shell assistant reaches for are actually here.
///
/// The full PATH set is 1733 entries (~3900 tokens) and useless in a
/// prompt; this curated intersection is ~32. It is also silent about
/// shell builtins, keywords, aliases and functions, which `read_dir`
/// cannot see — so it says what is installed, not what is callable.
const CURATED: &[&str] = &[
    "jq", "yq", "rg", "fd", "ag", "tree", "bat", "delta", "fzf", "gh", "git", "docker",
    "kubectl", "tmux", "curl", "wget", "ffmpeg", "zstd", "pigz", "pv", "rsync", "gsed",
    "gawk", "ggrep", "gdate", "gstat", "gfind", "gtar", "python3", "node", "cargo", "go",
    "make", "cmake", "tar", "unzip",
];

fn tools_line() -> String {
    let have = goulash::vendor::path_executable_set();
    let present: Vec<&str> = CURATED.iter().copied().filter(|t| have.contains(*t)).collect();
    let absent: Vec<&str> = CURATED.iter().copied().filter(|t| !have.contains(*t)).collect();
    format!(
        "Installed tools: {}. NOT installed, never suggest: {}.\n\n",
        present.join(" "),
        absent.join(" ")
    )
}

pub struct Variant {
    pub id: &'static str,
    pub preamble: String,
}

pub fn variants() -> Vec<Variant> {
    vec![
        Variant {
            id: "W0-baseline",
            preamble: BASE.to_string(),
        },
        Variant {
            id: "W1-platform",
            preamble: format!("{}{BASE}", platform_line()),
        },
        Variant {
            id: "W2-tools",
            preamble: format!("{}{BASE}", tools_line()),
        },
        Variant {
            id: "W3-both",
            preamble: format!("{}{}{BASE}", platform_line(), tools_line()),
        },
    ]
}

/// Questions chosen to bait the exact failures the sweep measured.
const QUESTIONS: &[(&str, &str)] = &[
    // grep -P bait — 114 occurrences in the sweep; BSD grep has no -P.
    ("perl-regex", "search the logs for lines matching a perl-style regex with a lookahead"),
    // du --max-depth bait — 40 occurrences; BSD du wants -d.
    ("disk-depth", "show me disk usage one level deep, biggest first"),
    // sed -i bait — BSD sed needs an explicit empty backup suffix.
    ("sed-inplace", "replace every TODO with DONE in notes.md, in place"),
    // date -d bait — BSD date wants -v.
    ("date-ago", "what was the date three days ago"),
    // stat bait — BSD stat wants -f.
    ("file-size", "print the size in bytes of every .rs file here"),
    // fd bait — the pinned memory says the user prefers fd, and fd is
    // NOT installed. Does the tools line stop it being suggested?
    ("find-files", "find every .rs file changed in the last week"),
    // A tool that IS installed, as a control: the line must not make the
    // model timid about things that are genuinely present.
    ("json-extract", "pull every .items[].name out of data.json"),
    // Pure control: no platform-specific surface at all.
    ("explain-flag", "what does the -exec flag do in find"),
];

const CTX: &str = "$ cd /Users/dev/project [exit 0, 08:59:12]\n\
$ ls [exit 0, 08:59:30]\nCargo.toml  data.json  notes.md  src  target\n";

pub fn run(dir: &Path) -> std::io::Result<()> {
    let catalog = load_catalog();
    let mut j = Journal::open(dir)?;
    let agent = agent();
    let paths = goulash::vendor::path_executable_set();
    let vs = variants();
    let memories = seed_memory().context_block();

    println!("  platform line: {}", platform_line().trim());
    println!("  tools line:    {}\n", tools_line().trim());

    let mut planned = Vec::new();
    for c in &catalog.cell {
        for v in &vs {
            for (qid, _) in QUESTIONS {
                planned.push(cell_key("pass-w", &c.provider, &c.model, v.id, qid));
            }
        }
    }
    j.write_manifest(&planned)?;
    println!(
        "pass-w: {} cells x {} variants x {} questions = {} generations ({} done)",
        catalog.cell.len(),
        vs.len(),
        QUESTIONS.len(),
        planned.len(),
        j.completed_in("pass-w")
    );

    for cell in &catalog.cell {
        let todo: Vec<(&Variant, &(&str, &str))> = vs
            .iter()
            .flat_map(|v| QUESTIONS.iter().map(move |q| (v, q)))
            .filter(|(v, (qid, _))| {
                !j.is_done(&cell_key("pass-w", &cell.provider, &cell.model, v.id, qid))
            })
            .collect();
        if todo.is_empty() {
            println!("  [skip] {} — complete", cell.model);
            continue;
        }
        println!("  {} ({}) — {} to go", cell.model, cell.provider, todo.len());
        await_headroom(15, Duration::from_secs(900));
        if cell.provider.starts_with("openai") {
            preload_lmstudio(&cell.model, NUM_CTX, "3m");
        }
        let provider = provider_for(cell);
        for (v, (qid, question)) in todo {
            // command_first: the settled default.
            let shape = PromptShape {
                memories: MemPos::BeforeLog,
                command_first: true,
                preamble: Some(Box::leak(v.preamble.clone().into_boxed_str())),
                directive: None,
            };
            run_one(
                &mut j,
                provider.as_ref(),
                &agent,
                cell,
                "pass-w",
                v.id,
                &shape,
                qid,
                0,
                question,
                false,
                CTX,
                &memories,
                &paths,
                &["\n\n".to_string()],
                Think::Off,
                0,
                256,
            );
        }
        unload_all(&agent, "http://127.0.0.1:11434");
    }
    summarize(dir);
    Ok(())
}

/// GNU-only forms that fail or behave differently under BSD userland.
fn gnu_only(cmd: &str) -> Option<&'static str> {
    let c = cmd;
    let has = |pat: &str| c.contains(pat);
    if has("--max-depth") {
        return Some("du --max-depth");
    }
    if c.split_whitespace().any(|w| w.starts_with("-") && w.contains('P'))
        && c.contains("grep")
        && !c.contains("ggrep")
    {
        return Some("grep -P");
    }
    if has("stat -c") {
        return Some("stat -c");
    }
    if has("--time-style") {
        return Some("ls --time-style");
    }
    if has("-printf") {
        return Some("find -printf");
    }
    if has("date -d") || has("date --date") {
        return Some("date -d");
    }
    if has("xargs -r") {
        return Some("xargs -r");
    }
    if has("readlink -f") {
        return Some("readlink -f");
    }
    // sed -i not followed by a quoted suffix
    if let Some(i) = c.find("sed ") {
        let rest = &c[i..];
        if let Some(j) = rest.find("-i") {
            let after = rest[j + 2..].trim_start();
            if !after.starts_with('\'') && !after.starts_with('"') && !after.starts_with(".bak") {
                return Some("sed -i (no suffix)");
            }
        }
    }
    None
}

pub fn summarize(dir: &Path) {
    let rows: Vec<_> = Journal::rows(dir)
        .into_iter()
        .filter(|r| r.pass == "pass-w" && r.error.is_none())
        .collect();
    if rows.is_empty() {
        return;
    }
    let have = goulash::vendor::path_executable_set();
    let absent: Vec<&str> = CURATED.iter().copied().filter(|t| !have.contains(*t)).collect();
    let names_absent = |c: &str| {
        c.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_')
            .any(|w| absent.contains(&w))
    };

    println!(
        "\n  {:<14} {:>5} {:>9} {:>12} {:>14} {:>10}",
        "variant", "n", "answered", "GNU-only", "absent tool", "prose ch"
    );
    let mut by: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for r in &rows {
        by.entry(r.shape.clone()).or_default().push(r);
    }
    for v in variants() {
        let Some(rs) = by.get(v.id) else { continue };
        let with_cmd: Vec<_> = rs.iter().filter(|r| r.command.is_some()).collect();
        let gnu = with_cmd
            .iter()
            .filter(|r| gnu_only(r.command.as_deref().unwrap_or("")).is_some())
            .count();
        let abs = with_cmd
            .iter()
            .filter(|r| names_absent(r.command.as_deref().unwrap_or("")))
            .count();
        let prose: Vec<u64> = rs
            .iter()
            .filter(|r| !r.text.trim().is_empty())
            .map(|r| r.text.chars().count() as u64)
            .collect();
        let med = if prose.is_empty() {
            0
        } else {
            let mut p = prose.clone();
            p.sort_unstable();
            p[p.len() / 2]
        };
        println!(
            "  {:<14} {:>5} {:>9} {:>12} {:>14} {:>10}",
            v.id,
            rs.len(),
            with_cmd.len(),
            format!("{gnu} ({:.0}%)", 100.0 * gnu as f64 / with_cmd.len().max(1) as f64),
            format!("{abs} ({:.0}%)", 100.0 * abs as f64 / with_cmd.len().max(1) as f64),
            med
        );
    }
    println!(
        "\n  GNU-only = a form that fails or differs under BSD userland.\n  \
         absent tool = names a binary that is not installed on this machine.\n  \
         Both are mechanical checks on the vended command; no grading involved."
    );
}
