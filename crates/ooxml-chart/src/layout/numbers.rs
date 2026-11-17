//! Owns: a number written as a chart writes it (`chart/layout.py`'s `format_number`): the
//! format code's section for the value's sign, its literals, digits with grouping, scaling
//! commas, optional decimals and an exponent, percent, and `General` -- through the locale's
//! separators. A date code shows through `ooxml_numfmt::format_serial` at the caller.
//!
//! Must not: write fractions, colours or conditions (a chart's labels do not), or decide
//! which format code applies.

use crate::pyfmt;
use ooxml_numfmt::{is_date_code, sections, Locale};

/// The kinds of a chart's own format tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Literal,
    General,
    Percent,
    Digits,
    Date,
}

/// A whole number as `str(int(value))`, else `f"{round(value, 10):g}"`.
pub fn general(value: f64) -> String {
    if value == value.trunc() {
        let whole = format!("{value:.0}");
        return if whole == "-0" { "0".into() } else { whole };
    }
    pyfmt::general(pyfmt::round(value, 10), 6)
}

/// `value` written through `format_code` (`General` or `@` where none).
pub fn format_number(value: f64, format_code: Option<&str>, locale: &Locale) -> String {
    let code = match format_code {
        None | Some("General" | "@") => return locale.number_text(&general(value)),
        Some(code) => code,
    };
    let all = sections(code);
    let (section, sign, mut magnitude) = match (all.get(2), all.get(1)) {
        (Some(zero), _) if value == 0.0 => (zero.as_str(), "", 0.0),
        (_, Some(negative)) if value < 0.0 => (negative.as_str(), "", value.abs()),
        _ => (
            all.first().map_or("", String::as_str),
            if value < 0.0 { "-" } else { "" },
            value.abs(),
        ),
    };
    let tokens = format_tokens(section);
    let has = |k: Kind| tokens.iter().any(|(kind, _)| *kind == k);
    if !has(Kind::Digits) && !has(Kind::General) && (has(Kind::Date) || tokens.is_empty()) {
        return locale.number_text(&general(value));
    }
    if has(Kind::Percent) {
        magnitude *= 100.0;
    }
    let pattern = tokens.iter().find(|(k, _)| *k == Kind::Digits).map(|(_, t)| t.as_str());
    let mut out = String::from(sign);
    let mut placed = false;
    for (kind, text) in &tokens {
        if matches!(kind, Kind::Digits | Kind::General) {
            if !placed {
                let number = match pattern {
                    Some(pattern) if *kind != Kind::General => format_digits(magnitude, pattern),
                    _ => general(magnitude),
                };
                out.push_str(&locale.number_text(&number));
                placed = true;
            }
        } else {
            out.push_str(text);
        }
    }
    out
}

/// The characters of `chars[from..to]`, clamped.
fn slice(chars: &[char], from: usize, to: usize) -> String {
    chars
        .get(from.min(chars.len())..to.min(chars.len()))
        .unwrap_or_default()
        .iter()
        .collect()
}

fn find(chars: &[char], target: char, from: usize) -> usize {
    chars
        .iter()
        .skip(from)
        .position(|&c| c == target)
        .map_or(chars.len(), |p| p + from)
}

fn digit_char(c: char) -> bool {
    "0#?.,".contains(c)
}

/// A section as the chart's own tokens.
fn format_tokens(section: &str) -> Vec<(Kind, String)> {
    let chars: Vec<char> = section.chars().collect();
    let length = chars.len();
    let mut tokens = Vec::new();
    let mut index = 0;
    while let Some(&c) = chars.get(index) {
        if c == '"' {
            let close = find(&chars, '"', index + 1);
            tokens.push((Kind::Literal, slice(&chars, index + 1, close)));
            index = close + 1;
        } else if c == '\\' {
            tokens.push((Kind::Literal, slice(&chars, index + 1, index + 2)));
            index += 2;
        } else if c == '_' {
            tokens.push((Kind::Literal, " ".into()));
            index += 2;
        } else if c == '*' {
            index += 2;
        } else if c == '[' {
            let close = find(&chars, ']', index);
            let inner = slice(&chars, index + 1, close);
            if let Some(rest) = inner.strip_prefix('$') {
                tokens.push((Kind::Literal, rest.split('-').next().unwrap_or("").to_owned()));
            }
            index = close + 1;
        } else if slice(&chars, index, index + 7).to_lowercase() == "general" {
            tokens.push((Kind::General, slice(&chars, index, index + 7)));
            index += 7;
        } else if c == '@' {
            tokens.push((Kind::General, "@".into()));
            index += 1;
        } else if c == '%' {
            tokens.push((Kind::Percent, "%".into()));
            index += 1;
        } else if "0#?".contains(c) || (".,".contains(c) && matches!(chars.get(index + 1), Some('0' | '#' | '?'))) {
            let mut run = index;
            while chars.get(run).is_some_and(|&r| digit_char(r)) {
                run += 1;
            }
            if run < length
                && matches!(chars.get(run), Some('E' | 'e'))
                && matches!(chars.get(run + 1), Some('+' | '-'))
            {
                run += 2;
                while chars.get(run).is_some_and(|&r| "0#?".contains(r)) {
                    run += 1;
                }
            }
            tokens.push((Kind::Digits, slice(&chars, index, run)));
            index = run;
        } else if c.is_alphabetic() && c.to_lowercase().all(|l| "dmyhs".contains(l)) && c.to_lowercase().count() == 1 {
            tokens.push((Kind::Date, c.to_string()));
            index += 1;
        } else {
            tokens.push((Kind::Literal, c.to_string()));
            index += 1;
        }
    }
    tokens
}

/// Whether the section `format_code` shows `value` through is a date.
pub fn shows_date(format_code: Option<&str>, value: f64) -> bool {
    let Some(code) = format_code.filter(|c| !c.is_empty()) else {
        return false;
    };
    let count = sections(code).len();
    let index = if value == 0.0 && count > 2 {
        2
    } else if value < 0.0 && count > 1 {
        1
    } else {
        0
    };
    is_date_code(Some(code), Some(index))
}

/// `value` with `places` decimals and commas between thousands.
fn grouped(value: f64, places: usize) -> String {
    let text = pyfmt::fixed(value, places);
    let (sign, body) = text.strip_prefix('-').map_or(("", text.as_str()), |b| ("-", b));
    let (whole, fraction) = body.split_once('.').map_or((body, None), |(w, f)| (w, Some(f)));
    let mut out = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    match fraction {
        Some(f) => format!("{sign}{out}.{f}"),
        None => format!("{sign}{out}"),
    }
}

fn count(text: &str, set: &str) -> usize {
    text.chars().filter(|c| set.contains(*c)).count()
}

/// `magnitude` written through a digits pattern.
fn format_digits(mut magnitude: f64, pattern: &str) -> String {
    let (pattern, exponent_part) = match pattern
        .char_indices()
        .find(|(i, c)| matches!(c, 'E' | 'e') && matches!(pattern.get(i + 1..i + 2), Some("+" | "-")))
    {
        Some((i, _)) => (pattern.get(..i).unwrap_or(""), pattern.get(i..).unwrap_or("")),
        None => (pattern, ""),
    };
    let (integer, decimals) = pattern.split_once('.').unwrap_or((pattern, ""));
    let integer_trimmed = integer.trim_end_matches(',');
    let decimals_trimmed = decimals.trim_end_matches(',');
    let scale = (integer.len() - integer_trimmed.len()) + (decimals.len() - decimals_trimmed.len());
    #[allow(clippy::cast_precision_loss)]
    let scale_f = scale as f64;
    magnitude /= crate::pymath::pow(1000.0, scale_f);
    let grouping = integer_trimmed.contains(',');
    let required = count(decimals_trimmed, "0");
    let places = count(decimals_trimmed, "0#?");
    if !exponent_part.is_empty() {
        let mut exp = if magnitude == 0.0 {
            0.0
        } else {
            magnitude.log10().floor()
        };
        let mut mantissa = if magnitude == 0.0 {
            0.0
        } else {
            magnitude / crate::pymath::pow(10.0, exp)
        };
        if pyfmt::round(mantissa, places) >= 10.0 {
            mantissa /= 10.0;
            exp += 1.0;
        }
        let digits = count(exponent_part, "0#?").max(1);
        let sign = if exp < 0.0 {
            "-"
        } else if exponent_part.get(1..2) == Some("+") {
            "+"
        } else {
            ""
        };
        let text = trim_decimals(&pyfmt::fixed(mantissa, places), required, places);
        let mark = exponent_part.get(..1).unwrap_or("E");
        #[allow(clippy::cast_possible_truncation)]
        let power = exp.abs() as i64;
        return format!("{text}{mark}{sign}{power:0digits$}");
    }
    let text = if grouping {
        grouped(magnitude, places)
    } else {
        pyfmt::fixed(magnitude, places)
    };
    let mut text = trim_decimals(&text, required, places);
    let no_zero = count(integer_trimmed, "0") == 0;
    if no_zero && text.starts_with('0') && (text.len() == 1 || text.get(1..2) == Some(".")) {
        text.remove(0);
    }
    text
}

/// Optional (`#`, `?`) decimals' trailing zeros removed, keeping the required ones.
fn trim_decimals(text: &str, required: usize, places: usize) -> String {
    if places <= required || !text.contains('.') {
        return text.to_owned();
    }
    let (head, tail) = text.split_once('.').unwrap_or((text, ""));
    let keep = tail.trim_end_matches('0').len();
    let tail = tail.get(..required.max(keep)).unwrap_or(tail);
    format!("{head}.{tail}")
}

#[cfg(test)]
mod tests {
    use super::format_number;
    use ooxml_numfmt::EN_US;

    #[test]
    fn numbers_write_as_charts_write_them() {
        for (value, code, written) in [
            (1234.5, None, "1234.5"),
            (1234.5, Some("#,##0.00"), "1,234.50"),
            (0.25, Some("0%"), "25%"),
            (-3.0, Some("0;(0)"), "(3)"),
            (0.0, Some("0;-0;\"zero\""), "zero"),
            (12345.0, Some("0.00E+00"), "1.23E+04"),
            (0.5, Some("#.##"), ".5"),
            (1500000.0, Some("#,##0,,\"M\""), "2M"),
            (3.0, Some("\"$\"#,##0"), "$3"),
        ] {
            assert_eq!(format_number(value, code, &EN_US), written, "{value} {code:?}");
        }
    }
}
