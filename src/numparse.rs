//! Hand-rolled `strtod`/`strtof`/`strtol` replicas.
//!
//! cbonsai's option parsing leans on C conversion semantics: hex floats and
//! inf/nan via strtod, base-0 integers via strtol, ERANGE checked through
//! errno for `-b`, colors and rowcol, and the end pointer ignored everywhere
//! except colors and rowcol. The decimal path delegates to `f64::from_str`,
//! which is correctly rounded just like glibc's strtod, so both agree to the
//! last bit; hex floats and inf/nan are recognized here.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CDouble {
    pub value: f64,
    /// Number of bytes consumed (0 when no conversion was performed).
    pub len: usize,
    /// errno would be ERANGE after the call.
    pub erange: bool,
}

fn starts_with_ignore_case(s: &[u8], prefix: &[u8]) -> bool {
    s.len() >= prefix.len()
        && s.iter()
            .zip(prefix)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
}

pub fn strtod(s: &[u8]) -> CDouble {
    let mut i = 0;
    while i < s.len() && (s[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    let mut sign = 1.0f64;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        if s[i] == b'-' {
            sign = -1.0;
        }
        i += 1;
    }

    let rest = &s[i..];
    // inf / infinity
    if starts_with_ignore_case(rest, b"infinity") {
        return CDouble { value: sign * f64::INFINITY, len: i + 8, erange: false };
    }
    if starts_with_ignore_case(rest, b"inf") {
        return CDouble { value: sign * f64::INFINITY, len: i + 3, erange: false };
    }
    if starts_with_ignore_case(rest, b"nan") {
        // plain NaN; the parenthesized subject-sequence form is consumed too
        let len = if rest.len() > 3 && rest[3] == b'(' {
            rest.iter().position(|&b| b == b')').map_or(3, |close| close + 1)
        } else {
            3
        };
        return CDouble { value: f64::NAN, len: i + len, erange: false };
    }

    // hex float: 0x... (optional p-exponent)
    if rest.len() >= 2 && rest[0] == b'0' && (rest[1] == b'x' || rest[1] == b'X') {
        return parse_hex_float(s, i, sign);
    }

    // decimal: locate the token span, then hand the exact digits to
    // f64::from_str (correctly rounded, same as glibc strtod).
    let start_digits = i;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    let mut frac_digits = 0usize;
    if i < s.len() && s[i] == b'.' {
        let mut j = i + 1;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        frac_digits = j - (i + 1);
        if start_digits + frac_digits > i || start_digits != i {
            i = j;
        }
    }
    if i == start_digits && frac_digits == 0 {
        return CDouble { value: 0.0, len: 0, erange: false };
    }
    let mut end = i;
    if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
        let mut j = i + 1;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        let ds = j;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        if j > ds {
            end = j;
        }
    }
    let token = std::str::from_utf8(&s[start_digits..end]).unwrap_or("0");
    let value: f64 = token.parse().unwrap_or(0.0) * sign;
    let nonzero_input = token.bytes().any(|b| b.is_ascii_digit() && b != b'0');
    let erange = value.is_infinite()
        || (nonzero_input && value != 0.0 && value.abs() < f64::MIN_POSITIVE)
        || (nonzero_input && value == 0.0);
    CDouble { value, len: end, erange }
}

fn parse_hex_float(s: &[u8], i: usize, sign: f64) -> CDouble {
    let rest = &s[i + 2..];
    let mut k = 0;
    let mut mant_digits = 0usize;
    while k < rest.len() && rest[k].is_ascii_hexdigit() {
        k += 1;
        mant_digits += 1;
    }
    let mut frac_digits = 0usize;
    if k < rest.len() && rest[k] == b'.' {
        let mut j = k + 1;
        while j < rest.len() && rest[j].is_ascii_hexdigit() {
            j += 1;
            frac_digits += 1;
        }
        if mant_digits + frac_digits > 0 {
            k = j;
        }
    }
    if mant_digits + frac_digits == 0 {
        return CDouble { value: 0.0, len: 0, erange: false };
    }
    let mut exp2: i32 = 0;
    let mut end = i + 2 + k;
    if k < rest.len() && (rest[k] == b'p' || rest[k] == b'P') {
        let mut j = k + 1;
        if j < rest.len() && (rest[j] == b'+' || rest[j] == b'-') {
            j += 1;
        }
        let ds = j;
        while j < rest.len() && rest[j].is_ascii_digit() {
            j += 1;
        }
        if j > ds {
            let (v, l) = strtol_base(&rest[ds..j], 10);
            if l > 0 {
                exp2 = v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                end = i + 2 + j;
            }
        }
    }
    // assemble the significand from nibbles
    let hex_text = &rest[..if k > mant_digits { k } else { mant_digits }];
    let mut value = 0f64;
    for &b in &hex_text[..mant_digits + frac_digits] {
        if b == b'.' {
            continue;
        }
        value = value * 16.0 + f64::from((b as char).to_digit(16).unwrap());
    }
    exp2 -= 4 * frac_digits as i32;
    let mut out = value;
    let mut e = exp2;
    while e > 1023 {
        out *= f64::from(1u32 << 30) * f64::from(1u32 << 22);
        e -= 52;
        if out.is_infinite() {
            break;
        }
    }
    while e < -1023 && out != 0.0 {
        out *= f64::from(1u32 << 30) * f64::from(1u32 << 22);
        e += 52;
    }
    out = sign * out * (2.0f64).powi(e);
    let erange = out.is_infinite() || (out != 0.0 && out.abs() < f64::MIN_POSITIVE);
    CDouble { value: out, len: end, erange }
}

/// `strtol(s, &end, base)` with base 0 or 10. Returns (value, bytes
/// consumed); 0 consumed means "no conversion performed".
pub fn strtol_base(s: &[u8], base: u32) -> (i64, usize) {
    let mut i = 0;
    while i < s.len() && (s[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    let mut sign = 1i64;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        if s[i] == b'-' {
            sign = -1;
        }
        i += 1;
    }
    let mut effective = base;
    if (base == 0 || base == 16) && i + 1 < s.len() && s[i] == b'0' && (s[i + 1] | 0x20) == b'x' {
        if i + 2 < s.len() && s[i + 2].is_ascii_hexdigit() {
            i += 2;
            effective = 16;
        } else {
            return (0, 0);
        }
    } else if base == 0 && i < s.len() && s[i] == b'0' {
        effective = 8;
    } else if base == 0 {
        effective = 10;
    }

    let digits_start = i;
    let mut value: i64 = 0;
    let mut overflow = false;
    while i < s.len() {
        let d = match (s[i] as char).to_digit(effective) {
            Some(d) => d as i64,
            None => break,
        };
        value = match value.checked_mul(effective as i64).and_then(|v| v.checked_add(d)) {
            Some(v) => v,
            None => {
                overflow = true;
                break;
            }
        };
        i += 1;
    }
    if i == digits_start {
        return (0, 0);
    }
    if overflow {
        return (if sign > 0 { i64::MAX } else { i64::MIN }, i);
    }
    (sign * value, i)
}

/// strtol with errno semantics: ERANGE when the value saturated i64.
pub struct LParse {
    pub value: i64,
    pub len: usize,
    pub erange: bool,
}

pub fn strtol(s: &[u8], base: u32) -> LParse {
    let (value, len) = strtol_base(s, base);
    let erange = value == i64::MAX || value == i64::MIN;
    LParse { value, len, erange }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_and_exponents() {
        assert_eq!(strtod(b"0.03").value, 0.03);
        assert_eq!(strtod(b"1e999").value, f64::INFINITY);
        assert!(strtod(b"1e999").erange);
        assert_eq!(strtod(b"-0.5").value, -0.5);
        assert_eq!(strtod(b"4").len, 1);
        assert_eq!(strtod(b"4.00").value, 4.0);
        assert_eq!(strtod(b" 12abc").value, 12.0);
        assert_eq!(strtod(b" 12abc").len, 3);
        assert_eq!(strtod(b"0.000001").value, 1e-6);
        // correctly rounded: 0.1 + 0.2 style values land exactly as C's
        assert_eq!(strtod(b"1e-320").value.abs() < f64::MIN_POSITIVE, true);
    }

    #[test]
    fn hex_floats_and_inf_nan() {
        assert_eq!(strtod(b"0x10").value, 16.0);
        assert_eq!(strtod(b"-0x1p4").value, -16.0);
        assert_eq!(strtod(b"inf").value, f64::INFINITY);
        assert!(strtod(b"nan").value.is_nan());
    }

    #[test]
    fn failed_conversions_consume_nothing() {
        assert_eq!(strtod(b"abc").len, 0);
        assert_eq!(strtod(b"+").len, 0);
        assert_eq!(strtod(b"0x").len, 0);
    }

    #[test]
    fn strtol_bases() {
        assert_eq!(strtol(b"010", 0).value, 8);
        assert_eq!(strtol(b"0x1f", 0).value, 31);
        assert_eq!(strtol(b"42", 0).value, 42);
        assert_eq!(strtol(b"12a", 10).value, 12);
        assert_eq!(strtol(b"12a", 10).len, 2);
        assert_eq!(strtol(b"-7", 10).value, -7);
        assert!(strtol(b"99999999999999999999999", 10).erange);
    }
}
