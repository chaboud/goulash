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


def dismiss_and_exit(mfd):
    """Esc to close whatever goulash has open, then leave.

    The bare CR in the middle is load-bearing. A lone Esc that reaches
    zsh leaves ZLE holding a meta prefix, and it does NOT time out --
    measured at 1.2s -- so the next key is swallowed and `exit` arrives
    as `xit`: command not found, shell still up, session hangs until the
    harness gives up. A CR dispatches the pending state and hands back a
    clean prompt.

    None of this is goulash behaviour; a bare Esc at a zsh prompt does
    the same thing unwrapped. It is the harness putting the keyboard
    back before it asks the shell to quit."""
    os.write(mfd, b"\x1b")
    time.sleep(0.3)
    os.write(mfd, b"\r")
    time.sleep(0.3)
    os.write(mfd, b"exit\r")


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


ZSH_PLUGIN = r"""
# A plausible plugin, installed the way a real one would be: from the
# user's .zshrc, i.e. BEFORE goulash's adapter loads. Everything here
# has to survive the adapter (wiki: shell-integration, fidelity audit).
myaccept() { print -r -- "PLUG-ACCEPT" >> "$HOME/plug.log"; zle .accept-line }
mypaste()  { print -r -- "PLUG-PASTE"  >> "$HOME/plug.log"; zle .bracketed-paste }
mydown()   { print -r -- "PLUG-DOWN"   >> "$HOME/plug.log"; zle .down-line-or-history }
myup()     { print -r -- "PLUG-UP"     >> "$HOME/plug.log"; zle .up-line-or-history }
zle -N accept-line myaccept
zle -N bracketed-paste mypaste
zle -N mydown; zle -N myup
bindkey '^[[B' mydown; bindkey '^[OB' mydown
bindkey '^[[A' myup;   bindkey '^[OA' myup
autoload -Uz add-zsh-hook
myprecmd() { print -r -- "PLUG-PRECMD:$?" >> "$HOME/plug.log"; command true }
add-zsh-hook precmd myprecmd
"""

BASH_PLUGIN = r"""
__plug_debug() { echo "PLUG-DEBUG" >> "$HOME/plug.log"; }
trap '__plug_debug' DEBUG
__plug_prompt() { echo "PLUG-PROMPT:$?" >> "$HOME/plug.log"; }
PROMPT_COMMAND='__plug_prompt'
"""


def test_adapter_fidelity():
    """The adapter must change NOTHING about the shell except the arrows
    and the async `#` interception. Two things get checked here that no
    amount of reading catches: that a plugin bound to the same widgets
    still runs, and that `#` no longer leaks into ordinary command
    lexing — goulash used to `setopt interactivecomments`, which changed
    what `echo a # b` means and broke Tab completion on every `#` line."""
    print("adapter fidelity (plugins survive, shell semantics untouched):")
    # `echo x # y` is the tell. Each shell has its own answer and the
    # adapter must not change it: zsh leaves interactive_comments unset,
    # so the `#` is just another argument; bash has it on by default, so
    # the tail is a comment. goulash used to `setopt interactivecomments`
    # and make zsh behave like bash — for every command the user typed.
    for shell, plugin, rc, expect in (("zsh", ZSH_PLUGIN, ".zshrc", b"mark-4 # trailing"),
                                      ("bash", BASH_PLUGIN, ".bashrc", b"mark-4")):
        if not shutil.which(shell):
            print(f"  [SKIP] {shell} not installed")
            continue
        home = tempfile.mkdtemp(prefix="goulash-test-")
        with open(os.path.join(home, rc), "w") as f:
            f.write(plugin)
        log = os.path.join(home, "plug.log")
        proc, mfd = spawn([shell], home=home)
        time.sleep(1.5)
        os.write(mfd, b"echo mark-$((2*2)) # trailing\r")
        out = read_until(mfd, rb"mark-4", 6.0)
        tail = out[out.rfind(b"mark-4"):][:40]
        check(f"{shell}: '#' mid-command lexed the shell's own way",
              tail.startswith(expect), repr(tail))
        os.write(mfd, b"false\r")
        time.sleep(0.6)
        os.write(mfd, b"# an aside with !! in it\r")
        if shell == "bash":
            # bash has no accept-line to hook, so an aside is recovered
            # from history at the NEXT prompt — one turn later by design.
            time.sleep(0.6)
            os.write(mfd, b"\r")
        out = read_until(mfd, rb"no engine", 8.0)
        check(f"{shell}: aside still intercepted", b"no engine" in out, out[-300:])
        check(f"{shell}: aside not history-expanded", b"echo mark" not in out[-200:],
              out[-200:])
        os.write(mfd, b"exit\r")
        drain_exit(proc, mfd)
        seen = open(log).read() if os.path.exists(log) else ""
        if shell == "zsh":
            check("zsh: plugin accept-line widget still runs",
                  "PLUG-ACCEPT" in seen, seen[-200:])
            check("zsh: plugin precmd sees the true exit code",
                  "PLUG-PRECMD:1" in seen, seen[-300:])
        else:
            check("bash: plugin DEBUG trap still fires",
                  "PLUG-DEBUG" in seen, seen[-200:])
            check("bash: plugin PROMPT_COMMAND sees the true exit code",
                  "PLUG-PROMPT:1" in seen, seen[-300:])
        logs = glob.glob(os.path.join(home, "history", "session-*.jsonl"))
        events = [json.loads(line) for line in open(logs[0])] if logs else []
        check(f"{shell}: aside reached goulash verbatim",
              any(e["ev"] == "aside" and e["text"] == "# an aside with !! in it"
                  for e in events),
              str([e.get("text") for e in events if e["ev"] == "aside"]))


ZSH_RC_FILES = (".zshenv", ".zprofile", ".zshrc", ".zlogin")
BASH_RC_FILES = (".bashrc", ".bash_profile", ".bash_login", ".profile")


def rc_log_home():
    """A home whose every startup file appends its own name — and, for
    zsh, what $ZDOTDIR looked like while it ran — to one log."""
    home = tempfile.mkdtemp(prefix="goulash-test-")
    for name in ZSH_RC_FILES:
        with open(os.path.join(home, name), "w") as f:
            f.write(f'print -r -- "{name} zdot=[${{ZDOTDIR-UNSET}}]" >> "$HOME/rc.log"\n')
    for name in BASH_RC_FILES:
        with open(os.path.join(home, name), "w") as f:
            f.write(f'echo "{name}" >> "$HOME/rc.log"\n')
    return home


def bare_startup(shell, args, home):
    """What you get by typing the shell's name at a prompt — no goulash
    anywhere. This is the reference the overlay has to match."""
    open(os.path.join(home, "rc.log"), "w").close()
    pid, fd = pty.fork()
    if pid == 0:
        os.environ.update(HOME=home, TERM="xterm-256color")
        os.environ.pop("ZDOTDIR", None)
        os.execv(shell, [os.path.basename(shell)] + args)
    set_winsize(fd, ROWS, COLS)
    time.sleep(1.2)
    os.write(fd, b"exit\n")
    time.sleep(0.5)
    try:
        os.close(fd)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    return open(os.path.join(home, "rc.log")).read().strip().splitlines()


def goulash_startup(shell, args, home):
    open(os.path.join(home, "rc.log"), "w").close()
    proc, mfd = spawn([shell] + args, home=home)
    time.sleep(2.0)
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)
    return open(os.path.join(home, "rc.log")).read().strip().splitlines()


def test_rc_loading():
    """Differential, not hardcoded: run the shell bare, run it under
    goulash, demand the same startup files in the same order with
    $ZDOTDIR reading the same way. Anything else is goulash changing how
    the machine starts a shell.

    Three real bugs this pins down: the zsh stubs skipped .zlogin and
    .zlogout entirely, they left ZDOTDIR pointing at goulash's own dir
    while the user's .zshenv ran, and they set it to $HOME when the user
    had never set it at all. And bash login shells got NO integration —
    bash ignores --rcfile with -l, so neither the adapter nor the user's
    own profile loaded."""
    print("startup files identical to the bare shell:")
    for shell in ("zsh", "bash"):
        path = shutil.which(shell)
        if not path:
            print(f"  [SKIP] {shell} not installed")
            continue
        for args in ([], ["-l"]):
            label = f"{shell} {' '.join(args) or '(no flags)'}"
            home = rc_log_home()
            want = bare_startup(path, args, home)
            got = goulash_startup(path, args, home)
            check(f"{label}: same startup sequence", want == got,
                  f"bare={want} goulash={got}")
            check(f"{label}: bare run is not vacuous", bool(want), str(want))
    if shutil.which("bash"):
        # The login path is emulated (bash cannot be told where to find
        # a profile), so prove the adapter actually arrives.
        home = rc_log_home()
        proc, mfd = spawn(["bash", "-l"], home=home)
        time.sleep(1.5)
        os.write(mfd, b'echo ADAPT=$__goulash_loaded\r')
        out = read_until(mfd, rb"ADAPT=1", 6.0)
        check("login bash gets the adapter", b"ADAPT=1" in out, out[-300:])
        os.write(mfd, b"exit\r")
        drain_exit(proc, mfd)
    else:
        print("  [SKIP] bash not installed")


TAB_RC = r"""
autoload -Uz compinit; compinit -u -d "$HOME/.zcd"
__dump() { print -r -- "[$BUFFER]" >> "$HOME/buf.log"; zle send-break }
zle -N __dump
bindkey '^X' __dump
"""


def tab_buffer(keys, home, via_goulash):
    """Type `keys` at a real zsh, then dump $BUFFER. The only honest way
    to ask what Tab did — the screen is a rendering, the buffer is the
    fact."""
    log = os.path.join(home, "buf.log")
    open(log, "w").close()
    if via_goulash:
        proc, mfd = spawn(["zsh"], home=home)
        time.sleep(2.0)
        # goulash's child inherits the RUNNER's cwd; the bare shell below
        # starts in $HOME. Completion is cwd-relative, so pin both.
        os.write(mfd, b'cd "$HOME"\r')
        time.sleep(1.0)
        os.write(mfd, keys)
        time.sleep(1.2)
        os.write(mfd, b"\x18")          # ^X: dump and break
        time.sleep(0.8)
        os.write(mfd, b"\x03")
        time.sleep(0.3)
        os.write(mfd, b"exit\r")
        drain_exit(proc, mfd)
    else:
        pid, fd = pty.fork()
        if pid == 0:
            os.environ.update(HOME=home, TERM="xterm-256color")
            os.environ.pop("ZDOTDIR", None)
            os.chdir(home)
            os.execv(shutil.which("zsh"), ["zsh", "-i"])
        set_winsize(fd, ROWS, COLS)
        time.sleep(1.2)
        os.write(fd, keys)
        time.sleep(1.0)
        os.write(fd, b"\x18")
        time.sleep(0.6)
        os.write(fd, b"\x03exit\n")
        time.sleep(0.4)
        try:
            os.close(fd)
        except OSError:
            pass
        try:
            os.waitpid(pid, 0)
        except ChildProcessError:
            pass
    return open(log).read().strip()


def test_tab_completion():
    """The field bug, as reported: typing `# mini` and hitting Tab gave
    an unfiltered dump of the whole home directory with the buffer
    untouched, where bare zsh completed it to `# miniconda3/`. Cause was
    `setopt interactivecomments` — a commented line hands the completion
    system CURRENT=0 and an empty PREFIX, so there is no word to filter
    by and none to replace, and a second Tab splices the match in at the
    cursor (`#@ Fa` -> `#@ FaFarmAnimals.md`).

    Differential again: bare zsh is the reference."""
    print("Tab completion on `#` lines matches bare zsh:")
    if not shutil.which("zsh"):
        print("  [SKIP] zsh not installed")
        return
    home = tempfile.mkdtemp(prefix="goulash-test-")
    os.mkdir(os.path.join(home, "miniconda3"))
    for name in ("minutes.txt", "README.md"):
        open(os.path.join(home, name), "w").close()
    with open(os.path.join(home, ".zshrc"), "w") as f:
        f.write(TAB_RC)
    for label, keys in (("# mini", b"# mini\t"),
                        ("#@ mini", b"#@ mini\t"),
                        ("# mini (twice)", b"# mini\t\t"),
                        ("ls mini", b"ls mini\t")):
        want = tab_buffer(keys, home, via_goulash=False)
        got = tab_buffer(keys, home, via_goulash=True)
        check(f"Tab after '{label}' behaves as bare zsh", want == got,
              f"bare={want!r} goulash={got!r}")
        check(f"Tab after '{label}' actually completed",
              "miniconda3" in want, repr(want))


def test_engine_openai():
    """LM Studio, llama.cpp's server and vLLM all speak the OpenAI `/v1`
    wire, so one provider reaches all three. Proxied here by a fake that
    is deliberately strict about the differences from ollama — it 400s
    on ollama-only fields rather than ignoring them, which is what a real
    strict server does and what would otherwise show up as a blank bar.

    The load-bearing assertion is that the PROMPT is unchanged: goulash
    targets /v1/completions, not /v1/chat/completions, precisely so the
    stable prefix the KV cache depends on survives the move."""
    print("OpenAI-compatible provider (fake LM Studio):")
    if not shutil.which("zsh"):
        print("  [SKIP] zsh not installed")
        return
    import http.server
    import threading

    seen = {"auth": None, "prompt": "", "path": None, "streamed": False}

    class FakeOpenAI(http.server.BaseHTTPRequestHandler):
        def _send(self, obj, code=200):
            body = json.dumps(obj).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == "/v1/models":
                self._send({"object": "list", "data": [
                    {"id": "lmstudio-model", "object": "model"},
                    {"id": "second-model", "object": "model"},
                ]})
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            req = json.loads(self.rfile.read(n) or b"{}")
            seen["path"] = self.path
            seen["auth"] = self.headers.get("Authorization")
            # A strict OpenAI server rejects unknown fields. If goulash
            # leaks ollama's dialect the failure has to be loud here,
            # not a mysteriously empty answer in the band.
            for bad in ("options", "keep_alive", "think", "num_ctx"):
                if bad in req:
                    self._send({"error": f"unknown field {bad}"}, code=400)
                    return
            if self.path != "/v1/completions":
                self._send({"error": "wrong endpoint"}, code=404)
                return
            prompt = req.get("prompt", "")
            if prompt:
                seen["prompt"] = prompt
            if not req.get("max_tokens"):
                self._send({"error": "no max_tokens"}, code=400)
                return
            ans = f"OAI-{req.get('model')}\nCMD: echo from-openai"
            if req.get("stream"):
                seen["streamed"] = True
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                # Split mid-answer so an accumulating reader is actually
                # exercised, and include a comment line + blank lines,
                # which real SSE is full of.
                for piece in (ans[:6], ans[6:]):
                    self.wfile.write(
                        b": keep-alive\n\n" + b"data: " + json.dumps(
                            {"choices": [{"text": piece, "finish_reason": None}]}
                        ).encode() + b"\n\n")
                    self.wfile.flush()
                self.wfile.write(b"data: [DONE]\n\n")
                self.wfile.flush()
            else:
                self._send({"choices": [{"text": ans, "finish_reason": "stop"}]})

        def log_message(self, *a):
            pass

    srv = http.server.HTTPServer(("127.0.0.1", 0), FakeOpenAI)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    home = tempfile.mkdtemp(prefix="goulash-test-")
    os.environ["GOULASH_TEST_KEY"] = "sk-test-123"
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write('[engine]\nprovider = "lmstudio"\n'
                f'openai_host = "http://127.0.0.1:{port}"\n'
                'api_key_env = "GOULASH_TEST_KEY"\n')
    proc, mfd = spawn(["zsh"], home=home)
    time.sleep(1.5)
    os.write(mfd, b"# what is the answer\r")
    out = read_until(mfd, rb"OAI-lmstudio-model", 8.0)
    check("model picked from /v1/models and answered",
          b"OAI-lmstudio-model" in out, out[-300:])
    check("targets /v1/completions, not chat-completions",
          seen["path"] == "/v1/completions", str(seen["path"]))
    check("SSE stream decoded and accumulated", seen["streamed"], str(seen))
    check("bearer token attached from api_key_env",
          seen["auth"] == "Bearer sk-test-123", str(seen["auth"]))
    # The reason for /v1/completions in the first place.
    check("the stable prefix survives the wire change",
          seen["prompt"].startswith("You are goulash")
          and "Session log" in seen["prompt"],
          repr(seen["prompt"][:80]))
    os.write(mfd, b"#/status\r")
    out = read_until(mfd, rb"blocks this session", 5.0)
    check("#/status names the openai provider", b"lmstudio-model" in out, out[-300:])
    dismiss_and_exit(mfd)
    drain_exit(proc, mfd)
    srv.shutdown()


def test_per_lane_providers():
    """Fast and slow bound to DIFFERENT servers — the case the two-lane
    design was always for: a small local model answering immediately, a
    bigger one elsewhere researching the same turn.

    Two fake servers, one ollama and one OpenAI-compatible, and the test
    asserts each lane's request landed on its own. Getting this wrong is
    invisible in normal use (both lanes answer, just from one box), so
    it needs a server that can say "nobody asked me"."""
    print("per-lane providers (fast and slow on different servers):")
    if not shutil.which("zsh"):
        print("  [SKIP] zsh not installed")
        return
    import http.server
    import threading

    hits = {"fast": 0, "slow": 0, "slow_model": None, "fast_model": None}

    class FastOllama(http.server.BaseHTTPRequestHandler):
        def _send(self, obj):
            body = json.dumps(obj).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == "/api/tags":
                self._send({"models": [{"name": "fastmodel", "size": 1_000_000}]})
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            req = json.loads(self.rfile.read(n) or b"{}")
            if self.path == "/api/show":
                self._send({"capabilities": ["completion"]})
                return
            if "prompt" not in req:
                self._send({"done": True})
                return
            hits["fast"] += 1
            hits["fast_model"] = req.get("model")
            self._send({"response": "FAST-SAYS\nCMD: echo fast"})

        def log_message(self, *a):
            pass

    class SlowOpenAI(http.server.BaseHTTPRequestHandler):
        def _send(self, obj):
            body = json.dumps(obj).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == "/v1/models":
                self._send({"data": [{"id": "slowmodel"}]})
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length", 0))
            req = json.loads(self.rfile.read(n) or b"{}")
            if not req.get("prompt"):
                self._send({"choices": [{"text": ""}]})
                return
            hits["slow"] += 1
            hits["slow_model"] = req.get("model")
            self._send({"choices": [{"text":
                        "SLOW-SAYS\nCMD: echo slow\nREASON: because"}]})

        def log_message(self, *a):
            pass

    fast_srv = http.server.HTTPServer(("127.0.0.1", 0), FastOllama)
    slow_srv = http.server.HTTPServer(("127.0.0.1", 0), SlowOpenAI)
    fport, sport = fast_srv.server_address[1], slow_srv.server_address[1]
    for srv in (fast_srv, slow_srv):
        threading.Thread(target=srv.serve_forever, daemon=True).start()

    home = tempfile.mkdtemp(prefix="goulash-test-")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write(
            '[engine]\n'
            'provider = "ollama"\n'
            f'host = "http://127.0.0.1:{fport}"\n'
            'slow = "manual"\n'
            '[engine.slow_lane]\n'
            'provider = "lmstudio"\n'
            f'openai_host = "http://127.0.0.1:{sport}"\n'
        )
    proc, mfd = spawn(["zsh"], home=home)
    time.sleep(1.5)
    os.write(mfd, b"#? which way\r")
    out = read_until(mfd, rb"FAST-SAYS", 8.0)
    check("fast lane answered from its own server", b"FAST-SAYS" in out, out[-300:])
    # Research is async and lands on the turn it came from.
    deadline = time.time() + 10
    while time.time() < deadline and hits["slow"] == 0:
        read_until(mfd, rb"__never__", 1.0)
    check("slow lane reached the OTHER server", hits["slow"] > 0, str(hits))
    check("each lane bound its own server's model",
          hits["fast_model"] == "fastmodel" and hits["slow_model"] == "slowmodel",
          str(hits))
    os.write(mfd, b"#/status\r")
    out = read_until(mfd, rb"slowmodel@openai", 6.0)
    check("#/status names both lanes",
          b"fastmodel@ollama" in out and b"slowmodel@openai" in out, out[-400:])
    # Both fakes are on loopback, so `trusted = "auto"` trusts both and
    # the warning stays silent. A marker beside every lane would train
    # people to stop reading it.
    check("no untrusted marker when both lanes are local",
          b"untrusted" not in out, out[-400:])
    dismiss_and_exit(mfd)
    drain_exit(proc, mfd)

    # Trust is STATED, so stating it has to win over what auto would
    # infer -- here, refusing to trust a lane that is plainly on this
    # machine. The inverse (trusting a box on your own LAN that auto
    # cannot know about) is the same switch the other way.
    home2 = tempfile.mkdtemp(prefix="goulash-test-")
    with open(os.path.join(home2, "config.toml"), "w") as f:
        f.write(
            '[engine]\n'
            'provider = "ollama"\n'
            f'host = "http://127.0.0.1:{fport}"\n'
            '[engine.slow_lane]\n'
            'provider = "lmstudio"\n'
            f'openai_host = "http://127.0.0.1:{sport}"\n'
            'trusted = "no"\n'
        )
    proc, mfd = spawn(["zsh"], home=home2)
    time.sleep(1.5)
    os.write(mfd, b"#/status\r")
    out = read_until(mfd, rb"untrusted", 6.0)
    check("stated distrust overrides what auto would infer",
          b"untrusted: slow" in out, out[-400:])
    dismiss_and_exit(mfd)
    drain_exit(proc, mfd)
    for srv in (fast_srv, slow_srv):
        srv.shutdown()


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
    # Field bug: command_first was the one setting with no session-side
    # variable, so Enter told the engine and rewrote the config while the
    # row itself was hardcoded to "on" and the session's own mid-stream
    # vending never changed. It looked exactly like a dead key.
    for _ in range(5):
        os.write(mfd, b"\x1b[B")
        time.sleep(0.15)
    out = read_until(mfd, rb"command_first: on", 4.0)
    check("command_first reachable in the list", b"command_first: on" in out,
          out[-300:])
    os.write(mfd, b"\r")
    out = read_until(mfd, rb"command_first: off", 4.0)
    check("command_first ROW toggles, not just the notice",
          out.count(b"command_first: off") >= 2, out[-400:])
    os.write(mfd, b"\r")            # back on, so later checks stand
    read_until(mfd, rb"command_first: on", 4.0)
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
    # Same paint, one accumulator: the answer row and the slot row below
    # it are written together, so a second read starting empty can miss
    # the whole thing.
    out = read_until(mfd, "↓ suggestion: echo p1-".encode(), 4.0, out)
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
    # file" is a thing you say mid-conversation. Bare @ opens the pin
    # browser over the chat panel...
    os.write(mfd, b"@\r")
    out = read_until(mfd, "@ pinned ▸".encode(), 5.0)
    check("@ opens the pin browser from inside chat",
          "@ pinned ▸".encode() in out, out[-300:])
    # ...and Esc drops the menu back to the chat it came from, not out
    # to the shell.
    os.write(mfd, b"\x1b")
    out = read_until(mfd, "## chat".encode(), 4.0)
    check("esc returns to chat, not to the shell",
          "## chat".encode() in out, out[-300:])
    time.sleep(0.4)
    os.write(mfd, b"\x1b")  # ... and Esc backs out
    time.sleep(0.4)
    os.write(mfd, b"echo bye-$((2*2))\r")
    out = read_until(mfd, rb"bye-4", 5.0)
    check("esc exits chat, shell keys flow again", b"bye-4" in out, out[-300:])
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)
    srv.shutdown()


def test_slow_lane():
    print("#? researches; fast keeps the microphone (fake ollama):")
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
                self._send({"models": [{"name": "slowmodel", "size": 1}]})
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
            if "Take your time and get this RIGHT" in p:
                # The slow lane. Deliberately slower than the fast reply,
                # so ordering is a fact rather than a race.
                time.sleep(1.0)
                self._send({"response":
                            "CMD: find . -name '*.log' -mtime +7 -delete\n"
                            "the researched way\n"
                            "REASON: because BECAUSE-KEPT and not the naive one"})
                return
            # Match the Question: line, not the whole prompt -- the
            # session log carries every earlier question, so a substring
            # test answers the wrong turn.
            q = ""
            for line in p.splitlines():
                if line.startswith("Question:"):
                    q = line
            if "why that one" in q:
                self._send({"response": "WHY-ANSWERED\nCMD: true"})
                return
            if "anything" in q:
                self._send({"response": "OFF-ANSWERED\nCMD: true"})
                return
            self._send({"response": "the quick way\nCMD: rm *.log"})

        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakeOllama)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    home = tempfile.mkdtemp(prefix="goulash-test-")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write("[engine]\nprovider = \"ollama\"\n"
                f"host = \"http://127.0.0.1:{port}\"\nstream = false\n"
                "commentary = false\nslow = \"ingest\"\n")
    proc, mfd = spawn(["zsh"], home=home)
    time.sleep(1.5)

    # #? asks BOTH: fast answers now and keeps the band; slow researches
    # the same turn and amends it later.
    os.write(mfd, b"#? how do i clear old logs\r")
    out = read_until(mfd, rb"the quick way", 8.0)
    check("fast answers a #? immediately", b"the quick way" in out, out[-300:])
    check("fast's command is the one vended", b"rm \\*.log" in out or b"rm *.log" in out,
          out[-300:])
    check("both lanes got the question",
          len([p for p in prompts if "clear old logs" in p]) == 2,
          f"{len(prompts)} prompts")

    # The finding lands on the turn it came from, not at the top.
    time.sleep(2.0)
    os.write(mfd, b"\x1b[B")          # Down: fast's command first
    out = read_until(mfd, rb"1/2", 5.0)
    check("the alternative is a step of its own", b"1/2" in out, out[-400:])
    check("the researched finding insets under the turn",
          b"\\xe2\\x86\\xb3" in out or "↳".encode() in out, out[-400:])
    check("an unselected finding is grey",
          b"\x1b[0;97;48;5;238m" in out, out[-400:])
    check("the question stub stays on terminal background",
          b"\x1b[0;2m ? how do i clear" in out, out[-500:])
    check("...and the block starts after it, not at column one",
          out.find(b"\x1b[0;97;48;5;238m") > out.find(b"\x1b[0;2m ? how do i clear"),
          out[-500:])
    # Down again walks INTO the alternative -- depth-first, one axis.
    os.write(mfd, b"\x1b[B")
    out = read_until(mfd, rb"2/2", 5.0)
    check("Down steps into the researched command", b"2/2" in out, out[-400:])
    # Orange is the SELECTION indicator, not a category: it moves down to
    # the finding, and fast's chip goes grey behind it.
    check("...and orange moves to it",
          out.rfind(b"\x1b[0;30;48;5;208m\xe2\x86\xb3") >
          out.rfind(b"\x1b[0;97;48;5;238m\xe2\x86\xb3"), out[-600:])
    check("...while fast's chip goes grey",
          b"\x1b[0;97;48;5;238m \xe2\x86\x93 suggestion" in out, out[-600:])
    os.write(mfd, b"\x15")            # ^U: drop the line, end browsing
    time.sleep(0.5)

    # The reasoning is retained where fast can read it, since fast is the
    # one who gets asked "why".
    os.write(mfd, b"#why that one\r")
    read_until(mfd, rb"WHY-ANSWERED", 8.0)
    asked = [p for p in prompts if "why that one" in p]
    check("the reasoning reaches fast's context",
          asked and "BECAUSE-KEPT" in asked[-1], "")
    check("the amendment is by reference, not a rewrite",
          asked and "amends the suggestion above" in asked[-1], "")

    # Turning slow off answers via fast and says so, rather than refusing.
    os.write(mfd, b"#/settings\r")
    read_until(mfd, rb"slow: ingest", 5.0)
    os.write(mfd, b"\x1b[B")
    time.sleep(0.3)
    for _ in range(3):                # ingest -> volunteer -> manual -> off
        os.write(mfd, b"\r")
        time.sleep(0.4)
    out = read_until(mfd, rb"slow: off", 4.0)
    check("the engagement ladder cycles to off", b"slow: off" in out, out[-300:])
    os.write(mfd, b"\x1b")
    time.sleep(0.4)
    before = len(prompts)
    os.write(mfd, b"#? anything\r")
    out = read_until(mfd, rb"OFF-ANSWERED", 8.0)
    check("#? with slow off still answers", b"OFF-ANSWERED" in out, out[-300:])
    check("...and says why it behaved like a #",
          b"answered by fast" in out, out[-400:])
    time.sleep(0.5)
    check("...and does not dispatch research",
          not any("Take your time" in p for p in prompts[before:]), "")

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
            if "crib for this reference" in p:
                # The card call, once per pin. Only commandRef.md needs a
                # meaningful answer; the others just have to not crash.
                if "commandRef.md" in p:
                    assert "widgetctl" in p, "card source lost the tool"
                    self._send({"response": "widgetctl: `widgetctl sync --all`  # CARDED"})
                else:
                    self._send({"response": "a crib"})
                return
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
    out = read_until(mfd, "@ pinned ▸".encode(), 5.0)
    check("bare #@ opens the pin browser",
          "@ pinned ▸".encode() in out, out[-300:])
    check("browser offers a + pin row", "+ pin a file".encode() in out, out[-400:])
    os.write(mfd, b"\x1b")
    time.sleep(0.4)

    # The deterministic form: no model involved at all.
    os.write(mfd, b"#@/path commandRef.md\r")
    out = read_until(mfd, rb"verbatim", 5.0)
    check("#@/path pins without the LLM",
          b"commandRef.md" in out and b"verbatim" in out, out[-300:])
    check("pin shows in the chrome", b"@commandRef.md" in out, out[-300:])

    # ...and the file's content is actually in front of the model.
    # Barrier: the card is cooked ASYNCHRONOUSLY, so asking before it
    # lands legitimately gets the deterministic card and the assertion
    # below fails for a reason that is not a bug. Wait for the crib call
    # itself rather than sleeping and hoping.
    deadline = time.time() + 10
    while time.time() < deadline and not any(
        "crib for this reference" in p and "commandRef.md" in p for p in prompts
    ):
        time.sleep(0.2)
    check("the card was actually cooked before we asked",
          any("crib for this reference" in p and "commandRef.md" in p for p in prompts),
          str(len(prompts)))
    os.write(mfd, b"#how do i sync\r")
    read_until(mfd, rb"PASS", 8.0)
    asked = [p for p in prompts if "how do i sync" in p]
    check("pinned text reaches the prompt",
          asked and "widgetctl sync --all" in asked[-1], "")
    check("pin rides the stable prefix, above the session log",
          asked and asked[-1].index("Working context") < asked[-1].index("Session log"),
          "")
    # The card is the SECOND emission: a few lines next to the question,
    # where a sliding-window model actually attends. Cache-warm prefix
    # copy above, cheap restatement below.
    check("a card rides next to the question",
          asked and "Pinned right now" in asked[-1], "")
    check("the card is below the session log, beside the question",
          asked and asked[-1].index("Pinned right now") > asked[-1].index("Session log")
          and asked[-1].index("Pinned right now") < asked[-1].index("how do i sync"),
          "")
    check("the card kept the command",
          asked and "widgetctl sync --all" in
          asked[-1][asked[-1].index("Pinned right now"):], "")
    check("a written card replaces the deterministic one",
          asked and "CARDED" in asked[-1], "")

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
    # Accumulate across both waits. goulash paints the band ONCE, when
    # the content changes; a read that matches the prompt and throws the
    # rest away can swallow that paint, and nothing re-sends it. (It used
    # to: an insurance repaint retransmitted the band every second, which
    # quietly propped up every test that discarded a paint.)
    out = read_until(mfd, rb"\$", 4.0)
    out = read_until(mfd, rb"@commandRef.md\+1\*", 5.0, out)
    check("a changed pin is marked in the chrome",
          b"@commandRef.md+1*" in out, out[-300:])
    # ...and the browser says which one, in words.
    os.write(mfd, b"#@\r")
    out = read_until(mfd, rb"changed", 5.0)
    check("the browser names the changed pin", b"changed" in out, out[-400:])
    os.write(mfd, b"\x1b")
    time.sleep(0.4)
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

    # The browser's two actions: compose a pin, and arm-then-confirm a
    # drop. Same gestures as the memory browser, deliberately.
    os.write(mfd, b"#@\r")
    read_until(mfd, "@ pinned ▸".encode(), 5.0)
    os.write(mfd, b"\r")                      # Enter on "+ pin a file …"
    out = read_until(mfd, "⏎ save".encode(), 4.0)
    check("compose mode entered from the browser",
          "⏎ save".encode() in out, out[-300:])
    os.write(mfd, b"OTHER.md\r")
    out = read_until(mfd, rb"OTHER.md", 6.0)
    check("a pin composed in the browser lands", b"OTHER.md" in out, out[-300:])
    os.write(mfd, b"big")                     # filter to the big pin
    time.sleep(0.4)
    os.write(mfd, b"\x1b[B")                  # past "+ pin", onto the entry
    time.sleep(0.3)
    # Enter reads the pin: not the file on disk, but what goulash is
    # actually SENDING for it — which for this one is the digest, and
    # that is otherwise invisible from inside the session.
    os.write(mfd, b"\r")
    out = read_until(mfd, "esc back".encode(), 4.0)
    check("Enter reads the pin", "esc back".encode() in out, out[-500:])
    check("the pane names the real path, not just the label",
          os.path.join(work, "big.md").encode() in out, out[-800:])
    check("the pane reports what tier is being sent",
          b"DIGESTED" in out or b"digest" in out, out[-800:])
    # The pane takes every row it can — but never the shell's floor.
    # Chrome geometry is `colsxinner+reserved`, so this reads it back.
    geom = re.findall(rb"80x(\d+)\+(\d+)", out)
    check("the pane grows the band", geom and int(geom[-1][1]) > 6, str(geom[-3:]))
    check("...but never squeezes the shell below its floor",
          geom and int(geom[-1][0]) >= 10, str(geom[-3:]))
    os.write(mfd, b"\x1b")                    # back to the list
    time.sleep(0.4)
    # Backspace is the destructive verb once the filter is empty; the
    # filter is "big" here, so it eats that first — three of them.
    os.write(mfd, b"\x7f\x7f\x7f")
    time.sleep(0.4)
    os.write(mfd, b"\x1b[B")                  # past "+ pin" again
    time.sleep(0.3)
    os.write(mfd, b"\x7f")                    # arms
    out = read_until(mfd, "again to unpin".encode(), 4.0)
    check("backspace on an empty filter arms the unpin",
          "again to unpin".encode() in out, out[-400:])
    os.write(mfd, b"\x7f")                    # confirms
    out = read_until(mfd, rb"dropped", 5.0)
    check("second backspace drops the pin", b"dropped" in out, out[-300:])
    os.write(mfd, b"\x1b")
    time.sleep(0.4)

    # A literal path resolves DIRECTLY, with no model call -- `#@ .` is
    # a path the user typed, not a request to interpret.
    before = len(prompts)
    os.write(mfd, b"#@ .\r")
    time.sleep(1.0)
    check("a bare literal path pins without the model",
          not any("change the pinned working context" in p
                  for p in prompts[before:]), "")

    # Unset really unsets, and the block goes back to costing nothing.
    os.write(mfd, b"#@/unset\r")
    time.sleep(0.5)
    os.write(mfd, b"true\r")
    out = read_until(mfd, rb"goulash", 5.0)
    # Measure the LAST chrome paint in the window: earlier ones in the
    # same buffer legitimately still carry the pin.
    chrome = out.rsplit("goulash │".encode(), 1)[-1]
    check("#@/unset clears the chrome marker",
          b"@" not in chrome, chrome[:200])
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
    # Enter READS. It used to forget, which put a destructive verb on the
    # key every other menu uses for "yes, this one".
    os.write(mfd, b"\r")
    out = read_until(mfd, "esc back".encode(), 4.0)
    check("Enter opens the reading pane", "esc back".encode() in out, out[-400:])
    check("the pane shows the slot's full text",
          b"TARGET=prod" in out, out[-400:])
    os.write(mfd, b"\x1b")            # back to the LIST, not out of the menu
    out = read_until(mfd, rb"\[1\] deploy", 4.0)
    check("esc leaves the pane, not the browser",
          b"[1] deploy" in out, out[-400:])
    # Delete is the destructive verb now, and it still needs confirming.
    os.write(mfd, b"\x1b[3~")         # arm
    out = read_until(mfd, "again to forget".encode(), 4.0)
    check("first delete arms, does not delete",
          "again to forget".encode() in out, out[-300:])
    os.write(mfd, b"\x1b[3~")         # confirm
    out = read_until(mfd, rb"forgot \[1\]", 4.0)
    check("second delete forgets the slot", b"forgot [1]" in out, out[-300:])
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


def last_paint(chunk, marker):
    """The bytes of the last complete band paint containing `marker`.

    One paint is one write: DECSC, the rows, DECRC. Anchoring on the
    whole paint lets a test measure the paint it cares about, instead of
    reading forward from a marker into whatever arrives next -- which
    only worked while a periodic repaint guaranteed something would."""
    end = chunk.rfind(marker)
    if end < 0:
        return b""
    start = chunk.rfind(b"\x1b7", 0, end)
    if start < 0:
        return b""
    stop = chunk.find(b"\x1b8", end)
    return chunk[start:stop + 2] if stop >= 0 else chunk[start:]


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
        # Paints are suspended until the drag settles, and a paint made
        # at the OLD width can still be in the pipe -- measuring that one
        # reports rows a cell too wide. The chrome row prints the live
        # geometry, so wait for the paint whose chrome says the NEW
        # width, and measure THAT paint: read to its closing DECRC and
        # slice back to its opening DECSC.
        marker = f"# {cols}x".encode()
        out = read_until(mfd, re.escape(marker) + rb".*?\x1b8", 4.0)
        rows = painted_rows(last_paint(out, marker))
        # A window that captured no paint at all would pass this check
        # vacuously, which is the failure mode of anchoring on a marker.
        check(f"band measured at {cols} cols", rows, "no paint in the window")
        wide = [(r, w) for r, w in rows if w > cols - 1]
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


def test_idle_with_repaint_off():
    """A crash found by a user sitting still. `idle_ticks` was a u8 bumped
    unconditionally but cleared only inside the repaint branch, so turning
    `idle_repaint` off left it climbing with nothing to stop it: overflow
    panic after ~64 seconds of genuine idle (256 ticks x 250ms).

    Slow by nature — it has to outlast the counter — but a crash that
    needs only "leave it alone for a minute" earns a minute."""
    print("survives a long idle with idle_repaint off:")
    home = tempfile.mkdtemp(prefix="goulash-test-")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write("[debug]\nidle_repaint = false\n")
    proc, mfd = spawn(["bash"], home=home)
    deadline = time.time() + 75
    while time.time() < deadline and proc.poll() is None:
        r, _, _ = select.select([mfd], [], [], 0.5)
        if r:
            try:
                os.read(mfd, 65536)
            except OSError:
                break
    check("still running after 75s idle", proc.poll() is None,
          f"exited rc={proc.poll()}")
    try:
        proc.kill()
    except OSError:
        pass
    try:
        os.close(mfd)
    except OSError:
        pass


def test_an_idle_session_writes_nothing():
    """Reported from the field: goulash left open for hours, WindowServer
    burning GPU on an idle terminal.

    The insurance repaint fired on a CLOCK -- once a second, forever --
    and unlike every other paint in the loop it compared nothing before
    writing. Each pass emitted a scroll-region set plus the whole band:
    3,600 terminal writes an hour on a session doing nothing, each one a
    redraw the emulator had to composite.

    It insures against output we mis-parsed damaging the band, so output
    is its precondition. No output since the last paint means nothing can
    be broken, and the correct number of repaints is zero. This measures
    exactly that: bytes on the wire while nobody touches anything."""
    print("an idle session writes nothing to the terminal:")
    home = tempfile.mkdtemp(prefix="goulash-test-")
    proc, mfd = spawn(["bash"], home=home)
    # Let startup and the shell's first prompt finish, then absorb the
    # one insurance pass the last of that output legitimately arms.
    deadline = time.time() + 6
    while time.time() < deadline:
        r, _, _ = select.select([mfd], [], [], 0.5)
        if r:
            try:
                os.read(mfd, 65536)
            except OSError:
                break
    # Now measure. Nothing is typed, nothing runs, nothing streams.
    quiet = 12
    seen = 0
    deadline = time.time() + quiet
    while time.time() < deadline:
        r, _, _ = select.select([mfd], [], [], 0.5)
        if r:
            try:
                seen += len(os.read(mfd, 65536))
            except OSError:
                break
    # The old code wrote ~600 bytes a second here, so ~7KB over this
    # window. A generous ceiling still fails that by an order of
    # magnitude, and leaves room for one late insurance pass.
    check(f"idle for {quiet}s emits almost nothing ({seen} bytes)",
          seen < 1000, f"{seen} bytes in {quiet}s of idle")
    check("and it is still running", proc.poll() is None,
          f"exited rc={proc.poll()}")
    try:
        proc.kill()
    except OSError:
        pass
    try:
        os.close(mfd)
    except OSError:
        pass


def test_stats_row():
    """The runaway meter. Every defect in the 2026-07 review had the same
    shape -- growth unconditional, clearing conditional -- and none was
    visible until it was fatal. This is the row that makes them visible,
    so the test has to prove the numbers MOVE, not merely that a row was
    drawn."""
    print("stats row reports live counters:")
    if not shutil.which("zsh"):
        print("  [SKIP] zsh not installed")
        return
    home = tempfile.mkdtemp(prefix="goulash-test-")
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write("[status]\nstats = true\n")
    proc, mfd = spawn(["zsh"], home=home, rows=24, cols=120)
    time.sleep(1.5)
    out = read_until(mfd, rb"slots", 6.0)
    check("stats appear in the chrome row", b"slots" in out and b"ctx" in out,
          out[-200:])
    check("memory is reported non-zero", re.search(rb"\d+[KMG] \xc2\xb7", out) is not None,
          out[-200:])
    # Counters are kept by the WORKER, so they only move if the engine
    # actually saw the job. No engine here, so the ask still counts.
    os.write(mfd, b"# does this count\r")
    out = read_until(mfd, rb"1a/", 8.0)
    check("an ask increments the meter", b"1a/" in out, out[-200:])
    os.write(mfd, b"exit\r")
    drain_exit(proc, mfd)


def test_startup_preserves_the_screen():
    """Launching used to leave the terminal half-overwritten: a fresh
    prompt at the top, the tail of whatever was there stranded below it,
    and none of the visible screen in scrollback. Reproduced by running
    `ls -R` first.

    Setting the scroll region homes the cursor, the shell's first prompt
    cycle draws from the region top, and the new session paints downward
    over live content while everything below the write head keeps the
    old — so it was neither an append nor a clear.

    This needs a real screen model: an under-erase is bytes that were
    never sent, so a byte-stream assertion cannot see it."""
    print("startup scrolls the screen into scrollback:")
    try:
        import pyte
    except ImportError:
        print("  [SKIP] pyte not installed (pip install pyte)")
        return
    rows, cols = 24, 80
    home = tempfile.mkdtemp(prefix="goulash-test-")
    markers = " ".join(f"echo MARK-{i};" for i in range(1, 16))
    pid, fd = pty.fork()
    if pid == 0:
        os.environ.update(TERM="xterm-256color", HOME=home, GOULASH_HOME=home)
        os.execv("/bin/sh", ["sh", "-c", f"{markers} exec {os.path.abspath(BIN)} /bin/bash"])
    set_winsize(fd, rows, cols)
    screen = pyte.HistoryScreen(cols, rows, history=2000)
    stream = pyte.ByteStream(screen)
    deadline = time.time() + 4
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.1)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            stream.feed(chunk)
    try:
        os.kill(pid, 9)
        os.close(fd)
    except OSError:
        pass

    def render(line):
        return "".join(line[i].data for i in sorted(line)) if hasattr(line, "keys") else str(line)

    visible = [l for l in screen.display if "MARK-" in l]
    check("no stale content left on the visible screen", not visible, str(visible[:3]))
    scrolled = [render(l) for l in screen.history.top if "MARK-" in render(l)]
    check("every prior line is in scrollback, not destroyed",
          len(scrolled) >= 15, f"{len(scrolled)}/15 recovered")
    band = [l for l in screen.display if "goulash" in l and "bash" in l]
    check("and the band is drawn", bool(band), str(screen.display[-1]))


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
        test_adapter_fidelity,
        test_rc_loading,
        test_tab_completion,
        test_engine_ollama,
        test_engine_openai,
        test_per_lane_providers,
        test_model_menu,
        test_model_capabilities,
        test_chat_mode,
        test_slow_lane,
        test_working_context,
        test_memory,
        test_resize_hygiene,
        test_idle_with_repaint_off,
        test_an_idle_session_writes_nothing,
        test_stats_row,
        test_startup_preserves_the_screen,
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
