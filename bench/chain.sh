#!/bin/bash
# Runs the remaining passes in priority order, waiting for any in-flight
# sweep first. Each stage is independently resumable; a failure in one
# does not block the next. Never edit this while it runs — bash reads a
# script incrementally, so an in-place edit can make it execute garbage.
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
trap 'echo "=== chain interrupted $(date) ==="; [ -n "$child" ] && kill -TERM $child 2>/dev/null; wait $child 2>/dev/null; unload; exit 130' INT TERM

while pgrep -f "bench/run.sh" >/dev/null 2>&1; do sleep 30; done

stage() {
  local label="$1"; shift
  echo "=== $label $(date) ==="
  "$@" & child=$!; wait $child; child=""
  unload
}

# 1. Prompt-wording variants — the refusal and memory-hijack hypotheses.
stage "pass-p prompt variants" ./target/debug/goulash-bench pass-p "$D"
# 2. Thinking vs display budget — what should the reasoning allowance be?
stage "pass-t thinking budget" ./target/debug/goulash-bench pass-t "$D"
# 3. Pressure-test the findings across five more sessions, chosen shape only.
stage "pass-b S3 expanded corpus" \
  env GOULASH_BENCH_SHAPES=S3 ./target/debug/goulash-bench pass-b "$D"
echo "=== chain done $(date) ==="
unload
