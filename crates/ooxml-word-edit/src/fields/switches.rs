//! Owns: a field instruction's switches as docx-agent reads them (`edit/fields.py`
//! `switches`, `toc_options`): `\o "1-3"` an argument, `\h` a flag, `\* ARABIC` an
//! argument after `\@`, `\#` or `\*`, a number after a letter; and what a table of
//! contents' instruction asks to list.
//!
//! Must not: compute a field's value (`ooxml_word_model::fieldtext` formats numbers).

/// A switch's argument: a flag, or text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Argument {
    /// Given alone.
    Flag,
    /// With an argument.
    Text(String),
}

/// The tokens: quoted strings, `\` and one character, runs of anything else but space,
/// `\` and `"` (an unclosed quote is skipped).
fn tokens(instruction: &str) -> Vec<String> {
    let chars: Vec<char> = instruction.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while let Some(&c) = chars.get(k) {
        if c == '"' {
            if let Some(end) = chars.iter().skip(k + 1).position(|d| *d == '"') {
                out.push(chars.get(k..=k + 1 + end).unwrap_or_default().iter().collect());
                k += end + 2;
            } else {
                k += 1;
            }
        } else if c == '\\' && chars.get(k + 1).is_some_and(|d| *d != '\n') {
            out.push(chars.get(k..k + 2).unwrap_or_default().iter().collect());
            k += 2;
        } else if c.is_whitespace() || c == '\\' {
            k += 1;
        } else {
            let end = chars
                .iter()
                .skip(k)
                .position(|d| d.is_whitespace() || *d == '\\' || *d == '"')
                .map_or(chars.len(), |p| k + p);
            out.push(chars.get(k..end).unwrap_or_default().iter().collect());
            k = end;
        }
    }
    out
}

/// A field's switches in order, the first of a key winning (`switches`): the key is the
/// letter lower-cased, or `@`, `#`, `*`, `!`.
pub(crate) fn switches(instruction: &str) -> Vec<(char, Argument)> {
    let tokens = tokens(instruction);
    let mut out: Vec<(char, Argument)> = Vec::new();
    let mut k = 0;
    while let Some(token) = tokens.get(k) {
        k += 1;
        let mut chars = token.chars();
        let (Some('\\'), Some(flag), None) = (chars.next(), chars.next(), chars.next()) else {
            continue;
        };
        let key = if "@#*!".contains(flag) {
            flag
        } else {
            flag.to_lowercase().next().unwrap_or(flag)
        };
        let mut argument = Argument::Flag;
        if let Some(following) = tokens.get(k).filter(|t| !t.starts_with('\\')) {
            let digits = !following.is_empty() && following.chars().all(|c| c.is_ascii_digit());
            if following.starts_with('"') || "@#*".contains(flag) || digits {
                argument = Argument::Text(following.trim_matches('"').to_owned());
                k += 1;
            }
        }
        if !out.iter().any(|(k, _)| *k == key) {
            out.push((key, argument));
        }
    }
    out
}

/// Whether the instruction has the switch `key`.
pub(crate) fn has(instruction: &str, key: char) -> bool {
    switches(instruction).iter().any(|(k, _)| *k == key)
}

/// The text argument of the switch `key`, if it has one.
pub(crate) fn switch(instruction: &str, key: char) -> Option<String> {
    switches(instruction).into_iter().find_map(|(k, a)| match a {
        Argument::Text(text) if k == key => Some(text),
        _ => None,
    })
}

/// What a table of contents lists (`toc_options`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TocOptions {
    /// `\o`'s levels (`\o` alone, or neither `\o` nor `\t`: 1-9).
    pub(crate) outline: Option<(i64, i64)>,
    /// `\u`: paragraphs' own levels.
    pub(crate) own: bool,
    /// Every `\t`'s style names and levels, in order.
    pub(crate) styles: Vec<(String, i64)>,
}

/// What a TOC field's instruction asks to list, `\t`'s list split on `separator`.
pub(crate) fn toc_options(instruction: &str, separator: &str) -> TocOptions {
    let options = switches(instruction);
    let tokens = tokens(instruction);
    let named: Vec<String> = tokens
        .windows(2)
        .filter_map(|pair| match pair {
            [t, next] if t.to_lowercase() == "\\t" && !next.starts_with('\\') => {
                Some(next.trim_matches('"').to_owned())
            }
            _ => None,
        })
        .collect();
    let mut styles = Vec::new();
    for value in &named {
        let items: Vec<&str> = value.split(separator).collect();
        for pair in items.chunks(2) {
            let name = pair.first().map_or("", |n| n.trim());
            let raw = pair.get(1).map_or("", |l| l.trim());
            let level = raw
                .parse::<i64>()
                .ok()
                .filter(|l| raw.chars().all(|c| c.is_ascii_digit()) && (1..=9).contains(l))
                .unwrap_or(1);
            if !name.is_empty() {
                styles.push((name.to_owned(), level));
            }
        }
    }
    let outline = match options.iter().find(|(k, _)| *k == 'o').map(|(_, a)| a) {
        Some(Argument::Flag) => Some((1, 9)),
        None if named.is_empty() => Some((1, 9)),
        None => None,
        Some(Argument::Text(span)) => levels(span),
    };
    TocOptions {
        outline,
        own: options.iter().any(|(k, _)| *k == 'u'),
        styles,
    }
}

/// `a-b` with digits 1-9 and spaces around them.
fn levels(span: &str) -> Option<(i64, i64)> {
    let (a, b) = span.split_once('-')?;
    let digit = |s: &str| {
        let s = s.trim();
        (s.len() == 1)
            .then(|| s.parse::<i64>().ok())
            .flatten()
            .filter(|d| (1..=9).contains(d))
    };
    Some((digit(a)?, digit(b)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_as_docx_agent_reads_them() {
        let found = switches(r#" TOC \o "1-3" \h \z \u "#);
        assert_eq!(found.first(), Some(&('o', Argument::Text("1-3".into()))));
        assert!(has(r" TOC \H ", 'h'));
        assert_eq!(
            switch(r#" DATE \@ "d MMMM yyyy" "#, '@').as_deref(),
            Some("d MMMM yyyy")
        );
        assert_eq!(switch(r" SEQ Figure \r 3 ", 'r').as_deref(), Some("3"));
        assert_eq!(switch(r" PAGE \* ARABIC ", '*').as_deref(), Some("ARABIC"));
    }

    #[test]
    fn what_a_table_of_contents_lists() {
        let found = toc_options(r#" TOC \o "1-3" \h \z \u "#, ",");
        assert_eq!((found.outline, found.own), (Some((1, 3)), true));
        let found = toc_options(r#" TOC \t "Title,1,Sub,x" "#, ",");
        assert_eq!(found.outline, None);
        assert_eq!(found.styles, vec![("Title".to_owned(), 1), ("Sub".to_owned(), 1)]);
        assert_eq!(toc_options(" TOC ", ",").outline, Some((1, 9)));
    }
}
