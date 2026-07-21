use nix::sys::termios::{self, LocalFlags};
use std::os::fd::{BorrowedFd, RawFd};

/// Who currently owns the wrapped terminal's keyboard, per job control,
/// plus the termios flags that matter for observation policy.
///
/// This is gate 1 (and the echo half of the privacy invariant) from the
/// input-ownership design: `fg_shell` distinguishes the shell itself from
/// a foreground child (pipeline, TUI, ssh, ...); `echo=false` means
/// secret entry — typed input must never be recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    pub fg_shell: bool,
    pub echo: bool,
    pub icanon: bool,
    pub alt_screen: bool,
}

impl State {
    /// Short label for the status row.
    pub fn label(&self) -> &'static str {
        if !self.fg_shell {
            if self.alt_screen { "tui" } else { "run" }
        } else if !self.echo {
            "secret"
        } else {
            "shell"
        }
    }
}

pub struct Sensor {
    master: RawFd,
    shell_pgid: libc::pid_t,
}

impl Sensor {
    pub fn new(master: RawFd, shell_pid: u32) -> Self {
        // The shell was made a session leader in pre_exec, so its process
        // group id equals its pid.
        Self {
            master,
            shell_pgid: shell_pid as libc::pid_t,
        }
    }

    pub fn read(&self, alt_screen: bool) -> State {
        // SAFETY: master is open for the session's lifetime.
        let fg = unsafe { libc::tcgetpgrp(self.master) };
        let fg_shell = fg == self.shell_pgid;
        let (echo, icanon) = {
            // SAFETY: same fd validity argument.
            let fd = unsafe { BorrowedFd::borrow_raw(self.master) };
            match termios::tcgetattr(fd) {
                Ok(t) => (
                    t.local_flags.contains(LocalFlags::ECHO),
                    t.local_flags.contains(LocalFlags::ICANON),
                ),
                Err(_) => (true, true),
            }
        };
        State {
            fg_shell,
            echo,
            icanon,
            alt_screen,
        }
    }
}
