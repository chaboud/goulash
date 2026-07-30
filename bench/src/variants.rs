//! Pass P: prompt wording exploration.
//!
//! Pass A found two failures that are plausibly *wording* problems rather
//! than model-capability problems:
//!
//! - **Refusal.** `qwen3.5:0.8b` answers a plain ffmpeg-syntax question
//!   with "I cannot convert video files … I am an AI assistant and not
//!   capable of …". It thinks it is being asked to *run* something.
//! - **Memory hijack.** `mistral-nemo` (and `qwen3.5:0.8b` on some asks)
//!   replies with a bare `REMEMBER:` line instead of an answer. The
//!   memory tool description is in the prompt and reads like an
//!   instruction.
//!
//! Each variant is a hypothesis about *why*, and changes only the
//! preamble and directive — never the log, memories, or question — so a
//! difference is attributable to wording alone.

use crate::journal::{Journal, cell_key};
use crate::load_catalog;
use crate::sweep::{NUM_CTX, agent, await_headroom, preload_lmstudio, wire_for, run_one, seed_memory, unload_all};
use crate::drive::{MemPos, PromptShape, Think};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

pub struct Variant {
    pub id: &'static str,
    pub hypothesis: &'static str,
    pub shape: PromptShape,
}

/// V1 — say the output is *text about* a command, never an action.
/// Hypothesis: refusals come from the model believing it must execute.
const PRE_NOT_EXECUTING: &str = "You are goulash, an assistant living in the user's \
terminal status bar. You WRITE shell command text for the user to read; you never \
run anything and nothing you write is executed automatically. Questions about how \
to do something are always requests for the command text, never requests to \
perform the action. Answer tersely in ONE short line of plain text, no markdown. \
Each command carries the local time it ran; treat old output as stale. The log \
also contains the running conversation: '#' lines are earlier user questions, \
'goulash:' lines are your earlier replies, and 'CMD:' lines are commands you \
suggested — follow-up questions refer back to them.\n\n";

/// V2 — spell out the grammar explicitly, as a grammar.
const DIR_GRAMMAR: &str = "Output format, exactly:\n\
line 1: CMD: <one shell command, may be long, no backticks, no markdown>\n\
line 2: <one short sentence of plain prose>\n\
Emit line 1 whenever any shell command could accomplish, fix, or demonstrate \
what was asked. Never put the command inside backticks or a code fence, and \
never write anything before 'CMD:'.";

/// V3 — one worked example. The M4 note in build-plan.md observed that
/// mimicry is what actually teaches small models a format.
const DIR_EXAMPLE: &str = "Reply in exactly this shape:\n\
CMD: du -sh * | sort -h\n\
Biggest directories first.\n\
That is: a 'CMD:' line holding one shell command, then one short prose line. \
Emit the CMD: line whenever any shell command could accomplish, fix, or \
demonstrate what was asked.";

/// V4 — fence off the memory tool so it stops eating answers.
const DIR_MEMGUARD: &str = "If any shell command could accomplish, fix, or \
demonstrate what was asked, you MUST put it FIRST on a line formatted exactly \
as: CMD: <command>. Then add ONE short prose line. Do NOT output a REMEMBER: \
or FORGET: line unless the user explicitly asked you to remember or forget \
something — they are tools, not a way to answer.";

/// V5 — everything that plausibly helps, together. If the combination
/// wins but no single variant does, the causes are independent.
const DIR_ALL: &str = "Output format, exactly:\n\
CMD: du -sh * | sort -h\n\
Biggest directories first.\n\
That is: line 1 is 'CMD:' followed by one shell command (it may be long); \
line 2 is one short prose sentence. Emit the CMD: line whenever any shell \
command could accomplish, fix, or demonstrate what was asked. Never use \
backticks or code fences. Do NOT output REMEMBER: or FORGET: unless the user \
explicitly asked you to remember or forget something.";

pub fn variants() -> Vec<Variant> {
    let base = PromptShape {
        memories: MemPos::BeforeLog,
        command_first: true,
        ..PromptShape::default()
    };
    vec![
        Variant {
            id: "V0-shipped-cmdfirst",
            hypothesis: "baseline: command-first, shipped wording",
            shape: base,
        },
        Variant {
            id: "V1-not-executing",
            hypothesis: "refusals come from thinking it must RUN the command",
            shape: PromptShape {
                divulge: Default::default(),
                preamble: Some(PRE_NOT_EXECUTING),
                ..base
            },
        },
        Variant {
            id: "V2-grammar",
            hypothesis: "an explicit line-by-line grammar raises tag compliance",
            shape: PromptShape {
                divulge: Default::default(),
                directive: Some(DIR_GRAMMAR),
                ..base
            },
        },
        Variant {
            id: "V3-example",
            hypothesis: "mimicry teaches small models better than description",
            shape: PromptShape {
                divulge: Default::default(),
                directive: Some(DIR_EXAMPLE),
                ..base
            },
        },
        Variant {
            id: "V4-memguard",
            hypothesis: "naming REMEMBER: a tool stops it eating answers",
            shape: PromptShape {
                divulge: Default::default(),
                directive: Some(DIR_MEMGUARD),
                ..base
            },
        },
        Variant {
            id: "V5-all",
            hypothesis: "combined; wins alone would mean independent causes",
            shape: PromptShape {
                divulge: Default::default(),
                preamble: Some(PRE_NOT_EXECUTING),
                directive: Some(DIR_ALL),
                ..base
            },
        },
    ]
}

/// The questions that actually failed in Pass A, plus controls.
const QUESTIONS: &[(&str, &str)] = &[
    // Refusal case.
    (
        "ffmpeg",
        "convert input.mov to a web-friendly h264 mp4 at 1080p, aac audio, \
         faststart for streaming",
    ),
    // Memory-hijack case.
    (
        "jq",
        "from data.json give me name and bytes for every failed item, sorted \
         by bytes descending, as tab-separated output",
    ),
    // Ordinary case: a variant must not break what already works.
    ("disk", "what's eating space in here, biggest first"),
    // Must NOT produce a command — checks the grammar push does not turn
    // every answer into a command.
    ("explain", "what does the -P flag do in grep"),
];

const CTX: &str = "$ cd /Users/dev/project [exit 0, 08:59:12]\n\
$ ls [exit 0, 08:59:30]\nCargo.toml  data.json  input.mov  src  target\n";

pub fn run(dir: &Path) -> std::io::Result<()> {
    let catalog = load_catalog();
    let mut j = Journal::open(dir)?;
    let agent = agent();
    let paths = goulash::vendor::path_executable_set();
    let vs = variants();
    let memories = seed_memory().context_block();

    let mut planned = Vec::new();
    for c in &catalog.cell {
        for v in &vs {
            for (qid, _) in QUESTIONS {
                planned.push(cell_key("pass-p", &c.provider, &c.model, v.id, qid));
            }
        }
    }
    j.write_manifest(&planned)?;
    println!(
        "pass-p: {} cells x {} variants x {} questions = {} generations ({} done)",
        catalog.cell.len(),
        vs.len(),
        QUESTIONS.len(),
        planned.len(),
        j.completed_in("pass-p")
    );

    for cell in &catalog.cell {
        let todo: Vec<(&Variant, &(&str, &str))> = vs
            .iter()
            .flat_map(|v| QUESTIONS.iter().map(move |q| (v, q)))
            .filter(|(v, (qid, _))| {
                !j.is_done(&cell_key("pass-p", &cell.provider, &cell.model, v.id, qid))
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
        let Some(wire) = wire_for(cell) else { continue };
        for (v, (qid, question)) in todo {
            run_one(
                &mut j,
                wire,
                &agent,
                cell,
                "pass-p",
                v.id,
                &v.shape,
                qid,
                0,
                question,
                false,
                CTX,
                &memories,
                &paths,
                &["\n\n".to_string()],
                Think::Off,
                crate::sweep::budget(),
            );
        }
        unload_all(&agent, "http://127.0.0.1:11434");
    }
    summarize(dir);
    Ok(())
}

/// Per-variant scoreboard. The counts are mechanical; whether a command
/// is *good* is a grading question, not one this table answers.
pub fn summarize(dir: &Path) {
    let rows: Vec<_> = Journal::rows(dir)
        .into_iter()
        .filter(|r| r.pass == "pass-p")
        .collect();
    if rows.is_empty() {
        return;
    }
    let refusal = |t: &str| {
        let l = t.to_lowercase();
        l.contains("i cannot") || l.contains("i can't") || l.contains("i am an ai") ||
            l.contains("unable to") || l.contains("i'm not able")
    };

    println!(
        "\n  {:<22} {:>6} {:>7} {:>7} {:>8} {:>8} {:>9}",
        "variant", "n", "CMD:", "empty", "mem-only", "refusal", "explain-ok"
    );
    let mut by: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for r in &rows {
        by.entry(r.shape.clone()).or_default().push(r);
    }
    for v in variants() {
        let Some(rs) = by.get(v.id) else { continue };
        let n = rs.len();
        let answering: Vec<_> = rs.iter().filter(|r| r.step != "explain").collect();
        // The control: "explain" must yield prose and NO command.
        let expl: Vec<_> = rs.iter().filter(|r| r.step == "explain").collect();
        let expl_ok = expl.iter().filter(|r| r.command.is_none()).count();
        println!(
            "  {:<22} {n:>6} {:>7} {:>7} {:>8} {:>8} {:>9}",
            v.id,
            format!(
                "{}/{}",
                answering.iter().filter(|r| r.has_cmd_tag).count(),
                answering.len()
            ),
            rs.iter()
                .filter(|r| r.text.trim().is_empty() && r.command.is_none())
                .count(),
            rs.iter().filter(|r| !r.remembers.is_empty()).count(),
            rs.iter().filter(|r| refusal(&r.raw)).count(),
            format!("{}/{}", expl_ok, expl.len()),
        );
    }
    println!(
        "\n  CMD: counts exclude the 'explain' control, which SHOULD have no\n  \
         command; explain-ok counts how often that was respected.\n"
    );
    for v in variants() {
        println!("  {:<22} {}", v.id, v.hypothesis);
    }
}
