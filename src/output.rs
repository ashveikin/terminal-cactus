//! The pair→SGR table shared by the live renderer and the `-p` print path,
//! plus `printstdscr` (ANSI dump of the composed screen).
//!
//! Color pairs:
//! - 1..=4 resolve through the user's `-k` list: dark body, dark spines,
//!   light body, light spines (restricted with %8 on 8-color terminals).
//! - 5 is the text pair for messages/borders (7 on 8-color, 8 otherwise).
//! - 6 is the terracotta pot (fixed; red on 8-color terminals).
//! - 7..=10 are the flower palette: pink, red, yellow, white.

use crate::screen::{Screen, Window};
use crate::wcwidth::wcwidth;

pub const PAIR_DARK_BODY: u8 = 1;
pub const PAIR_DARK_SPINE: u8 = 2;
pub const PAIR_LIGHT_BODY: u8 = 3;
pub const PAIR_LIGHT_SPINE: u8 = 4;
pub const PAIR_TEXT: u8 = 5;
pub const PAIR_POT: u8 = 6;
pub const PAIR_FLOWER_PINK: u8 = 7;
pub const PAIR_FLOWER_RED: u8 = 8;
pub const PAIR_FLOWER_YELLOW: u8 = 9;
pub const PAIR_FLOWER_WHITE: u8 = 10;

fn pair_fg(pair: u8, colors: &[i32; 4], term_colors: i32) -> i16 {
    match pair {
        0 => {
            if term_colors > 0 {
                -1
            } else {
                0
            }
        }
        1..=4 => {
            let c = colors[(pair as usize) - 1];
            if term_colors < 256 {
                (c % 8) as i16
            } else {
                c as i16
            }
        }
        5 => {
            if term_colors < 256 {
                7
            } else {
                8
            }
        }
        6 => {
            if term_colors >= 256 {
                130
            } else if term_colors > 0 {
                1
            } else {
                0
            }
        }
        7 => {
            if term_colors >= 256 {
                207
            } else if term_colors > 0 {
                5
            } else {
                0
            }
        }
        8 => {
            if term_colors >= 256 {
                196
            } else if term_colors > 0 {
                1
            } else {
                0
            }
        }
        9 => {
            if term_colors >= 256 {
                226
            } else if term_colors > 0 {
                3
            } else {
                0
            }
        }
        10 => {
            if term_colors >= 256 {
                231
            } else if term_colors > 0 {
                7
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// SGR sequence for a pair, for the live renderer.
pub fn pair_sgr(pair: u8, colors: &[i32; 4], term_colors: i32) -> Option<String> {
    let fg = pair_fg(pair, colors, term_colors);
    if fg <= 0 {
        return Some("\x1b[0m".to_string());
    }
    Some(
        if fg >= 16 {
            format!("\x1b[38;5;{fg}m")
        } else if fg <= 7 {
            format!("\x1b[3{fg}m")
        } else {
            format!("\x1b[9{}m", fg - 8)
        },
    )
}

/// printstdscr: walk stdscr cell by cell and emit ANSI bytes.
pub fn printstdscr(screen: &Screen, colors: &[i32; 4], term_colors: i32) -> Vec<u8> {
    let stdscr: &Window = &screen.stdscr;
    let rows = screen.rows;
    let cols = screen.cols;
    let mut out = Vec::with_capacity((rows * cols * 12) as usize);

    for y in 0..rows {
        let mut x = 0i32;
        while x < cols {
            let c = stdscr.get(y, x);

            if c.bold {
                out.extend_from_slice(b"\x1b[1m");
            } else {
                out.extend_from_slice(b"\x1b[0m");
            }

            let fg = pair_fg(c.pair, colors, term_colors);
            if fg <= 0 {
                out.extend_from_slice(b"\x1b[0m");
            } else if fg >= 16 {
                out.extend_from_slice(format!("\x1b[38;5;{fg}m").as_bytes());
            } else if fg <= 7 {
                out.extend_from_slice(format!("\x1b[3{fg}m").as_bytes());
            } else if fg >= 8 {
                out.extend_from_slice(format!("\x1b[9{}m", fg - 8).as_bytes());
            }

            crate::utf8::encode_utf8(c.cp, &mut out);

            if x == cols - 1 {
                out.push(b'\n');
            }

            let mut cwidth = wcwidth(c.cp);
            if cwidth < 0 {
                cwidth = 1;
            }
            if cwidth > 1 {
                x += cwidth - 1;
            }
            x += 1;
        }
    }

    out.extend_from_slice(b"\x1b[0m\n");
    out
}

/// Plain-text render of a composed frame (visual tools and tests).
#[cfg_attr(not(test), allow(dead_code))]
pub fn grid_text(cells: &[crate::screen::Cell], rows: i32, cols: i32) -> String {
    let mut out = String::new();
    for row in 0..rows {
        let mut col = 0i32;
        while col < cols {
            let cell = cells[(row * cols + col) as usize];
            if cell.cp == 0 {
                out.push(' ');
            } else {
                let ch = char::from_u32(cell.cp).unwrap_or('?');
                out.push(ch);
                let w = wcwidth(cell.cp).max(1);
                for _ in 1..w {
                    out.push(' ');
                    col += 1;
                }
            }
            col += 1;
        }
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
    out
}

/// `%02d`, `% 3d`, `%03d` replicas used by the verbosity overlay.
pub fn fmt_02d(v: i32) -> String {
    format!("{:02}", v)
}

pub fn fmt_03d(v: i32) -> String {
    format!("{:03}", v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printf_flag_replicas() {
        assert_eq!(fmt_02d(7), "07");
        assert_eq!(fmt_02d(123), "123");
        assert_eq!(fmt_03d(7), "007");
        assert_eq!(fmt_03d(-7), "-07");
    }

    #[test]
    fn pair_colors_on_256_and_8_color_terminals() {
        let colors = [2, 3, 10, 11];
        assert_eq!(pair_fg(3, &colors, 256), 10);
        assert_eq!(pair_fg(3, &colors, 8), 2);
        assert_eq!(pair_fg(5, &colors, 256), 8);
        assert_eq!(pair_fg(5, &colors, 8), 7);
        assert_eq!(pair_fg(0, &colors, 256), -1);
        assert_eq!(pair_fg(0, &colors, 0), 0);
    }

    #[test]
    fn fixed_pairs_have_8_color_fallbacks() {
        let colors = [2, 3, 10, 11];
        assert_eq!(pair_fg(PAIR_POT, &colors, 256), 130);
        assert_eq!(pair_fg(PAIR_POT, &colors, 8), 1);
        assert_eq!(pair_fg(PAIR_FLOWER_PINK, &colors, 256), 207);
        assert_eq!(pair_fg(PAIR_FLOWER_PINK, &colors, 8), 5);
        assert_eq!(pair_fg(PAIR_FLOWER_RED, &colors, 8), 1);
        assert_eq!(pair_fg(PAIR_FLOWER_YELLOW, &colors, 8), 3);
        assert_eq!(pair_fg(PAIR_FLOWER_WHITE, &colors, 8), 7);
    }
}
