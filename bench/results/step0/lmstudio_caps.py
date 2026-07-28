#!/usr/bin/env python3
"""Probe what LM Studio's OpenAI-compatible API actually honors.

Every answer here lands in Caps (src/engine/provider.rs). Guessing any of
them means mis-attributing a provider limitation to the model.
"""
import json, time, urllib.error, urllib.request

HOST, MODEL = "http://127.0.0.1:1234", "qwen/qwen3-1.7b"
PROMPT = "Reply with exactly two lines separated by a blank line.\nLine one.\n\nLine two."

def post(path, body, stream=False, timeout=180):
    req = urllib.request.Request(f"{HOST}{path}", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    t0 = time.time()
    try:
        r = urllib.request.urlopen(req, timeout=timeout)
        if not stream:
            return json.loads(r.read()), (time.time()-t0)*1000, None
        chunks, ttft = [], None
        for raw in r:
            line = raw.decode(errors="replace").strip()
            if not line.startswith("data:"): continue
            p = line[5:].strip()
            if p == "[DONE]": break
            try: v = json.loads(p)
            except Exception: continue
            if ttft is None and (v.get("choices") or [{}])[0].get("delta", {}).get("content"):
                ttft = (time.time()-t0)*1000
            chunks.append(v)
        return chunks, (time.time()-t0)*1000, ttft
    except urllib.error.HTTPError as e:
        return {"__http_error__": e.code, "body": e.read().decode()[:200]}, (time.time()-t0)*1000, None
    except Exception as e:
        return {"__error__": str(e)}, (time.time()-t0)*1000, None

res = {}
def report(k, ok, detail=""):
    res[k] = ok
    print(f"  {'YES' if ok else ' NO'}  {k:34s} {detail}")

print(f"LM Studio capability probe  model={MODEL}\n")

# 1. raw completions endpoint (matches ollama's /api/generate shape)
v, ms, _ = post("/v1/completions", {"model": MODEL, "prompt": "Say hi.", "max_tokens": 8})
ok = isinstance(v, dict) and "choices" in v
report("/v1/completions (raw prompt)", ok,
       f"{ms:.0f}ms" if ok else str(v)[:90])

# 2. chat endpoint
v, ms, _ = post("/v1/chat/completions",
                {"model": MODEL, "messages": [{"role": "user", "content": "Say hi."}],
                 "max_tokens": 8})
report("/v1/chat/completions", isinstance(v, dict) and "choices" in v, f"{ms:.0f}ms")
usage = v.get("usage", {}) if isinstance(v, dict) else {}
report("usage.prompt_tokens (non-stream)", bool(usage.get("prompt_tokens")),
       f"prompt={usage.get('prompt_tokens')} completion={usage.get('completion_tokens')}")
report("any prompt-eval TIMING in response",
       any(k in (v if isinstance(v, dict) else {}) for k in ("stats", "timings", "timing")),
       "-> cache metric must be client-side TTFT")

# 3. SSE streaming + TTFT
ch, ms, ttft = post("/v1/chat/completions",
                    {"model": MODEL, "messages": [{"role": "user", "content": "Count to five."}],
                     "max_tokens": 40, "stream": True}, stream=True)
report("SSE streaming", isinstance(ch, list) and len(ch) > 1,
       f"{len(ch) if isinstance(ch,list) else 0} chunks, ttft={ttft:.0f}ms" if ttft else "")

# 4. usage in a streamed response
ch, _, _ = post("/v1/chat/completions",
                {"model": MODEL, "messages": [{"role": "user", "content": "Say hi."}],
                 "max_tokens": 8, "stream": True,
                 "stream_options": {"include_usage": True}}, stream=True)
su = [c for c in ch if isinstance(c, dict) and c.get("usage")] if isinstance(ch, list) else []
report("stream_options.include_usage", bool(su),
       f"prompt_tokens={su[-1]['usage'].get('prompt_tokens')}" if su else "no usage chunk")

# 5. stop sequences  -- the top quirk suspect
v, _, _ = post("/v1/completions",
               {"model": MODEL, "prompt": PROMPT, "max_tokens": 60, "stop": ["\n\n"]})
txt = (v.get("choices") or [{}])[0].get("text", "") if isinstance(v, dict) else ""
fin = (v.get("choices") or [{}])[0].get("finish_reason") if isinstance(v, dict) else None
report("stop sequences honored", "\n\n" not in txt, f"finish_reason={fin} text={txt[:40]!r}")

# 6. reasoning suppression
v, _, _ = post("/v1/chat/completions",
               {"model": MODEL, "messages": [{"role": "user", "content": "What is 2+2?"}],
                "max_tokens": 200, "chat_template_kwargs": {"enable_thinking": False}})
err = isinstance(v, dict) and "__http_error__" in v
msg = (v.get("choices") or [{}])[0].get("message", {}) if isinstance(v, dict) and not err else {}
content = msg.get("content") or ""
report("chat_template_kwargs accepted", not err,
       f"HTTP {v.get('__http_error__')}" if err else "no 400")
report("  -> thinking actually suppressed", not err and "<think>" not in content,
       f"reasoning field={'yes' if msg.get('reasoning_content') else 'no'} "
       f"content={content[:45]!r}")

# 7. does the same model leak <think> WITHOUT the kwarg? (is the kwarg load-bearing)
v, _, _ = post("/v1/chat/completions",
               {"model": MODEL, "messages": [{"role": "user", "content": "What is 2+2?"}],
                "max_tokens": 200})
m2 = (v.get("choices") or [{}])[0].get("message", {}) if isinstance(v, dict) else {}
c2 = m2.get("content") or ""
report("thinking leaks WITHOUT the kwarg", "<think>" in c2 or bool(m2.get("reasoning_content")),
       f"content={c2[:45]!r}")

json.dump(res, open(__file__.replace("lmstudio_caps.py", "lmstudio_caps.json"), "w"), indent=2)
print(f"\n  wrote lmstudio_caps.json")
