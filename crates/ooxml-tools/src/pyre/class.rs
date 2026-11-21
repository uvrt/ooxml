//! Owns: a character class (`[...]`) and the escapes that stand for one character, read as
//! Python's `re` parser (`_parse`'s set branch, `_class_escape`, `_escape`) reads them and
//! written in `fancy-regex`'s syntax with the same meaning; Python's `\w`, `\d` and `\s`
//! written out as `str` patterns mean them (Unicode, or ASCII under the flag `a`).
//!
//! Must not: write a class or escape whose meaning here is not Python's.

use super::translate::Parser;
use super::PatternError;

/// Python's `\w` on `str`: `str.isalnum()` (letters and numbers) or `_`.
const WORD: &str = r"\p{L}\p{N}_";
/// Python's `\s` on `str`: `str.isspace()`, Unicode's white space and the four
/// information separators.
const SPACE: &str = r"\s\x{1C}-\x{1F}";
/// `\w`, `\d` and `\s` under the flag `a`.
const ASCII_WORD: &str = "A-Za-z0-9_";
const ASCII_DIGIT: &str = "0-9";
const ASCII_SPACE: &str = r" \t\n\r\x0C\x0B";

/// A category escape (`\w`, `\W`, `\d`, `\D`, `\s`, `\S`) as the inside of a class: the
/// set, and whether it is the set's complement.
pub(super) fn category(c: char, ascii: bool) -> Option<(&'static str, bool)> {
    let set = match (c.to_ascii_lowercase(), ascii) {
        ('w', false) => WORD,
        ('w', true) => ASCII_WORD,
        ('d', false) => r"\d",
        ('d', true) => ASCII_DIGIT,
        ('s', false) => SPACE,
        ('s', true) => ASCII_SPACE,
        _ => return None,
    };
    Some((set, c.is_ascii_uppercase()))
}

/// The word characters `\b` and `\B` look at.
pub(super) fn word(ascii: bool) -> &'static str {
    if ascii {
        ASCII_WORD
    } else {
        WORD
    }
}

/// An element of a class: a character, or a set written out.
enum Item {
    Char(char),
    Set(String),
}

impl Parser {
    /// The character an escape that stands for one character means outside a class (`\n`,
    /// `\x41`, `\.`, octal `\0` and `\101`), `c` the character after the backslash; `None`
    /// for a digit escape that is a group reference (the caller reads it).
    pub(super) fn character_escape(&mut self, c: char) -> Result<Option<char>, PatternError> {
        match c {
            '0' => Ok(Some(self.octal(c, 2)?)),
            '1'..='9' => {
                let (d1, d2) = (self.peek(), self.peek_at(1));
                if is_octal(c) && d1.is_some_and(is_octal) && d2.is_some_and(is_octal) {
                    return Ok(Some(self.octal(c, 2)?));
                }
                Ok(None)
            }
            _ => self.simple_escape(c).map(Some),
        }
    }

    /// An escape inside a class: `\b` is a backspace, a digit an octal escape.
    fn class_escape(&mut self, c: char, start: usize) -> Result<char, PatternError> {
        match c {
            'b' => Ok('\u{8}'),
            '0'..='7' => self.octal(c, start),
            '8' | '9' => Err(self.error_at(&format!("bad escape \\{c}"), start)),
            _ => self.simple_escape(c, start),
        }
    }

    /// `\` `first` and up to two octal digits after it: the character, refused past 0o377.
    fn octal(&mut self, first: char, start: usize) -> Result<char, PatternError> {
        let mut digits = first.to_string();
        while digits.len() < 3 {
            match self.peek().filter(|d| is_octal(*d)) {
                Some(d) => {
                    digits.push(d);
                    self.next();
                }
                None => break,
            }
        }
        let value = u32::from_str_radix(&digits, 8).unwrap_or(u32::MAX);
        if value > 0o377 {
            return Err(self.error_at(
                &format!("octal escape value \\{digits} outside of range 0-0o377"),
                start,
            ));
        }
        char::from_u32(value).ok_or_else(|| self.error_at("bad escape", start))
    }

    fn simple_escape(&mut self, c: char, start: usize) -> Result<char, PatternError> {
        match c {
            'a' => Ok('\u{7}'),
            'f' => Ok('\u{c}'),
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            'v' => Ok('\u{b}'),
            'x' => self.hex(2, "x", start),
            'u' => self.hex(4, "u", start),
            'U' => self.hex(8, "U", start),
            'N' => Err(PatternError::unported("named characters (\\N{...}) are not read here")),
            c if c.is_ascii_alphanumeric() => Err(self.error_at(&format!("bad escape \\{c}"), start)),
            other => Ok(other),
        }
    }

    fn hex(&mut self, n: usize, letter: &str, start: usize) -> Result<char, PatternError> {
        let digits: String = self
            .rest(self.at)
            .iter()
            .take(n)
            .take_while(|d| d.is_ascii_hexdigit())
            .collect();
        self.at += digits.chars().count();
        if digits.chars().count() != n {
            return Err(self.error_at(&format!("incomplete escape \\{letter}{digits}"), start));
        }
        u32::from_str_radix(&digits, 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| self.error_at(&format!("bad escape \\{letter}{digits}"), start))
    }

    /// A class after its `[`, written out element by element.
    pub(super) fn class(&mut self) -> Result<(), PatternError> {
        let here = self.at - 1;
        self.out.push('[');
        if self.eat("^") {
            self.out.push('^');
        }
        let mut first = true;
        loop {
            let Some(c) = self.next() else {
                return Err(self.error_at("unterminated character set", here));
            };
            if c == ']' && !first {
                break;
            }
            first = false;
            let (low, this) = self.class_item(c)?;
            if self.peek() == Some('-') {
                self.next();
                let Some(that) = self.next() else {
                    return Err(self.error_at("unterminated character set", here));
                };
                if that == ']' {
                    self.class_write(low);
                    self.out.push_str(r"\-");
                    break;
                }
                let (high, that) = self.class_item(that)?;
                match (low, high) {
                    (Item::Char(a), Item::Char(b)) if a <= b => {
                        class_literal(&mut self.out, a);
                        self.out.push('-');
                        class_literal(&mut self.out, b);
                    }
                    _ => {
                        // Python's position: back from here by the two tokens and the dash
                        let back = this.chars().count() + 1 + that.chars().count();
                        return Err(self.error_at(&format!("bad character range {this}-{that}"), self.at - back));
                    }
                }
                continue;
            }
            self.class_write(low);
        }
        self.out.push(']');
        Ok(())
    }

    /// The element `c` starts, and its token (Python's: a character, or an escape's two).
    fn class_item(&mut self, c: char) -> Result<(Item, String), PatternError> {
        if c != '\\' {
            return Ok((Item::Char(c), c.to_string()));
        }
        let start = self.at - 1;
        let Some(e) = self.next() else {
            return Err(self.error_at("bad escape (end of pattern)", start));
        };
        let token = format!("\\{e}");
        if let Some((set, negated)) = category(e, self.scope().ascii) {
            let set = if negated { format!("[^{set}]") } else { set.to_owned() };
            return Ok((Item::Set(set), token));
        }
        self.class_escape(e, start).map(|c| (Item::Char(c), token))
    }

    fn class_write(&mut self, item: Item) {
        match item {
            Item::Char(c) => class_literal(&mut self.out, c),
            Item::Set(s) => self.out.push_str(&s),
        }
    }
}

fn is_octal(c: char) -> bool {
    ('0'..='7').contains(&c)
}

/// `c` as a literal outside a class.
pub(super) fn literal(out: &mut String, c: char) {
    if r"\.+*?()|[]{}^$#&-~".contains(c) {
        out.push('\\');
    }
    out.push(c);
}

/// `c` as a literal inside a class.
fn class_literal(out: &mut String, c: char) {
    if r"\[]^-&~".contains(c) {
        out.push('\\');
    }
    out.push(c);
}
