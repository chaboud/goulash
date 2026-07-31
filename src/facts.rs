//! Facts about this machine, derived fresh each run.
//!
//! **Derived, never stored.** A stored fact cannot notice it has gone
//! stale — `brew install fd` and a pinned note keeps saying `fd` is
//! absent, with all the authority of something the user wrote. Deriving
//! costs ~4ms (0.8ms of `read_dir` over ~1700 PATH entries, 2.7ms of
//! `uname` fork+exec) and is deterministic, so the string is byte-stable
//! run to run and the provider's prefix cache is unaffected — a change
//! costs exactly one cache miss, at exactly the moment the truth changed.
//!
//! Nothing here executes a third-party binary. Probing a tool with
//! `--help` would break the no-run invariant and can trip installers on
//! shimmed commands; `read_dir` only asks the filesystem what exists.

use crate::config::DivulgeConfig;

/// Tools a shell assistant actually reaches for. The full PATH set is
/// ~1700 names and mostly noise (`kextload`, `segedit`, `prl_convert`).
const CURATED: &[&str] = &[
    "jq", "yq", "rg", "fd", "ag", "tree", "bat", "delta", "fzf", "gh", "git", "docker",
    "kubectl", "tmux", "curl", "wget", "ffmpeg", "zstd", "pigz", "pv", "rsync", "gsed",
    "gawk", "ggrep", "gdate", "gstat", "gfind", "gtar", "python3", "node", "cargo", "go",
    "make", "cmake", "tar", "unzip",
];

fn uname() -> String {
    std::process::Command::new("uname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// The shell the model is actually talking to.
///
/// `$SHELL` is the *login* shell and answers a different question. Run
/// `goulash bash` from a zsh login and it still says zsh — so the one
/// line that exists to stop wrong-platform suggestions was naming the
/// wrong platform, and confidently. Observed: bash under goulash, the
/// model replying "…largest first on macOS zsh".
///
/// The caller passes the shell it launched. `$SHELL` remains the
/// fallback for a caller that has not launched one (the bench).
fn shell(actual: &str) -> String {
    if !actual.is_empty() {
        return actual.rsplit('/').next().unwrap_or(actual).to_string();
    }
    std::env::var("SHELL")
        .ok()
        .and_then(|s| s.rsplit('/').next().map(String::from))
        .unwrap_or_else(|| "sh".into())
}

/// OS, userland flavour and shell, plus the BSD/GNU differences that
/// account for essentially all observed platform errors.
///
/// Measured over 4355 vended commands (bench/gnucheck.py): 91 used
/// GNU-only syntax on a Darwin box, and **70 of those 91 were
/// `du --max-depth`**. It is not a spread of Linux assumptions, it is
/// one flag, over and over, with the failure sitting in the session log
/// the whole time. Naming the userland is the cheapest thing that has
/// ever worked on it.
pub fn platform_line(actual_shell: &str) -> String {
    let (os, sh) = (uname(), shell(actual_shell));
    if os == "Darwin" {
        format!(
            "Environment: macOS ({os}), BSD userland, {sh} shell. BSD differs from GNU: \
             'du -d N' not '--max-depth', 'sed -i \"\"' not 'sed -i', 'date -v' not \
             'date -d', 'stat -f' not 'stat -c'. BSD grep has NO -P.\n\n"
        )
    } else {
        format!("Environment: {os}, GNU userland, {sh} shell.\n\n")
    }
}

pub fn tools_line() -> String {
    let have = crate::vendor::path_executable_set();
    let present: Vec<&str> = CURATED.iter().copied().filter(|t| have.contains(*t)).collect();
    let absent: Vec<&str> = CURATED.iter().copied().filter(|t| !have.contains(*t)).collect();
    format!(
        "Installed: {}. NOT installed, never suggest: {}.\n\n",
        present.join(" "),
        absent.join(" ")
    )
}

/// Every executable on PATH. Debug only — ~3900 tokens, and it showed no
/// benefit over the curated list at 7x the context.
pub fn full_path_line() -> String {
    let mut have: Vec<String> = crate::vendor::path_executable_set().into_iter().collect();
    have.sort();
    format!(
        "Every executable on PATH ({} total): {}.\n\n",
        have.len(),
        have.join(" ")
    )
}

/// The block prepended to the prompt's cached prefix. Empty when nothing
/// is enabled, so the prompt is byte-identical to having no feature.
pub fn block(cfg: &DivulgeConfig, actual_shell: &str) -> String {
    let mut s = String::new();
    if cfg.platform {
        s.push_str(&platform_line(actual_shell));
    }
    // full_path replaces tools rather than stacking with it.
    if cfg.full_path {
        s.push_str(&full_path_line());
    } else if cfg.tools {
        s.push_str(&tools_line());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DivulgeConfig;

    #[test]
    fn nothing_enabled_is_byte_identical_to_no_feature() {
        let off = DivulgeConfig {
            platform: false,
            tools: false,
            full_path: false,
        };
        assert_eq!(block(&off, "zsh"), "");
    }

    /// The cache argument depends on this: same machine, same bytes, so
    /// regenerating every run costs nothing until something actually
    /// changes.
    #[test]
    fn derivation_is_deterministic() {
        let cfg = DivulgeConfig {
            platform: true,
            tools: true,
            full_path: false,
        };
        assert_eq!(block(&cfg, "zsh"), block(&cfg, "zsh"));
        assert_eq!(platform_line("zsh"), platform_line("zsh"));
    }

    #[test]
    fn full_path_replaces_tools_rather_than_stacking() {
        let both = DivulgeConfig {
            platform: false,
            tools: true,
            full_path: true,
        };
        let b = block(&both, "zsh");
        assert!(b.starts_with("Every executable on PATH"));
        assert!(!b.contains("NOT installed, never suggest"));
    }
}

