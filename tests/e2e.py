#!/usr/bin/env python3
"""End-to-end tests for the goulash M0 PTY wrapper.

Drives the goulash binary under a real PTY (stdlib only, no pexpect) and
checks: shrunken winsize, byte passthrough, status row, scroll-region
assertion, resize propagation, and exit-code propagation.
"""
import fcntl
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time

BIN = os.path.join(os.path.dirname(__file__), "..", "target", "debug", "goulash")
ROWS, COLS = 24, 80
failures = []


def set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def spawn(argv, rows=ROWS, cols=COLS):
    mfd, sfd = pty.openpty()
    set_winsize(mfd, rows, cols)
    env = dict(os.environ, GOULASH_HOME="/nonexistent", TERM="xterm-256color")
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
    inner = ROWS - 1
    check("scroll region asserted", f"\x1b[1;{inner}r".encode() in out,
          "no DECSTBM 1..%d in %r" % (inner, out[-200:]))
    check("status row drawn", b"goulash" in out and b"bash" in out)

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
    check("resize propagates (rows-1)", m and m.group(1) == b"29", f"got {m and m.group(1)}")
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


def test_non_tty():
    print("refuses to run without a tty:")
    r = subprocess.run([BIN, "true"], capture_output=True)
    check("exit code 2", r.returncode == 2, f"got {r.returncode}")
    check("clear error message", b"must be a terminal" in r.stderr)


def main():
    for t in (test_basic, test_exit_code, test_fullscreen_clear, test_non_tty):
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
