#!/bin/bash
# Ping-pong: gemma4:e4b across every situated arm, then read out.
cd /Volumes/case/git/goulash || exit 1
export GOULASH_BENCH_MAX_GB=12
child=""
unload() {
  for m in $(curl -s -m 5 http://127.0.0.1:11434/api/ps 2>/dev/null \
    | python3 -c 'import json,sys
try: print("\n".join(m.get("name") or m.get("model","") for m in json.load(sys.stdin).get("models",[])))
except Exception: pass' 2>/dev/null); do
    curl -s -m 10 http://127.0.0.1:11434/api/generate -d "{\"model\":\"$m\",\"keep_alive\":0}" >/dev/null 2>&1
  done
  "$HOME/.lmstudio/bin/lms" unload --all >/dev/null 2>&1
}
trap 'echo "=== interrupted $(date) ==="; [ -n "$child" ] && kill -TERM $child 2>/dev/null; wait $child 2>/dev/null; unload; exit 130' INT TERM
echo "=== gemma4:e4b — S4 platform / S5 +tools / S6 +full PATH / S7 same-as-S5-via-MEMORY $(date) ==="
GOULASH_BENCH_ONLY="gemma4:e4b" GOULASH_BENCH_SHAPES=S4,S5,S6,S7 \
  ./target/debug/goulash-bench pass-b bench/results/2026-07-28 & child=$!; wait $child; child=""
unload
echo "=== READOUT READY $(date) ==="
