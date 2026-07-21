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
    ///
    /// Line editors (zle, readline) put the terminal in non-canonical
    /// mode with ECHO off and echo keystrokes themselves — so
    /// `fg_shell && !icanon` reads as the prompt being active, and
    /// "secret" is reserved for canonical-mode echo-off (sudo, `read -s`),
    /// where the tty would normally echo but was told not to.
    pub fn label(&self) -> &'static str {
        if !self.fg_shell {
            if self.alt_screen { "tui" } else { "run" }
        } else if !self.icanon {
            "prompt"
        } else if !self.echo {
            "secret"
        } else {
            "shell"
        }
    }
}

/// Where the shell integration says we are, when hooks are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPhase {
    Prompt,
    Command,
}

/// Status-row label combining job-control/termios sensing with the
/// shell-hook phase (authoritative when present, since hooks distinguish
/// prompt from a builtin-running state that termios alone cannot).
pub fn label(st: &State, hook: Option<HookPhase>) -> &'static str {
    if !st.fg_shell {
        return if st.alt_screen { "tui" } else { "run" };
    }
    if st.icanon && !st.echo {
        return "secret";
    }
    match hook {
        Some(HookPhase::Prompt) => "prompt",
        Some(HookPhase::Command) => "cmd",
        None => st.label(),
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

#[cfg(test)]
mod tests {
    use super::State;

    fn st(fg_shell: bool, echo: bool, icanon: bool, alt_screen: bool) -> State {
        State {
            fg_shell,
            echo,
            icanon,
            alt_screen,
        }
    }

    #[test]
    fn labels() {
        // zle/readline at an idle prompt: raw-ish mode, echoes itself
        assert_eq!(st(true, false, false, false).label(), "prompt");
        // sudo / read -s: canonical mode, echo suppressed
        assert_eq!(st(true, false, true, false).label(), "secret");
        // builtin or script reading normally
        assert_eq!(st(true, true, true, false).label(), "shell");
        // external command in the foreground
        assert_eq!(st(false, true, true, false).label(), "run");
        // full-screen app
        assert_eq!(st(false, false, false, true).label(), "tui");
    }
}
