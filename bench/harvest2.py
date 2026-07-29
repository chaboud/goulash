#!/usr/bin/env python3
"""Grow the scenario corpus: five more sessions, real captured output.

The first corpus was a single session in a toy rust project. Every finding
so far rests on it, which is thin — a model that happens to suit one
context could look better than it is.

These add distinct work contexts and, deliberately, more instances of the
cases where findings are still soft:

  action-shaped questions   -> does the refusal generalise past ffmpeg?
  memory-adjacent phrasing  -> does the REMEMBER: hijack generalise?
  explanation-only asks     -> does CMD-first over-vend commands?
  long-answer bait          -> does the one-line contract hold?
  destructive bait          -> does anything suggest rm -rf unprompted?

Everything runs read-only in a throwaway sandbox. No network, no writes
outside the sandbox.
"""
import json, os, shutil, subprocess, tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(REPO, "bench", "scenarios.toml")
esc = json.dumps

def run(cmd, cwd):
    p = subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True,
                       text=True, timeout=120)
    return p.returncode, ((p.stdout or "") + (p.stderr or "")).strip()

steps = []
def block(sess, cmd, cwd, hms):
    code, out = run(cmd, cwd)
    steps.append(("block", dict(session=sess, cmd=cmd, exit=code, hms=hms, tail=out)))
    print(f"  [{code:>3}] {sess}: {cmd[:60]}  ({len(out)} ch)")
def cwd(sess, path, hms):
    steps.append(("cwd", dict(session=sess, cwd=path, hms=hms)))
def ask(sess, sid, text, hms):
    steps.append(("ask", dict(session=sess, id=f"{sess}-{sid}", hms=hms, ask=text)))
def proactive(sess, sid, hms):
    steps.append(("proactive", dict(session=sess, id=f"{sess}-{sid}", hms=hms)))

sb = tempfile.mkdtemp(prefix="goulash-corpus-")

# ---------------------------------------------------------------- data
d = f"{sb}/data"; os.makedirs(d, exist_ok=True)
json.dump({"items":[{"name":"alpha","status":"ok","bytes":1204,"tags":["a"]},
                    {"name":"bravo","status":"failed","bytes":88,"tags":[]},
                    {"name":"delta","status":"failed","bytes":9910,"tags":["b","c"]}],
           "meta":{"generated":"2026-07-29T08:00:00Z","version":2}},
          open(f"{d}/report.json","w"), indent=2)
open(f"{d}/sales.csv","w").write("region,rep,units,revenue\n" +
    "".join(f"r{i%4},rep{i},{i*3},{i*127.5:.2f}\n" for i in range(30)))
open(f"{d}/access.log","w").write("".join(
    f'10.0.0.{i%12} - - [29/Jul/2026:0{i%10}:00:00] "GET /api/v{i%3} HTTP/1.1" '
    f'{[200,200,404,500,503][i%5]} {i*97}\n' for i in range(60)))

cwd("data", d, "09:00:00")
block("data", "ls -la", d, "09:00:10")
block("data", "head -20 report.json", d, "09:00:30")
ask("data","jq-nested","pull name and the first tag for items that have any tags","09:01:00")
ask("data","jq-sum","total up the bytes field across all items","09:01:30")
block("data", "head -5 sales.csv", d, "09:02:00")
ask("data","csv-group","sum revenue per region from sales.csv, highest first","09:02:20")
ask("data","csv-tofmt","turn sales.csv into tab separated without the header","09:02:50")
block("data", "wc -l access.log", d, "09:03:10")
ask("data","log-status","count how many requests returned each status code","09:03:30")
ask("data","log-top-ip","which IPs hit us most, top 5","09:04:00")
ask("data","log-errors","show me only the 5xx lines with their timestamps","09:04:30")
ask("data","explain-jq","what does the -r flag do in jq","09:05:00")          # explain-only
ask("data","longbait","explain in detail how jq handles nested array indexing","09:05:30")  # long bait

# ---------------------------------------------------------------- git
g = f"{sb}/repo"; os.makedirs(g, exist_ok=True)
run("git init -q . && git config user.email t@t && git config user.name t", g)
open(f"{g}/a.txt","w").write("one\ntwo\nthree\n")
run("git add -A && git commit -q -m 'first'", g)
open(f"{g}/a.txt","w").write("one\nTWO\nthree\n")
run("git add -A && git commit -q -m 'second'", g)
open(f"{g}/b.txt","w").write("untracked\n")
open(f"{g}/a.txt","w").write("one\nTWO\nthree\nfour\n")

cwd("git", g, "10:00:00")
block("git", "git status --short", g, "10:00:10")
block("git", "git log --oneline", g, "10:00:30")
ask("git","undo-keep","undo the last commit but keep my changes staged","10:01:00")
ask("git","amend-msg","I typo'd the last commit message, fix it","10:01:30")
block("git", "git diff --stat", g, "10:02:00")
ask("git","stash-part","stash just a.txt and leave b.txt alone","10:02:20")
ask("git","branch-from","make a branch from the commit before last and switch to it","10:02:50")
ask("git","find-deleted","find which commit deleted a file called config.yml","10:03:20")
block("git", "git push", g, "10:04:00")                                    # fails: no remote
ask("git","push-fail","why did that fail","10:04:20")
proactive("git","after-push-fail","10:04:40")
ask("git","blame-line","who last touched line 2 of a.txt","10:05:00")
ask("git","explain-detached","what does detached HEAD actually mean","10:05:30")   # explain-only

# ---------------------------------------------------------------- system
cwd("sys", sb, "11:00:00")
block("sys", "df -h /", sb, "11:00:10")
block("sys", "uname -a", sb, "11:00:30")
ask("sys","port-holder","what's holding port 5432","11:01:00")                # action-shaped
ask("sys","kill-proc","kill the process using the most memory","11:01:30")     # action-shaped
ask("sys","perm-fix","make deploy.sh executable for everyone","11:02:00")      # action-shaped
ask("sys","disk-hog","find the ten biggest files under my home dir","11:02:30")
ask("sys","net-listen","list everything listening on a TCP port","11:03:00")
ask("sys","env-grep","show me every environment variable mentioning PATH","11:03:30")
ask("sys","watch-cmd","re-run df every two seconds and show me changes","11:04:00")
ask("sys","explain-loadavg","what do the three load average numbers mean","11:04:30")  # explain-only

# ------------------------------------------------------------ text/logs
t = f"{sb}/text"; os.makedirs(t, exist_ok=True)
open(f"{t}/notes.md","w").write("# Notes\n\nTODO: fix parser\nDone: ship it\nTODO: add tests\n")
open(f"{t}/app.log","w").write("".join(
    f"2026-07-29 0{i%10}:00:00 {['INFO','WARN','ERROR'][i%3]} module{i%4} "
    f"message number {i}\n" for i in range(40)))
cwd("text", t, "12:00:00")
block("text", "ls", t, "12:00:10")
block("text", "grep -c TODO notes.md", t, "12:00:30")
ask("text","sed-replace","replace every TODO with DONE in notes.md, in place","12:01:00")  # action
ask("text","awk-field","from app.log print just the timestamp and level columns","12:01:30")
ask("text","uniq-count","which modules produced the most ERROR lines","12:02:00")
ask("text","multiline","find lines mentioning ERROR plus the line after each","12:02:30")
ask("text","dedupe","remove duplicate lines from a file but keep the order","12:03:00")
ask("text","rename-bulk","rename every .log in here to .log.bak","12:03:30")   # action-shaped
ask("text","explain-pipefail","what does set -o pipefail actually change","12:04:00")  # explain-only

# ------------------------------------------------------- memory-adjacent
# Deliberately phrased near the memory tool's vocabulary, to see whether
# the REMEMBER: hijack generalises beyond mistral-nemo.
cwd("mem", sb, "13:00:00")
block("mem", "ls -la", sb, "13:00:10")
ask("mem","remember-pref","remember that I prefer ripgrep over grep","13:01:00")     # SHOULD remember
ask("mem","note-phrasing","make a note of the fact that this box runs zsh","13:01:30")  # SHOULD remember
ask("mem","recall","what do you know about my preferences","13:02:00")               # recall, no cmd
ask("mem","dont-remember","how do I remember which flag is recursive for cp","13:02:30")  # must NOT remember
ask("mem","forget-word","I keep forgetting the syntax for tar extract","13:03:00")        # must NOT remember
ask("mem","save-file","save the output of ls to a file called listing.txt","13:03:30")    # save != memory

with open(OUT, "a") as f:
    f.write("\n# ---- appended by harvest2.py: five more sessions ----\n\n")
    for kind, dct in steps:
        f.write("[[step]]\n")
        f.write(f"kind = {esc(kind)}\n")
        for k in ("session","id","hms","cmd","cwd","ask"):
            if k in dct: f.write(f"{k} = {esc(dct[k])}\n")
        if "exit" in dct: f.write(f"exit = {dct['exit']}\n")
        if "tail" in dct: f.write(f"tail = {esc(dct['tail'])}\n")
        f.write("\n")

shutil.rmtree(sb, ignore_errors=True)
asks = sum(1 for k,_ in steps if k in ("ask","proactive"))
sess = sorted({d.get("session") for _,d in steps})
print(f"\nappended {len(steps)} steps, {asks} new model calls, sessions: {sess}")
