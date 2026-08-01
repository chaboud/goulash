//! Flat, slot-limited durable memory (wiki: architecture/agent-memory.md).
//! Lives in ~/.goulash/memory.toml — enabled state and limits persist
//! there too, so `#/memory on` sticks across sessions. The whole store is
//! baked into the prompt's stable prefix when enabled; changes are rare,
//! so the prefix cache mostly survives — and the few slots that bear on
//! the question are restated next to it, where a sliding-window model
//! will actually look.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Characters the memory cards may spend next to the question. Same
/// reasoning as the pin cards (context.rs): this rides in the volatile
/// suffix and is re-prefilled on every ask, so it stays small.
const CARD_BUDGET: usize = 400;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Slot {
    pub id: u64,
    pub text: String,
    pub at: String,
    pub by: String, // "user" | "llm"
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryStore {
    pub enabled: bool,
    pub limit: usize,
    pub max_chars: usize,
    next_id: u64,
    pub slots: Vec<Slot>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self {
            // ON. It was off, and off is not neutral: the prompt said
            // nothing about memory at all, so a model asked to remember
            // something invented a file to write it to and said it had
            // saved it. Either state costs stable-prefix tokens and
            // either state is a rebuild to change; the one that is worth
            // paying for is the one where the feature works.
            enabled: true,
            // Sized for machine volume, not hand-curation: #/study writes
            // far more than a person would. Still the user's to set
            // (`#/memory limit N`, persisted).
            limit: 50,
            max_chars: 240,
            next_id: 1,
            slots: Vec::new(),
            path: None,
        }
    }
}

impl MemoryStore {
    pub fn load(dir: Option<PathBuf>) -> MemoryStore {
        let path = dir.map(|d| d.join("memory.toml"));
        let mut store: MemoryStore = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default();
        store.path = path;
        store
    }

    fn save(&self) {
        if let Some(p) = &self.path {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(text) = toml::to_string(self) {
                let _ = std::fs::write(p, text);
            }
        }
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        self.save();
    }

    pub fn set_limit(&mut self, n: usize) {
        self.limit = n.clamp(1, 200);
        self.save();
    }

    pub fn add(&mut self, text: &str, by: &str) -> Result<u64, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("empty memory".to_string());
        }
        if self.slots.len() >= self.limit {
            return Err(format!(
                "memory full ({}/{} slots)",
                self.slots.len(),
                self.limit
            ));
        }
        let clipped: String = text.chars().take(self.max_chars).collect();
        let id = self.next_id;
        self.next_id += 1;
        self.slots.push(Slot {
            id,
            text: clipped,
            at: crate::engine::local_now(),
            by: by.to_string(),
        });
        self.save();
        Ok(id)
    }

    pub fn delete(&mut self, id: u64) -> bool {
        let before = self.slots.len();
        self.slots.retain(|s| s.id != id);
        let removed = self.slots.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    pub fn modify(&mut self, id: u64, text: &str) -> bool {
        let max = self.max_chars;
        let now = crate::engine::local_now();
        let mut found = false;
        for s in &mut self.slots {
            if s.id == id {
                s.text = text.trim().chars().take(max).collect();
                s.at = now.clone();
                found = true;
                break;
            }
        }
        if found {
            self.save();
        }
        found
    }

    pub fn find(&self, query: &str) -> Vec<&Slot> {
        let q = query.to_lowercase();
        self.slots
            .iter()
            .filter(|s| q.is_empty() || s.text.to_lowercase().contains(&q))
            .collect()
    }

    pub fn status_line(&self) -> String {
        format!(
            "memory {} \u{b7} {}/{} slots \u{b7} \u{2264}{} chars each",
            if self.enabled { "on" } else { "off" },
            self.slots.len(),
            self.limit,
            self.max_chars
        )
    }

    /// The stable-prefix block: the notes, then the protocol for
    /// managing them. Empty when disabled.
    ///
    /// Notes first, deliberately. This used to lead with four lines
    /// explaining `REMEMBER:`/`FORGET:` before reaching a single fact,
    /// so the one thing that mattered was the fifth thing the model
    /// read. Instructions are for the rare turn that writes a memory;
    /// the notes are for every turn.
    pub fn context_block(&self) -> String {
        if !self.enabled {
            // Not silence. Told nothing about memory, a model asked to
            // remember something invents a way to do it: the observed
            // answer was `echo "User likes cats" > ~/.goulash_memory.txt`
            // with "I saved your preference to a file for reference"
            // underneath — a claim to have done something goulash does
            // not do, about a file nothing will ever read. One line of
            // stable prefix buys a true answer and names the control.
            // Worded to be FOLLOWED, not quoted. An earlier draft said
            // "answer with NO CMD: line" and the model put the phrase
            // "NO CMD:" in the band, verbatim — an instruction that
            // names its own tag invites the model to echo the tag.
            return "goulash has a memory store and it is currently OFF, so \
                    you cannot save anything. If the user asks you to \
                    remember or forget something: say plainly that memory \
                    is off, and that '#/memory on' turns it on. Suggest no \
                    command for it. Writing a note into a file is not \
                    remembering \u{2014} nothing would ever read that file, \
                    and goulash does not run commands anyway. Never claim \
                    you have saved anything.\n\n"
                .to_string();
        }
        let mut s = String::from(
            "Remembered about this user and this machine \u{2014} prefer these \
             over general knowledge when they conflict:\n",
        );
        for slot in &self.slots {
            s.push_str(&format!("  [{}] {}\n", slot.id, slot.text));
        }
        if self.slots.is_empty() {
            s.push_str("  (nothing yet)\n");
        }
        s.push_str(&format!(
            "These are yours to manage ({}/{} slots, \u{2264}{} chars each). Save \
             one by outputting a line 'REMEMBER: <note>'. Delete one with \
             'FORGET: <id>'. To revise one, output both: 'FORGET: <id>' plus a \
             'REMEMBER:' line with the new text. That line IS the save: \
             asked to remember something, write it and suggest no command \
             for it. A note echoed into a file is not remembered \
             \u{2014} goulash does not run commands, and nothing would read \
             that file back \u{2014} and never say you have saved something \
             unless you wrote the line.\n\n",
            self.slots.len(),
            self.limit,
            self.max_chars
        ));
        s
    }

    /// The near-question block: the slots most likely to bear on THIS
    /// question, restated where a sliding-window model will actually
    /// look.
    ///
    /// The prefix copy above is complete and cache-warm, and it also
    /// sits at the furthest point in the prompt from the question —
    /// which is the exact argument that produced the pin cards, applied
    /// to the store we left out of that reasoning. Field case: a slot
    /// recording that macOS `du` wants `-d <depth>` sat in the prefix
    /// while the model suggested `--max-depth=1` twice in a row.
    ///
    /// Ranking is keyword overlap with the question, newest first on a
    /// tie. Crude next to embeddings, and it has the properties that
    /// matter here: instant, no engine, cannot fail.
    pub fn cards_block(&self, question: &str) -> String {
        if !self.enabled || self.slots.is_empty() {
            return String::new();
        }
        let words: Vec<String> = question
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 3)
            .map(|w| w.to_string())
            .collect();
        let mut ranked: Vec<(usize, &Slot)> = self
            .slots
            .iter()
            .map(|s| {
                let text = s.text.to_lowercase();
                let hits = words.iter().filter(|w| text.contains(w.as_str())).count();
                (hits, s)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.id.cmp(&a.1.id)));
        let mut left = CARD_BUDGET;
        let mut body = String::new();
        for (_, slot) in ranked {
            let line = format!("  [{}] {}\n", slot.id, slot.text);
            let n = line.chars().count();
            // `continue`, not `break`: a long note that does not fit
            // must not shut out the shorter ones behind it.
            if n > left {
                continue;
            }
            left -= n;
            body.push_str(&line);
        }
        if body.is_empty() {
            return String::new();
        }
        format!("Remembered, most relevant to this question first:\n{body}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryStore;

    fn store() -> MemoryStore {
        MemoryStore {
            enabled: true,
            ..Default::default()
        }
    }

    /// The field case that produced this block: the note was in the
    /// prompt the whole time, at the top of the stable prefix, and the
    /// model suggested GNU `--max-depth=1` on macOS twice running.
    #[test]
    fn the_relevant_memory_rides_next_to_the_question() {
        let mut m = store();
        m.add(
            "Don't forget to update the log file after each command.",
            "user",
        )
        .unwrap();
        m.add(
            "The correct flag for limiting directory depth in du is -d <depth>.",
            "user",
        )
        .unwrap();
        m.add("Sorted file listing command for .so files.", "user")
            .unwrap();
        let block = m.cards_block("biggest directories with du");
        let du = block.find("-d <depth>").expect("the du note is present");
        let log = block.find("update the log file").unwrap_or(usize::MAX);
        assert!(du < log, "relevant first, not id order: {block}");
    }

    #[test]
    fn cards_are_bounded_and_absent_when_they_would_be_empty() {
        let mut m = store();
        assert!(m.cards_block("anything").is_empty(), "no slots, no block");
        for i in 0..40 {
            m.add(&format!("note {i} {}", "x".repeat(60)), "user").ok();
        }
        let block = m.cards_block("note");
        assert!(
            block.chars().count() < super::CARD_BUDGET + 120,
            "cards blew the budget: {} chars",
            block.chars().count()
        );
        m.set_enabled(false);
        assert!(m.cards_block("note").is_empty(), "disabled costs nothing");
    }

    /// The notes lead. This block used to open with four lines of
    /// REMEMBER/FORGET protocol, so the first fact was the fifth thing
    /// the model read.
    #[test]
    fn the_prefix_block_puts_notes_before_protocol() {
        let mut m = store();
        m.add("deploy is make release TARGET=prod", "user").unwrap();
        let b = m.context_block();
        assert!(
            b.find("TARGET=prod").unwrap() < b.find("REMEMBER:").unwrap(),
            "notes before protocol: {b}"
        );
    }

    #[test]
    fn add_find_delete_modify() {
        let mut m = store();
        let id = m
            .add("deploy needs make release TARGET=prod", "user")
            .unwrap();
        assert_eq!(m.find("deploy").len(), 1);
        assert!(m.modify(id, "deploy: make release TARGET=prod (signing!)"));
        assert!(m.find("signing").len() == 1);
        assert!(m.delete(id));
        assert!(m.find("").is_empty());
        assert!(!m.delete(id));
    }

    #[test]
    fn limit_and_clip() {
        let mut m = store();
        m.limit = 2;
        m.max_chars = 10;
        m.add("0123456789abcdef", "llm").unwrap();
        assert_eq!(m.slots[0].text.chars().count(), 10);
        m.add("two", "llm").unwrap();
        assert!(m.add("three", "llm").is_err());
    }

    #[test]
    fn context_block_shape() {
        let mut m = store();
        m.add("a note", "user").unwrap();
        let b = m.context_block();
        assert!(b.contains("REMEMBER:"));
        assert!(b.contains("[1] a note"));
        // Off is not silent. Saying nothing about memory at all left a
        // model asked to remember something inventing a file to write
        // it to, and claiming it had — so the off state says so, and
        // names the control that changes it.
        m.enabled = false;
        let off = m.context_block();
        assert!(off.contains("OFF"), "{off}");
        assert!(off.contains("#/memory on"), "{off}");
        assert!(!off.contains("[1] a note"), "off means the slots are not sent");
        assert!(!off.contains("REMEMBER:"), "and the verb is not offered");
    }
}
