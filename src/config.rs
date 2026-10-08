//! `struct config`, the option parser (a getopt_long consumer), and the
//! number-parsing paths the CLI behavior depends on.
//!
//! The option surface matches rbonsai/cbonsai: same short flags, long names
//! and defaults. Two cactus adaptations: `-c` picks **spine** characters
//! (botanically, cactus spines are modified leaves) and `-k`'s four color
//! slots order as dark body, dark spines, light body, light spines.
//! Edge-case handling is sanitized: `-p`/`--print` are both plain flags,
//! errors go to stderr with a nonzero exit, and degenerate numeric values
//! are rejected instead of crashing.

use crate::getopt::{GetOpt, LongOpt, Opt, HAS_ARG, NO_ARG};
use crate::numparse::{strtol, strtod};
use crate::rand::c_int_from_f64;

pub const OPTSTRING: &str = "lt:iw:Sm:T:b:g:c:k:M:L:ps:C:W:vh";

pub const LONG_OPTIONS: &[LongOpt] = &[
    LongOpt { name: "live", has_arg: NO_ARG, val: b'l' },
    LongOpt { name: "time", has_arg: HAS_ARG, val: b't' },
    LongOpt { name: "infinite", has_arg: NO_ARG, val: b'i' },
    LongOpt { name: "wait", has_arg: HAS_ARG, val: b'w' },
    LongOpt { name: "screensaver", has_arg: NO_ARG, val: b'S' },
    LongOpt { name: "message", has_arg: HAS_ARG, val: b'm' },
    LongOpt { name: "text-origin", has_arg: HAS_ARG, val: b'T' },
    LongOpt { name: "base", has_arg: HAS_ARG, val: b'b' },
    LongOpt { name: "geometry", has_arg: HAS_ARG, val: b'g' },
    LongOpt { name: "leaf", has_arg: HAS_ARG, val: b'c' },
    LongOpt { name: "colors", has_arg: HAS_ARG, val: b'k' },
    LongOpt { name: "multiplier", has_arg: HAS_ARG, val: b'M' },
    LongOpt { name: "life", has_arg: HAS_ARG, val: b'L' },
    LongOpt { name: "print", has_arg: NO_ARG, val: b'p' },
    LongOpt { name: "seed", has_arg: HAS_ARG, val: b's' },
    LongOpt { name: "save", has_arg: HAS_ARG, val: b'W' },
    LongOpt { name: "load", has_arg: HAS_ARG, val: b'C' },
    LongOpt { name: "verbose", has_arg: NO_ARG, val: b'v' },
    LongOpt { name: "help", has_arg: NO_ARG, val: b'h' },
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputResult {
    Continue,
    Quit,
    Resize,
}

pub struct Config {
    pub live: bool,
    pub infinite: bool,
    pub screensaver: bool,
    pub print_cactus: bool,
    pub verbosity: i32,
    pub life_start: i32,
    pub multiplier: i32,
    pub base_type: i32,
    pub base_width: i32,
    pub base_height: i32,
    pub base_origin_x: i32,
    pub base_origin_y: i32,
    pub automatic_base_position: bool,
    pub growth_height: i32,
    pub text_origin_x: i32,
    pub text_origin_y: i32,
    pub seed: i32,
    /// Spine strings (`-c`); empty list = a smooth, spineless cactus.
    pub spines: Vec<Vec<u8>>,
    pub colors: [i32; 4],
    pub save: bool,
    pub load: bool,
    /// Growth steps (slices + bloom frames) to replay to when loading.
    pub target_steps: i32,
    pub time_wait: f64,
    pub time_step: f32,
    pub message: Option<Vec<u8>>,
    pub text: Option<Vec<u8>>,
    pub save_file: Vec<u8>,
    pub load_file: Vec<u8>,
    /// `-T` used stdin for text; main must freopen the tty afterwards.
    pub reopen_stdin: bool,
    /// Test hook: never touch the real terminal.
    pub headless: bool,
}

/// XDG Base Directory Specification, as createDefaultCachePath walks it.
fn default_cache_path() -> Vec<u8> {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        let bytes = into_vec(xdg);
        if !bytes.is_empty() {
            let mut out = bytes;
            out.extend_from_slice(b"/tcac");
            return out;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let bytes = into_vec(home);
        if !bytes.is_empty() {
            let mut out = bytes;
            out.extend_from_slice(b"/.cache/tcac");
            return out;
        }
    }
    b"tcac".to_vec()
}

#[cfg(unix)]
fn into_vec(s: std::ffi::OsString) -> Vec<u8> {
    use std::os::unix::ffi::OsStringExt;
    s.into_vec()
}

#[cfg(not(unix))]
fn into_vec(s: std::ffi::OsString) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

impl Config {
    pub fn new() -> Config {
        Config {
            live: false,
            infinite: false,
            screensaver: false,
            print_cactus: false,
            verbosity: 0,
            life_start: 32,
            multiplier: 5,
            base_type: 1,
            base_width: 0,
            base_height: 0,
            base_origin_y: -1,
            base_origin_x: -1,
            automatic_base_position: true,
            growth_height: 0,
            text_origin_y: -1,
            text_origin_x: -1,
            seed: 0,
            spines: vec![b"'".to_vec()],
            colors: [2, 3, 10, 11],
            save: false,
            load: false,
            target_steps: 0,
            time_wait: 4.0,
            time_step: 0.03,
            message: None,
            text: None,
            save_file: default_cache_path(),
            load_file: default_cache_path(),
            reopen_stdin: false,
            headless: false,
        }
    }
}

/// Outcome of argument parsing: either run, or print and exit with a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseOutcome {
    Run,
    /// printHelp() then exit 0.
    Help,
    /// error message already on stderr; exit 1.
    Error,
}

/// Parse a string like "10,20" (parse_rowcol). Returns None on error.
pub fn parse_rowcol(rowcol: &[u8]) -> Option<(i32, i32)> {
    let first = strtol(rowcol, 10);
    if first.len == 0 || first.erange || first.value < 0 || first.value > i64::from(i32::MAX) {
        return None;
    }
    if first.len >= rowcol.len() || rowcol[first.len] != b',' {
        return None;
    }
    let second = strtol(&rowcol[first.len + 1..], 10);
    if second.len == 0 || second.erange || second.value < 0 || second.value > i64::from(i32::MAX) {
        return None;
    }
    // "*end != '\0'" check: the rowcol slice must be fully consumed
    if second.len != rowcol.len() - first.len - 1 {
        return None;
    }
    Some((first.value as i32, second.value as i32))
}

/// slurp_stdin: read to EOF, then behave as a C string (first NUL ends it).
pub fn slurp_stdin() -> Option<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    if std::io::stdin().read_to_end(&mut buf).is_err() {
        return None;
    }
    if let Some(nul) = buf.iter().position(|&b| b == 0) {
        buf.truncate(nul);
    }
    Some(buf)
}

/// strtok(input, ",") equivalent: split on single delimiters, skipping
/// empty tokens, in order.
fn split_strtok(input: &[u8], delim: u8) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &b) in input.iter().chain(std::iter::once(&delim)).enumerate() {
        if b == delim {
            if let Some(s) = start.take() {
                out.push(input[s..i].to_vec());
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    out
}

pub fn print_help(out: &mut Vec<u8>) {
    out.extend_from_slice(
        b"Usage: tcac [OPTION]...\n\
          \n\
          tcac is a beautifully random cactus generator.\n\
          \n\
          Options:\n\
          \x20 -l, --live             live mode: show each step of growth\n\
          \x20 -t, --time=TIME        in live mode, wait TIME secs between\n\
          \x20                          steps of growth (must be larger than 0) [default: 0.03]\n\
          \x20 -i, --infinite         infinite mode: keep growing cacti\n\
          \x20 -w, --wait=TIME        in infinite mode, wait TIME between each cactus\n\
          \x20                          generation [default: 4.00]\n\
          \x20 -S, --screensaver      screensaver mode; equivalent to -li and\n\
          \x20                          quit on any keypress\n\
          \x20 -m, --message=STR      attach message next to the cactus\n\
          \x20 -T, --text-origin=ROW,COL\n\
          \x20                          display text from stdin at row ROW, column COL\n\
          \x20 -g, --geometry=ROW,COL\n\
          \x20                          place the cactus pot at row ROW, column COL\n\
          \x20 -b, --base=INT         ascii-art pot to use, 0 is none\n\
          \x20 -c, --leaf=LIST        list of comma-delimited strings randomly chosen\n\
          \x20                          for spines [default: ']\n\
          \x20 -k, --color=LIST       list of 4 comma-delimited color indices (0-255) for\n\
          \x20                          each of dark body, dark spines, light body, and\n\
          \x20                          light spines, in that order [default: 2,3,10,11]\n\
          \x20 -M, --multiplier=INT   arm multiplier; higher -> more\n\
          \x20                          arms (0-20) [default: 5]\n\
          \x20 -L, --life=INT         life; higher -> more growth (0-200) [default: 32]\n\
          \x20 -p, --print            print cactus to terminal when finished\n\
          \x20 -s, --seed=INT         seed random number generator\n\
          \x20 -W, --save=FILE        save progress to file [default: $XDG_CACHE_HOME/tcac or $HOME/.cache/tcac]\n\
          \x20 -C, --load=FILE        load progress from file [default: $XDG_CACHE_HOME/tcac]\n\
          \x20 -v, --verbose          increase output verbosity\n\
          \x20 -h, --help             show help\n",
    );
}

/// Parse `argv` into the config. Parse errors land on `stderr_out` and yield
/// [`ParseOutcome::Error`].
pub fn parse_args(
    argv: Vec<Vec<u8>>,
    conf: &mut Config,
    stderr_out: &mut Vec<u8>,
) -> ParseOutcome {
    let mut go = GetOpt::new(argv);
    let mut spines_input: Vec<u8> = b"'".to_vec();
    let mut colors_input: Vec<u8> = b"2,3,10,11".to_vec();

    fn opt_name(c: u8) -> String {
        format!("-{}", c as char)
    }

    loop {
        match go.next(OPTSTRING, LONG_OPTIONS) {
            Opt::End => break,
            Opt::Char(c) => match c {
                b'l' => conf.live = true,
                b't' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    let parsed = strtod(&arg);
                    let f = parsed.value as f32;
                    if f != 0.0 {
                        conf.time_step = f;
                    } else {
                        stderr_out
                            .extend_from_slice(format!("error: invalid step time: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                    if conf.time_step < 0.0 {
                        stderr_out
                            .extend_from_slice(format!("error: invalid step time: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                }
                b'i' => conf.infinite = true,
                b'w' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    let parsed = strtod(&arg);
                    if parsed.value != 0.0 {
                        conf.time_wait = parsed.value;
                    } else {
                        stderr_out
                            .extend_from_slice(format!("error: invalid wait time: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                    if conf.time_wait < 0.0 {
                        stderr_out
                            .extend_from_slice(format!("error: invalid wait time: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                }
                b'S' => {
                    conf.live = true;
                    conf.infinite = true;
                    conf.save = true;
                    conf.load = true;
                    conf.screensaver = true;
                }
                b'm' => conf.message = Some(go.optarg.clone().unwrap_or_default()),
                b'T' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    match parse_rowcol(&arg) {
                        Some((row, col)) => {
                            conf.text_origin_y = row;
                            conf.text_origin_x = col;
                        }
                        None => {
                            stderr_out
                                .extend_from_slice(format!("error: failed parsing row,col: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                            return ParseOutcome::Error;
                        }
                    }
                    match slurp_stdin() {
                        Some(text) => conf.text = Some(text),
                        None => {
                            stderr_out.extend_from_slice(b"error: Failed reading text from stdin!\n");
                            return ParseOutcome::Error;
                        }
                    }
                    // freopen(ctermid(NULL), "r", stdin) happens in main
                    // after parsing
                    conf.reopen_stdin = true;
                }
                b'b' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    let value = strtod(&arg).value;
                    if value.is_nan() {
                        stderr_out
                            .extend_from_slice(format!("error: invalid pot index: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                    conf.base_type = c_int_from_f64(value);
                }
                b'g' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    match parse_rowcol(&arg) {
                        Some((row, col)) => {
                            conf.base_origin_y = row;
                            conf.base_origin_x = col;
                        }
                        None => {
                            stderr_out
                                .extend_from_slice(format!("error: failed parsing row,col: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                            return ParseOutcome::Error;
                        }
                    }
                    conf.automatic_base_position = false;
                }
                b'c' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    // strncpy(spinesInput, optarg, 127)
                    spines_input.truncate(0);
                    spines_input.extend_from_slice(&arg[..arg.len().min(127)]);
                }
                b'k' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    colors_input.truncate(0);
                    colors_input.extend_from_slice(&arg[..arg.len().min(127)]);
                }
                b'M' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    let value = strtod(&arg).value;
                    if value != 0.0 {
                        conf.multiplier = c_int_from_f64(value);
                    } else {
                        stderr_out
                            .extend_from_slice(format!("error: invalid multiplier: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                    // truncation may turn -0.5 into 0; both are unusable
                    if conf.multiplier < 1 {
                        stderr_out
                            .extend_from_slice(format!("error: invalid multiplier: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                }
                b'L' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    let value = strtod(&arg).value;
                    if value != 0.0 {
                        conf.life_start = c_int_from_f64(value);
                    } else {
                        stderr_out
                            .extend_from_slice(format!("error: invalid initial life: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                    if conf.life_start < 1 {
                        stderr_out
                            .extend_from_slice(format!("error: invalid initial life: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                }
                b'p' => conf.print_cactus = true,
                b's' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    let value = strtod(&arg).value;
                    if value != 0.0 {
                        conf.seed = c_int_from_f64(value);
                    } else {
                        stderr_out
                            .extend_from_slice(format!("error: invalid seed: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                    if conf.seed < 0 {
                        stderr_out
                            .extend_from_slice(format!("error: invalid seed: '{}'\n", String::from_utf8_lossy(&arg)).as_bytes());
                        return ParseOutcome::Error;
                    }
                }
                b'W' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    // skip argument if it's actually an option
                    if arg.first() == Some(&b'-') {
                        go.optind -= 1;
                    } else {
                        conf.save_file = arg;
                    }
                    conf.save = true;
                }
                b'C' => {
                    let arg = go.optarg.clone().unwrap_or_default();
                    if arg.first() == Some(&b'-') {
                        go.optind -= 1;
                    } else {
                        conf.load_file = arg;
                    }
                    conf.load = true;
                }
                b'v' => conf.verbosity += 1,
                b'h' => return ParseOutcome::Help,
                _ => unreachable!("getopt returned an unhandled letter"),
            },
            Opt::MissingArg => {
                let c = go.optopt;
                stderr_out.extend_from_slice(
                    format!("error: option '{}' requires an argument\n", opt_name(c)).as_bytes(),
                );
                return ParseOutcome::Error;
            }
            Opt::Unknown => {
                if go.optopt != 0 {
                    stderr_out.extend_from_slice(
                        format!("error: invalid option -- '{}'\n", go.optopt as char).as_bytes(),
                    );
                } else {
                    stderr_out
                        .extend_from_slice(b"error: unrecognized option (use --help to list options)\n");
                }
                return ParseOutcome::Error;
            }
        }
    }

    // delimit spines on "," — strtok skips empty tokens; a list with no
    // entries simply means "no spines"
    conf.spines = split_strtok(&spines_input, b',');
    if conf.spines.len() > 64 {
        conf.spines.truncate(64);
    }

    // colors
    let mut tokens = split_strtok(&colors_input, b',').into_iter();
    let mut parsed_colors = [0i32; 4];
    for slot in parsed_colors.iter_mut() {
        match tokens.next() {
            None => {
                stderr_out.extend_from_slice(b"error: too few color indices provided\n");
                return ParseOutcome::Error;
            }
            Some(token) => {
                let parsed = strtol(&token, 0);
                let ok_end = parsed.len == token.len();
                if parsed.len == 0
                    || !ok_end
                    || parsed.erange
                    || parsed.value < 0
                    || parsed.value >= 256
                {
                    stderr_out
                        .extend_from_slice(format!("error: invalid color index: '{}'\n", String::from_utf8_lossy(&token)).as_bytes());
                    return ParseOutcome::Error;
                }
                *slot = parsed.value as i32;
            }
        }
    }
    if tokens.next().is_some() {
        stderr_out.extend_from_slice(b"error: too many color indices provided\n");
        return ParseOutcome::Error;
    }
    conf.colors = parsed_colors;

    ParseOutcome::Run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> (Config, Vec<u8>) {
        let mut conf = Config::new();
        let mut err = Vec::new();
        let argv: Vec<Vec<u8>> = std::iter::once(b"tcac".to_vec())
            .chain(args.iter().map(|a| a.as_bytes().to_vec()))
            .collect();
        let outcome = parse_args(argv, &mut conf, &mut err);
        assert_eq!(outcome, ParseOutcome::Run, "err: {}", String::from_utf8_lossy(&err));
        (conf, err)
    }

    fn parse_err(args: &[&str]) -> String {
        let mut conf = Config::new();
        let mut err = Vec::new();
        let argv: Vec<Vec<u8>> = std::iter::once(b"tcac".to_vec())
            .chain(args.iter().map(|a| a.as_bytes().to_vec()))
            .collect();
        let outcome = parse_args(argv, &mut conf, &mut err);
        assert_eq!(outcome, ParseOutcome::Error);
        String::from_utf8_lossy(&err).into_owned()
    }

    #[test]
    fn defaults() {
        let (conf, _) = &parse(&[]);
        assert_eq!(conf.life_start, 32);
        assert_eq!(conf.multiplier, 5);
        assert_eq!(conf.base_type, 1);
        assert_eq!(conf.colors, [2, 3, 10, 11]);
        assert_eq!(conf.spines, vec![b"'".to_vec()]);
        assert!(!conf.print_cactus);
    }

    #[test]
    fn print_is_a_plain_flag_in_both_forms() {
        let (conf, _) = &parse(&["-p"]);
        assert!(conf.print_cactus);
        let (conf, _) = &parse(&["--print"]);
        assert!(conf.print_cactus);
        // --print no longer swallows the next option (sanitized)
        let (conf, _) = &parse(&["--print", "-l"]);
        assert!(conf.print_cactus && conf.live);
    }

    #[test]
    fn modes_and_values() {
        let (conf, _) = &parse(&["-S"]);
        assert!(conf.live && conf.infinite && conf.screensaver && conf.save && conf.load);
        let (conf, _) = &parse(&["-t", "0.5", "-w", "2", "-M", "9", "-L", "50", "-s", "42"]);
        assert_eq!(conf.time_step, 0.5);
        assert_eq!(conf.time_wait, 2.0);
        assert_eq!(conf.multiplier, 9);
        assert_eq!(conf.life_start, 50);
        assert_eq!(conf.seed, 42);
    }

    #[test]
    fn negative_truncation_is_rejected_not_fatal() {
        let msg = parse_err(&["-M", "-0.5"]);
        assert!(msg.contains("invalid multiplier"), "{msg}");
        let msg = parse_err(&["-L", "-0.5"]);
        assert!(msg.contains("invalid initial life"), "{msg}");
    }

    #[test]
    fn unknown_options_are_clean_errors() {
        let msg = parse_err(&["-z"]);
        assert!(msg.contains("invalid option -- 'z'"), "{msg}");
        assert!(!msg.contains('\0'), "no NUL byte in errors");
        let msg = parse_err(&["--bogus"]);
        assert!(msg.contains("unrecognized option"), "{msg}");
    }

    #[test]
    fn missing_argument_is_a_clean_error() {
        let msg = parse_err(&["-t"]);
        assert!(msg.contains("requires an argument"), "{msg}");
    }

    #[test]
    fn empty_spine_list_means_no_spines() {
        let (conf, _) = &parse(&["-c", ","]);
        assert!(conf.spines.is_empty());
    }

    #[test]
    fn spine_list_splits_on_commas() {
        let (conf, _) = &parse(&["-c", "a,bb,ccc,*"]);
        assert_eq!(conf.spines.len(), 4);
        assert_eq!(conf.spines[1], b"bb".to_vec());
        // a value with no commas is one token, quotes included
        let (conf, _) = &parse(&["-c", "'\""]);
        assert_eq!(conf.spines, vec![b"'\"".to_vec()]);
        let (conf, _) = &parse(&["-c", "++"]);
        assert_eq!(conf.spines, vec![b"++".to_vec()]);
    }

    #[test]
    fn color_list_validation() {
        let msg = parse_err(&["-k", "1,2,3"]);
        assert!(msg.contains("too few color indices"), "{msg}");
        let msg = parse_err(&["-k", "1,2,3,4,5"]);
        assert!(msg.contains("too many color indices"), "{msg}");
        let msg = parse_err(&["-k", "1,2,3,999"]);
        assert!(msg.contains("invalid color index"), "{msg}");
        let (conf, _) = &parse(&["-k", "4,5,6,7"]);
        assert_eq!(conf.colors, [4, 5, 6, 7]);
    }

    #[test]
    fn rowcol_parsing() {
        let (conf, _) = &parse(&["-g", "3,10", "-T", "0,0"]);
        assert_eq!((conf.base_origin_y, conf.base_origin_x), (3, 10));
        assert!(!conf.automatic_base_position);
        assert_eq!((conf.text_origin_y, conf.text_origin_x), (0, 0));
        let msg = parse_err(&["-g", "3"]);
        assert!(msg.contains("failed parsing row,col"), "{msg}");
    }
}
