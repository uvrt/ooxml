//! Owns: a format code read as Excel reads it -- its `;` sections, each section's tokens
//! (dates, times, digits, quoted and escaped text, brackets, locales), whether an `m` is a
//! month or minutes, and what a section shows ([`classify`]) -- equal to ooxml-common's
//! `numfmt.py`.
//!
//! Must not: write a number (a chart's rules are `ooxml-chart`'s; Excel's are the sheets
//! phase's).

/// What a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A run of one date letter (`mmmm`).
    Date,
    /// A run of one time letter (`hh`), or minutes.
    Time,
    /// Elapsed time in brackets, without them.
    Elapsed,
    /// `AM/PM`, `A/P` or `上午/下午`.
    AmPm,
    /// One digit placeholder: `0`, `#` or `?`.
    Digit,
    /// `E+` or `E-`.
    Exponent,
    /// `%`.
    Percent,
    /// `General`.
    General,
    /// `@`.
    Text,
    /// A quoted text, without its quotes.
    Quoted,
    /// The character after a backslash.
    Escaped,
    /// The character after `_`.
    Pad,
    /// The character after `*`.
    Fill,
    /// A colour bracket's inside.
    Colour,
    /// A condition bracket's inside.
    Condition,
    /// A locale bracket's inside after the `$`.
    Locale,
    /// Any other bracket's inside.
    Bracket,
    /// A letter that is no token.
    Letter,
    /// Any other character.
    Literal,
}

/// One token of a section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Its kind.
    pub kind: TokenKind,
    /// Its text.
    pub text: String,
}

impl Token {
    fn new(kind: TokenKind, text: impl Into<String>) -> Token {
        Token {
            kind,
            text: text.into(),
        }
    }
}

/// What a section shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `General`.
    General,
    /// A number.
    Number,
    /// A percentage.
    Percent,
    /// A date.
    Date,
    /// A time.
    Time,
    /// Both.
    DateTime,
    /// Text.
    Text,
}

/// A section's classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classification {
    /// What it shows.
    pub kind: Kind,
    /// Whether its time is elapsed time.
    pub elapsed: bool,
    /// The locale id it states.
    pub lcid: Option<i64>,
}

impl Classification {
    /// Whether it shows a date, a time or both.
    pub fn is_date(&self) -> bool {
        matches!(self.kind, Kind::Date | Kind::Time | Kind::DateTime)
    }
}

const COLOURS: [&str; 8] = ["black", "blue", "cyan", "green", "magenta", "red", "white", "yellow"];
const SYSTEM_DATE: i64 = 0xF800;
const SYSTEM_TIME: i64 = 0xF400;

fn date_letter(c: &str) -> bool {
    matches!(c, "d" | "m" | "y" | "e" | "g" | "b" | "ว" | "ด" | "ป")
}

fn time_letter(c: &str) -> bool {
    matches!(c, "h" | "s" | "ช" | "น" | "ท")
}

/// Python's `str.lower()` of one character.
fn lower(c: char) -> String {
    c.to_lowercase().collect()
}

/// The text of `chars[from..to]`, clamped.
fn slice(chars: &[char], from: usize, to: usize) -> String {
    chars
        .get(from.min(chars.len())..to.min(chars.len()))
        .unwrap_or_default()
        .iter()
        .collect()
}

/// A code's sections, split at each `;` outside quotes and not escaped.
pub fn sections(code: &str) -> Vec<String> {
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let (mut start, mut index, mut quoted) = (0, 0, false);
    while let Some(&c) = chars.get(index) {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && matches!(c, '\\' | '_' | '*') {
            index += 1;
        } else if !quoted && c == ';' {
            out.push(slice(&chars, start, index));
            start = index + 1;
        }
        index += 1;
    }
    out.push(slice(&chars, start, chars.len()));
    out
}

/// The index of the first `target` at or after `from`, else the length.
fn find(chars: &[char], target: char, from: usize) -> usize {
    chars
        .iter()
        .skip(from)
        .position(|&c| c == target)
        .map_or(chars.len(), |p| p + from)
}

/// The length of the AM/PM marker at `index`, if one starts there.
fn ampm_at(chars: &[char], index: usize) -> Option<usize> {
    for marker in ["AM/PM", "A/P"] {
        let size = marker.chars().count();
        if slice(chars, index, index + size).to_uppercase() == marker {
            return Some(size);
        }
    }
    let exact: Vec<char> = "上午/下午".chars().collect();
    chars
        .get(index..index + exact.len())
        .filter(|s| *s == exact.as_slice())
        .map(|_| exact.len())
}

/// A section's tokens, with each `m` decided between month and minutes.
pub fn tokens(section: &str) -> Vec<Token> {
    let chars: Vec<char> = section.chars().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while let Some(&c) = chars.get(index) {
        let low = lower(c);
        if c == '"' {
            let close = find(&chars, '"', index + 1);
            out.push(Token::new(TokenKind::Quoted, slice(&chars, index + 1, close)));
            index = close + 1;
        } else if matches!(c, '\\' | '_' | '*') {
            let kind = match c {
                '\\' => TokenKind::Escaped,
                '_' => TokenKind::Pad,
                _ => TokenKind::Fill,
            };
            out.push(Token::new(kind, slice(&chars, index + 1, index + 2)));
            index += 2;
        } else if c == '[' {
            let close = find(&chars, ']', index + 1);
            out.push(bracket(&slice(&chars, index + 1, close)));
            index = close + 1;
        } else if slice(&chars, index, index + 7).to_lowercase() == "general" {
            out.push(Token::new(TokenKind::General, slice(&chars, index, index + 7)));
            index += 7;
        } else if let Some(size) = ampm_at(&chars, index) {
            out.push(Token::new(TokenKind::AmPm, slice(&chars, index, index + size)));
            index += size;
        } else if low == "e" && matches!(chars.get(index + 1), Some('+' | '-')) {
            out.push(Token::new(TokenKind::Exponent, slice(&chars, index, index + 2)));
            index += 2;
        } else if date_letter(&low) || time_letter(&low) {
            let mut end = index;
            while chars.get(end).is_some_and(|&e| lower(e) == low) {
                end += 1;
            }
            let kind = if time_letter(&low) {
                TokenKind::Time
            } else {
                TokenKind::Date
            };
            out.push(Token::new(kind, slice(&chars, index, end)));
            index = end;
        } else {
            let kind = match c {
                '0' | '#' | '?' => TokenKind::Digit,
                '%' => TokenKind::Percent,
                '@' => TokenKind::Text,
                _ if c.is_alphabetic() => TokenKind::Letter,
                _ => TokenKind::Literal,
            };
            out.push(Token::new(kind, c.to_string()));
            index += 1;
        }
    }
    minutes(out)
}

fn bracket(inner: &str) -> Token {
    let low = inner.to_lowercase();
    let mut chars = low.chars();
    if let Some(first) = chars.next() {
        if "hms".contains(first) && low.chars().all(|c| c == first) {
            return Token::new(TokenKind::Elapsed, inner);
        }
    }
    if let Some(first) = inner.chars().next() {
        if time_letter(&first.to_string()) && inner.chars().all(|c| c == first) {
            return Token::new(TokenKind::Elapsed, inner);
        }
    }
    if let Some(rest) = inner.strip_prefix('$') {
        return Token::new(TokenKind::Locale, rest);
    }
    let numbered = low.strip_prefix("color").is_some_and(ooxml_base::pystr::str_is_digit);
    if COLOURS.contains(&low.as_str()) || numbered {
        return Token::new(TokenKind::Colour, inner);
    }
    if inner.starts_with(['<', '>', '=']) {
        return Token::new(TokenKind::Condition, inner);
    }
    Token::new(TokenKind::Bracket, inner)
}

/// A token's role in deciding minutes: `h`, `s`, `m`, other, or none.
fn role(token: &Token) -> Option<char> {
    match token.kind {
        TokenKind::Date | TokenKind::Time | TokenKind::Elapsed => {
            let first = token.text.chars().next().map(lower).unwrap_or_default();
            Some(match first.as_str() {
                "h" | "ช" => 'h',
                "s" | "ท" => 's',
                "m" if token.kind != TokenKind::Elapsed => 'm',
                _ => 'o',
            })
        }
        TokenKind::AmPm => Some('o'),
        _ => None,
    }
}

fn minutes(mut found: Vec<Token>) -> Vec<Token> {
    let marks: Vec<(usize, char)> = found
        .iter()
        .enumerate()
        .filter_map(|(i, t)| Some((i, role(t)?)))
        .collect();
    for (at, &(position, mark)) in marks.iter().enumerate() {
        if mark != 'm' {
            continue;
        }
        let before = at.checked_sub(1).and_then(|i| marks.get(i)).map(|m| m.1);
        let after = marks.get(at + 1).map(|m| m.1);
        let kind = if before == Some('h') || after == Some('s') {
            TokenKind::Time
        } else {
            TokenKind::Date
        };
        if let Some(token) = found.get_mut(position) {
            token.kind = kind;
        }
    }
    found
}

/// A locale bracket's id: the hexadecimal after its last `-`.
fn locale_id(text: &str) -> Option<i64> {
    let (_, tail) = text.rsplit_once('-')?;
    i64::from_str_radix(tail, 16).ok()
}

fn system_id(text: &str) -> Option<i64> {
    let language = text.split_once('-').map(|(_, l)| l.to_lowercase()).unwrap_or_default();
    if language == "x-sysdate" {
        return Some(SYSTEM_DATE);
    }
    if language == "x-systime" {
        return Some(SYSTEM_TIME);
    }
    locale_id(text)
        .map(|n| n & 0xFFFF)
        .filter(|n| matches!(*n, SYSTEM_DATE | SYSTEM_TIME))
}

/// What section `section` of `code` shows (the first where `None`); `None` when the code has
/// no such section.
pub fn classify(code: &str, section: Option<usize>) -> Option<Classification> {
    let parts = sections(code);
    let found = tokens(parts.get(section.unwrap_or(0))?);
    let (mut date, mut time, mut elapsed, mut percent, mut text, mut general, mut digits) =
        (false, false, false, false, false, false, false);
    let mut lcid = None;
    let mut systems = Vec::new();
    for token in &found {
        match token.kind {
            TokenKind::Date => date = true,
            TokenKind::Time | TokenKind::AmPm => time = true,
            TokenKind::Elapsed => (time, elapsed) = (true, true),
            TokenKind::Locale => {
                if let Some(number) = locale_id(&token.text) {
                    lcid = Some(number);
                }
                if section.unwrap_or(0) == 0 {
                    systems.push(system_id(&token.text));
                }
            }
            TokenKind::Digit | TokenKind::Exponent => digits = true,
            TokenKind::Percent => percent = true,
            TokenKind::Text => text = true,
            TokenKind::General => general = true,
            _ => {}
        }
    }
    if !text {
        date = date || systems.contains(&Some(SYSTEM_DATE));
        time = time || systems.contains(&Some(SYSTEM_TIME));
    }
    let (kind, elapsed) = match () {
        () if date && time => (Kind::DateTime, elapsed),
        () if date => (Kind::Date, false),
        () if time => (Kind::Time, elapsed),
        () if percent => (Kind::Percent, false),
        () if text && !digits => (Kind::Text, false),
        () if general && !digits => (Kind::General, false),
        () => (Kind::Number, false),
    };
    Some(Classification { kind, elapsed, lcid })
}

/// Whether section `section` of `code` (the first where `None`) shows a date or a time; a
/// missing code or section shows none.
pub fn is_date_code(code: Option<&str>, section: Option<usize>) -> bool {
    match code {
        Some(code) if !code.is_empty() => classify(code, section).is_some_and(|c| c.is_date()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, sections, tokens, Kind, TokenKind};

    #[test]
    fn codes_classify_as_excel_shows_them() {
        for (code, kind) in [
            ("General", Kind::General),
            ("0", Kind::Number),
            ("0%", Kind::Percent),
            ("@", Kind::Text),
            ("yyyy-mm-dd", Kind::Date),
            ("h:mm", Kind::Time),
            ("mm:ss", Kind::Time),
            ("[h]:mm:ss", Kind::Time),
            ("dd/mm/yyyy hh:mm", Kind::DateTime),
            ("[>=100]d;0", Kind::Date),
            ("[$-x-sysdate]dddd", Kind::Date),
        ] {
            assert_eq!(classify(code, None).map(|c| c.kind), Some(kind), "{code}");
        }
        assert_eq!(sections(r#"0;"a;b";\;"#), vec!["0", r#""a;b""#, r"\;"]);
    }

    #[test]
    fn a_numbered_colour_takes_what_python_reads_as_digits() {
        // `low[5:].isdigit()`: a superscript digit is one, a vulgar fraction is not
        for (code, kind) in [
            ("[Color10]0", TokenKind::Colour),
            ("[Color\u{b2}]0", TokenKind::Colour),
            ("[Color\u{bd}]0", TokenKind::Bracket),
            ("[Color]0", TokenKind::Bracket),
        ] {
            assert_eq!(tokens(code)[0].kind, kind, "{code}");
        }
    }
}
