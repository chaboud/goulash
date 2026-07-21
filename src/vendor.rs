use std::collections::HashSet;
use std::path::Path;

/// A finished command block: what vendors reason over.
/// (wiki: architecture/suggestion-vendors.md)
pub struct CmdBlock {
    pub cmd: String,
    pub exit_code: i32,
    pub cwd: String,
    /// Last few KB of the command's (clean) output, lossy UTF-8.
    pub output_tail: String,
}

/// A vended suggestion, before the session assigns it an ID.
pub struct Vended {
    pub command: String,
    pub why: String,
    pub vendor: &'static str,
}

/// Deterministic thefuck-style rules: instant, offline, always on.
/// Fires only on failed commands and only on crisp matches.
pub struct RulesVendor {
    path_cache: Option<HashSet<String>>,
}

impl RulesVendor {
    pub fn new() -> Self {
        Self { path_cache: None }
    }

    #[cfg(test)]
    fn with_path(names: &[&str]) -> Self {
        Self {
            path_cache: Some(names.iter().map(|s| s.to_string()).collect()),
        }
    }

    pub fn suggest(&mut self, b: &CmdBlock) -> Vec<Vended> {
        if b.exit_code == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(v) = rule_git_set_upstream(b) {
            out.push(v);
        }
        if let Some(v) = rule_git_similar(b) {
            out.push(v);
        }
        if let Some(v) = self.rule_command_not_found(b) {
            out.push(v);
        }
        if let Some(v) = rule_permission_denied(b) {
            out.push(v);
        }
        if let Some(v) = rule_cd_typo(b) {
            out.push(v);
        }
        out.truncate(3);
        out
    }

    fn path_executables(&mut self) -> &HashSet<String> {
        self.path_cache.get_or_insert_with(|| {
            let mut set = HashSet::new();
            if let Some(path) = std::env::var_os("PATH") {
                for dir in std::env::split_paths(&path) {
                    if let Ok(entries) = std::fs::read_dir(&dir) {
                        for e in entries.flatten() {
                            if let Some(name) = e.file_name().to_str() {
                                set.insert(name.to_string());
                            }
                        }
                    }
                }
            }
            set
        })
    }

    /// `lls: command not found` → closest PATH executable.
    fn rule_command_not_found(&mut self, b: &CmdBlock) -> Option<Vended> {
        let missing = b.output_tail.lines().find_map(|l| {
            // bash/sh: "bash: lls: command not found"
            if let Some(pre) = l.strip_suffix(": command not found") {
                return Some(pre.rsplit(':').next().unwrap_or(pre).trim().to_string());
            }
            // zsh: "zsh: command not found: lls"
            l.split("command not found:")
                .nth(1)
                .map(|s| s.trim().to_string())
        })?;
        let first = b.cmd.split_whitespace().next()?;
        if first != missing {
            return None;
        }
        let best = closest(&missing, self.path_executables().iter(), 2)?;
        let fixed = b.cmd.replacen(first, &best, 1);
        Some(Vended {
            why: format!("`{missing}` isn't installed or is a typo; `{best}` is in PATH"),
            command: fixed,
            vendor: "rules",
        })
    }
}

/// git tells you the exact fix; lift it verbatim.
fn rule_git_set_upstream(b: &CmdBlock) -> Option<Vended> {
    if !b.cmd.trim_start().starts_with("git push") {
        return None;
    }
    let line = b
        .output_tail
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("git push --set-upstream"))?;
    Some(Vended {
        command: line.to_string(),
        why: "the branch has no upstream; this sets it and pushes".to_string(),
        vendor: "rules",
    })
}

/// "git: 'psuh' is not a git command ... The most similar command is push"
fn rule_git_similar(b: &CmdBlock) -> Option<Vended> {
    let tail = &b.output_tail;
    if !tail.contains("is not a git command") {
        return None;
    }
    let bad = tail.split('\'').nth(1)?.to_string();
    let good = tail
        .split("most similar command")
        .nth(1)?
        .split_whitespace()
        .find(|w| *w != "is" && *w != "are")?
        .to_string();
    if !b.cmd.contains(&bad) {
        return None;
    }
    Some(Vended {
        command: b.cmd.replacen(&bad, &good, 1),
        why: format!("`{bad}` isn't a git command; git suggests `{good}`"),
        vendor: "rules",
    })
}

/// exit != 0 with a permission complaint → sudo it (suggestion only).
fn rule_permission_denied(b: &CmdBlock) -> Option<Vended> {
    let t = b.output_tail.to_lowercase();
    if !t.contains("permission denied") || b.cmd.trim_start().starts_with("sudo ") {
        return None;
    }
    Some(Vended {
        command: format!("sudo {}", b.cmd),
        why: "permission denied; retry with sudo".to_string(),
        vendor: "rules",
    })
}

/// `cd <typo>` → closest subdirectory of the cwd.
fn rule_cd_typo(b: &CmdBlock) -> Option<Vended> {
    let mut parts = b.cmd.split_whitespace();
    if parts.next()? != "cd" || !b.output_tail.contains("o such file or directory") {
        return None;
    }
    let target = parts.next()?;
    let dirs: Vec<String> = std::fs::read_dir(Path::new(&b.cwd))
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    let best = closest(target, dirs.iter(), 2)?;
    Some(Vended {
        why: format!("`{target}` doesn't exist here; `{best}` does"),
        command: format!("cd {best}"),
        vendor: "rules",
    })
}

/// Best candidate within `max_dist` edit distance (and not equal).
/// Ties break toward subsequence matches (pure insert/delete typos like
/// `lls`→`ls` beat substitutions like `lls`→`llc`), then shorter names,
/// then lexicographic — fully deterministic.
fn closest<'a>(
    target: &str,
    candidates: impl Iterator<Item = &'a String>,
    max_dist: usize,
) -> Option<String> {
    type Key<'k> = (usize, bool, usize, &'k String);
    let mut best: Option<(Key<'_>, &String)> = None;
    for c in candidates {
        if c == target || c.len().abs_diff(target.len()) > max_dist {
            continue;
        }
        let d = levenshtein(target, c);
        if d > max_dist {
            continue;
        }
        let not_subseq = !is_subsequence(c, target) && !is_subsequence(target, c);
        let key = (d, not_subseq, c.len(), c);
        if best.as_ref().map(|(bk, _)| key < *bk).unwrap_or(true) {
            best = Some((key, c));
        }
    }
    best.map(|(_, c)| c.clone())
}

/// Is `needle` a subsequence of `haystack` (chars in order, gaps allowed)?
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut it = haystack.chars();
    needle.chars().all(|c| it.by_ref().any(|h| h == c))
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(cmd: &str, code: i32, tail: &str) -> CmdBlock {
        CmdBlock {
            cmd: cmd.into(),
            exit_code: code,
            cwd: "/tmp".into(),
            output_tail: tail.into(),
        }
    }

    #[test]
    fn no_suggestions_on_success() {
        let mut v = RulesVendor::new();
        assert!(v.suggest(&block("ls", 0, "")).is_empty());
    }

    #[test]
    fn git_set_upstream() {
        let mut v = RulesVendor::new();
        let tail = "fatal: The current branch feat has no upstream branch.\n\
                    To push the current branch and set the remote as upstream, use\n\n\
                    \tgit push --set-upstream origin feat\n";
        let s = v.suggest(&block("git push", 128, tail));
        assert_eq!(s[0].command, "git push --set-upstream origin feat");
    }

    #[test]
    fn git_similar() {
        let mut v = RulesVendor::new();
        let tail = "git: 'psuh' is not a git command. See 'git --help'.\n\n\
                    The most similar command is\n\tpush\n";
        let s = v.suggest(&block("git psuh origin main", 1, tail));
        assert_eq!(s[0].command, "git push origin main");
    }

    #[test]
    fn command_not_found_bash_and_zsh() {
        let mut v = RulesVendor::with_path(&["ls", "llc", "lld", "git", "less"]);
        for tail in [
            "bash: lls: command not found\n",
            "zsh: command not found: lls\n",
        ] {
            let s = v.suggest(&block("lls -la", 127, tail));
            assert!(!s.is_empty(), "no suggestion for {tail:?}");
            assert_eq!(s[0].command, "ls -la");
        }
    }

    #[test]
    fn permission_denied_adds_sudo() {
        let mut v = RulesVendor::new();
        let s = v.suggest(&block("apt install jq", 100, "E: Permission denied\n"));
        assert_eq!(s[0].command, "sudo apt install jq");
        // and never doubles up
        let s2 = v.suggest(&block("sudo apt install jq", 100, "Permission denied\n"));
        assert!(s2.is_empty());
    }

    #[test]
    fn levenshtein_sane() {
        assert_eq!(levenshtein("gti", "git"), 2);
        assert_eq!(levenshtein("lls", "ls"), 1);
        assert_eq!(levenshtein("same", "same"), 0);
    }
}
