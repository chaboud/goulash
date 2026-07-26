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

/// Ceiling on what we will hand a model to compress in one call. Local
/// context windows are small (`num_ctx` defaults to 8192 tokens); a
/// digest request that overflows one is worse than no digest, because it
/// silently truncates at the wrong end.
const DIGEST_SOURCE_CAP: usize = 12_000;

/// Digest attempts per pin before settling for the outline. A model that
/// ignores the target length would otherwise be re-asked on every emit.
const MAX_DIGEST_ATTEMPTS: u8 = 2;

/// Total characters all **cards** together may spend, across every pin.
///
/// The card rides in the *volatile suffix*, right next to the question,
/// which is the only place a pin lands inside a sliding-window model's
/// attention. That position is re-sent on every ask and re-prefilled
/// every time, so it has to stay tiny — a few hundred characters is
/// noise against the prompt, five pins' worth of full digests is not.
/// (wiki: architecture/two-lane-engagement.md)
const CARD_BUDGET: usize = 400;

/// Per-card ceiling, so one verbose pin cannot eat the whole budget.
const CARD_MAX: usize = 240;

/// Card attempts per pin, same reasoning as MAX_DIGEST_ATTEMPTS.
const MAX_CARD_ATTEMPTS: u8 = 2;

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
    /// LLM compression: prose summarised rather than dropped.
    Digest,
    /// Structure kept, prose dropped — headings, fences, flags, tables.
    /// The floor: instant, needs no engine, and always available.
    Outline,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Verbatim => "verbatim",
            Tier::Digest => "digest",
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
    /// LLM compression of `raw`, when one has landed. Strictly better
    /// than the outline for a large document — it can *summarise* prose
    /// instead of dropping it — but it costs a model call, so it arrives
    /// asynchronously behind a deterministic first pass.
    pub digest: Option<String>,
    /// The card: a handful of lines that ride next to the question
    /// rather than in the stable prefix. None until one is written; the
    /// deterministic fallback is computed on demand, so a pin always has
    /// a card even with no engine bound.
    pub card: Option<String>,
    /// A digest or card has been asked for and has not come back. Drives
    /// the chrome meter, and stops us asking twice.
    pub cooking: bool,
    pub card_cooking: bool,
    /// Digest attempts spent. A model that ignores the target would
    /// otherwise be re-asked forever, since the trigger is "the digest
    /// still doesn't fit".
    attempts: u8,
    card_attempts: u8,
    /// The file changed under us. Goulash does not watch the filesystem
    /// and pounce — this is a marker, and re-cooking is the user's call.
    pub dirty: bool,
}

impl Pin {
    /// What this pin contributes to the prompt, given its share of the
    /// budget. Best available wins, and there is always something: the
    /// deterministic outline needs no engine and cannot fail, so a pin
    /// is useful the instant it is made and only gets better.
    pub fn emit(&self, share: usize) -> (Tier, String) {
        if self.raw.chars().count() <= share {
            return (Tier::Verbatim, self.raw.clone());
        }
        if let Some(d) = &self.digest
            && d.chars().count() <= share
        {
            return (Tier::Digest, d.clone());
        }
        (Tier::Outline, outline(&self.raw, share))
    }

    /// The few lines that ride next to the question. Written by a model
    /// when one has answered; otherwise pulled out of the text
    /// deterministically, so a card exists from the instant a pin does.
    pub fn card_text(&self, budget: usize) -> String {
        let budget = budget.min(CARD_MAX);
        match &self.card {
            Some(c) if c.chars().count() <= budget => c.clone(),
            _ => deterministic_card(&self.raw, budget),
        }
    }

    /// What to hand the model to compress. **Not** the raw text — that
    /// can be half a megabyte, and no local context window will take it.
    /// The deterministic outline at a generous multiple of the target is
    /// bounded by construction and has already thrown away the least
    /// useful material, so the model spends its window on the parts
    /// worth keeping.
    pub fn digest_source(&self, target: usize) -> String {
        let room = (target * 4).min(DIGEST_SOURCE_CAP);
        if self.raw.chars().count() <= room {
            return self.raw.clone();
        }
        outline(&self.raw, room)
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
            digest: None,
            card: None,
            cooking: false,
            card_cooking: false,
            attempts: 0,
            card_attempts: 0,
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
             they describe THIS user's tools. Each block says whether it \
             is the full text, a compressed digest, or an outline with \
             prose omitted \u{2014} in the last two, absence of a detail is \
             not evidence it does not exist.\n",
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

    /// Pins that want a digest right now: over their share, with no
    /// digest that fits, nothing already in flight, and attempts left.
    /// Returns `(id, label, source, target)` ready to hand the engine.
    ///
    /// Marks them cooking as it goes, so asking twice is impossible —
    /// the caller is a poll, not an event, and polls repeat.
    pub fn digest_wanted(&mut self) -> Vec<(u64, String, String, usize)> {
        let share = self.share();
        let mut out = Vec::new();
        for pin in &mut self.pins {
            let too_big = pin.raw.chars().count() > share;
            let digest_fits = pin
                .digest
                .as_ref()
                .is_some_and(|d| d.chars().count() <= share);
            if too_big && !digest_fits && !pin.cooking && pin.attempts < MAX_DIGEST_ATTEMPTS {
                pin.cooking = true;
                pin.attempts += 1;
                out.push((pin.id, pin.label.clone(), pin.digest_source(share), share));
            }
        }
        out
    }

    /// A digest came back. An empty one is a failure: the pin keeps its
    /// outline, which is why the outline had to exist first.
    pub fn set_digest(&mut self, id: u64, text: Option<String>) -> Option<String> {
        let share = self.share();
        let pin = self.pins.iter_mut().find(|p| p.id == id)?;
        pin.cooking = false;
        let text = text.filter(|t| !t.trim().is_empty())?;
        let fits = text.chars().count() <= share;
        pin.digest = Some(text);
        Some(format!(
            "@ {} digested{}",
            pin.label,
            if fits { "" } else { " (still over budget)" }
        ))
    }

    /// Abandon everything in flight — `#@/cancel`. The pins keep whatever
    /// they already had.
    pub fn cancel_cooking(&mut self) -> usize {
        let mut n = 0;
        for pin in &mut self.pins {
            if pin.cooking || pin.card_cooking {
                pin.cooking = false;
                pin.card_cooking = false;
                // A cancel is a decision, not a failure: don't spend the
                // attempt, but don't immediately re-queue it either.
                pin.attempts = MAX_DIGEST_ATTEMPTS;
                pin.card_attempts = MAX_CARD_ATTEMPTS;
                n += 1;
            }
        }
        n
    }

    pub fn cooking_count(&self) -> usize {
        self.pins
            .iter()
            .filter(|p| p.cooking || p.card_cooking)
            .count()
    }

    /// The near-question block: a card per pin, newest first, until the
    /// budget runs out.
    ///
    /// Newest-first is the whole ranking. A pin the user just made is
    /// what they are working on; one from twenty minutes ago is
    /// background, and background belongs in the stable prefix where it
    /// is already sitting. Nothing here replaces the prefix copy — this
    /// is a second, much smaller emission at a position the model
    /// actually attends to.
    pub fn cards_block(&self) -> String {
        if self.pins.is_empty() {
            return String::new();
        }
        let mut left = CARD_BUDGET;
        let mut body = String::new();
        for pin in self.pins.iter().rev() {
            if left == 0 {
                break;
            }
            let card = pin.card_text(left);
            if card.trim().is_empty() {
                continue;
            }
            body.push_str(&format!("@{}:\n{}", pin.label, card));
            left = left.saturating_sub(card.chars().count());
        }
        if body.trim().is_empty() {
            return String::new();
        }
        format!("Pinned right now (full text is above):\n{body}\n")
    }

    /// Pins wanting a card written. Every pin wants one — unlike a
    /// digest, this is not conditional on size, because even a small
    /// file benefits from having its three key lines restated next to
    /// the question.
    pub fn card_wanted(&mut self) -> Vec<(u64, String, String, usize)> {
        let mut out = Vec::new();
        for pin in &mut self.pins {
            if pin.card.is_none() && !pin.card_cooking && pin.card_attempts < MAX_CARD_ATTEMPTS {
                pin.card_cooking = true;
                pin.card_attempts += 1;
                out.push((
                    pin.id,
                    pin.label.clone(),
                    pin.digest_source(CARD_MAX),
                    CARD_MAX,
                ));
            }
        }
        out
    }

    pub fn set_card(&mut self, id: u64, text: Option<String>) -> Option<String> {
        let pin = self.pins.iter_mut().find(|p| p.id == id)?;
        pin.card_cooking = false;
        let text = text.filter(|t| !t.trim().is_empty())?;
        pin.card = Some(text.chars().take(CARD_MAX).collect());
        Some(format!("@ {} carded", pin.label))
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
        // A silent multi-second cook is exactly the "am I frozen?"
        // failure we hit with model loads, so ingest reports itself.
        let cooking = self.cooking_count();
        let meter = if cooking > 0 {
            let done = self.pins.len() - cooking;
            format!(" {}%", done * 100 / self.pins.len().max(1))
        } else {
            String::new()
        };
        Some(if extra > 0 {
            format!("@{}+{extra}{dirty}{meter}", first.label)
        } else {
            format!("@{}{dirty}{meter}", first.label)
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
                    if p.cooking {
                        "digesting \u{2026}"
                    } else {
                        tier.as_str()
                    },
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

/// The card, without a model: the document's own title, then the lines
/// that look most like invocations. Crude next to a written card, and
/// it has the two properties that matter — instant, and never absent.
fn deterministic_card(text: &str, budget: usize) -> String {
    let mut out = String::new();
    let push = |line: &str, out: &mut String| -> bool {
        let line = line.trim();
        if line.is_empty() || out.chars().count() + line.chars().count() + 1 > budget {
            return false;
        }
        out.push_str(line);
        out.push('\n');
        true
    };
    // The title, if the document has one.
    if let Some(h) = text.lines().find(|l| l.trim_start().starts_with('#')) {
        push(h, &mut out);
    }
    // Then whatever most resembles a command: fenced lines, `backticks`,
    // and flag-bearing lines, in document order.
    let mut in_fence = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        let command_ish = in_fence || t.starts_with('$') || t.contains('`') || t.contains(" --");
        if command_ish && !push(line, &mut out) {
            break;
        }
    }
    out
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

    /// The whole point of the tier order: a digest that fits wins, but
    /// only ever *replaces* an outline that was already serving.
    #[test]
    fn a_digest_replaces_the_outline_once_it_fits() {
        let d = tmpdir("digest");
        let f = d.join("guide.md");
        std::fs::write(&f, "# Guide\n".to_string() + &"prose. ".repeat(500)).unwrap();
        let mut wc = WorkContext::new(400);
        wc.pin(&f).unwrap();
        assert_eq!(wc.pins[0].emit(wc.share()).0, Tier::Outline);

        let want = wc.digest_wanted();
        assert_eq!(want.len(), 1, "an over-budget pin asks to be digested");
        assert!(wc.pins[0].cooking, "and says so while it waits");
        assert!(
            wc.digest_wanted().is_empty(),
            "polling twice must not queue it twice"
        );

        wc.set_digest(want[0].0, Some("widgetctl sync --all".into()));
        assert!(!wc.pins[0].cooking);
        let (tier, body) = wc.pins[0].emit(wc.share());
        assert_eq!(tier, Tier::Digest);
        assert_eq!(body, "widgetctl sync --all");
        assert!(wc.context_block().contains("digest"));
    }

    #[test]
    fn a_failed_digest_leaves_the_outline_serving() {
        let d = tmpdir("digestfail");
        let f = d.join("guide.md");
        std::fs::write(&f, "# Guide\n".to_string() + &"prose. ".repeat(500)).unwrap();
        let mut wc = WorkContext::new(400);
        wc.pin(&f).unwrap();
        let want = wc.digest_wanted();
        wc.set_digest(want[0].0, None);
        assert!(!wc.pins[0].cooking);
        assert_eq!(wc.pins[0].emit(wc.share()).0, Tier::Outline);
        assert!(wc.context_block().contains("# Guide"), "still useful");
    }

    /// A model that ignores the target must not be asked forever.
    #[test]
    fn an_oversized_digest_is_retried_then_given_up_on() {
        let d = tmpdir("digestloop");
        let f = d.join("guide.md");
        std::fs::write(&f, "# Guide\n".to_string() + &"prose. ".repeat(500)).unwrap();
        let mut wc = WorkContext::new(400);
        wc.pin(&f).unwrap();
        for _ in 0..2 {
            let want = wc.digest_wanted();
            assert_eq!(want.len(), 1);
            wc.set_digest(want[0].0, Some("x".repeat(5000)));
        }
        assert!(
            wc.digest_wanted().is_empty(),
            "attempts are bounded; the outline is a fine place to stop"
        );
        assert_eq!(wc.pins[0].emit(wc.share()).0, Tier::Outline);
    }

    #[test]
    fn the_digest_source_is_bounded_not_the_raw_file() {
        let d = tmpdir("digestsrc");
        let f = d.join("huge.md");
        std::fs::write(&f, "# H\n".to_string() + &"line of prose here\n".repeat(20_000)).unwrap();
        let mut wc = WorkContext::new(600);
        wc.pin(&f).unwrap();
        let want = wc.digest_wanted();
        let source_len = want[0].2.chars().count();
        assert!(
            source_len <= DIGEST_SOURCE_CAP,
            "no local context window would take {source_len} chars"
        );
    }

    #[test]
    fn cancelling_stops_the_cook_without_losing_what_was_there() {
        let d = tmpdir("digestcancel");
        let f = d.join("guide.md");
        std::fs::write(&f, "# Guide\n".to_string() + &"prose. ".repeat(500)).unwrap();
        let mut wc = WorkContext::new(400);
        wc.pin(&f).unwrap();
        wc.digest_wanted();
        assert_eq!(wc.cooking_count(), 1);
        assert_eq!(wc.cancel_cooking(), 1);
        assert_eq!(wc.cooking_count(), 0);
        assert!(wc.digest_wanted().is_empty(), "a cancel is a decision");
        assert_eq!(wc.pins[0].emit(wc.share()).0, Tier::Outline);
    }

    #[test]
    fn the_chrome_meters_a_cook_and_puts_it_away_after() {
        let d = tmpdir("digestmeter");
        let a = d.join("a.md");
        let b = d.join("b.md");
        let big = "# H\n".to_string() + &"prose. ".repeat(500);
        std::fs::write(&a, &big).unwrap();
        std::fs::write(&b, &big).unwrap();
        let mut wc = WorkContext::new(400);
        wc.pin(&a).unwrap();
        wc.pin(&b).unwrap();
        let want = wc.digest_wanted();
        assert_eq!(want.len(), 2);
        assert!(wc.chrome_tag().unwrap().contains("0%"));
        wc.set_digest(want[0].0, Some("short".into()));
        assert!(wc.chrome_tag().unwrap().contains("50%"));
        wc.set_digest(want[1].0, Some("short".into()));
        let tag = wc.chrome_tag().unwrap();
        assert!(!tag.contains('%'), "meter collapses when done: {tag}");
    }

    /// A card exists from the instant a pin does — no engine required.
    /// That is the property the whole two-position scheme rests on.
    #[test]
    fn a_card_exists_before_any_model_answers() {
        let d = tmpdir("card");
        let f = d.join("commandRef.md");
        std::fs::write(
            &f,
            "# widgetctl\n\nSome explanation nobody needs.\n\n\
             Run `widgetctl sync --all` to sync.\n\
             Use `widgetctl purge --force` carefully.\n",
        )
        .unwrap();
        let mut wc = WorkContext::new(4000);
        wc.pin(&f).unwrap();
        let block = wc.cards_block();
        assert!(block.contains("# widgetctl"), "title kept: {block}");
        assert!(block.contains("widgetctl sync --all"), "{block}");
        assert!(!block.contains("nobody needs"), "prose dropped: {block}");
        assert!(block.contains("@commandRef.md"), "labelled: {block}");
    }

    #[test]
    fn a_written_card_replaces_the_deterministic_one() {
        let d = tmpdir("card2");
        let f = d.join("ref.md");
        std::fs::write(&f, "# tool\n\n`tool run`\n").unwrap();
        let mut wc = WorkContext::new(4000);
        wc.pin(&f).unwrap();
        let want = wc.card_wanted();
        assert_eq!(want.len(), 1, "every pin wants a card, not just big ones");
        assert!(wc.card_wanted().is_empty(), "asked once, not twice");
        wc.set_card(want[0].0, Some("tool: `tool run --now`".into()));
        assert!(wc.cards_block().contains("--now"));
        // Failure keeps the floor.
        let mut wc2 = WorkContext::new(4000);
        wc2.pin(&f).unwrap();
        let w2 = wc2.card_wanted();
        wc2.set_card(w2[0].0, None);
        assert!(wc2.cards_block().contains("tool run"), "floor survives");
    }

    /// The card's whole justification is that it is cheap enough to sit
    /// in the re-prefilled suffix. Several pins must not undo that.
    #[test]
    fn cards_share_one_small_budget_newest_first() {
        let d = tmpdir("card3");
        let mut wc = WorkContext::new(40_000);
        for i in 0..6 {
            let f = d.join(format!("ref{i}.md"));
            std::fs::write(&f, format!("# tool{i}\n\n`tool{i} run --flag-{i}`\n")).unwrap();
            wc.pin(&f).unwrap();
        }
        let block = wc.cards_block();
        assert!(
            block.chars().count() <= CARD_BUDGET + 120,
            "cards blew the budget: {} chars",
            block.chars().count()
        );
        // Newest first: the pin the user just made is the one they are
        // working on.
        assert!(block.contains("tool5"), "newest pin carded: {block}");
        let p5 = block.find("tool5").unwrap();
        let p4 = block.find("tool4").unwrap_or(usize::MAX);
        assert!(p5 < p4, "newest should come first: {block}");
    }

    #[test]
    fn no_pins_means_no_card_block_at_all() {
        let wc = WorkContext::new(4000);
        assert!(wc.cards_block().is_empty());
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
