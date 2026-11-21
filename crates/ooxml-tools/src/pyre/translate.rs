//! Owns: reading a Python pattern as `re`'s parser (`re._parser`) reads it and writing it
//! in `fancy-regex`'s syntax with the same meaning: flags (global at the start, or scoped),
//! the verbose flag's white space and comments, groups, lookaround, conditionals, repeats,
//! back references and Python's refusals.
//!
//! Must not: write any construct whose meaning in `fancy-regex` is not Python's: such a
//! pattern is [`PatternError::Unported`].

use super::class::{category, literal, word};
use super::PatternError;
use std::collections::HashMap;
use std::fmt::Write as _;

/// The flags in force at a point of a pattern.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Scope {
    pub(super) multiline: bool,
    pub(super) verbose: bool,
    pub(super) ascii: bool,
    pub(super) ignore_case: bool,
}

/// A pattern in `fancy-regex`'s syntax: the text, its capturing groups' count, and its
/// named groups' numbers (Python's numbering, which the translation keeps).
#[derive(Debug, Clone)]
pub struct Translated {
    /// The pattern as `fancy-regex` reads it.
    pub source: String,
    /// How many capturing groups it has.
    pub groups: usize,
    /// Its named groups' numbers.
    pub names: HashMap<String, usize>,
}

/// `pattern` written in `fancy-regex`'s syntax, meaning what Python's `re` means by it
/// (`ignore_case`: compiled with `re.IGNORECASE`).
///
/// # Errors
/// [`PatternError::Invalid`] where Python refuses the pattern, [`PatternError::Unported`]
/// where its meaning here would differ.
pub fn translate(pattern: &str, ignore_case: bool) -> Result<Translated, PatternError> {
    let mut parser = Parser {
        chars: pattern.chars().collect(),
        at: 0,
        out: String::new(),
        groups: 0,
        open: Vec::new(),
        names: HashMap::new(),
        scopes: vec![Scope {
            ignore_case,
            ..Scope::default()
        }],
        conditions: Vec::new(),
    };
    parser.alternation(true)?;
    if parser.at < parser.chars.len() {
        return Err(parser.error("unbalanced parenthesis"));
    }
    if let Some((n, pos)) = parser.conditions.iter().find(|(n, _)| *n > parser.groups) {
        return Err(parser.error_at(&format!("invalid group reference {n}"), *pos));
    }
    Ok(Translated {
        source: parser.out,
        groups: parser.groups,
        names: parser.names,
    })
}

pub(super) struct Parser {
    chars: Vec<char>,
    pub(super) at: usize,
    pub(super) out: String,
    /// Capturing groups opened so far.
    groups: usize,
    /// The groups open now.
    open: Vec<usize>,
    names: HashMap<String, usize>,
    /// The flags in force, innermost last.
    scopes: Vec<Scope>,
    /// The groups conditionals test, with where their numbers start (Python checks them at
    /// the end).
    conditions: Vec<(usize, usize)>,
}

/// What one item of a sequence is, for the repeat after it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    /// Matches text (or is a group): repeatable.
    Text,
    /// An anchor (`^`, `$`, `\A`, `\Z`, `\b`, `\B`): Python refuses to repeat it.
    Anchor,
    /// A lookaround: repeatable in Python; written in a group so that it is here too.
    Look,
    /// Nothing at all (a comment, the global flags).
    Nothing,
}

/// Python's `MAXREPEAT`: a repeat's counts must be below it.
const MAX_REPEAT: u128 = 4_294_967_295;
/// Python's verbose white space.
const WHITESPACE: &str = " \t\n\r\x0B\x0C";
/// Python's flag letters.
const FLAGS: &str = "iLmsxatu";

impl Parser {
    pub(super) fn rest(&self, from: usize) -> &[char] {
        self.chars.get(from..).unwrap_or(&[])
    }

    pub(super) fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    pub(super) fn peek_at(&self, k: usize) -> Option<char> {
        self.chars.get(self.at + k).copied()
    }

    pub(super) fn next(&mut self) -> Option<char> {
        let c = self.peek();
        self.at += usize::from(c.is_some());
        c
    }

    pub(super) fn eat(&mut self, text: &str) -> bool {
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

    pub(super) fn scope(&self) -> Scope {
        self.scopes.last().copied().unwrap_or_default()
    }

    /// Python's refusal of the pattern (`re.error`) where the next token starts, saying why.
    pub(super) fn error(&self, why: &str) -> PatternError {
        self.error_at(why, self.at)
    }

    /// Python's refusal of the pattern at `pos` (a character index), worded as `re.error`
    /// words it: the position, and the line and column in a pattern of several lines.
    pub(super) fn error_at(&self, why: &str, pos: usize) -> PatternError {
        let mut message = format!("{why} at position {pos}");
        if self.chars.contains(&'\n') {
            let before = self.chars.get(..pos).unwrap_or(&self.chars);
            let line = before.iter().filter(|c| **c == '\n').count() + 1;
            let column = pos - before.iter().rposition(|c| *c == '\n').map_or(0, |k| k + 1) + 1;
            let _ = write!(message, " (line {line}, column {column})");
        }
        PatternError::Invalid(message)
    }

    /// Under the verbose flag: white space and `#` comments skipped.
    fn skip_space(&mut self) {
        if !self.scope().verbose {
            return;
        }
        while let Some(c) = self.peek() {
            if WHITESPACE.contains(c) {
                self.next();
            } else if c == '#' {
                while let Some(c) = self.next() {
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    /// Branches separated by `|`, up to a `)` or the end; `top`: the pattern's own (its
    /// first branch may start with global flags).
    fn alternation(&mut self, top: bool) -> Result<(), PatternError> {
        self.sequence(top)?;
        while self.peek() == Some('|') {
            self.next();
            self.out.push('|');
            self.sequence(false)?;
        }
        Ok(())
    }

    /// Items up to a `|`, a `)` or the end, each with its repeat; `first`: the pattern's
    /// first branch, where global flags may come before any item.
    fn sequence(&mut self, first: bool) -> Result<(), PatternError> {
        // the last item: where it is written from, what it is, and whether it is repeated
        let mut last: Option<(usize, Item, bool)> = None;
        let mut items = 0usize;
        loop {
            self.skip_space();
            match self.peek() {
                None | Some('|' | ')') => break,
                Some('*' | '+' | '?') => {}
                Some('{') if braces(self.rest(self.at)).is_some() => {}
                Some(_) => {
                    let start = self.out.len();
                    let item = self.atom(first && items == 0)?;
                    if item != Item::Nothing {
                        items += 1;
                        last = Some((start, item, false));
                    }
                    continue;
                }
            }
            match last {
                None | Some((_, Item::Anchor | Item::Nothing, _)) => return Err(self.error("nothing to repeat")),
                Some((_, _, true)) => return Err(self.error("multiple repeat")),
                Some((start, item, false)) => {
                    self.repeat(start, item)?;
                    last = Some((start, item, true));
                }
            }
        }
        Ok(())
    }

    /// The quantifier here, applied to the item written from `start`.
    fn repeat(&mut self, start: usize, item: Item) -> Result<(), PatternError> {
        let written = self.quantifier()?;
        if item == Item::Look {
            self.out.insert_str(start, "(?:");
            self.out.push(')');
        }
        let lazy = self.eat("?");
        let possessive = !lazy && self.eat("+");
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
        Ok(())
    }

    /// The quantifier here (one is), taken: how it is written.
    fn quantifier(&mut self) -> Result<String, PatternError> {
        if let Some(c @ ('*' | '+' | '?')) = self.peek() {
            self.next();
            return Ok(c.to_string());
        }
        let Some((low, high, len)) = braces(self.rest(self.at)) else {
            return Err(self.error("nothing to repeat"));
        };
        if low >= MAX_REPEAT || high.is_some_and(|h| h >= MAX_REPEAT) {
            return Err(PatternError::Overflow("the repetition number is too large".to_owned()));
        }
        if high.is_some_and(|h| h < low) {
            return Err(self.error_at("min repeat greater than max repeat", self.at + 1));
        }
        self.at += len;
        Ok(match high {
            Some(h) if h == low => format!("{{{low}}}"),
            Some(h) => format!("{{{low},{h}}}"),
            None => format!("{{{low},}}"),
        })
    }

    /// One item; `flags_allowed`: global flags may be here.
    fn atom(&mut self, flags_allowed: bool) -> Result<Item, PatternError> {
        let Some(c) = self.next() else {
            return Err(self.error("unexpected end of pattern"));
        };
        match c {
            '(' => self.group(flags_allowed),
            '[' => {
                self.cased()?;
                self.class()?;
                Ok(Item::Text)
            }
            '.' => {
                self.out.push('.');
                Ok(Item::Text)
            }
            '^' => {
                self.out.push('^');
                Ok(Item::Anchor)
            }
            '$' => {
                self.out
                    .push_str(if self.scope().multiline { "$" } else { r"(?=\n?\z)" });
                Ok(Item::Anchor)
            }
            '\\' => self.escape(),
            other => {
                self.cased()?;
                literal(&mut self.out, other);
                Ok(Item::Text)
            }
        }
    }

    /// Refused where Python's matching ignores case by ASCII only (`a` with `i`), which
    /// this engine does not do.
    fn cased(&self) -> Result<(), PatternError> {
        let scope = self.scope();
        if scope.ascii && scope.ignore_case {
            return Err(PatternError::Unported(
                "the flags a and i together are not read here".to_owned(),
            ));
        }
        Ok(())
    }

    fn group(&mut self, flags_allowed: bool) -> Result<Item, PatternError> {
        let start = self.at - 1;
        if !self.eat("?") {
            return self.capture(None, start);
        }
        let question = self.at - 1;
        let Some(c) = self.next() else {
            return Err(self.error("unexpected end of pattern"));
        };
        match c {
            ':' => {
                self.out.push_str("(?:");
                self.close_group(None, start).map(|()| Item::Text)
            }
            'P' => self.python_group(start, question),
            '=' | '!' => self.look(c, "", start),
            '<' => match self.next() {
                Some(d @ ('=' | '!')) => self.look(d, "<", start),
                Some(d) => Err(self.error_at(&format!("unknown extension ?<{}", self.token(d)), question)),
                None => Err(self.error("unexpected end of pattern")),
            },
            '>' => {
                self.out.push_str("(?>");
                self.close_group(None, start).map(|()| Item::Text)
            }
            '#' => {
                while let Some(c) = self.next() {
                    if c == ')' {
                        return Ok(Item::Nothing);
                    }
                }
                Err(self.error_at("missing ), unterminated comment", start))
            }
            '(' => self.conditional(start),
            c if FLAGS.contains(c) || c == '-' => {
                self.at -= 1;
                self.flags(flags_allowed, start)
            }
            c => Err(self.error_at(&format!("unknown extension ?{}", self.token(c)), question)),
        }
    }

    /// The token that starts with `c`, just read: an escape is two characters (Python's
    /// tokens), the second taken.
    fn token(&mut self, c: char) -> String {
        if c == '\\' {
            if let Some(d) = self.next() {
                return format!("\\{d}");
            }
        }
        c.to_string()
    }

    /// `(?P<name>...)` or `(?P=name)` after the `P`.
    fn python_group(&mut self, start: usize, question: usize) -> Result<Item, PatternError> {
        if self.eat("<") {
            let (name, at) = self.name('>')?;
            if let Some(was) = self.names.get(&name) {
                let why = format!(
                    "redefinition of group name {} as group {}; was group {was}",
                    py_repr(&name),
                    self.groups + 1
                );
                return Err(self.error_at(&why, at));
            }
            return self.capture(Some(name), start);
        }
        if self.eat("=") {
            let (name, at) = self.name(')')?;
            let Some(&n) = self.names.get(&name) else {
                return Err(self.error_at(&format!("unknown group name {}", py_repr(&name)), at));
            };
            if self.open.contains(&n) {
                return Err(self.error_at("cannot refer to an open group", at));
            }
            return self.back_reference(n);
        }
        match self.next() {
            Some(c) => Err(self.error_at(&format!("unknown extension ?P{}", self.token(c)), question)),
            None => Err(self.error("unexpected end of pattern")),
        }
    }

    /// A lookaround after its `(?=`, `(?!`, `(?<=` or `(?<!`.
    fn look(&mut self, kind: char, behind: &str, start: usize) -> Result<Item, PatternError> {
        let _ = write!(self.out, "(?{behind}{kind}");
        self.close_group(None, start).map(|()| Item::Look)
    }

    /// A capturing group after its `(` (and name); `start`, where its `(` is.
    fn capture(&mut self, name: Option<String>, start: usize) -> Result<Item, PatternError> {
        self.groups += 1;
        let n = self.groups;
        if let Some(name) = name {
            self.names.insert(name, n);
        }
        self.open.push(n);
        self.out.push('(');
        self.close_group(Some(n), start)?;
        Ok(Item::Text)
    }

    /// `(?(group)yes|no)` after its `(?(`; `start`, where its `(` is.
    fn conditional(&mut self, start: usize) -> Result<Item, PatternError> {
        let (name, at) = self.until(')')?;
        let n = if name.chars().all(|c| c.is_ascii_digit()) {
            let n: usize = name
                .parse()
                .map_err(|_| self.error_at(&format!("invalid group reference {name}"), at))?;
            if n == 0 {
                return Err(self.error_at("bad group number", at));
            }
            n
        } else if is_identifier(&name) {
            *self
                .names
                .get(&name)
                .ok_or_else(|| self.error_at(&format!("unknown group name {}", py_repr(&name)), at))?
        } else {
            return Err(self.error_at(&format!("bad character in group name {}", py_repr(&name)), at));
        };
        self.conditions.push((n, at));
        let _ = write!(self.out, "(?({n})");
        self.sequence(false)?;
        self.out.push('|');
        if self.eat("|") {
            self.sequence(false)?;
            if self.peek() == Some('|') {
                return Err(self.error("conditional backref with more than two branches"));
            }
        }
        if !self.eat(")") {
            return Err(self.error_at("missing ), unterminated subpattern", start));
        }
        self.out.push(')');
        Ok(Item::Text)
    }

    /// Flags after `(?`: global ones (`(?im)`, only before any item of the pattern), or
    /// scoped ones (`(?i:...)`, `(?-i:...)`), read as Python's `_parse_flags` reads them.
    fn flags(&mut self, flags_allowed: bool, start: usize) -> Result<Item, PatternError> {
        let mut on = String::new();
        let mut off = String::new();
        let mut c = self.next();
        if c != Some('-') {
            loop {
                let Some(flag) = c else {
                    return Err(self.error("missing -, : or )"));
                };
                if flag == 'L' {
                    return Err(self.error("bad inline flags: cannot use 'L' flag with a str pattern"));
                }
                on.push(flag);
                if is_type(flag) && on.chars().any(|f| is_type(f) && f != flag) {
                    return Err(self.error("bad inline flags: flags 'a', 'u' and 'L' are incompatible"));
                }
                c = self.next();
                match c {
                    None => return Err(self.error("missing -, : or )")),
                    Some(')' | '-' | ':') => break,
                    Some(x) if !FLAGS.contains(x) => {
                        let why = if x.is_alphabetic() {
                            "unknown flag"
                        } else {
                            "missing -, : or )"
                        };
                        return Err(self.error_at(why, self.at - 1));
                    }
                    Some(_) => {}
                }
            }
        }
        if on.contains('t') {
            return Err(PatternError::Unported("the flag t is not read here".to_owned()));
        }
        if c == Some(')') {
            if !flags_allowed {
                return Err(self.error_at("global flags not at the start of the expression", start));
            }
            for scope in &mut self.scopes {
                apply(scope, &on, "");
            }
            self.write_flags(&on, "", ")");
            return Ok(Item::Nothing);
        }
        if c == Some('-') {
            c = self.next();
            loop {
                match c {
                    None => return Err(self.error(if off.is_empty() { "missing flag" } else { "missing :" })),
                    Some(':') if !off.is_empty() => break,
                    Some(x) if !FLAGS.contains(x) => {
                        let why = match (x.is_alphabetic(), off.is_empty()) {
                            (true, _) => "unknown flag",
                            (false, true) => "missing flag",
                            (false, false) => "missing :",
                        };
                        return Err(self.error_at(why, self.at - 1));
                    }
                    Some(flag) => {
                        if is_type(flag) {
                            return Err(self.error("bad inline flags: cannot turn off flags 'a', 'u' and 'L'"));
                        }
                        off.push(flag);
                    }
                }
                c = self.next();
            }
        }
        if on.chars().any(|f| off.contains(f)) {
            return Err(self.error_at("bad inline flags: flag turned on and off", self.at - 1));
        }
        let mut scope = self.scope();
        apply(&mut scope, &on, &off);
        self.write_flags(&on, &off, ":");
        self.scopes.push(scope);
        let closed = self.close_group(None, start);
        self.scopes.pop();
        closed.map(|()| Item::Text)
    }

    /// The flags `fancy-regex` takes (`i`, `m`, `s`) written as a group's start, `end` its
    /// `)` or `:`; the others are Python's that the translation itself carries out.
    fn write_flags(&mut self, on: &str, off: &str, end: &str) {
        let keep = |flags: &str| -> String { flags.chars().filter(|f| "ims".contains(*f)).collect() };
        let (on, off) = (keep(on), keep(off));
        match (on.is_empty() && off.is_empty(), end) {
            (true, ")") => {}
            (true, _) => self.out.push_str("(?:"),
            (false, _) => {
                let dash = if off.is_empty() { "" } else { "-" };
                let _ = write!(self.out, "(?{on}{dash}{off}{end}");
            }
        }
    }

    /// A group's content and its `)`; `capture`, the number of the capturing group it
    /// closes (no longer open after it); `start`, where its `(` is.
    fn close_group(&mut self, capture: Option<usize>, start: usize) -> Result<(), PatternError> {
        self.alternation(false)?;
        if self.next() != Some(')') {
            return Err(self.error_at("missing ), unterminated subpattern", start));
        }
        self.out.push(')');
        if let Some(n) = capture {
            self.open.retain(|k| *k != n);
        }
        Ok(())
    }

    /// A group's name up to `end` (Python's `getuntil` and `checkgroupname`): the name
    /// and where it starts.
    fn name(&mut self, end: char) -> Result<(String, usize), PatternError> {
        let (name, at) = self.until(end)?;
        if is_identifier(&name) {
            Ok((name, at))
        } else {
            Err(self.error_at(&format!("bad character in group name {}", py_repr(&name)), at))
        }
    }

    /// The text up to `end`, taken with it (Python's `getuntil`): the text and where it
    /// starts.
    fn until(&mut self, end: char) -> Result<(String, usize), PatternError> {
        let at = self.at;
        let mut text = String::new();
        loop {
            match self.next() {
                Some(c) if c == end => {
                    if text.is_empty() {
                        return Err(self.error_at("missing group name", self.at - 1));
                    }
                    return Ok((text, at));
                }
                Some('\\') => {
                    text.push('\\');
                    if let Some(d) = self.next() {
                        text.push(d);
                    }
                }
                Some(c) => text.push(c),
                None if text.is_empty() => return Err(self.error("missing group name")),
                None => return Err(self.error_at(&format!("missing {end}, unterminated name"), at)),
            }
        }
    }

    /// A reference to group `n`, checked (open, unknown) where `\\n` or `(?P=` read it.
    fn back_reference(&mut self, n: usize) -> Result<Item, PatternError> {
        self.cased()?;
        let _ = write!(self.out, r"(?:\{n})");
        Ok(Item::Text)
    }

    fn escape(&mut self) -> Result<Item, PatternError> {
        let start = self.at - 1;
        let Some(c) = self.next() else {
            return Err(self.error_at("bad escape (end of pattern)", start));
        };
        let ascii = self.scope().ascii;
        match c {
            'A' => self.out.push_str(r"\A"),
            'Z' => self.out.push_str(r"\z"),
            'b' | 'B' => {
                let w = word(ascii);
                let b = if c == 'b' {
                    format!("(?:(?<=[{w}])(?![{w}])|(?<![{w}])(?=[{w}]))")
                } else {
                    format!("(?:(?<=[{w}])(?=[{w}])|(?<![{w}])(?![{w}]))")
                };
                self.out.push_str(&b);
            }
            _ => {
                if let Some((set, negated)) = category(c, ascii) {
                    let caret = if negated { "^" } else { "" };
                    let _ = write!(self.out, "[{caret}{set}]");
                    return Ok(Item::Text);
                }
                return match self.character_escape(c, start)? {
                    Some(value) => {
                        self.cased()?;
                        literal(&mut self.out, value);
                        Ok(Item::Text)
                    }
                    None => {
                        // a group's number: one digit, or two
                        let mut digits = c.to_string();
                        if let Some(d) = self.peek().filter(char::is_ascii_digit) {
                            digits.push(d);
                            self.next();
                        }
                        let n: usize = digits.parse().unwrap_or(0);
                        if n > self.groups {
                            return Err(self.error_at(&format!("invalid group reference {n}"), start + 1));
                        }
                        if self.open.contains(&n) {
                            return Err(self.error_at("cannot refer to an open group", start));
                        }
                        self.back_reference(n)
                    }
                };
            }
        }
        Ok(Item::Anchor)
    }
}

/// `text` as Python's `repr` writes a `str`.
fn py_repr(text: &str) -> String {
    crate::pyfmt::repr(&serde_json::Value::from(text))
}

/// The scope `on` and `off` (flag letters) make of `scope`.
fn apply(scope: &mut Scope, on: &str, off: &str) {
    for (flags, value) in [(on, true), (off, false)] {
        for f in flags.chars() {
            match f {
                'i' => scope.ignore_case = value,
                'm' => scope.multiline = value,
                'x' => scope.verbose = value,
                'a' => scope.ascii = true,
                'u' => scope.ascii = false,
                _ => {}
            }
        }
    }
}

/// Python's type flags (`a`, `u`, `L`): one at most.
fn is_type(flag: char) -> bool {
    matches!(flag, 'a' | 'u' | 'L')
}

/// Python's `str.isidentifier()`, near enough for group names: a letter or `_`, then
/// letters, digits or `_`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic()) && chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// `{m}`, `{m,}`, `{,n}`, `{m,n}`, `{,}` at the start of `chars`, as Python reads a repeat:
/// its least and most counts and its length; `None` where Python reads `{` as itself.
fn braces(chars: &[char]) -> Option<(u128, Option<u128>, usize)> {
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
    let count = |digits: &str| digits.parse::<u128>().unwrap_or(u128::MAX);
    let low_n = if low.is_empty() { 0 } else { count(&low) };
    let high_n = if high.is_empty() { None } else { Some(count(&high)) };
    Some((low_n, high_n, k + 1))
}
