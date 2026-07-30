#!/bin/bash
# Situated context on the real corpus, then the long-context probe.
# Never edit while running.
cd /Volumes/case/git/goulash || exit 1
D=bench/results/2026-07-28
export GOULASH_BENCH_MAX_GB=12
SUB="gemma4:e2b,gemma4:e4b,gemma4:12b,qwen3.5:2b,qwen3.5:4b,qwen3.5:9b,gemma3:12b,llama3.2:3b,qwen/qwen3-4b,qwen/qwen3-8b,google/gemma-4-12b-qat"
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
restart_engines() {
  unload
  "$HOME/.lmstudio/bin/lms" server stop >/dev/null 2>&1
  pkill -f "ollama serve|Ollama.app.*ollama" 2>/dev/null
  sleep 5
  open -a Ollama 2>/dev/null || (ollama serve >/dev/null 2>&1 &)
  "$HOME/.lmstudio/bin/lms" server start >/dev/null 2>&1
  for _ in $(seq 30); do curl -s -m 2 http://127.0.0.1:11434/api/tags >/dev/null 2>&1 && break; sleep 2; done
  for _ in $(seq 30); do curl -s -m 2 http://127.0.0.1:1234/v1/models  >/dev/null 2>&1 && break; sleep 2; done
  echo "  wired: $(vm_stat | awk '/wired/ {gsub(/\./,"",$NF); printf "%.1f", $NF*16384/1073741824}') GB"
}
trap 'echo "=== interrupted $(date) ==="; [ -n "$child" ] && kill -TERM $child 2>/dev/null; wait $child 2>/dev/null; unload; exit 130' INT TERM

# 1. Situated context (S4/S5) on the FULL corpus, curated model subset.
#    S3 already exists for these cells, so this is a paired comparison.
restart_engines
echo "=== situated S4/S5 on full corpus $(date) ==="
GOULASH_BENCH_ONLY="$SUB" GOULASH_BENCH_SHAPES=S4,S5 \
  ./target/debug/goulash-bench pass-b "$D" & child=$!; wait $child; child=""

# 2. Long context: bulk session, big tails, 32k window. Separate results
#    dir because the log-building knobs change every prompt, so these rows
#    are not comparable with the main journal.
restart_engines
echo "=== long-context bulk $(date) ==="
GOULASH_BENCH_ONLY="$SUB" GOULASH_BENCH_SHAPES=S3 \
  GOULASH_BENCH_TAIL_CHARS=12000 GOULASH_BENCH_CONTEXT_MAX=90000 \
  GOULASH_BENCH_NUM_CTX=32768 \
  ./target/debug/goulash-bench pass-b bench/results/longctx & child=$!; wait $child; child=""

echo "=== done $(date) ==="
unload
