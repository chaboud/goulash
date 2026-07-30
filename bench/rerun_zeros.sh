#!/bin/bash
# Re-run the cells that scored 0.00 in QUALITY.md. All three were
# starved by the old settings (stop sequence + no reasoning allowance),
# not incapable — so the ranking needs their real numbers.
cd /Volumes/case/git/goulash || exit 1
export GOULASH_BENCH_MAX_GB=12
D=bench/results/rerun-0.4.0
unload() {
  for m in $(curl -s -m 5 http://127.0.0.1:11434/api/ps 2>/dev/null \
    | python3 -c 'import json,sys
try: print("\n".join(m.get("name") or m.get("model","") for m in json.load(sys.stdin).get("models",[])))
except Exception: pass' 2>/dev/null); do
    curl -s -m 10 http://127.0.0.1:11434/api/generate -d "{\"model\":\"$m\",\"keep_alive\":0}" >/dev/null 2>&1
  done
  "$HOME/.lmstudio/bin/lms" unload --all >/dev/null 2>&1
}
trap 'unload; exit 130' INT TERM
echo "=== rerun of the 0.00 cells under 0.4.0 settings $(date) ==="
GOULASH_BENCH_ONLY="deepseek-r1,qwen/qwen3-8b,google/gemma-4-e4b" \
  GOULASH_BENCH_SHAPES=S1,S3 ./target/debug/goulash-bench pass-b "$D"
unload
echo "=== RERUN DONE $(date) ==="
