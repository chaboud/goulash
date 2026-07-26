use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Flat, slot-limited durable memory (wiki: architecture/agent-memory.md).
/// Lives in ~/.goulash/memory.toml — enabled state and limits persist
/// there too, so `#/memory on` sticks across sessions. The whole store is
/// baked into the prompt's stable prefix when enabled; changes are rare,
/// so the prefix cache mostly survives.
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
            enabled: false,
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

    /// The stable-prefix block: pinned memories plus the model's tool
    /// instructions. Empty when disabled.
    pub fn context_block(&self) -> String {
        if !self.enabled {
            return String::new();
        }
        let mut s = format!(
            "Pinned memories \u{2014} yours to manage ({}/{} slots, \u{2264}{} chars \
             each). Save one by outputting a line 'REMEMBER: <note>'. Delete one \
             with 'FORGET: <id>'. To revise one, output both: 'FORGET: <id>' plus \
             a 'REMEMBER:' line with the new text.\n",
            self.slots.len(),
            self.limit,
            self.max_chars
        );
        for slot in &self.slots {
            s.push_str(&format!("  [{}] {}\n", slot.id, slot.text));
        }
        s.push('\n');
        s
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
        m.enabled = false;
        assert!(m.context_block().is_empty());
    }
}
