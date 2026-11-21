//! Owns: a replacement template read as Python's `sre_parse.parse_template` reads it (what
//! `match.expand` and `re.sub` take): text, group references (`\1`..`\99`, `\g<n>`,
//! `\g<name>`), the character escapes and octal ones; a template Python refuses refused.
//!
//! Must not: expand anything (the pattern's matches do), or accept what Python refuses.

use super::PatternError;
use std::collections::HashMap;

/// One piece of a template: text, or a group's match (empty when it took no part).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Piece {
    Text(String),
    Group(usize),
}

fn refuse(why: &str) -> PatternError {
    PatternError::Invalid(format!("bad replacement template: {why}"))
}

fn octal(c: Option<&char>) -> bool {
    c.is_some_and(|c| ('0'..='7').contains(c))
}

/// What an escape stands for: a group, or text.
enum Escape {
    Group(usize),
    Text(String),
}

/// `\g<...>` after the `g`, from `k`; the group and where the escape ends.
fn named(chars: &[char], k: usize, names: &HashMap<String, usize>) -> Result<(usize, usize), PatternError> {
    if chars.get(k) != Some(&'<') {
        return Err(refuse("missing <"));
    }
    let close = chars
        .get(k..)
        .unwrap_or_default()
        .iter()
        .position(|c| *c == '>')
        .ok_or_else(|| refuse("missing >"))?;
    let name: String = chars.get(k + 1..k + close).unwrap_or_default().iter().collect();
    let n = if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
        name.parse::<usize>().map_err(|_| refuse("invalid group reference"))?
    } else {
        *names.get(&name).ok_or_else(|| refuse("unknown group name"))?
    };
    Ok((n, k + close + 1))
}

/// `\` then a digit `first`, from `k` (after it): an octal escape, or a group number.
fn numbered(chars: &[char], first: char, k: usize) -> Result<(Escape, usize), PatternError> {
    if first == '0' {
        let mut value = 0u32;
        let mut at = k;
        while at < k + 2 {
            match chars.get(at).filter(|c| octal(Some(c))).and_then(|c| c.to_digit(8)) {
                Some(d) => value = value * 8 + d,
                None => break,
            }
            at += 1;
        }
        let c = char::from_u32(value).ok_or_else(|| refuse("bad octal escape"))?;
        return Ok((Escape::Text(c.to_string()), at));
    }
    if octal(Some(&first)) && octal(chars.get(k)) && octal(chars.get(k + 1)) {
        let digits: String = std::iter::once(first)
            .chain(chars.get(k..k + 2).unwrap_or_default().iter().copied())
            .collect();
        let value = u32::from_str_radix(&digits, 8).map_err(|_| refuse("bad octal escape"))?;
        if value > 0o377 {
            return Err(refuse("octal escape value outside of range 0-0o377"));
        }
        let c = char::from_u32(value).ok_or_else(|| refuse("bad octal escape"))?;
        return Ok((Escape::Text(c.to_string()), k + 2));
    }
    let mut digits = first.to_string();
    let mut at = k;
    if let Some(&d) = chars.get(k).filter(|c| c.is_ascii_digit()) {
        digits.push(d);
        at += 1;
    }
    let n = digits.parse::<usize>().map_err(|_| refuse("invalid group reference"))?;
    Ok((Escape::Group(n), at))
}

/// The escape `\` `c`, the next character at `k`; what it stands for and where it ends.
fn escape(chars: &[char], c: char, k: usize, names: &HashMap<String, usize>) -> Result<(Escape, usize), PatternError> {
    let text = |s: &str| Ok((Escape::Text(s.to_owned()), k));
    match c {
        'g' => named(chars, k, names).map(|(n, at)| (Escape::Group(n), at)),
        '0'..='9' => numbered(chars, c, k),
        'a' => text("\u{7}"),
        'b' => text("\u{8}"),
        'f' => text("\u{c}"),
        'n' => text("\n"),
        'r' => text("\r"),
        't' => text("\t"),
        'v' => text("\u{b}"),
        '\\' => text("\\"),
        c if c.is_ascii_alphabetic() => Err(refuse(&format!("bad escape \\{c}"))),
        c => Ok((Escape::Text(format!("\\{c}")), k)),
    }
}

/// `template` read into pieces, against a pattern of `groups` groups named `names`.
pub(super) fn parse(template: &str, groups: usize, names: &HashMap<String, usize>) -> Result<Vec<Piece>, PatternError> {
    let chars: Vec<char> = template.chars().collect();
    let mut out: Vec<Piece> = Vec::new();
    let mut text = String::new();
    let mut k = 0;
    while let Some(&c) = chars.get(k) {
        k += 1;
        if c != '\\' {
            text.push(c);
            continue;
        }
        let next = *chars.get(k).ok_or_else(|| refuse("a bad escape at the end"))?;
        let (found, at) = escape(&chars, next, k + 1, names)?;
        k = at;
        match found {
            Escape::Text(t) => text.push_str(&t),
            Escape::Group(n) => {
                if n > groups {
                    return Err(refuse("invalid group reference"));
                }
                if !text.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut text)));
                }
                out.push(Piece::Group(n));
            }
        }
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{parse, Piece};
    use std::collections::HashMap;

    #[test]
    fn templates_read_as_python_reads_them() {
        let names = HashMap::from([("year".to_owned(), 2)]);
        let got = parse(r"\2-\g<year>\g<1>\n\101x\-", 2, &names).unwrap();
        assert_eq!(
            got,
            vec![
                Piece::Group(2),
                Piece::Text("-".to_owned()),
                Piece::Group(2),
                Piece::Group(1),
                Piece::Text("\nAx\\-".to_owned()),
            ]
        );
        assert!(parse(r"\3", 2, &names).is_err());
        assert!(parse(r"\q", 2, &names).is_err());
        assert!(parse(r"\g<nope>", 2, &names).is_err());
    }
}
