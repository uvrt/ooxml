//! Owns: the character classes markdown-it reads delimiters by (`common/utils.py`):
//! white space, ASCII punctuation, Unicode punctuation and symbols.
//!
//! Must not: hold anything but these tests.

use crate::punct::RANGES;

/// markdown-it's `isWhiteSpace`: Zs and the ASCII controls `\t\n\v\f\r`.
pub(crate) fn is_white_space(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

/// markdown-it's `isMdAsciiPunct` or `isPunctChar` (general categories P and S).
pub(crate) fn is_punctuation(ch: char) -> bool {
    if ch.is_ascii() {
        return ch.is_ascii_punctuation();
    }
    let code = u32::from(ch);
    let at = RANGES.partition_point(|&(_, end)| end < code);
    RANGES.get(at).is_some_and(|&(start, _)| start <= code)
}

/// Python's `str.isspace()` (what `\s` and `str.strip` read as white space): Unicode
/// white space and the information separators U+001C..U+001F.
pub(crate) fn is_py_space(ch: char) -> bool {
    ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch)
}

/// markdown-it's `MD_TRIM_CHARS`: what JavaScript's `trim` takes, U+FEFF aside.
pub(crate) fn is_trim(ch: char) -> bool {
    is_white_space(ch) || matches!(ch, '\u{2028}' | '\u{2029}')
}

/// markdown-it's `mdTrim`.
pub(crate) fn md_trim(text: &str) -> &str {
    text.trim_matches(is_trim)
}

/// markdown-it's `isStrSpace`: a tab or a space.
pub(crate) fn is_str_space(ch: char) -> bool {
    ch == ' ' || ch == '\t'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert!(is_punctuation('*') && is_punctuation('€') && is_punctuation('«') && is_punctuation('+'));
        assert!(!is_punctuation('a') && !is_punctuation('é') && !is_punctuation(' '));
        assert!(is_white_space('\u{2003}') && !is_white_space('\u{200b}'));
    }
}
