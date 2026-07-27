//! Runaway detection, in the bottom bar.
//!
//! Every bug found in the 2026-07 review had the same shape — something
//! whose growth was unconditional while its clearing was not — and none
//! of them were visible until they were fatal. A counter overflowed
//! after a minute of idle; a research queue grew without limit behind a
//! non-default setting; a transcript has always grown forever. The cure
//! for that class is not more asserts, it is **being able to watch the
//! numbers move**.
//!
//! So this is diagnostic, not decorative. Everything here answers "is
//! something climbing that should not be?"

use std::path::Path;
use std::time::{Duration, Instant};

/// Sampling interval for the expensive fields (RSS, disk). The cheap
/// counters are live; these two cost a syscall or a directory walk, and
/// nobody diagnoses a leak at 250ms resolution.
const SAMPLE_EVERY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Stats {
    /// Engine invocations, split by lane, since start.
    pub asks: u64,
    pub research: u64,
    pub digests: u64,
    /// Depths that have run away before: the digest queue and the
    /// backfill of superseded research, reported by the worker.
    pub queued: usize,
    pub backfill: usize,
    /// Session-side collections with a growth story.
    pub slots: usize,
    pub held: usize,
    pub pins: usize,
    pub ctx_chars: usize,
    /// Sampled.
    rss_kb: u64,
    disk_kb: u64,
    last_sample: Option<Instant>,
}

impl Stats {
    pub fn new() -> Stats {
        Stats::default()
    }

    /// Refresh the expensive fields if the interval has elapsed. Cheap
    /// to call from the paint path; does nothing most of the time.
    pub fn sample(&mut self, home: &Path) {
        if self.last_sample.is_some_and(|t| t.elapsed() < SAMPLE_EVERY) {
            return;
        }
        self.last_sample = Some(Instant::now());
        self.rss_kb = rss_kb();
        self.disk_kb = dir_kb(home);
    }

    /// One bar-width line. Ordered by how often it is the answer:
    /// memory first, then the queues that have actually run away, then
    /// the stores that grow forever.
    pub fn line(&self) -> String {
        format!(
            "{} \u{b7} {}a/{}r/{}d \u{b7} q{}+{} \u{b7} {} slots \u{b7} {} ctx \u{b7} {}",
            human_kb(self.rss_kb),
            self.asks,
            self.research,
            self.digests,
            self.queued,
            self.backfill,
            self.slots,
            human_n(self.ctx_chars),
            human_kb(self.disk_kb),
        )
    }
}

fn human_kb(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.1}G", kb as f64 / (1024.0 * 1024.0))
    } else if kb >= 1024 {
        format!("{}M", kb / 1024)
    } else {
        format!("{kb}K")
    }
}

fn human_n(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        format!("{n}")
    }
}

/// Resident set size of this process.
///
/// Linux reads `/proc/self/statm`, which is free. Everywhere else — read
/// macOS — shells out to `ps`, which is a fork we would not otherwise
/// pay. That is why it is behind `SAMPLE_EVERY` and behind the setting
/// being on at all: at most one fork every five seconds, and only for
/// someone who asked to watch the numbers.
///
/// `proc_pidinfo` would avoid the fork, but it cannot be compiled or
/// tested from here, and an untestable platform branch in the paint path
/// is a worse trade than a fork nobody takes by default.
#[cfg(target_os = "linux")]
fn rss_kb() -> u64 {
    let Ok(s) = std::fs::read_to_string("/proc/self/statm") else {
        return 0;
    };
    let pages: u64 = s
        .split_whitespace()
        .nth(1)
        .and_then(|f| f.parse().ok())
        .unwrap_or(0);
    // SAFETY: sysconf is a pure lookup.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page <= 0 {
        return 0;
    }
    pages.saturating_mul(page as u64) / 1024
}

#[cfg(not(target_os = "linux"))]
fn rss_kb() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p"])
        .arg(std::process::id().to_string())
        .output();
    out.ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Total bytes under a directory, in KiB. Bounded: a runaway transcript
/// is exactly what this is meant to catch, so the walk must not itself
/// become the runaway.
fn dir_kb(root: &Path) -> u64 {
    const MAX_ENTRIES: usize = 4096;
    let mut total: u64 = 0;
    let mut seen = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return total / 1024;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => {
                    if let Ok(md) = entry.metadata() {
                        total = total.saturating_add(md.len());
                    }
                }
                _ => {}
            }
        }
    }
    total / 1024
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_at_a_glance() {
        assert_eq!(human_kb(512), "512K");
        assert_eq!(human_kb(2048), "2M");
        assert_eq!(human_kb(3 * 1024 * 1024), "3.0G");
        assert_eq!(human_n(999), "999");
        assert_eq!(human_n(12_500), "12k");
        assert_eq!(human_n(2_500_000), "2.5M");
    }

    #[test]
    fn the_line_names_every_runaway_we_have_actually_had() {
        let mut s = Stats::new();
        s.asks = 3;
        s.backfill = 8;
        s.ctx_chars = 11_500;
        let l = s.line();
        // backfill grew without limit behind backfill_abandoned; ctx_log
        // and the transcript both grow forever by design.
        assert!(l.contains("+8"), "{l}");
        assert!(l.contains("11k ctx"), "{l}");
        assert!(l.contains("3a/"), "{l}");
    }

    #[test]
    fn sampling_is_rate_limited() {
        let mut s = Stats::new();
        let dir = std::env::temp_dir();
        s.sample(&dir);
        let first = s.last_sample;
        s.sample(&dir);
        assert_eq!(
            first, s.last_sample,
            "a paint-path call must be nearly free"
        );
    }

    #[test]
    fn a_missing_home_is_zero_not_a_panic() {
        assert_eq!(dir_kb(Path::new("/no/such/place/at/all")), 0);
    }
}
