#!/usr/bin/env python3
"""Stratified blind corpus.

The full sweep holds 2299 answers. Reading all of them with real care is
not feasible, and skimming them would produce grades not worth having.
This samples for the two decisions grading has to inform:

  1. Does command-first (S3) cost answer quality against S1?
     -> the SAME question and model under both shapes, side by side.
  2. Which models actually produce correct commands?
     -> a spread of every cell on a fixed question set.

Questions are chosen to have checkable answers — a jq filter is either
right or it isn't — plus explain-only controls where a command is wrong
by definition.
"""
import json, os, sys

D = sys.argv[1] if len(sys.argv) > 1 else "bench/results/2026-07-28"
QUESTIONS = [
    "jq-extract",        # .items[].name — exactly right or not
    "disk-size",         # du|sort idiom
    "tree-view",         # depth limit + prune
    "git-undo",          # soft vs hard reset: a correctness trap
    "data-csv-group",    # sum per region: awk/sort composition
    "data-log-status",   # count by status code
    "no-command-needed",  # control: prose only, no command
    "text-explain-pipefail",  # control: prose only
]

def blind_id(key):
    h = 0xcbf29ce484222325
    for b in key.encode():
        h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return f"{h & 0xffffff:06x}"

rows = [json.loads(l) for l in open(os.path.join(D, "journal.jsonl"))]
pick = [r for r in rows
        if r["pass"] == "pass-b" and r["step"] in QUESTIONS
        and r["shape"] in ("S1", "S3") and not r["error"]]

by_q = {}
for r in pick:
    by_q.setdefault(r["step"], []).append(r)

out = ["# Blind grading corpus (stratified)\n",
       "Model, provider and prompt shape are hidden. Grade on the answer",
       "text alone. One JSON object per line into `grades.jsonl`:\n",
       '```json\n{"id":"a1b2c3","correct":0-3,"idiom":0-3,"fit":0-3,"why":"..."}\n```\n',
       "- **correct** 0=wrong/harmful 1=wrong but close 2=works 3=works and handles the ask fully",
       "- **idiom** 0=bizarre 1=clumsy 2=fine 3=how a practitioner would write it",
       "- **fit** 0=unusable in a status bar 1=too long 2=ok 3=crisp one-liner",
       "\nFor the two `explain` controls a command is WRONG by definition: score",
       "`correct` on the prose, and mark any command down in `idiom`.\n"]

keymap = []
for q in QUESTIONS:
    rs = by_q.get(q, [])
    if not rs:
        continue
    out.append(f"\n## {q}\n\n> {rs[0]['question'][:160]}\n")
    for r in sorted(rs, key=lambda r: blind_id(r["key"])):
        i = blind_id(r["key"])
        prose = r["text"].strip() or "(no prose)"
        cmd = f"\n      `CMD: {r['command']}`" if r["command"] else "\n      (no command)"
        out.append(f"- `[{i}]` {prose}{cmd}")
        keymap.append({"id": i, "key": r["key"], "model": r["model"],
                       "provider": r["provider"], "shape": r["shape"], "step": r["step"]})

open(os.path.join(D, "blind_sample.md"), "w").write("\n".join(out) + "\n")
with open(os.path.join(D, "blind_sample_keys.jsonl"), "w") as f:
    for k in keymap:
        f.write(json.dumps(k) + "\n")
print(f"{len(keymap)} answers across {len(by_q)} questions -> {D}/blind_sample.md")
