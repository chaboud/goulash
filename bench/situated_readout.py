#!/usr/bin/env python3
"""Situated-context readout: does telling the model where it is help?

Scored with the honest detector (bench/gnucheck.py) — the naive regex
counted 'grep -P' as an error even when the QUESTION was "what does -P do
in grep", which inflated the rate from 2.0% to 6.9%.
"""
import json, statistics, sys
sys.path.insert(0, "bench")
from gnucheck import violation

D = sys.argv[1] if len(sys.argv) > 1 else "bench/results/2026-07-28"
MODELS = sys.argv[2].split(",") if len(sys.argv) > 2 else \
    ["qwen3.5:4b", "llama3.2:3b", "qwen3.5:9b"]
NAMES = {"S3": "baseline", "S4": "+platform", "S5": "+platform+tools",
         "S6": "+full PATH", "S7": "S5 via MEMORY"}

rows = [json.loads(l) for l in open(f"{D}/journal.jsonl")]
med = lambda v: statistics.median(v) if v else 0

print("PLATFORM ERRORS by arm (honest detector)\n")
print(f"  {'model':<14} {'arm':<16} {'cmds':>5} {'errors':>12} {'p50 tok':>8} {'p50 peval':>10}")
totals = {}
for m in MODELS:
    for sh in ("S3", "S4", "S5", "S6", "S7"):
        rs = [r for r in rows if r["model"] == m and r["shape"] == sh and not r["error"]]
        cm = [r for r in rs if r["command"]]
        if not cm:
            continue
        bad = [r for r in cm if violation(r)]
        totals.setdefault(sh, [0, 0])
        totals[sh][0] += len(bad); totals[sh][1] += len(cm)
        print(f"  {m:<14} {NAMES[sh]:<16} {len(cm):>5} "
              f"{f'{len(bad)} ({100*len(bad)/len(cm):.1f}%)':>12} "
              f"{med([r['prompt_tokens'] for r in rs if r['prompt_tokens']]):>8.0f} "
              f"{med([r['prompt_eval_ms'] for r in rs if r['prompt_eval_ms']]):>9.0f}ms")
    print()

print("POOLED across these models:")
print(f"  {'arm':<16} {'cmds':>6} {'errors':>14} {'vs baseline':>13}")
base = totals.get("S3", [0, 1])
brate = 100 * base[0] / max(base[1], 1)
for sh in ("S3", "S4", "S5", "S6", "S7"):
    if sh not in totals:
        continue
    b, n = totals[sh]
    rate = 100 * b / max(n, 1)
    delta = "" if sh == "S3" else f"{rate-brate:+.1f} pts"
    print(f"  {NAMES[sh]:<16} {n:>6} {f'{b} ({rate:.1f}%)':>14} {delta:>13}")

print("\nWHAT KIND of error remains, by arm:")
from collections import Counter
for sh in ("S3", "S4", "S5", "S6", "S7"):
    cm = [r for r in rows if r["model"] in MODELS and r["shape"] == sh
          and r.get("command") and not r["error"]]
    c = Counter(violation(r) for r in cm if violation(r))
    print(f"  {NAMES[sh]:<16} {dict(c) if c else '(clean)'}")
