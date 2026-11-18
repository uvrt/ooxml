//! Owns: what a page-number field draws for a number (docx2svg's `fields.field_text`,
//! `number_text`, `format_value`, `switches`, `_picture`): a page's number in its
//! section's `w:pgNumType/@w:fmt` or by the instruction's `\*` and `\#` switches, and the
//! case switches; and a field instruction's first argument (`_argument`).
//!
//! Must not: find a page or a bookmark (`crate::fields` does), or compute a value in the
//! application's language (that is drawn as Word cached it).

/// Why a field's text cannot be computed from the file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Uncomputable(pub String);

fn uncomputable(why: impl Into<String>) -> Uncomputable {
    Uncomputable(why.into())
}

/// Formats and switches whose words are the application's language.
const LANGUAGE: [&str; 4] = ["ordinal", "cardinaltext", "ordinaltext", "dollartext"];
/// `chicago`'s symbols, repeated once more at every cycle.
const CHICAGO: [&str; 4] = ["*", "\u{2020}", "\u{2021}", "\u{a7}"];

fn roman(mut value: i64) -> String {
    const TABLE: [(i64, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (number, letters) in TABLE {
        while value >= number {
            out.push_str(letters);
            value -= number;
        }
    }
    out
}

/// `a` .. `z`, then `aa` .. `zz`: the letter repeated once more every 26; a space for 0.
fn letters(value: i64) -> String {
    if value <= 0 {
        return " ".into();
    }
    let index = u8::try_from((value - 1) % 26).unwrap_or(0);
    let count = usize::try_from((value - 1) / 26 + 1).unwrap_or(1);
    char::from(b'a' + index).to_string().repeat(count)
}

/// A number in a `w:pgNumType/@w:fmt` (`format_value`).
pub fn format_value(value: i64, format: Option<&str>) -> Result<String, Uncomputable> {
    let format = format.unwrap_or("decimal");
    Ok(match format {
        "decimal" => value.to_string(),
        "upperRoman" | "lowerRoman" => {
            let r = if value > 0 { roman(value) } else { " ".into() };
            if format == "upperRoman" {
                r.to_uppercase()
            } else {
                r
            }
        }
        "upperLetter" => letters(value).to_uppercase(),
        "lowerLetter" => letters(value),
        "numberInDash" => format!("- {value} -"),
        "decimalZero" => format!("{value:02}"),
        "hex" => format!("{value:X}"),
        "chicago" if value > 0 => {
            let k = usize::try_from((value - 1) % 4).unwrap_or(0);
            let count = usize::try_from((value - 1) / 4 + 1).unwrap_or(1);
            CHICAGO.get(k).copied().unwrap_or("*").repeat(count)
        }
        other if LANGUAGE.contains(&other.to_lowercase().as_str()) => {
            return Err(uncomputable(format!("{other} is in the application's language")))
        }
        other => {
            return Err(uncomputable(format!(
                "the page number format {other:?} is not measured"
            )))
        }
    })
}

/// `[(kind, argument)]` of an instruction's switches (`\* roman`, `\# "000"`): the
/// pattern `\\([*#@!])\s*("[^"]*"|\S+)`.
pub fn switches(instruction: &str) -> Vec<(char, String)> {
    let chars: Vec<char> = instruction.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k + 1 < chars.len() {
        let kind = chars.get(k + 1).copied().unwrap_or(' ');
        if chars.get(k) != Some(&'\\') || !matches!(kind, '*' | '#' | '@' | '!') {
            k += 1;
            continue;
        }
        let mut j = k + 2;
        while chars.get(j).is_some_and(|c| c.is_whitespace()) {
            j += 1;
        }
        let (argument, next) = if chars.get(j) == Some(&'"') {
            match chars.iter().skip(j + 1).position(|c| *c == '"') {
                Some(close) => (chars.get(j..j + close + 2), j + close + 2),
                None => (None, j),
            }
        } else {
            let end = chars
                .iter()
                .skip(j)
                .position(|c| c.is_whitespace())
                .map_or(chars.len(), |p| j + p);
            (if end > j { chars.get(j..end) } else { None }, end)
        };
        match argument {
            Some(a) => {
                let text: String = a.iter().collect();
                out.push((kind, text.trim_matches('"').to_owned()));
                k = next;
            }
            None => k += 2,
        }
    }
    out
}

/// A numeric picture of `0` and `#` only: the digits right-aligned in it (`_picture`).
fn picture(value: i64, picture: &str) -> Result<String, Uncomputable> {
    if picture.is_empty() || picture.chars().any(|c| c != '0' && c != '#') {
        return Err(uncomputable(format!("the numeric picture {picture:?} is not measured")));
    }
    let digits = value.to_string();
    let places = picture.chars().count().saturating_sub(digits.chars().count());
    let head: String = picture
        .chars()
        .take(places)
        .map(|c| if c == '0' { '0' } else { ' ' })
        .collect();
    Ok(head + &digits)
}

fn first_cap(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `value` formatted by the instruction's switches, else in `format`, then its case
/// switches applied (`number_text`).
pub fn number_text(instruction: &str, value: i64, format: Option<&str>) -> Result<String, Uncomputable> {
    let mut text = None;
    let mut case = None;
    for (kind, argument) in switches(instruction) {
        if kind == '#' {
            text = Some(picture(value, &argument)?);
            continue;
        }
        if kind != '*' {
            continue;
        }
        let word = argument.to_lowercase();
        let upper = argument.chars().next().is_some_and(char::is_uppercase);
        match word.as_str() {
            "mergeformat" | "charformat" => {}
            "upper" | "lower" | "firstcap" | "caps" => case = Some(word),
            "roman" => {
                text = Some(format_value(
                    value,
                    Some(if upper { "upperRoman" } else { "lowerRoman" }),
                )?)
            }
            "alphabetic" => {
                text = Some(format_value(
                    value,
                    Some(if upper { "upperLetter" } else { "lowerLetter" }),
                )?);
            }
            "arabic" => text = Some(value.to_string()),
            "arabicdash" => text = Some(format!("- {value} -")),
            "hex" => text = Some(format!("{value:X}")),
            w if LANGUAGE.contains(&w) || matches!(w, "ordinal" | "cardtext" | "ordtext") => {
                return Err(uncomputable(format!("\\* {argument} is in the application's language")))
            }
            _ => return Err(uncomputable(format!("the switch \\* {argument} is not measured"))),
        }
    }
    let text = match text {
        Some(t) => t,
        None => format_value(value, format)?,
    };
    Ok(match case.as_deref() {
        Some("upper") => text.to_uppercase(),
        Some("lower") => text.to_lowercase(),
        Some("firstcap") => first_cap(&text),
        Some("caps") => text.split(' ').map(first_cap).collect::<Vec<_>>().join(" "),
        _ => text,
    })
}

/// The instruction with its quoted strings emptied (`_unquoted`).
fn unquoted(instruction: &str) -> String {
    let mut out = String::with_capacity(instruction.len());
    let mut quoted = false;
    for c in instruction.chars() {
        if c == '"' {
            quoted = !quoted;
            out.push('"');
        } else if !quoted {
            out.push(c);
        }
    }
    out
}

/// What a page-number field draws: `PAGE` (and `PAGEREF`, at its bookmark's page) the
/// page's number in its section's format, `NUMPAGES` the page count, `SECTIONPAGES` the
/// section's (`field_text`).
pub fn field_text(
    instruction: &str,
    keyword: &str,
    page: i64,
    counts: (Option<i64>, Option<i64>),
    section_format: Option<&str>,
) -> Result<String, Uncomputable> {
    let (pages, section_pages) = counts;
    let keyword = if keyword == "PAGEREF" {
        let flags = unquoted(instruction);
        if flags.split_whitespace().any(|w| w.eq_ignore_ascii_case("\\p")) {
            return Err(uncomputable(
                "\\p says where the bookmark is in the application's language",
            ));
        }
        "PAGE"
    } else {
        keyword
    };
    let (value, format) = match keyword {
        "PAGE" => (page, section_format),
        "NUMPAGES" => (
            pages.ok_or_else(|| uncomputable("the page count is not known past where the layout stops"))?,
            None,
        ),
        "SECTIONPAGES" => (
            section_pages
                .ok_or_else(|| uncomputable("the section's page count is not known past where the layout stops"))?,
            None,
        ),
        other => return Err(uncomputable(format!("{other} is not computed"))),
    };
    number_text(instruction, value, format)
}

/// A field's first argument: `REF Name \h` -> `Name`, quotes removed (`_argument`).
pub fn argument(instruction: &str) -> Option<String> {
    let mut words = Vec::new();
    let mut chars = instruction.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut word = String::new();
        if c == '"' {
            word.push(c);
            chars.next();
            for d in chars.by_ref() {
                word.push(d);
                if d == '"' {
                    break;
                }
            }
        } else {
            while let Some(&d) = chars.peek() {
                if d.is_whitespace() {
                    break;
                }
                word.push(d);
                chars.next();
            }
        }
        words.push(word);
        if words.len() > 1 {
            break;
        }
    }
    words
        .get(1)
        .filter(|w| !w.starts_with('\\'))
        .map(|w| w.trim_matches('"').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(format_value(14, Some("upperRoman")).ok().as_deref(), Some("XIV"));
        assert_eq!(format_value(27, Some("upperLetter")).ok().as_deref(), Some("AA"));
        assert_eq!(format_value(5, Some("chicago")).ok().as_deref(), Some("**"));
        assert_eq!(format_value(3, Some("decimalZero")).ok().as_deref(), Some("03"));
        assert!(format_value(3, Some("ordinal")).is_err());
    }

    #[test]
    fn switches_and_pictures() {
        assert_eq!(switches(r#"PAGE \* roman \# "00" \* MERGEFORMAT"#).len(), 3);
        assert_eq!(number_text(r"PAGE \* Roman", 4, None).ok().as_deref(), Some("IV"));
        assert_eq!(number_text(r##"PAGE \# "#0""##, 2, None).ok().as_deref(), Some(" 2"));
        assert_eq!(number_text(r#"PAGE \# "00""#, 120, None).ok().as_deref(), Some("120"));
        assert_eq!(argument(r#"PAGEREF _Toc123 \h"#).as_deref(), Some("_Toc123"));
        assert_eq!(argument(r"PAGEREF \h").as_deref(), None);
        assert!(field_text(r"PAGEREF x \p", "PAGEREF", 3, (None, None), None).is_err());
        assert_eq!(
            field_text("NUMPAGES", "NUMPAGES", 1, (Some(9), None), None)
                .ok()
                .as_deref(),
            Some("9")
        );
    }
}
