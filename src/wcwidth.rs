//! glibc-compatible `wcwidth` / `iswspace` / `iswcntrl`.
//!
//! The tables in [`crate::wcwidth_table`] are generated from the host libc by
//! `tools/parity/gen_wcwidth.c`; cbonsai's leaf-alignment gate and message
//! layout inherit glibc's exact width decisions through them.

use crate::wcwidth_table::{CNTRL_RANGES, SPACE_RANGES, WIDTH_RANGES};

/// glibc `wcwidth`. Returns the column width, 0 for combining characters and
/// -1 for non-printable ones.
pub fn wcwidth(cp: u32) -> i32 {
    let mut lo = 0usize;
    let mut hi = WIDTH_RANGES.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let (start, end, w) = WIDTH_RANGES[mid];
        if cp < start {
            hi = mid;
        } else if cp > end {
            lo = mid + 1;
        } else {
            return w as i32;
        }
    }
    // Every codepoint 0..=0x10FFFF is covered by the generated table.
    unreachable!("wcwidth table gap at cp {cp:#x}");
}

pub fn iswspace(cp: u32) -> bool {
    in_ranges(cp, &SPACE_RANGES)
}

pub fn iswcntrl(cp: u32) -> bool {
    in_ranges(cp, &CNTRL_RANGES)
}

fn in_ranges(cp: u32, ranges: &[(u32, u32)]) -> bool {
    let mut lo = 0usize;
    let mut hi = ranges.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let (start, end) = ranges[mid];
        if cp < start {
            hi = mid;
        } else if cp > end {
            lo = mid + 1;
        } else {
            return true;
        }
    }
    false
}
