mod config;
mod osc;
mod pty;
mod record;
mod sense;
mod session;
mod status;
mod term;
mod vendor;

use std::os::fd::AsRawFd;
use std::process::ExitCode;

const USAGE: &str = "usage: goulash [shell [args...]]

Wraps an interactive shell in the goulash overlay. With no arguments,
runs $SHELL (falling back to /bin/sh). Config: ~/.goulash/config.toml";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("goulash {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let argv = if args.is_empty() {
        vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())]
    } else {
        args
    };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let is_tty =
        unsafe { libc::isatty(stdin.as_raw_fd()) == 1 && libc::isatty(stdout.as_raw_fd()) == 1 };
    if !is_tty {
        eprintln!("goulash: stdin and stdout must be a terminal");
        return ExitCode::from(2);
    }

    let cfg = config::Config::load();
    match session::run(&cfg, argv) {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(err) => {
            eprintln!("goulash: {err}");
            ExitCode::from(1)
        }
    }
}
