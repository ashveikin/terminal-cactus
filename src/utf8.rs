//! Byte-wise UTF-8 decoding with glibc `mbrtowc` semantics.
//!
//! cbonsai decodes one character per call and, on EILSEQ or an incomplete
//! sequence, substitutes '?' and advances exactly one byte. Callers choose
//! what to do with each outcome, so the decoder reports it faithfully:
//!
//! - [`decode_one`] returns the decoded codepoint and its byte length, or
//!   [`DecodeError::Invalid`] / [`DecodeError::Incomplete`] for the mbrtowc
//!   error returns ((size_t)-1 and (size_t)-2).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// mbrtowc returned (size_t)-1: bytes do not form valid UTF-8.
    Invalid,
    /// mbrtowc returned (size_t)-2: the buffer ends mid-sequence.
    Incomplete,
}

fn continuation(b: u8) -> bool {
    b & 0xC0 == 0x80
}

/// Decode one character from `bytes`. Returns `(codepoint, length)` or the
/// mbrtowc-style error. Strict UTF-8: overlongs, surrogates and values above
/// 0x10FFFF are EILSEQ, exactly like glibc.
pub fn decode_one(bytes: &[u8]) -> Result<(u32, usize), DecodeError> {
    debug_assert!(!bytes.is_empty());
    let b0 = bytes[0];
    let need: usize;
    let min: u32;
    let mut cp: u32;
    if b0 < 0x80 {
        return Ok((b0 as u32, 1));
    } else if (0xC2..=0xDF).contains(&b0) {
        need = 1;
        min = 0x80;
        cp = (b0 & 0x1F) as u32;
    } else if (0xE0..=0xEF).contains(&b0) {
        need = 2;
        min = 0x800;
        cp = (b0 & 0x0F) as u32;
    } else if (0xF0..=0xF4).contains(&b0) {
        need = 3;
        min = 0x10000;
        cp = (b0 & 0x07) as u32;
    } else {
        // 0x80..0xC1 and 0xF5..0xFF can never start valid UTF-8
        return Err(DecodeError::Invalid);
    }

    if bytes.len() < 1 + need {
        return Err(DecodeError::Incomplete);
    }
    for &b in &bytes[1..=need] {
        if !continuation(b) {
            return Err(DecodeError::Invalid);
        }
        cp = (cp << 6) | (b & 0x3F) as u32;
    }

    if cp < min {
        return Err(DecodeError::Invalid);
    }
    if (0xD800..=0xDFFF).contains(&cp) {
        return Err(DecodeError::Invalid);
    }
    if cp > 0x10FFFF {
        return Err(DecodeError::Invalid);
    }
    Ok((cp, need + 1))
}

/// Encode a codepoint as UTF-8 (valid codepoints only; the decoder never
/// produces anything else).
pub fn encode_utf8(cp: u32, out: &mut Vec<u8>) {
    let ch = char::from_u32(cp).expect("cell codepoints are valid Unicode");
    let mut buf = [0u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii() {
        assert_eq!(decode_one(b"a"), Ok((0x61, 1)));
    }

    #[test]
    fn sakura() {
        // 桜 U+685C
        let bytes = "桜".as_bytes();
        assert_eq!(decode_one(bytes), Ok((0x685C, 3)));
    }

    #[test]
    fn overlong_is_invalid() {
        // overlong encoding of '/'
        assert_eq!(decode_one(&[0xC0, 0xAF]), Err(DecodeError::Invalid));
    }

    #[test]
    fn surrogate_is_invalid() {
        // U+D800 encoded
        assert_eq!(decode_one(&[0xED, 0xA0, 0x80]), Err(DecodeError::Invalid));
    }

    #[test]
    fn truncated_is_incomplete() {
        // 桜 missing its last byte
        assert_eq!(decode_one(&[0xE6, 0xA1]), Err(DecodeError::Incomplete));
    }

    #[test]
    fn cbonsai_advance_is_one_byte_on_errors() {
        // "ab\xE2\x82" — truncated €: layoutMessage emits '?' per byte.
        let bytes = b"ab\xE2\x82";
        let mut cursor = 0;
        let mut out = Vec::new();
        while cursor < bytes.len() {
            match decode_one(&bytes[cursor..]) {
                Ok((cp, len)) => {
                    out.push(cp as u8 as char);
                    cursor += len;
                }
                Err(_) => {
                    out.push('?');
                    cursor += 1;
                }
            }
        }
        assert_eq!(out.iter().collect::<String>(), "ab??");
    }
}
