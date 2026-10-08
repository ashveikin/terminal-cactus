//! The ncurses-equivalent screen model: cells, windows, panel stacking.
//!
//! Faithfulness notes (all validated against the reference shim's frames):
//! - Cells carry the attributes *at write time*; a window's current
//!   bold/pair state is applied per character (`wattron` semantics: bold ORs,
//!   the color pair replaces).
//! - Wide characters occupy two cells, both holding the codepoint — ncurses
//!   duplicates them, which `overlay` copies cell-by-cell and the
//!   printstdscr walk skips by width.
//! - `waddch` wraps at the window edge; at the bottom-right the cursor stays
//!   and the next character is dropped (scrollok is always false here).
//! - `waddch('\n')` clears to end-of-line (documented ncurses behavior).
//! - The composite is a bottom-to-top full-rect merge of the panel windows;
//!   the panel library's changed-line fixups reduce to exactly this merge
//!   for windows that never move.
//! - `stdscr` is a separate layer: `refresh()` copies only its *changed*
//!   lines over the composite (an untouched stdscr does not blank the
//!   screen — verified frame 1 of the reference dump). `wgetch` internally
//!   flushes a modified window before reading input; that behavior is
//!   mirrored by [`Screen::flush_touched`].

use crate::wcwidth::wcwidth;
use crate::utf8::{decode_one, DecodeError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub cp: u32,
    pub bold: bool,
    pub pair: u8,
}

impl Cell {
    pub const BLANK: Cell = Cell { cp: b' ' as u32, bold: false, pair: 0 };

}

#[derive(Clone)]
pub struct Window {
    /// Absolute position on the terminal (ncurses `newwin(begin_y, begin_x)`).
    pub y: i32,
    pub x: i32,
    pub h: i32,
    pub w: i32,
    cells: Vec<Cell>,
    pub cy: i32,
    pub cx: i32,
    /// Pending-wrap state at the bottom-right corner.
    pending_wrap: bool,
    /// Current attribute state (`wattron`/`wattroff`).
    pub bold: bool,
    pub pair: u8,
}

impl Window {
    /// `newwin(num_lines, num_columns, begin_y, begin_x)` with ncurses
    /// semantics: zero dimensions mean "to the bottom/right of the screen",
    /// negative or beyond-screen origins (and negative dimensions) yield
    /// NULL; sizes exceeding the screen are clamped (merge-side clipping is
    /// equivalent, but the verbosity row checks read the clamped height).
    pub fn newwin(h: i32, w: i32, y: i32, x: i32, rows: i32, cols: i32) -> Option<Window> {
        if y < 0 || x < 0 || y >= rows || x >= cols || h < 0 || w < 0 {
            return None;
        }
        let h = if h == 0 { rows - y } else { h };
        let w = if w == 0 { cols - x } else { w };
        let h = h.min(rows - y);
        let w = w.min(cols - x);
        Some(Window {
            y,
            x,
            h,
            w,
            cells: vec![Cell::BLANK; (h as usize) * (w as usize)],
            cy: 0,
            cx: 0,
            pending_wrap: false,
            bold: false,
            pair: 0,
        })
    }

    /// A detached window (stdscr layers); never NULL in this program.
    pub fn new(h: i32, w: i32, y: i32, x: i32) -> Window {
        let h = h.max(0);
        let w = w.max(0);
        Window {
            y,
            x,
            h,
            w,
            cells: vec![Cell::BLANK; (h as usize) * (w as usize)],
            cy: 0,
            cx: 0,
            pending_wrap: false,
            bold: false,
            pair: 0,
        }
    }

    pub fn get(&self, row: i32, col: i32) -> Cell {
        if row < 0 || col < 0 || row >= self.h || col >= self.w {
            return Cell::BLANK;
        }
        self.cells[(row * self.w + col) as usize]
    }

    fn put(&mut self, row: i32, col: i32, cell: Cell) {
        if row < 0 || col < 0 || row >= self.h || col >= self.w {
            return;
        }
        let idx = (row * self.w + col) as usize;
        if idx < self.cells.len() {
            self.cells[idx] = cell;
        }
    }

    /// `wmove`: false when out of bounds (ncurses returns ERR).
    pub fn wmove(&mut self, row: i32, col: i32) -> bool {
        if row < 0 || col < 0 || row >= self.h || col >= self.w {
            return false;
        }
        self.cy = row;
        self.cx = col;
        self.pending_wrap = false;
        true
    }

    fn current_cell(&self, cp: u32) -> Cell {
        Cell { cp, bold: self.bold, pair: self.pair }
    }

    /// `waddch` for one codepoint (width already known by the caller for
    /// wide characters — see [`Window::addch_wide`]).
    pub fn addch(&mut self, cp: u32) -> bool {
        match cp {
            0x0A => {
                // newline: clear to end of line, move to start of next line
                self.clrtoeol();
                if self.cy + 1 < self.h {
                    self.cy += 1;
                    self.cx = 0;
                    self.pending_wrap = false;
                    true
                } else {
                    false
                }
            }
            0x0D => {
                self.cx = 0;
                self.pending_wrap = false;
                true
            }
            0x09 => {
                // tab: advance to the next multiple of 8 (cursor only)
                let target = (self.cx / 8 + 1) * 8;
                self.cx = target.min(self.w.max(1) - 1);
                self.pending_wrap = false;
                true
            }
            c if c < 0x20 || c == 0x7F => {
                // control characters render as ^X (two cells)
                let caret = if c == 0x7F { b'?' as u32 } else { c + (b'@' as u32) };
                self.addch_plain(b'^' as u32, 1) && self.addch_plain(caret, 1)
            }
            c => self.addch_wide(c, wcwidth(c).max(1) as i32),
        }
    }

    fn addch_plain(&mut self, cp: u32, width: i32) -> bool {
        self.addch_wide(cp, width)
    }

    /// Place one character of the given column width with wrap semantics.
    pub fn addch_wide(&mut self, cp: u32, width: i32) -> bool {
        if width < 1 {
            // combining characters attach to the previous cell in ncurses;
            // the parity dump only reads the base char, so they are dropped
            return true;
        }
        if self.pending_wrap {
            return false;
        }
        if self.cx + width > self.w {
            // wrap to the next line if possible, else drop
            if self.cy + 1 < self.h && self.w >= width {
                self.cy += 1;
                self.cx = 0;
            } else {
                return false;
            }
        }
        let cell = self.current_cell(cp);
        self.put(self.cy, self.cx, cell);
        for k in 1..width {
            // continuation cells hold the same codepoint (ncurses behavior)
            self.put(self.cy, self.cx + k, cell);
        }
        self.cx += width;
        if self.cx >= self.w {
            if self.cy + 1 < self.h {
                self.cy += 1;
                self.cx = 0;
            } else {
                self.cx = self.w - 1;
                self.pending_wrap = true;
            }
        }
        true
    }

    /// `waddstr` over raw bytes: decode UTF-8; stop at the first invalid or
    /// incomplete sequence (ncurses' waddnstr returns ERR there).
    pub fn addstr_bytes(&mut self, bytes: &[u8]) {
        let mut cursor = 0;
        while cursor < bytes.len() {
            match decode_one(&bytes[cursor..]) {
                Ok((cp, len)) => {
                    if !self.addch(cp) {
                        return;
                    }
                    cursor += len;
                }
                Err(DecodeError::Invalid) | Err(DecodeError::Incomplete) => return,
            }
        }
    }

    /// `waddwstr` over decoded codepoints (used by the message renderer).
    pub fn addwstr(&mut self, chars: &[u32]) {
        for &cp in chars {
            if !self.addch(cp) {
                return;
            }
        }
    }

    pub fn clrtoeol(&mut self) {
        for col in self.cx..self.w {
            self.put(self.cy, col, Cell::BLANK);
        }
    }

    /// `wattr_on(win, ...)`: bold Some→set, None→untouched; the color pair
    /// replaces when present.
    pub fn attr_on(&mut self, bold: Option<bool>, pair: Option<u8>) {
        if let Some(b) = bold {
            self.bold = b;
        }
        if let Some(p) = pair {
            self.pair = p;
        }
    }

    /// `wattr_off(win, A_BOLD)`.
    pub fn attr_off_bold(&mut self) {
        self.bold = false;
    }

    /// `wborder(win, '|', '|', '-', '-', '+', '+', '+', '+')` with the
    /// current attributes.
    pub fn border(&mut self) {
        let attrs = Cell { cp: 0, bold: self.bold, pair: self.pair };
        for col in 0..self.w {
            self.put(0, col, Cell { cp: b'-' as u32, ..attrs });
            if self.h > 1 {
                self.put(self.h - 1, col, Cell { cp: b'-' as u32, ..attrs });
            }
        }
        for row in 0..self.h {
            self.put(row, 0, Cell { cp: b'|' as u32, ..attrs });
            if self.w > 1 {
                self.put(row, self.w - 1, Cell { cp: b'|' as u32, ..attrs });
            }
        }
        let corner = Cell { cp: b'+' as u32, ..attrs };
        self.put(0, 0, corner);
        self.put(0, self.w - 1, corner);
        self.put(self.h - 1, 0, corner);
        self.put(self.h - 1, self.w - 1, corner);
    }
}

/// The virtual screen (`newscr`) plus the stdscr change-tracking layer.
pub struct Screen {
    pub rows: i32,
    pub cols: i32,
    pub cells: Vec<Cell>,
    /// stdscr as its own window.
    pub stdscr: Window,
    /// Lines of stdscr modified since the last flush (`wnoutrefresh`).
    stdscr_touched: Vec<bool>,
    stdscr_touched_any: bool,
}

impl Screen {
    pub fn new(rows: i32, cols: i32) -> Screen {
        Screen {
            rows,
            cols,
            cells: vec![Cell::BLANK; (rows.max(0) as usize) * (cols.max(0) as usize)],
            stdscr: Window::new(rows, cols, 0, 0),
            stdscr_touched: vec![false; rows.max(0) as usize],
            stdscr_touched_any: false,
        }
    }

    /// `wnoutrefresh(win)`: copy the window's lines into the virtual screen.
    fn merge_window(&mut self, win: &Window) {
        for row in 0..win.h {
            let sy = win.y + row;
            if sy < 0 || sy >= self.rows {
                continue;
            }
            for col in 0..win.w {
                let sx = win.x + col;
                if sx < 0 || sx >= self.cols {
                    continue;
                }
                self.cells[(sy * self.cols + sx) as usize] = win.get(row, col);
            }
        }
    }

    /// `update_panels(); doupdate();` — composite in creation order (the
    /// panel stack is never reordered by cbonsai) and return the result as
    /// a snapshot for the parity dump / renderer.
    pub fn update_panels_doupdate(&mut self, panels: &[&Window]) -> Vec<Cell> {
        for win in panels {
            self.merge_window(win);
        }
        self.cells.clone()
    }

    /// Writes to stdscr mark lines as changed (ncurses touches them).
    pub fn stdscr_write_mark(&mut self, row: i32) {
        if row >= 0 && row < self.rows {
            let r = row as usize;
            if r < self.stdscr_touched.len() {
                self.stdscr_touched[r] = true;
                self.stdscr_touched_any = true;
            }
        }
    }

    /// `refresh()`: copy only stdscr's changed lines over the composite.
    pub fn refresh_snapshot(&mut self, panels: &[&Window]) -> Vec<Cell> {
        for win in panels {
            self.merge_window(win);
        }
        for row in 0..self.rows {
            let r = row as usize;
            if self.stdscr_touched.get(r).copied().unwrap_or(false) {
                for col in 0..self.cols {
                    self.cells[(row * self.cols + col) as usize] = self.stdscr.get(row, col);
                }
                if let Some(t) = self.stdscr_touched.get_mut(r) {
                    *t = false;
                }
            }
        }
        self.cells.clone()
    }

    /// `erase()` on stdscr: blank it and mark every line touched.
    pub fn stdscr_erase(&mut self) {
        self.stdscr = Window::new(self.rows, self.cols, 0, 0);
        for t in self.stdscr_touched.iter_mut() {
            *t = true;
        }
        self.stdscr_touched_any = true;
    }

    /// ncurses' `wgetch` refreshes a modified window before reading input.
    /// cbonsai checks for keys on `stdscr` during growth; the flush happens
    /// once per modification and never emits a frame of its own (the
    /// internal doupdate is not interposable).
    pub fn flush_touched(&mut self) {
        if !self.stdscr_touched_any {
            return;
        }
        self.stdscr_touched_any = false;
        for row in 0..self.rows {
            let r = row as usize;
            if self.stdscr_touched.get(r).copied().unwrap_or(false) {
                for col in 0..self.cols {
                    self.cells[(row * self.cols + col) as usize] = self.stdscr.get(row, col);
                }
                if let Some(t) = self.stdscr_touched.get_mut(r) {
                    *t = false;
                }
            }
        }
    }

    /// `overlay(src, dst)`: copy non-blank cells. ncurses' is_blank tests
    /// the character only — a space keeps its attributes out of the copy
    /// (base-art rows print with attributes in the frame dump yet stay
    /// blank through overlay), which the print path depends on.
    pub fn overlay(&mut self, src: &Window) {
        for row in 0..src.h {
            let sy = src.y + row;
            if sy < 0 || sy >= self.rows {
                continue;
            }
            for col in 0..src.w {
                let sx = src.x + col;
                if sx < 0 || sx >= self.cols {
                    continue;
                }
                let cell = src.get(row, col);
                if cell.cp != b' ' as u32 {
                    self.stdscr.put(sy, sx, cell);
                    self.stdscr_write_mark(sy);
                }
            }
        }
    }

    /// `overwrite(src, dst)`: copy all cells.
    pub fn overwrite(&mut self, src: &Window) {
        for row in 0..src.h {
            let sy = src.y + row;
            if sy < 0 || sy >= self.rows {
                continue;
            }
            for col in 0..src.w {
                let sx = src.x + col;
                if sx < 0 || sx >= self.cols {
                    continue;
                }
                let cell = src.get(row, col);
                self.stdscr.put(sy, sx, cell);
                self.stdscr_write_mark(sy);
            }
        }
    }

    pub fn resize(&mut self, rows: i32, cols: i32) {
        self.rows = rows;
        self.cols = cols;
        self.cells = vec![Cell::BLANK; (rows.max(0) as usize) * (cols.max(0) as usize)];
        self.stdscr = Window::new(rows, cols, 0, 0);
        self.stdscr_touched = vec![false; rows.max(0) as usize];
        self.stdscr_touched_any = true; // resize touched everything via erase
    }
}
