#!/bin/bash
# Sweep driver.
#
# Unloads every model on BOTH engines on any exit path — normal
# completion, Ctrl-C, or SIGTERM from a pkill. Without that, a killed run
# strands a multi-GB model resident on a machine someone is still using;
# LM Studio JIT-loads at a 1-hour TTL by default, so "it will expire on
# its own" is not an answer.
#
# The bench child runs in the BACKGROUND with an explicit wait. Bash does
# not run a trap while blocked on a foreground child, so a trap written
# the obvious way fires only after the current cell finishes — which for a
# 12 GB model is a minute of continued GPU work after you asked it to
# stop. Verified: the first version of this script failed exactly that way.
#
# usage: bench/run.sh [RESULTS_DIR]
#        GOULASH_BENCH_MAX_GB=12  cap per-model footprint (default 12)
#        GOULASH_BENCH_ONLY=...   comma-separated substring filter

cd "$(dirname "$0")/.." || exit 1
export GOULASH_BENCH_MAX_GB="${GOULASH_BENCH_MAX_GB:-12}"
D="${1:-bench/results/$(date +%F)}"
BIN=./target/debug/goulash-bench
child=""

unload_everything() {
  for m in $(curl -s -m 5 http://127.0.0.1:11434/api/ps 2>/dev/null \
             | python3 -c 'import json,sys
try:
    print("\n".join(m.get("name") or m.get("model","") for m in json.load(sys.stdin).get("models",[])))
except Exception:
    pass' 2>/dev/null); do
    curl -s -m 10 http://127.0.0.1:11434/api/generate \
      -d "{\"model\":\"$m\",\"keep_alive\":0}" >/dev/null 2>&1
    echo "  unloaded $m"
  done
  "$HOME/.lmstudio/bin/lms" unload --all >/dev/null 2>&1 && echo "  lms cleared"
}

on_signal() {
  echo "=== interrupted $(date) — stopping child and unloading ==="
  if [ -n "$child" ]; then
    kill -TERM "$child" 2>/dev/null
    wait "$child" 2>/dev/null
  fi
  unload_everything
  exit 130
}
trap on_signal INT TERM

run_pass() {
  echo "=== $1 $(date) ==="
  "$BIN" "$1" "$D" &
  child=$!
  wait "$child"
  local rc=$?
  child=""
  return $rc
}

echo "=== sweep start $(date) (cap ${GOULASH_BENCH_MAX_GB} GB, dir $D) ==="
run_pass pass-a || echo "pass-a exited non-zero"
run_pass pass-b || echo "pass-b exited non-zero"
echo "=== done $(date) ==="
unload_everything
