#!/usr/bin/env python3
"""Long-run liability probe: does goulash cost anything while it sits there?

goulash is an overlay that stays resident for as long as a terminal is
open — days, plausibly. Two ways that becomes a liability:

  1. **Idle cost.** Bytes emitted, CPU burned, and wakeups taken when
     nothing is happening. On a laptop that is battery; over ssh it is
     bandwidth; in tmux it is a repaint storm.
  2. **Growth.** RSS, file descriptors, threads, and on-disk transcript
     climbing without bound across a long session.

Uses a fake engine (local HTTP, instant replies) so the probe is pure CPU
and never competes with a GPU sweep for the machine.

    python3 tests/longrun.py            # ~6 min
    python3 tests/longrun.py --turns 400 --idle 120
"""
import argparse, json, os, pty, re, select, shutil, subprocess, sys
import tempfile, termios, fcntl, struct, threading, time
import http.server

BIN = os.path.join(os.path.dirname(__file__), "..", "target", "debug", "goulash")
ROWS, COLS = 40, 120


# ----------------------------------------------------------------- engine
class FakeEngine(http.server.BaseHTTPRequestHandler):
    """Answers instantly so the probe measures goulash, not inference."""

    def _send(self, obj):
        body = json.dumps(obj).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/api/tags":
            self._send({"models": [{"name": "fake:1b", "size": 1_000_000_000}]})
        elif self.path == "/api/ps":
            self._send({"models": []})
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        req = json.loads(self.rfile.read(n) or b"{}")
        if "prompt" not in req:
            self._send({"done": True})
            return
        ans = "A short answer for the band.\nCMD: echo probe-reply"
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


# ------------------------------------------------------------------ probe
def sample(pid, home):
    """One point-in-time reading of everything that could grow."""
    out = {}
    # macOS ps has no thcount/nlwp; a bad keyword makes it drop the field
    # silently and shift every column after it, which is how an earlier
    # version of this probe reported 0 threads and 0 CPU.
    try:
        ps = subprocess.run(["ps", "-o", "rss=,time=", "-p", str(pid)],
                            capture_output=True, text=True, timeout=10).stdout.split()
        if len(ps) >= 2:
            out["rss_mb"] = int(ps[0]) / 1024
            mm, _, ss = ps[1].rpartition(":")
            out["cpu_s"] = float(ss) + (float(mm) * 60 if mm else 0)
    except Exception:
        pass
    try:
        # -M lists one line per thread, plus a header.
        out["threads"] = max(0, len(subprocess.run(
            ["ps", "-M", "-p", str(pid)], capture_output=True, text=True,
            timeout=10).stdout.splitlines()) - 1)
    except Exception:
        pass
    try:
        lsof = subprocess.run(["lsof", "-p", str(pid)], capture_output=True,
                              text=True, timeout=25).stdout.splitlines()[1:]
        out["fds"] = len(lsof)
        # Graphics/IOKit handles show up as device entries; on an overlay
        # with no GPU use these should stay flat at zero.
        out["gpu_handles"] = sum(1 for l in lsof
                                 if "IOAccelerator" in l or "AGX" in l or "Metal" in l)
    except Exception:
        pass
    try:
        hist = os.path.join(home, "history")
        out["transcript_mb"] = sum(
            os.path.getsize(os.path.join(hist, f)) for f in os.listdir(hist)
        ) / 1e6 if os.path.isdir(hist) else 0.0
    except Exception:
        pass
    return out


class Reader(threading.Thread):
    """Drains the pty master and counts every byte goulash emits."""

    def __init__(self, fd):
        super().__init__(daemon=True)
        self.fd, self.total, self.window, self.tail = fd, 0, 0, b""
        self.stop = False

    def run(self):
        while not self.stop:
            r, _, _ = select.select([self.fd], [], [], 0.2)
            if self.fd not in r:
                continue
            try:
                d = os.read(self.fd, 65536)
            except OSError:
                break
            if not d:
                break
            self.total += len(d)
            self.window += len(d)
            self.tail = (self.tail + d)[-4096:]

    def take(self):
        w, self.window = self.window, 0
        return w

    def classify(self):
        """What ARE those idle bytes? Cursor moves and repaints look very
        different from real output, and the distinction decides whether
        idle emission is a bug or just the status row ticking."""
        t = self.tail
        return {
            "cursor_pos": len(re.findall(rb"\x1b\[\d+;\d+H", t)),
            "erase": len(re.findall(rb"\x1b\[[0-2]?K", t)),
            "sgr": len(re.findall(rb"\x1b\[[0-9;]*m", t)),
            "save_restore": len(re.findall(rb"\x1b[78]", t)),
            "scroll_region": len(re.findall(rb"\x1b\[\d*;?\d*r", t)),
            "printable": sum(1 for c in t if 32 <= c < 127),
        }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--turns", type=int, default=200)
    ap.add_argument("--idle", type=int, default=60)
    args = ap.parse_args()

    if not shutil.which("zsh"):
        sys.exit("needs zsh")

    srv = http.server.HTTPServer(("127.0.0.1", 0), FakeEngine)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    home = tempfile.mkdtemp(prefix="goulash-longrun-")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write(f'[engine]\nprovider = "ollama"\n'
                f'host = "http://127.0.0.1:{port}"\nmodel = "fake:1b"\n')

    mfd, sfd = pty.openpty()
    fcntl.ioctl(mfd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    env = dict(os.environ, GOULASH_HOME=home, HOME=home, TERM="xterm-256color")
    proc = subprocess.Popen([BIN, "zsh"], stdin=sfd, stdout=sfd, stderr=sfd,
                            start_new_session=True, env=env, close_fds=True)
    os.close(sfd)
    rd = Reader(mfd)
    rd.start()
    time.sleep(3)

    print(f"pid {proc.pid}  home {home}\n")
    rows = []

    def measure(phase, note=""):
        s = sample(proc.pid, home)
        s["phase"], s["note"] = phase, note
        s["emitted_mb"] = rd.total / 1e6
        rows.append(s)
        print(f"  {phase:<22} rss={s.get('rss_mb', 0):7.1f}MB "
              f"fds={s.get('fds', 0):>4} thr={s.get('threads', 0):>3} "
              f"cpu={s.get('cpu_s', 0):7.2f}s gpu={s.get('gpu_handles', 0):>3} "
              f"tx={s.get('transcript_mb', 0):6.2f}MB emit={s['emitted_mb']:7.2f}MB {note}")
        return s

    # -- phase 1: idle. Nothing typed. Anything emitted here is unprompted.
    print(f"PHASE 1  idle {args.idle}s (no input at all)")
    base = measure("idle-start")
    rd.take()
    t0 = time.time()
    idle_marks = []
    while time.time() - t0 < args.idle:
        time.sleep(10)
        idle_marks.append(rd.take())
    idle = measure("idle-end", f"emitted {sum(idle_marks)}B while idle")

    # -- phase 2: sustained work
    print(f"\nPHASE 2  {args.turns} turns (commands, asks, chat, model switches)")
    for i in range(args.turns):
        os.write(mfd, f"echo turn-{i}\r".encode())
        time.sleep(0.28)
        if i % 5 == 4:
            os.write(mfd, f"# what happened in turn {i}\r".encode())
            time.sleep(0.45)
        if i % 25 == 24:
            os.write(mfd, b"##\r"); time.sleep(0.3)
            os.write(mfd, f"chat question {i}\r".encode()); time.sleep(0.45)
            os.write(mfd, b"\x1b"); time.sleep(0.25)
        if i % 40 == 39:
            os.write(mfd, b"#/model fake:1b\r"); time.sleep(0.4)
        if i and i % 50 == 0:
            measure(f"turn-{i}")
    work = measure("work-end")

    # -- phase 3: idle again. Does it settle, or keep costing?
    print(f"\nPHASE 3  idle {args.idle}s again")
    rd.take()
    t0 = time.time()
    post_marks = []
    while time.time() - t0 < args.idle:
        time.sleep(10)
        post_marks.append(rd.take())
    post = measure("post-idle", f"emitted {sum(post_marks)}B while idle")

    os.write(mfd, b"exit\r")
    time.sleep(2)
    rd.stop = True
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
    srv.shutdown()

    # ------------------------------------------------------------- verdict
    print("\n" + "=" * 72)
    grow = lambda k: work.get(k, 0) - base.get(k, 0)
    print(f"across {args.turns} turns:")
    print(f"  RSS            {base.get('rss_mb',0):7.1f} -> {work.get('rss_mb',0):7.1f} MB "
          f"({grow('rss_mb'):+.1f}, {grow('rss_mb')/max(args.turns,1)*1000:+.1f} KB/turn)")
    print(f"  file handles   {base.get('fds',0):7} -> {work.get('fds',0):7} ({grow('fds'):+})")
    print(f"  threads        {base.get('threads',0):7} -> {work.get('threads',0):7} ({grow('threads'):+})")
    print(f"  gpu handles    {base.get('gpu_handles',0):7} -> {work.get('gpu_handles',0):7}")
    print(f"  transcript     {base.get('transcript_mb',0):7.2f} -> {work.get('transcript_mb',0):7.2f} MB "
          f"({grow('transcript_mb')/max(args.turns,1)*1000:+.1f} KB/turn)")

    idle_b = sum(idle_marks)
    post_b = sum(post_marks)
    idle_cpu = idle.get("cpu_s", 0) - base.get("cpu_s", 0)
    post_cpu = post.get("cpu_s", 0) - work.get("cpu_s", 0)
    print(f"\nidle behaviour ({args.idle}s windows):")
    print(f"  bytes emitted while idle, fresh:  {idle_b:>8}  ({idle_b/args.idle:.1f} B/s)")
    print(f"  bytes emitted while idle, after:  {post_b:>8}  ({post_b/args.idle:.1f} B/s)")
    print(f"  CPU while idle, fresh:            {idle_cpu:>8.2f}s")
    print(f"  CPU while idle, after work:       {post_cpu:>8.2f}s")
    cls = rd.classify()
    print(f"  last 4KB emitted, by kind: " +
          ", ".join(f"{k}={v}" for k, v in cls.items() if v))

    print("\nverdict:")
    ok = True
    if grow("fds") > 2:
        print(f"  FAIL  file handles grew {grow('fds')}"); ok = False
    if grow("threads") > 0:
        print(f"  FAIL  threads grew {grow('threads')}"); ok = False
    if grow("rss_mb") > 20:
        print(f"  FAIL  RSS grew {grow('rss_mb'):.1f} MB"); ok = False
    if post_b > 5000:
        print(f"  WARN  emits {post_b}B over {args.idle}s while idle — repaint churn"); ok = False
    if post_cpu > 2.0:
        print(f"  WARN  burns {post_cpu:.2f}s CPU over {args.idle}s idle"); ok = False
    if ok:
        print("  clean: no handle/thread growth, bounded RSS, quiet when idle")
    json.dump(rows, open("/tmp/longrun.json", "w"), indent=1)
    print(f"\nsamples -> /tmp/longrun.json   home kept at {home}")


if __name__ == "__main__":
    main()
