//! GNU getopt_long replica — cbonsai's CLI semantics depend on it.
//!
//! Reproduced behaviors (all exercised by the parity suite):
//! - leading ':' in optstring: getopt itself never prints; missing arguments
//!   return ':' instead of '?'.
//! - short-option bundling (`-li`), attached (`-t0.03`) and separate
//!   (`-t 0.03`) arguments, `--opt=arg` and `--opt arg`.
//! - unambiguous long-option abbreviation (`--liv` → `--live`).
//! - GNU argv permutation: non-options are moved to the end, options after
//!   them are still parsed.
//! - optopt: the offending letter for short options; 0 for unrecognized long
//!   options (cbonsai prints it with %c, producing a NUL byte — observed on
//!   the reference); the mapped short letter for a missing long-arg.
//! - optind arithmetic compatible with cbonsai's `-W`/`-C`
//!   `optarg[0]=='-' → optind -= 1` trick.

pub const HAS_ARG: u8 = 1;
pub const NO_ARG: u8 = 0;

pub struct LongOpt {
    pub name: &'static str,
    pub has_arg: u8,
    /// The short letter this long option maps to (cbonsai's `val`).
    pub val: u8,
}

pub struct GetOpt {
    argv: Vec<Vec<u8>>,
    pub optind: usize,
    nextchar: usize,
    pub optarg: Option<Vec<u8>>,
    pub optopt: u8,
    done: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Opt {
    Char(u8),
    /// Missing required argument (getopt's ':' with a leading-':' optstring).
    MissingArg,
    /// Unrecognized option (getopt's '?').
    Unknown,
    /// No more options.
    End,
}

fn is_optionish(a: &[u8]) -> bool {
    a.len() > 1 && a[0] == b'-'
}

impl GetOpt {
    pub fn new<I: IntoIterator<Item = Vec<u8>>>(argv: I) -> Self {
        let argv: Vec<Vec<u8>> = argv.into_iter().collect();
        GetOpt { argv, optind: 1, nextchar: 0, optarg: None, optopt: 0, done: false }
    }

    /// Parse the next option. `optstring` excludes the leading ':' (it is
    /// implied; error output is always suppressed).
    pub fn next(&mut self, optstring: &str, longopts: &[LongOpt]) -> Opt {
        if self.done {
            return Opt::End;
        }
        let os: &[u8] = optstring.as_bytes();
        loop {
            if self.nextchar == 0 {
                // advance to the next candidate, permuting non-options away
                while self.optind < self.argv.len() {
                    let arg = self.argv[self.optind].clone();
                    if arg == b"--" {
                        self.optind += 1;
                        self.done = true;
                        return Opt::End;
                    }
                    if is_optionish(&arg) {
                        break;
                    }
                    // non-option: exchange it with the next option, or end
                    let mut k = self.optind + 1;
                    while k < self.argv.len() && !is_optionish(&self.argv[k]) && self.argv[k] != b"--" {
                        k += 1;
                    }
                    if k >= self.argv.len() {
                        self.done = true;
                        return Opt::End;
                    }
                    // rotate argv[optind..=k] right by one so argv[k] lands
                    // at optind
                    let option = self.argv[k].clone();
                    for j in (self.optind..k).rev() {
                        self.argv[j + 1] = self.argv[j].clone();
                    }
                    self.argv[self.optind] = option;
                }
                if self.optind >= self.argv.len() {
                    self.done = true;
                    return Opt::End;
                }
            }

            let arg = self.argv[self.optind].clone();
            if arg.len() >= 2 && arg[0] == b'-' && arg[1] == b'-' {
                // long option
                let body = &arg[2..];
                let (name, eq_arg) = match body.iter().position(|&b| b == b'=') {
                    Some(p) => (&body[..p], Some(body[p + 1..].to_vec())),
                    None => (body, None),
                };
                // exact match first, then unique prefix
                let exact = longopts.iter().filter(|o| o.name.as_bytes() == name).count();
                let matches: Vec<&LongOpt> = if exact == 1 {
                    longopts.iter().filter(|o| o.name.as_bytes() == name).collect()
                } else {
                    longopts
                        .iter()
                        .filter(|o| o.name.as_bytes().starts_with(name))
                        .collect()
                };
                if matches.len() != 1 {
                    // unrecognized or ambiguous: silent '?' with optopt 0
                    self.optind += 1;
                    self.nextchar = 0;
                    self.optopt = 0;
                    return Opt::Unknown;
                }
                let opt = matches[0];
                self.optind += 1;
                self.nextchar = 0;
                if opt.has_arg == HAS_ARG {
                    if let Some(a) = eq_arg {
                        self.optarg = Some(a);
                    } else if self.optind < self.argv.len() {
                        self.optarg = Some(self.argv[self.optind].clone());
                        self.optind += 1;
                    } else {
                        self.optopt = opt.val;
                        return Opt::MissingArg;
                    }
                } else {
                    if eq_arg.is_some() {
                        // "doesn't allow an argument": silent '?' with optopt 0
                        self.optopt = 0;
                        return Opt::Unknown;
                    }
                    self.optarg = None;
                }
                return Opt::Char(opt.val);
            }

            // short option cluster
            let cluster = &arg[1..];
            if self.nextchar == 0 {
                self.nextchar = 1;
            }
            if self.nextchar >= cluster.len() + 1 {
                // defensive: nothing left in the cluster
                self.optind += 1;
                self.nextchar = 0;
                continue;
            }
            let c = cluster[self.nextchar - 1];
            let spec_pos = os.iter().position(|&o| o == c);
            match spec_pos {
                None => {
                    self.optopt = c;
                    self.nextchar += 1;
                    if self.nextchar > cluster.len() {
                        self.optind += 1;
                        self.nextchar = 0;
                    }
                    return Opt::Unknown;
                }
                Some(p) => {
                    let takes_arg = p + 1 < os.len() && os[p + 1] == b':';
                    if takes_arg {
                        if self.nextchar < cluster.len() {
                            self.optarg = Some(cluster[self.nextchar..].to_vec());
                            self.optind += 1;
                            self.nextchar = 0;
                        } else if self.optind + 1 < self.argv.len() {
                            self.optarg = Some(self.argv[self.optind + 1].clone());
                            self.optind += 2;
                            self.nextchar = 0;
                        } else {
                            self.optind += 1;
                            self.nextchar = 0;
                            self.optopt = c;
                            return Opt::MissingArg;
                        }
                    } else {
                        self.optarg = None;
                        self.nextchar += 1;
                        if self.nextchar > cluster.len() {
                            self.optind += 1;
                            self.nextchar = 0;
                        }
                    }
                    return Opt::Char(c);
                }
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTSTRING: &str = "lt:iw:Sm:T:b:g:c:k:M:L:ps:C:W:vh";
    const LONGS: &[LongOpt] = &[
        LongOpt { name: "live", has_arg: NO_ARG, val: b'l' },
        LongOpt { name: "time", has_arg: HAS_ARG, val: b't' },
        LongOpt { name: "infinite", has_arg: NO_ARG, val: b'i' },
        LongOpt { name: "wait", has_arg: HAS_ARG, val: b'w' },
        LongOpt { name: "screensaver", has_arg: NO_ARG, val: b'S' },
        LongOpt { name: "message", has_arg: HAS_ARG, val: b'm' },
        LongOpt { name: "print", has_arg: HAS_ARG, val: b'p' },
        LongOpt { name: "seed", has_arg: HAS_ARG, val: b's' },
        LongOpt { name: "life", has_arg: HAS_ARG, val: b'L' },
        LongOpt { name: "load", has_arg: HAS_ARG, val: b'C' },
        LongOpt { name: "verbose", has_arg: NO_ARG, val: b'v' },
        LongOpt { name: "help", has_arg: NO_ARG, val: b'h' },
    ];

    fn run(args: &[&str]) -> Vec<Opt> {
        let mut go = GetOpt::new(args.iter().map(|a| a.as_bytes().to_vec()));
        let mut out = Vec::new();
        while let Some(o) = {
            let r = go.next(OPTSTRING, LONGS);
            if r == Opt::End {
                None
            } else {
                Some(r)
            }
        } {
            out.push(o);
        }
        out
    }

    #[test]
    fn bundling_and_attached_args() {
        assert_eq!(run(&["tcac", "-li"]), vec![Opt::Char(b'l'), Opt::Char(b'i')]);
        assert_eq!(run(&["tcac", "-t0.03"]), vec![Opt::Char(b't')]);
        assert_eq!(run(&["tcac", "-t", "0.03"]), vec![Opt::Char(b't')]);
    }

    #[test]
    fn long_options_and_abbreviation() {
        assert_eq!(run(&["tcac", "--liv"]), vec![Opt::Char(b'l')]);
        assert_eq!(run(&["tcac", "--time=2"]), vec![Opt::Char(b't')]);
        assert_eq!(run(&["tcac", "--time", "2"]), vec![Opt::Char(b't')]);
        assert_eq!(run(&["tcac", "--bogus"]), vec![Opt::Unknown]);
        // "l" prefixes live/life/load: ambiguous
        assert_eq!(run(&["tcac", "--l"]), vec![Opt::Unknown]);
        // no_argument option with an = argument is rejected
        assert_eq!(run(&["tcac", "--live=1"]), vec![Opt::Unknown]);
    }

    #[test]
    fn missing_args() {
        assert_eq!(run(&["tcac", "-t"]), vec![Opt::MissingArg]);
        assert_eq!(run(&["tcac", "--time"]), vec![Opt::MissingArg]);
        assert_eq!(run(&["tcac", "-W"]), vec![Opt::MissingArg]);
    }

    #[test]
    fn permutation() {
        assert_eq!(run(&["tcac", "foo", "-l"]), vec![Opt::Char(b'l')]);
        assert_eq!(run(&["tcac", "-l", "foo", "-p"]), vec![Opt::Char(b'l'), Opt::Char(b'p')]);
    }

    #[test]
    fn dash_dash_ends_options() {
        assert_eq!(run(&["tcac", "--", "-l"]), vec![]);
    }

    #[test]
    fn optind_after_W_with_option_argument() {
        // -W consumes "-p" as its argument; the re-parse is cbonsai's
        // `optarg[0]=='-' -> optind -= 1` trick, so verify the arithmetic
        // parse_args relies on: optarg == "-p" and optind moved past BOTH.
        let mut go = GetOpt::new([
            b"tcac".to_vec(), b"-W".to_vec(), b"-p".to_vec(),
        ]);
        assert_eq!(go.next(OPTSTRING, LONGS), Opt::Char(b'W'));
        assert_eq!(go.optarg.as_deref(), Some(&b"-p"[..]));
        assert_eq!(go.optind, 3);
        go.optind -= 1; // the trick
        assert_eq!(go.next(OPTSTRING, LONGS), Opt::Char(b'p'));
    }
}
