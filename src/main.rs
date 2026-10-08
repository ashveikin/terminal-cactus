//! tcac — a beautifully random cactus generator, dependency-free like the
//! rbonsai engine it descends from.
//!
//! Terminal handling is hand-rolled ANSI + termios (no ncurses); the growth
//! RNG is a glibc rand() clone pinned by golden tests, so every seed grows
//! the same cactus every time.

mod bloom;
mod cactus;
mod config;
mod ctx;
mod getopt;
mod message;
mod numparse;
mod output;
mod rand;
mod save;
mod screen;
mod terminal;
mod utf8;
mod wcwidth;
mod wcwidth_table;

use config::{Config, ParseOutcome};
use ctx::Ctx;

pub fn time_now() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

fn flush_out(stdout_out: Vec<u8>, stderr_out: Vec<u8>) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(&stdout_out);
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().write_all(&stderr_out);
    let _ = std::io::stderr().flush();
}

fn main() {
    terminal::restore_sigpipe();
    terminal::install_sigwinch_handler();

    let argv: Vec<Vec<u8>> = std::env::args_os()
        .map(|a| {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStringExt;
                a.into_vec()
            }
            #[cfg(not(unix))]
            {
                a.to_string_lossy().into_owned().into_bytes()
            }
        })
        .collect();

    let mut conf = Config::new();

    let mut stdout_out = Vec::new();
    let mut stderr_out = Vec::new();

    match config::parse_args(argv, &mut conf, &mut stderr_out) {
        ParseOutcome::Help => {
            config::print_help(&mut stdout_out);
            flush_out(stdout_out, stderr_out);
            std::process::exit(ctx::EXIT_OK);
        }
        ParseOutcome::Error => {
            flush_out(stdout_out, stderr_out);
            std::process::exit(ctx::EXIT_ERR);
        }
        ParseOutcome::Run => {}
    }

    // -T: freopen the controlling tty for input after slurping stdin
    if conf.reopen_stdin {
        let mut term = terminal::Terminal::new();
        if !term.reopen_stdin_tty() {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            stderr_out.extend_from_slice(
                format!("error: freopen process tty: {}\n", strerror_glibc(errno)).as_bytes(),
            );
            flush_out(stdout_out, stderr_out);
            std::process::exit(ctx::EXIT_ERR);
        }
    }

    if conf.load {
        if let Some((seed, count)) = save::load_from_file(&conf.load_file, &mut stderr_out) {
            conf.seed = seed;
            conf.target_steps = count;
        }
    }

    // seed the random number generator
    if conf.seed == 0 {
        conf.seed = time_now() as i32;
    }

    let mut app = Ctx::new(conf, stdout_out, stderr_out);
    app.run();
}

/// strerror text for the one perror()-style call (freopen of the controlling
/// tty for -T). Only errnos open(2) can produce there.
fn strerror_glibc(errno: i32) -> &'static str {
    match errno {
        2 => "No such file or directory",
        6 => "No such device or address",
        13 => "Permission denied",
        24 => "Too many open files",
        _ => "Unknown error",
    }
}
