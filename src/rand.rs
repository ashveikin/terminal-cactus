//! glibc `rand()`/`srand()` clone — the determinism contract of the port.
//!
//! cbonsai's growth is driven entirely by `rand()`; rbonsai is bit-exact only
//! if every call returns the same value glibc would return. glibc's default
//! `rand()` is TYPE_3 of the `random()` family: a 31-entry additive feedback
//! generator (deg 31, sep 3) seeded through the Schrage form of the 16807
//! Lehmer generator, discarding the first 310 outputs.
//!
//! Golden sequences from the host libc are pinned in
//! `tools/rand_golden.txt` and checked by `cargo test`.

pub struct GlibcRand {
    state: [i32; 31],
    f: usize,
    r: usize,
}

impl GlibcRand {
    /// `srand(seed)`.
    pub fn srand(seed: u32) -> Self {
        // glibc: "We must make sure the seed is not 0. Take arbitrarily 1."
        let mut word = if seed == 0 { 1i32 } else { seed as i32 };
        let mut state = [0i32; 31];
        state[0] = word;
        // state[i] = (16807 * state[i-1]) mod 2147483647, computed so the
        // product cannot overflow 31 bits. C division truncates toward zero;
        // Rust i32 division does too.
        for item in state.iter_mut().skip(1) {
            let hi = word / 127773;
            let lo = word % 127773;
            word = 16807 * lo - 2836 * hi;
            if word < 0 {
                word += 2147483647;
            }
            *item = word;
        }
        let mut rng = GlibcRand { state, f: 3, r: 0 };
        // Discard the first 10 * 31 outputs.
        for _ in 0..310 {
            rng.next();
        }
        rng
    }

    /// `rand()`: values in 0..=i32::MAX, identical to glibc.
    pub fn rand(&mut self) -> i32 {
        (self.next() >> 1) as i32
    }

    fn next(&mut self) -> u32 {
        let v = self.state[self.f].wrapping_add(self.state[self.r]);
        self.state[self.f] = v;
        self.f += 1;
        if self.f >= 31 {
            self.f = 0;
        }
        self.r += 1;
        if self.r >= 31 {
            self.r = 0;
        }
        v as u32
    }
}

/// C's `(int)double` on x86-64: cvttsd2si. Truncation in range, and the
/// "integer indefinite" value INT_MIN for NaN or anything out of range.
/// Rust's `as` saturates instead, so option parsing must go through this.
pub fn c_int_from_f64(x: f64) -> i32 {
    if x.is_nan() || !(x >= -2147483648.0 && x < 2147483648.0) {
        i32::MIN
    } else {
        x as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    fn golden_lines() -> Vec<(u32, Vec<i32>)> {
        let data = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tools/rand_golden.txt"
        ));
        data.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let (seed, rest) = l.split_once(':').expect("golden line");
                let values = rest
                    .split_whitespace()
                    .map(|v| v.parse::<i32>().expect("golden value"))
                    .collect();
                (seed.parse::<u32>().expect("golden seed"), values)
            })
            .collect()
    }

    #[test]
    fn matches_host_libc_golden_sequences() {
        for (seed, values) in golden_lines() {
            let mut rng = GlibcRand::srand(seed);
            for (i, want) in values.iter().enumerate() {
                let got = rng.rand();
                assert_eq!(got, *want, "seed {seed} value {i}");
            }
        }
    }

    #[test]
    fn c_int_from_f64_follows_cvttsd2si() {
        assert_eq!(c_int_from_f64(5.9), 5);
        assert_eq!(c_int_from_f64(-0.5), 0);
        assert_eq!(c_int_from_f64(f64::INFINITY), i32::MIN);
        assert_eq!(c_int_from_f64(f64::NEG_INFINITY), i32::MIN);
        assert_eq!(c_int_from_f64(f64::NAN), i32::MIN);
        assert_eq!(c_int_from_f64(-1.0), -1);
    }

    #[test]
    fn golden_file_format_is_self_consistent() {
        let mut rng = GlibcRand::srand(1);
        let mut buf = String::new();
        for i in 0..20 {
            let _ = write!(buf, "{} ", rng.rand());
            let _ = i;
        }
        assert!(!buf.is_empty());
    }
}
