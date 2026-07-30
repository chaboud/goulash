#!/usr/bin/env python3
"""A 'bulk' session: real commands that produce genuinely large output.

The existing corpus totals 3471 chars of captured output, so prompts peak
at 2929 tokens — 36% of an 8192 window. Nothing there tests what happens
when a session log gets big, which is the normal state of a terminal left
open for a day.

These are all read-only commands against this repo and the local system,
chosen because their real output runs to thousands of lines.
"""
import json, os, subprocess

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(REPO, "bench", "scenarios.toml")
esc = json.dumps

def run(cmd, cwd=REPO):
    p = subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True,
                       text=True, timeout=180)
    return p.returncode, ((p.stdout or "") + (p.stderr or "")).strip()

steps = []
def block(cmd, hms, cap=12000):
    code, out = run(cmd)
    out = out[:cap]
    steps.append(("block", dict(session="bulk", cmd=cmd, exit=code, hms=hms, tail=out)))
    print(f"  [{code}] {cmd[:52]:<52} {len(out):>6} chars")
def ask(sid, text, hms):
    steps.append(("ask", dict(session="bulk", id=f"bulk-{sid}", hms=hms, ask=text)))
def cwd(p, hms):
    steps.append(("cwd", dict(session="bulk", cwd=p, hms=hms)))

print("harvesting bulk outputs (read-only)...")
cwd(REPO, "14:00:00")
block("ls -laR src bench tests wiki | head -400", "14:00:10")
ask("what-here", "what's the overall shape of this tree", "14:01:00")
block("git log --stat -20", "14:01:30")
ask("recent-work", "summarise what's changed recently", "14:02:00")
block("cargo tree --depth 2 2>&1 | head -200", "14:02:30")
ask("deps", "which dependency pulls in the most transitively", "14:03:00")
block("wc -l src/*.rs src/engine/*.rs bench/src/*.rs", "14:03:30")
ask("biggest-file", "which source file is the biggest and should it be split", "14:04:00")
block("git log --oneline -60", "14:04:30")
ask("commit-pattern", "show me commits that touched the engine", "14:05:00")
block("grep -rn 'TODO\\|FIXME\\|XXX' src bench tests 2>/dev/null | head -80", "14:05:30")
ask("todos", "how many open TODOs are there and where", "14:06:00")
block("cargo metadata --no-deps --format-version 1 2>/dev/null | head -c 6000", "14:06:30")
ask("pkg-info", "what version is this crate and what does it depend on", "14:07:00")
block("env | sort", "14:07:30")
ask("env-check", "is there anything unusual in my environment", "14:08:00")
block("ps aux | head -60", "14:08:30")
ask("proc-heavy", "which process is using the most memory right now", "14:09:00")
ask("final-recall", "back at the start of this session, what did the tree look like", "14:09:30")

with open(OUT, "a") as f:
    f.write("\n# ---- appended by harvest_bulk.py: large-output session ----\n\n")
    for kind, d in steps:
        f.write("[[step]]\n")
        f.write(f"kind = {esc(kind)}\n")
        for k in ("session", "id", "hms", "cmd", "cwd", "ask"):
            if k in d: f.write(f"{k} = {esc(d[k])}\n")
        if "exit" in d: f.write(f"exit = {d['exit']}\n")
        if "tail" in d: f.write(f"tail = {esc(d['tail'])}\n")
        f.write("\n")

tot = sum(len(d.get("tail", "")) for _, d in steps)
asks = sum(1 for k, _ in steps if k == "ask")
print(f"\nbulk session: {tot} chars of real output, {asks} asks")
print(f"  at TAIL_CHARS=12000 the log reaches ~{tot} chars ~= {tot//3} tokens")
