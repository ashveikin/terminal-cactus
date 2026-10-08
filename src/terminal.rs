//! Hand-rolled terminal handling in the ttfx style: direct libc symbols via
//! `unsafe extern`, raw ANSI sequences, no ncurses and no crates.
//!
//! ncurses-behavior mapping used by cbonsai:
//! - `noecho` + `cbreak`: termios with ECHO and ICANON cleared, ISIG kept.
//! - `curs_set(0)`: `\e[?25l`, restored by `\e[?25h`.
//! - `initscr`/`endwin`: alternate screen enter/leave.
//! - LINES/COLUMNS env vars override the terminal size, exactly like
//!   ncurses (the parity harness pins the size through them).
//! - SIGWINCH is recorded and consumed from the run loop (ttfx's
//!   `take_terminal_resize` pattern).

use std::io::{Read, Write};

use crate::wcwidth::wcwidth;
use std::sync::atomic::{AtomicBool, Ordering};

const TIOCGWINSZ: u64 = 0x5413;
const SIGWINCH: i32 = 28;
const SIG_DFL: usize = 0;

#[repr(C)]
#[derive(Clone, Copy)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; 32],
    c_ispeed: u32,
    c_ospeed: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 0x001;
const TCLFLAG_ECHO: u32 = 0o0000010;
const TCLFLAG_ICANON: u32 = 0o0000002;
const TCSANOW: i32 = 0;

unsafe extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn tcgetattr(fd: i32, termios_p: *mut Termios) -> i32;
    fn tcsetattr(fd: i32, optional_actions: i32, termios_p: *const Termios) -> i32;
    fn isatty(fd: i32) -> i32;
    fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
    fn signal(signum: i32, handler: usize) -> usize;
    fn dup2(oldfd: i32, newfd: i32) -> i32;
    fn open(path: *const std::ffi::c_char, oflag: i32, ...) -> i32;
    fn close(fd: i32) -> i32;
}

static TERMINAL_RESIZED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigwinch(_: i32) {
    TERMINAL_RESIZED.store(true, Ordering::SeqCst);
}

/// cbonsai installs its SIGWINCH marker before ncurses runs; rbonsai keeps
/// the ttfx flag pattern.
pub fn install_sigwinch_handler() {
    // SAFETY: signal(2) with a signal-safe handler that only stores a flag.
    unsafe {
        signal(SIGWINCH, handle_sigwinch as *const () as usize);
    }
}

pub fn take_terminal_resize() -> bool {
    TERMINAL_RESIZED.swap(false, Ordering::SeqCst)
}

/// Terminal size with ncurses precedence: LINES/COLUMNS env first, then the
/// ioctl, then 80x24.
pub fn terminal_size() -> (i32, i32) {
    if let (Ok(l), Ok(c)) = (
        std::env::var("LINES").map(|v| v.trim().parse::<i32>()),
        std::env::var("COLUMNS").map(|v| v.trim().parse::<i32>()),
    ) {
        if let (Ok(l), Ok(c)) = (l, c) {
            if l > 0 && c > 0 {
                return (l, c);
            }
        }
    }
    let mut ws = Winsize { ws_row: 0, ws_col: 0, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: ws is a valid TIOCGWINSZ target.
    let ok = unsafe { ioctl(1, TIOCGWINSZ, &mut ws as *mut Winsize) } == 0;
    if ok && ws.ws_row > 0 && ws.ws_col > 0 {
        return (i32::from(ws.ws_row), i32::from(ws.ws_col));
    }
    (24, 80)
}

/// The `COLORS` value ncurses reports for the current TERM, reduced to the
/// distinctions cbonsai makes (has_colors, < 256 or not).
pub fn colors_for_term() -> i32 {
    match std::env::var("TERM").as_deref() {
        Ok(t) => {
            if t.contains("256color") || t.contains("direct") || t.ends_with("truecolor") {
                256
            } else if t == "dumb" || t.is_empty() {
                0
            } else {
                8
            }
        }
        Err(_) => 0,
    }
}

pub struct Terminal {
    saved_termios: Option<Termios>,
    out: std::io::BufWriter<std::io::Stdout>,
    pending: Vec<u8>,
    pub colors: i32,
    /// The user's `-k` color list, resolved by the caller before rendering;
    /// pairs 1..=4 resolve through it.
    pub user_colors: [i32; 4],
}

impl Terminal {
    pub fn new() -> Terminal {
        Terminal {
            saved_termios: None,
            out: std::io::BufWriter::with_capacity(1 << 16, std::io::stdout()),
            pending: Vec::new(),
            colors: colors_for_term(),
            user_colors: [2, 3, 10, 11],
        }
    }

    pub fn is_tty(&self) -> bool {
        // SAFETY: constant fd, no preconditions.
        unsafe { isatty(0) == 1 }
    }

    /// `savetty` + `initscr` + noecho/cbreak/curs_set(0).
    pub fn init(&mut self) {
        if self.saved_termios.is_none() && self.is_tty() {
            let mut t = unsafe { std::mem::zeroed::<Termios>() };
            // SAFETY: t is a valid tcgetattr target.
            if unsafe { tcgetattr(0, &mut t) } == 0 {
                self.saved_termios = Some(t);
                t.c_lflag &= !(TCLFLAG_ECHO | TCLFLAG_ICANON);
                // SAFETY: t came from tcgetattr.
                unsafe { tcsetattr(0, TCSANOW, &t) };
            }
        }
        let _ = self.out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
        let _ = self.out.flush();
    }

    /// `endwin`: restore cursor and terminal.
    pub fn endwin(&mut self) {
        let _ = self.out.write_all(b"\x1b[?25h\x1b[?1049l");
        let _ = self.out.flush();
        if let Some(t) = self.saved_termios.take() {
            // SAFETY: t was captured by tcgetattr.
            unsafe { tcsetattr(0, TCSANOW, &t) };
        }
    }

    /// Full-screen repaint of a frame snapshot.
    pub fn render(&mut self, rows: i32, cols: i32, cells: &[Cell]) {
        let mut buf: Vec<u8> = Vec::with_capacity(cells.len() * 6);
        buf.extend_from_slice(b"\x1b[H");
        let mut last_bold = false;
        let mut last_pair: Option<u8> = None;
        for row in 0..rows {
            if row > 0 {
                buf.extend_from_slice(b"\x1b[K");
                // move to the next row explicitly (avoids scroll at the last row)
                buf.extend_from_slice(format!("\x1b[{};1H", row + 1).as_bytes());
            }
            let mut col = 0i32;
            while col < cols {
                let cell = cells[(row * cols + col) as usize];
                if cell.bold != last_bold {
                    buf.extend_from_slice(if cell.bold { b"\x1b[1m" } else { b"\x1b[22m" });
                    last_bold = cell.bold;
                }
                if last_pair != Some(cell.pair) {
                    if let Some(sgr) = crate::output::pair_sgr(cell.pair, &self.user_colors, self.colors) {
                        buf.extend_from_slice(sgr.as_bytes());
                    }
                    last_pair = Some(cell.pair);
                }
                if cell.cp != 0 {
                    crate::utf8::encode_utf8(cell.cp, &mut buf);
                    // wide character: the terminal advances two columns
                    col += wcwidth(cell.cp).max(1) - 1;
                } else {
                    buf.push(b' ');
                }
                col += 1;
            }
        }
        buf.extend_from_slice(b"\x1b[K");
        let _ = self.out.write_all(&buf);
        let _ = self.out.flush();
    }

    /// Non-blocking key read (`nodelay(stdscr, TRUE)` + `wgetch`).
    /// Returns None for ERR (no input), Some(None-ish variants below).
    pub fn getch_nodelay(&mut self) -> Option<Option<u8>> {
        if self.drain_pending() {
            return Some(self.pending_key());
        }
        if !self.wait_readable(0) {
            return None;
        }
        let mut buf = [0u8; 64];
        let n = std::io::stdin().read(&mut buf).unwrap_or(0);
        if n == 0 {
            return Some(None); // EOF behaves like ERR for cbonsai's checks
        }
        self.pending.extend_from_slice(&buf[..n]);
        Some(self.pending_key())
    }

    /// Blocking read with timeout in ms (`timeout(ms)` + `wgetch`).
    pub fn getch_timeout(&mut self, timeout_ms: i32) -> Option<Option<u8>> {
        if self.drain_pending() {
            return Some(self.pending_key());
        }
        if !self.wait_readable(timeout_ms) {
            return None;
        }
        let mut buf = [0u8; 64];
        let n = std::io::stdin().read(&mut buf).unwrap_or(0);
        if n == 0 {
            return Some(None);
        }
        self.pending.extend_from_slice(&buf[..n]);
        Some(self.pending_key())
    }

    /// Blocking read without timeout (`nodelay(FALSE)` + `wgetch`).
    pub fn getch_blocking(&mut self) -> Option<u8> {
        loop {
            if self.drain_pending() {
                if let Some(k) = self.pending_key() {
                    return Some(k);
                }
                return None;
            }
            if !self.wait_readable(-1) {
                return None;
            }
            let mut buf = [0u8; 64];
            let n = std::io::stdin().read(&mut buf).unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.pending.extend_from_slice(&buf[..n]);
        }
    }

    /// Collapse escape sequences into a single key event so arrow keys act
    /// as "any key" like ncurses' keypad translation.
    fn pending_key(&mut self) -> Option<u8> {
        if self.pending.is_empty() {
            return None;
        }
        let b = self.pending.remove(0);
        if b == 0x1b {
            // consume CSI / SS3 introducers as one key
            if self.pending.first().copied() == Some(b'[') {
                while let Some(&c) = self.pending.first() {
                    self.pending.remove(0);
                    if (0x40..=0x7e).contains(&c) {
                        break;
                    }
                }
            } else {
                if !self.pending.is_empty() {
                    self.pending.remove(0);
                }
            }
            return Some(0x1b);
        }
        Some(b)
    }

    fn drain_pending(&mut self) -> bool {
        !self.pending.is_empty()
    }

    fn wait_readable(&self, timeout_ms: i32) -> bool {
        let mut fds = PollFd { fd: 0, events: POLLIN, revents: 0 };
        // SAFETY: one valid PollFd.
        let rc = unsafe { poll(&mut fds, 1, timeout_ms) };
        // EINTR (resize) and timeout both report "no key"
        rc > 0 && (fds.revents & POLLIN) != 0
    }

    /// `freopen(ctermid(NULL), "r", stdin)` for `-T`: reopen /dev/tty on fd 0.
    pub fn reopen_stdin_tty(&mut self) -> bool {
        let path = b"/dev/tty\0";
        // SAFETY: path is NUL-terminated; O_RDONLY is a valid open mode.
        let fd = unsafe { open(path.as_ptr().cast(), 0) };
        if fd < 0 {
            return false;
        }
        // SAFETY: both fds are open.
        let ok = unsafe { dup2(fd, 0) } == 0;
        if fd != 0 {
            unsafe { close(fd) };
        }
        ok
    }

}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

/// `stty`-free SIGPIPE posture: cbonsai inherits the default, Rust ignores
/// SIGPIPE; restoring it keeps `rbonsai | head` behaving like cbonsai.
pub fn restore_sigpipe() {
    const SIGPIPE: i32 = 13;
    // SAFETY: signal(2) with SIG_DFL.
    unsafe {
        signal(SIGPIPE, SIG_DFL);
    }
}

pub type Cell = crate::screen::Cell;
