use crate::config::Config;
use crate::engine::{self, Engine};
use crate::osc::{Mark, OscFilter, Seg};
use crate::pty;
use crate::record::Recorder;
use crate::sense::{self, HookPhase, Sensor, State};
use crate::status;
use crate::term::{self, RawGuard, Size};
use crate::vendor::{self, RulesVendor};
use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::sys::signal::{self, SaFlags, SigAction, SigHandler, SigSet, Signal};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::os::unix::process::ExitStatusExt;
use std::sync::atomic::{AtomicI32, Ordering};

const STDIN: RawFd = 0;
const STDOUT: RawFd = 1;

/// Write end of the SIGWINCH self-pipe; -1 until installed.
static WINCH_PIPE_WR: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_sigwinch(_: libc::c_int) {
    let fd = WINCH_PIPE_WR.load(Ordering::Relaxed);
    if fd >= 0 {
        // SAFETY: write(2) is async-signal-safe; best-effort wakeup.
        unsafe { libc::write(fd, b"w".as_ptr().cast(), 1) };
    }
}

fn write_all(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        // SAFETY: fd is a valid open descriptor for the duration of the session.
        let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}

fn read_some(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        // SAFETY: fd valid, buf sized by caller.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        return Ok(n as usize);
    }
}

/// Ask the real terminal where the cursor is (DSR). Bytes that arrive that
/// are not part of the report are returned as `typed_ahead` for forwarding
/// to the shell. Falls back to bottom-of-screen on timeout so the reserved
/// rows always end up clear of existing content.
fn query_cursor_row(real: Size) -> (u16, u16, Vec<u8>) {
    let _ = write_all(STDOUT, b"\x1b[6n");
    let mut acc: Vec<u8> = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    loop {
        let remain = deadline.saturating_duration_since(std::time::Instant::now());
        if remain.is_zero() {
            break;
        }
        let stdin_fd = unsafe { BorrowedFd::borrow_raw(STDIN) };
        let mut fds = [PollFd::new(stdin_fd, PollFlags::POLLIN)];
        let timeout = PollTimeout::try_from(remain.as_millis().max(1) as i32).unwrap();
        match nix::poll::poll(&mut fds, timeout) {
            Ok(0) => break,
            Ok(_) => {
                let mut byte = [0u8; 64];
                match read_some(STDIN, &mut byte) {
                    Ok(0) => break,
                    Ok(n) => {
                        acc.extend_from_slice(&byte[..n]);
                        if acc.contains(&b'R') {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            Err(_) => break,
        }
    }
    // Find the last ESC [ row ; col R in the accumulated bytes.
    if let Some(start) = acc.windows(2).rposition(|w| w == b"\x1b[")
        && let Some(end_rel) = acc[start..].iter().position(|&b| b == b'R')
    {
        let body = &acc[start + 2..start + end_rel];
        let text: String = body.iter().map(|&b| b as char).collect();
        let mut parts = text.split(';');
        if let (Some(r), Some(c)) = (parts.next(), parts.next())
            && let (Ok(row), Ok(col)) = (r.parse::<u16>(), c.parse::<u16>())
        {
            let mut typed_ahead = acc[..start].to_vec();
            typed_ahead.extend_from_slice(&acc[start + end_rel + 1..]);
            return (row.min(real.rows), col.min(real.cols), typed_ahead);
        }
    }
    (real.rows, 1, acc)
}

/// Sequences in the child's output that can blow away our scroll region or
/// status row: DECSTBM reset, RIS, DECSTR soft reset, full clears — and
/// erase-below (ESC[J / ESC[0J), which line editors emit on every refresh
/// and which erases to the end of the *real* screen, straight through the
/// reserved rows (ED is not bounded by scroll regions). The caller repaints
/// in the same write batch as the trigger, so the wiped bar never renders.
/// Returns the end index of the earliest trigger in `chunk`, so the caller
/// can re-pin the scroll region at exactly that boundary — before any
/// scrolling output that follows it in the same chunk can drag the status
/// row up into the inner region.
fn find_trigger_end(chunk: &[u8]) -> Option<usize> {
    const TRIGGERS: [&[u8]; 7] = [
        b"\x1b[r", b"\x1bc", b"[!p", b"[2J", b"[3J", b"\x1b[J", b"\x1b[0J",
    ];
    TRIGGERS
        .iter()
        .filter_map(|t| {
            chunk
                .windows(t.len())
                .position(|w| w == *t)
                .map(|i| i + t.len())
        })
        .min()
}

struct Layout {
    real: Size,
    reserved: u16,
}

impl Layout {
    fn inner(&self) -> Size {
        Size {
            rows: (self.real.rows.saturating_sub(self.reserved)).max(1),
            cols: self.real.cols,
        }
    }
    fn status_row(&self) -> u16 {
        // 1-based real row of the (first) reserved row.
        self.inner().rows + 1
    }
}

/// Scroll-region assertion + status redraw + cursor/attribute restore,
/// derived from the tracked inner-screen state.
fn fixup_bytes(layout: &Layout, screen: &vt100::Screen, bar: &str) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(256);
    let inner = layout.inner();
    out.extend_from_slice(b"\x1b[?25l"); // hide cursor while we work
    out.extend_from_slice(format!("\x1b[1;{}r", inner.rows).as_bytes());
    out.extend_from_slice(format!("\x1b[{};1H", layout.status_row()).as_bytes());
    out.extend_from_slice(bar.as_bytes());
    out.extend_from_slice(&screen.attributes_formatted());
    out.extend_from_slice(&screen.cursor_state_formatted());
    out
}

#[allow(clippy::type_complexity)]
fn render_bar(
    layout: &Layout,
    shell_name: &str,
    st: &State,
    hook: Option<HookPhase>,
    suggestions: &[(u64, String, String)],
    notice: &Option<String>,
) -> String {
    // A notice (e.g. an acknowledged aside) displays verbatim; a pending
    // suggestion gets the pull-arrow affordance.
    let extra = notice
        .clone()
        .or_else(|| suggestions.first().map(|s| format!("\u{2193} {}", s.1)));
    status::render(
        layout.real,
        layout.inner().rows,
        shell_name,
        sense::label(st, hook),
        extra.as_deref(),
    )
}

/// `#/` command dispatch. Returns the bar notice to show.
fn slash_command(
    cmd: &str,
    arg: Option<&str>,
    engine: Option<&Engine>,
    engine_model: &Option<String>,
    blocks: usize,
) -> Option<String> {
    match (cmd, arg) {
        ("model", Some(name)) => match engine {
            Some(eng) => {
                eng.set_model(name.to_string());
                Some(format!("switching model to {name} \u{2026}"))
            }
            None => Some("no engine running".to_string()),
        },
        ("model", None) => match engine {
            Some(eng) => {
                eng.list_models();
                Some("listing models \u{2026}".to_string())
            }
            None => Some("no engine running".to_string()),
        },
        ("status", _) => Some(format!(
            "goulash {} \u{b7} engine: {} \u{b7} {} blocks this session",
            env!("CARGO_PKG_VERSION"),
            engine_model.as_deref().unwrap_or("none"),
            blocks,
        )),
        ("help", _) => {
            Some("#/model [name] \u{b7} #/status \u{b7} #/help \u{b7} # <question>".to_string())
        }
        _ => Some(format!("unknown command /{cmd} \u{2014} try #/help")),
    }
}

pub fn run(cfg: &Config, argv: Vec<String>) -> io::Result<i32> {
    let real = term::get_size(STDOUT)?;
    if real.rows < 4 || real.cols < 10 {
        return Err(io::Error::other("terminal too small"));
    }
    let mut layout = Layout {
        real,
        reserved: cfg.reserved_rows(),
    };
    let shell_name = std::path::Path::new(&argv[0])
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| argv[0].clone());

    let integ = crate::integrate::prepare(argv.clone(), cfg.shell.auto_integrate);
    let mut p = pty::spawn(&integ.argv, &integ.envs, layout.inner())?;
    let master = p.master.as_raw_fd();

    let raw = RawGuard::new(unsafe { BorrowedFd::borrow_raw(STDIN) })
        .map_err(|e| io::Error::other(format!("raw mode: {e}")))?;

    // SIGWINCH self-pipe.
    let (winch_rd, winch_wr) = nix::unistd::pipe()?;
    WINCH_PIPE_WR.store(winch_wr.as_raw_fd(), Ordering::Relaxed);
    let action = SigAction::new(
        SigHandler::Handler(on_sigwinch),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    // SAFETY: handler only does an async-signal-safe write to the pipe.
    unsafe { signal::sigaction(Signal::SIGWINCH, &action) }
        .map_err(|e| io::Error::other(format!("sigaction: {e}")))?;

    // Place existing screen content clear of the reserved rows, without
    // clearing the screen or entering the alternate screen: scroll just
    // enough that the cursor lands inside the inner region.
    let (row, col, typed_ahead) = query_cursor_row(layout.real);
    let inner_rows = layout.inner().rows;
    let mut init: Vec<u8> = Vec::new();
    let final_row = if row > inner_rows {
        init.extend(std::iter::repeat_n(b'\n', (row - inner_rows) as usize));
        inner_rows
    } else {
        row
    };
    init.extend_from_slice(format!("\x1b[1;{inner_rows}r").as_bytes());
    init.extend_from_slice(format!("\x1b[{final_row};{col}H").as_bytes());
    write_all(STDOUT, &init)?;

    let mut parser = vt100::Parser::new(inner_rows, layout.real.cols, 0);
    let sensor = Sensor::new(master, p.child.id());
    let mut rec = Recorder::new(cfg);
    rec.start(&argv, layout.real.rows, layout.real.cols);
    let mut cur_state: State = sensor.read(false);
    rec.state(cur_state);

    let mut oscf = OscFilter::new();
    let mut hook: Option<HookPhase> = None;
    let mut rules = RulesVendor::new();
    let mut suggestions: Vec<(u64, String, String)> = Vec::new();
    let mut next_sid: u64 = 1;
    let mut cur_cmd: Option<String> = None;
    let mut block_tail: Vec<u8> = Vec::new();
    let mut last_cwd = String::new();
    let mut notice: Option<String> = None;
    let mut recent_blocks: Vec<(String, i32, String)> = Vec::new();
    let mut engine: Option<Engine> = if cfg.engine.provider != "none" {
        Engine::start(cfg.engine.clone()).ok()
    } else {
        None
    };
    let mut engine_model: Option<String> = None;
    let mut last_bar = render_bar(
        &layout,
        &shell_name,
        &cur_state,
        hook,
        &suggestions,
        &notice,
    );
    write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &last_bar))?;
    if !typed_ahead.is_empty() {
        let _ = write_all(master, &typed_ahead);
    }

    let mut buf = [0u8; 65536];
    let mut stdin_open = true;
    let mut dirty = false;
    let mut idle_ticks: u8 = 0;

    'session: loop {
        let stdin_fd = unsafe { BorrowedFd::borrow_raw(STDIN) };
        let master_fd = unsafe { BorrowedFd::borrow_raw(master) };
        let mut fds: Vec<PollFd> = Vec::with_capacity(4);
        fds.push(PollFd::new(master_fd, PollFlags::POLLIN));
        fds.push(PollFd::new(winch_rd.as_fd(), PollFlags::POLLIN));
        let engine_idx = if let Some(eng) = engine.as_ref() {
            fds.push(PollFd::new(eng.wake.as_fd(), PollFlags::POLLIN));
            Some(fds.len() - 1)
        } else {
            None
        };
        let stdin_idx = if stdin_open {
            fds.push(PollFd::new(stdin_fd, PollFlags::POLLIN));
            Some(fds.len() - 1)
        } else {
            None
        };
        // Tick at 250ms even when idle so job-control transitions with no
        // accompanying I/O (e.g. `sleep 5` starting) are still sensed;
        // 30ms while dirty for quiescence-debounced redraws.
        let timeout = PollTimeout::try_from(if dirty { 30 } else { 250 }).unwrap();
        let n = match nix::poll::poll(&mut fds, timeout) {
            Ok(n) => n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(io::Error::other(format!("poll: {e}"))),
        };

        let master_ready = n > 0 && fds[0].revents().is_some_and(|r| !r.is_empty());
        let winch_ready = n > 0
            && fds[1]
                .revents()
                .is_some_and(|r| r.contains(PollFlags::POLLIN));
        let engine_revents = if n > 0 {
            engine_idx
                .and_then(|i| fds.get(i))
                .and_then(|f| f.revents())
        } else {
            None
        };
        let engine_ready_fd = engine_revents.is_some_and(|r| r.contains(PollFlags::POLLIN));
        let engine_hup =
            engine_revents.is_some_and(|r| r.intersects(PollFlags::POLLHUP | PollFlags::POLLERR));
        let stdin_ready = n > 0
            && stdin_idx
                .and_then(|i| fds.get(i))
                .and_then(|f| f.revents())
                .is_some_and(|r| !r.is_empty());

        if master_ready {
            match read_some(master, &mut buf) {
                Ok(0) => break 'session,
                Ok(len) => {
                    // Cleaned stream and marks arrive as ordered segments,
                    // so output is attributed to the correct command block
                    // even when B/output/D land in a single read.
                    let mut trigger_seen = false;
                    for seg in oscf.feed(&buf[..len]) {
                        match seg {
                            Seg::Bytes(bytes) => {
                                if cur_cmd.is_some() {
                                    block_tail.extend_from_slice(&bytes);
                                    let excess = block_tail.len().saturating_sub(8192);
                                    if excess > 0 {
                                        block_tail.drain(..excess);
                                    }
                                }
                                rec.output(&bytes);
                                // Re-pin the scroll region at trigger
                                // boundaries (see find_trigger_end).
                                let mut rest: &[u8] = &bytes;
                                while let Some(end) = find_trigger_end(rest) {
                                    let (pre, post) = rest.split_at(end);
                                    parser.process(pre);
                                    write_all(STDOUT, pre)?;
                                    let mut fix =
                                        format!("\x1b[1;{}r", layout.inner().rows).into_bytes();
                                    fix.extend_from_slice(
                                        &parser.screen().cursor_state_formatted(),
                                    );
                                    write_all(STDOUT, &fix)?;
                                    trigger_seen = true;
                                    rest = post;
                                }
                                parser.process(rest);
                                write_all(STDOUT, rest)?;
                            }
                            Seg::Mark(m) => match m {
                                Mark::Prompt => {
                                    hook = Some(HookPhase::Prompt);
                                    rec.prompt();
                                }
                                Mark::CmdStart(cmd) => {
                                    hook = Some(HookPhase::Command);
                                    rec.cmd_start(&cmd);
                                    cur_cmd = Some(cmd);
                                    block_tail.clear();
                                    notice = None;
                                }
                                Mark::CmdEnd(code) => {
                                    rec.cmd_end(code);
                                    if code == 0 {
                                        // Whatever was broken got fixed (or
                                        // moved past): drop stale fixes.
                                        suggestions.clear();
                                    }
                                    if let Some(cmd) = cur_cmd.take() {
                                        let block = vendor::CmdBlock {
                                            cmd,
                                            exit_code: code,
                                            cwd: last_cwd.clone(),
                                            output_tail: String::from_utf8_lossy(&block_tail)
                                                .into_owned(),
                                        };
                                        recent_blocks.push((
                                            block.cmd.clone(),
                                            block.exit_code,
                                            block.output_tail.chars().take(1200).collect(),
                                        ));
                                        if recent_blocks.len() > 8 {
                                            recent_blocks.remove(0);
                                        }
                                        for v in rules.suggest(&block) {
                                            let id = next_sid;
                                            next_sid += 1;
                                            rec.suggest(id, &v.command, &v.why, v.vendor);
                                            suggestions.insert(0, (id, v.command, v.why));
                                        }
                                        suggestions.truncate(8);
                                    }
                                }
                                Mark::Cwd(p) => {
                                    if !last_cwd.is_empty() && p != last_cwd {
                                        // cwd changed: context moved, old
                                        // suggestions are stale.
                                        suggestions.clear();
                                    }
                                    rec.cwd(&p);
                                    last_cwd = p;
                                }
                                Mark::Ask(q) => {
                                    rec.aside(&q);
                                    let body = q.trim_start_matches('#').trim();
                                    if let Some(cmdline) = body.strip_prefix('/') {
                                        // #/ commands: goulash controls, not
                                        // LLM asides. One arg max — the
                                        // single most obvious swivel.
                                        let mut it = cmdline.split_whitespace();
                                        let cmd = it.next().unwrap_or("");
                                        let arg = it.next();
                                        notice = slash_command(
                                            cmd,
                                            arg,
                                            engine.as_ref(),
                                            &engine_model,
                                            recent_blocks.len(),
                                        );
                                    } else if let Some(eng) = engine.as_ref()
                                        && engine_model.is_some()
                                    {
                                        let ctx = engine::build_context(&recent_blocks, &last_cwd);
                                        eng.ask(body.to_string(), ctx);
                                        notice = Some(format!("{q} \u{2026}"));
                                    } else {
                                        notice =
                                            Some(format!("{q} \u{2014} no engine configured yet"));
                                    }
                                }
                                Mark::Pull(buffer) => {
                                    // Context shifting: empty buffer pulls
                                    // the top suggestion; a buffer that IS
                                    // one of our suggestions cycles to the
                                    // next (kill line, repaste); the user's
                                    // own text is never clobbered.
                                    let next = if suggestions.is_empty() {
                                        None
                                    } else if buffer.is_empty() {
                                        Some(0)
                                    } else {
                                        suggestions
                                            .iter()
                                            .position(|s| s.1 == buffer)
                                            .map(|p| (p + 1) % suggestions.len())
                                    };
                                    if let Some(i) = next {
                                        let (id, cmdtext, _why) = suggestions[i].clone();
                                        rec.accept(id);
                                        let mut bytes = Vec::new();
                                        if !buffer.is_empty() {
                                            bytes.push(0x15); // ^U: kill line
                                        }
                                        bytes.extend_from_slice(b"\x1b[200~");
                                        bytes.extend_from_slice(cmdtext.as_bytes());
                                        bytes.extend_from_slice(b"\x1b[201~");
                                        write_all(master, &bytes)?;
                                    }
                                }
                            },
                        }
                    }
                    if trigger_seen {
                        last_bar = render_bar(
                            &layout,
                            &shell_name,
                            &cur_state,
                            hook,
                            &suggestions,
                            &notice,
                        );
                        write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &last_bar))?;
                        dirty = false;
                    } else {
                        dirty = true;
                    }
                }
                Err(e) if e.raw_os_error() == Some(libc::EIO) => break 'session,
                Err(e) => return Err(e),
            }
        }

        if winch_ready {
            let mut drain = [0u8; 64];
            let _ = read_some(winch_rd.as_raw_fd(), &mut drain);
            if let Ok(real) = term::get_size(STDOUT) {
                layout.real = real;
                let inner = layout.inner();
                parser.screen_mut().set_size(inner.rows, inner.cols);
                let _ = term::set_size(master, inner);
                rec.resize(real.rows, real.cols);
                last_bar = render_bar(
                    &layout,
                    &shell_name,
                    &cur_state,
                    hook,
                    &suggestions,
                    &notice,
                );
                write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &last_bar))?;
                dirty = false;
            }
        }

        if stdin_ready {
            match read_some(STDIN, &mut buf) {
                Ok(0) => stdin_open = false,
                Ok(len) => {
                    // Alt-Down: generic-shell suggestion pull. Only ever
                    // intercepted at a hook-confirmed prompt with a live
                    // suggestion; everything else passes through verbatim.
                    const ALT_DOWN: &[u8] = b"\x1b[1;3B";
                    let chunk = &buf[..len];
                    let pos = if hook == Some(HookPhase::Prompt) && !suggestions.is_empty() {
                        chunk.windows(ALT_DOWN.len()).position(|w| w == ALT_DOWN)
                    } else {
                        None
                    };
                    if let Some(p) = pos {
                        write_all(master, &chunk[..p])?;
                        let (id, cmdtext, _why) = suggestions[0].clone();
                        rec.accept(id);
                        let mut paste = Vec::new();
                        paste.extend_from_slice(b"\x1b[200~");
                        paste.extend_from_slice(cmdtext.as_bytes());
                        paste.extend_from_slice(b"\x1b[201~");
                        write_all(master, &paste)?;
                        write_all(master, &chunk[p + ALT_DOWN.len()..])?;
                        dirty = true;
                    } else {
                        write_all(master, chunk)?;
                    }
                }
                Err(_) => stdin_open = false,
            }
        }

        if engine_hup && !engine_ready_fd {
            // Worker died (panic or clean end): stop polling its pipe.
            engine = None;
            engine_model = None;
        }

        if engine_ready_fd && let Some(eng) = engine.as_ref() {
            let mut drain = [0u8; 64];
            let _ = read_some(eng.wake.as_raw_fd(), &mut drain);
            while let Ok(ev) = eng.events.try_recv() {
                match ev {
                    engine::Event::Ready { provider, model } => {
                        rec.engine_ready(&provider, &model);
                        notice = Some(format!("engine: {provider} \u{b7} {model}"));
                        engine_model = Some(model);
                    }
                    engine::Event::Answer(text) => {
                        let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        rec.aside_answer(&one_line, true);
                        notice = Some(one_line);
                    }
                    engine::Event::Error(msg) => {
                        rec.aside_answer(&msg, false);
                        notice = Some(format!("engine error: {msg}"));
                    }
                    engine::Event::Models(names) => {
                        let list = names
                            .iter()
                            .map(|n| {
                                if Some(n) == engine_model.as_ref() {
                                    format!("{n}*")
                                } else {
                                    n.clone()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" \u{b7} ");
                        notice = Some(format!("models: {list}"));
                    }
                }
                dirty = true;
            }
        }

        // Sense job-control / termios / alt-screen transitions.
        let st = sensor.read(parser.screen().alternate_screen());
        if st != cur_state {
            cur_state = st;
            rec.state(st);
            dirty = true;
        }

        // Quiescent and dirty: redraw the status row — but only if its
        // content actually changed. Ordinary output can't touch the bar
        // (the inner world is winsize-fenced; bar-threatening sequences
        // are handled as same-batch triggers above), so skipping no-op
        // redraws avoids needless writes. As insurance against trigger
        // sequences we don't know about, an idle repaint runs about once
        // a second — overwriting an intact bar with identical content is
        // invisible, so this costs nothing visually.
        if n == 0 {
            if dirty {
                let bar = render_bar(
                    &layout,
                    &shell_name,
                    &cur_state,
                    hook,
                    &suggestions,
                    &notice,
                );
                if bar != last_bar {
                    write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &bar))?;
                    last_bar = bar;
                }
                dirty = false;
                idle_ticks = 0;
            } else {
                idle_ticks += 1;
                if idle_ticks >= 4 {
                    idle_ticks = 0;
                    write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &last_bar))?;
                }
            }
        }
    }

    // Restore the terminal: full scroll region, cursor to the old status
    // row start, default attributes, visible cursor.
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"\x1b[r");
    out.extend_from_slice(format!("\x1b[{};1H", layout.status_row()).as_bytes());
    out.extend_from_slice(b"\x1b[0m\x1b[2K\x1b[?25h");
    let _ = write_all(STDOUT, &out);
    drop(raw);
    WINCH_PIPE_WR.store(-1, Ordering::Relaxed);

    let st = p.child.wait()?;
    let code = st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0));
    rec.end(code);
    Ok(code)
}
