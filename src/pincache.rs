//! Ingest results, kept across sessions.
//!
//! Pinning a document costs a model call — often two, since a pin over
//! its share wants a digest as well as a card — and nothing persisted,
//! so every session re-cooked every pin from scratch. Field-observed on
//! a 4 KB reference: the digest landed twenty-five seconds after the
//! pin, and its notice arrived on top of the next pin's.
//!
//! The cache is keyed on **exactly what the ingest consumed**, which
//! makes a hit provably identical to a fresh cook rather than merely
//! likely to be:
//!
//! - `raw`, the assembled ingest input. Not the file's bytes: a pin can
//!   be a path, and then the input is a rendered tree. Not the mtime
//!   either — a file can be touched without changing, or restored with
//!   an old one. Hashing what goes in covers files, trees, and the
//!   READ_CAP truncation in one stroke.
//! - the app version, so a release re-cooks.
//! - the ingest prompts themselves, so rewording a prompt re-cooks even
//!   within a version. That is the one that actually keeps the promise:
//!   a version bump catches releases, but the change that MATTERS is
//!   the prompt, and a dev build would otherwise serve cards cooked by
//!   wording that no longer exists.
//!
//! FNV-1a rather than `DefaultHasher`: the standard library explicitly
//! reserves the right to change SipHash's output between releases, and
//! a compiler upgrade silently invalidating every entry — with no way
//! to tell that is what happened — is exactly the kind of quiet
//! surprise this cache must not produce.

use std::path::{Path, PathBuf};

/// One cached ingest.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cached {
    /// What it was called when it was cooked. Never used for lookup —
    /// the key is the content — but a directory of hex filenames is
    /// unreadable without it.
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub card: Option<String>,
    #[serde(default)]
    pub at: String,
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// The cache key for an ingest input, as hex.
///
/// `rev` is whatever the caller considers the shape of the cook — the
/// engine passes its prompt templates. It is a parameter rather than a
/// call into `engine` so this module stays a leaf: the cache should not
/// need to know how an ingest is performed, only that its shape can
/// change.
pub fn key(raw: &str, rev: &str) -> String {
    let mut h = fnv1a(FNV_OFFSET, raw.as_bytes());
    // A separator between fields, so `raw` ending in the version cannot
    // collide with a shorter `raw` and a longer version.
    h = fnv1a(h, b"\x1f");
    h = fnv1a(h, env!("CARGO_PKG_VERSION").as_bytes());
    h = fnv1a(h, b"\x1f");
    h = fnv1a(h, rev.as_bytes());
    format!("{h:016x}")
}

/// Where the cache lives by default. Callers hold the directory rather
/// than reaching for this on every call: an ambient path resolved from
/// `$HOME` meant `cargo test` wrote into the developer's own
/// `~/.goulash`, which is the same defect as the suite that once
/// answered from the developer's real ollama.
pub fn default_dir() -> Option<PathBuf> {
    crate::config::Config::dir().map(|d| d.join("pins"))
}

/// What was cooked for this input last time, if anything.
pub fn load(dir: &Path, key: &str) -> Option<Cached> {
    let text = std::fs::read_to_string(dir.join(format!("{key}.toml"))).ok()?;
    toml::from_str(&text).ok()
}

/// Keep what was cooked. Best-effort in every direction: a cache that
/// cannot be written is a slower session, not a broken one, so nothing
/// here reports failure upward.
pub fn store(dir: &Path, key: &str, label: &str, digest: Option<&str>, card: Option<&str>) {
    let p = dir.join(format!("{key}.toml"));
    // Merge rather than replace: the digest and the card land in
    // separate events, and writing one must not erase the other.
    let mut c = load(dir, key).unwrap_or_default();
    c.label = label.to_string();
    if digest.is_some() {
        c.digest = digest.map(str::to_string);
    }
    if card.is_some() {
        c.card = card.map(str::to_string);
    }
    c.at = crate::engine::local_now();
    if c.digest.is_none() && c.card.is_none() {
        return;
    }
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(text) = toml::to_string(&c) {
        let _ = std::fs::write(&p, text);
    }
}

/// Hold the cache to `keep` entries, oldest-touched first.
///
/// Called after a write, not on a timer: the only moment the cache can
/// grow is the only moment worth checking it. Age is the file's own
/// mtime, which the OS updates on write — so an entry that keeps being
/// re-cooked keeps its place, and one pinned once a year ago goes.
pub fn evict(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort_by_key(|(t, _)| *t);
    for (_, p) in files.iter().take(files.len() - keep) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_moves_with_everything_that_changes_the_answer() {
        let a = key("hello", "rev1");
        assert_eq!(a, key("hello", "rev1"), "same input, same key");
        assert_ne!(a, key("hello ", "rev1"), "content");
        assert_ne!(a, key("hello", "rev2"), "the shape of the cook");
        // A pin can be a path, and then the input is a rendered tree —
        // which is why the key is over `raw` and not over file bytes.
        let tree = "a.txt\nb.txt\nsub/c.txt\n";
        assert_ne!(key(tree, "rev1"), a);
    }

    #[test]
    fn a_cook_survives_and_the_two_halves_do_not_erase_each_other() {
        let d = std::env::temp_dir().join(format!("goulash-pc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let k = key("some document", "rev");
        assert!(load(&d, &k).is_none(), "nothing cooked yet");
        // The digest and the card arrive as separate events.
        store(&d, &k, "doc.md", Some("compressed"), None);
        store(&d, &k, "doc.md", None, Some("crib"));
        let c = load(&d, &k).expect("cached");
        assert_eq!(
            c.digest.as_deref(),
            Some("compressed"),
            "kept by the card write"
        );
        assert_eq!(c.card.as_deref(), Some("crib"));
        // A different revision of the ingest is a different entry.
        assert!(load(&d, &key("some document", "rev2")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn field_separators_stop_a_shift_from_colliding() {
        // Without a separator, ("ab", "c") and ("a", "bc") hash the same
        // stream. A cache that served one document's digest for another
        // would be worse than no cache at all.
        assert_ne!(key("ab", "c"), key("a", "bc"));
    }
}
