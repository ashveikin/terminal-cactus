//! Port of cbonsai's message layout (`layoutMessage` and friends) — a
//! word-wrap state machine over decoded wide characters, using the glibc
//! wcwidth/iswspace/iswcntrl tables so decisions land on the same cells.

use crate::utf8::{decode_one, DecodeError};
use crate::wcwidth::{iswcntrl, iswspace, wcwidth};

#[derive(Default, Clone)]
pub struct MessageLine {
    pub characters: Vec<u32>,
    pub width: i32,
}

#[derive(Default)]
pub struct MessageLayout {
    pub lines: Vec<Vec<u32>>,
    pub width: i32,
}

/// normalizeMessageCharacter: control or non-printable characters become
/// '?'; width is recomputed alongside.
fn normalize(cp: u32) -> (u32, i32) {
    let mut width = wcwidth(cp);
    if iswcntrl(cp) || width < 0 {
        return (0x3F, 1); // L'?'
    }
    if width == 0 {
        // combining characters carry no columns; upstream keeps them and
        // ncurses attaches them to the previous cell — the parity dump only
        // reads base chars, so width 0 is preserved as-is
        width = 0;
    }
    (cp, width)
}

fn append_to_line(line: &mut MessageLine, character: u32, width: i32) {
    line.characters.push(character);
    line.width += width;
}

fn add_message_line(layout: &mut MessageLayout, line: &mut MessageLine) {
    if line.width > layout.width {
        layout.width = line.width;
    }
    layout.lines.push(std::mem::take(&mut line.characters));
    line.width = 0;
}

/// appendWrappedCharacter
fn append_wrapped(layout: &mut MessageLayout, line: &mut MessageLine, mut cp: u32, max_width: i32) {
    let mut width;
    (cp, width) = normalize(cp);
    if width > max_width {
        cp = 0x3F;
        width = 1;
    }
    if width > 0 && line.width > 0 && line.width + width > max_width {
        add_message_line(layout, line);
    }
    append_to_line(line, cp, width);
}

/// appendMessageWord
fn append_word(
    layout: &mut MessageLayout,
    line: &mut MessageLine,
    word: &[u32],
    add_space: bool,
    max_width: i32,
) {
    let mut word_width = 0;
    for &cp in word {
        let (_c, w) = normalize(cp);
        let w = if w > max_width { 1 } else { w };
        word_width += w;
    }

    if add_space && !line.characters.is_empty() {
        if line.width + 1 + word_width > max_width {
            add_message_line(layout, line);
        } else {
            append_to_line(line, b' ' as u32, 1);
        }
    }

    for &cp in word {
        append_wrapped(layout, line, cp, max_width);
    }
}

/// flushMessageWord
fn flush_word(
    layout: &mut MessageLayout,
    line: &mut MessageLine,
    word: &mut MessageLine,
    add_space: &mut bool,
    max_width: i32,
) {
    if word.characters.is_empty() {
        return;
    }
    let chars = std::mem::take(&mut word.characters);
    word.width = 0;
    append_word(layout, line, &chars, *add_space, max_width);
    *add_space = false;
}

/// layoutMessage: wrap UTF-8 text into lines of at most `max_width` columns.
/// Horizontal whitespace separates words; \r/\n/\r\n produce line breaks
/// (with \r\n collapsing to one); invalid bytes become '?' advancing one
/// byte, exactly like the mbrtowc error handling in cbonsai.
pub fn layout_message(message: &[u8], max_width: i32) -> MessageLayout {
    let mut layout = MessageLayout::default();
    let mut line = MessageLine::default();
    let mut word = MessageLine::default();
    let mut add_space = false;
    let mut previous_was_cr = false;

    let mut cursor = 0usize;
    while cursor < message.len() {
        let (character, bytes) = match decode_one(&message[cursor..]) {
            Ok((cp, len)) => (cp, len),
            Err(DecodeError::Invalid) | Err(DecodeError::Incomplete) => (0x3F, 1),
        };
        if character == 0 {
            // mbrtowc returned 0 for a NUL byte; the caller's loop over a C
            // string would stop there
            break;
        }
        cursor += bytes;

        if character == b'\r' as u32 || character == b'\n' as u32 {
            if character == b'\n' as u32 && previous_was_cr {
                previous_was_cr = false;
                continue;
            }
            flush_word(&mut layout, &mut line, &mut word, &mut add_space, max_width);
            add_message_line(&mut layout, &mut line);
            previous_was_cr = character == b'\r' as u32;
            add_space = false;
            continue;
        }
        previous_was_cr = false;

        if iswspace(character) {
            flush_word(&mut layout, &mut line, &mut word, &mut add_space, max_width);
            add_space = !line.characters.is_empty();
            continue;
        }

        let width = wcwidth(character);
        append_to_line(&mut word, character, width.max(0));
    }

    flush_word(&mut layout, &mut line, &mut word, &mut add_space, max_width);
    if !line.characters.is_empty() || layout.lines.is_empty() {
        add_message_line(&mut layout, &mut line);
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(layout: &MessageLayout) -> Vec<String> {
        layout
            .lines
            .iter()
            .map(|l| l.iter().filter_map(|&c| char::from_u32(c)).collect())
            .collect()
    }

    #[test]
    fn wraps_on_word_boundaries() {
        let layout = layout_message(b"hello world", 8);
        assert_eq!(texts(&layout), vec!["hello", "world"]);
    }

    #[test]
    fn repeated_whitespace_is_one_separator() {
        let layout = layout_message(b"one     two", 20);
        assert_eq!(texts(&layout), vec!["one two"]);
    }

    #[test]
    fn crlf_is_one_break() {
        let layout = layout_message(b"a\r\nb", 20);
        assert_eq!(texts(&layout), vec!["a", "b"]);
    }

    #[test]
    fn invalid_bytes_become_question_marks() {
        let layout = layout_message(b"ab\xE2\x82", 20);
        assert_eq!(texts(&layout), vec!["ab??"]);
    }

    #[test]
    fn trailing_blank_line_preserved() {
        let layout = layout_message(b"a\n\nb", 20);
        assert_eq!(texts(&layout), vec!["a", "", "b"]);
    }
}
