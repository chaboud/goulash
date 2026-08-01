use goulash::{config, configcli, session};

use std::os::fd::AsRawFd;
use std::process::ExitCode;

const USAGE: &str = "usage: goulash [shell [args...]]

Wraps an interactive shell in the goulash overlay: the shell is still
yours, with a status band under it that answers questions and suggests
commands. goulash never runs anything on your behalf \u{2014} a suggestion
lands on your prompt line for you to edit and press Enter.

With no arguments it runs $SHELL (falling back to /bin/sh). Name a
shell to use that one instead:

  goulash                 wrap $SHELL
  goulash bash            wrap bash
  goulash zsh -f          wrap zsh, passing it -f

Options:
  --config [SUBCOMMAND]   read or edit settings; --config --help for
                          the subcommands (print, path, set, reset)
  --version, -V           print the version
  --help, -h              this text

Inside a session, everything starts with `#`:

  #<question>             ask; the answer lands in the band
  #? <question>           ask the slow lane to research it
  \u{2193} / \u{2191}                   walk suggestions below the prompt
  #/help                  the full reference, filterable
  #/settings              live-tune every setting

Needs a local model server \u{2014} ollama or an OpenAI-compatible one such
as LM Studio \u{2014} which goulash finds by itself. Config lives at
~/.goulash/config.toml and is optional; every setting has a working
default, and without a server goulash is still a working shell.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Before the tty check: editing settings is exactly what you want to
    // do from a script, a Dockerfile, or a machine you have only ssh'd
    // into to fix something.
    if let Some(i) = args.iter().position(|a| a == "--config") {
        return ExitCode::from(configcli::run(&args[i + 1..]) as u8);
    }
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
