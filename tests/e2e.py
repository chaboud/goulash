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


def show_caps(model):
    """What the fake /api/show advertises. Real ollama reports a
    `capabilities` list per model; goulash trusts it over its own table,
    so the stubs have to speak it too."""
    caps = ["completion"]
    if any(f in model for f in ("qwen3", "gpt-oss", "deepseek-r1")):
        caps.append("thinking")
    return caps


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
    # Up slides back to the neutral empty line (one continuous axis):
    # if any slot text lingered, the marker command would garble.
    os.write(mfd, b"\x1b[A")
    time.sleep(0.6)
    os.write(mfd, b"echo neut-$((3*3))\r")
    out = read_until(mfd, rb"neut-9", 5.0)
    check("Up returns to the neutral empty line", b"neut-9" in out, out[-300:])
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
            if self.path == "/api/show":
                self._send({"capabilities": show_caps(req.get("model", ""))})
                return
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
            assert "think" not in req, "think sent to a non-reasoning model"
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
    # #/settings: Enter cycles a value live and persists it.
    os.write(mfd, b"#/settings\r")
    out = read_until(mfd, rb"commentary: on", 5.0)
    check("#/settings lists live values", b"commentary: on" in out, out[-300:])
    os.write(mfd, b"\r")            # cycle commentary on -> off
    out = read_until(mfd, rb"commentary: off", 4.0)
    check("Enter cycles a setting", b"commentary: off" in out, out[-300:])
    os.write(mfd, b"\r")            # ... and back on, so later checks stand
    out = read_until(mfd, rb"commentary: on", 4.0)
    check("cycling wraps around", b"commentary: on" in out, out[-300:])
    os.write(mfd, b"\x1b")
    time.sleep(0.4)
    # #/debug: the terminal-hackery drawer, same cycle mechanic.
    os.write(mfd, b"#/debug\r")
    out = read_until(mfd, rb"cursor_save: decsc", 5.0)
    check("#/debug lists the esoteric knobs",
          b"cursor_save: decsc" in out and b"idle_repaint: on" in out, out[-400:])
    os.write(mfd, b"\r")            # decsc -> absolute
    out = read_until(mfd, rb"cursor_save: absolute", 4.0)
    check("Enter cycles a debug knob", b"cursor_save: absolute" in out, out[-300:])
    os.write(mfd, b"\r")            # ... and back, so the fix stays on
    out = read_until(mfd, rb"cursor_save: decsc", 4.0)
    check("debug knob wraps back", b"cursor_save: decsc" in out, out[-300:])
    os.write(mfd, b"\x1b")
    time.sleep(0.4)
    os.write(mfd, b"#/help\r")
    out = read_until(mfd, rb"#@/path", 4.0)
    check("#/help lists current commands", b"#@/path" in out, out[-300:])
    # The reference outgrew one screen, so it filters like any menu.
    os.write(mfd, b"settings")
    out = read_until(mfd, rb"#/settings", 4.0)
    check("#/help filters to a command", b"#/settings" in out, out[-300:])
    os.write(mfd, b"\x1b")
    time.sleep(0.4)
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


def test_model_menu():
    print("bare #/model opens the modal selector; Enter persists (fake ollama):")
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
                    {"name": "gemma3:4b", "size": 3_000_000_000},
                    {"name": "qwen3:1.7b", "size": 1_000_000_000},
                ]})
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            req = json.loads(self.rfile.read(n) or b"{}")
            if self.path == "/api/show":
                self._send({"capabilities": show_caps(req.get("model", ""))})
                return
            if "prompt" not in req:
                self._send({"done": True})
                return
            self._send({"response": "PASS"})

        def log_message(self, *a):
            pass

    srv = http.server.HTTPServer(("127.0.0.1", 0), FakeOllama)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    home = tempfile.mkdtemp(prefix="goulash-test-")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write("# keep me\n[engine]\nprovider = \"ollama\"\n"
                f"host = \"http://127.0.0.1:{port}\"\nstream = false\n")
    proc, mfd = spawn(["zsh"], home=home)
    time.sleep(1.5)
    os.write(mfd, b"#/model\r")
    out = read_until(mfd, rb"model \xe2\x96\xb8", 6.0)  # title chip "model ▸"
    check("menu opened", "model ▸".encode() in out, out[-300:])
    # The area grows for the list: 24 rows - (rule + 8 items + chrome).
    check("menu grows the area (inner 24->14)", b"\x1b[1;14r" in out, out[-300:])
    out = read_until(mfd, rb"auto", 5.0)
    check("auto is a first-class entry", b"auto" in out, out[-300:])
    time.sleep(0.4)
    os.write(mfd, b"gem")  # type-to-filter
    out = read_until(mfd, rb"1/1", 5.0)
    check("filter narrows to one", b"1/1" in out, out[-300:])
    os.write(mfd, b"\r")  # Enter commits + persists; engine rebinds + warms
    out = read_until(mfd, rb"\x1b\[1;20r", 4.0)
    check("menu hands the rows back on close", b"\x1b[1;20r" in out, out[-300:])
    out = read_until(mfd, rb"gemma3:4b ready", 6.0, out)
    check("commit rebinds and warms the engine",
          b"gemma3:4b ready" in out, out[-300:])
    time.sleep(0.5)
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)
    srv.shutdown()

    conf = open(os.path.join(home, "config.toml")).read()
    check("config comment preserved", "# keep me" in conf, conf)
    check("model persisted surgically", 'model = "gemma3:4b"' in conf, conf)
    stat_path = os.path.join(home, "state.toml")
    stat = open(stat_path).read() if os.path.exists(stat_path) else ""
    check("probation recorded in state.toml", 'probation = "gemma3:4b"' in stat, stat)


def test_model_capabilities():
    print("thinking follows the model's own dialect (fake ollama):")
    if not shutil.which("zsh"):
        print("  [SKIP] zsh not installed")
        return
    import http.server
    import threading

    seen = []  # every generate request, in order

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
                    {"name": "gemma3:4b", "size": 3_000_000_000},
                    {"name": "qwen3:1.7b", "size": 1_000_000_000},
                ]})
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            req = json.loads(self.rfile.read(n) or b"{}")
            if self.path == "/api/show":
                self._send({"capabilities": show_caps(req.get("model", ""))})
                return
            if "prompt" not in req:
                self._send({"done": True})
                return
            seen.append(req)
            self._send({"response": "PASS"})

        def log_message(self, *a):
            pass

    srv = http.server.HTTPServer(("127.0.0.1", 0), FakeOllama)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    home = tempfile.mkdtemp(prefix="goulash-test-")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write("[engine]\nprovider = \"ollama\"\n"
                f"host = \"http://127.0.0.1:{port}\"\nstream = false\n"
                "model = \"gemma3:4b\"\nmax_tokens = 256\n"
                "thinking_tokens = 400\ncommentary = false\n")
    proc, mfd = spawn(["zsh"], home=home)
    time.sleep(1.5)

    # gemma cannot reason: the dial must say so rather than pretend.
    os.write(mfd, b"#/thinking high\r")
    out = read_until(mfd, "doesn't reason".encode(), 6.0)
    check("dial admits it does nothing here",
          "doesn't reason".encode() in out, out[-300:])
    os.write(mfd, b"#is this on\r")
    read_until(mfd, rb"PASS", 8.0)
    check("no think field sent to a non-reasoning model",
          seen and "think" not in seen[-1], seen[-1:] )
    check("no reasoning allowance either",
          seen and seen[-1]["options"]["num_predict"] == 256,
          seen[-1:])

    # qwen3 does, in boolean: same dial, different wire, bigger budget.
    os.write(mfd, b"#/model qwen3:1.7b\r")
    read_until(mfd, rb"qwen3:1.7b ready", 8.0)
    time.sleep(0.5)
    os.write(mfd, b"#and now\r")
    read_until(mfd, rb"PASS", 8.0)
    check("boolean reasoner gets think:true",
          seen and seen[-1].get("think") is True, seen[-1:])
    # high = twice the family's 1024, not the configured 400.
    check("allowance sized from the model, not the config",
          seen and seen[-1]["options"]["num_predict"] == 256 + 2048,
          seen[-1:])

    os.write(mfd, b"#/status\r")
    out = read_until(mfd, rb"reasons", 6.0)
    check("status reports the resolved capability",
          b"reasons on/off" in out, out[-300:])

    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)
    srv.shutdown()


def test_chat_mode():
    print("## chat focus: multi-turn without #, Up hands command to shell:")
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
                self._send({"models": [{"name": "chatmodel", "size": 1}]})
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            req = json.loads(self.rfile.read(n) or b"{}")
            if self.path == "/api/show":
                self._send({"capabilities": show_caps(req.get("model", ""))})
                return
            if "prompt" not in req:
                self._send({"done": True})
                return
            p = req["prompt"]
            if "Without being asked" in p:
                ans = "PASS"  # proactive commentary stays quiet
            else:
                FakeOllama.asks += 1
                n = FakeOllama.asks
                ans = "ANS" + ("-CTX" if "goulash:" in p else "")
                # distinct command per turn -> a browsable slot stack;
                # the $(( )) form separates display from execution
                ans += f"\nCMD: echo p{n}-$((6*{6+n}))"
            self._send({"response": ans})

        def log_message(self, *a):
            pass

    FakeOllama.asks = 0
    srv = http.server.HTTPServer(("127.0.0.1", 0), FakeOllama)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    home = tempfile.mkdtemp(prefix="goulash-test-")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write(f'[engine]\nprovider = "ollama"\nhost = "http://127.0.0.1:{port}"\n'
                'stream = false\n')
    proc, mfd = spawn(["zsh"], home=home)
    time.sleep(1.5)
    os.write(mfd, b"## first question\r")
    # Chat grows the area: reserved 4 -> 8 on a 24-row term, inner 16.
    out = read_until(mfd, rb"\x1b\[1;16r", 6.0)
    check("chat expands the goulash area", b"\x1b[1;16r" in out, out[-300:])
    out = read_until(mfd, rb"goulash: ANS", 8.0)
    check("first answer in the transcript", b"goulash: ANS" in out, out[-300:])
    out = read_until(mfd, "↓ suggestion: echo p1-".encode(), 4.0)
    check("slot row under the chat panel shows the command",
          "↓ suggestion: echo p1-".encode() in out, out[-300:])
    time.sleep(0.3)
    os.write(mfd, b"and a follow-up\r")  # no '#' needed — chat has focus
    out = read_until(mfd, rb"ANS-CTX", 8.0)
    check("follow-up carries the running chat", b"ANS-CTX" in out, out[-300:])
    time.sleep(0.3)
    os.write(mfd, b"\x1b[B")  # Down: onto the slot row (newest selected)
    out = read_until(mfd, rb"1/2", 4.0)
    check("Down selects the newest slot", b"1/2" in out, out[-300:])
    os.write(mfd, b"\r")  # Enter: hand it up to the shell line
    out = read_until(mfd, rb"\x1b\[1;20r", 4.0)
    check("handoff returns focus (area restored)", b"\x1b[1;20r" in out, out[-300:])
    time.sleep(0.4)
    os.write(mfd, b"\r")
    out = read_until(mfd, rb"p2-48", 6.0)  # newest = second turn's command
    check("handed-off command ran in the shell", b"p2-48" in out, out[-300:])
    time.sleep(0.5)
    # Reopen and browse the slot stack IN chat: Down Down selects the
    # older turn's command; Enter hands that one off.
    os.write(mfd, b"## \r")
    time.sleep(0.8)
    os.write(mfd, b"\x1b[B")
    time.sleep(0.3)
    os.write(mfd, b"\x1bOB")  # SS3 form: what real zle sessions send
    out = read_until(mfd, rb"2/2", 4.0)
    check("Down browses older slots in chat (incl. SS3 arrows)",
          b"2/2" in out, out[-300:])
    os.write(mfd, b"\r")  # Enter on the selection: handoff the OLDER cmd
    time.sleep(0.6)
    os.write(mfd, b"\r")
    out = read_until(mfd, rb"p1-42", 6.0)
    check("Enter hands off the selected older command", b"p1-42" in out, out[-300:])
    time.sleep(0.5)
    os.write(mfd, b"## \r")  # reopen ...
    time.sleep(0.8)
    # goulash's own controls keep their sigils inside chat: "pin that
    # file" is a thing you say mid-conversation.
    os.write(mfd, b"@\r")
    out = read_until(mfd, rb"nothing pinned", 5.0)
    check("@ commands work from inside chat", b"nothing pinned" in out, out[-300:])
    time.sleep(0.4)
    os.write(mfd, b"\x1b")  # ... and Esc backs out
    time.sleep(0.4)
    os.write(mfd, b"echo bye-$((2*2))\r")
    out = read_until(mfd, rb"bye-4", 5.0)
    check("esc exits chat, shell keys flow again", b"bye-4" in out, out[-300:])
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)
    srv.shutdown()


def test_working_context():
    print("#@ pins files into the model's context (fake ollama):")
    if not shutil.which("zsh"):
        print("  [SKIP] zsh not installed")
        return
    import http.server
    import threading

    prompts = []

    class FakeOllama(http.server.BaseHTTPRequestHandler):
        def _send(self, obj):
            body = json.dumps(obj).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == "/api/tags":
                self._send({"models": [{"name": "pinmodel", "size": 1}]})
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            req = json.loads(self.rfile.read(n) or b"{}")
            if self.path == "/api/show":
                self._send({"capabilities": show_caps(req.get("model", ""))})
                return
            if "prompt" not in req:
                self._send({"done": True})
                return
            p = req["prompt"]
            prompts.append(p)
            if "Compress this reference document" in p:
                # The background ingest call. Prove it was handed the
                # OUTLINE (commands kept, prose already dropped), then
                # take a beat -- the pin must be useful in the meantime.
                assert "widgetctl sync --all" in p, "digest source lost the commands"
                time.sleep(1.2)
                self._send({"response": "widgetctl sync --all  # DIGESTED"})
                return
            if "change the pinned working context" in p:
                # The mediated form: resolve to a path, answer in verbs.
                if "OTHER.md" in p:      # got a candidate listing
                    self._send({"response": "pinning that\nPIN: ./OTHER.md"})
                else:
                    self._send({"response": "NOCANDIDATES"})
                return
            self._send({"response": "PASS"})

        def log_message(self, *a):
            pass

    # Threading: a slow digest must not wedge the whole fake server, or
    # the test would be measuring the stub rather than goulash.
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakeOllama)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    home = tempfile.mkdtemp(prefix="goulash-test-")
    work = tempfile.mkdtemp(prefix="goulash-work-")
    with open(os.path.join(work, "commandRef.md"), "w") as f:
        f.write("# widgetctl\n\nRun `widgetctl sync --all` to sync.\n")
    with open(os.path.join(work, "OTHER.md"), "w") as f:
        f.write("# other\n\nMENTIONS-OTHER\n")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write("[engine]\nprovider = \"ollama\"\n"
                f"host = \"http://127.0.0.1:{port}\"\nstream = false\n"
                "commentary = false\ncontext_files_max_chars = 900\n")
    proc, mfd = spawn(["zsh"], home=home)
    time.sleep(1.2)
    os.write(mfd, f"cd {work}\r".encode())
    read_until(mfd, rb"\$", 4.0)

    # Nothing pinned: say so, and don't spend a byte of prompt on it.
    os.write(mfd, b"#@\r")
    out = read_until(mfd, rb"nothing pinned", 5.0)
    check("bare #@ reports an empty context", b"nothing pinned" in out, out[-200:])

    # The deterministic form: no model involved at all.
    os.write(mfd, b"#@/path commandRef.md\r")
    out = read_until(mfd, rb"verbatim", 5.0)
    check("#@/path pins without the LLM",
          b"commandRef.md" in out and b"verbatim" in out, out[-300:])
    check("pin shows in the chrome", b"@commandRef.md" in out, out[-300:])

    # ...and the file's content is actually in front of the model.
    os.write(mfd, b"#how do i sync\r")
    read_until(mfd, rb"PASS", 8.0)
    asked = [p for p in prompts if "how do i sync" in p]
    check("pinned text reaches the prompt",
          asked and "widgetctl sync --all" in asked[-1], "")
    check("pin rides the stable prefix, above the session log",
          asked and asked[-1].index("Working context") < asked[-1].index("Session log"),
          "")

    # The mediated form: words in, PIN verb out, goulash does the read.
    os.write(mfd, b"#@ can you use the other markdown instead\r")
    # The chrome is the durable evidence: a second pin appeared, chosen
    # by the model from a listing goulash gave it.
    out = read_until(mfd, rb"@commandRef.md\+1", 8.0)
    check("#@ <words> resolves through the model",
          b"@commandRef.md+1" in out, out[-300:])
    os.write(mfd, b"#and now\r")
    read_until(mfd, rb"PASS", 8.0)
    asked = [p for p in prompts if "and now" in p]
    check("model-chosen pin reaches the prompt too",
          asked and "MENTIONS-OTHER" in asked[-1], "")

    # Changed on disk: marked, never silently reloaded.
    with open(os.path.join(work, "commandRef.md"), "a") as f:
        f.write("\nNEW LINE ADDED LATER\n")
    os.write(mfd, b"true\r")
    read_until(mfd, rb"\$", 4.0)
    time.sleep(0.4)
    out = read_until(mfd, rb"@commandRef.md\+1\*", 5.0)
    check("a changed pin is marked in the chrome",
          b"@commandRef.md+1*" in out, out[-300:])
    os.write(mfd, b"#still here\r")
    read_until(mfd, rb"PASS", 8.0)
    asked = [p for p in prompts if "still here" in p]
    check("stale text keeps serving until asked to re-cook",
          asked and "NEW LINE ADDED LATER" not in asked[-1], "")

    # A file too big for its share: outline immediately, digest behind it.
    with open(os.path.join(work, "big.md"), "w") as f:
        f.write("# widgetctl\n" + "Explanatory prose that carries nothing.\n" * 400
                + "Run `widgetctl sync --all` to sync.\n")
    os.write(mfd, b"#@/path big.md\r")
    # The meter is the durable evidence that a cook is running: a silent
    # multi-second ingest is exactly the "am I frozen?" failure.
    out = read_until(mfd, rb"@commandRef.md\+2\*? \d+%", 6.0)
    check("the chrome meters a running cook",
          re.search(rb"@commandRef.md\+2\*? \d+%", out) is not None, out[-300:])
    time.sleep(0.4)
    # Asked WHILE the digest is still cooking: the pin has to be useful
    # already, which is the whole reason the outline is computed first.
    os.write(mfd, b"#during the cook\r")
    read_until(mfd, rb"PASS", 12.0)
    asked = [p for p in prompts if "during the cook" in p]
    check("an over-budget pin is useful before its digest lands",
          asked and "[outline: prose omitted]" in asked[-1], "")
    check("and the outline kept the command",
          asked and "widgetctl sync --all" in asked[-1], "")

    # ...and when it lands the meter collapses back to a plain marker.
    time.sleep(2.0)
    os.write(mfd, b"true\r")
    out = read_until(mfd, rb"goulash", 5.0)
    check("the meter collapses when the cook finishes",
          re.search(rb"@commandRef.md\+2\*? \d+%", out) is None, out[-300:])
    os.write(mfd, b"#after the cook\r")
    read_until(mfd, rb"PASS", 8.0)
    asked = [p for p in prompts if "after the cook" in p]
    check("the digest is what reaches the prompt afterwards",
          asked and "DIGESTED" in asked[-1], "")
    check("and the dropped prose stays dropped",
          asked and "Explanatory prose" not in asked[-1], "")

    # Unset really unsets, and the block goes back to costing nothing.
    os.write(mfd, b"#@/unset\r")
    time.sleep(0.5)
    os.write(mfd, b"true\r")
    out = read_until(mfd, rb"goulash", 5.0)
    check("#@/unset clears the chrome marker",
          b"@commandRef.md" not in out, out[-300:])
    os.write(mfd, b"#gone now\r")
    read_until(mfd, rb"PASS", 8.0)
    asked = [p for p in prompts if "gone now" in p]
    check("an empty context costs zero prompt bytes",
          asked and "Working context" not in asked[-1], "")

    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)
    srv.shutdown()


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
            if self.path == "/api/show":
                self._send({"capabilities": show_caps(req.get("model", ""))})
                return
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
    os.write(mfd, b"#/memory status\r")
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
    # Bare #/memory opens the browser: filter, then arm-and-confirm.
    os.write(mfd, b"#/memory\r")
    out = read_until(mfd, "memory \u25b8".encode(), 5.0)
    check("#/memory opens the browser", "memory \u25b8".encode() in out, out[-300:])
    out = read_until(mfd, rb"\[1\] deploy", 3.0, out)
    check("slots listed in the browser", b"[1] deploy" in out, out[-300:])
    check("+ new row offered", "+ new memory".encode() in out, out[-300:])
    # Compose a memory from inside the browser: Enter on "+ new", type, Enter.
    os.write(mfd, b"\r")
    out = read_until(mfd, "esc cancel".encode(), 4.0)
    check("compose mode entered", "esc cancel".encode() in out, out[-300:])
    os.write(mfd, b"typed from the browser")
    time.sleep(0.4)
    os.write(mfd, b"\r")
    out = read_until(mfd, rb"remembered \[", 4.0)
    check("composed memory saved", b"remembered [" in out, out[-300:])
    time.sleep(0.3)
    os.write(mfd, b"deploy")          # type-to-filter
    time.sleep(0.4)
    os.write(mfd, b"\r")              # arm
    out = read_until(mfd, "again to forget".encode(), 4.0)
    check("first Enter arms, does not delete",
          "again to forget".encode() in out, out[-300:])
    os.write(mfd, b"\r")              # confirm
    out = read_until(mfd, rb"forgot \[1\]", 4.0)
    check("second Enter forgets the slot", b"forgot [1]" in out, out[-300:])
    time.sleep(0.3)
    os.write(mfd, b"\x1b")            # close the browser
    time.sleep(0.4)
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)
    srv.shutdown()

    mpath = os.path.join(home, "memory.toml")
    check("memory.toml durable", os.path.exists(mpath))
    mt = open(mpath).read() if os.path.exists(mpath) else ""
    check("browser-composed memory persisted",
          "typed from the browser" in mt, mt[-300:])
    check("model note persisted", "model saved this note" in mt, mt[-300:])
    check("deleted slot gone from toml", "TARGET=prod" not in mt, mt[-300:])
    check("enabled persists in toml", "enabled = true" in mt, mt[:120])
    logs = glob.glob(os.path.join(home, "history", "session-*.jsonl"))
    events = [json.loads(line) for line in open(logs[0])] if logs else []
    check("llm memory add recorded",
          any(e["ev"] == "memory" and e["op"] == "add" for e in events))


def painted_rows(chunk):
    """[(row, printable_width)] for each band row goulash painted."""
    out = []
    # Body runs to the next cursor move, the DECRC that ends goulash's
    # paint, or the visibility restore after it -- anything past that is
    # the shell's own output.
    for m in re.finditer(
            rb"\x1b\[(\d+);1H\x1b\[0m\x1b\[K"
            rb"((?:(?!\x1b\[\d+;\d*H|\x1b\[\?25|\x1b[78]).)*)",
            chunk, re.S):
        # Strip CSI sequences AND the two-byte DECSC/DECRC pair, which
        # print nothing but would otherwise count as two cells.
        body = re.sub(rb"\x1b\[[0-9;?]*[a-zA-Z]|\x1b[78]", b"", m.group(2))
        out.append((int(m.group(1)), len(body.decode("utf-8", "replace"))))
    return out


def test_resize_hygiene():
    print("resize leaves no stale band and never fills the last column:")
    proc, mfd = spawn(["bash", "--norc"])
    read_until(mfd, rb"\$")
    os.write(mfd, b"lls\r")   # typo -> suggestion chip rides the rule
    read_until(mfd, "suggestion: ls".encode())

    # Narrow the window a few steps, like a drag. A row that fills the
    # final column gets flagged soft-wrapped and reflows into a second
    # line on the next width change -- that was the spray.
    for cols in (78, 76, 75):
        set_winsize(mfd, 24, cols)
        os.killpg(os.getpgid(proc.pid), signal.SIGWINCH)
        # Paints are suspended until the drag settles, so let it settle
        # and drain, then measure only a FRESH paint at the new width.
        time.sleep(0.6)
        read_until(mfd, rb"$^", 0.3)
        out = read_until(mfd, rb"goulash", 2.5)
        wide = [(r, w) for r, w in painted_rows(out) if w > cols - 1]
        check(f"no row fills the last column at {cols} cols", not wide, f"{wide}")

    # Grow taller: the rows the band just left must be erased, or a
    # stale copy sits there until something scrolls it away.
    set_winsize(mfd, 30, 75)
    os.killpg(os.getpgid(proc.pid), signal.SIGWINCH)
    out = read_until(mfd, rb"\x1b\[27;1H", 3.0)
    rows = painted_rows(out)
    erased = {r for r, w in rows if w == 0}
    check("vacated band rows erased", {21, 22, 23, 24} <= erased,
          f"erased={sorted(erased)}")
    check("band repainted at the new bottom",
          {27, 28, 29, 30} <= {r for r, _ in rows}, f"rows={sorted(r for r, _ in rows)}")

    os.write(mfd, b"echo alive-$((7*6))\r")
    out = read_until(mfd, rb"alive-42", 5.0)
    check("shell still healthy after the resizes", b"alive-42" in out, out[-200:])
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)


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
        test_model_menu,
        test_model_capabilities,
        test_chat_mode,
        test_working_context,
        test_memory,
        test_resize_hygiene,
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
