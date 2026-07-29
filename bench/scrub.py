#!/usr/bin/env python3
"""Remove suspect rows from a journal so a resume re-runs them.

Rows are dropped, never edited: the journal stays append-only in spirit,
and the resume path rebuilds them from scratch. Refuses to run while a
sweep holds the directory lock.
"""
import json, os, sys, time

d = sys.argv[1] if len(sys.argv) > 1 else "bench/results/2026-07-28"
keys = set(sys.argv[2:])
jp = os.path.join(d, "journal.jsonl")

lock = os.path.join(d, "sweep.lock")
if os.path.exists(lock):
    pid = int(open(lock).read().strip() or 0)
    try:
        os.kill(pid, 0)
        sys.exit(f"refusing: sweep pid {pid} still holds {d}")
    except (ProcessLookupError, PermissionError):
        pass

rows = [json.loads(l) for l in open(jp) if l.strip()]
if not keys:
    # Default: rows with the torn-stream signature — no stop_reason and no
    # token counts means the response never completed.
    keys = {r["key"] for r in rows
            if not r["error"] and r["stop_reason"] is None and r["eval_tokens"] is None}
    print(f"auto-detected {len(keys)} torn row(s)")

kept = [r for r in rows if r["key"] not in keys]
dropped = len(rows) - len(kept)
if not dropped:
    print("nothing to drop")
    sys.exit(0)
os.rename(jp, jp + f".bak-{int(time.time())}")
with open(jp, "w") as f:
    for r in kept:
        f.write(json.dumps(r) + "\n")
print(f"dropped {dropped} row(s); {len(kept)} remain. Re-run the pass to refill.")
for k in sorted(keys):
    print(f"  {k}")
