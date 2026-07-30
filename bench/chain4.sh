#!/bin/bash
# Situated arms against the models that ACTUALLY make platform errors:
# qwen3.5:4b (5.5%), llama3.2:3b (6.1%), qwen3.5:9b (5.2%).
# gemma4:e4b was a 1% offender — no signal to move.
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
echo "=== offenders: qwen3.5:4b, llama3.2:3b, qwen3.5:9b — S4..S7 $(date) ==="
GOULASH_BENCH_ONLY="qwen3.5:4b,llama3.2:3b,qwen3.5:9b" GOULASH_BENCH_SHAPES=S4,S5,S6,S7 \
  ./target/debug/goulash-bench pass-b bench/results/2026-07-28 & child=$!; wait $child; child=""
unload
echo "=== OFFENDER READOUT READY $(date) ==="
