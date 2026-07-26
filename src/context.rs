//! `#@` working context: files pinned into the prompt's stable prefix.
//! (wiki: architecture/working-context.md)
//!
//! The killer case is a vendor-authored command guide. Drop a
//! `commandRef.md` next to a proprietary CLI and the model can suggest
//! correct invocations for a tool it has never seen. That is *kind of*
//! tool use — no function-calling protocol, no schema negotiation, just
//! the reference material in front of the model when it writes a `CMD:`
//! line. Nothing about the autonomy model changes: goulash still only
//! suggests, and the user still runs.
//!
//! The capability granted is **read-only, and goulash performs it, not
//! the shell**: stat a path, list a directory, read a file. No execution
//! ever, so a mis-resolved pin costs a wasted read, not a side effect.
//! That is what lets the natural-language form exist without an approval
//! prompt in front of it.

use std::path::{Path, PathBuf};

/// Hard ceiling on bytes read from any one file, before any budgeting.
/// Stops `#@ /var/log/everything` from becoming a memory problem while
/// we are still deciding whether it is a context problem.
const READ_CAP: usize = 512 * 1024;

/// Directory walk limits. A tree pin is a convenience, not a crawler.
const WALK_MAX_FILES: usize = 64;
const WALK_MAX_DEPTH: usize = 3;
const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    "build",
    ".next",
    ".cargo",
];

/// How a pin's text is actually reaching the model right now. Derived at
/// emit time from the live budget rather than stored, so it can never
/// disagree with what was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The text as-is.
    Verbatim,
    /// Structure kept, prose dropped — headings, fences, flags, tables.
    Outline,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Verbatim => "verbatim",
            Tier::Outline => "outline",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Pin {
    pub id: u64,
    pub path: PathBuf,
    /// What the chrome and the listing call it — a basename, or a
    /// basename with a trailing `/` for a tree.
    pub label: String,
    /// Size and mtime as of the last ingest; a cheap `stat` at prompt
    /// turns compares against these to set `dirty`.
    pub size: u64,
    pub mtime: u64,
    /// Everything we read, capped at READ_CAP. The emitted form is
    /// computed from this against the live budget.
    raw: String,
    /// The file changed under us. Goulash does not watch the filesystem
    /// and pounce — this is a marker, and re-cooking is the user's call.
    pub dirty: bool,
}

impl Pin {
    /// What this pin contributes to the prompt, given its share of the
    /// budget: verbatim if it fits, structure-only if it does not.
    pub fn emit(&self, share: usize) -> (Tier, String) {
        if self.raw.chars().count() <= share {
            return (Tier::Verbatim, self.raw.clone());
        }
        (Tier::Outline, outline(&self.raw, share))
    }
}

#[derive(Debug)]
pub struct WorkContext {
    pub pins: Vec<Pin>,
    /// Total characters all pins together may spend in the prefix.
    pub max_chars: usize,
    next_id: u64,
}

impl WorkContext {
    pub fn new(max_chars: usize) -> WorkContext {
        WorkContext {
            pins: Vec::new(),
            max_chars,
            next_id: 1,
        }
    }

    /// Each pin's fair share of the budget. Equal shares beat clever
    /// weighting here: the user picked these files deliberately, and a
    /// scheme that quietly starves one of them is worse than one that
    /// outlines both.
    fn share(&self) -> usize {
        if self.pins.is_empty() {
            return self.max_chars;
        }
        self.max_chars / self.pins.len()
    }

    /// Pin a path, replacing any existing pin on the same path (that is
    /// what a re-cook is). Returns the human-facing result line.
    pub fn pin(&mut self, path: &Path) -> Result<String, String> {
        let path = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let is_dir = meta.is_dir();
        let (raw, note) = if is_dir {
            read_tree(&path)?
        } else {
            (read_text(&path)?, String::new())
        };
        let label = label_for(&path, is_dir);
        let id = match self.pins.iter().position(|p| p.path == path) {
            Some(i) => {
                let id = self.pins[i].id;
                self.pins.remove(i);
                id
            }
            None => {
                let id = self.next_id;
                self.next_id += 1;
                id
            }
        };
        let chars = raw.chars().count();
        self.pins.push(Pin {
            id,
            path,
            label: label.clone(),
            size: meta.len(),
            mtime: mtime_of(&meta),
            raw,
            dirty: false,
        });
        let (tier, _) = self.pins.last().unwrap().emit(self.share());
        Ok(format!(
            "@ {label} \u{b7} {chars} chars \u{b7} {}{note}",
            tier.as_str()
        ))
    }

    pub fn clear(&mut self) -> usize {
        let n = self.pins.len();
        self.pins.clear();
        n
    }

    pub fn drop_id(&mut self, id: u64) -> Option<String> {
        let i = self.pins.iter().position(|p| p.id == id)?;
        Some(self.pins.remove(i).label)
    }

    /// A cheap `stat` per pin, run at prompt turns. Sets the dirty
    /// marker; never re-reads on its own.
    pub fn refresh_dirty(&mut self) {
        for pin in &mut self.pins {
            if let Ok(meta) = std::fs::metadata(&pin.path) {
                pin.dirty = meta.len() != pin.size || mtime_of(&meta) != pin.mtime;
            } else {
                pin.dirty = true; // gone counts as changed
            }
        }
    }

    /// The block that rides in the stable prefix, next to pinned
    /// memories. Empty when nothing is pinned, so an unused feature
    /// costs exactly zero prompt bytes and zero cache churn.
    pub fn context_block(&self) -> String {
        if self.pins.is_empty() {
            return String::new();
        }
        let share = self.share();
        let mut s = String::from(
            "Working context \u{2014} files the user pinned as relevant right \
             now. Prefer these over general knowledge when they conflict: \
             they describe THIS user's tools. Some are outlines rather than \
             full text.\n",
        );
        for pin in &self.pins {
            let (tier, body) = pin.emit(share);
            s.push_str(&format!(
                "--- @{} ({}, {}{}) ---\n{}\n",
                pin.label,
                pin.path.display(),
                tier.as_str(),
                if pin.dirty { ", CHANGED ON DISK" } else { "" },
                body.trim_end()
            ));
        }
        s.push('\n');
        s
    }

    /// The chrome marker: the active `@` at a glance, `+N` for the rest,
    /// `*` when something changed under us.
    pub fn chrome_tag(&self) -> Option<String> {
        let first = self.pins.first()?;
        let extra = self.pins.len() - 1;
        let dirty = if self.pins.iter().any(|p| p.dirty) {
            "*"
        } else {
            ""
        };
        Some(if extra > 0 {
            format!("@{}+{extra}{dirty}", first.label)
        } else {
            format!("@{}{dirty}", first.label)
        })
    }

    /// One line per pin for `#@` with no argument.
    pub fn list(&self) -> Vec<String> {
        let share = self.share();
        self.pins
            .iter()
            .map(|p| {
                let (tier, _) = p.emit(share);
                format!(
                    "[{}] {} \u{b7} {} chars \u{b7} {}{}",
                    p.id,
                    p.path.display(),
                    p.raw.chars().count(),
                    tier.as_str(),
                    if p.dirty { " \u{b7} changed *" } else { "" }
                )
            })
            .collect()
    }

    /// A cheap directory listing for the model to resolve a
    /// natural-language pin against. Names only — resolution needs
    /// candidates, not contents.
    pub fn candidates(dir: &Path) -> String {
        let mut names: Vec<String> = match std::fs::read_dir(dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    if e.path().is_dir() { format!("{n}/") } else { n }
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names.truncate(120);
        names.join("  ")
    }
}

fn mtime_of(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn label_for(path: &Path, is_dir: bool) -> String {
    let base = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    if is_dir { format!("{base}/") } else { base }
}

/// Read a file as text, refusing what is obviously not. A binary in the
/// prompt is a hundred wasted tokens and a confused model.
fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = &bytes[..bytes.len().min(READ_CAP)];
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Err(format!("{}: looks binary", path.display()));
    }
    Ok(String::from_utf8_lossy(bytes).to_string())
}

/// Walk a tree into one text: a file list, then each readable file's
/// content. Bounded hard — this is a convenience, not a crawler, and the
/// budget will outline it anyway.
fn read_tree(root: &Path) -> Result<(String, String), String> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect(root, 0, &mut files);
    let hit_cap = files.len() >= WALK_MAX_FILES;
    files.sort();
    let mut s = format!("Tree {} ({} files):\n", root.display(), files.len());
    for f in &files {
        s.push_str(&format!("  {}\n", f.strip_prefix(root).unwrap_or(f).display()));
    }
    for f in &files {
        if let Ok(text) = read_text(f) {
            s.push_str(&format!(
                "\n== {} ==\n{}\n",
                f.strip_prefix(root).unwrap_or(f).display(),
                text
            ));
        }
    }
    let note = if hit_cap {
        format!(" \u{b7} capped at {WALK_MAX_FILES} files")
    } else {
        String::new()
    };
    Ok((s, note))
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > WALK_MAX_DEPTH || out.len() >= WALK_MAX_FILES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.filter_map(|e| e.ok()) {
        if out.len() >= WALK_MAX_FILES {
            return;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                collect(&path, depth + 1, out);
            }
        } else {
            out.push(path);
        }
    }
}

/// Structure-preserving compression: keep the parts of a document that
/// tell you how to invoke something, drop the parts that explain why.
/// Truncation would cut a command table in half; this keeps the table
/// and loses the prose around it.
///
/// Deterministic on purpose — it is instant, it works with no engine
/// bound, and it gives the pin something useful to say the moment it is
/// made. An LLM digest is the better answer for a large document and is
/// the next tier to build; this is the floor beneath it.
pub fn outline(text: &str, budget: usize) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            keep(&mut out, line, budget);
            continue;
        }
        let interesting = in_fence
            || t.starts_with('#')
            || t.starts_with("==")
            || t.starts_with('|')
            || t.starts_with("--")
            || t.contains(" --")
            || t.contains('`')
            || t.starts_with('$')
            || (t.starts_with("  ") && t.contains('/'));
        if interesting {
            keep(&mut out, line, budget);
        }
        if out.chars().count() >= budget {
            break;
        }
    }
    if out.trim().is_empty() {
        // Nothing structural to hold on to: fall back to the head, which
        // is at least where a document introduces itself.
        out = text.chars().take(budget).collect();
    }
    out.push_str("\n[outline: prose omitted]\n");
    out
}

fn keep(out: &mut String, line: &str, budget: usize) {
    if out.chars().count() + line.chars().count() + 1 > budget {
        return;
    }
    out.push_str(line);
    out.push('\n');
}

/// Line-protocol verbs the model uses to act on the working context,
/// alongside the existing `CMD:` / `REMEMBER:` / `FORGET:`.
/// `PIN: <path>` pins, `PINCLEAR` drops everything.
pub fn extract_pin_ops(answer: &str) -> (String, Vec<String>, bool) {
    let mut rest = String::new();
    let mut pins = Vec::new();
    let mut clear = false;
    for line in answer.lines() {
        let t = line.trim();
        if let Some(p) = t.strip_prefix("PIN:") {
            let p = p.trim().trim_matches(['"', '\'', '`']);
            if !p.is_empty() {
                pins.push(p.to_string());
            }
        } else if t == "PINCLEAR" {
            clear = true;
        } else {
            rest.push_str(line);
            rest.push('\n');
        }
    }
    (rest.trim().to_string(), pins, clear)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("goulash-ctx-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn a_small_file_rides_verbatim_and_shows_in_the_chrome() {
        let d = tmpdir("small");
        let f = d.join("commandRef.md");
        std::fs::write(&f, "# widgetctl\n\nUse `widgetctl sync --all`.\n").unwrap();
        let mut wc = WorkContext::new(4000);
        let msg = wc.pin(&f).unwrap();
        assert!(msg.contains("verbatim"), "{msg}");
        let block = wc.context_block();
        assert!(block.contains("widgetctl sync --all"), "{block}");
        assert!(block.contains("Working context"));
        assert_eq!(wc.chrome_tag().as_deref(), Some("@commandRef.md"));
    }

    #[test]
    fn nothing_pinned_costs_nothing() {
        let wc = WorkContext::new(4000);
        assert!(wc.context_block().is_empty());
        assert_eq!(wc.chrome_tag(), None);
    }

    #[test]
    fn an_oversized_file_keeps_its_commands_and_drops_its_prose() {
        let d = tmpdir("big");
        let f = d.join("guide.md");
        let mut text = String::from("# Guide\n");
        for i in 0..400 {
            text.push_str(&format!(
                "This is a long paragraph of explanatory prose number {i} \
                 that carries no invocation at all and should not survive.\n"
            ));
        }
        text.push_str("| sync | `widgetctl sync --all` |\n");
        std::fs::write(&f, &text).unwrap();
        let mut wc = WorkContext::new(600);
        let msg = wc.pin(&f).unwrap();
        assert!(msg.contains("outline"), "{msg}");
        let block = wc.context_block();
        assert!(block.contains("widgetctl sync --all"), "{block}");
        assert!(!block.contains("explanatory prose number 5"), "{block}");
        assert!(block.contains("# Guide"));
    }

    #[test]
    fn re_pinning_the_same_path_replaces_rather_than_duplicates() {
        let d = tmpdir("recook");
        let f = d.join("ref.md");
        std::fs::write(&f, "# one\n").unwrap();
        let mut wc = WorkContext::new(4000);
        wc.pin(&f).unwrap();
        std::fs::write(&f, "# two\n").unwrap();
        wc.pin(&f).unwrap();
        assert_eq!(wc.pins.len(), 1, "re-cook must not stack duplicates");
        assert_eq!(wc.pins[0].id, 1, "and it keeps its identity");
        assert!(wc.context_block().contains("# two"));
    }

    #[test]
    fn a_changed_file_is_marked_not_reloaded() {
        let d = tmpdir("dirty");
        let f = d.join("ref.md");
        std::fs::write(&f, "# before\n").unwrap();
        let mut wc = WorkContext::new(4000);
        wc.pin(&f).unwrap();
        std::fs::write(&f, "# after, and longer than before\n").unwrap();
        wc.refresh_dirty();
        assert!(wc.pins[0].dirty);
        // Stale beats stalled: the OLD text still serves, labelled.
        let block = wc.context_block();
        assert!(block.contains("# before"), "{block}");
        assert!(block.contains("CHANGED ON DISK"), "{block}");
        assert!(wc.chrome_tag().unwrap().ends_with('*'));
    }

    #[test]
    fn budget_is_shared_between_pins() {
        let d = tmpdir("share");
        let a = d.join("a.md");
        let b = d.join("b.md");
        let body = "x".repeat(500);
        std::fs::write(&a, &body).unwrap();
        std::fs::write(&b, &body).unwrap();
        let mut wc = WorkContext::new(1200);
        wc.pin(&a).unwrap();
        // Alone, 500 chars fits the 1200 budget.
        assert_eq!(wc.pins[0].emit(wc.share()).0, Tier::Verbatim);
        wc.pin(&b).unwrap();
        // Two pins, 600 each: still fits, but the share really did move.
        assert_eq!(wc.share(), 600);
    }

    #[test]
    fn binaries_are_refused_rather_than_pasted() {
        let d = tmpdir("bin");
        let f = d.join("thing.bin");
        std::fs::write(&f, [0u8, 1, 2, 3, 0, 5]).unwrap();
        let mut wc = WorkContext::new(4000);
        assert!(wc.pin(&f).unwrap_err().contains("binary"));
        assert!(wc.pins.is_empty());
    }

    #[test]
    fn missing_paths_fail_without_pinning() {
        let mut wc = WorkContext::new(4000);
        assert!(wc.pin(Path::new("/definitely/not/here.md")).is_err());
        assert!(wc.pins.is_empty());
    }

    #[test]
    fn pin_verbs_are_split_out_of_the_prose() {
        let (rest, pins, clear) =
            extract_pin_ops("Found it.\nPIN: ./docs/eero.md\nPIN: `b.md`\nmore prose");
        assert_eq!(pins, vec!["./docs/eero.md", "b.md"]);
        assert!(!clear);
        assert_eq!(rest, "Found it.\nmore prose");
        let (_, _, clear) = extract_pin_ops("dropping those\nPINCLEAR");
        assert!(clear);
    }

    #[test]
    fn a_tree_pins_as_a_listing_plus_contents() {
        let d = tmpdir("tree");
        let sub = d.join("deep");
        let _ = std::fs::create_dir_all(&sub);
        let _ = std::fs::create_dir_all(d.join("target"));
        std::fs::write(d.join("one.md"), "# one\n").unwrap();
        std::fs::write(sub.join("two.md"), "# two\n").unwrap();
        std::fs::write(d.join("target/skip.md"), "# nope\n").unwrap();
        let mut wc = WorkContext::new(40_000);
        let msg = wc.pin(&d).unwrap();
        assert!(msg.starts_with("@ "), "{msg}");
        let block = wc.context_block();
        assert!(block.contains("# one") && block.contains("# two"), "{block}");
        assert!(!block.contains("# nope"), "build dirs must be skipped");
        assert!(wc.chrome_tag().unwrap().contains('/'));
    }
}
