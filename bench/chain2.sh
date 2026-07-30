#!/bin/bash
# Situated context on the real corpus, then the long-context probe.
# Never edit while running.
cd /Volumes/case/git/goulash || exit 1
D=bench/results/2026-07-28
export GOULASH_BENCH_MAX_GB=12
SUB="qwen3.5:4b,gemma4:e4b,gemma4:12b"
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
# Bounce the SERVERS, never the GUIs.
#
# `open -a Ollama` launches Ollama.app/Contents/MacOS/Ollama — the Electron
# wrapper, which is what pops the startup wizard. The server is just
# `ollama serve` (and /usr/local/bin/ollama is a symlink to the same binary
# inside the bundle), so starting it directly gives a headless server with
# no window, no menu-bar item and no wizard.
restart_engines() {
  unload
  "$HOME/.lmstudio/bin/lms" server stop >/dev/null 2>&1
  # The Electron app OWNS its server (the serve process is its child) and
  # holds :11434, so starting a headless server while the app is alive
  # silently loses the bind. The app has to go first. Headless always.
  osascript -e 'tell application "Ollama" to quit' 2>/dev/null \
    || pkill -f "Ollama.app/Contents/MacOS/Ollama" 2>/dev/null
  sleep 3
  pkill -f "ollama serve" 2>/dev/null
  sleep 3
  nohup /usr/local/bin/ollama serve >/dev/null 2>&1 &
  sleep 2
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
