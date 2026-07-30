#!/usr/bin/env python3
"""Step 0d (tightened): does the warm prefix-cache floor scale with prompt size?

Fixes over the first attempt: 3 repeats + median, and a genuinely cold
baseline (a DISTINCT prefix, not a tail perturbation of the same one).
Writes results next to itself so an interrupted run leaves evidence.
"""
import json, statistics, sys, urllib.request

HOST, MODEL = "http://127.0.0.1:11434", "qwen3.5:0.8b"
REPS = 3

def call(prompt):
    body = json.dumps({"model": MODEL, "prompt": prompt, "stream": False,
        "think": False, "options": {"temperature": 0.2, "num_predict": 4,
        "num_ctx": 16384}, "keep_alive": "10m"}).encode()
    req = urllib.request.Request(f"{HOST}/api/generate", data=body,
                                 headers={"Content-Type": "application/json"})
    v = json.loads(urllib.request.urlopen(req, timeout=300).read())
    return v.get("prompt_eval_duration", 0)/1e6, (v.get("prompt_eval_count") or 0)

def build(blocks, seed):
    s = f"You are goulash. Session {seed}.\n\nSession log (oldest first):\n"
    for i in range(blocks):
        s += (f"$ ls -la /{seed}/path/{i} [exit 0, 01:{i%60:02d}:00]\n"
              f"total {i*7}\ndrwxr-xr-x {i} user staff {i*32} dir{i}\n")
    return s + "\nQuestion: summarize\nAnswer:"

out = []
hdr = f"{'blocks':>7} {'tokens':>7} {'cold_ms':>9} {'warm_ms':>9} {'us/tok':>8} {'saved':>7}"
print(hdr); out.append(hdr)
rows = []
for blocks in (10, 40, 120, 240):
    colds, warms, ntok = [], [], 0
    for rep in range(REPS):
        # cold: a prefix this model has never seen (unique seed per rep)
        c, _ = call(build(blocks, f"cold{blocks}x{rep}"))
        colds.append(c)
        # warm: establish, then measure an identical re-ask (maximal hit)
        p = build(blocks, f"warm{blocks}x{rep}")
        call(p)
        w, ntok = call(p)
        warms.append(w)
    cold, warm = statistics.median(colds), statistics.median(warms)
    rows.append((blocks, ntok, cold, warm))
    line = (f"{blocks:>7} {ntok:>7} {cold:>9.1f} {warm:>9.1f} "
            f"{warm/ntok*1000 if ntok else 0:>8.1f} {100*(1-warm/cold) if cold else 0:>6.0f}%")
    print(line); out.append(line)
    sys.stdout.flush()

# Judge from the MARGINAL cost of cached tokens, not from the endpoints:
# comparing row[0] to row[-1] conflates a fully-cached small prompt with
# whatever anomaly happens to land on the last row.
big, small = rows[-1], rows[1]
d_tok = big[1] - small[1]
marg_warm = (big[3] - small[3]) / d_tok * 1000 if d_tok else 0     # us/token
marg_cold = (big[2] - small[2]) / d_tok * 1000 if d_tok else 0
saved = [100 * (1 - w / c) for _, _, c, w in rows if c]
verdict = [
    f"\n  marginal cost per cached token: {marg_warm:.0f}us  "
    f"(vs {marg_cold:.0f}us uncached, {100*marg_warm/marg_cold if marg_cold else 0:.0f}%)",
    f"  per-size savings: " + ", ".join(f"{s:.0f}%" for s in saved),
]
if marg_cold and marg_warm < marg_cold * 0.25:
    verdict.append("  => cache is STRONG and sublinear: keeping the prefix")
    verdict.append("     byte-stable (ORDERING) is a large lever.")
elif marg_cold and marg_warm > marg_cold * 0.6:
    verdict.append("  => reuse is PARTIAL: prompt SIZE dominates ordering.")
else:
    verdict.append("  => mixed; defer to the full sweep.")
for v in verdict: print(v); out.append(v)

open(__file__.replace("floor_probe.py", "floor_result.txt"), "w").write("\n".join(out) + "\n")
