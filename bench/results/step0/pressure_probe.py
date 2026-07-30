#!/usr/bin/env python3
"""Does memory pressure inflate inference latency on this machine?

The multi-turn sweep cannot answer this: session logs accumulate model
answers, answers differ run to run at temperature 0.2, so prompts diverge
after turn 1 and prompt_eval_ms is not comparable across runs.

This uses ONE fixed prompt, repeated, under two conditions:
  clean     - nothing else resident
  pressured - a large second model loaded to consume memory

Same prompt, same model, same everything else. Any difference is pressure.
"""
import json, statistics, subprocess, time, urllib.request

H = "http://127.0.0.1:11434"
SMALL, BIG, REPS = "qwen3.5:0.8b", "gemma4:12b", 7

PROMPT = ("You are goulash. Session log (oldest first):\n"
          + "".join(f"$ ls -la /p/{i} [exit 0, 01:{i%60:02d}:00]\n"
                    f"total {i*7}\ndrwxr-xr-x {i} user staff {i*32} dir{i}\n"
                    for i in range(40))
          + "\nQuestion: what is here\nAnswer:")

def gen(model, ctx=8192, keep="5m", npred=16):
    body = {"model": model, "prompt": PROMPT, "stream": False, "think": False,
            "options": {"temperature": 0.2, "num_predict": npred, "num_ctx": ctx},
            "keep_alive": keep}
    r = urllib.request.Request(f"{H}/api/generate", data=json.dumps(body).encode(),
                               headers={"Content-Type": "application/json"})
    v = json.loads(urllib.request.urlopen(r, timeout=600).read())
    return (v.get("prompt_eval_duration", 0)/1e6, v.get("eval_duration", 0)/1e6,
            v.get("prompt_eval_count") or 0)

def unload(m):
    urllib.request.urlopen(urllib.request.Request(f"{H}/api/generate",
        data=json.dumps({"model": m, "keep_alive": 0}).encode(),
        headers={"Content-Type": "application/json"}), timeout=120).read()

def freepct():
    o = subprocess.run(["memory_pressure"], capture_output=True, text=True).stdout
    for l in o.splitlines():
        if "free percentage" in l:
            return int(l.rsplit(":", 1)[1].strip().rstrip("%"))
    return -1

def wired():
    o = subprocess.run(["vm_stat"], capture_output=True, text=True).stdout
    for l in o.splitlines():
        if "wired" in l:
            return int(l.split()[-1].rstrip(".")) * 16384 / 1e9
    return 0

def measure(tag):
    gen(SMALL)  # warm the prefix cache so we measure steady state
    pe = [gen(SMALL)[0] for _ in range(REPS)]
    ev = [gen(SMALL)[1] for _ in range(REPS)]
    print(f"  {tag:<12} free={freepct():>3}% wired={wired():>5.1f}GB  "
          f"prompt_eval med={statistics.median(pe):7.1f}ms  "
          f"eval med={statistics.median(ev):6.1f}ms")
    return statistics.median(pe), statistics.median(ev)

# Refuse to run while anything else touches the GPU. The first attempt at
# this probe was confounded by a concurrent validation sweep: the
# "clean" condition measured 1326ms against 660ms for the "pressured"
# one, purely because another job held the GPU during it. Contention
# dominates memory by a wide margin, so it has to be excluded, not
# averaged over.
def busy():
    out = subprocess.run(["ps", "ax", "-o", "command"], capture_output=True, text=True).stdout
    hits = [l for l in out.splitlines()
            if ("goulash-bench" in l or "chain.sh" in l or "run.sh" in l)
            and "ps ax" not in l and "grep" not in l]
    return hits

hits = busy()
if hits:
    print("refusing: other GPU work is running —")
    for h in hits[:4]:
        print(f"  {h[:96]}")
    raise SystemExit(1)

print(f"fixed prompt, {SMALL}, {REPS} reps per condition\n")
unload(SMALL); unload(BIG); time.sleep(2)
clean = measure("CLEAN")

print(f"  ... loading {BIG} to consume memory")
gen(BIG, npred=1)
time.sleep(2)
pressed = measure("PRESSURED")

unload(BIG); time.sleep(3)
after = measure("CLEAN again")
unload(SMALL)

d = 100*(pressed[0]-clean[0])/clean[0]
print(f"\n  prompt-eval under pressure: {d:+.0f}%")
print(f"  eval under pressure:        {100*(pressed[1]-clean[1])/clean[1]:+.0f}%")
print()
if abs(d) < 10:
    print("  => memory pressure does NOT materially affect inference latency here,")
    print("     as long as the working model still fits. Pass B timings stand.")
else:
    print("  => pressure DOES move latency. Late pass-B cells need a re-run.")
