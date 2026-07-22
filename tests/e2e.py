#!/usr/bin/env python3
"""End-to-end tests for the goulash M0 PTY wrapper.

Drives the goulash binary under a real PTY (stdlib only, no pexpect) and
checks: shrunken winsize, byte passthrough, status row, scroll-region
assertion, resize propagation, and exit-code propagation.
"""
import fcntl
import glob
import shutil
import json
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

BIN = os.path.join(os.path.dirname(__file__), "..", "target", "debug", "goulash")
ROWS, COLS = 24, 80
failures = []


def set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def spawn(argv, rows=ROWS, cols=COLS, home=None):
    mfd, sfd = pty.openpty()
    set_winsize(mfd, rows, cols)
    home = home or tempfile.mkdtemp(prefix="goulash-test-")
    env = dict(os.environ, GOULASH_HOME=home, HOME=home, TERM="xterm-256color")
    proc = subprocess.Popen(
        [BIN] + argv, stdin=sfd, stdout=sfd, stderr=sfd,
        start_new_session=True, env=env, close_fds=True,
    )
    os.close(sfd)
    return proc, mfd


def read_until(mfd, pattern, timeout=8.0, acc=b""):
    """Read from the pty until regex `pattern` matches or timeout."""
    deadline = time.time() + timeout
    rx = re.compile(pattern, re.DOTALL)
    while time.time() < deadline:
        if rx.search(acc):
            return acc
        r, _, _ = select.select([mfd], [], [], 0.2)
        if mfd in r:
            try:
                data = os.read(mfd, 65536)
            except OSError:
                break
            if not data:
                break
            acc += data
    return acc


def check(name, cond, detail=""):
    tag = "PASS" if cond else "FAIL"
    print(f"  [{tag}] {name}" + ("" if cond else f"  -- {detail}"))
    if not cond:
        failures.append(name)


def drain_exit(proc, mfd, timeout=8.0):
    deadline = time.time() + timeout
    while time.time() < deadline and proc.poll() is None:
        r, _, _ = select.select([mfd], [], [], 0.2)
        if mfd in r:
            try:
                os.read(mfd, 65536)
            except OSError:
                break
    os.close(mfd)
    return proc.wait(timeout=5)


def test_basic():
    print("basic session (bash --norc):")
    proc, mfd = spawn(["bash", "--norc"])
    out = read_until(mfd, rb"\$")  # prompt
    inner = ROWS - 4  # fixed goulash area: rule+question+text+chrome
    check("scroll region asserted", f"\x1b[1;{inner}r".encode() in out,
          "no DECSTBM 1..%d in %r" % (inner, out[-200:]))
    check("status row drawn", b"goulash" in out and b"bash" in out)
    check("ingress tip shown at idle", b"#/help for help" in
          read_until(mfd, rb"#/help for help", 3.0, out))

    os.write(mfd, b"echo LINES=$(tput lines) COLS=$(tput cols)\r")
    out = read_until(mfd, rb"LINES=\d+ COLS=\d+")
    m = re.search(rb"LINES=(\d+) COLS=(\d+)", out)
    check("inner sees shrunken rows", m and m.group(1) == str(inner).encode(),
          f"got {m and m.group(1)}")
    check("inner sees full cols", m and m.group(2) == str(COLS).encode(),
          f"got {m and m.group(2)}")

    os.write(mfd, b"echo marker-$((6*7))\r")
    out = read_until(mfd, rb"marker-42")
    check("passthrough works", b"marker-42" in out)

    # Resize the outer terminal and confirm propagation.
    set_winsize(mfd, 30, 100)
    os.killpg(os.getpgid(proc.pid), signal.SIGWINCH)
    time.sleep(0.3)
    os.write(mfd, b"echo RESIZED=$(tput lines)x$(tput cols)\r")
    out = read_until(mfd, rb"RESIZED=\d+x\d+")
    m = re.search(rb"RESIZED=(\d+)x(\d+)", out)
    check("resize propagates (rows-4)", m and m.group(1) == b"26", f"got {m and m.group(1)}")
    check("resize propagates (cols)", m and m.group(2) == b"100", f"got {m and m.group(2)}")

    os.write(mfd, b"exit\r")
    code = drain_exit(proc, mfd)
    check("clean exit code 0", code == 0, f"got {code}")


def test_exit_code():
    print("exit-code propagation:")
    proc, mfd = spawn(["bash", "--norc"])
    read_until(mfd, rb"\$")
    os.write(mfd, b"exit 3\r")
    code = drain_exit(proc, mfd)
    check("exit 3 propagates", code == 3, f"got {code}")


def test_fullscreen_clear():
    print("full clear does not kill the status row:")
    proc, mfd = spawn(["bash", "--norc"])
    read_until(mfd, rb"\$")
    os.write(mfd, b"clear; echo AFTERCLEAR\r")
    out = read_until(mfd, rb"AFTERCLEAR")
    # after the clear (2J/3J trigger), a fresh status draw must appear
    tail = out[out.find(b"\x1b[2J"):] if b"\x1b[2J" in out else out[out.find(b"\x1b[3J"):]
    check("status redrawn after clear", b"goulash" in read_until(mfd, rb"goulash", 3.0, tail))
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)


def test_erase_below():
    print("erase-below (ESC[J) repaints the bar in the same batch:")
    proc, mfd = spawn(["bash", "--norc"])
    read_until(mfd, rb"\$")
    os.write(mfd, b"printf 'X\\033[J'; echo EJ-$((5*9))\r")
    out = read_until(mfd, rb"EJ-45")
    idx = out.rfind(b"X\x1b[J")
    check("ESC[J seen in stream", idx != -1)
    tail = out[idx:] + read_until(mfd, rb"goulash", 3.0)
    check("bar repainted after ESC[J", b"goulash" in tail)
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)


def test_state_log():
    print("session transcript records state transitions:")
    home = tempfile.mkdtemp(prefix="goulash-test-")
    proc, mfd = spawn(["bash", "--norc"], home=home)
    read_until(mfd, rb"\$")
    os.write(mfd, b"sleep 0.6\r")
    time.sleep(1.0)
    read_until(mfd, rb"\$")
    os.write(mfd, b"read -s x\r")
    time.sleep(0.6)
    os.write(mfd, b"topsecret\r")
    time.sleep(0.4)
    os.write(mfd, b"exit\r")
    code = drain_exit(proc, mfd)

    logs = glob.glob(os.path.join(home, "history", "session-*.jsonl"))
    check("transcript file created", len(logs) == 1, f"found {logs}")
    if not logs:
        return
    events = [json.loads(line) for line in open(logs[0])]
    evs = [e["ev"] for e in events]
    check("start event", "start" in evs)
    check("end event with code", any(e["ev"] == "end" and e["code"] == 0 for e in events),
          f"end events: {[e for e in events if e['ev'] == 'end']}")
    check("output recorded", "out" in evs)
    states = [e for e in events if e["ev"] == "state"]
    check("child fg seen (sleep)", any(s["fg"] == "child" for s in states),
          f"states: {states}")
    check("returned to shell fg", states and states[-1]["fg"] == "shell")
    check("echo-off seen (read -s)", any(not s["echo"] and s["fg"] == "shell" for s in states),
          f"states: {states}")
    raw = open(logs[0], "rb").read()
    check("secret input NOT recorded", b"topsecret" not in raw)
    check("goulash exited 0", code == 0, f"got {code}")


def test_shell_hooks():
    print("bash integration emits command blocks:")
    home = tempfile.mkdtemp(prefix="goulash-test-")
    rc = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "shell", "goulash.bash"))
    proc, mfd = spawn(["bash", "--rcfile", rc, "-i"], home=home)
    read_until(mfd, rb"\$")
    os.write(mfd, b"echo hook-$((6*7))\r")
    read_until(mfd, rb"hook-42")
    os.write(mfd, b"false\r")
    time.sleep(0.6)
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)

    logs = glob.glob(os.path.join(home, "history", "session-*.jsonl"))
    check("transcript created", len(logs) == 1, f"found {logs}")
    if not logs:
        return
    events = [json.loads(line) for line in open(logs[0])]
    cmds = [e for e in events if e["ev"] == "cmd"]
    ends = [e for e in events if e["ev"] == "cmd_end"]
    prompts = [e for e in events if e["ev"] == "prompt"]
    cwds = [e for e in events if e["ev"] == "cwd"]
    check("cmd block with command text", any("echo hook-" in c["text"] for c in cmds),
          f"cmds: {cmds}")
    check("failing exit code recorded", any(e["code"] == 1 for e in ends),
          f"ends: {ends}")
    check("prompt marks recorded", len(prompts) >= 2, f"{len(prompts)} prompts")
    check("cwd recorded", any(c["path"].startswith("/") for c in cwds), f"cwds: {cwds}")
    import base64 as b64mod
    raw_out = b"".join(b64mod.b64decode(e["b64"]) for e in events if e["ev"] == "out")
    check("marks stripped from recorded output", b"\x1b]7770;" not in raw_out)


def test_suggestions():
    print("rules vendor + Alt-Down acceptance:")
    home = tempfile.mkdtemp(prefix="goulash-test-")
    rc = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "shell", "goulash.bash"))
    proc, mfd = spawn(["bash", "--rcfile", rc, "-i"], home=home)
    read_until(mfd, rb"\$")
    os.write(mfd, b"lls\r")  # typo'd ls -> command not found -> rules vendor
    out = read_until(mfd, "suggestion: ls".encode())  # bar redraw with the suggestion arrow
    check("suggestion shown in bar", "suggestion: ls".encode() in out, out[-200:])

    if sys.platform.startswith("linux"):  # bracketed paste needs readline >= 8.1
        os.write(mfd, b"\x1b[1;3B")  # Alt-Down: pull suggestion into the line
        time.sleep(0.5)
        os.write(mfd, b"\r")
        out = read_until(mfd, rb"Cargo\.toml")
        check("accepted suggestion executed", b"Cargo.toml" in out, out[-200:])

    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)

    logs = glob.glob(os.path.join(home, "history", "session-*.jsonl"))
    events = [json.loads(line) for line in open(logs[0])] if logs else []
    sugg = [e for e in events if e["ev"] == "suggest"]
    check("suggest event recorded", any(s["cmd"] == "ls" for s in sugg), f"{sugg}")
    check("suggestion has a why", any("PATH" in s["why"] for s in sugg))
    if sys.platform.startswith("linux"):
        check("accept event recorded", any(e["ev"] == "accept" for e in events))


def test_zsh_auto_integration():
    print("zsh auto-integration, zero setup (# aside + plain Down pull):")
    if not shutil.which("zsh"):
        print("  [SKIP] zsh not installed")
        return
    home = tempfile.mkdtemp(prefix="goulash-test-")
    proc, mfd = spawn(["zsh"], home=home)
    time.sleep(1.5)
    os.write(mfd, b"lls\r")
    out = read_until(mfd, "suggestion: ls".encode())
    check("suggestion vended under zsh", "suggestion: ls".encode() in out, out[-300:])
    os.write(mfd, b"\x1b[B")  # plain Down, past end of history
    time.sleep(0.6)
    os.write(mfd, b"\r")
    out = read_until(mfd, rb"Cargo\.toml")
    check("Down pull executed", b"Cargo.toml" in out, out[-300:])
    # Success cleared the live suggestion list, but the slot HISTORY
    # survives: Down again re-enters it, with the position indicator.
    os.write(mfd, b"\x1b[B")
    out = read_until(mfd, rb"1/1", 4.0)
    check("slot history survives the clear", b"1/1" in out, out[-300:])
    os.write(mfd, b"\x15")  # ^U: abandon the browsed slot
    time.sleep(0.3)
    os.write(mfd, b"# hello goulash\r")
    out = read_until(mfd, rb"no engine", 5.0)
    check("aside acknowledged (no engine reachable)", b"no engine" in out, out[-300:])
    time.sleep(0.5)
    os.write(mfd, b"\x1b[A")  # Up: native history recall of the aside
    out = read_until(mfd, rb"# hello goulash", 4.0)
    check("aside recallable from history", b"# hello goulash" in out, out[-200:])
    os.write(mfd, b"\x03")  # abort the recalled line
    time.sleep(0.3)
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)
    logs = glob.glob(os.path.join(home, "history", "session-*.jsonl"))
    events = [json.loads(line) for line in open(logs[0])] if logs else []
    check("aside recorded",
          any(e["ev"] == "aside" and "hello goulash" in e["text"] for e in events))
    check("accept recorded", any(e["ev"] == "accept" for e in events))


def test_engine_ollama():
    print("engine probe + # aside answered (fake ollama):")
    if not shutil.which("zsh"):
        print("  [SKIP] zsh not installed")
        return
    import http.server
    import threading

    class FakeOllama(http.server.BaseHTTPRequestHandler):
        def _send(self, obj):
            body = json.dumps(obj).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == "/api/tags":
                self._send({"models": [
                    {"name": "bigmodel", "size": 9_000_000_000},
                    {"name": "fakemodel", "size": 1_000_000_000},
                ]})
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            req = json.loads(self.rfile.read(n) or b"{}")
            if "prompt" not in req:
                # prewarm/load request: model + keep_alive only
                assert req.get("keep_alive"), "keep_alive missing from warm"
                self._send({"done": True})
                return
            assert req.get("keep_alive"), "keep_alive missing from request"
            assert "Session log" in req.get("prompt", ""), "stable preamble missing"
            opts = req.get("options", {})
            assert opts.get("num_predict"), "token cap missing"
            assert opts.get("num_ctx"), "num_ctx missing"
            assert req.get("think") is False, "think:false missing"
            ans = f"ANS-{req.get('model')}"
            if "goulash:" in req.get("prompt", ""):
                ans += "-CTX"  # proof the chat history reached the prompt
            ans += f"\nCMD: echo from-{req.get('model')}"
            if req.get("stream"):
                body = (json.dumps({"response": ans, "done": False}) + "\n"
                        + json.dumps({"response": "", "done": True}) + "\n").encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                self._send({"response": ans})

        def log_message(self, *a):
            pass

    srv = http.server.HTTPServer(("127.0.0.1", 0), FakeOllama)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    home = tempfile.mkdtemp(prefix="goulash-test-")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write(f'[engine]\nprovider = "ollama"\nhost = "http://127.0.0.1:{port}"\n')
    proc, mfd = spawn(["zsh"], home=home)
    time.sleep(1.5)
    os.write(mfd, b"# what is the answer\r")
    out = read_until(mfd, rb"ANS-fakemodel", 8.0)
    check("smallest model auto-picked, answer shown", b"ANS-fakemodel" in out, out[-300:])
    # Band opens: status(22) + question(23) + text(24) on a 24-row term.
    out += read_until(mfd, rb"\x1b\[23;1H", 3.0)
    check("heckle band opened (question row drawn)", b"\x1b[23;1H" in out)
    os.write(mfd, b"#/model bigmodel\r")
    time.sleep(0.8)
    os.write(mfd, b"# again please\r")
    out = read_until(mfd, rb"ANS-bigmodel-CTX", 8.0)
    check("#/model switch took effect", b"ANS-bigmodel" in out, out[-300:])
    check("follow-up ask carries chat history", b"ANS-bigmodel-CTX" in out, out[-300:])
    os.write(mfd, b"#/status\r")
    out = read_until(mfd, rb"blocks this session", 5.0)
    check("#/status shows engine", b"bigmodel" in out, out[-300:])
    # Proactive commentary: a plain command turn should produce an
    # unprompted engine answer (fake always replies, never PASS).
    os.write(mfd, b"echo ctest-$((3*4))\r")
    read_until(mfd, rb"ctest-12", 5.0)
    time.sleep(1.5)
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)
    srv.shutdown()

    logs = glob.glob(os.path.join(home, "history", "session-*.jsonl"))
    events = [json.loads(line) for line in open(logs[0])] if logs else []
    check("engine ready recorded",
          any(e["ev"] == "engine" and e["model"] == "fakemodel" for e in events))
    check("model switch recorded",
          any(e["ev"] == "engine" and e["model"] == "bigmodel" for e in events))
    check("answer recorded",
          any(e["ev"] == "answer" and e["ok"] and "ANS-" in e["text"] for e in events))
    check("candidate command vended as suggestion",
          any(e["ev"] == "suggest" and e["vendor"] == "engine"
              and e["cmd"].startswith("echo from-") for e in events))
    asides = [e for e in events if e["ev"] == "aside"]
    answers = [e for e in events if e["ev"] == "answer" and e["ok"]]
    check("proactive commentary answered without an ask",
          len(answers) > 2, f"{len(asides)} asides, {len(answers)} answers")
    check("commentary suggestion tagged",
          any(e["ev"] == "suggest" and e.get("why") == "commentary" for e in events))


def test_memory():
    print("#/memory flat store + model REMEMBER line (fake ollama):")
    if not shutil.which("zsh"):
        print("  [SKIP] zsh not installed")
        return
    import http.server
    import threading

    class FakeOllama(http.server.BaseHTTPRequestHandler):
        def _send(self, obj):
            body = json.dumps(obj).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == "/api/tags":
                self._send({"models": [{"name": "memmodel", "size": 1}]})
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            req = json.loads(self.rfile.read(n) or b"{}")
            if "prompt" not in req:
                self._send({"done": True})
                return
            p = req["prompt"]
            if "Question: save a note" in p:
                # Echo proof that the pinned block (with the user's slot)
                # reached the stable prefix, and exercise the tool line.
                if "Pinned memories" in p and "make release TARGET=prod" in p:
                    ans = "noted MEM-SEEN\nREMEMBER: model saved this note"
                else:
                    ans = "MEM-MISSING"
            else:
                ans = "PASS"  # keep proactive commentary silent
            if req.get("stream"):
                body = (json.dumps({"response": ans, "done": False}) + "\n"
                        + json.dumps({"response": "", "done": True}) + "\n").encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                self._send({"response": ans})

        def log_message(self, *a):
            pass

    srv = http.server.HTTPServer(("127.0.0.1", 0), FakeOllama)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    home = tempfile.mkdtemp(prefix="goulash-test-")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write(f'[engine]\nprovider = "ollama"\nhost = "http://127.0.0.1:{port}"\n')
    proc, mfd = spawn(["zsh"], home=home)
    time.sleep(1.5)
    os.write(mfd, b"#/memory\r")
    out = read_until(mfd, rb"memory off", 5.0)
    check("memory defaults off", b"memory off" in out, out[-300:])
    os.write(mfd, b"#/memory on\r")
    out = read_until(mfd, rb"memory on", 5.0)
    check("#/memory on", b"memory on" in out, out[-300:])
    os.write(mfd, b"#/memory add deploy is make release TARGET=prod\r")
    out = read_until(mfd, rb"remembered \[1\]", 5.0)
    check("user add stored", b"remembered [1]" in out, out[-300:])
    os.write(mfd, b"# save a note\r")
    out = read_until(mfd, rb"MEM-SEEN", 8.0)
    check("pinned block reached the prompt", b"MEM-SEEN" in out, out[-300:])
    time.sleep(0.5)  # let the REMEMBER op land in the store
    os.write(mfd, b"#/memory find saved\r")
    out = read_until(mfd, rb"model saved this note", 5.0)
    check("model REMEMBER stored", b"model saved this note" in out, out[-300:])
    os.write(mfd, b"#/memory delete 1\r")
    out = read_until(mfd, rb"forgot \[1\]", 5.0)
    check("#/memory delete", b"forgot [1]" in out, out[-300:])
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)
    srv.shutdown()

    mpath = os.path.join(home, "memory.toml")
    check("memory.toml durable", os.path.exists(mpath))
    mt = open(mpath).read() if os.path.exists(mpath) else ""
    check("model note persisted", "model saved this note" in mt, mt[-300:])
    check("deleted slot gone from toml", "TARGET=prod" not in mt, mt[-300:])
    check("enabled persists in toml", "enabled = true" in mt, mt[:120])
    logs = glob.glob(os.path.join(home, "history", "session-*.jsonl"))
    events = [json.loads(line) for line in open(logs[0])] if logs else []
    check("llm memory add recorded",
          any(e["ev"] == "memory" and e["op"] == "add" for e in events))


def test_non_tty():
    print("refuses to run without a tty:")
    r = subprocess.run([BIN, "true"], capture_output=True)
    check("exit code 2", r.returncode == 2, f"got {r.returncode}")
    check("clear error message", b"must be a terminal" in r.stderr)


def main():
    for t in (
        test_basic,
        test_exit_code,
        test_fullscreen_clear,
        test_erase_below,
        test_state_log,
        test_shell_hooks,
        test_suggestions,
        test_zsh_auto_integration,
        test_engine_ollama,
        test_memory,
        test_non_tty,
    ):
        try:
            t()
        except Exception as e:  # noqa: BLE001
            check(t.__name__ + " (no exception)", False, repr(e))
    print()
    if failures:
        print(f"{len(failures)} FAILED: {failures}")
        sys.exit(1)
    print("all tests passed")


if __name__ == "__main__":
    main()
