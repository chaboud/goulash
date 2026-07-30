#!/usr/bin/env python3
"""Count platform-syntax errors honestly.

The naive version — regex the command for GNU-only forms — reported 6.9%.
That number was wrong: 68% of its hits came from questions whose SUBJECT
was the flag. Asked "what does the -P flag do in grep", a model replying
`grep -P "<regex>" file` is answering, not erring. Excluding those, the
real rate is 2.2%.

Two rules keep it honest:
  1. skip explain-type questions entirely
  2. skip a hit when the flag appears in the QUESTION text
"""
import json, re, sys
from collections import Counter

GNU = [
    ("du --max-depth",  r'--max-depth'),
    ("grep -P",         r'\bg?grep\b[^|;]*\s-\w*P'),
    ("stat -c",         r'\bstat\b[^|;]*\s-c\b'),
    ("ls --time-style", r'--time-style'),
    ("find -printf",    r'-printf'),
    ("date -d",         r'\bdate\b[^|;]*\s-d\b'),
    ("xargs -r",        r'\bxargs\b[^|;]*\s-r\b'),
    ("readlink -f",     r'\breadlink\b[^|;]*\s-f\b'),
]
# The flag itself, for rule 2.
SUBJECT = {"du --max-depth": "--max-depth", "grep -P": "-P", "stat -c": "-c",
           "ls --time-style": "--time-style", "find -printf": "-printf",
           "date -d": "-d", "xargs -r": "-r", "readlink -f": "-f"}
EXPLAIN = {"explain", "no-command-needed", "explain-flag", "data-explain-jq",
           "text-explain-pipefail", "git-explain-detached", "sys-explain-loadavg",
           "bulk-pkg-info"}

def violation(row):
    """The GNU-only form this command wrongly uses, or None."""
    cmd = row.get("command") or ""
    if not cmd or row.get("error"):
        return None
    if row.get("step") in EXPLAIN:
        return None                      # rule 1
    q = (row.get("question") or "").lower()
    for name, pat in GNU:
        if re.search(pat, cmd):
            if SUBJECT[name].lower() in q:
                continue                 # rule 2: the flag is what was asked about
            return name
    return None

if __name__ == "__main__":
    d = sys.argv[1] if len(sys.argv) > 1 else "bench/results/2026-07-28"
    rows = [json.loads(l) for l in open(f"{d}/journal.jsonl")]
    cm = [r for r in rows if r.get("command") and not r.get("error")]
    bad = [r for r in cm if violation(r)]
    print(f"{len(cm)} commands, {len(bad)} platform errors = {100*len(bad)/len(cm):.1f}%\n")
    print("by kind:")
    for k, v in Counter(violation(r) for r in bad).most_common():
        print(f"  {k:<18} {v}")
    print("\nby model:")
    for k, v in Counter(r["model"] for r in bad).most_common(8):
        t = sum(1 for r in cm if r["model"] == k)
        print(f"  {k:<26} {v:>3}/{t:<5} = {100*v/t:>5.1f}%")
