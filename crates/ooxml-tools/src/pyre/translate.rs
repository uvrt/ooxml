//! Owns: reading a Python pattern as `re`'s parser (`sre_parse`) reads it and writing it in
//! `fancy-regex`'s syntax with the same meaning, with the shortest match it can make.
//!
//! Must not: write any construct whose meaning in `fancy-regex` is not Python's: such a
//! pattern is [`Unsupported`].

use super::Unsupported;
use std::collections::HashMap;
use std::fmt::Write as _;

/// Python's `\w` on `str`: `str.isalnum()` (letters and numbers) or `_`.
const WORD: &str = r"\p{L}\p{N}_";
/// Python's `\s` on `str`: `str.isspace()`, Unicode's white space and the four
/// information separators.
const SPACE: &str = r"\s\x{1C}-\x{1F}";

/// `pattern` written in `fancy-regex`'s syntax, meaning what Python's `re` means by it.
///
/// # Errors
/// [`Unsupported`] with why: Python refuses it, its meaning would differ, or it can match
/// the empty string.
pub fn translate(pattern: &str) -> Result<String, Unsupported> {
    let mut parser = Parser {
        chars: pattern.chars().collect(),
        at: 0,
        out: String::new(),
        groups: 0,
        open: Vec::new(),
        names: HashMap::new(),
        multiline: Vec::new(),
    };
    parser.leading_flags()?;
    let shortest = parser.alternation()?;
    if parser.at < parser.chars.len() {
        return Err(refuse("unbalanced parenthesis"));
    }
    if shortest == 0 {
        return Err(refuse(
            "the pattern can match nothing (empty), and Python's matches after an empty one may differ",
        ));
    }
    Ok(parser.out)
}

fn refuse(why: &str) -> Unsupported {
    Unsupported(why.to_owned())
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    out: String,
    /// Capturing groups opened so far.
    groups: usize,
    /// The groups open now.
    open: Vec<usize>,
    names: HashMap<String, usize>,
    /// Whether `$` means a line's end, innermost last (the leading flags first).
    multiline: Vec<bool>,
}

/// What one atom is: its shortest match, and whether it matches no character at all (an
/// anchor or a lookaround), which Python does not repeat.
struct Atom {
    shortest: usize,
    zero_width: bool,
}

impl Parser {
    fn rest(&self, from: usize) -> &[char] {
        self.chars.get(from..).unwrap_or(&[])
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn peek_at(&self, k: usize) -> Option<char> {
        self.chars.get(self.at + k).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek();
        self.at += usize::from(c.is_some());
        c
    }

    fn eat(&mut self, text: &str) -> bool {
        let n = text.chars().count();
        let found = self
            .chars
            .get(self.at..self.at + n)
            .is_some_and(|s| s.iter().copied().eq(text.chars()));
        if found {
            self.at += n;
        }
        found
    }

    fn multiline(&self) -> bool {
        self.multiline.last().copied().unwrap_or(false)
    }

    /// Global flags at the start (`(?im)`): Python takes them only there.
    fn leading_flags(&mut self) -> Result<(), Unsupported> {
        let mut multiline = false;
        while self.peek() == Some('(') && self.peek_at(1) == Some('?') {
            let flags: String = self
                .rest(self.at + 2)
                .iter()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            let end = self.at + 2 + flags.chars().count();
            if flags.is_empty() || self.chars.get(end) != Some(&')') {
                break;
            }
            check_flags(&flags)?;
            multiline |= flags.contains('m');
            let _ = write!(self.out, "(?{flags})");
            self.at = end + 1;
        }
        self.multiline.push(multiline);
        Ok(())
    }

    /// Branches separated by `|`, up to a `)` or the end: the shortest of their shortest.
    fn alternation(&mut self) -> Result<usize, Unsupported> {
        let mut shortest = self.sequence()?;
        while self.peek() == Some('|') {
            self.next();
            self.out.push('|');
            shortest = shortest.min(self.sequence()?);
        }
        Ok(shortest)
    }

    fn sequence(&mut self) -> Result<usize, Unsupported> {
        let mut total: usize = 0;
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let start = self.out.len();
            let atom = self.atom()?;
            total = total.saturating_add(self.repeat(start, atom)?);
        }
        Ok(total)
    }

    /// The repeat after an atom written from `start`, if any: the shortest it makes.
    fn repeat(&mut self, start: usize, atom: Atom) -> Result<usize, Unsupported> {
        let Some((low, written)) = self.quantifier()? else {
            return Ok(atom.shortest);
        };
        if atom.zero_width {
            return Err(refuse("nothing to repeat"));
        }
        let lazy = self.peek() == Some('?');
        let possessive = !lazy && self.peek() == Some('+');
        if lazy || possessive {
            self.next();
        }
        if possessive {
            self.out.insert_str(start, "(?>");
            self.out.push_str(&written);
            self.out.push(')');
        } else {
            self.out.push_str(&written);
            if lazy {
                self.out.push('?');
            }
        }
        if self.quantifier_ahead() {
            return Err(refuse("multiple repeat"));
        }
        Ok(atom.shortest.saturating_mul(low))
    }

    fn quantifier_ahead(&self) -> bool {
        match self.peek() {
            Some('*' | '+' | '?') => true,
            Some('{') => braces(self.rest(self.at)).is_some(),
            _ => false,
        }
    }

    /// A quantifier here, taken: its least count and how it is written.
    fn quantifier(&mut self) -> Result<Option<(usize, String)>, Unsupported> {
        let found = match self.peek() {
            Some('*') => (0, "*".to_owned(), 1),
            Some('+') => (1, "+".to_owned(), 1),
            Some('?') => (0, "?".to_owned(), 1),
            Some('{') => match braces(self.rest(self.at)) {
                Some((low, high, len)) => {
                    if high.is_some_and(|h| h < low) {
                        return Err(refuse("min repeat greater than max repeat"));
                    }
                    let written = match high {
                        Some(h) if h == low => format!("{{{low}}}"),
                        Some(h) => format!("{{{low},{h}}}"),
                        None => format!("{{{low},}}"),
                    };
                    (low, written, len)
                }
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        self.at += found.2;
        Ok(Some((found.0, found.1)))
    }

    fn atom(&mut self) -> Result<Atom, Unsupported> {
        let one = Atom {
            shortest: 1,
            zero_width: false,
        };
        let anchor = Atom {
            shortest: 0,
            zero_width: true,
        };
        let Some(c) = self.next() else {
            return Err(refuse("unexpected end of pattern"));
        };
        match c {
            '(' => self.group(),
            '[' => {
                self.class()?;
                Ok(one)
            }
            '.' => {
                self.out.push('.');
                Ok(one)
            }
            '^' => {
                self.out.push('^');
                Ok(anchor)
            }
            '$' => {
                self.out.push_str(if self.multiline() { "$" } else { r"(?=\n?\z)" });
                Ok(anchor)
            }
            '*' | '+' | '?' => Err(refuse("nothing to repeat")),
            '{' if braces(self.rest(self.at - 1)).is_some() => Err(refuse("nothing to repeat")),
            '\\' => self.escape(),
            other => {
                literal(&mut self.out, other);
                Ok(one)
            }
        }
    }

    fn group(&mut self) -> Result<Atom, Unsupported> {
        if !self.eat("?") {
            self.groups += 1;
            self.open.push(self.groups);
            self.out.push('(');
            return self.close_group(Some(self.groups));
        }
        if self.eat(":") {
            self.out.push_str("(?:");
            return self.close_group(None);
        }
        if self.eat("P<") {
            let name = self.name('>')?;
            if self.names.contains_key(&name) {
                return Err(refuse("redefinition of group name"));
            }
            self.groups += 1;
            self.names.insert(name, self.groups);
            self.open.push(self.groups);
            self.out.push('(');
            return self.close_group(Some(self.groups));
        }
        if self.eat("P=") {
            let name = self.name(')')?;
            let n = *self.names.get(&name).ok_or_else(|| refuse("unknown group name"))?;
            return self.back_reference(n);
        }
        for look in ["=", "!", "<=", "<!"] {
            if self.eat(look) {
                let _ = write!(self.out, "(?{look}");
                self.close_group(None)?;
                return Ok(Atom {
                    shortest: 0,
                    zero_width: true,
                });
            }
        }
        if self.eat(">") {
            self.out.push_str("(?>");
            return self.close_group(None);
        }
        if self.eat("#") {
            while let Some(c) = self.next() {
                if c == ')' {
                    return Ok(Atom {
                        shortest: 0,
                        zero_width: false,
                    });
                }
            }
            return Err(refuse("missing ), unterminated comment"));
        }
        self.scoped_flags()
    }

    /// `(?i:...)`, `(?-i:...)`, `(?s-i:...)`; a global flag past the start is refused, as
    /// Python refuses it.
    fn scoped_flags(&mut self) -> Result<Atom, Unsupported> {
        let on: String = self
            .rest(self.at)
            .iter()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        self.at += on.chars().count();
        let mut off = String::new();
        if self.eat("-") {
            off = self
                .rest(self.at)
                .iter()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            self.at += off.chars().count();
            if off.is_empty() {
                return Err(refuse("missing flag"));
            }
        }
        if on.is_empty() && off.is_empty() {
            return Err(refuse("unknown extension"));
        }
        check_flags(&on)?;
        check_flags(&off)?;
        if self.peek() == Some(')') {
            return Err(refuse("global flags not at the start of the expression"));
        }
        if !self.eat(":") {
            return Err(refuse("missing :"));
        }
        let multiline = if on.contains('m') {
            true
        } else if off.contains('m') {
            false
        } else {
            self.multiline()
        };
        let dash = if off.is_empty() { "" } else { "-" };
        let _ = write!(self.out, "(?{on}{dash}{off}:");
        self.multiline.push(multiline);
        let atom = self.close_group(None);
        self.multiline.pop();
        atom
    }

    /// A group's content and its `)`; `capture`, the number of the capturing group it
    /// closes (no longer open after it).
    fn close_group(&mut self, capture: Option<usize>) -> Result<Atom, Unsupported> {
        let shortest = self.alternation()?;
        if self.next() != Some(')') {
            return Err(refuse("missing ), unterminated subpattern"));
        }
        self.out.push(')');
        if let Some(n) = capture {
            self.open.retain(|k| *k != n);
        }
        Ok(Atom {
            shortest,
            zero_width: false,
        })
    }

    fn name(&mut self, end: char) -> Result<String, Unsupported> {
        let mut name = String::new();
        loop {
            match self.next() {
                Some(c) if c == end => break,
                Some(c) => name.push(c),
                None => return Err(refuse("missing group name end")),
            }
        }
        let mut chars = name.chars();
        let ok = chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
            && chars.all(|c| c == '_' || c.is_alphanumeric());
        if ok {
            Ok(name)
        } else {
            Err(refuse("bad character in group name"))
        }
    }

    fn back_reference(&mut self, n: usize) -> Result<Atom, Unsupported> {
        if self.open.contains(&n) || n > self.groups {
            return Err(refuse("cannot refer to an open or unknown group"));
        }
        let _ = write!(self.out, r"(?:\{n})");
        Ok(Atom {
            shortest: 0,
            zero_width: false,
        })
    }

    fn escape(&mut self) -> Result<Atom, Unsupported> {
        let one = Atom {
            shortest: 1,
            zero_width: false,
        };
        let anchor = Atom {
            shortest: 0,
            zero_width: true,
        };
        let Some(c) = self.next() else {
            return Err(refuse("bad escape (end of pattern)"));
        };
        let written = match c {
            'A' => return self.write(r"\A", anchor),
            'Z' => return self.write(r"\z", anchor),
            'b' => {
                let b = format!("(?:(?<=[{WORD}])(?![{WORD}])|(?<![{WORD}])(?=[{WORD}]))");
                return self.write(&b, anchor);
            }
            'B' => {
                let b = format!("(?:(?<=[{WORD}])(?=[{WORD}])|(?<![{WORD}])(?![{WORD}]))");
                return self.write(&b, anchor);
            }
            'd' => r"\d".to_owned(),
            'D' => r"\D".to_owned(),
            's' => format!("[{SPACE}]"),
            'S' => format!("[^{SPACE}]"),
            'w' => format!("[{WORD}]"),
            'W' => format!("[^{WORD}]"),
            '1'..='9' => {
                if self.peek().is_some_and(|d| d.is_ascii_digit()) {
                    return Err(refuse("two-digit and octal escapes are not read here"));
                }
                return self.back_reference(c as usize - '0' as usize);
            }
            other => {
                let value = self.character_escape(other)?;
                let mut s = String::new();
                literal(&mut s, value);
                s
            }
        };
        self.out.push_str(&written);
        Ok(one)
    }

    fn write(&mut self, text: &str, atom: Atom) -> Result<Atom, Unsupported> {
        self.out.push_str(text);
        Ok(atom)
    }

    /// The character an escape that stands for one character means (`\n`, `\x41`, `\.`).
    fn character_escape(&mut self, c: char) -> Result<char, Unsupported> {
        let hex = |p: &mut Parser, n: usize| -> Result<char, Unsupported> {
            let digits: String = p.chars.get(p.at..p.at + n).unwrap_or(&[]).iter().collect();
            if digits.chars().count() != n || !digits.chars().all(|d| d.is_ascii_hexdigit()) {
                return Err(refuse("incomplete escape"));
            }
            p.at += n;
            u32::from_str_radix(&digits, 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| refuse("bad escape"))
        };
        match c {
            'a' => Ok('\u{7}'),
            'f' => Ok('\u{c}'),
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            'v' => Ok('\u{b}'),
            'x' => hex(self, 2),
            'u' => hex(self, 4),
            'U' => hex(self, 8),
            c if c.is_ascii_alphanumeric() => Err(refuse("this escape is Python's error or reads otherwise here")),
            other => Ok(other),
        }
    }

    /// A class after its `[`, written out element by element.
    fn class(&mut self) -> Result<(), Unsupported> {
        self.out.push('[');
        if self.eat("^") {
            self.out.push('^');
        }
        let mut first = true;
        loop {
            let Some(c) = self.next() else {
                return Err(refuse("unterminated character set"));
            };
            if c == ']' && !first {
                break;
            }
            first = false;
            let low = self.class_item(c)?;
            if self.peek() == Some('-') && self.peek_at(1).is_some() {
                self.next();
                let Some(that) = self.next() else {
                    return Err(refuse("unterminated character set"));
                };
                if that == ']' {
                    self.class_write(low);
                    self.out.push_str(r"\-");
                    break;
                }
                let high = self.class_item(that)?;
                match (low, high) {
                    (Item::Char(a), Item::Char(b)) if a <= b => {
                        class_literal(&mut self.out, a);
                        self.out.push('-');
                        class_literal(&mut self.out, b);
                    }
                    _ => return Err(refuse("bad character range")),
                }
                continue;
            }
            self.class_write(low);
        }
        self.out.push(']');
        Ok(())
    }

    fn class_item(&mut self, c: char) -> Result<Item, Unsupported> {
        if c != '\\' {
            return Ok(Item::Char(c));
        }
        let Some(e) = self.next() else {
            return Err(refuse("unterminated character set"));
        };
        Ok(match e {
            'd' => Item::Set(r"\d".to_owned()),
            'D' => Item::Set(r"\D".to_owned()),
            's' => Item::Set(SPACE.to_owned()),
            'S' => Item::Set(format!("[^{SPACE}]")),
            'w' => Item::Set(WORD.to_owned()),
            'W' => Item::Set(format!("[^{WORD}]")),
            'b' => Item::Char('\u{8}'),
            other => Item::Char(self.character_escape(other)?),
        })
    }

    fn class_write(&mut self, item: Item) {
        match item {
            Item::Char(c) => class_literal(&mut self.out, c),
            Item::Set(s) => self.out.push_str(&s),
        }
    }
}

/// An element of a class: a character, or a set written out.
enum Item {
    Char(char),
    Set(String),
}

/// `{m}`, `{m,}`, `{,n}`, `{m,n}`, `{,}` at the start of `chars`, as Python reads a repeat:
/// its least and most counts and its length; `None` where Python reads `{` as itself.
fn braces(chars: &[char]) -> Option<(usize, Option<usize>, usize)> {
    if chars.first() != Some(&'{') || chars.get(1) == Some(&'}') {
        return None;
    }
    let mut k = 1;
    let digits = |k: &mut usize| -> String {
        let mut s = String::new();
        while let Some(d) = chars.get(*k).filter(|d| d.is_ascii_digit()) {
            s.push(*d);
            *k += 1;
        }
        s
    };
    let low = digits(&mut k);
    let high = if chars.get(k) == Some(&',') {
        k += 1;
        digits(&mut k)
    } else {
        low.clone()
    };
    if chars.get(k) != Some(&'}') {
        return None;
    }
    let low_n = if low.is_empty() { 0 } else { low.parse().ok()? };
    let high_n = if high.is_empty() {
        None
    } else {
        Some(high.parse().ok()?)
    };
    Some((low_n, high_n, k + 1))
}

/// Flags this engine reads as Python does.
fn check_flags(flags: &str) -> Result<(), Unsupported> {
    match flags.chars().find(|f| !matches!(f, 'i' | 'm' | 's')) {
        None => Ok(()),
        Some(f) => Err(Unsupported(format!(
            "the flag {f} is not read here (only i, m and s are)"
        ))),
    }
}

/// `c` as a literal outside a class.
fn literal(out: &mut String, c: char) {
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
