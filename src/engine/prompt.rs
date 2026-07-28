//! Prompt assembly and answer parsing.
//!
//! Split out of `engine.rs` so the characterization bench can vary the
//! prompt *shape* — the ordering levers — while still building the byte
//! sequence the product actually sends. [`PromptShape::default()`]
//! reproduces the shipped prompt exactly.

/// Byte-stable preamble: identical across asks so the provider's KV
/// prefix cache (ollama caches against the previous request) re-uses the
/// preamble + unchanged session-log prefix; only the appended tail and
/// the question get re-evaluated.
pub const PREAMBLE: &str = "You are goulash, an assistant living in the user's \
terminal status bar. Answer tersely in ONE short line of plain text, no \
markdown. Each command carries the local time it ran; treat old output as \
stale. The log also contains the running conversation: '#' lines are \
earlier user questions, 'goulash:' lines are your earlier replies, and \
'CMD:' lines are commands you suggested — follow-up questions refer back \
to them.\n\n";

/// Where pinned memories sit relative to the session log.
///
/// This is a *cache* lever, not a wording one. `memory.rs`'s block leads
/// with a live `(N/25 slots)` count, so under [`MemPos::BeforeLog`] every
/// REMEMBER/FORGET rewrites a string that sits in front of the whole
/// session log — invalidating the prefix behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemPos {
    /// Shipped shape: between the preamble and the session log.
    #[default]
    BeforeLog,
    /// After the log but still inside the stable prefix.
    AfterLog,
    /// In the volatile suffix, alongside the time and question.
    Suffix,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PromptShape {
    pub memories: MemPos,
    /// Ask for `CMD:` before the prose line. The command is the payload;
    /// emitting it first means truncation eats the explanation instead of
    /// the command, and the suggestion chip can populate mid-stream.
    pub command_first: bool,
}

/// The contract, repeated at point-of-use: small models lose instructions
/// that appear only at the top of a long prompt. Two modes of ingress get
/// two contracts — a `#` ask is usually fishing for a runnable command,
/// while unprompted commentary has to earn its CMD line.
pub fn directive(command_first: bool, proactive: bool) -> &'static str {
    match (command_first, proactive) {
        (false, true) => {
            "Reply with ONE short prose line. Add a second line formatted \
             exactly as: CMD: <command> ONLY if a genuinely useful next \
             command exists."
        }
        (false, false) => {
            "Reply with ONE short prose line. If any shell command could \
             accomplish, fix, or demonstrate what was asked, you MUST add a \
             second line formatted exactly as: CMD: <command>"
        }
        (true, true) => {
            "If a genuinely useful next command exists, put it FIRST on a \
             line formatted exactly as: CMD: <command>. Then reply with ONE \
             short prose line. Most observations need no command."
        }
        (true, false) => {
            "If any shell command could accomplish, fix, or demonstrate what \
             was asked, you MUST put it FIRST on a line formatted exactly \
             as: CMD: <command>. Then add ONE short prose line."
        }
    }
}

/// Assemble the prompt. Volatile parts (current time, question) go AFTER
/// the stable prefix so the KV cache keeps the expensive part.
///
/// `now` is a parameter rather than a call to [`local_now`] so a harness
/// can freeze it: `Current local time:` sits in the volatile suffix, and
/// a live clock makes two runs of the same cell produce different bytes,
/// which would drift every latency comparison.
pub fn build_prompt(
    shape: &PromptShape,
    memories: &str,
    context: &str,
    question: &str,
    now: &str,
    proactive: bool,
) -> String {
    let d = directive(shape.command_first, proactive);
    let (before_log, after_log, in_suffix) = match shape.memories {
        MemPos::BeforeLog => (memories, "", ""),
        MemPos::AfterLog => ("", memories, ""),
        MemPos::Suffix => ("", "", memories),
    };
    format!(
        "{PREAMBLE}{before_log}Session log (oldest first):\n{context}\n{after_log}\
         Current local time: {now}\n{in_suffix}Question: {question}\n{d}\nAnswer:"
    )
}

fn tm_now() -> libc::tm {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as libc::time_t;
    // SAFETY: localtime_r fills the tm struct; zeroed is a valid init.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    tm
}

/// "2026-07-21 01:10:53" — volatile, kept out of the stable prefix.
pub fn local_now() -> String {
    let tm = tm_now();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// "01:10:53" — stamped onto session-log block headers (stable once
/// written, so it doesn't break the prefix cache).
pub fn hms() -> String {
    let tm = tm_now();
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// Pull REMEMBER:/FORGET: tool lines out of an answer; returns the
/// remaining text plus the requested memory operations.
pub fn extract_memory_ops(raw: &str) -> (String, Vec<String>, Vec<u64>) {
    let mut rest = Vec::new();
    let mut remembers = Vec::new();
    let mut forgets = Vec::new();
    for line in raw.lines() {
        let l = line.trim();
        if let Some(note) = l.strip_prefix("REMEMBER:") {
            if !note.trim().is_empty() {
                remembers.push(note.trim().to_string());
            }
        } else if let Some(id) = l.strip_prefix("FORGET:") {
            if let Ok(n) = id.trim().trim_matches(['[', ']']).parse::<u64>() {
                forgets.push(n);
            }
        } else {
            rest.push(line);
        }
    }
    (rest.join("\n"), remembers, forgets)
}

/// Split a raw answer into (prose, candidate command): the first
/// non-empty non-CMD line is the prose (one-line contract enforced
/// here), and the first `CMD: ...` line is the command. Small models
/// often reply with a bare command and no tag, so a fallback treats a
/// short line whose first word is a PATH executable as the command.
///
/// Order-independent by construction, so it already parses the
/// `command_first` shape without changes.
pub fn split_answer(
    raw: &str,
    path_set: &std::collections::HashSet<String>,
) -> (String, Option<String>) {
    let mut text = String::new();
    let mut command = None;
    for line in raw.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(c) = l.strip_prefix("CMD:") {
            if command.is_none() && !c.trim().is_empty() {
                command = Some(c.trim().to_string());
            }
        } else if text.is_empty() {
            text = l.to_string();
        }
    }
    if command.is_none() {
        for line in raw.lines().take(4) {
            let l = line.trim().trim_matches('`');
            let words: Vec<&str> = l.split_whitespace().collect();
            if !l.is_empty()
                && !l.starts_with('#')
                && (1..=8).contains(&words.len())
                && !l.ends_with(['.', '!', '?'])
                && path_set.contains(words[0])
            {
                command = Some(l.to_string());
                break;
            }
        }
    }
    (text, command)
}

#[cfg(test)]
mod answer_tests {
    use super::split_answer;
    use std::collections::HashSet;

    fn paths(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn text_only() {
        assert_eq!(
            split_answer("It is Tuesday.\n", &paths(&[])),
            ("It is Tuesday.".into(), None)
        );
    }

    #[test]
    fn text_and_command() {
        let (t, c) = split_answer(
            "Disk is mostly node_modules.\nCMD: du -sh * | sort -h\n",
            &paths(&[]),
        );
        assert_eq!(t, "Disk is mostly node_modules.");
        assert_eq!(c.as_deref(), Some("du -sh * | sort -h"));
    }

    #[test]
    fn command_first_and_rambling() {
        let (t, c) = split_answer(
            "\nCMD: git pull\nRun this to update.\nExtra ramble.",
            &paths(&[]),
        );
        assert_eq!(t, "Run this to update.");
        assert_eq!(c.as_deref(), Some("git pull"));
    }

    #[test]
    fn bare_command_fallback() {
        // The field case: model answers `ls -lhR` with no CMD: tag.
        let (t, c) = split_answer("ls -lhR", &paths(&["ls", "du"]));
        assert_eq!(t, "ls -lhR");
        assert_eq!(c.as_deref(), Some("ls -lhR"));
        // Prose sentences never trip the fallback.
        let (_, c2) = split_answer("Use the ls command to list.", &paths(&["ls"]));
        assert_eq!(c2, None);
    }
}

#[cfg(test)]
mod shape_tests {
    use super::*;

    const NOW: &str = "2026-07-28 09:00:00";

    /// The default shape must reproduce the shipped prompt byte for byte;
    /// e2e.py asserts against this exact text.
    #[test]
    fn default_shape_matches_shipped_prompt() {
        let got = build_prompt(
            &PromptShape::default(),
            "MEM\n\n",
            "$ ls [exit 0, 01:00:00]",
            "what is here",
            NOW,
            false,
        );
        let want = format!(
            "{PREAMBLE}MEM\n\nSession log (oldest first):\n$ ls [exit 0, 01:00:00]\n\
             Current local time: {NOW}\nQuestion: what is here\n{}\nAnswer:",
            directive(false, false)
        );
        assert_eq!(got, want);
    }

    #[test]
    fn empty_memories_leave_no_residue() {
        for pos in [MemPos::BeforeLog, MemPos::AfterLog, MemPos::Suffix] {
            let shape = PromptShape {
                memories: pos,
                command_first: false,
            };
            let got = build_prompt(&shape, "", "$ ls", "q", NOW, false);
            assert_eq!(
                got,
                build_prompt(&PromptShape::default(), "", "$ ls", "q", NOW, false),
                "{pos:?} with no memories must equal the default shape"
            );
        }
    }

    /// The point of MemPos: moving memories out of the prefix must leave
    /// the preamble+log prefix byte-identical when memories change.
    #[test]
    fn suffix_position_keeps_the_log_prefix_stable() {
        let shape = PromptShape {
            memories: MemPos::Suffix,
            command_first: false,
        };
        let a = build_prompt(&shape, "slots 1/25\n\n", "$ ls", "q", NOW, false);
        let b = build_prompt(&shape, "slots 2/25\n\n", "$ ls", "q", NOW, false);
        let shared = a
            .chars()
            .zip(b.chars())
            .take_while(|(x, y)| x == y)
            .count();
        assert!(
            shared > a.find("Question:").unwrap() - 40,
            "memories in the suffix should diverge only near the tail, \
             diverged at {shared} of {}",
            a.len()
        );

        // Contrast: the shipped shape diverges before the log even starts.
        let c = build_prompt(&PromptShape::default(), "slots 1/25\n\n", "$ ls", "q", NOW, false);
        let d = build_prompt(&PromptShape::default(), "slots 2/25\n\n", "$ ls", "q", NOW, false);
        let shared_default = c
            .chars()
            .zip(d.chars())
            .take_while(|(x, y)| x == y)
            .count();
        assert!(
            shared_default < c.find("Session log").unwrap(),
            "shipped shape should invalidate the prefix before the log"
        );
    }

    #[test]
    fn command_first_flips_the_directive_only() {
        let flipped = PromptShape {
            memories: MemPos::default(),
            command_first: true,
        };
        let a = build_prompt(&PromptShape::default(), "", "$ ls", "q", NOW, false);
        let b = build_prompt(&flipped, "", "$ ls", "q", NOW, false);
        assert_ne!(a, b);
        // Everything up to the directive is shared: the lever must not
        // disturb the cached prefix.
        let head = a.find("Current local time").unwrap();
        assert_eq!(a[..head], b[..head]);
        assert!(b.contains("FIRST"));
    }
}
