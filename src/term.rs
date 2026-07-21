use nix::sys::termios::{self, SetArg, Termios};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

pub fn get_size(fd: RawFd) -> std::io::Result<Size> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ reads into a properly sized winsize struct.
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Size {
        rows: ws.ws_row,
        cols: ws.ws_col,
    })
}

pub fn set_size(fd: RawFd, size: Size) -> std::io::Result<()> {
    let ws = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ reads from a properly sized winsize struct.
    if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Puts the real terminal into raw mode; restores the original settings on drop.
pub struct RawGuard {
    fd: RawFd,
    orig: Termios,
}

impl RawGuard {
    pub fn new(fd: impl AsFd) -> nix::Result<Self> {
        let raw_fd = fd.as_fd().as_raw_fd();
        let orig = termios::tcgetattr(fd.as_fd())?;
        let mut raw = orig.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(fd.as_fd(), SetArg::TCSANOW, &raw)?;
        Ok(Self { fd: raw_fd, orig })
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        // SAFETY: fd was valid at construction and outlives the session loop.
        let fd = unsafe { BorrowedFd::borrow_raw(self.fd) };
        let _ = termios::tcsetattr(fd, SetArg::TCSANOW, &self.orig);
    }
}
