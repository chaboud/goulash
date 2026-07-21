use crate::term::Size;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

pub struct Pty {
    pub master: OwnedFd,
    pub child: Child,
}

/// Allocate a PTY sized `size` (already shrunk by the reserved rows) and
/// spawn `argv` on the slave side as a controlling-terminal session leader.
pub fn spawn(argv: &[String], size: Size) -> io::Result<Pty> {
    let ws = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ends = nix::pty::openpty(Some(&ws), None)?;
    let master = ends.master;
    let slave = ends.slave;

    // The child must not inherit the master side.
    nix::fcntl::fcntl(
        &master,
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
    )?;

    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.env("GOULASH", env!("CARGO_PKG_VERSION"));

    let slave_fd = slave.as_raw_fd();
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            // TIOCSCTTY is c_uint on Apple targets, c_ulong on Linux.
            #[allow(clippy::unnecessary_cast)]
            if libc::ioctl(slave_fd, libc::TIOCSCTTY as libc::c_ulong, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            for fd in 0..3 {
                if libc::dup2(slave_fd, fd) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            if slave_fd > 2 {
                libc::close(slave_fd);
            }
            Ok(())
        });
    }

    let child = cmd.spawn()?;
    drop(slave);
    Ok(Pty { master, child })
}
