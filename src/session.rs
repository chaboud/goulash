use crate::config::Config;
use crate::pty;
use crate::status;
use crate::term::{self, RawGuard, Size};
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
/// status row and therefore warrant an immediate fixup rather than waiting
/// for quiescence: DECSTBM reset, RIS, DECSTR soft reset, full clears.
fn needs_immediate_fixup(chunk: &[u8]) -> bool {
    const TRIGGERS: [&[u8]; 5] = [b"\x1b[r", b"\x1bc", b"[!p", b"[2J", b"[3J"];
    TRIGGERS
        .iter()
        .any(|t| chunk.windows(t.len()).any(|w| w == *t))
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
fn fixup_bytes(layout: &Layout, screen: &vt100::Screen, shell_name: &str) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(256);
    let inner = layout.inner();
    out.extend_from_slice(b"\x1b[?25l"); // hide cursor while we work
    out.extend_from_slice(format!("\x1b[1;{}r", inner.rows).as_bytes());
    out.extend_from_slice(format!("\x1b[{};1H", layout.status_row()).as_bytes());
    out.extend_from_slice(status::render(layout.real, inner.rows, shell_name).as_bytes());
    out.extend_from_slice(&screen.attributes_formatted());
    out.extend_from_slice(&screen.cursor_state_formatted());
    out
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

    let mut p = pty::spawn(&argv, layout.inner())?;
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
    write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &shell_name))?;
    if !typed_ahead.is_empty() {
        let _ = write_all(master, &typed_ahead);
    }

    let mut buf = [0u8; 65536];
    let mut stdin_open = true;
    let mut dirty = false;

    'session: loop {
        let stdin_fd = unsafe { BorrowedFd::borrow_raw(STDIN) };
        let master_fd = unsafe { BorrowedFd::borrow_raw(master) };
        let mut fds: Vec<PollFd> = Vec::with_capacity(3);
        fds.push(PollFd::new(master_fd, PollFlags::POLLIN));
        fds.push(PollFd::new(winch_rd.as_fd(), PollFlags::POLLIN));
        if stdin_open {
            fds.push(PollFd::new(stdin_fd, PollFlags::POLLIN));
        }
        let timeout = if dirty {
            PollTimeout::try_from(30).unwrap()
        } else {
            PollTimeout::NONE
        };
        let n = match nix::poll::poll(&mut fds, timeout) {
            Ok(n) => n,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(io::Error::other(format!("poll: {e}"))),
        };

        if n == 0 {
            // Quiescent and dirty: redraw the status row.
            if dirty {
                write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &shell_name))?;
                dirty = false;
            }
            continue;
        }

        let master_ready = fds[0].revents().is_some_and(|r| !r.is_empty());
        let winch_ready = fds[1]
            .revents()
            .is_some_and(|r| r.contains(PollFlags::POLLIN));
        let stdin_ready = stdin_open
            && fds
                .get(2)
                .and_then(|f| f.revents())
                .is_some_and(|r| !r.is_empty());

        if master_ready {
            match read_some(master, &mut buf) {
                Ok(0) => break 'session,
                Ok(len) => {
                    let chunk = &buf[..len];
                    parser.process(chunk);
                    write_all(STDOUT, chunk)?;
                    if needs_immediate_fixup(chunk) {
                        write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &shell_name))?;
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
                write_all(STDOUT, &fixup_bytes(&layout, parser.screen(), &shell_name))?;
                dirty = false;
            }
        }

        if stdin_ready {
            match read_some(STDIN, &mut buf) {
                Ok(0) => stdin_open = false,
                Ok(len) => write_all(master, &buf[..len])?,
                Err(_) => stdin_open = false,
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
    Ok(st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0)))
}
