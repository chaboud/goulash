#!/bin/bash
# Runs the remaining passes in priority order, waiting for any in-flight
# sweep first. Each stage is independently resumable; a failure in one
# does not block the next. Never edit while running — bash reads scripts
# incrementally.
cd /Volumes/case/git/goulash || exit 1
D="${1:-bench/results/2026-07-28}"
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

wired_gb() { vm_stat | awk '/wired/ {gsub(/\./,"",$NF); printf "%.1f", $NF*16384/1073741824}'; }

# Unloading a model does NOT return its GPU allocation. Measured: the
# driver held 20.4 GB allocated / 18.3 GB in use while all process RSS
# totalled 6.96 GB and a single 4.6 GB model was resident. Wired memory
# accumulates across load/unload cycles, so a long multi-pass run has to
# bounce the servers to reclaim it.
restart_engines() {
  echo "  wired before restart: $(wired_gb) GB"
  unload
  "$HOME/.lmstudio/bin/lms" server stop  >/dev/null 2>&1
  pkill -f "ollama serve|Ollama.app.*ollama" 2>/dev/null
  sleep 5
  open -a Ollama 2>/dev/null || (ollama serve >/dev/null 2>&1 &)
  "$HOME/.lmstudio/bin/lms" server start >/dev/null 2>&1
  for _ in $(seq 30); do
    curl -s -m 2 http://127.0.0.1:11434/api/tags >/dev/null 2>&1 && break
    sleep 2
  done
  for _ in $(seq 30); do
    curl -s -m 2 http://127.0.0.1:1234/v1/models >/dev/null 2>&1 && break
    sleep 2
  done
  echo "  wired after restart:  $(wired_gb) GB"
}

trap 'echo "=== chain interrupted $(date) ==="; [ -n "$child" ] && kill -TERM $child 2>/dev/null; wait $child 2>/dev/null; unload; exit 130' INT TERM

# Wait on the sweep's own lock file, not pgrep. A `pgrep -f` pattern
# matches ANY command line containing it — including monitor shells that
# merely mention the script — which deadlocked this chain against my own
# watchdogs for two minutes. The lock records a real pid; check that.
wait_for_lock() {
  local lock="$D/sweep.lock"
  while [ -f "$lock" ]; do
    local pid; pid=$(cat "$lock" 2>/dev/null)
    [ -z "$pid" ] && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 20
  done
}
wait_for_lock

stage() {
  local label="$1"; shift
  restart_engines
  echo "=== $label $(date) ==="
  "$@" & child=$!; wait $child; child=""
  unload
}

stage "pass-p prompt variants" ./target/debug/goulash-bench pass-p "$D"
stage "pass-t thinking budget"  ./target/debug/goulash-bench pass-t "$D"
stage "pass-b S3 expanded corpus" \
  env GOULASH_BENCH_SHAPES=S3 ./target/debug/goulash-bench pass-b "$D"
echo "=== chain done $(date) ==="
restart_engines
