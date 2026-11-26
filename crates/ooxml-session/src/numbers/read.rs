//! Owns: the numbers of a text a check of numbers compares (ooxml-edit's `numbers.find_numbers`
//! and `_parse`, ported exactly): each read in the locale -- its value, the decimals written, a
//! sign, a percent sign or word, a scale word, a thousands separator -- and those left aside,
//! counted by reason (dates and times, years, list numbers, ids, whole numbers, a score's
//! scale, the application's ignore list); and what a cell's number format shows of a number
//! ([`shown`]).
//!
//! Python's patterns are kept as written, read as Python reads them
//! ([`ooxml_tools::pyre::translate`]); offsets are bytes here, characters where Python counts
//! them (the window before a number, a span written by reference).
//!
//! Must not: compare a number with a value (the index does), or know a format.

use super::NumberRules;
use fancy_regex::Regex;
use ooxml_base::pystr::{is_alpha, is_decimal, is_digit};
use ooxml_numfmt::Locale;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// The spaces a locale groups thousands with, and that may come before a percent sign
/// (`_SPACE`).
const SPACE: &str = "[ \u{a0}\u{202f}]";
/// The signs a number may be written with (`_MINUS`).
const MINUS: &str = "-\u{2212}\u{2012}\u{2013}";
/// The hyphens that join an identifier's parts (round 49): ASCII, U+2010 and Word's
/// non-breaking hyphen U+2011 (`_JOINS`).
const JOINS: [char; 3] = ['-', '\u{2010}', '\u{2011}'];

/// The scale words and their factors (`_SCALES`), in Python's order.
pub(crate) const SCALES: [(&str, u64); 16] = [
    ("k", 1000),
    ("thousand", 1000),
    ("duizend", 1000),
    ("m", 1_000_000),
    ("mn", 1_000_000),
    ("mln", 1_000_000),
    ("mio", 1_000_000),
    ("million", 1_000_000),
    ("millions", 1_000_000),
    ("miljoen", 1_000_000),
    ("bn", 1_000_000_000),
    ("mld", 1_000_000_000),
    ("mrd", 1_000_000_000),
    ("billion", 1_000_000_000),
    ("billions", 1_000_000_000),
    ("miljard", 1_000_000_000),
];

/// The language tags whose measured month names a date's are (ooxml-common's `NAMES`).
const NAMED: [&str; 10] = [
    "en-US", "en-GB", "nl-NL", "nl-BE", "de-DE", "fr-FR", "es-ES", "it-IT", "es-MX", "es",
];

/// The patterns, compiled once.
struct Patterns {
    candidate: Regex,
    spaced: Regex,
    percent: Regex,
    scale: Regex,
    dates: Vec<Regex>,
    naming: Regex,
    scale_after: Regex,
    leading: Regex,
    lines: Regex,
}

/// Python's pattern `pattern` (`re.IGNORECASE` where `ignore_case`) as `fancy-regex` reads it.
pub(crate) fn compiled(pattern: &str, ignore_case: bool) -> Result<Regex, String> {
    let translated = ooxml_tools::pyre::translate(pattern).map_err(|e| format!("{pattern}: {e}"))?;
    let source = if ignore_case {
        format!("(?i){translated}")
    } else {
        translated
    };
    Regex::new(&source).map_err(|e| format!("{pattern}: {e}"))
}

/// A pattern anchored as Python's `pattern.match` anchors it.
fn anchored(pattern: &str, ignore_case: bool) -> Result<Regex, String> {
    compiled(&format!("^(?:{pattern})"), ignore_case)
}

/// The month names and abbreviations of every measured language, folded, without a final
/// `.`, three characters or more, longest first (`_MONTHS`).
fn months() -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for tag in NAMED {
        let Some(names) = ooxml_numfmt::dates::measured_names(tag) else {
            continue;
        };
        for name in names.months.iter().chain(names.short_months.iter()) {
            let folded = super::casefold(name).trim_end_matches('.').to_owned();
            if name.chars().count() >= 3 && !found.contains(&folded) {
                found.push(folded);
            }
        }
    }
    found.sort_by_key(|n| std::cmp::Reverse(n.chars().count()));
    found
}

/// Python's `re.escape`.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if !(c.is_alphanumeric() || c == '_') && c.is_ascii() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl Patterns {
    fn new() -> Result<Patterns, String> {
        let month = format!(
            "(?:{})\\.?",
            months().iter().map(|m| escaped(m)).collect::<Vec<_>>().join("|")
        );
        let mut scales: Vec<&str> = SCALES.iter().map(|(word, _)| *word).collect();
        scales.sort_by_key(|w| std::cmp::Reverse(w.len()));
        let body = r"\d+(?:[.,'’]\d+)*";
        Ok(Patterns {
            candidate: compiled(&format!(r"(?<![\w#§№.,])([{MINUS}]?)({body})"), false)?,
            spaced: compiled(
                &format!(r"(?<![\w#§№.,])([{MINUS}]?)(\d+(?:(?:[.,'’]|{SPACE}(?=\d{{3}}(?!\d)))\d+)*)"),
                false,
            )?,
            percent: anchored(
                &format!(r"{SPACE}?(?:(%)|(‰)|(?:per\s?cent|percent|procent|prozent|pour\s?cent)(?![\w]))"),
                true,
            )?,
            scale: anchored(&format!(r"{SPACE}?({})\.?(?![\w])", scales.join("|")), true)?,
            dates: vec![
                compiled(r"(?<![\d.,])\d{1,4}([./-])\d{1,2}\1\d{1,4}(?![\d])", false)?,
                compiled(r"(?<![\d.,])\d{4}-\d{2}(?![\d])", false)?,
                compiled(r"(?<![\d.,:])\d{1,2}:\d{2}(?::\d{2})?(?![\d])", false)?,
                compiled(
                    &format!(r"(?<![\d.,])\d{{1,2}}[.hu]\d{{2}}{SPACE}?(?:u|uur|h|am|pm|a\.m\.|p\.m\.)(?![\w])"),
                    true,
                )?,
                compiled(
                    &format!(r"(?<![\w.,])\d{{1,2}}(?:st|nd|rd|th|e|er|de|ste)?{SPACE}?{month}(?![\w])"),
                    true,
                )?,
                compiled(&format!(r"(?<![\w]){month}{SPACE}?\d{{1,2}}(?![\d]|[.,]\d)"), true)?,
            ],
            naming: compiled(
                &format!(
                    r"(?:\b(?:sections?|chapters?|pages?|pp?\.|figures?|fig\.|tables?|versions?|v\.|releases?|steps?|items?|no\.|nr\.?|art\.|articles?|annex|appendix|hoofdstuk|paragraaf|pagina|p\.|figuur|tabel|versie|bijlage|artikel|stap|punt|kapitel|seite|abschnitt|chapitre|annexe){SPACE}?)$"
                ),
                true,
            )?,
            scale_after: compiled(&format!(r"\d{SPACE}?(?:/|out\s+of|op|van|sur|von){SPACE}?$"), true)?,
            leading: anchored(
                &format!(r"\s*\(?\d+(?:\.\d+)*(?:[.)](?=\s)|(?=\t)|(?={SPACE}+[A-Z]))"),
                false,
            )?,
            lines: compiled(r"[^\n]+", false)?,
        })
    }
}

static PATTERNS: LazyLock<Result<Patterns, String>> = LazyLock::new(Patterns::new);

/// The patterns, or why they did not compile (an `internal` failure).
fn patterns() -> Result<&'static Patterns, String> {
    PATTERNS.as_ref().map_err(Clone::clone)
}

/// A decimal number as Python's `Decimal` reads digits: its digits (ASCII) and how many of
/// them are decimals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal {
    /// Its digits, ASCII, without a point.
    digits: String,
    /// How many of `digits` come after the point.
    decimals: usize,
}

impl Decimal {
    /// `whole` and `fraction` (Python's digits, any script) as one decimal.
    fn of(whole: &str, fraction: &str) -> Option<Decimal> {
        let mut digits = String::with_capacity(whole.len() + fraction.len());
        for c in whole.chars().chain(fraction.chars()) {
            digits.push(char::from(
                b'0' + u8::try_from(ooxml_base::pystr::decimal_value(c)?).ok()?,
            ));
        }
        Some(Decimal {
            digits,
            decimals: fraction.chars().count(),
        })
    }

    /// Its value as a float (`float(value)`).
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        let (whole, fraction) = self
            .digits
            .split_at(self.digits.len() - self.decimals.min(self.digits.len()));
        format!("{}.{}", if whole.is_empty() { "0" } else { whole }, fraction)
            .parse()
            .unwrap_or(0.0)
    }

    /// Its value without trailing zeros, as `Decimal.normalize()` compares it: the digits
    /// with no leading or trailing zeros, and the power of ten of the last (`0` for zero).
    #[must_use]
    pub fn normalized(&self) -> (String, i64) {
        let exponent = -i64::try_from(self.decimals).unwrap_or(0);
        normalized(&self.digits, exponent)
    }
}

/// `digits` times ten to `exponent`, normalized as [`Decimal::normalized`] says.
pub(crate) fn normalized(digits: &str, exponent: i64) -> (String, i64) {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        return ("0".to_owned(), 0);
    }
    let kept = trimmed.trim_end_matches('0');
    let zeros = i64::try_from(trimmed.len() - kept.len()).unwrap_or(0);
    (kept.to_owned(), exponent + zeros)
}

/// A number in a text (`Found`): where (bytes: the digits, a sign and a percent sign or scale
/// word with them), as written, its magnitude, the decimals written, a sign written, a
/// percent sign's factor (`%` 100, `‰` 1000), a scale word's factor, a thousands separator
/// written, the span of the digits alone, and whether its decimal is the other convention's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Its first byte.
    pub start: usize,
    /// The byte after its last.
    pub end: usize,
    /// As written.
    pub text: String,
    /// Its magnitude.
    pub value: Decimal,
    /// The decimals written.
    pub decimals: usize,
    /// A sign written.
    pub negative: bool,
    /// `%`: 100, `‰`: 1000.
    pub percent: Option<u32>,
    /// A scale word's factor.
    pub scale: Option<u64>,
    /// A thousands separator written.
    pub grouped: bool,
    /// The digits alone, bytes.
    pub digits: (usize, usize),
    /// Its decimal is the other convention's.
    pub other: bool,
}

impl Found {
    /// A whole number without a percent sign (`whole`).
    #[must_use]
    pub fn whole(&self) -> bool {
        self.decimals == 0 && self.percent.is_none()
    }
}

/// The numbers of a text, and how many were left aside, by reason.
pub type Numbers = (Vec<Found>, BTreeMap<String, usize>);

/// `(value, decimals, grouped, other)` of a run of digits and separators read in `locale`
/// (`_parse`); `None` where it is no number (`2.1.3`).
#[must_use]
pub fn parse(body: &str, locale: &Locale) -> Option<(Decimal, usize, bool, bool)> {
    let decimal = locale.decimal_separator;
    let group = locale.thousands_separator;
    let spaces = ['\u{a0}', '\u{202f}', ' '];
    let quotes = ['\'', '\u{2019}'];
    let seps: Vec<char> = body.chars().filter(|c| !is_digit(*c)).collect();
    if seps.is_empty() {
        return Some((Decimal::of(body, "")?, 0, false, false));
    }
    let parts: Vec<&str> = body.split(|c: char| !is_decimal(c)).collect();
    let grouping = |chars: &[char], pieces: &[&str]| {
        chars.iter().all(|c| {
            *c == group
                || (spaces.contains(c) && spaces.contains(&group))
                || (quotes.contains(c) && quotes.contains(&group))
        }) && pieces.first().is_some_and(|p| (1..=3).contains(&p.chars().count()))
            && pieces.iter().skip(1).all(|p| p.chars().count() == 3)
    };
    let (last_sep, first_seps) = seps.split_last()?;
    let (last, first) = parts.split_last()?;
    if *last_sep == decimal && grouping(first_seps, first) {
        let whole: String = first.concat();
        return Some((Decimal::of(&whole, last)?, last.chars().count(), seps.len() > 1, false));
    }
    if grouping(&seps, &parts) {
        return Some((Decimal::of(&parts.concat(), "")?, 0, true, false));
    }
    if let ([sep], [whole, fraction]) = (seps.as_slice(), parts.as_slice()) {
        if matches!(sep, '.' | ',') {
            // the other convention's decimal
            return Some((Decimal::of(whole, fraction)?, fraction.chars().count(), false, true));
        }
    }
    None
}

/// The byte `count` characters before `at` in `text` (at least 0).
fn back(text: &str, at: usize, count: usize) -> usize {
    text.get(..at)
        .and_then(|before| before.char_indices().rev().nth(count.saturating_sub(1)).map(|(i, _)| i))
        .filter(|_| count > 0)
        .unwrap_or(if count == 0 { at } else { 0 })
}

/// The byte of the character `index` of `text` (its length past the end).
pub(crate) fn byte_of(text: &str, index: usize) -> usize {
    text.char_indices().nth(index).map_or(text.len(), |(i, _)| i)
}

/// Whether the span `at` overlaps one of `spans`.
fn inside(at: (usize, usize), spans: &[(usize, usize)]) -> bool {
    spans.iter().any(|(start, end)| *start < at.1 && at.0 < *end)
}

/// Whether the digits at `start` are a part of an identifier (round 49, R93): a hyphen
/// right before them, and before it a token -- letters, digits, `.`, `_` and hyphens, up to
/// a space or other mark -- with a letter in it (`_identifier_part`).
fn identifier_part(text: &str, start: usize) -> bool {
    let mut before = text.get(..start).unwrap_or_default().chars().rev();
    if !before.next().is_some_and(|c| JOINS.contains(&c)) {
        return false;
    }
    if text.get(..start).map_or(0, |b| b.chars().count()) < 2 {
        return false;
    }
    for c in before {
        if is_alpha(c) {
            return true;
        }
        if !(is_digit(c) || c == '.' || c == '_' || JOINS.contains(&c)) {
            return false;
        }
    }
    false
}

/// Every match of `pattern` in `text`, as `(start, end)` bytes.
fn spans(pattern: &Regex, text: &str) -> Vec<(usize, usize)> {
    pattern
        .find_iter(text)
        .flatten()
        .map(|m| (m.start(), m.end()))
        .collect()
}

/// Whether `pattern` is found in `text` from the byte `from` up to the byte `to` (Python's
/// `pattern.search(text, from, to)`: what comes before `from` is seen, nothing after `to`).
fn search(pattern: &Regex, text: &str, from: usize, to: usize) -> bool {
    text.get(..to)
        .is_some_and(|head| pattern.find_from_pos(head, from).ok().flatten().is_some())
}

/// `pattern` matched right at the byte `at` of `text` (`pattern.match(text, at)`): the
/// match's captures, its offsets from `at`.
fn match_at<'t>(pattern: &Regex, text: &'t str, at: usize) -> Option<fancy_regex::Captures<'t>> {
    pattern.captures(text.get(at..)?).ok().flatten()
}

/// The spans of the ignore list's phrases in `text`: each phrase found in the folded text,
/// at the same character offsets in `text` (as Python indexes the folded text).
fn ignored_spans(text: &str, phrases: &[String]) -> Vec<(usize, usize)> {
    let folded = super::casefold(text);
    let mut out = Vec::new();
    for phrase in phrases {
        let mut from = 0;
        while let Some(found) = folded.get(from..).and_then(|rest| rest.find(phrase.as_str())) {
            let at = from + found;
            let start = folded.get(..at).map_or(0, |b| b.chars().count());
            let end = start + phrase.chars().count();
            out.push((byte_of(text, start), byte_of(text, end)));
            from = at
                + folded
                    .get(at..)
                    .and_then(|r| r.chars().next())
                    .map_or(1, char::len_utf8);
        }
    }
    out
}

/// The spans of the paragraphs' own numbers at a line's start (`1.`, `2)`, `2.3 Results`).
fn leading_spans(text: &str, patterns: &Patterns) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for line in patterns.lines.find_iter(text).flatten() {
        if let Some(found) = match_at(&patterns.leading, line.as_str(), 0).and_then(|c| c.get(0)) {
            out.push((line.start(), line.start() + found.end()));
        }
    }
    out
}

/// The numbers of `text` a check compares, and how many were left aside, by reason
/// (`find_numbers`); `written`: the character spans written by reference, left out.
///
/// # Errors
/// The patterns' failure to compile (an `internal` failure, never the text's).
pub fn find_numbers(
    text: &str,
    locale: &Locale,
    rules: &NumberRules,
    written: &[(usize, usize)],
) -> Result<Numbers, String> {
    let patterns = patterns()?;
    let written: Vec<(usize, usize)> = written
        .iter()
        .map(|(start, end)| (byte_of(text, *start), byte_of(text, *end)))
        .collect();
    let mut aside: BTreeMap<String, usize> = BTreeMap::new();
    let mut set_aside = |reason: &str| *aside.entry(reason.to_owned()).or_default() += 1;
    let dates: Vec<(usize, usize)> = patterns.dates.iter().flat_map(|p| spans(p, text)).collect();
    let ignored = ignored_spans(text, &rules.phrases);
    let leading = leading_spans(text, patterns);
    let spaced = [' ', '\u{a0}', '\u{202f}'].contains(&locale.thousands_separator);
    let candidate = if spaced { &patterns.spaced } else { &patterns.candidate };
    let mut out = Vec::new();
    for captures in candidate.captures_iter(text).flatten() {
        let (Some(whole), Some(body)) = (captures.get(0), captures.get(2)) else {
            continue;
        };
        let (start, end) = (body.start(), body.end());
        let mut sign = captures.get(1).map_or("", |s| s.as_str());
        let before = text.get(..whole.start()).and_then(|b| b.chars().next_back());
        if !sign.is_empty() && before.is_some_and(|c| is_digit(c) || is_alpha(c)) {
            sign = ""; // a range 2-3, a word's hyphen
        }
        let span = (start - sign.len(), end);
        if inside(span, &written) {
            continue;
        }
        let reason = aside_before_parse(text, span, sign, (start, end), patterns, (&ignored, &dates, &leading));
        if let Some(reason) = reason {
            set_aside(reason);
            continue;
        }
        let parsed = parse(body.as_str(), locale);
        let named = search(&patterns.naming, text, back(text, span.0, 16), span.0);
        let Some((value, decimals, grouped, other)) = parsed.filter(|_| !named) else {
            set_aside("ids"); // 2.1.3; section 4.2, version 2.1
            continue;
        };
        let number = read_after(
            text,
            span,
            (start, end),
            sign,
            (value, decimals, grouped, other),
            patterns,
        );
        if let Some(reason) = aside_after_parse(text, &number, body.as_str(), rules, patterns) {
            set_aside(reason);
            continue;
        }
        out.push(number);
    }
    Ok((out, aside))
}

/// Spans of a text, bytes.
type Spans<'a> = &'a [(usize, usize)];

/// Why a candidate at `span` (its digits at `digits`) is left aside before it is read, if
/// it is: an identifier's digits, digits joined to letters, the ignore list, a date, a list
/// number.
fn aside_before_parse(
    text: &str,
    span: (usize, usize),
    sign: &str,
    digits: (usize, usize),
    patterns: &Patterns,
    (ignored, dates, leading): (Spans<'_>, Spans<'_>, Spans<'_>),
) -> Option<&'static str> {
    if sign.is_empty() && identifier_part(text, digits.0) {
        return Some("ids"); // K7-12.3, DOC-004.2 (round 49)
    }
    let after = text.get(digits.1..).and_then(|a| a.chars().next());
    if after.is_some_and(|c| is_alpha(c) || c == '_') && match_at(&patterns.scale, text, digits.1).is_none() {
        return Some("ids"); // Q3, CO2, 3D, 2nd
    }
    if inside(span, ignored) {
        return Some("ignored");
    }
    if inside(span, dates) {
        return Some("dates");
    }
    if leading
        .iter()
        .any(|(first, last)| *first <= digits.0 && digits.1 <= *last)
    {
        return Some("list numbers");
    }
    None
}

/// The number at `span` read: with the percent sign or word, else the scale word, after it.
fn read_after(
    text: &str,
    span: (usize, usize),
    digits: (usize, usize),
    sign: &str,
    (value, decimals, grouped, other): (Decimal, usize, bool, bool),
    patterns: &Patterns,
) -> Found {
    let mut stop = digits.1;
    let mut percent = None;
    if let Some(found) = match_at(&patterns.percent, text, digits.1) {
        percent = Some(if found.get(2).is_some() { 1000 } else { 100 });
        stop = digits.1 + found.get(0).map_or(0, |m| m.end());
    }
    let mut scale = None;
    if percent.is_none() {
        if let Some(found) = match_at(&patterns.scale, text, digits.1) {
            let word = found.get(1).map_or(String::new(), |w| super::casefold(w.as_str()));
            scale = SCALES.iter().find(|(w, _)| *w == word).map(|(_, factor)| *factor);
            stop = digits.1 + found.get(0).map_or(0, |m| m.end());
        }
    }
    Found {
        start: span.0,
        end: stop,
        text: text.get(span.0..stop).unwrap_or_default().to_owned(),
        value,
        decimals,
        negative: !sign.is_empty(),
        percent,
        scale,
        grouped,
        digits,
        other,
    }
}

/// Why a number read is left aside, if it is: a year, the ignore list, a whole number, a
/// score's scale.
fn aside_after_parse(
    text: &str,
    number: &Found,
    body: &str,
    rules: &NumberRules,
    patterns: &Patterns,
) -> Option<&'static str> {
    let whole = number.whole();
    let magnitude = number.value.to_f64();
    if whole && number.scale.is_none() && (1800.0..=2199.0).contains(&magnitude) && body.chars().count() == 4 {
        return Some("years");
    }
    if rules.numbers.contains(&number.value.normalized()) {
        return Some("ignored");
    }
    if whole && !rules.whole_numbers {
        return Some("whole");
    }
    if whole && search(&patterns.scale_after, text, back(text, number.start, 12), number.start) {
        return Some("scales"); // the 5 of 2,7/5, out of 5
    }
    None
}

/// What a cell's number format `code` shows of a number (`shown`): `(decimals, percent,
/// divide)` -- the digits after its decimal point, whether it multiplies by 100, and by
/// what it divides (each `,` right after the last digit placeholder divides by 1000) --
/// read from its first section, as ECMA-376 Part 1, 18.8.31 (numFmt) writes the codes.
/// `None` for `General`, a text format, or none.
#[must_use]
pub fn shown(code: Option<&str>) -> Option<(usize, bool, u64)> {
    let code = code.filter(|c| !c.is_empty())?;
    let section = code.split(';').next().unwrap_or_default();
    let plain = plain_code(section);
    let bare = super::casefold(crate::facts::py_strip(&plain));
    if matches!(bare.as_str(), "general" | "@" | "") || !plain.contains(['0', '#', '?']) {
        return None;
    }
    let chars: Vec<char> = plain.chars().collect();
    let exponent = chars
        .windows(2)
        .any(|w| matches!(w, [e, s] if matches!(e, 'e' | 'E') && matches!(s, '+' | '-')));
    if exponent {
        return None; // scientific: no fixed decimals
    }
    let placeholder = |c: &char| matches!(c, '0' | '#' | '?');
    let decimals = match chars.iter().position(|c| *c == '.') {
        Some(point) => chars.iter().skip(point + 1).filter(|c| placeholder(c)).count(),
        None => 0,
    };
    let last = chars.iter().rposition(placeholder)?;
    let commas = chars.iter().skip(last + 1).take_while(|c| **c == ',').count();
    let divide = 1000_u64.saturating_pow(u32::try_from(commas).unwrap_or(u32::MAX));
    Some((decimals, plain.contains('%'), divide))
}

/// A format section without its quoted text, escapes, brackets, padding and fills
/// (`re.sub(r'"[^"]*"|\\.|\[[^\]]*\]|_.|\*.', "", section)`).
fn plain_code(section: &str) -> String {
    let chars: Vec<char> = section.chars().collect();
    let mut out = String::with_capacity(section.len());
    let mut at = 0;
    while let Some(c) = chars.get(at).copied() {
        let skip = match c {
            '"' => chars.iter().skip(at + 1).position(|c| *c == '"').map(|n| n + 2),
            '[' => chars.iter().skip(at + 1).position(|c| *c == ']').map(|n| n + 2),
            '\\' | '_' | '*' => chars.get(at + 1).filter(|c| **c != '\n').map(|_| 2),
            _ => None,
        };
        match skip {
            Some(n) => at += n,
            None => {
                out.push(c);
                at += 1;
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
